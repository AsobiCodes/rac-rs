//! The player's run log, `<player dir>/log.txt` (the run before it: `log.prev.txt`).
//!
//! The game may be started with no terminal to read (the application menu), so everything the game writes to stderr goes
//! into this file, and the terminal keeps a copy of it when there is one. The load steps, the freeze notes and the
//! panic's calls are in it too.
//!
//! The watchdog: once a second the main thread's CPU time, from /proc/self/task/<tid>/stat (the main thread's tid is
//! the process's pid). When it stops changing, a note with the last load step, the CPU time (so "busy" against
//! "blocked") and the named waits of every thread.
//!
//! Upstream's own file does the same with the Windows API (the stderr handle, the unhandled exception filter, the
//! thread contexts and the symbol table). This is the Linux version, with the same API, so the call sites are the
//! same: the Windows one and this one are interchangeable.
//!
//! RAC_RUN_LOG=0 turns it off.

use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// The log stops growing at this size (a runaway message must not fill the disk).
const MAX_BYTES: u64 = 16 << 20;
/// The watchdog's notes: when the main thread has not moved for each of these many seconds.
const STALL_NOTES: [u64; 7] = [10, 30, 60, 120, 300, 600, 1800];

/// True once `start` sent the game's output into the run log.
static ACTIVE: AtomicBool = AtomicBool::new(false);
/// The log file, for the lines this module writes itself (they go straight to the file: a pipe would lose them when
/// the game ends).
static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);
/// The last `loadlog::say` line.
static STEP: Mutex<String> = Mutex::new(String::new());
/// The waits in progress: id, the thread's name, what it waits for.
static WAITS: Mutex<Vec<(u64, Arc<str>, &'static str)>> = Mutex::new(Vec::new());
static NEXT_WAIT: AtomicU64 = AtomicU64::new(1);
/// The main thread's tid: on Linux it is the process's pid.
static MAIN_TID: AtomicU64 = AtomicU64::new(0);
/// The terminal's stderr, before the pipe took its place. The lines this module writes go there and not through the pipe, or
/// the log would have them twice (once from here and once from the pipe).
static ORIG: AtomicI32 = AtomicI32::new(-1);

thread_local! {
	/// This thread's name in the notes.
	static LABEL: std::cell::OnceCell<Arc<str>> = const { std::cell::OnceCell::new() };
}

/// True once `start` sent the game's output into the run log.
pub fn active() -> bool {
	ACTIVE.load(Ordering::Relaxed)
}

/// Starts the run log in `dir`: the log file, the run before it, and the game's own output into the file (the terminal
/// keeps a copy of it when there is one). Called once on the main thread, before any other thread starts.
pub fn start(dir: &std::path::Path) -> bool {
	if std::env::var("RAC_RUN_LOG").ok().as_deref() == Some("0") {
		return false;
	}
	let _ = std::fs::create_dir_all(dir);
	let path = dir.join("log.txt");
	let _ = std::fs::rename(&path, dir.join("log.prev.txt"));
	let file = match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
		Ok(f) => f,
		Err(_) => return false,
	};
	if let Ok(mut l) = LOG.lock() {
		*l = Some(file);
	}
	// The pipe the game's stderr goes into: the reader writes the log and the terminal.
	let mut fds = [0 as libc::c_int; 2];
	// SAFETY: pipe takes two valid pointers and fills them with two descriptors.
	if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
		return false;
	}
	let (read, write) = (fds[0], fds[1]);
	// The terminal's copy of stderr, before it is replaced by the pipe.
	// SAFETY: dup returns a new descriptor, or -1 when there is none.
	let orig = unsafe { libc::dup(2) };
	ORIG.store(orig, Ordering::Relaxed);
	// SAFETY: the pipe's write end is a valid descriptor; dup2 replaces fd 2 with it.
	unsafe { libc::dup2(write, 2) };
	// SAFETY: fd 2 is the pipe's write end now, so the original is not needed.
	unsafe { libc::close(write) };
	LABEL.with(|l| {
		let _ = l.set("main".into());
	});
	MAIN_TID.store(std::process::id() as u64, Ordering::Relaxed);
	ACTIVE.store(true, Ordering::Release);
	let log = path.clone();
	std::thread::Builder::new()
		.name("runlog".into())
		.spawn(move || copy_out(read, orig, log))
		.ok();
	say(&format!("rac-rs run log: {} started (local time {})", crate::app_name(), local_time()));
	say(&format!("system: {}", system()));
	say(&format!(
		"exe: {}",
		std::env::current_exe().map_or_else(|e| e.to_string(), |p| p.display().to_string())
	));
	let _ = std::thread::Builder::new().name("runlog-watch".into()).spawn(move || watch(path));
	true
}

