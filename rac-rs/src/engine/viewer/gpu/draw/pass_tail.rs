//! [PC] Issue #29 (Qwark's TV): the draws of the blended texels (retail's ABE pass) and the late rounds of the mobys ranked after a
//! blended no-Z tail (`tail_order.rs`).

use super::*;
use crate::viewer::gpu::moby_add::AddPart;

/// The props batch's pose bind and pipelines when it is posed by `vs_props` (`pass_world`'s `skinned`, kept for the props batch only).
pub(super) type Skinned<'a> = Option<(&'a crate::viewer::gpuskin::PropsGpu, &'a wgpu::Buffer)>;

/// The moby batches of one level pass (`pass_world`): what the blend pass and the late rounds draw.
pub(super) struct Mobies<'a> {
	pub scene: &'a Scene,
	pub skinned: Skinned<'a>,
	/// Moby draw instance word (`pass_world`'s `moby_inst`).
	pub inst: &'a dyn Fn(&Draw) -> u32,
	pub marker: Option<&'a Batch>,
	pub props: Option<&'a Batch>,
	pub clipped: Option<&'a Batch>,
	pub debris: Option<&'a Batch>,
	pub warp: Option<&'a Batch>,
	pub metal: Option<(&'a Batch, &'a [wgpu::BindGroup; 2])>,
}

/// One blend pass: the draws of each batch it covers (`marker`, `debris` and `warp` draw their batch whole).
pub(super) struct Blend<'a> {
	pub marker: bool,
	pub props: &'a [Draw],
	pub clip: &'a [Draw],
	pub debris: bool,
	pub warp: bool,
	pub metal: &'a [Draw],
}

impl Blend<'_> {
	pub(super) const NONE: Blend<'static> = Blend { marker: false, props: &[], clip: &[], debris: false, warp: false, metal: &[] };
}

/// A late round: the draws of the mobys of one group of `tail_order.rs` (props, their CPU-clipped faces and their metal faces).
pub(super) struct Round<'a> {
	pub props: &'a [Draw],
	pub clip: &'a [Draw],
	pub metal: &'a [Draw],
}

impl Round<'_> {
	pub(super) fn is_empty(&self) -> bool {
		self.props.is_empty() && self.clip.is_empty() && self.metal.is_empty()
	}
}

impl Gpu {
	/// Retail draws every moby texel through one ABE pass: one batch's texels below alpha 0x80 blend here (MOBY-GS), `draws` through
	/// both moby pipelines. `skinned`: the props batch's pose (None for every other batch).
	pub(super) fn abe_batch(&self, pass: &mut wgpu::RenderPass<'_>, scene: &Scene, batch: &Batch, skinned: Skinned, draws: &[Draw], inst: &dyn Fn(&Draw) -> u32) {
		pass.set_vertex_buffer(0, batch.vbuf.slice(..));
		pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
		let (pipelines, culled) = match skinned {
			Some((pg, sj)) => {
				pass.set_bind_group(2, &pg.bind, &[]);
				pass.set_vertex_buffer(1, sj.slice(..));
				(&pg.moby_pipelines, &pg.moby_cull)
			}
			None => (&self.moby_pipelines, &self.moby_cull),
		};
		for (k, pipeline) in pipelines.iter().enumerate() {
			pass.set_pipeline(pipeline);
			for d in draws {
				// Retail culls in VU1 before the GS: the blended texels of a culled packet keep its cull mode.
				if d.cull != 0 {
					pass.set_pipeline(&culled[2 * (d.cull as usize - 1) + k]);
				}
				pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
				let inst = inst(d);
				pass.draw_indexed(d.start..d.start + d.count, 0, inst..inst + 1);
				if d.cull != 0 {
					pass.set_pipeline(pipeline);
				}
			}
		}
	}

	/// BRIDGE-45: the metal texels below alpha 0x80 and every metal face drawn with a metal alpha below 0x80 (the ABE pass), `draws`
	/// of the metal batch.
	fn abe_metal(&self, pass: &mut wgpu::RenderPass<'_>, metal: Option<(&Batch, &[wgpu::BindGroup; 2])>, draws: &[Draw], texture: Option<usize>) {
		if let Some((batch, tex)) = metal.filter(|_| !draws.is_empty()) {
			pass.set_vertex_buffer(0, batch.vbuf.slice(..));
			pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
			for pipeline in self.moby_pipelines.iter() {
				pass.set_pipeline(pipeline);
				for d in draws.iter().filter(|d| texture.is_none_or(|t| t == d.texture)) {
					pass.set_bind_group(1, &tex[d.texture], &[]);
					let inst = crate::viewer::metalfx::chrome_instance(d); pass.draw_indexed(d.start..d.start + d.count, 0, inst..inst + 1);
				}
			}
		}
	}

