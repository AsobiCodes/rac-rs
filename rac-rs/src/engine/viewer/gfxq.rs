//! [PC] GPU-cost switches of the v0.4 effects (lane gfxq, 2026-10-26): the same picture for less GPU time, each one an environment
//! switch so a change can be A/B'd (ENV.md rows). Read once. A switch is on by default only where the look check passed and the user OK'd it (the
//! default of each is in its function).
//!
//! - `RAC_FOG_DIV=N` (1..3): overrides the fog's grid divisor at Fog High (volfog.rs `Quality::div`, 2 = a quarter of the picture; 1 = half).
//! - `RAC_CLOUD_DIV=N` (2..8): overrides the cloud grid divisor at Clouds High (volclouds/params.rs `Quality::div`, 6; 4 = a quarter).
//! - `RAC_CLOUD_REUSE=1`: the clouds are marched every other frame; the frame between draws the last result turned to the new view
//!   (volclouds.rs `march`, layer.rs `fs_apply_clouds`); only while the view stays within `CLOUD_REUSE_DEG` of the marched one.
//! - `RAC_SUN_PLAIN`, `RAC_SUN_NODYN`: measuring aids of the shadow pass (texel-alpha casters drawn without the cut, no props / mobys).
//! - `RAC_SUN_NOCULL`: A/B aid, the shadow pass without the exact cuts of lane v042_shadow (same picture).
//! - `RAC_SUN_PROPCUT=0`: measuring aid, mobys cast depth-only again (no texel-alpha cut, additive slots cast: lane nsun's base).
//! - `RAC_SUN_LOG=1`, `RAC_SUN_SKIP=c,c`: diagnosis of the shadow pass's moby casters (sunshadow/props.rs `draws`, `skipped`);
//!   `RAC_SUN_LOG_AT`: the same for terrain, ties and shrubs near a world point's ray to the sun (sunshadow/diag.rs).
//! - `RAC_SUN_UNSEEN=1`: ties and terrain pieces that no camera cell shows (hidden by design) cast again (lane nsun's base).

use std::sync::OnceLock;

fn num(name: &'static str, lo: u32, hi: u32) -> Option<u32> {
	std::env::var(name).ok().and_then(|v| v.trim().parse::<u32>().ok()).filter(|n| (lo..=hi).contains(n))
}

fn flag(name: &'static str, default: bool) -> bool {
	match std::env::var(name) {
		Ok(v) => !matches!(v.trim(), "" | "0"),
		Err(_) => default,
	}
}

/// RAC_FOG_DIV: overrides the grid divisor at Fog High (unset: volfog.rs `quality`, 2 = a quarter of the picture, user OK 2026-10-26); 1 = the old half picture.
pub(super) fn fog_div() -> Option<u32> {
	static S: OnceLock<Option<u32>> = OnceLock::new();
	*S.get_or_init(|| num("RAC_FOG_DIV", 1, 3))
}

/// RAC_CLOUD_DIV: overrides the grid divisor at Clouds High (unset: volclouds/params.rs `quality`, 6 = Medium's grid, user OK 2026-10-26); 4 = the old quarter picture.
pub(super) fn cloud_div() -> Option<u32> {
	static S: OnceLock<Option<u32>> = OnceLock::new();
	*S.get_or_init(|| num("RAC_CLOUD_DIV", 2, 8))
}

/// RAC_CLOUD_REUSE: march the clouds every other frame; default on (user OK 2026-10-26), 0 = every frame.
pub(super) fn cloud_reuse() -> bool {
	static S: OnceLock<bool> = OnceLock::new();
	*S.get_or_init(|| flag("RAC_CLOUD_REUSE", true))
}

/// RAC_SUN_PLAIN: measuring aid, terrain / tie casters without the texel-alpha cut.
pub(super) fn sun_plain() -> bool {
	static S: OnceLock<bool> = OnceLock::new();
	*S.get_or_init(|| flag("RAC_SUN_PLAIN", false))
}

/// RAC_SUN_NODYN: measuring aid, no props / mobys in the shadow pass.
pub(super) fn sun_nodyn() -> bool {
	static S: OnceLock<bool> = OnceLock::new();
	*S.get_or_init(|| flag("RAC_SUN_NODYN", false))
}

