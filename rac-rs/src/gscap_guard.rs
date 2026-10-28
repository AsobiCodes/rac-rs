//! Start-up guard of the GPU capture (the sharp in-game HUD). The capture draws on the window's own Vulkan device; on a graphics card
//! or driver we cannot test it may freeze or crash while it opens (GitHub #12: a white window on an AMD RX 9070 XT with v0.3.0).
//! So the start leaves a marker, `<player dir>/gscap_start.txt` (game version, adapter, driver, time), BEFORE the risky step
//! (`gpu_shared::acquire`: the device, the kernels, the first submits) and removes it only once the start is proven alive: the GPU
//! capture took over AND `FIELDS_NEEDED` fields were drawn by it AND `FRAMES_NEEDED` frames were presented since. A normal quit
//! (`quit`: the window's close, the end of `main`) removes it too, so a slow but live start never counts as frozen: the window keeps
//! presenting with the CPU capture while the kernels build, and only a dead or killed process leaves the marker behind.
//! The next start finds it. ONE dead start is not proof (v0.4.1 safety review: a Task-Manager kill, a power cut, an unrelated crash or a
//! second click on the shortcut while the first start opens would have switched the sharp HUD off for good): the marker carries
//! `dead=<n>`, the dead starts before it, and the next start tries the GPU capture again with `dead` one higher. Only a SECOND dead start
//! in a row switches it off: `gscap_gpu=off` goes into `pc_options.txt` (every other key stays) with `gscap_gpu_note` ("(auto: start
//! on <adapter> froze <date>)") and `gscap_gpu_froze` (the game version), the run log says so, a short non-blocking box tells the
//! player once, and the capture draws on the CPU (the Original HUD, the v0.3.1 picture). One proven or normally
//! quit start in between resets the count (the marker is gone); a proven start that wrote no marker of its own (`Other`) removes a
//! marker whose owner has died meanwhile (`clear_dead_owner`). The marker also names its owner (`pid=`, `born=`: the process id and
//! the process start time): a marker whose owner is still running belongs to a second game window that is still opening, not to a
//! dead start, so a start that finds one neither counts it nor deletes it, and leaves its own marker unwritten (`Start::Other`).
//! The marker is written whole to a temporary file next to it and renamed over it, so a start that reads it never sees half a file;
//! an empty file counts as no marker.
//! A newer game version tries `auto` again once
//! (the next version may fix the driver issue); a marker of another version is dropped. `RAC_GSCAP_GPU` set always wins over all this.
//! Only a start with a window is guarded (`activate`, from `gpu_shared::expect_window`): warm-gpu, headless smoke runs and unit tests
//! leave no marker. Tests: `RAC_TEST_GSGPU_DELAY=<s>` waits that long inside the risky step, `RAC_TEST_GSGPU_CRASH=1` aborts there,
//! `RAC_TEST_GAME_VERSION` fakes the game version.
use crate::retail::pcopts::PcOptions;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::Relaxed};
use std::sync::OnceLock;
use std::time::Instant;

/// Fields the GPU capture must have drawn, and frames presented after it took over, before the start counts as alive.
const FIELDS_NEEDED: u32 = 30;
const FRAMES_NEEDED: u32 = 30;
/// Frames presented while the capture is still opening that the run log mentions once (the window did not freeze).
const PRE_NOTE: u32 = 120;

static ACTIVE: AtomicBool = AtomicBool::new(false);
/// This start owns the marker file (it wrote it and removes it).
static ARMED: AtomicBool = AtomicBool::new(false);
/// This start counts its fields and frames towards "alive": `ARMED`, or a start that found a live owner's marker (`FOREIGN`).
static TRACK: AtomicBool = AtomicBool::new(false);
static TOOK: AtomicBool = AtomicBool::new(false);
static FIELDS: AtomicU32 = AtomicU32::new(0);
static FRAMES: AtomicU32 = AtomicU32::new(0);
static PRE: AtomicU32 = AtomicU32::new(0);
/// Dead starts in a row before this one (`start_up` found a marker and tried again); the new marker carries it.
static DEAD: AtomicU32 = AtomicU32::new(0);
/// `start_up` found the marker of another game process that is still running: this start does not write or remove a marker.
static FOREIGN: AtomicBool = AtomicBool::new(false);
/// `proven` ran: this process is a proven start, so `begin` never arms again (a second `acquire` must not leave a marker for the session).
static PROVEN: AtomicBool = AtomicBool::new(false);
/// `quit` ran: the game is ending normally, so nothing may write a marker any more (a window closed while the capture was still opening).
static QUIT: AtomicBool = AtomicBool::new(false);
static SINCE: OnceLock<Instant> = OnceLock::new();
/// What `off` decided for this start (once).
static OFF: OnceLock<bool> = OnceLock::new();
/// The window's adapter (`note_window`): what the marker names until the capture has its own answer.
static WINDOW: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// What the start finds on disk.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(crate) enum Start {
	/// Nothing wrong: the GPU capture is tried.
	Gpu,
	/// `gscap_gpu=off` stays (the player's choice, or the guard's for this game version).
	Off,
	/// A marker of this version, the second dead start in a row: the last two starts froze or crashed while the capture opened. Switch it off.
	SwitchOff,
	/// A marker of this version, the first dead start: it may have been killed for another reason. The GPU capture is tried again.
	Doubt,
	/// `gscap_gpu=off` was the guard's for another game version: the options are cleared, the GPU capture is tried again.
	Retry,
	/// A marker of another game version: dropped, the GPU capture is tried.
	StaleMarker,
	/// The marker belongs to a game process that is still running (a second window, still opening): not a dead start. It is neither
	/// counted nor removed, the GPU capture is tried, and this start writes no marker of its own.
	Other,
}