/// A line of the run log: to the file (so it is there even when the game dies) and to the game's stderr (so the terminal
/// shows it, and the pipe copies it into the log as well).
fn say(text: &str) {
	if let Ok(mut l) = LOG.lock() {
		if let Some(f) = l.as_mut() {
			let _ = writeln!(f, "{text}");
			let _ = f.flush();
		}
	}
	// The terminal's copy, straight to its descriptor: the pipe's write end is fd 2 now, and the pipe would write the line
	// into the log a second time.
	let orig = ORIG.load(Ordering::Relaxed);
	if orig >= 0 {
		// SAFETY: the descriptor is open and points at the terminal.
		unsafe { libc::write(orig, text.as_ptr() as *const libc::c_void, text.len()) };
		// SAFETY: as above, one byte.
		unsafe { libc::write(orig, b"\n".as_ptr() as *const libc::c_void, 1) };
	}
}

/// The reader: the game's output into the log file and, when the terminal was there, into it too.
fn copy_out(read: libc::c_int, orig: libc::c_int, log: std::path::PathBuf) {
	let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&log) else { return };
	let mut buf = [0u8; 4096];
	loop {
		// SAFETY: reading into a valid buffer, from a descriptor this thread owns.
		let n = unsafe { libc::read(read, buf.as_mut_ptr() as *mut libc::c_void, buf.len()) };
		if n <= 0 {
			break;
		}
		let _ = file.write_all(&buf[..n as usize]);
		let _ = file.flush();
		if orig >= 0 {
			// SAFETY: the original descriptor is still open and points at the terminal.
			unsafe { libc::write(orig, buf.as_ptr() as *const libc::c_void, n as usize) };
		}
	}
}

/// The last line of a run that ended on its own (a log without it: the game was closed from outside or crashed hard).
pub fn end() {
	if active() {
		say(&format!("runlog: +{:.3} the game ended", crate::loadlog::secs()));
	}
}

/// The calling thread's calls: the panic hook (where the game panicked).
pub fn note_calls() {
	if active() {
		say(&format!(
			"runlog: calls of thread {}:\n{}",
			std::thread::current().name().unwrap_or("unnamed"),
			std::backtrace::Backtrace::force_capture()
		));
	}
}

/// Remembers a load step (`loadlog::say`) for the watchdog's notes.
pub fn note_step(line: &str) {
	if active() {
		if let Ok(mut s) = STEP.lock() {
			s.clear();
			s.push_str(line);
		}
	}
}

/// The game window. Linux has no equivalent of the Windows "does the window answer" message, so this does nothing: the
/// watchdog watches the main thread's CPU time instead.
pub fn set_window(_w: &winit::window::Window) {}

/// While the guard lives, the watchdog's notes list `what` as a wait of this thread. Only for waits that can last (a
/// frame's vsync, another thread's result), not for spin loops: it takes a lock.
pub fn waiting(what: &'static str) -> Waiting {
	if !active() {
		return Waiting(0);
	}
	let thread = LABEL.with(|l| l.get_or_init(|| std::thread::current().name().unwrap_or("unnamed").into()).clone());
	let id = NEXT_WAIT.fetch_add(1, Ordering::Relaxed);
	if let Ok(mut w) = WAITS.lock() {
		w.push((id, thread, what));
	}
	Waiting(id)
}

pub struct Waiting(u64);

impl Drop for Waiting {
	fn drop(&mut self) {
		if self.0 != 0 {
			if let Ok(mut w) = WAITS.lock() {
				w.retain(|e| e.0 != self.0);
			}
		}
	}
}

/// "main: the present thread ...; gs-gpu: ..." (every wait in progress), or "none named".
fn waits_now() -> String {
	let w = WAITS.lock().map(|w| w.iter().map(|(_, t, what)| format!("{t}: {what}")).collect::<Vec<_>>()).unwrap_or_default();
	if w.is_empty() { "none named".into() } else { w.join("; ") }
}

