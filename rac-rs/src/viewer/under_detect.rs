//! [PC] Water Low / Medium / High: when the camera counts as under water (the gate of underwater.rs).
//!
//! Retail's camera water flag (0x167414) comes from a vertical probe through water FACES (decomp/notes/water.md). Where the hero swims
//! through a flooded passage that has no water face (the Pokitaru Seebad tunnels: the hero keeps swimming under the sticky surface
//! height 0x13f640, hero_swim.md), the flag stays 0 and retail draws the dry fog there. The PC look must still turn on: it also
//! counts the camera as under water when the hero is swimming (group 0x11 under / 0x12 at the surface, `SwapWords::surf`) and the
//! camera is below that surface. A camera above the surface, or any camera while the hero is not swimming, gets only retail's flag,
//! so a floor under some other water's plane elsewhere in the level never turns it on (no plane or box lookup is involved).

/// The camera must sit this far below the hero's water surface to count (retail's probe uses +0.04; the fade of `Under::ramp` smooths
/// a camera bobbing at the surface).
const BELOW: f32 = 0.1;

/// The flag the underwater look runs on: retail's camera flag, or the camera below the surface of the water the hero swims in.
pub(super) fn camera_under(retail_flag: bool, hero_surf: Option<f32>, cam_z: f32) -> bool {
	retail_flag || hero_surf.is_some_and(|s| s.is_finite() && cam_z < s - BELOW)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn retail_flag_always_counts() {
		assert!(camera_under(true, None, 10.0));
		assert!(camera_under(true, Some(5.0), 10.0));
	}

	#[test]
	fn swimming_hero_with_camera_below_the_surface() {
		assert!(camera_under(false, Some(149.5), 135.0));
		assert!(camera_under(false, Some(149.5), 149.3));
	}

	#[test]
	fn above_the_surface_or_not_swimming_stays_off() {
		assert!(!camera_under(false, Some(149.5), 149.45));
		assert!(!camera_under(false, Some(149.5), 160.0));
		assert!(!camera_under(false, None, 100.0));
		assert!(!camera_under(false, Some(f32::NAN), 100.0));
	}
}
