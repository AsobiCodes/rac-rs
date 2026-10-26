//! Start-up guard of the GPU capture (the sharp in-game HUD). The capture draws on the window's own Vulkan device; on a graphics card
//! or driver we cannot test it may freeze or crash while it opens (GitHub #12: a white window on an AMD RX 9070 XT with v0.3.0).
//! So the start leaves a marker, `<player dir>/gscap_start.txt` (game version, adapter, driver, time), BEFORE the risky step
//! (`gpu_shared::acquire`: the device, the kernels, the first submits) and removes it only once the start is proven alive: the GPU
//! capture took over AND `FIELDS_NEEDED` fields were drawn by it AND `FRAMES_NEEDED` frames were presented since. A normal quit
//! (`quit`: the window's close, the end of `main`) removes it too, so a slow but live start never counts as frozen: the window keeps
//! presenting with the CPU capture while the kernels build, and only a dead or killed process leaves the marker behind.
//! The next start finds it: `gscap_gpu=off` goes into `pc_options.txt` (every other key stays) with `gscap_gpu_note` ("(auto: start
//! on <adapter> froze <date>)") and `gscap_gpu_froze` (the game version), the run log says so, a short non-blocking box tells the
//! player once, and the capture draws on the CPU (the Original HUD, the v0.3.1 picture). A newer game version tries `auto` again once
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
static ARMED: AtomicBool = AtomicBool::new(false);
static TOOK: AtomicBool = AtomicBool::new(false);
static FIELDS: AtomicU32 = AtomicU32::new(0);
static FRAMES: AtomicU32 = AtomicU32::new(0);
static PRE: AtomicU32 = AtomicU32::new(0);
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
	/// A marker of this version: the last start froze or crashed while the capture opened. Switch it off.
	SwitchOff,
	/// `gscap_gpu=off` was the guard's for another game version: the options are cleared, the GPU capture is tried again.
	Retry,
	/// A marker of another game version: dropped, the GPU capture is tried.
	StaleMarker,
}

/// What a marker file holds.
#[derive(Debug, PartialEq, Eq, Clone, Default)]
pub(crate) struct Mark {
	pub version: String,
	pub adapter: String,
}

