//! [PC] Issue #29 (Umbris turret bullets barely visible): the additive moby pass.
//!
//! Retail's moby walk (Umbris 0x27e2d0..0x27ec08) puts ALPHA_1 0x48 before the packets of a moby whose flag +0x34 has bit 0x200 and
//! restores 0x8000000044 after them: Cs * As / 128 + Cd, the destination is NOT scaled down (additive). Class 880 (the turret bullet,
//! texel alpha * draw alpha 0x10 >> 7 = As <= 16) is drawn so: three stacked yellow-orange layers, bright. rac-rs drew it with the
//! generic moby blend (Cs - Cd) * As + Cd, which at As = 16/128 left almost nothing. TEST_1 stays 0x5360b (AREF 0x60): a texel with
//! As below 0x60 writes colour but no Z (pass b), the rest writes both (pass a), as the generic moby passes (`fs_moby_a` / `_b`).
//!
//! Both faces are drawn (no back-face culling): the packets of these classes carry VU1 cull mode 2 (`mobies::cull_bits`, back faces
//! dropped), but the GS stream of retail kicks BOTH windings of every triangle of the beam (class 880, fg1_0296 field 12: 12 beams,
//! 384 kicked triangles each, 188 / 190 of the two screen windings after the strip parity; Oltanis / Veldin orbit classes too), so
//! the cull does not apply to a moby with flag 0x200 [C, GS qword dump]. Culling them left 12% of retail's pixel-layers (1497 of
//! 12488 with the same constant source): the beam was ~8x too dim and only visible from some angles. Why the VU1 passes them is
//! not decoded [?]; the flag's drawing rule is "additive, two-sided".
//!
//! The flagged slots (`TailOrder::add`, set in `Gpu::set_props`) leave the opaque and blended draws (`props_live_draws`) and the CPU
//! clipper; this pass draws them last, depth-tested, over the blended mobys (`AddPart::All`). With shadow volumes this frame it
//! follows retail's order, main walk -> volumes -> deferred list (BRIDGE-64a): the flagged slots off the deferred list (`props_late`,
//! moby +0x34 & 0xc00) draw before the volumes (`AddPart::Main`, so the volumes darken them as retail's), the deferred ones after
//! (`AddPart::Deferred`; the Oltanis orbit classes, flags 0xa48 / 0xa4b, are such). The planar mirrors and the water bed draw the
//! slots too (`mirror.rs` `draw_copy`, `AddPart::All`), which have no volumes. Needs the posed props batch (`skinned`); without it
//! the slots would not draw, but that batch is always present in play.
//!
//! Known limits. [?] The flag is honoured for pool props only (`Prop::is_additive`, the props batch): not for the hero / marker
//! batch, the scene actors, the debris or the warp batch. [C] In the 51 converted states (scan of every moby's +0x34) the only
//! classes with bit 0x200 are 880, 404, 405, 674, 677, 680, 934, 1009 and 1017, none of them the hero (class 0), so no picture
//! shown so far needs more; a state outside that set may.

use super::*;
use crate::viewer::gpuskin::PropsGpu;

/// The two fragment entry points appended to the level shader (`volfog::soft::level_source`): the generic moby shading
/// (`moby_texel`, `moby_a`, `moby_rgb` of viewer.rs, fog included) with the split at As = 96 of `fs_moby_a` / `fs_moby_b`.
pub(in crate::viewer) const WGSL: &str = r#"
@fragment fn fs_moby_add_a(i: VOut) -> @location(0) vec4<f32> {
	let c = moby_texel(i);
	let ra = moby_a(i, c);
	if (ra * 128.0 < 96.0) { discard; }
	return vec4<f32>(moby_rgb(i, c), ra);
}

@fragment fn fs_moby_add_b(i: VOut) -> @location(0) vec4<f32> {
	let c = moby_texel(i);
	let ra = moby_a(i, c);
	if (ra * 128.0 >= 96.0) { discard; }
	return vec4<f32>(moby_rgb(i, c), ra);
}
"#;

/// ALPHA_1 0x48 = Cs * As + Cd.
pub(super) const ADD_BLEND: wgpu::BlendState = wgpu::BlendState {
	color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::SrcAlpha, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
	alpha: wgpu::BlendComponent::OVER,
};