/// The watchdog: once a second, the main thread's CPU time. When it stops changing, a note with the last load step,
/// the CPU time since it last moved and the named waits.
fn watch(path: std::path::PathBuf) {
	let mut cpu0 = cpu_secs(MAIN_TID.load(Ordering::Relaxed));
	let mut moved = std::time::Instant::now();
	let mut notes = 0usize;
	loop {
		std::thread::sleep(std::time::Duration::from_secs(1));
		if std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_BYTES) {
			say(&format!("runlog: the log reached {} MB: the rest of this run is not written", MAX_BYTES >> 20));
			return;
		}
		let cpu = cpu_secs(MAIN_TID.load(Ordering::Relaxed));
		if cpu != cpu0 {
			cpu0 = cpu;
			moved = std::time::Instant::now();
			continue;
		}
		let s = moved.elapsed().as_secs();
		if notes < STALL_NOTES.len() && s >= STALL_NOTES[notes] {
			notes += 1;
			let step = STEP.lock().map(|g| g.clone()).unwrap_or_default();
			say(&format!(
				"runlog: +{:.3} the main thread has not moved for {s} s; last load step: {}; main thread CPU meanwhile: {}; waits: {}",
				crate::loadlog::secs(),
				if step.is_empty() { "none" } else { step.as_str() },
				cpu_note(cpu0),
				waits_now()
			));
		}
	}
}

/// A thread's CPU time in seconds, from /proc/self/task/<tid>/stat. None when it cannot be read.
fn cpu_secs(tid: u64) -> Option<f64> {
	let stat = std::fs::read_to_string(format!("/proc/self/task/{tid}/stat")).ok()?;
	// The fields after the comm (in parentheses) are the state and then the times: utime and stime are the twelfth and the
	// thirteenth of them.
	let rest = stat.rsplit_once(") ")?.1;
	let f: Vec<&str> = rest.split_whitespace().collect();
	let utime: u64 = f.get(11)?.parse().ok()?;
	let stime: u64 = f.get(12)?.parse().ok()?;
	// The kernel's clock ticks are 100 a second.
	Some((utime + stime) as f64 / 100.0)
}

/// The main thread's CPU time as a note: how much it used since it last moved (near the wall time: busy; near zero:
/// blocked).
fn cpu_note(since: Option<f64>) -> String {
	match (since, cpu_secs(MAIN_TID.load(Ordering::Relaxed))) {
		(Some(a), Some(b)) => format!("{:.1} s", b - a),
		_ => "unknown".into(),
	}
}

/// The local time, for the log's first line.
fn local_time() -> String {
	// SAFETY: localtime_r fills the caller's struct with the broken-down local time; a zeroed tm is a valid start.
	let mut tm: libc::tm = unsafe { std::mem::zeroed() };
	// SAFETY: time writes the current time through the pointer, or returns it when the pointer is null.
	let now = unsafe { libc::time(std::ptr::null_mut()) };
	// SAFETY: the time pointer and the struct are valid.
	unsafe { libc::localtime_r(&now, &mut tm) };
	format!(
		"{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
		tm.tm_year + 1900,
		tm.tm_mon + 1,
		tm.tm_mday,
		tm.tm_hour,
		tm.tm_min,
		tm.tm_sec
	)
}

/// The system: the distribution, the kernel and the processors.
fn system() -> String {
	let distro = std::fs::read_to_string("/etc/os-release")
		.ok()
		.and_then(|s| s.lines().find(|l| l.starts_with("PRETTY_NAME=")).map(|l| l.trim_start_matches("PRETTY_NAME=").trim_matches('"').to_string()))
		.unwrap_or_default();
	let kernel = std::fs::read_to_string("/proc/sys/kernel/osrelease").map(|s| s.trim().to_string()).unwrap_or_default();
	let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0);
	format!("{distro} (kernel {kernel}, {cpus} processors)")
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_cpu_time_of_a_thread_is_read() {
		// The test thread's own tid is the process's pid on Linux.
		let tid = std::process::id() as u64;
		let cpu = cpu_secs(tid).expect("the CPU time of this thread");
		assert!(cpu >= 0.0);
		// A thread that does not exist has none.
		assert!(cpu_secs(u64::MAX).is_none());
	}

	#[test]
	fn the_local_time_is_a_timestamp() {
		let t = local_time();
		assert_eq!(t.len(), 19, "{t}");
		assert_eq!(t.as_bytes()[4], b'-');
		assert_eq!(t.as_bytes()[13], b':');
	}

	#[test]
	fn the_system_line_names_the_kernel_and_the_processors() {
		let s = system();
		assert!(s.contains("kernel "), "{s}");
		assert!(s.contains("processors"), "{s}");
	}
}
