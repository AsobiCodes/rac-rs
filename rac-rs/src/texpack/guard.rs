//! Panics the texture-pack code expects and survives. A decoder that panics on a hostile pack file is caught where the file is read
//! (`repl::load_file`) and where an entry is built (`LevelRepl::run`); the game goes on with the original texture. The process-wide panic
//! hook (`main.rs`) asks `take_expected` first, so such a panic is one log line and does not write `crash.log` or a call trace (bug
//! reports would show a crash that was none). `guarded_with` does the same for other caught panics (the overlay helper, viewer/gpu/ovwrite.rs)
//! with their own log line.

use std::cell::Cell;
use std::panic::{catch_unwind, AssertUnwindSafe};

/// The hook's log line for a panic caught by `guarded` (the texture packs' own).
const TEXPACK_NOTE: &str = "texpack: a pack file made the decoder panic (caught, the original texture stays)";

thread_local! {
	static EXPECTED: Cell<bool> = const { Cell::new(false) };
	static NOTE: Cell<&'static str> = const { Cell::new(TEXPACK_NOTE) };
}

/// Whether the calling thread is inside `guarded` (a read-only look; the panic hook uses `take_expected`).
pub fn panic_expected() -> bool {
	EXPECTED.with(Cell::get)
}

/// The panic hook's question: is this panic the expected one? Clears the mark, so a second panic on this thread before `guarded_with`
/// returns (a Drop panicking during the unwind, which aborts) is reported as a real crash; `guarded_with` restores the mark after the catch.
pub fn take_expected() -> bool {
	EXPECTED.with(|e| e.replace(false))
}

/// What the panic hook logs for an expected panic on the calling thread (the note given to `guarded_with`).
pub fn expected_note() -> &'static str {
	NOTE.with(Cell::get)
}

/// Runs `f`; a panic inside it is `Err(())` and is marked expected for the panic hook.
pub fn guarded<R>(f: impl FnOnce() -> R) -> Result<R, ()> {
	guarded_with(TEXPACK_NOTE, f)
}

/// Like `guarded`, with the hook's log line `note` (it prints `note: <panic message>`; no crash.log, no call trace).
pub fn guarded_with<R>(note: &'static str, f: impl FnOnce() -> R) -> Result<R, ()> {
	let before = (EXPECTED.with(|e| e.replace(true)), NOTE.with(|n| n.replace(note)));
	let r = catch_unwind(AssertUnwindSafe(f));
	EXPECTED.with(|e| e.set(before.0));
	NOTE.with(|n| n.set(before.1));
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

	#[test]
	fn a_note_is_given_back_while_guarded_and_restored_after() {
		assert_eq!(expected_note(), TEXPACK_NOTE);
		let r = guarded_with("other", || (panic_expected(), expected_note()));
		assert_eq!(r, Ok((true, "other")));
		assert_eq!(expected_note(), TEXPACK_NOTE);
	}

	#[test]
	fn the_hook_takes_the_mark_and_guarded_gives_it_back() {
		let r = guarded(|| (take_expected(), panic_expected()));
		assert_eq!(r, Ok((true, false)), "taken once: a second panic in the same unwind is a real one");
		assert!(!panic_expected());
		let r = guarded(|| {
			let _ = guarded(|| take_expected());
			panic_expected()
		});
		assert_eq!(r, Ok(true), "the outer mark is back after the inner catch");
	}
}