/// The decision: the options' `gscap_gpu` (None / true = auto, false = off), the version that froze (the guard's own note), the marker
/// and the running game version.
pub(crate) fn decide(opt: Option<bool>, froze: Option<&str>, marker: Option<&Mark>, version: &str) -> Start {
	if opt == Some(false) {
		return if froze.is_some_and(|v| v != version) { Start::Retry } else { Start::Off };
	}
	match marker {
		// A marker whose version is unknown (a half-written file) counts: the writer died.
		Some(m) if m.version.is_empty() || m.version == version => Start::SwitchOff,
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
	let get = |k: &str| t.lines().find_map(|l| l.strip_prefix(k)).unwrap_or("").trim().to_string();
	Some(Mark { version: get("version="), adapter: get("adapter=") })
}

fn write_marker(dir: &Path, version: &str, adapter: &str, driver: &str) {
	let _ = std::fs::create_dir_all(dir);
	let text = format!("version={version}\nadapter={adapter}\ndriver={driver}\ntime={}\n", stamp(now_secs()));
	let _ = std::fs::write(marker_path(dir), text);
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
	let file = dir.join("pc_options.txt");
	let mut o = PcOptions::load(&file);
	let marker = read_marker(dir);
	let s = decide(o.gscap_gpu, o.gscap_froze.as_deref(), marker.as_ref(), version);
	match s {
		Start::Gpu => {}
		Start::Off => {
			remove_marker(dir);
			eprintln!("gscap guard: the GPU capture is off (pc_options.txt gscap_gpu=off{})", o.gscap_note.as_deref().map(|n| format!(" {n}")).unwrap_or_default());
		}
		Start::SwitchOff => {
			let adapter = marker.as_ref().map(|m| m.adapter.clone()).filter(|a| !a.is_empty()).unwrap_or_else(|| "an unknown adapter".into());
			o.gscap_gpu = Some(false);
			o.gscap_note = Some(format!("(auto: start on {adapter} froze {})", stamp(secs).split(' ').next().unwrap_or("")));
			o.gscap_froze = Some(version.to_string());
			let saved = o.store(&file).is_ok();
			remove_marker(dir);
			eprintln!("gscap guard: the last start froze or crashed while the GPU capture opened ({adapter}): the GPU capture is OFF for this start; pc_options.txt gscap_gpu=off {}", if saved { "saved" } else { "could not be saved" });
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

/// The step that may freeze begins (`gpu_shared::acquire`): the marker is written first. Then the test hooks.
pub(crate) fn begin() {
	if ACTIVE.load(Relaxed) && !ARMED.swap(true, Relaxed) {
		let _ = SINCE.set(Instant::now());
		let w = WINDOW.lock().unwrap_or_else(|e| e.into_inner()).clone();
		write_marker(&crate::retail::memcard::player_dir(), &version(), if w.is_empty() { "unknown" } else { &w }, "");
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
		write_marker(&crate::retail::memcard::player_dir(), &version(), &describe(info), &format!("{} {}", info.driver, info.driver_info));
	}
}

/// The capture could not open (no GPU, device refused): that start is not frozen, the marker goes.
pub(crate) fn cancel() {
	if ARMED.swap(false, Relaxed) {
		remove_marker(&crate::retail::memcard::player_dir());
	}
}

/// The game took the opened GPU capture over (draw/gpu.rs `Gpu::take`).
pub(crate) fn took_over() {
	if ARMED.load(Relaxed) && !TOOK.swap(true, Relaxed) {
		eprintln!("gscap guard: the GPU capture took over after {:.1} s ({} frames presented meanwhile)", SINCE.get().map_or(0.0, |t| t.elapsed().as_secs_f64()), PRE.load(Relaxed));
	}
}

/// A field ended on the GPU capture (gscap.rs `run`).
pub(crate) fn field_done() {
	if TOOK.load(Relaxed) && ARMED.load(Relaxed) && FIELDS.fetch_add(1, Relaxed) + 1 == FIELDS_NEEDED {
		proven();
	}
}

/// A frame was presented (viewer/handler/frame_start.rs `frame_begin`, main thread).
pub(crate) fn frame() {
	if !ARMED.load(Relaxed) {
		return;
	}
	if TOOK.load(Relaxed) {
		if FRAMES.fetch_add(1, Relaxed) + 1 == FRAMES_NEEDED {
			proven();
		}
	} else if PRE.fetch_add(1, Relaxed) + 1 == PRE_NOTE {
		eprintln!("gscap guard: {PRE_NOTE} frames presented while the GPU capture is still opening ({:.1} s): the window is alive", SINCE.get().map_or(0.0, |t| t.elapsed().as_secs_f64()));
	}
}

/// Both counters reached their number: the marker goes.
fn proven() {
	if FIELDS.load(Relaxed) >= FIELDS_NEEDED && FRAMES.load(Relaxed) >= FRAMES_NEEDED && ARMED.swap(false, Relaxed) {
		remove_marker(&crate::retail::memcard::player_dir());
		eprintln!("gscap guard: the start is alive ({} fields drawn on the GPU, {} frames presented since it took over): marker removed", FIELDS.load(Relaxed), FRAMES.load(Relaxed));
	}
}

/// The game ends on its own (the window's close, the end of `main`): a start that was only slow leaves no marker.
pub(crate) fn quit() {
	if ARMED.swap(false, Relaxed) {
		remove_marker(&crate::retail::memcard::player_dir());
		eprintln!("gscap guard: normal quit before the start was proven: marker removed");
	}
}

/// The text of the one-time note.
fn note_text() -> &'static str {
	"The last start froze while the sharp HUD (the GPU capture) opened on your graphics card, so it is switched off now. The game uses the original HUD (the same picture as before) and runs normally.\n\nIt is tried again after the next game update. Details are in player/log.txt."
}

/// A short box on a thread of its own (the game does not wait for it); tests only log.
fn tell_player() {
	eprintln!("gscap guard: note to the player: {}", note_text().lines().next().unwrap_or(""));
	if !crate::rtss_crash::test_mode() {
		let _ = std::thread::Builder::new().name("gscap note".into()).spawn(|| info_box(note_text()));
	}
}

#[cfg(windows)]
fn info_box(text: &str) {
	#[link(name = "user32")]
	unsafe extern "system" {
		fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
	}
	let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
	// MB_OK | MB_ICONINFORMATION | MB_SETFOREGROUND | MB_TOPMOST.
	// SAFETY: NUL-terminated UTF-16 strings that outlive the call.
	unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide("Ratchet & Clank (rac-rs)").as_ptr(), 0x30 | 0x1_0000 | 0x4_0000) };
}

#[cfg(not(windows))]
fn info_box(_text: &str) {}

#[cfg(test)]
mod tests {
	use super::*;

	fn mark(v: &str) -> Mark {
		Mark { version: v.into(), adapter: "AMD Radeon RX 9070 XT".into() }
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
		assert_eq!(decide(None, None, None, v), Start::Gpu);
		assert_eq!(decide(Some(true), None, None, v), Start::Gpu);
		assert_eq!(decide(None, None, Some(&mark(v)), v), Start::SwitchOff);
		assert_eq!(decide(Some(true), None, Some(&mark(v)), v), Start::SwitchOff);
		assert_eq!(decide(None, None, Some(&mark("rac-rs v0.3.1")), v), Start::StaleMarker);
		assert_eq!(decide(None, None, Some(&mark("")), v), Start::SwitchOff, "a half-written marker counts");
		// option off: the player's own (no note) stays; the guard's stays for its version and retries for a newer one
		assert_eq!(decide(Some(false), None, None, v), Start::Off);
		assert_eq!(decide(Some(false), None, Some(&mark(v)), v), Start::Off);
		assert_eq!(decide(Some(false), Some(v), None, v), Start::Off);
		assert_eq!(decide(Some(false), Some("rac-rs v0.3.1"), None, v), Start::Retry);
		assert_eq!(decide(Some(false), None, None, "rac-rs v9"), Start::Off, "off without the guard's note never retries");
	}

	#[test]
	fn a_frozen_start_switches_off_once_and_keeps_the_other_keys() {
		let d = dir("freeze");
		let v = "rac-rs v0.3.5";
		std::fs::write(d.join("pc_options.txt"), "version=1\ncamera=modern\nsome_future_key=7\n").unwrap();
		write_marker(&d, v, "AMD Radeon RX 9070 XT", "radv");
		assert_eq!(read_marker(&d).unwrap(), mark(v));
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
		let d = dir("calm");
		assert_eq!(start_up(&d, "v", 0), Start::Gpu);
		assert!(!d.join("pc_options.txt").exists(), "nothing is written for a calm start");
		write_marker(&d, "rac-rs v0.3.1", "X", "");
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
