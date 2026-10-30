//! [PC] Sun shadow pass (lane v042_shadow): the props batch's draws of one cascade, only the slots that can reach it.
//!
//! The props batch holds the model vertices of every slot (`Scene::props_ranges`), and its draws run over all of them: the vertex
//! shader (`vs_sh_props`) then moves the vertices of the slots that are hidden, or far outside the cascade, out of the picture one
//! by one, three times a frame. Here the draws of a cascade are built per slot instead, from the slot's own index runs
//! (`Scene::slot_runs`), and a slot is left out when it casts nothing (hidden, or a kind the vertex shader drops) or when the
//! world sphere around all its vertices (`gpuskin::slot_sphere`, the sphere the clipper trusts too) lies outside the cascade's box
//! (`Cascade::touches`). A slot left out would only have produced triangles outside the map or none at all, so the map is the same.

use super::super::gpuskin::{slot_sphere, PropInst, KIND_ACTOR, KIND_CAST, KIND_PROP};
use super::view::View;
use super::{Cascade, Draw, Scene};

/// A drawn slot is never smaller than its sphere: the slack covers the rounding of the posing on the GPU (as `sphere_skips`).
const SLACK: f32 = 0.01;

/// The `(first index, index count, 0)` draws of the props batch for `c`, in the batch's own order with runs that touch merged (the
/// conventions of the water lane's `props_live::draws_of`), into `out`; `cut` is scratch (both are cleared first, kept between
/// frames by the caller). `solid`: the draws that cast at all (see-through shells do not). A slot the records do not cover is
/// taken; without a per-slot radius no sphere test is made.
pub(super) fn draws(scene: &Scene, c: &Cascade, view: Option<&View>, solid: &dyn Fn(&Draw) -> bool, cut: &mut Vec<(u32, u32, u32)>, out: &mut Vec<(u32, u32, u32)>) {
	let runs = &scene.slot_runs;
	cut.clear();
	out.clear();
	for (s, r) in runs.iter().enumerate() {
		// Slots past the records (spare room of the pool) are kept as they are: the vertex shader reads whatever record they hold.
		if let Some(inst) = scene.props_inst.get(s) {
			if !casts(inst.k[0]) {
				continue;
			}
			if let Some((centre, rad)) = scene.sun_rad.get(s).and_then(|&m| sphere(inst, m)) {
				if !c.touches(centre, rad * 1.001 + SLACK) || view.is_some_and(|v| v.rejects_sphere(centre, rad * 1.001 + SLACK)) {
					continue;
				}
			}
		}
		cut.extend(r.iter().copied().filter(|r| r.2 > 0));
	}
	cut.sort_unstable();
	let mut last: Option<u32> = None;
	for &(di, start, count) in cut.iter() {
		let Some(d) = scene.props.draws.get(di as usize) else { continue };
		if !solid(d) {
			continue;
		}
		match out.last_mut() {
			Some(p) if last == Some(di) && p.0 + p.1 == start => p.1 += count,
			_ => out.push((start, count, 0)),
		}
		last = Some(di);
	}
}

/// The world sphere around every vertex of a slot (`gpuskin::slot_sphere`). A cast-only moby (kind 4) is placed as a prop (kind 1)
/// by `vs_sh_props`, so its sphere is the prop's.
fn sphere(inst: &PropInst, rad: f32) -> Option<([f32; 3], f32)> {
	if inst.k[0] & 0xff == KIND_CAST {
		let mut as_prop = *inst;
		as_prop.k[0] = (inst.k[0] & !0xff) | KIND_PROP;
		return slot_sphere(&as_prop, rad);
	}
	slot_sphere(inst, rad)
}

/// Whether the vertex shader keeps a slot with this kind word (`vs_sh_props`): kinds 1, 2 and 4, the prompt flag (0x100) clear.
/// `the_shader_keeps_the_kinds_casts_expects` checks the WGSL text for this very rule.
fn casts(k0: u32) -> bool {
	matches!(k0 & 0xff, KIND_PROP | KIND_ACTOR | KIND_CAST) && k0 & 0x100 == 0
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn only_the_kinds_the_vertex_shader_keeps_cast() {
		assert!(casts(KIND_PROP) && casts(KIND_ACTOR) && casts(KIND_CAST));
		assert!(!casts(0) && !casts(3), "hidden and deferred slots cast nothing");
		assert!(!casts(KIND_PROP | 0x100), "the prompt flag hides a slot");
		assert!(casts(KIND_PROP | 0x200), "other flag bits do not");
	}

	/// `casts` mirrors one condition of `vs_sh_props`; if the shader's changes, the CPU cull would drop slots the shader draws.
	#[test]
	fn the_shader_keeps_the_kinds_casts_expects() {
		let src = super::super::SUN_SHADER;
		assert_eq!((KIND_PROP, KIND_ACTOR, KIND_CAST), (1, 2, 4));
		assert!(src.contains("let kind = s.k.x & 0xffu;"), "the kind is the low byte of k.x");
		assert!(src.contains("if ((kind == 1u || kind == 2u || kind == 4u) && (s.k.x & 0x100u) == 0u && alpha >= 96.0) {"), "vs_sh_props' keep rule changed: update props.rs `casts`");
		assert_eq!(src.matches("kind == ").count(), 3, "no other kind test in the shader (the three of the keep rule; the placement uses `kind != 2u`)");
	}

	#[test]
	fn a_cast_only_moby_has_the_sphere_of_a_prop() {
		let mut p = PropInst::hidden();
		p.m[0] = 2.0;
		p.m[5] = 2.0;
		p.m[10] = 2.0;
		p.m[12..15].copy_from_slice(&[10.0, 20.0, 30.0]);
		p.a[0] = 1.5;
		p.k[0] = KIND_PROP | 0x200;
		let want = slot_sphere(&p, 3.0);
		assert!(want.is_some());
		p.k[0] = KIND_CAST | 0x200;
		assert_eq!(sphere(&p, 3.0), want);
		p.k[0] = KIND_ACTOR;
		assert_eq!(sphere(&p, 3.0).map(|s| s.1), Some(3.0));
		assert_eq!(sphere(&p, f32::INFINITY), None);
	}
}