type MakeProps<'a> = &'a dyn Fn(&str, Option<wgpu::BlendState>, bool, wgpu::CompareFunction, Option<wgpu::Face>) -> wgpu::RenderPipeline;

/// The two additive pipelines (index = pass: 0 = a, 1 = b), no face culling, built with the caller's `make_props` (`PropsGpu::add`).
pub(super) fn pipelines(make: MakeProps) -> [wgpu::RenderPipeline; 2] {
	[
		make("fs_moby_add_a", Some(ADD_BLEND), true, wgpu::CompareFunction::LessEqual, None),
		make("fs_moby_add_b", Some(ADD_BLEND), false, wgpu::CompareFunction::LessEqual, None),
	]
}

/// Which of the additive slots a draw covers (retail's order is main walk -> shadow volumes -> deferred list, BRIDGE-64a).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::viewer) enum AddPart {
	/// All of them (a frame without shadow volumes, the planar mirrors and the water bed).
	All,
	/// With volumes this frame: the slots off the deferred list, drawn before the volumes.
	Main,
	/// With volumes this frame: the deferred-list slots (`Scene::props_late`), drawn after them.
	Deferred,
}

impl AddPart {
	/// Does this part draw a slot that is on the deferred list (`deferred`) or not?
	pub(super) fn takes(self, deferred: bool) -> bool {
		match self {
			AddPart::All => true,
			AddPart::Main => !deferred,
			AddPart::Deferred => deferred,
		}
	}
}

impl Scene {
	/// The live draws of the additive slots `part` covers (empty when there are none).
	pub(super) fn props_add_draws(&self, part: AddPart) -> Vec<Draw> {
		if !self.props_shown || !self.tail.any_add() {
			return Vec::new();
		}
		let (live, add) = (|s: usize| self.prop_live(s), |s: usize| self.tail.in_add(s));
		super::props_live::split_draws(&self.props.draws, &self.slot_runs, live, add, true, |s| part.takes(self.props_late.contains(&s)))
	}
}

