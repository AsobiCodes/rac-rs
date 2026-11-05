//! [PC] The far-sea look and the shader's cheap shortcuts (lane fps3, v0.4.2; WGSL appended to `SHADER` in `WaterFx::new`).
//! `far_lod` blends the geometric sea from its full picture (nearer than `water_lod0` units, byte for byte the picture of before) to
//! a cheap far picture (from `water_lod1`): no rings, wakes or churn, no caustics, no glint ripple, no shore fade, a sky
//! visibility of five taps, two mirror taps instead of four. Each part is faded out over the blend (the same smoothstep shape as
//! `ds`), so nothing pops. The near-identical switches (`w.lod.w` bits: 1 reflection march of at most 20 steps, 2 shore fade from
//! 8 bed taps, 4 sky visibility from 5 taps, 8 the planar mirror's blur summed in encoded colour) trade a few levels of colour on
//! some pixels for speed; `NEAR_DEFAULT` is what is on without a switch.
//! BOTH ARE OFF BY DEFAULT: the far band of the far look still differs from the full picture by up to 124/255 on some pixels (a bug
//! not found yet); with the look off (`w.lod.z` = 0, every `lod` = 0) the shader gives the same picture as without this file.
//! RAC_WATER_FAR_LOD=1 turns the far look on, RAC_WATER_NEAR=N sets the switches (0 = none), RAC_TUNE `water_lod0`, `water_lod1`,
//! `water_near` change the distances and the switches without a rebuild (read once).

use std::sync::OnceLock;

/// The near-identical switches that are on without `RAC_WATER_NEAR` (none).
pub(super) const NEAR_DEFAULT: u32 = 0;

pub(super) const FAR_LOD_WGSL: &str = r#"
// [PC] Far-sea look (fps3): 0 = the full picture (nearer than w.lod.x units: byte for byte the old picture), 1 = the cheap far
// picture (from w.lod.y units), blended in between by the same smoothstep shape as `ds`. w.lod = (near edge, far edge, on, near switches).
fn far_lod(fdist: f32, geo: bool) -> f32 {
	if (!geo || w.lod.z < 0.5) { return 0.0; }
	return smoothstep(w.lod.x, w.lod.y, fdist);
}

// The share of the sun reaching p: ten taps, five taps in the far sea (and with the near switch 4).
fn sky_vis_lod(p: vec3<f32>, jit: f32, lod: f32) -> f32 {
	if (lod >= 1.0 || (u32(w.lod.w) & 4u) != 0u) { return sky_vis_n(p, jit, 5.0); }
	return sky_vis(p, jit);
}
"#;

/// The uniform: (near edge, far edge, on, switches). The far edge stays at least half a unit past the near one.
pub(super) fn params_from(far: Option<&str>, lod0: f32, lod1: f32, near: Option<&str>, tune_near: Option<f32>) -> [f32; 4] {
	let lo = lod0.max(0.0);
	let mask = tune_near.map(|v| v.max(0.0) as u32).or_else(|| near.and_then(|v| v.trim().parse::<u32>().ok())).unwrap_or(NEAR_DEFAULT) & 15;
	[lo, lod1.max(lo + 0.5), if far == Some("1") { 1.0 } else { 0.0 }, mask as f32]
}

pub(super) fn params() -> [f32; 4] {
	static P: OnceLock<[f32; 4]> = OnceLock::new();
	*P.get_or_init(|| {
		let tune_near = crate::devtune::f32("water_near", -1.0);
		params_from(
			std::env::var("RAC_WATER_FAR_LOD").ok().as_deref(),
			crate::devtune::f32("water_lod0", 25.0),
			crate::devtune::f32("water_lod1", 50.0),
			std::env::var("RAC_WATER_NEAR").ok().as_deref(),
			(tune_near >= 0.0).then_some(tune_near),
		)
	})
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn far_look_and_near_switches_are_off_by_default() {
		assert_eq!(NEAR_DEFAULT, 0);
		assert_eq!(params_from(None, 25.0, 50.0, None, None), [25.0, 50.0, 0.0, 0.0]);
	}

	#[test]
	fn far_lod_one_turns_it_on() {
		assert_eq!(params_from(Some("1"), 25.0, 50.0, None, None), [25.0, 50.0, 1.0, 0.0]);
		assert_eq!(params_from(Some("0"), 25.0, 50.0, None, None)[2], 0.0);
		assert_eq!(params_from(Some("on"), 25.0, 50.0, None, None)[2], 0.0);
	}

	#[test]
	fn the_far_edge_stays_past_the_near_edge() {
		let p = params_from(None, 40.0, 10.0, None, None);
		assert!(p[1] > p[0]);
		let p = params_from(None, -5.0, -5.0, None, None);
		assert!(p[0] == 0.0 && p[1] > p[0]);
	}

	#[test]
	fn the_near_switches_come_from_the_env_then_the_tune_value() {
		assert_eq!(params_from(None, 25.0, 50.0, Some("0"), None)[3], 0.0);
		assert_eq!(params_from(None, 25.0, 50.0, Some("7"), None)[3], 7.0);
		assert_eq!(params_from(None, 25.0, 50.0, Some("99"), None)[3], (99 & 15) as f32);
		assert_eq!(params_from(None, 25.0, 50.0, Some("7"), Some(2.0))[3], 2.0);
		assert_eq!(params_from(None, 25.0, 50.0, Some("x"), None)[3], NEAR_DEFAULT as f32);
	}
}
