//! RivaTuner (RTSS) + Vulkan crash box. With RivaTuner running (its `RTSSHooks64.dll` injected into the game) and NVIDIA driver 621.01,
//! the Vulkan renderer crashes a few seconds after the start (access violation in `nvoglv64.dll`, user test 2026-10-22), also with the
//! RTSS Vulkan layer switched off (`rtss_layer.rs`); DirectX 12 works. Not seen since dev s97 (one shared Vulkan device): 2026-10-23,
//! 10 hidden Vulkan starts with RTSS running and the user's play session on dev s100 (counter on screen) were fine; the box stays as a
//! safety net. A rare case, so no automatic switch: the crash filter
//! (`runlog.rs` `win::crashed`) and, once, the start-up watchdog (60 s without an answer from the open window) call in here, and the
//! player gets a Yes/No box offering `renderer=dx12` in `pc_options.txt` (every other key stays). A crash asks only when the
//! faulting module is RivaTuner's hook DLL or the NVIDIA Vulkan driver (`culprit`); any other crash gets no box and no write. The
//! crash box runs on the crashed thread itself: a new thread inside the filter would wait for the loader lock, which that thread
//! may hold, and the filter would hang. A second crash meanwhile: a fault inside the box returns; any other crashing thread (whatever
//! its module or exception code, a stack overflow too) waits for the answer (`crash_entry`, `park_until`, at most `PARK_STEPS` x 50 ms
//! = 10 minutes, then it goes on and the process ends as it would have), so the process does not end under an unanswered box. The
//! freeze box runs on its own thread (the watchdog stays free) and is not offered while a crash box is up (`freeze_due`). The answer
//! is written through the writer of the game's own option saves (`pcext/renderer_switch.rs`).
//! Tests (hidden window, `RAC_TEST_WINDOW` / `RAC_RETAIL_SMOKE` / `RAC_TEST_CRASH` set): no box, only the run-log lines.
//! `RAC_TEST_CRASH=rtss` pretends RTSS is loaded and the crash is in its module (and crashes like `=1`); `RAC_TEST_RTSS_ANSWER=yes|no`
//! answers the box (tests only).
use std::sync::atomic::{AtomicBool, Ordering};

/// The window's graphics API is Vulkan (set when the GPU is opened; the player's choice until the adapter is known).
static VULKAN: AtomicBool = AtomicBool::new(true);
/// `RAC_TEST_CRASH=rtss`: act as if RTSSHooks64.dll were loaded.
static PRETEND: AtomicBool = AtomicBool::new(false);
/// The freeze box was offered already.
static FROZEN_ASKED: AtomicBool = AtomicBool::new(false);
/// The crash box's state.
struct Gate {
	/// A crash box is up (or was): a second crash meanwhile, on any thread or inside the box itself, shows no second box.
	boxed: AtomicBool,
	/// The crash box was answered: a second crashing thread that waits for it goes on.
	done: AtomicBool,
}
static GATE: Gate = Gate { boxed: AtomicBool::new(false), done: AtomicBool::new(false) };
/// How long a second crashing thread waits for the box: 50 ms steps, 10 minutes. A box that cannot be answered (the loader lock held
/// by the waiting thread itself) must not turn "the process ends" into "the process hangs unseen".
const PARK_STEPS: u32 = 12_000;
thread_local! {
	/// This thread shows the crash box (a fault inside the box comes back into the filter on the same thread).
	static BOXING_HERE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// EXCEPTION_STACK_OVERFLOW: too little stack is left for a box.
const STACK_OVERFLOW: u32 = 0xC000_00FD;
/// A main thread that used more CPU than this over the 60 s stall was busy (a slow start), not blocked.
const BUSY_CPU_SECS: f64 = 10.0;

/// `viewer/gpu/renderer.rs`: the API of the window (before the instance is made, then the adapter's).
pub fn note_backend(vulkan: bool) {
	VULKAN.store(vulkan, Ordering::Relaxed);
}

/// `runlog.rs`, `RAC_TEST_CRASH=rtss`.
pub fn pretend_loaded() {
	PRETEND.store(true, Ordering::Relaxed);
}

/// What to do for a crash or a freeze.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Action {
	Nothing,
	/// A test run: the log lines only.
	LogOnly,
	/// A test run with `RAC_TEST_RTSS_ANSWER`: the answer (true = Yes) without a box.
	Auto(bool),
	Ask,
}

