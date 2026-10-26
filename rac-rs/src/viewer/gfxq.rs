//! [PC] GPU-cost switches of the v0.4 effects (lane gfxq, 2026-10-26): the same picture for less GPU time, each one an environment
//! switch so a change can be A/B'd (ENV.md rows). Read once. A switch is on by default only where the look check passed and the user OK'd it (the
//! default of each is in its function).
//!
//! - `RAC_FOG_DIV=N` (1..3): the fog's grid divisor at Fog High (volfog.rs `Quality::div`; 1 = half the picture, 2 = a quarter).
//! - `RAC_CLOUD_DIV=N` (2..8): the cloud grid divisor at Clouds High (volclouds/params.rs `Quality::div`; 4 = a quarter).
//! - `RAC_CLOUD_REUSE=1`: the clouds are marched every other frame; the frame between draws the last result turned to the new view
//!   (volclouds.rs `march`, layer.rs `fs_apply_clouds`); only while the view stays within `CLOUD_REUSE_DEG` of the marched one.
//! - `RAC_SUN_PLAIN`, `RAC_SUN_NODYN`: measuring aids of the shadow pass (texel-alpha casters drawn depth-only, no props / mobys).

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

/// RAC_FOG_DIV: the grid divisor at Fog High; default 2 (a quarter of the picture, user OK 2026-10-26); 1 = the old half picture.
pub(super) fn fog_div() -> Option<u32> {
	static S: OnceLock<Option<u32>> = OnceLock::new();
	*S.get_or_init(|| num("RAC_FOG_DIV", 1, 3).or(Some(2)))
}

/// RAC_CLOUD_DIV: the grid divisor at Clouds High; default 6 (Medium's grid, user OK 2026-10-26); 4 = the old quarter picture.
pub(super) fn cloud_div() -> Option<u32> {
	static S: OnceLock<Option<u32>> = OnceLock::new();
	*S.get_or_init(|| num("RAC_CLOUD_DIV", 2, 8).or(Some(6)))
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

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn switch_values_are_range_checked() {
		assert_eq!(num("GFXQ_NOT_A_SWITCH", 1, 3), None);
		assert!(flag("GFXQ_NOT_A_SWITCH", true) && !flag("GFXQ_NOT_A_SWITCH", false));
	}
}