	/// One blend pass over the batches `b` selects (marker, props, clipped, debris, warp, then the metal faces, as retail's order).
	pub(super) fn abe_round(&self, pass: &mut wgpu::RenderPass<'_>, m: &Mobies, b: &Blend) {
		let scene = m.scene;
		if let Some(batch) = m.marker.filter(|_| b.marker) {
			self.abe_batch(pass, scene, batch, None, &batch.draws, m.inst);
		}
		for (batch, skinned, draws) in [(m.props, m.skinned, b.props), (m.clipped, None, b.clip)] {
			if let Some(batch) = batch.filter(|_| !draws.is_empty()) {
				self.abe_batch(pass, scene, batch, skinned, draws, m.inst);
			}
		}
		for batch in [m.debris.filter(|_| b.debris), m.warp.filter(|_| b.warp)].into_iter().flatten() {
			self.abe_batch(pass, scene, batch, None, &batch.draws, m.inst);
		}
		self.abe_metal(pass, m.metal, b.metal, None);
	}

	/// The late rounds `rounds` of the level pass, then the additive mobys of `add` (`moby_add.rs`, retail's ALPHA_1 0x48) over everything blended.
	pub(super) fn tail_rounds_add(&self, pass: &mut wgpu::RenderPass<'_>, m: &Mobies, rounds: &[Round], add: AddPart) {
		for r in rounds {
			self.tail_round(pass, m, r);
		}
		self.add_round(pass, m.scene, m.props, m.skinned, m.inst, add);
	}

	/// Original metal packets in a late round: keep glass before the body (Clank's soft dome rim), but chrome after it.
	/// Chrome is a coplanar reflection over the LOD surface; a later opaque body would erase it.
	fn late_metal(&self, pass: &mut wgpu::RenderPass<'_>, m: &Mobies, draws: &[Draw], texture: usize) {
		if let Some((batch, tex)) = m.metal.filter(|_| !draws.is_empty()) {
			pass.set_vertex_buffer(0, batch.vbuf.slice(..));
			pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
			for d in draws.iter().filter(|d| d.texture == texture) {
				pass.set_pipeline(if d.cull != 0 { &self.pipeline_cull[d.cull as usize - 1] } else { &self.pipeline });
				pass.set_bind_group(1, &tex[d.texture], &[]);
				let inst = crate::viewer::metalfx::chrome_instance(d); pass.draw_indexed(d.start..d.start + d.count, 0, inst..inst + 1);
			}
		}
		if !dskip("abe") { self.abe_metal(pass, m.metal, draws, Some(texture)); }
	}

	/// The late mobys' base surfaces, then their original chrome reflection. The order is the same whichever tail group
	/// visibility selects; it does not depend on the optional metal shader or use an opacity/depth offset workaround.
	pub(super) fn tail_round(&self, pass: &mut wgpu::RenderPass<'_>, m: &Mobies, r: &Round) {
		let scene = m.scene;
		self.late_metal(pass, m, r.metal, 1);
		for (batch, skinned, draws) in [(m.props, m.skinned, r.props), (m.clipped, None, r.clip)] {
			let Some(batch) = batch.filter(|_| !draws.is_empty()) else { continue };
			let (pipeline, cull) = match skinned {
				Some((pg, sj)) => {
					pass.set_bind_group(2, &pg.bind, &[]);
					pass.set_vertex_buffer(1, sj.slice(..));
					(&pg.pipeline, &pg.pipeline_cull)
				}
				None => (&self.pipeline, &self.pipeline_cull),
			};
			pass.set_pipeline(pipeline);
			pass.set_vertex_buffer(0, batch.vbuf.slice(..));
			pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
			for d in draws {
				pass.set_pipeline(if d.cull != 0 { &cull[d.cull as usize - 1] } else { pipeline });
				pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
				let inst = (m.inst)(d);
				pass.draw_indexed(d.start..d.start + d.count, 0, inst..inst + 1);
			}
		}
		if !dskip("abe") {
			self.abe_round(pass, m, &Blend { props: r.props, clip: r.clip, ..Blend::NONE });
		}
		self.late_metal(pass, m, r.metal, 0);
	}
}