/// What a marker file holds.
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub(crate) struct Mark {
	pub version: String,
	pub adapter: String,
	/// Dead starts in a row before the start that wrote this marker.
	pub dead: u32,
	/// The process that wrote it (0 = an old marker without `pid=`), and when that process started (0 = unknown).
	pub pid: u32,
	pub born: u64,
}

/// The decision: the options' `gscap_gpu` (None / true = auto, false = off), the version that froze (the guard's own note), the marker,
/// whether the marker's owner is a process that is still running, and the running game version.
pub(crate) fn decide(opt: Option<bool>, froze: Option<&str>, marker: Option<&Mark>, owner_running: bool, version: &str) -> Start {
	if opt == Some(false) {
		return if froze.is_some_and(|v| v != version) { Start::Retry } else { Start::Off };
	}
	match marker {
		// A live owner is no dead start, whatever version it is.
		Some(_) if owner_running => Start::Other,
		// A marker whose version is unknown (a half-written file) counts: the writer died.
		Some(m) if m.version.is_empty() || m.version == version => if m.dead >= 1 { Start::SwitchOff } else { Start::Doubt },
		Some(_) => Start::StaleMarker,
		None => Start::Gpu,
	}
}

fn version() -> String {
	std::env::var("RAC_TEST_GAME_VERSION").unwrap_or_else(|_| crate::app_name().to_string())
}

fn marker_path(dir: &Path) -> std::path::PathBuf {
	dir.join("gscap_start.txt")
}

fn read_marker(dir: &Path) -> Option<Mark> {
	let t = std::fs::read(marker_path(dir)).ok()?;
	let t = String::from_utf8_lossy(&t);
	// an empty (or zero-filled) file is no marker: it says nothing about an owner, and it must not delete one
	if t.chars().all(|c| c.is_whitespace() || c == '\0') {
		return None;
	}
	let get = |k: &str| t.lines().find_map(|l| l.strip_prefix(k)).unwrap_or("").trim().to_string();
	Some(Mark { version: get("version="), adapter: get("adapter="), dead: get("dead=").parse().unwrap_or(0), pid: get("pid=").parse().unwrap_or(0), born: get("born=").parse().unwrap_or(0) })
}

/// The marker of this process.
fn write_marker(dir: &Path, version: &str, adapter: &str, driver: &str, dead: u32) {
	let pid = std::process::id();
	write_marker_for(dir, version, adapter, driver, dead, pid, started(pid).unwrap_or(0));
}

fn write_marker_for(dir: &Path, version: &str, adapter: &str, driver: &str, dead: u32, pid: u32, born: u64) {
	let _ = std::fs::create_dir_all(dir);
	let text = format!("version={version}\nadapter={adapter}\ndriver={driver}\ndead={dead}\npid={pid}\nborn={born}\ntime={}\n", stamp(now_secs()));
	// whole file or nothing: a start that reads the marker meanwhile never sees it empty or cut
	let tmp = dir.join(format!("gscap_start.{pid}.tmp"));
	if std::fs::write(&tmp, text).is_err() || std::fs::rename(&tmp, marker_path(dir)).is_err() {
		let _ = std::fs::remove_file(&tmp);
	}
}

/// When the process `pid` started (an opaque number, equal for the same process: Windows the creation time in 100 ns ticks, Linux field
/// 22 of /proc/<pid>/stat), or None when no such process is running. With the pid it tells a live owner from a pid that was reused.
#[cfg(windows)]
fn started(pid: u32) -> Option<u64> {
	use std::ffi::c_void;
	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn OpenProcess(access: u32, inherit: i32, pid: u32) -> *mut c_void;
		fn GetExitCodeProcess(process: *mut c_void, code: *mut u32) -> i32;
		fn GetProcessTimes(process: *mut c_void, creation: *mut u64, exit: *mut u64, kernel: *mut u64, user: *mut u64) -> i32;
		fn CloseHandle(handle: *mut c_void) -> i32;
	}
	const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
	const STILL_ACTIVE: u32 = 259;
	// SAFETY: plain kernel32 calls; the handle is closed before returning and the out values are local integers (a FILETIME is two u32).
	unsafe {
		let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
		if h.is_null() {
			return None;
		}
		let mut code = 0u32;
		let (mut created, mut exited, mut kernel, mut user) = (0u64, 0u64, 0u64, 0u64);
		let running = GetExitCodeProcess(h, &mut code) != 0 && code == STILL_ACTIVE;
		let known = running && GetProcessTimes(h, &mut created, &mut exited, &mut kernel, &mut user) != 0;
		CloseHandle(h);
		known.then_some(created)
	}
}

#[cfg(target_os = "linux")]
fn started(pid: u32) -> Option<u64> {
	parse_stat(&std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?)
}

/// The start time out of a /proc/<pid>/stat line, None for a process that has ended (a zombie, 'Z', stays in /proc until its parent waits
/// for it; 'X' is dead). "pid (name) state ppid ...": the name may hold spaces and brackets, so count from the last ')'; the state is the
/// first field after it, field 22 (the start time) the 20th.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_stat(t: &str) -> Option<u64> {
	let mut f = t.rsplit_once(')')?.1.split_whitespace();
	let state = f.next()?;
	if state == "Z" || state == "X" {
		return None;
	}
	f.nth(18)?.parse().ok()
}

#[cfg(not(any(windows, target_os = "linux")))]
fn started(_pid: u32) -> Option<u64> {
	None
}

/// The marker's owner is another process that is running right now (same pid and the same start time, so not a reused pid). A marker
/// without a start time proves nothing: it counts like a dead start.
fn owner_running(m: &Mark) -> bool {
	m.pid != 0 && m.pid != std::process::id() && m.born != 0 && started(m.pid) == Some(m.born)
}

