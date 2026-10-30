//! The Start skips cutscenes row (Pause -> Options -> Graphics -> System -> Start skips cutscenes: On / Off; the title's Graphics
//! page shows it too (`pcext/titlegfx.rs`)), retail side.
//!
//! Retail lets a plain Start skip a movie or an in-engine scene only in Veldin, in the New Game intro, in New Game+ and after the
//! game is beaten (decomp/notes/cutscene_skip.md); everywhere else in a normal run the player's test wants Start with L1 + R1 + L2 +
//! R2 held (the PAL and the USA disc run the same rule), and a ship takeoff / landing wants a new Triangle or Cross. GitHub #34 / #23:
//! players pressed Start and nothing happened. With this row On (the default; the options file's `skipcut=on|off`, pcopts.rs) a
//! plain Start, on the keyboard (Enter / Esc) or the controller, stands for those buttons in `input.rs` (`tick_keys` feeds the scene
//! and flight tests through `skip_feed`, `movie_poll` the movie test), so retail's own rule decides and ends the cutscene; Off is
//! retail's rule untouched (Esc and Space keep skipping, as before). The boot logo movies end on a new Start, Esc, Enter or Space too (v0.4.2, `movie_poll`; the Skip intro row cuts them before they play).
//!
//! The row's byte lives in the pcctl block, stored INVERTED (0 = On): a default RAM stays all zero, so the RAM hashes of the
//! replays do not move (`pcctl::skipcut_on`). `sync` hands a change to the options file.
use super::pcopts::PcOptions;
use std::sync::atomic::{AtomicBool, Ordering};

/// The row as last applied (the file's value until the player changes it).
static ON: AtomicBool = AtomicBool::new(true);

/// `pcext::init`: the file's choice is the row's start value.
pub fn init(opts: &PcOptions) {
	ON.store(opts.skipcut_on(), Ordering::Relaxed);
}

/// True = a plain Start skips movies, scenes and flights.
pub fn skip() -> bool {
	ON.load(Ordering::Relaxed)
}

/// The PS2 pad bits of Start (`prompts::map_xinput`).
pub const START: u16 = 0x800;

/// `Retail::tick_keys`: true when the pad bits of this field hold Start and the row is On (the keys Esc / Space stand for a skip as
/// before, `input::skip_feed`).
pub fn start_pressed(buttons: u16) -> bool {
	skip() && buttons & START != 0
}

/// Worker, every field after the page patch: a changed row goes into the options file (true = the caller saves).
pub fn sync(mem: &mut ee_oracle::mem::Mem, opts: &mut PcOptions) -> bool {
	let Some(on) = super::pcctl::skipcut_on(mem) else { return false };
	if ON.swap(on, Ordering::Relaxed) == on {
		return false;
	}
	eprintln!("retail: PC options: Start skips cutscenes {}", if on { "On" } else { "Off" });
	opts.skipcut = Some(on);
	true
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_file_sets_the_start_value_and_a_changed_row_is_saved() {
		let _g = crate::retail::pcopts::globals_guard();
		init(&PcOptions::parse("skipcut=off\n"));
		assert!(!skip());
		assert!(!start_pressed(START), "Off: Start is just Start");
		init(&PcOptions::parse(""));
		assert!(skip(), "a new player starts with the row On");
		assert!(start_pressed(START | 0x40) && !start_pressed(0x40));
		let mut mem = ee_oracle::mem::Mem::new();
		let mut o = PcOptions::default();
		assert!(!sync(&mut mem, &mut o), "page not built yet");
		// The Graphics page built on a fresh RAM with the default On: the row byte stays 0 (RAM hashes do not move) and reads On.
		let rows: Vec<[u32; 6]> = (0..15u32).map(|i| [0x100 + i, 0x200 + i, 0x300, 0x301, 0, 0]).collect();
		super::super::pcctl::build_gfx(&mut mem, super::super::pcext::BASE + 0xaa0, &rows, true, false);
		assert_eq!(mem.rd8(super::super::pcctl::SKIPB), 0, "default On = byte 0");
		assert_eq!(super::super::pcctl::skipcut_on(&mut mem), Some(true));
		assert!(!sync(&mut mem, &mut o), "no change: nothing to save");
		// The player picks Off in the menu: the byte goes to 1, sync saves `skipcut=off` once.
		mem.wr8(super::super::pcctl::SKIPB, 1);
		assert!(sync(&mut mem, &mut o) && !skip());
		assert_eq!(o.skipcut, Some(false));
		assert!(!sync(&mut mem, &mut o), "saved once");
		init(&PcOptions::parse(""));
	}
}