/// The faulting module of a crash that the box may blame: RivaTuner's hook DLL or the NVIDIA Vulkan driver (file name only).
fn culprit(file: &str) -> bool {
	PRETEND.load(Ordering::Relaxed) || ["RTSSHooks64.dll", "nvoglv64.dll"].iter().any(|m| file.eq_ignore_ascii_case(m))
}

/// The one-time claim of `flag`: true for the first caller only.
fn claim(flag: &AtomicBool) -> bool {
	!flag.swap(true, Ordering::SeqCst)
}

/// What a crash that wants the box does.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Role {
	/// The first crash: shows the box.
	Box,
	/// A fault inside the box (same thread): no second box, hands on.
	Return,
	/// Another thread crashed while the box is up: it waits for the answer, so the process does not end under an unanswered box.
	Park,
}

/// `claimed`: this crash got `GATE.boxed`; `boxing_here`: this thread shows the box already.
fn crash_role(claimed: bool, boxing_here: bool) -> Role {
	match (claimed, boxing_here) {
		(true, _) => Role::Box,
		(false, true) => Role::Return,
		(false, false) => Role::Park,
	}
}

/// Wait (`pause` between looks, at most `steps` of them) until `done` is set: the crashed thread of a second crash waits for the first
/// box's answer. True = answered, false = the bound ran out (the crash then goes on as before).
fn park_until(done: &AtomicBool, pause: impl Fn(), steps: u32) -> bool {
	for _ in 0..steps {
		if done.load(Ordering::SeqCst) {
			return true;
		}
		pause();
	}
	done.load(Ordering::SeqCst)
}

/// What the crash filter does with a crash on entry (before the module / code decide anything).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Entry {
	/// A box is up and this is another thread: wait for the answer, whatever the module or exception code. `say`: a log line first
	/// (not after a stack overflow: too little stack is left).
	Park { say: bool },
	/// A stack overflow with no box up, or on the box's own thread: nothing.
	Skip,
	/// Go on to `handle` (the first crash, or a fault inside the box on its own thread).
	Handle,
}

/// `boxed`: a crash box is up or was; `boxing_here`: this thread shows it.
fn crash_entry(code: u32, boxed: bool, boxing_here: bool) -> Entry {
	match (boxed && !boxing_here, wants_box(code)) {
		(true, wants) => Entry::Park { say: wants },
		(false, false) => Entry::Skip,
		(false, true) => Entry::Handle,
	}
}

/// `at_fault`: a freeze (no faulting module), or a crash in a `culprit` module.
fn decide(rtss: bool, vulkan: bool, at_fault: bool, test_mode: bool, answer: Option<&str>) -> Action {
	match (rtss && vulkan && at_fault, answer, test_mode) {
		(false, ..) => Action::Nothing,
		(true, Some("yes"), _) => Action::Auto(true),
		(true, Some("no"), _) => Action::Auto(false),
		(true, _, true) => Action::LogOnly,
		(true, _, false) => Action::Ask,
	}
}

pub(crate) fn test_mode() -> bool {
	["RAC_TEST_WINDOW", "RAC_RETAIL_SMOKE", "RAC_TEST_CRASH"].iter().any(|k| std::env::var_os(k).is_some())
}

#[cfg(windows)]
fn rtss_loaded() -> bool {
	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn GetModuleHandleA(name: *const u8) -> *mut std::ffi::c_void;
	}
	// SAFETY: a NUL-terminated name; the call only looks the module up.
	PRETEND.load(Ordering::Relaxed) || !unsafe { GetModuleHandleA(b"RTSSHooks64.dll\0".as_ptr()) }.is_null()
}