/// A start that found a live owner's marker has proven itself: if that owner has died since, its marker is the left-over of a dead
/// start the proof outweighs, so it goes and the next start counts nothing. A marker of a running owner stays.
fn clear_dead_owner(dir: &Path, running: &dyn Fn(&Mark) -> bool) -> bool {
	match read_marker(dir) {
		Some(m) if !running(&m) => {
			remove_marker(dir);
			true
		}
		_ => false,
	}
}

fn remove_marker(dir: &Path) {
	let _ = std::fs::remove_file(marker_path(dir));
}

fn now_secs() -> u64 {
	std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
fn civil(days: u64) -> (u64, u64, u64) {
	let z = days + 719_468;
	let era = z / 146_097;
	let doe = z - era * 146_097;
	let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
	let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
	let mp = (5 * doy + 2) / 153;
	let d = doy - (153 * mp + 2) / 5 + 1;
	let m = if mp < 10 { mp + 3 } else { mp - 9 };
	(yoe + era * 400 + u64::from(m <= 2), m, d)
}

/// "2026-10-23 05:39:12 UTC".
fn stamp(secs: u64) -> String {
	let (y, m, d) = civil(secs / 86_400);
	let s = secs % 86_400;
	format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} UTC", s / 3600, s / 60 % 60, s % 60)
}

/// The start-up check against `dir` (the player folder): applies the decision to the files and returns it.
pub(crate) fn start_up(dir: &Path, version: &str, secs: u64) -> Start {
	sweep_temp_files(dir, &|pid| started(pid).is_some());
	start_up_with(dir, version, secs, &owner_running)
}

/// The `gscap_start.<pid>.tmp` files a writer left behind when it died between writing and renaming: those of a pid that is not a
/// running process (and not this one) go. A running pid's file stays (its owner removes it itself).
fn sweep_temp_files(dir: &Path, running: &dyn Fn(u32) -> bool) {
	let Ok(entries) = std::fs::read_dir(dir) else { return };
	for e in entries.flatten() {
		let name = e.file_name().to_string_lossy().into_owned();
		let pid = name.strip_prefix("gscap_start.").and_then(|r| r.strip_suffix(".tmp")).and_then(|p| p.parse::<u32>().ok());
		if let Some(pid) = pid.filter(|p| *p != std::process::id() && !running(*p)) {
			let _ = std::fs::remove_file(e.path());
		}
	}
}

/// `start_up` with the "is the marker's owner running" check given (tests).
fn start_up_with(dir: &Path, version: &str, secs: u64, running: &dyn Fn(&Mark) -> bool) -> Start {
	let file = dir.join("pc_options.txt");
	let mut o = PcOptions::load(&file);
	let marker = read_marker(dir);
	let live = marker.as_ref().is_some_and(running);
	let s = decide(o.gscap_gpu, o.gscap_froze.as_deref(), marker.as_ref(), live, version);
	match s {
		Start::Gpu => {}
		Start::Other => {
			FOREIGN.store(true, Relaxed);
			eprintln!("gscap guard: another game window (process {}) is still opening its GPU capture: its marker is left alone, this start counts nothing and writes no marker", marker.as_ref().map_or(0, |m| m.pid));
		}
		Start::Off => {
			remove_marker(dir);
			eprintln!("gscap guard: the GPU capture is off (pc_options.txt gscap_gpu=off{})", o.gscap_note.as_deref().map(|n| format!(" {n}")).unwrap_or_default());
		}
		Start::Doubt => {
			DEAD.store(marker.as_ref().map_or(1, |m| m.dead + 1), Relaxed);
			remove_marker(dir);
			eprintln!("gscap guard: the last start ended while the GPU capture was opening (a kill, a crash or a freeze): the GPU capture is tried again; a second dead start in a row switches it off");
		}
		Start::SwitchOff => {
			let adapter = marker.as_ref().map(|m| m.adapter.clone()).filter(|a| !a.is_empty()).unwrap_or_else(|| "an unknown adapter".into());
			o.gscap_gpu = Some(false);
			o.gscap_note = Some(format!("(auto: start on {adapter} froze {})", stamp(secs).split(' ').next().unwrap_or("")));
			o.gscap_froze = Some(version.to_string());
			let saved = o.store(&file).is_ok();
			remove_marker(dir);
			eprintln!("gscap guard: the last two starts ended while the GPU capture was opening ({adapter}): the GPU capture is OFF for this start; pc_options.txt gscap_gpu=off {}", if saved { "saved" } else { "could not be saved" });
		}
		Start::Retry => {
			let (note, froze) = (o.gscap_note.take(), o.gscap_froze.take());
			o.gscap_gpu = None;
			let saved = o.store(&file).is_ok();
			remove_marker(dir);
			eprintln!("gscap guard: this game version ({version}) is not the one that froze ({}): the GPU capture is tried again once {} {}", froze.unwrap_or_default(), note.unwrap_or_default(), if saved { "(pc_options.txt cleared)" } else { "(pc_options.txt could not be saved)" });
		}
		Start::StaleMarker => {
			remove_marker(dir);
			eprintln!("gscap guard: a marker of another game version ({}) was dropped", marker.map(|m| m.version).unwrap_or_default());
		}
	}
	s
}

/// A window is starting: the guard works for this run (gpu_shared.rs `expect_window`).
pub(crate) fn activate() {
	ACTIVE.store(true, Relaxed);
}

/// Whether the GPU capture is off for this start (the first call checks the disk; only for a start with a window).
pub(crate) fn off() -> bool {
	if !ACTIVE.load(Relaxed) {
		return false;
	}
	*OFF.get_or_init(|| {
		let s = start_up(&crate::retail::memcard::player_dir(), &version(), now_secs());
		if s == Start::SwitchOff {
			tell_player();
		}
		matches!(s, Start::Off | Start::SwitchOff)
	})
}

