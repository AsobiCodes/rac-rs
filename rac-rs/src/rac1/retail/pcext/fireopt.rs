//! The Graphics page's Fire row (realistic flames and burning buildings, lane fire; the renderer side is `engine/fire_option.rs` and
//! `viewer/firefx.rs`). Like the Fur row: Off / High / Medium / Low in the menu (byte 0 Off, 1 High, 2 Medium, 3 Low; `swap_level` turns
//! it into the level 0 Off .. 3 High that the file's `fire` key and the renderer use). It sits in the "More effects" section of both
//! Graphics pages after Fur style, and a Graphics preset sets it (gfx_presets.txt: Low off, Medium low, High high).
//!
//! The value lives outside the packed word (`pack_gfx` has no free bit): `apply_graphics` and `init` store it here and pass it on to
//! the engine (`rac_engine::fire_option::set`). A `pc_options.txt` without a `fire` key follows the missing-key rule of `furopt::rows`.
use super::*;
use std::sync::atomic::AtomicU8;

/// Graphics -> Fire (the row byte, see above).
pub const FIRE: u32 = BASE + 0x1a;

/// Text: the row label (the values reuse Off / High / Medium / Low).
pub const ID_FIRE: u32 = 0x7f29;

/// The level the Fire row asks for (0 Off .. 3 High), as last applied.
static FIRE_Q: AtomicU8 = AtomicU8::new(3);

/// `apply_graphics` / `init`: the level of the Fire row (0 Off .. 3 High); the renderer follows.
pub(super) fn set(level: u8) {
	FIRE_Q.store(level.min(3), Ordering::Relaxed);
	rac_engine::fire_option::set(level);
}

/// The Fire level (0 Off .. 3 High) of the row (the player's setting; `RAC_FIRE` only changes what the renderer uses).
pub(super) fn level() -> u8 {
	FIRE_Q.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_fire_row_is_the_eighteenth() {
		assert_eq!(GROWS[17], FIRE);
		assert_eq!(ROWS[17], [ID_FIRE, FIRE, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW]);
		assert_eq!((row_values(17), FIRE, ROWS.len()), (4, 0xB001A, 19));
		assert_eq!(DEFAULT_ROWS[17], 1, "a new player: High");
		assert_ne!(FIRE, furopt::FUR + 1, "BASE + 24 is the title stepper, not Fire");
	}

	#[test]
	fn the_level_is_clamped_and_reaches_the_engine() {
		let _turn = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		set(9);
		assert_eq!(level(), 3);
		set(0);
		assert_eq!(level(), 0);
		assert_eq!(rac_engine::fire_option::level(), 0);
		set(2);
		assert_eq!((level(), rac_engine::fire_option::level()), (2, 2));
		set(3); // back to the default: other tests read the Preset row of a new player
	}

	#[test]
	fn the_text_exists_in_five_languages() {
		let k = NEW_IDS.iter().position(|&n| n == ID_FIRE).expect("listed in NEW_IDS");
		assert!(TEXTS[k].iter().all(|t| !t.is_empty() && t.len() <= 22));
		assert_eq!((TEXTS[k][0], TEXTS[k][1], TEXTS[k][4]), (&b"Fire"[..], &b"Feu"[..], &b"Fuoco"[..]));
		assert_eq!(texts::pool_slot(k), BASE + 0x1440, "the first free slot of the third pool");
	}
}