#[cfg(not(windows))]
fn rtss_loaded() -> bool {
	PRETEND.load(Ordering::Relaxed)
}

/// The box's text; `crash` false = the window froze at the start.
fn text(crash: bool) -> String {
	format!(
		"The game {} because RivaTuner Statistics Server (RTSS) is running. With current NVIDIA drivers RivaTuner only works with the DirectX 12 renderer.\n\nSwitch the renderer to DirectX 12 now? (You can change it later in Options > Graphics > Screen > Renderer, or in the launcher.)",
		if crash { "crashed" } else { "is not responding, probably" }
	)
}

/// Yes/No box (true = Yes), topmost.
#[cfg(windows)]
fn ask(text: &str) -> bool {
	#[link(name = "user32")]
	unsafe extern "system" {
		fn MessageBoxW(hwnd: *mut std::ffi::c_void, text: *const u16, caption: *const u16, kind: u32) -> i32;
	}
	let wide = |s: &str| s.encode_utf16().chain(Some(0)).collect::<Vec<u16>>();
	// MB_YESNO | MB_ICONWARNING | MB_SETFOREGROUND | MB_TOPMOST; IDYES = 6.
	// SAFETY: NUL-terminated UTF-16 strings that outlive the call.
	unsafe { MessageBoxW(std::ptr::null_mut(), wide(text).as_ptr(), wide("Ratchet & Clank (rac-rs)").as_ptr(), 0x4 | 0x30 | 0x1_0000 | 0x4_0000) == 6 }
}

#[cfg(not(windows))]
fn ask(_text: &str) -> bool {
	false
}

/// `renderer=dx12` into the player's options file, every other key as it was (the way when the game's own writer is not available).
fn switch_to_dx12(path: &std::path::Path) -> std::io::Result<()> {
	use crate::retail::pcopts::PcOptions;
	let mut o = PcOptions::load(path);
	o.renderer = Some(true);
	// Its own temp name: the game's writer (`store`, `.tmp`) may be mid-save in another thread.
	o.store_via(path, ".crash.tmp")
}

/// Yes: the file gets `renderer=dx12`; the log says so. Through the game's own writer (the same lock, so no two read-modify-write
/// cycles of the file overlap and a later save of the game keeps the key); `wait` false (the crashed thread may hold that lock
/// itself): only if the lock is free, else the file is written directly (the game is ending).
fn answered_yes(log: fn(&str), wait: bool) {
	let r = crate::retail::pcext::renderer_switch::set_dx12(wait).unwrap_or_else(|| switch_to_dx12(&crate::retail::pcopts::file()));
	log(&line(&format!("RivaTuner box: Yes, renderer=dx12 {}", if r.is_ok() { "saved" } else { "could not be saved" })));
}

/// One park step.
fn nap() {
	std::thread::sleep(std::time::Duration::from_millis(50));
}

fn line(s: &str) -> String {
	format!("runlog: +{:.3} {s}\n", crate::loadlog::secs())
}