fn describe(info: &wgpu::AdapterInfo) -> String {
	format!("{} (vendor 0x{:04x}, device 0x{:04x}, {:?}, {:?})", info.name, info.vendor, info.device, info.backend, info.device_type)
}

/// The window's adapter is known (`gpu_shared::window_ready`).
pub(crate) fn note_window(info: &wgpu::AdapterInfo) {
	*WINDOW.lock().unwrap_or_else(|e| e.into_inner()) = describe(info);
}

/// The marker of this start: it carries the dead starts in a row found by `start_up` (`DEAD`), the link that makes the second strike work.
fn arm(dir: &Path, version: &str, adapter: &str, driver: &str) {
	if QUIT.load(Relaxed) {
		return;
	}
	write_marker(dir, version, adapter, driver, DEAD.load(Relaxed));
	// `quit` may have run while the file was written: its removal found nothing, so this one goes
	if QUIT.load(Relaxed) {
		remove_marker(dir);
	}
}

/// The step that may freeze begins (`gpu_shared::acquire`): the marker is written first. Then the test hooks.
pub(crate) fn begin() {
	// once proven or quit nothing arms: a second opening of the capture in a proven session must not leave a marker (DEAD may still be 1)
	if ACTIVE.load(Relaxed) && !PROVEN.load(Relaxed) && !QUIT.load(Relaxed) && !TRACK.swap(true, Relaxed) {
		let _ = SINCE.set(Instant::now());
		if !FOREIGN.load(Relaxed) {
			ARMED.store(true, Relaxed);
			let w = WINDOW.lock().unwrap_or_else(|e| e.into_inner()).clone();
			arm(&crate::retail::memcard::player_dir(), &version(), if w.is_empty() { "unknown" } else { &w }, "");
		}
	}
	if let Some(s) = std::env::var("RAC_TEST_GSGPU_DELAY").ok().and_then(|v| v.parse::<f64>().ok()).filter(|s| *s > 0.0) {
		eprintln!("gscap guard: test delay {s} s inside the opening step");
		std::thread::sleep(std::time::Duration::from_secs_f64(s));
	}
	if std::env::var_os("RAC_TEST_GSGPU_CRASH").is_some() {
		eprintln!("gscap guard: test crash inside the opening step");
		std::process::abort();
	}
}

/// The adapter is known: the marker names it.
pub(crate) fn note_adapter(info: &wgpu::AdapterInfo) {
	if ARMED.load(Relaxed) {
		arm(&crate::retail::memcard::player_dir(), &version(), &describe(info), &format!("{} {}", info.driver, info.driver_info));
	}
}

/// The capture could not open (no GPU, device refused): that start is not frozen, the marker goes.
pub(crate) fn cancel() {
	TRACK.store(false, Relaxed);
	if ARMED.swap(false, Relaxed) {
		remove_marker(&crate::retail::memcard::player_dir());
	}
}

/// The game took the opened GPU capture over (draw/gpu.rs `Gpu::take`).
pub(crate) fn took_over() {
	if TRACK.load(Relaxed) && !TOOK.swap(true, Relaxed) {
		eprintln!("gscap guard: the GPU capture took over after {:.1} s ({} frames presented meanwhile)", SINCE.get().map_or(0.0, |t| t.elapsed().as_secs_f64()), PRE.load(Relaxed));
	}
}

/// A field ended on the GPU capture (gscap.rs `run`).
pub(crate) fn field_done() {
	if TOOK.load(Relaxed) && TRACK.load(Relaxed) && counted(&FIELDS, FIELDS_NEEDED) {
		proven();
	}
}

/// A frame was presented (viewer/handler/frame_start.rs `frame_begin`, main thread).
pub(crate) fn frame() {
	if !TRACK.load(Relaxed) {
		return;
	}
	if TOOK.load(Relaxed) {
		if counted(&FRAMES, FRAMES_NEEDED) {
			proven();
		}
	} else if PRE.fetch_add(1, Relaxed) + 1 == PRE_NOTE {
		eprintln!("gscap guard: {PRE_NOTE} frames presented while the GPU capture is still opening ({:.1} s): the window is alive", SINCE.get().map_or(0.0, |t| t.elapsed().as_secs_f64()));
	}
}

/// One more field or frame: whether the counter has reached its number (`>=`, so two threads that both pass the number at once cannot
/// both miss it; `proven` is gated to run once).
fn counted(counter: &AtomicU32, needed: u32) -> bool {
	counter.fetch_add(1, Relaxed) + 1 >= needed
}

/// Both counters reached their number: the marker goes (a start that wrote none removes a dead owner's).
fn proven() {
	if TRACK.load(Relaxed) && FIELDS.load(Relaxed) >= FIELDS_NEEDED && FRAMES.load(Relaxed) >= FRAMES_NEEDED {
		proven_in(&crate::retail::memcard::player_dir());
	}
}

/// The proof, once (`TRACK` and `PROVEN` are swapped): the dead count of the start before is forgotten, and from now on `begin` arms nothing.
fn proven_in(dir: &Path) {
	if TRACK.swap(false, Relaxed) && !PROVEN.swap(true, Relaxed) {
		DEAD.store(0, Relaxed);
		if ARMED.swap(false, Relaxed) {
			remove_marker(dir);
			eprintln!("gscap guard: the start is alive ({} fields drawn on the GPU, {} frames presented since it took over): marker removed", FIELDS.load(Relaxed), FRAMES.load(Relaxed));
		} else if clear_dead_owner(dir, &owner_running) {
			eprintln!("gscap guard: the start is alive ({} fields drawn on the GPU, {} frames presented since it took over): the marker of the other window, which has ended since, is removed", FIELDS.load(Relaxed), FRAMES.load(Relaxed));
		}
	}
}

