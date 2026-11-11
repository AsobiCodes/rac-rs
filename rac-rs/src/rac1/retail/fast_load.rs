//! Fast travel after the title's Load Game (v0.5.1, lane fload; the same row and options key `fasttravel` as fast_travel.rs, the
//! same replay rule). Off = retail: nothing runs, nothing is written.
//!
//! [C] decomp/notes/level_transition.md section 6, "Loading a save runs the same warp": the title's Load Game calls the front
//! end's twin of FUN_002a6228 (boot ELF `FUN_00231b00`, frontbin copy `FUN_002314b8`, same data addresses): front mode word = 6,
//! first-visit cards / movies, the hold on the level loader, the target, loader state 0, warp init `FUN_00230a70` (substate 4),
//! then loop 1 until the warp-over flag: the front warp tick `FUN_0022e5b8` (frontbin `FUN_0022df70`, the same logic as the level
//! warp tick FUN_002a2ce0 on the front end's words: fade, tick, segment end, warp-over flag), its black quad, one loader step.
//! The loader, its hold, the substate and the segment count are the level warp's core words. So fast_travel.rs `act` runs it with
//! these words: black while the loader reads, the warp ended at its next tick once the reads are done, the fade back after it,
//! the warp sound hushed; the cards, the movies, the card read and the landing on the loaded planet stay retail's.
//!
//! RAM: the front end's mode word through `title_boot::front_mode` (`g`), the others through `va` (the front end's overlay data;
//! USA: mode 0x15f604, done 0x15f618, fade 0x15f43c, tick 0x18cb54, end 0x18cb60, test `usa_words`).
use super::fast_travel::{act, active, trace, Ctx, Warp};
use crate::retail::disc::va;

/// The front end's warp words (PAL; the cold-boot title and the frontbin alike): black fade (f32), warp tick, segment end,
/// warp-over flag. The tick and the end are the title scene's timeline words, reused by the warp.
const FADE: u32 = 0x15f43c;
const TICK: u32 = 0x18cad4;
const END: u32 = 0x18cae0;
const DONE: u32 = 0x15f5f8;
/// Worker: the title Load warp's state.
static FRONT_WARP: Ctx = Ctx::new("fast load");

/// The front end's warp words on the disc that runs (`code` false: `front_field` runs only while the frontbin is in memory).
fn front_warp() -> Warp {
	Warp { mode: super::title_boot::front_mode(), fade: va(FADE), done: va(DONE), words: Some((va(TICK), va(END))), code: false }
}

/// Worker, a level field (fast_travel.rs `pre_field`): the front end is gone, so a warp it still held is over (tb5 review N2: the
/// level's load replaced the frontbin before its release field ran; the hold no longer outlives it into the next title visit).
pub(super) fn level_field() {
	if FRONT_WARP.clear() {
		eprintln!("retail: fast load: the front end left memory before its release field: hold cleared");
	}
}

/// Worker, before a front-end field (`worker/field_run.rs`; `replay`: a recording feeds the pad). Off: reads nothing (the
/// RAC_SCENE_LOG trace only).
pub fn front_field(m: &mut ee_oracle::Machine, field: u32, replay: bool) {
	let w = front_warp();
	trace(m, field, &w, &FRONT_WARP, u32::MAX);
	act(m, field, active(replay), &w, &FRONT_WARP);
}

#[cfg(test)]
mod tests {
	use super::*;

	/// tb5 review N2: a level field clears a hold the front end left (its release field never ran).
	#[test]
	fn a_level_field_clears_the_front_hold() {
		assert!(!front_warp().code, "the front end's words: no code check");
		FRONT_WARP.hold_for_test();
		level_field();
		assert!(!FRONT_WARP.clear(), "the level field cleared it");
	}

	/// PAL: the words as written in the note.
	#[test]
	fn pal_words() {
		let w = front_warp();
		assert_eq!((w.mode, w.fade, w.done, w.words), (0x15f5e4, FADE, DONE, Some((TICK, END))));
	}

	/// USA: the front end's words in both front-end overlays (frontbin scope 19, the boot ELF's own part 20), agent 5's table.
	#[test]
	fn usa_words() {
		use crate::retail::disc::{segs_of, traced, SCOPE_BOOT, SCOPE_FRONT};
		let at = std::panic::Location::caller();
		super::super::disc::tests::as_usa(|| {
			assert_eq!(super::super::title_boot::front_mode(), 0x15f604, "mode");
			for sc in [SCOPE_FRONT, SCOPE_BOOT] {
				let map = |a| traced(sc, segs_of(sc), a, at);
				assert_eq!([map(FADE), map(TICK), map(END), map(DONE)], [0x15f43c, 0x18cb54, 0x18cb60, 0x15f618], "scope {sc}");
			}
		});
	}
}
