//! Level-light slots of the pause menu and the gold weapon (LIT-7a), and the scene actors' light snapshot (SCN-3);
//! with their tests.

use super::*;

/// Level-light slot 14, filled at pause open for the menu mobies (FUN_0028ba40; decomp/notes/lighting_fog.md
/// "Reserved level-light slots 12-14"): colour, wrap -0.25, the direction it travels; slot B zero.
pub(super) const MENU_LIGHT_SLOT: usize = 14;
pub(super) const MENU_LIGHT_COLOUR: [f32; 3] = [0.9, 0.9, 0.6];
pub(super) const MENU_LIGHT_DIR: [f32; 3] = [0.7155, 0.5367, -0.4472];

/// LIT-7a: the owned gold weapon in pause panel 3 is lit by level-light slot 12 (FUN_00206e78 with the frame moby's light
/// as B, decomp/notes/lighting_fog.md "LIT-7a port spec"). Ambient bytes (R, G) = (n * 4 / 10, n * 3 / 10) for n = 3 x 0x20 = 96
/// (integer divisions), blue 0; slot 12 colour A = (s / 2, 0.4 s, 0) with s the sum of slot 14's colour, wrap -0.3, slot 14's direction.
pub(super) const GOLD_LIGHT_SLOT: usize = 12;
pub(super) const GOLD_AMBIENT: [f32; 3] = [(96 * 4 / 10) as f32 / 128.0, (96 * 3 / 10) as f32 / 128.0, 0.0];
pub(super) const GOLD_LIGHT_WRAP: f32 = -0.3;

/// Slot 12's colour A.
pub(super) fn gold_light_colour() -> [f32; 3] {
	let s = MENU_LIGHT_COLOUR[0] + MENU_LIGHT_COLOUR[1] + MENU_LIGHT_COLOUR[2];
	[s * 0.5, s * 0.4, s * 0.0]
}

/// Whether panel-3 item `id` gets the gold light: a gold gadget that is not a kind 1-3 item (ids 1-7, 0x1c-0x1e) and not 0x18.
pub(super) fn gold_lit(id: u8, golds: &[u8]) -> bool {
	golds.contains(&id) && !(1..=7).contains(&id) && !(0x1c..=0x1e).contains(&id) && id != 0x18
}

/// SCN-3: the scene actors' light, captured once when a scene starts (the hero's at that instant) and held until it ends.
pub(super) fn scene_light_snapshot(held: &mut Option<[f32; 4]>, active: bool, now: impl FnOnce() -> [f32; 4]) -> Option<[f32; 4]> {
	if !active {
		*held = None;
		return None;
	}
	Some(*held.get_or_insert_with(now))
}

#[cfg(test)]
mod scene_light_tests {
	use super::*;

	/// HERO-AFTERIMAGE-15 test 4: the two afterimage alpha bytes encode as the spec's fractions and flag translucent.
	#[test]
	fn afterimage_alpha_encoding() {
		assert_eq!(moby_alpha_encoding(0.0, 0x30), (0.15625, true));
		assert_eq!(moby_alpha_encoding(0.0, 0x17), (0.20507812, true));
	}

	/// LIT-7a tests 1-3: ambient bytes (38, 28, 0), slot 12 colour (1.2, 0.96, 0) wrap -0.3, the item gate.
	#[test]
	fn gold_light_constants_and_gate() {
		assert_eq!(GOLD_AMBIENT, [0.296875, 0.21875, 0.0]);
		assert_eq!(((96 * 4 / 10) as u32, (96 * 3 / 10) as u32), (38, 28));
		assert_eq!(gold_light_colour(), [1.2f32, 0.96000004, 0.0]);
		assert_eq!(GOLD_LIGHT_WRAP, -0.3);
		assert_eq!(Gpu::pack_light(GOLD_AMBIENT, GOLD_LIGHT_SLOT, GOLD_LIGHT_SLOT, 0.0)[3], 204.0);
		assert!(gold_lit(0x10, &[0x10, 3]));
		assert!(!gold_lit(0x18, &[0x18]));
		assert!(!gold_lit(3, &[3]));
		assert!(!gold_lit(0x10, &[]));
	}

	#[test]
	fn snapshot_is_taken_once_per_scene() {
		let mut held = None;
		assert_eq!(scene_light_snapshot(&mut held, true, || [1.0, 2.0, 3.0, 4.0]), Some([1.0, 2.0, 3.0, 4.0]));
		assert_eq!(scene_light_snapshot(&mut held, true, || [9.0; 4]), Some([1.0, 2.0, 3.0, 4.0]));
		scene_light_snapshot(&mut held, false, || [0.0; 4]);
		assert_eq!(scene_light_snapshot(&mut held, true, || [5.0; 4]), Some([5.0; 4]));
	}
}