impl Gpu {
	/// Draws the additive mobys of `part`: pass a (As >= 96, Z written) then pass b, each over the slots' draws. `inst`: the draws'
	/// instance word (the level pass adds the material's TEX1 word, the copies use `Draw::inst`).
	pub(in crate::viewer) fn add_round(&self, pass: &mut wgpu::RenderPass<'_>, scene: &Scene, batch: Option<&Batch>, skinned: Option<(&PropsGpu, &wgpu::Buffer)>, inst: &dyn Fn(&Draw) -> u32, part: AddPart) {
		let (Some(batch), Some((pg, sj))) = (batch, skinned) else { return };
		let draws = scene.props_add_draws(part);
		if draws.is_empty() {
			return;
		}
		pass.set_bind_group(2, &pg.bind, &[]);
		pass.set_vertex_buffer(0, batch.vbuf.slice(..));
		pass.set_vertex_buffer(1, sj.slice(..));
		pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
		for pipeline in &pg.add {
			pass.set_pipeline(pipeline);
			for d in &draws {
				pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
				let i = inst(d);
				pass.draw_indexed(d.start..d.start + d.count, 0, i..i + 1);
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::super::props_live::{draws_of, split_draws};
	use super::*;

	#[test]
	fn flag_0x200_is_the_additive_rule() {
		use crate::retail::is_additive;
		for flags in [0x208, 0x200, 0xa48, 0xa4b, 0x240] {
			assert!(is_additive(flags), "{flags:#x}");
		}
		for flags in [0x08, 0x40, 0x140, 0xc3, 0x5420, 0x5060, 0x80a] {
			assert!(!is_additive(flags), "{flags:#x}");
		}
		let mut p = Prop::retail(880, 7);
		assert!(!p.is_additive());
		p.set_retail_additive(true);
		assert!(p.is_additive());
	}

	#[test]
	fn only_shown_flagged_props_take_the_additive_pass() {
		let mut a = Prop::retail(880, 7);
		a.set_retail_additive(true);
		let b = Prop::retail(1, 7);
		let mut out = Vec::new();
		super::super::tail_order::mark_add(&mut out, &[a.clone(), b.clone(), a.clone()], &[true, true, false]);
		assert_eq!(out, vec![true, false, false]);
		super::super::tail_order::mark_add(&mut out, &[b.clone(), a], &[true, false]);
		assert!(out.is_empty(), "none flagged and shown: no vector");
	}

	/// Four slots with runs in two draws (indices 0..8 of draw 0, 8..12 of draw 1; slot 3 has no run in draw 1).
	fn fixture() -> ([Draw; 2], Vec<Vec<(u32, u32, u32)>>) {
		let d = |texture, start, count| Draw { inst: 3, cull: 1, texture, start, count };
		([d(0, 0, 8), d(1, 8, 4)], vec![vec![(0, 0, 2), (1, 8, 1)], vec![(0, 2, 2), (1, 9, 1)], vec![(0, 4, 2), (1, 10, 1)], vec![(0, 6, 2), (1, 11, 0)]])
	}

	/// Every index the draws cover, sorted (a duplicate stays in the list).
	fn indices(draws: &[Draw]) -> Vec<u32> {
		let mut v: Vec<u32> = draws.iter().flat_map(|d| d.start..d.start + d.count).collect();
		v.sort_unstable();
		v
	}

	/// The normal passes (`props_live_draws`) and the additive pass (`props_add_draws`) split the live slots: for every combination
	/// of dead and flagged slots, every index of a live slot's runs is drawn exactly once, by one of the two; a dead slot by none.
	#[test]
	fn normal_and_additive_draws_partition_the_live_slots() {
		let (draws, runs) = fixture();
		for live_mask in 0..16u32 {
			for add_mask in 0..16u32 {
				let (live, add) = (|s: usize| live_mask >> s & 1 == 1, |s: usize| add_mask >> s & 1 == 1);
				let normal = split_draws(&draws, &runs, live, add, false, |_| true);
				let additive = split_draws(&draws, &runs, live, add, true, |_| true);
				let mut got = indices(&normal);
				got.extend(indices(&additive));
				got.sort_unstable();
				assert_eq!(got, indices(&draws_of(&draws, &runs, live)), "live {live_mask:04b} add {add_mask:04b}");
				assert_eq!(indices(&additive), indices(&draws_of(&draws, &runs, |s| live(s) && add(s))), "live {live_mask:04b} add {add_mask:04b}");
			}
		}
	}

	/// With shadow volumes the additive slots split by the deferred list into `Main` (before the volumes) and `Deferred` (after
	/// them), and together they are exactly what a frame without volumes (`All`) draws.
	#[test]
	fn main_and_deferred_parts_partition_all() {
		for deferred in [false, true] {
			assert!(AddPart::All.takes(deferred));
			assert_ne!(AddPart::Main.takes(deferred), AddPart::Deferred.takes(deferred), "deferred {deferred}");
		}
		let (draws, runs) = fixture();
		let late = [false, true, false, true];
		let part = |p: AddPart| split_draws(&draws, &runs, |_| true, |s| s != 3, true, |s| p.takes(late[s]));
		let mut both = indices(&part(AddPart::Main));
		both.extend(indices(&part(AddPart::Deferred)));
		both.sort_unstable();
		assert_eq!(both, indices(&part(AddPart::All)));
		assert_eq!(indices(&part(AddPart::Main)), vec![0, 1, 4, 5, 8, 10]);
		assert_eq!(indices(&part(AddPart::Deferred)), vec![2, 3, 9]);
	}

	#[test]
	fn the_shader_with_the_additive_entry_points_validates() {
		for early in [false, true] {
			let src = crate::viewer::volfog::soft::level_source(crate::viewer::SHADER, early);
			assert!(src.contains("fn fs_moby_add_a(") && src.contains("fn fs_moby_add_b("));
			for s in [src.clone(), crate::viewer::bindless::shader_source(&src)] {
				let m = naga::front::wgsl::parse_str(&s).unwrap_or_else(|e| panic!("{}", e.emit_to_string(&s)));
				naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validates");
			}
		}
	}
}