/// RAC_SUN_NOCULL: A/B aid, the shadow pass without the exact cuts of lane v042_shadow (depth-only casters, props cull, tie / shrub box
/// cull): the draw lists of the lane's base. The picture is the same either way.
pub(super) fn sun_nocull() -> bool {
	static S: OnceLock<bool> = OnceLock::new();
	*S.get_or_init(|| flag("RAC_SUN_NOCULL", false))
}

/// RAC_SUN_FRUSTUM: Sun shadow pass leaves out the casters whose shadow cannot enter the camera's main view pyramid (sunshadow/view.rs).
/// Lossy for the other readers of the cascades (water mirror, cloud sky capture, water sky map), so off by default.
pub(super) fn sun_frustum() -> bool {
	static S: OnceLock<bool> = OnceLock::new();
	*S.get_or_init(|| flag("RAC_SUN_FRUSTUM", false))
}

/// RAC_SUN_PROPCUT: on by default (lane nsun); =0 mobys cast depth-only, additive slots included (the shadows before the lane).
pub(super) fn sun_propcut() -> bool {
	static S: OnceLock<bool> = OnceLock::new();
	*S.get_or_init(|| flag("RAC_SUN_PROPCUT", true))
}

/// RAC_SUN_LOG: diagnosis, every 120th frame of the shadow pass: the camera, the light, the cascades and the moby slots that cast
/// into each cascade (class, kind, centre, radius; sunshadow/diag.rs `begin`).
pub(super) fn sun_log() -> bool {
	static S: OnceLock<bool> = OnceLock::new();
	*S.get_or_init(|| flag("RAC_SUN_LOG", false))
}

/// RAC_SUN_SKIP=c,c,...: diagnosis, moby classes left out of the shadow pass (empty: none).
pub(super) fn sun_skip() -> &'static [u32] {
	static S: OnceLock<Vec<u32>> = OnceLock::new();
	S.get_or_init(|| std::env::var("RAC_SUN_SKIP").map(|v| classes(&v)).unwrap_or_default())
}

/// RAC_SUN_LOG_AT="x y z [m]": diagnosis, with RAC_SUN_LOG the terrain pieces, ties and shrubs whose sphere comes within `m` metres
/// (default 0.75) of the ray from this world point towards the sun (sunshadow/diag.rs `near_ray`).
pub(super) fn sun_log_at() -> Option<([f32; 3], f32)> {
	static S: OnceLock<Option<([f32; 3], f32)>> = OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_SUN_LOG_AT").ok().and_then(|v| {
		let n = numbers(&v);
		(n.len() == 3 || n.len() == 4).then(|| ([n[0], n[1], n[2]], n.get(3).copied().unwrap_or(0.75)))
	}))
}

/// RAC_SUN_UNSEEN: =1 the ties and terrain pieces no camera cell shows (`Occlusion::ever_visible`, never drawn) cast again.
pub(super) fn sun_unseen() -> bool {
	static S: OnceLock<bool> = OnceLock::new();
	*S.get_or_init(|| flag("RAC_SUN_UNSEEN", false))
}

fn numbers(v: &str) -> Vec<f32> {
	v.split([' ', ',']).filter_map(|c| c.trim().parse().ok()).collect()
}

fn classes(v: &str) -> Vec<u32> {
	v.split(',').filter_map(|c| c.trim().parse().ok()).collect()
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn switch_values_are_range_checked() {
		assert_eq!(num("GFXQ_NOT_A_SWITCH", 1, 3), None);
		assert!(flag("GFXQ_NOT_A_SWITCH", true) && !flag("GFXQ_NOT_A_SWITCH", false));
		assert_eq!(classes("12, 880,x,,7"), [12, 880, 7]);
		assert!(classes("").is_empty());
		assert_eq!(numbers("1.5 -2 3"), [1.5, -2.0, 3.0]);
		assert_eq!(numbers("1,2,3, 0.75"), [1.0, 2.0, 3.0, 0.75]);
	}
}
