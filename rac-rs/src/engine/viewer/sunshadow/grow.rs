//! [PC] Sun shadow pass (lane nsun, 2026-11-03): thin casters keep their shadow in the far cascades.
//!
//! A cascade texel is 2 r / res world units (High, at a walkway camera: 0.011 / 0.034 / 0.10 m for cascades 0 / 1 / 2). A lamp
//! post on Pokitaru (a shrub, ~0.06 m thick) cast a 5-8 texel line in cascade 0, ~2 texels in cascade 1, which the 4 x 4 PCF of the
//! receivers washed out, and under one texel in cascade 2 (gone), while its 0.92 m head stayed: from the game camera, lamps 10-30 m
//! away cast round heads floating on the walkway with no pole. Every moby and shrub vertex is therefore moved `texels` x texel
//! along its normal in the cascades past the first (`sh_grow` in sunshadow.rs; across the light only, the depth stays, so the 1.5
//! slope bias still holds), so a post stays 2-4 texels wide; a bigger caster grows by that much at its rim (0.03 / 0.10 m),
//! invisible at those distances. Terrain, ties (buildings) and the world batch cast as they are. RAC_TUNE `sun_grow1` /
//! `sun_grow2` (texels, default 1 / 1; 0 = the shadows before) and `sun_far_shrubs` (0 = shrubs cast nothing into the last High
//! cascade: a far lamp casts nothing rather than a head) are dev knobs (devtune.rs).

use super::{Cascade, CASCADES};

/// Bytes of the cascade uniform `SU`: the light view-projection, then `grow` (`uniform`).
pub(super) const SU_BYTES: u64 = 80;

/// Texels each moby and shrub caster widens by on each side, per cascade: none in the first (its texels are finer than any post).
const TEXELS: [f32; CASCADES] = [0.0, 1.0, 1.0];

/// This frame's widening per cascade (`TEXELS`, or the RAC_TUNE knobs).
pub(super) fn texels() -> [f32; CASCADES] {
	[TEXELS[0], crate::devtune::f32("sun_grow1", TEXELS[1]).max(0.0), crate::devtune::f32("sun_grow2", TEXELS[2]).max(0.0)]
}

/// Whether shrubs cast into cascade `k` (all of them unless RAC_TUNE `sun_far_shrubs=0` drops the last High cascade's).
pub(super) fn shrubs_cast(k: usize) -> bool {
	k + 1 < CASCADES || crate::devtune::f32("sun_far_shrubs", 1.0) != 0.0
}

/// The uniform of cascade `k` widened by `texels`: `vp` (columns), then (widening in world units, k, texel, 0).
pub(super) fn uniform(c: &Cascade, k: usize, texels: f32) -> [f32; 20] {
	let mut u = [0.0f32; 20];
	for (i, col) in c.vp.iter().enumerate() {
		u[i * 4..i * 4 + 4].copy_from_slice(col);
	}
	u[16..].copy_from_slice(&[texels * c.texel, k as f32, c.texel, 0.0]);
	u
}

#[cfg(test)]
mod tests {
	use super::*;

	fn cascade() -> Cascade {
		Cascade::fit([0.0, 0.0, 10.0], [1.0, 0.0, 0.0], 0.6, 16.0 / 9.0, 12.0, 36.0, [0.3, 0.2, -0.9], 2048)
	}

	#[test]
	fn the_uniform_is_the_view_projection_then_the_widening() {
		let c = cascade();
		let u = uniform(&c, 1, 1.0);
		assert_eq!(u.len() * 4, SU_BYTES as usize);
		assert_eq!(&u[..16], bytemuck::cast_slice::<[f32; 4], f32>(&c.vp));
		assert_eq!(u[16..], [c.texel, 1.0, c.texel, 0.0]);
		assert_eq!(uniform(&c, 0, 0.0)[16], 0.0, "no widening: the vertex stays put (`sh_grow` returns early)");
	}

	#[test]
	fn only_the_far_cascades_widen() {
		assert_eq!(TEXELS[0], 0.0);
		assert!(TEXELS[1..].iter().all(|&t| (0.5..=1.5).contains(&t)), "about a texel: the 1.5 slope bias covers the shifted depth");
		assert!(shrubs_cast(0) && shrubs_cast(1));
	}

	/// The widened entries: shrubs (`vs_sh_grow`) and the posed props (`sh_place`) move along the normal, terrain and ties do not.
	#[test]
	fn the_shaders_widen_mobys_and_shrubs_only() {
		let tex = super::super::shrub_shader();
		let props = super::super::SUN_SHADER;
		for (name, src) in [("texel alpha", tex.clone()), ("sun shadow", props.to_string())] {
			assert!(src.contains("struct SU { vp: mat4x4<f32>, grow: vec4<f32> };"), "{name}");
			let m = naga::front::wgsl::parse_str(&src).unwrap_or_else(|e| panic!("{name}: {e}"));
			naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).unwrap_or_else(|e| panic!("{name}: {e:?}"));
		}
		assert!(tex.contains("o.pos = sh_grow(su.vp * vec4<f32>(p, 1.0), n);"));
		assert_eq!(tex.matches("o.pos = su.vp * vec4<f32>(p, 1.0);").count(), 1, "vs_sh_shrub (terrain, ties) as before");
		assert!(props.contains("return sh_grow(su.vp * vec4<f32>(pos, 1.0), nrm);"));
		assert!(props.contains("nrm = s.m0.xyz * nn.x + s.m1.xyz * nn.y + s.m2.xyz * nn.z;"), "the normal placed as `vs_props_pose` places it");
	}
}
