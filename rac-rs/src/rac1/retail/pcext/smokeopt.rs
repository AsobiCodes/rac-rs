//! The Graphics page's Smoke row (volumetric smoke; the renderer side is `engine/smoke_option.rs` and `viewer/volsmoke.rs`). The same
//! shape as `fireopt.rs`: Off / High / Medium / Low in the menu (byte 0 Off, 1 High, 2 Medium, 3 Low, `swap_level` to the level 0 Off ..
//! 3 High), after the Fire row in "More effects", set by the Graphics presets, passed on with `rac_engine::smoke_option::set`.
use super::*;
use std::sync::atomic::AtomicU8;

/// Graphics -> Smoke (the row byte, see above).
pub const SMOKE: u32 = BASE + 0x1b;

/// Text: the row label (the values reuse Off / High / Medium / Low).
pub const ID_SMOKE: u32 = 0x7f2a;

/// The level the Smoke row asks for (0 Off .. 3 High), as last applied.
static SMOKE_Q: AtomicU8 = AtomicU8::new(3);

/// `apply_graphics` / `init`: the level of the Smoke row (0 Off .. 3 High); the renderer follows.
pub(super) fn set(level: u8) {
	SMOKE_Q.store(level.min(3), Ordering::Relaxed);
	rac_engine::smoke_option::set(level);
}

/// The Smoke level (0 Off .. 3 High) of the row (the player's setting; `RAC_SMOKE` only changes what the renderer uses).
pub(super) fn level() -> u8 {
	SMOKE_Q.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_smoke_row_is_the_nineteenth() {
		assert_eq!(GROWS[18], SMOKE);
		assert_eq!(ROWS[18], [ID_SMOKE, SMOKE, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW]);
		assert_eq!((row_values(18), SMOKE, ROWS.len()), (4, 0xB001B, 19));
		assert_eq!(DEFAULT_ROWS[18], 1, "a new player: High");
		assert_eq!(SMOKE, fireopt::FIRE + 1);
	}

	#[test]
	fn the_level_is_clamped_and_reaches_the_engine() {
		let _turn = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		set(9);
		assert_eq!(level(), 3);
		set(1);
		assert_eq!((level(), rac_engine::smoke_option::level()), (1, 1));
		set(0);
		assert_eq!(rac_engine::smoke_option::level(), 0);
		set(3); // back to the default: other tests read the Preset row of a new player
	}

	#[test]
	fn the_text_exists_in_five_languages() {
		let k = NEW_IDS.iter().position(|&n| n == ID_SMOKE).expect("listed in NEW_IDS");
		assert!(TEXTS[k].iter().all(|t| !t.is_empty() && t.len() <= 22));
		assert_eq!((TEXTS[k][0], TEXTS[k][3], TEXTS[k][2]), (&b"Smoke"[..], &b"Humo"[..], &b"Rauch"[..]));
		assert_eq!(texts::pool_slot(k), BASE + 0x1480);
	}
}
