//! The clean exit of the game ("Quit to Desktop", v0.5 menu lane; plan `scratch_perf_menu/QUIT_PLAN.md`). Game-neutral.
//!
//! Any thread may ASK to quit (`request`: the pause menu's worker thread, a menu button on the main thread); only the main thread
//! can end the program, because the winit event loop (`ActiveEventLoop::exit`) exists only there. So the ask is a flag, and the
//! window handler polls it (`take`, once per event and per redraw) and runs `quit_app`. The window's own close (the X button,
//! Alt+F4) goes through the same `quit_app`, so every way out saves the same things in the same order:
//! 1. `achievements::flush_now`: the unlock file is written NOW (the 10 s debounce of counters is skipped) and, if the worker is in
//!    the middle of a write, waited for (bounded), so the process never ends inside that write;
//! 2. `gscap_guard::quit`: a normal quit before the start was proven removes the start marker;
//! 3. the loop is told to exit (`exit`, a closure so a test needs no window); `run` then flushes the frame CSV and returns.
//! PC options are saved when a row changes, not here.
use std::sync::atomic::{AtomicBool, Ordering};

/// Somebody asked to quit and the main thread has not run `quit_app` yet.
static REQUEST: AtomicBool = AtomicBool::new(false);

/// Any thread: asks the main thread to quit (idempotent; a held button asks every field).
pub fn request() {
	REQUEST.store(true, Ordering::SeqCst);
}

/// Whether a quit is asked and not yet taken (tests, and a UI that wants to stop drawing).
pub fn requested() -> bool {
	REQUEST.load(Ordering::SeqCst)
}

/// Main thread: takes the ask (one-shot: true once per `request`, however many were made).
pub fn take() -> bool {
	REQUEST.swap(false, Ordering::SeqCst)
}

/// The one quit path: flush the achievements, tell the capture guard, then `exit` the event loop.
pub fn quit_app(exit: impl FnOnce()) {
	run_in_order(|| drop(crate::achievements::flush_now()), crate::gscap_guard::quit, exit);
}

/// The order of `quit_app` on its own (the steps are closures so the test can record them).
pub(crate) fn run_in_order(flush: impl FnOnce(), guard: impl FnOnce(), exit: impl FnOnce()) {
	flush();
	guard();
	exit();
}

#[cfg(test)]
pub(crate) static TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
mod tests {
	use super::*;
	use std::cell::RefCell;

	#[test]
	fn the_ask_is_one_shot_from_any_thread() {
		let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		assert!(!take() && !requested(), "nothing asked yet");
		std::thread::spawn(request).join().unwrap();
		std::thread::spawn(request).join().unwrap();
		assert!(requested());
		assert!(take(), "the ask arrives on the main thread");
		assert!(!take() && !requested(), "once: two asks are one quit");
	}

	#[test]
	fn the_flush_comes_before_the_guard_and_the_exit() {
		let log = RefCell::new(Vec::new());
		run_in_order(|| log.borrow_mut().push("flush"), || log.borrow_mut().push("guard"), || log.borrow_mut().push("exit"));
		assert_eq!(*log.borrow(), ["flush", "guard", "exit"], "achievements are on disk before the loop is told to end");
	}

	#[test]
	fn quit_app_exits_even_without_an_achievements_hub() {
		let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		let done = RefCell::new(false);
		quit_app(|| *done.borrow_mut() = true);
		assert!(*done.borrow(), "no hub (title, other games): flush_now is a no-op and the exit still runs");
	}
}
