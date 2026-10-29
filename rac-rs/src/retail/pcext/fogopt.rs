//! The Graphics page's Fog row (volumetric fog, viewer/volfog.rs; lane yfog, 2026-10-24): Classic / High / Medium / Low like the
//! other effect rows (menu byte: 0 Classic = the retail picture, 1 High, 2 Medium, 3 Low; `swap_level` turns it into the level
//! 0 Classic .. 3 High that the file's `fog` key and the renderer use). Like the other effects it only acts with Lighting Enhanced.
//!
//! `pack_gfx` has no free bit left, so the level lives here: `apply_graphics` and `init` store it, `graphics()` reads it into
//! `Graphics::fog_q`. The title's Graphics page keeps its 13 rows (its value panel holds 13); Fog is changed on the pause page
//! (Pause -> Options -> Graphics -> Effects) or with the key `fog=classic|low|medium|high` in pc_options.txt.
use super::*;
use std::sync::atomic::AtomicU8;

/// Graphics -> Fog (the row byte, see above). A new player has High (like the other effect rows), which shows with Lighting Enhanced.
pub const FOG: u32 = BASE + 21;

/// The level the Fog row asks for (0 Classic .. 3 High), as last applied.
static FOG_Q: AtomicU8 = AtomicU8::new(3);

/// `apply_graphics` / `init`: the level of the row (0 Classic .. 3 High).
pub(super) fn set(level: u8) {
	FOG_Q.store(level.min(3), Ordering::Relaxed);
}

/// The scale of retail's own fog (f32 bits; 1.0 = the retail picture): the volumetric fog (viewer/gpu/draw/prepare.rs, from `volfog::Plan`)
/// lowers it while it draws, because retail's fog would paint a flat opaque floor under the haze. The world uniform upload
/// (viewer/gpu/draw/prepare.rs -> volfog `fade_retail_fog`) multiplies the fog's share (1 - f) by it. The GS capture is not
/// involved: the world's fog is drawn by the native shaders (viewer.rs), not by captured GS prims.
static GS_SCALE: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0x3f80_0000);

/// The viewer's per-frame verdict: the retail fog's scale (1.0 when no volumetric fog is drawn).
pub fn set_gs_scale(s: f32) {
	GS_SCALE.store(s.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

/// The retail GS fog's scale for the capture (exactly 1.0 = unchanged maths).
pub fn gs_scale() -> f32 {
	f32::from_bits(GS_SCALE.load(Ordering::Relaxed))
}

/// The Fog level (0 Classic .. 3 High) for `Graphics::fog_q`.
pub(super) fn level() -> u8 {
	FOG_Q.load(Ordering::Relaxed)
}

/// The fog key's level name (what the file and the log write).
pub fn name(level: u8) -> &'static str {
	pcopts::LEVELS_CLASSIC[level.min(3) as usize]
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The Fog row is the 14th of `ROWS`: Classic, High, Medium, Low; its byte is outside the title page and the packed word.
	#[test]
	fn the_fog_row_is_the_last_one() {
		assert_eq!(GROWS[13], FOG);
		assert_eq!(ROWS[13], [ID_FOG, FOG, ID_CLASSIC, ID_HIGH, ID_MEDIUM, ID_LOW]);
		assert_eq!((row_values(13), ROWS.len()), (4, 15));
		assert_eq!(DEFAULT_ROWS[13], 1, "a new player: High");
		assert_eq!([0u8, 1, 2, 3].map(|b| name(swap_level(b))), ["classic", "high", "medium", "low"]);
		// The word does not hold Fog: the other 13 rows are what they were.
		let (a, b) = (pack_gfx(DEFAULT_ROWS), pack_gfx([0, 1, 1, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 3, 3]));
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