/// `fault`: the file name of the module a crash happened in; None for a freeze.
fn handle(fault: Option<&str>, log: fn(&str)) {
	const WHY: &str = "RivaTuner (RTSSHooks64.dll) is loaded and the renderer is Vulkan:";
	let crash = fault.is_some();
	let answer = std::env::var("RAC_TEST_RTSS_ANSWER").ok();
	match decide(rtss_loaded(), VULKAN.load(Ordering::Relaxed), fault.map_or(true, culprit), test_mode(), answer.as_deref()) {
		Action::Nothing => {}
		Action::LogOnly => log(&line(&format!("{WHY} would show the RivaTuner box (test run, no box)"))),
		Action::Auto(yes) => {
			log(&line(&format!("{WHY} would show the RivaTuner box (test answer {})", if yes { "Yes" } else { "No" })));
			if yes {
				answered_yes(log, !crash);
			}
		}
		Action::Ask => {
			// A second crash while the first box is up: a fault inside the box returns; another thread waits for the answer (the
			// process must not end under an unanswered box, and a second box would be worse).
			if crash {
				let role = crash_role(claim(&GATE.boxed), BOXING_HERE.with(|b| b.get()));
				if role != Role::Box {
					log(&line(&format!("{WHY} a box is up already, no second box")));
					if role == Role::Park {
						park_until(&GATE.done, nap, PARK_STEPS);
					}
					return;
				}
				BOXING_HERE.with(|b| b.set(true));
			}
			log(&line(&format!("{WHY} showing the RivaTuner box")));
			if crash {
				// The crashed thread waits for the answer (no new thread inside the filter); the filter hands on afterwards.
				if ask(&text(true)) {
					answered_yes(log, false);
				}
				GATE.done.store(true, Ordering::SeqCst);
			} else {
				// A freeze keeps the watchdog free: the box and its answer run on their own thread.
				let _ = std::thread::Builder::new().name("rtss box".into()).spawn(move || {
					if ask(&text(false)) {
						answered_yes(log, true);
					}
				});
			}
		}
	}
}

/// A stack overflow gets no box (too little stack is left for it); every other crash code may.
fn wants_box(code: u32) -> bool {
	code != STACK_OVERFLOW
}

/// The crash filter, after the crash lines are written (waits for the box; the filter hands on afterwards). `code`: the exception
/// code (a stack overflow gets no box); `module`: the file name of the module the exception happened in ("" when in none).
pub fn on_crash(code: u32, module: &str, log: fn(&str)) {
	match crash_entry(code, GATE.boxed.load(Ordering::SeqCst), BOXING_HERE.with(|b| b.get())) {
		Entry::Park { say } => {
			if say {
				log(&line("a RivaTuner box is up already, this thread waits for the answer"));
			}
			park_until(&GATE.done, nap, PARK_STEPS);
		}
		Entry::Skip => {}
		Entry::Handle => handle(Some(module), log),
	}
}

/// The freeze box asks only for a blocked main thread: RivaTuner's deadlock is a wait inside its Vulkan layer (the log of the real
/// freeze: the main thread in `vkCreateSwapchainKHR`, another thread in `vkCreateInstance`, both waiting), which uses no CPU. A slow
/// but normal start (cold shader cache, shader compiles) keeps the main thread busy. `main_cpu`: its CPU seconds over the stall (None:
/// not readable, then it asks as before).
fn main_blocked(main_cpu: Option<f64>) -> bool {
	main_cpu.is_none_or(|c| c < BUSY_CPU_SECS)
}

/// The freeze box is due: the window exists (the freeze is a window freeze; a slow start before it is not), no crash box is up (a main
/// thread that waits under it is not a second problem) and it was not asked yet.
fn freeze_due(asked: &AtomicBool, crash_box_up: bool, window_open: bool, main_cpu: Option<f64>) -> bool {
	window_open && !crash_box_up && main_blocked(main_cpu) && claim(asked)
}