/// The game ends on its own (the window's close, the end of `main`): a start that was only slow leaves no marker.
pub(crate) fn quit() {
	QUIT.store(true, Relaxed);
	TRACK.store(false, Relaxed);
	if ARMED.swap(false, Relaxed) {
		remove_marker(&crate::retail::memcard::player_dir());
		eprintln!("gscap guard: normal quit before the start was proven: marker removed");
	}
}

/// The text of the one-time note.
fn note_text() -> &'static str {
	"The last two starts ended while the sharp HUD (the GPU capture) opened on your graphics card, so it is switched off now. The game switched to the standard in-game HUD (the same picture as before) and runs normally. Nothing else changed.\n\nThe sharp HUD is tried again after the next game update."
}

/// A short box on a thread of its own (the game does not wait for it); tests only log.
fn tell_player() {
	eprintln!("gscap guard: note to the player: {}", note_text().lines().next().unwrap_or(""));
	if !crate::rtss_crash::test_mode() {
		let _ = std::thread::Builder::new().name("gscap note".into()).spawn(|| info_box(note_text()));
	}
}

pub(crate) fn info_box(text: &str) {
	// MB_OK | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST.
	message_box(text, "Ratchet & Clank (rac-rs)", 0x30 | 0x1_0000 | 0x4_0000);
}

/// A MessageBox with its own caption and `kind` flags (MB_*), blocking this thread until it is closed. Other boxes (`info_box`, a
/// second rac-rs process) have other captions, so a caller that closes its box by caption never touches them.
#[cfg(windows)]
pub(crate) fn message_box(text: &str, caption: &str, kind: u32) {
	#[link(name = "user32")]
	unsafe extern "system" {
		fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
	}
	let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
	// SAFETY: NUL-terminated UTF-16 strings that outlive the call.
	unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide(caption).as_ptr(), kind) };
}

#[cfg(not(windows))]
pub(crate) fn message_box(_text: &str, _caption: &str, _kind: u32) {}

#[cfg(test)]
mod tests {
	use super::*;
	use std::sync::{Mutex, MutexGuard};

	/// The tests that touch the process-wide statics (DEAD, FOREIGN, PROVEN, QUIT ...) run one at a time, each from a clean state.
	static STATICS: Mutex<()> = Mutex::new(());

