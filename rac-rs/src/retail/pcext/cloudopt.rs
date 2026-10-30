//! The Graphics page's Volumetric clouds row (dynamic 3D sky clouds, viewer/volclouds.rs; lane ycld, 2026-10-24): Classic / High /
//! Medium / Low like the Fog row (menu byte: 0 Classic = the retail sky only, 1 High, 2 Medium, 3 Low; `swap_level` turns it into
//! the level 0 Classic .. 3 High that the file's `clouds` key and the renderer use). Like the other effects it only acts with
//! Lighting Enhanced. Like Fog it is on the Effects section of both Graphics pages (pause and title), and its level is kept in
//! an AtomicU8 because `pack_gfx` has no free bit: `apply_graphics` and `init` store it, `graphics()` reads it into `Graphics::clouds_q`.
use super::*;
use std::sync::atomic::AtomicU8;

/// Graphics -> Volumetric clouds (the row byte). A new player has High (like the other effect rows), which shows with Lighting Enhanced.
pub const CLOUDS: u32 = BASE + 22;

/// The level the row asks for (0 Classic .. 3 High), as last applied.
static CLOUDS_Q: AtomicU8 = AtomicU8::new(3);

/// `apply_graphics` / `init`: the level of the row (0 Classic .. 3 High).
pub(super) fn set(level: u8) {
	CLOUDS_Q.store(level.min(3), Ordering::Relaxed);
}

/// The Volumetric clouds level (0 Classic .. 3 High) for `Graphics::clouds_q`.
pub(super) fn level() -> u8 {
	CLOUDS_Q.load(Ordering::Relaxed)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The clouds row is the 15th of `ROWS`: Classic, High, Medium, Low; its byte is outside the title page and the packed word.
	#[test]
	fn the_clouds_row_is_the_last_one() {
		assert_eq!(GROWS[14], CLOUDS);
		assert_eq!(ROWS[14], [ID_CLOUDS, CLOUDS, ID_CLASSIC, ID_HIGH, ID_MEDIUM, ID_LOW]);
		assert_eq!((row_values(14), ROWS.len(), CLOUDS), (4, 15, fogopt::FOG + 1));
		assert_eq!(DEFAULT_ROWS[14], 1, "a new player: High");
		assert_eq!([0u8, 1, 2, 3].map(|b| fogopt::name(swap_level(b))), ["classic", "high", "medium", "low"]);
		// The word does not hold Fog or Clouds: the other 13 rows are what they were.
		let (a, b) = (pack_gfx(DEFAULT_ROWS), pack_gfx([0, 1, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 3, 0]));
		assert_eq!(a, b);
	}

	#[test]
	fn the_level_is_clamped() {
		let _turn = super::super::tests::SERIAL.lock().unwrap_or_else(|e| e.into_inner());
		set(7);
		assert_eq!(level(), 3);
		set(0);
		assert_eq!(level(), 0);
		set(3);
	}
}