/// The start-up watchdog, after a long stall of the open window or of the start (`window_open`): once, and only with a window and a
/// blocked main thread (`main_cpu`, see `main_blocked`).
pub fn on_freeze(window_open: bool, main_cpu: Option<f64>, log: fn(&str)) {
	if freeze_due(&FROZEN_ASKED, GATE.boxed.load(Ordering::SeqCst), window_open, main_cpu) {
		handle(None, log);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_box_needs_rtss_and_vulkan() {
		assert_eq!(decide(false, true, true, false, None), Action::Nothing);
		assert_eq!(decide(true, false, true, false, None), Action::Nothing, "DirectX 12 is the fix, not the case");
		assert_eq!(decide(true, true, true, false, None), Action::Ask);
	}

	#[test]
	fn a_crash_elsewhere_gets_no_box() {
		assert!(culprit("RTSSHooks64.dll") && culprit("nvoglv64.dll") && culprit("NVOGLV64.DLL"));
		for other in ["rac-rs.exe", "ntdll.dll", "KERNELBASE.dll", "nvwgf2umx.dll", "amdvlk64.dll", "some_mod_plugin.dll", ""] {
			assert!(!culprit(other), "{other}");
			assert_eq!(decide(true, true, culprit(other), false, None), Action::Nothing, "{other}: no RivaTuner wording, no renderer write");
			assert_eq!(decide(true, true, culprit(other), true, Some("yes")), Action::Nothing, "{other}");
		}
		assert_eq!(decide(true, true, culprit("nvoglv64.dll"), false, None), Action::Ask);
		assert_eq!(decide(true, true, culprit("RTSSHooks64.dll"), false, None), Action::Ask);
	}

	#[test]
	fn the_freeze_box_waits_for_the_window_and_asks_once() {
		let asked = AtomicBool::new(false);
		assert!(!freeze_due(&asked, false, false, Some(0.0)), "a slow start before the window opens is no window freeze");
		assert!(!asked.load(Ordering::Relaxed), "and it does not use up the one question");
		assert!(freeze_due(&asked, false, true, Some(0.0)));
		assert!(!freeze_due(&asked, false, true, Some(0.0)), "once");
	}

	#[test]
	fn no_freeze_box_under_an_unanswered_crash_box() {
		let asked = AtomicBool::new(false);
		assert!(!freeze_due(&asked, true, true, Some(0.0)), "the main thread waits under the crash box: no second box");
		assert!(!asked.load(Ordering::Relaxed), "and the question is kept");
		assert!(freeze_due(&asked, false, true, Some(0.0)), "without a crash box it asks as before");
	}

	#[test]
	fn a_busy_main_thread_is_a_slow_start_not_the_freeze() {
		let asked = AtomicBool::new(false);
		// 60 s stall: the RTSS deadlock waits (~0 s CPU); a cold-cache start works (tens of s CPU).
		assert!(main_blocked(Some(0.02)) && main_blocked(Some(9.9)));
		assert!(!main_blocked(Some(10.0)) && !main_blocked(Some(55.0)));
		assert!(main_blocked(None), "unreadable CPU time: ask as before");
		assert!(!freeze_due(&asked, false, true, Some(40.0)), "a busy start gets no box");
		assert!(!asked.load(Ordering::Relaxed), "and keeps the question");
		assert!(freeze_due(&asked, false, true, Some(0.1)), "the real freeze still asks");
	}

	#[test]
	fn a_second_crash_gets_no_second_box_and_a_stack_overflow_none() {
		let flag = AtomicBool::new(false);
		assert!(claim(&flag), "the first crash gets the box");
		assert!(!claim(&flag), "a crash while it is up (any thread, or inside the box) does not");
		assert!(!claim(&flag));
		assert!(wants_box(0xC000_0005) && wants_box(0x8000_0003), "an access violation or a breakpoint may get the box");
		assert!(!wants_box(STACK_OVERFLOW), "a stack overflow gets none: ~20 KB of stack is left, not even a log line");
	}

	#[test]
	fn another_thread_waits_under_the_box_and_the_same_thread_returns() {
		assert_eq!(crash_role(true, false), Role::Box);
		assert_eq!(crash_role(false, true), Role::Return, "a fault inside the box");
		assert_eq!(crash_role(false, false), Role::Park, "another thread: the process must not end under the box");
	}

	/// `park_until` on a real thread: stays while the box is unanswered, goes on at the answer (`pause` is short).
	#[test]
	fn a_parked_thread_waits_for_the_answer() {
		let done = std::sync::Arc::new(AtomicBool::new(false));
		let (tx, rx) = std::sync::mpsc::channel();
		let d = done.clone();
		let t = std::thread::spawn(move || tx.send(park_until(&d, || std::thread::sleep(std::time::Duration::from_millis(2)), 100_000)).unwrap());
		assert!(rx.recv_timeout(std::time::Duration::from_millis(150)).is_err(), "still waiting: the box is unanswered");
		done.store(true, Ordering::SeqCst);
		assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(10)), Ok(true), "answered: it goes on");
		t.join().unwrap();
	}

	#[test]
	fn the_park_is_bounded() {
		// An answer that never comes (the box thread is stuck): the park ends after `steps`, so the process can end.
		let done = std::sync::Arc::new(AtomicBool::new(false));
		let (tx, rx) = std::sync::mpsc::channel();
		let d = done.clone();
		let t = std::thread::spawn(move || tx.send(park_until(&d, || std::thread::sleep(std::time::Duration::from_millis(2)), 25)).unwrap());
		assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(10)), Ok(false), "bound reached, not answered");
		t.join().unwrap();
		assert!((6_000..=24_000).contains(&PARK_STEPS), "about 10 minutes of 50 ms steps");
	}

	#[test]
	fn a_second_thread_parks_whatever_it_crashed_in() {
		// The box is up (boxed), this is another thread: every crash waits, an unrelated module and a stack overflow too.
		assert_eq!(crash_entry(0xC000_0005, true, false), Entry::Park { say: true }, "access violation in rac-rs.exe / ntdll: no culprit, still waits");
		assert_eq!(crash_entry(0x8000_0003, true, false), Entry::Park { say: true });
		assert_eq!(crash_entry(STACK_OVERFLOW, true, false), Entry::Park { say: false }, "a stack overflow waits too, without a log line");
		// The box's own thread: a fault inside the box is handled as before, a stack overflow there is skipped.
		assert_eq!(crash_entry(0xC000_0005, true, true), Entry::Handle);
		assert_eq!(crash_entry(STACK_OVERFLOW, true, true), Entry::Skip);
		// No box up: the first crash.
		assert_eq!(crash_entry(0xC000_0005, false, false), Entry::Handle);
		assert_eq!(crash_entry(STACK_OVERFLOW, false, false), Entry::Skip);
	}

	#[test]
	fn tests_get_no_box() {
		assert_eq!(decide(true, true, true, true, None), Action::LogOnly);
		assert_eq!(decide(true, true, true, true, Some("yes")), Action::Auto(true));
		assert_eq!(decide(true, true, true, true, Some("no")), Action::Auto(false));
		assert_eq!(decide(true, true, true, true, Some("maybe")), Action::LogOnly);
		assert_eq!(decide(false, true, true, true, Some("yes")), Action::Nothing);
	}

	#[test]
	fn the_text_names_both_cases() {
		assert!(text(true).starts_with("The game crashed because RivaTuner"));
		assert!(text(false).contains("not responding"));
		assert!(text(true).contains("DirectX 12"));
	}

	#[test]
	fn yes_writes_dx12_and_keeps_the_other_keys() {
		let d = std::env::temp_dir().join(format!("rtss_crash_test_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		let f = d.join("pc_options.txt");
		std::fs::write(&f, "camera=modern\nrenderer=vulkan\nsome_future_key=7\n").unwrap();
		// The game's own saver is mid-write on its temp file (`.tmp`): the crash path must not touch it.
		std::fs::write(d.join("pc_options.txt.tmp"), "half written by the game").unwrap();
		switch_to_dx12(&f).unwrap();
		assert_eq!(std::fs::read_to_string(d.join("pc_options.txt.tmp")).unwrap(), "half written by the game");
		assert!(!d.join("pc_options.txt.crash.tmp").exists(), "its own temp file is renamed away");
		let t = std::fs::read_to_string(&f).unwrap();
		assert!(t.contains("renderer=dx12") && !t.contains("renderer=vulkan"), "{t}");
		assert!(t.contains("camera=modern") && t.contains("some_future_key=7"), "{t}");
		// no file yet: created with the one key
		let g = d.join("new").join("pc_options.txt");
		switch_to_dx12(&g).unwrap();
		assert!(std::fs::read_to_string(&g).unwrap().contains("renderer=dx12"));
		let _ = std::fs::remove_dir_all(&d);
	}
}