	fn statics() -> MutexGuard<'static, ()> {
		let g = STATICS.lock().unwrap_or_else(|e| e.into_inner());
		for b in [&ACTIVE, &ARMED, &TRACK, &TOOK, &FOREIGN, &PROVEN, &QUIT] {
			b.store(false, Relaxed);
		}
		for n in [&FIELDS, &FRAMES, &PRE, &DEAD] {
			n.store(0, Relaxed);
		}
		g
	}

	fn mark(v: &str) -> Mark {
		Mark { version: v.into(), adapter: "AMD Radeon RX 9070 XT".into(), dead: 0, pid: 0, born: 0 }
	}

	fn mark_dead(v: &str, dead: u32) -> Mark {
		Mark { dead, ..mark(v) }
	}

	fn dir(name: &str) -> std::path::PathBuf {
		let d = std::env::temp_dir().join(format!("gscap_guard_test_{}_{name}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	#[test]
	fn the_decision_table() {
		let v = "rac-rs v0.3.5";
		// option auto / missing x marker x version
		assert_eq!(decide(None, None, None, false, v), Start::Gpu);
		assert_eq!(decide(Some(true), None, None, false, v), Start::Gpu);
		assert_eq!(decide(None, None, Some(&mark(v)), false, v), Start::Doubt, "one dead start only doubts");
		assert_eq!(decide(None, None, Some(&mark_dead(v, 1)), false, v), Start::SwitchOff, "the second dead start in a row switches off");
		assert_eq!(decide(Some(true), None, Some(&mark(v)), false, v), Start::Doubt);
		assert_eq!(decide(Some(true), None, Some(&mark_dead(v, 4)), false, v), Start::SwitchOff);
		assert_eq!(decide(None, None, Some(&mark("rac-rs v0.3.1")), false, v), Start::StaleMarker);
		assert_eq!(decide(None, None, Some(&mark("")), false, v), Start::Doubt, "a half-written marker counts as a dead start");
		// option off: the player's own (no note) stays; the guard's stays for its version and retries for a newer one
		assert_eq!(decide(Some(false), None, None, false, v), Start::Off);
		assert_eq!(decide(Some(false), None, Some(&mark(v)), false, v), Start::Off);
		assert_eq!(decide(Some(false), Some(v), None, false, v), Start::Off);
		assert_eq!(decide(Some(false), Some("rac-rs v0.3.1"), None, false, v), Start::Retry);
		assert_eq!(decide(Some(false), None, None, false, "rac-rs v9"), Start::Off, "off without the guard's note never retries");
		// a marker whose owner is still running is no dead start, of any version and any count; the player's own off still wins
		assert_eq!(decide(None, None, Some(&mark(v)), true, v), Start::Other);
		assert_eq!(decide(None, None, Some(&mark_dead(v, 1)), true, v), Start::Other);
		assert_eq!(decide(None, None, Some(&mark("rac-rs v0.3.1")), true, v), Start::Other);
		assert_eq!(decide(None, None, None, true, v), Start::Gpu);
		assert_eq!(decide(Some(false), None, Some(&mark(v)), true, v), Start::Off);
	}

	#[test]
	fn the_marker_text_names_its_owner_and_old_markers_still_read() {
		let d = dir("owner");
		write_marker_for(&d, "v", "A", "drv", 1, 4242, 99_000);
		assert_eq!(read_marker(&d).unwrap(), Mark { version: "v".into(), adapter: "A".into(), dead: 1, pid: 4242, born: 99_000 });
		// a marker of the previous guard (no pid, no born, no dead) reads as before: no owner, nothing to be alive
		std::fs::write(marker_path(&d), "version=v\nadapter=A\ndriver=x\ntime=2026-10-27 00:00:00 UTC\n").unwrap();
		let old = read_marker(&d).unwrap();
		assert_eq!((old.dead, old.pid, old.born), (0, 0, 0));
		assert!(!owner_running(&old), "no pid = no live owner");
		// garbage after the keys never panics
		std::fs::write(marker_path(&d), "pid=-3\nborn=x\ndead=\u{0}\u{0}\n").unwrap();
		assert_eq!(read_marker(&d).unwrap().pid, 0);
		// this process writes its own pid and start time, and is never its own foreign owner
		write_marker(&d, "v", "A", "", 0);
		let own = read_marker(&d).unwrap();
		assert_eq!(own.pid, std::process::id());
		assert!(own.born != 0 || started(own.pid).is_none(), "the start time is written whenever the platform can tell it");
		assert!(!owner_running(&own));
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn a_running_process_is_found_and_a_finished_one_is_not() {
		if cfg!(any(windows, target_os = "linux")) {
			assert!(started(std::process::id()).is_some_and(|b| b != 0), "this process runs");
			assert_eq!(started(std::process::id()), started(std::process::id()), "same process, same start time");
			let mut child = std::process::Command::new(std::env::current_exe().unwrap()).arg("--list").stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
			let pid = child.id();
			child.wait().unwrap();
			assert_eq!(started(pid), None, "a finished process is dead");
			// a marker of the finished process, or of a live pid with another start time (a reused pid), has no live owner
			let gone = Mark { pid, born: 5, ..mark("v") };
			assert!(!owner_running(&gone));
		}
	}

	#[test]
	fn a_live_owner_needs_its_pid_and_its_own_start_time() {
		if !cfg!(any(windows, target_os = "linux")) {
			return;
		}
		// a really running second process: its pid is live, and only the start time tells the owner from a program that reused the pid
		let mut cmd = if cfg!(windows) { std::process::Command::new("ping") } else { std::process::Command::new("sleep") };
		let args: &[&str] = if cfg!(windows) { &["-n", "20", "127.0.0.1"] } else { &["20"] };
		let mut child = cmd.args(args).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap();
		let pid = child.id();
		let born = started(pid).expect("the child runs");
		assert!(born != 0);
		assert!(owner_running(&Mark { pid, born, ..mark("v") }), "same pid, same start time = the owner is running");
		assert!(!owner_running(&Mark { pid, born: born + 1, ..mark("v") }), "same pid, another start time = the pid was reused by another process");
		assert!(!owner_running(&Mark { pid, born: 0, ..mark("v") }), "no start time = no proof of a live owner (a damaged marker counts as a dead start)");
		assert!(!owner_running(&Mark { pid: std::process::id(), born: started(std::process::id()).unwrap(), ..mark("v") }), "never this process itself");
		child.kill().unwrap();
		child.wait().unwrap();
		assert!(!owner_running(&Mark { pid, born, ..mark("v") }), "the same marker once the owner ended");
	}

	#[test]
	fn a_stat_line_gives_the_start_time_and_a_zombie_is_not_running() {
		let run = "1234 (rac-rs) S 1 1234 1234 0 -1 4194560 100 0 0 0 5 3 0 0 20 0 8 0 987654 1000000 500 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 2 0 0 0 0 0";
		assert_eq!(parse_stat(run), Some(987_654));
		// the name may hold spaces and brackets
		let odd = "77 (a b) (c) R 1 77 77 0 -1 0 0 0 0 0 0 0 0 0 20 0 1 0 555 0 0 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0";
		assert_eq!(parse_stat(odd), Some(555));
		// a zombie (the game crashed, its parent has not waited for it) and a dead one are no live owner, with the same start time
		assert_eq!(parse_stat(&run.replace(") S ", ") Z ")), None);
		assert_eq!(parse_stat(&run.replace(") S ", ") X ")), None);
		assert_eq!(parse_stat(&run.replace(") S ", ") T ")), Some(987_654), "a stopped process still lives");
		assert_eq!(parse_stat("garbage"), None);
		assert_eq!(parse_stat("1 (x) S 1 2"), None, "a cut line");
	}

	#[test]
	fn a_marker_that_is_empty_or_cut_never_deletes_a_live_owners_marker() {
		let _g = statics();
		let d = dir("atomic");
		let v = "rac-rs v0.4.1";
		// the writer leaves a whole file and no temporary file
		write_marker_for(&d, v, "AMD", "drv", 0, 4242, 7);
		let whole = std::fs::read_to_string(marker_path(&d)).unwrap();
		assert!(whole.ends_with("UTC\n") && whole.contains("pid=4242\n") && whole.contains("born=7\n"), "{whole}");
		let tmps: Vec<_> = std::fs::read_dir(&d).unwrap().flatten().filter(|e| e.file_name().to_string_lossy().ends_with(".tmp")).collect();
		assert!(tmps.is_empty(), "no temporary file is left");
		// the marker is replaced in one step: rewriting over an existing one gives the new whole text
		write_marker_for(&d, v, "AMD", "drv", 1, 4242, 7);
		assert_eq!(read_marker(&d).unwrap().dead, 1);
		// what a start that reads in the middle of an old-style truncate-then-write sees: an empty (or zero-filled) file is no marker
		for empty in ["", "\n", "  \r\n", "\0\0\0\0"] {
			std::fs::write(marker_path(&d), empty).unwrap();
			assert_eq!(read_marker(&d), None, "{empty:?}");
			assert_eq!(start_up_with(&d, v, 0, &|_| false), Start::Gpu, "an empty marker is not a dead start");
			assert!(marker_path(&d).exists(), "and it deletes nothing");
		}
		// a missing file is no marker either
		std::fs::remove_file(marker_path(&d)).unwrap();
		assert_eq!(read_marker(&d), None);
		assert_eq!(start_up_with(&d, v, 0, &|_| false), Start::Gpu);
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn a_proven_start_clears_the_marker_of_a_window_that_died_meanwhile() {
		let _g = statics();
		// the double click: window A writes its marker, window B finds A alive (Other, no marker of its own); A is killed, B plays
		let d = dir("double");
		let v = "rac-rs v0.4.1";
		write_marker_for(&d, v, "AMD", "", 0, 4242, 7);
		assert_eq!(start_up_with(&d, v, 0, &|m| m.pid == 4242), Start::Other);
		FOREIGN.store(false, Relaxed);
		// B is proven while A still runs: A's marker stays
		assert!(!clear_dead_owner(&d, &|m| m.pid == 4242));
		assert!(read_marker(&d).is_some());
		// A died, B is proven: the dead owner's marker goes, so the next start counts nothing
		assert!(clear_dead_owner(&d, &|_| false));
		assert!(read_marker(&d).is_none());
		assert_eq!(start_up_with(&d, v, 0, &|_| false), Start::Gpu, "no Doubt: the proven start in between reset the count");
		// nothing to clear is not an error
		assert!(!clear_dead_owner(&d, &|_| false));
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn a_live_owner_keeps_its_marker_and_is_not_counted() {
		let _g = statics();
		let d = dir("live");
		let v = "rac-rs v0.4.1";
		std::fs::write(d.join("pc_options.txt"), "version=1\ncamera=modern\n").unwrap();
		write_marker_for(&d, v, "AMD", "", 1, 4242, 7);
		let before = std::fs::read_to_string(marker_path(&d)).unwrap();
		assert_eq!(start_up_with(&d, v, 0, &|m| m.pid == 4242), Start::Other);
		assert!(FOREIGN.load(Relaxed), "this start writes no marker of its own");
		FOREIGN.store(false, Relaxed);
		assert_eq!(std::fs::read_to_string(marker_path(&d)).unwrap(), before, "a live owner's marker is not deleted");
		assert_eq!(std::fs::read_to_string(d.join("pc_options.txt")).unwrap(), "version=1\ncamera=modern\n", "nothing switched off");
		// the same marker with a finished owner is the dead start it was before (here the second one in a row)
		assert_eq!(start_up_with(&d, v, 0, &|_| false), Start::SwitchOff);
		assert!(read_marker(&d).is_none());
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn the_marker_of_the_next_start_carries_the_dead_count() {
		let _g = statics();
		// the link that makes the second strike work: after a Doubt the REAL marker writer (begin / note_adapter call `arm`) writes dead=1
		let d = dir("arm");
		let v = "rac-rs v0.4.1";
		write_marker_for(&d, v, "AMD", "", 0, 1, 1);
		assert_eq!(start_up_with(&d, v, 0, &|_| false), Start::Doubt);
		assert_eq!(DEAD.load(Relaxed), 1);
		arm(&d, v, "AMD", "radv");
		assert_eq!(read_marker(&d).unwrap().dead, 1, "the marker after a Doubt counts the dead start before it");
		assert_eq!(start_up_with(&d, v, 1_792_000_000, &|_| false), Start::SwitchOff, "so the next dead start switches off");
		assert!(std::fs::read_to_string(d.join("pc_options.txt")).unwrap().contains("gscap_gpu=off\n"));
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn a_proven_start_forgets_the_dead_count_and_never_arms_again() {
		let _g = statics();
		let d = dir("proven");
		let v = "rac-rs v0.4.1";
		// a start after a Doubt: its marker carries dead=1
		write_marker_for(&d, v, "AMD", "", 0, 1, 1);
		assert_eq!(start_up_with(&d, v, 0, &|_| false), Start::Doubt);
		assert_eq!(DEAD.load(Relaxed), 1);
		ARMED.store(true, Relaxed);
		TRACK.store(true, Relaxed);
		arm(&d, v, "AMD", "");
		assert_eq!(read_marker(&d).unwrap().dead, 1);
		// proof: the marker goes, the count is forgotten, PROVEN is set
		proven_in(&d);
		assert!(read_marker(&d).is_none() && PROVEN.load(Relaxed) && !TRACK.load(Relaxed) && !ARMED.load(Relaxed));
		assert_eq!(DEAD.load(Relaxed), 0);
		// a second opening of the capture in the same process arms nothing (begin would write into the real player folder otherwise)
		ACTIVE.store(true, Relaxed);
		begin();
		assert!(!TRACK.load(Relaxed) && !ARMED.load(Relaxed), "begin does nothing once proven");
		// and a second proof does nothing: a marker that appeared meanwhile is not touched
		write_marker_for(&d, v, "AMD", "", 0, 4242, 7);
		TRACK.store(true, Relaxed);
		proven_in(&d);
		assert!(read_marker(&d).is_some(), "proven runs once");
		// whatever marker a late arm writes carries no strike
		assert_eq!(DEAD.load(Relaxed), 0);
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn nothing_writes_a_marker_after_quit() {
		let _g = statics();
		let d = dir("quit");
		ACTIVE.store(true, Relaxed);
		quit();
		assert!(QUIT.load(Relaxed));
		begin();
		assert!(!TRACK.load(Relaxed) && !ARMED.load(Relaxed), "begin arms nothing after quit");
		arm(&d, "v", "AMD", "");
		assert!(read_marker(&d).is_none(), "arm writes no marker after quit");
		assert!(std::fs::read_dir(&d).unwrap().next().is_none(), "and leaves no file");
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn the_counters_fire_at_the_number_and_after_it() {
		let c = AtomicU32::new(0);
		let fired: Vec<bool> = (0..4).map(|_| counted(&c, 3)).collect();
		assert_eq!(fired, [false, false, true, true], "a counter that already passed the number still reports it");
		let past = AtomicU32::new(FIELDS_NEEDED + 7);
		assert!(counted(&past, FIELDS_NEEDED));
	}

	#[test]
	fn left_over_temporary_files_of_ended_processes_are_swept() {
		let d = dir("tmp");
		let own = std::process::id();
		for n in ["gscap_start.111.tmp", "gscap_start.222.tmp", &format!("gscap_start.{own}.tmp"), "gscap_start.txt", "gscap_start.x.tmp", "pc_options.txt"] {
			std::fs::write(d.join(n), "x").unwrap();
		}
		sweep_temp_files(&d, &|pid| pid == 222);
		let mut left: Vec<String> = std::fs::read_dir(&d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
		left.sort();
		let mut want = vec!["gscap_start.222.tmp".to_string(), format!("gscap_start.{own}.tmp"), "gscap_start.txt".into(), "gscap_start.x.tmp".into(), "pc_options.txt".into()];
		want.sort();
		assert_eq!(left, want, "only the ended process's temp file goes");
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn the_player_note_names_no_file_to_edit() {
		let t = note_text();
		assert!(!t.contains("pc_options") && !t.contains("gscap_gpu"), "{t}");
		// no path and no file name at all
		let lower = t.to_lowercase();
		for bad in [".txt", ".log", ".ini", ".cfg", ".json", ".exe", "log", "/", "\\", "player"] {
			assert!(!lower.contains(bad), "the note names {bad:?}: {t}");
		}
		assert!(t.contains("standard in-game HUD") && t.contains("Nothing else changed"));
	}

	#[test]
	fn a_frozen_start_switches_off_once_and_keeps_the_other_keys() {
		let _g = statics();
		let d = dir("freeze");
		let v = "rac-rs v0.3.5";
		std::fs::write(d.join("pc_options.txt"), "version=1\ncamera=modern\nsome_future_key=7\n").unwrap();
		write_marker(&d, v, "AMD Radeon RX 9070 XT", "radv", 0);
		assert_eq!(read_marker(&d).unwrap(), Mark { pid: std::process::id(), born: started(std::process::id()).unwrap_or(0), ..mark(v) }, "the marker names this process");
		// the first dead start only doubts: the options stay as they are, the next marker will count one dead start
		assert_eq!(start_up(&d, v, 1_792_000_000), Start::Doubt);
		assert_eq!(std::fs::read_to_string(d.join("pc_options.txt")).unwrap(), "version=1\ncamera=modern\nsome_future_key=7\n");
		assert!(read_marker(&d).is_none() && DEAD.load(Relaxed) == 1);
		// a start that was proven or quit normally leaves no marker: the count starts again (nothing to find)
		assert_eq!(start_up(&d, v, 0), Start::Gpu);
		// two dead starts in a row (the second marker carries dead=1)
		write_marker(&d, v, "AMD Radeon RX 9070 XT", "radv", 1);
		assert_eq!(start_up(&d, v, 1_792_000_000), Start::SwitchOff);
		let t = std::fs::read_to_string(d.join("pc_options.txt")).unwrap();
		assert!(t.contains("gscap_gpu=off\n") && t.contains("camera=modern\n") && t.contains("some_future_key=7\n"), "{t}");
		assert!(t.contains("gscap_gpu_note=(auto: start on AMD Radeon RX 9070 XT froze 2026-10-"), "{t}");
		assert!(t.contains("gscap_gpu_froze=rac-rs v0.3.5\n"), "{t}");
		assert!(read_marker(&d).is_none(), "the marker is consumed");
		// the third start stays off, no new write needed
		assert_eq!(start_up(&d, v, 0), Start::Off);
		assert_eq!(std::fs::read_to_string(d.join("pc_options.txt")).unwrap(), t);
		// a newer game version tries again once and clears the guard's keys, the others stay
		assert_eq!(start_up(&d, "rac-rs v0.3.6", 0), Start::Retry);
		let t = std::fs::read_to_string(d.join("pc_options.txt")).unwrap();
		assert!(!t.contains("gscap_gpu") && t.contains("camera=modern") && t.contains("some_future_key=7"), "{t}");
		assert_eq!(start_up(&d, "rac-rs v0.3.6", 0), Start::Gpu);
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn no_marker_changes_nothing_and_a_stale_marker_is_dropped() {
		let _g = statics();
		let d = dir("calm");
		assert_eq!(start_up(&d, "v", 0), Start::Gpu);
		assert!(!d.join("pc_options.txt").exists(), "nothing is written for a calm start");
		write_marker(&d, "rac-rs v0.3.1", "X", "", 0);
		assert_eq!(start_up(&d, "rac-rs v0.3.5", 0), Start::StaleMarker);
		assert!(read_marker(&d).is_none() && !d.join("pc_options.txt").exists());
		let _ = std::fs::remove_dir_all(&d);
	}

	#[test]
	fn the_date_text() {
		assert_eq!(stamp(0), "1970-01-01 00:00:00 UTC");
		assert_eq!(stamp(1_792_000_000), "2026-10-14 17:46:40 UTC");
		assert_eq!(stamp(951_782_400), "2000-02-29 00:00:00 UTC");
	}
}
