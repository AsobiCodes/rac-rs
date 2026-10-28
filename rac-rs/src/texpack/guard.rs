//! Panics the texture-pack code expects and survives. A decoder that panics on a hostile pack file is caught where the file is read
//! (`repl::load_file`) and where an entry is built (`LevelRepl::run`); the game goes on with the original texture. The process-wide panic
//! hook (`main.rs`) asks `panic_expected` first, so such a panic is one log line and does not write `crash.log` or a call trace (bug
//! reports would show a crash that was none).

use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};

thread_local! {
	static EXPECTED: Cell<bool> = const { Cell::new(false) };
}

/// Whether the calling thread is inside `guarded` (read by the panic hook, on the panicking thread).
pub fn panic_expected() -> bool {
	EXPECTED.with(Cell::get)
}

/// Runs `f`; a panic inside it is `Err(())` and is marked expected for the panic hook.
pub fn guarded<R>(f: impl FnOnce() -> R) -> Result<R, ()> {
	let before = EXPECTED.with(|e| e.replace(true));
	let r = catch_unwind(AssertUnwindSafe(f));
	EXPECTED.with(|e| e.set(before));
	r.map_err(|_| ())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn a_panic_inside_guarded_is_expected_and_caught_and_the_mark_is_restored() {
		assert!(!panic_expected());
		assert_eq!(guarded(|| 3), Ok(3));
		// The hook (here the default one, printing) runs on this thread before the unwind: the flag is set then.
		let seen = Cell::new(false);
		let r = guarded(|| {
			assert!(panic_expected());
			seen.set(panic_expected());
			if seen.get() {
				panic!("expected test panic");
			}
		});
		assert!(r.is_err() && seen.get());
		assert!(!panic_expected(), "back to unmarked after the panic");
		// Nested: the outer mark stays.
		let r = guarded(|| {
			let _ = guarded(|| ());
			panic_expected()
		});
		assert_eq!(r, Ok(true));
	}
}
