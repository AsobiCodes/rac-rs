//! [PC] Fur on the hero (Graphics -> Fur, Fur style Soft / Fluffy; user 2026-10-31, "the rift apart type of fluffiness"). Shell fur
//! on the hero marker: at the end of the world pass (after every opaque and blended moby, so nothing drawn later hides the soft
//! outline) the fur faces are drawn again N times, each shell pushed out along the vertex normal and combed down the surface
//! (`fur.wgsl`), blended inner to outer with the strand coverage of a mipmapped noise texture (`noise.rs`). The shells reuse
//! `marker.vbuf` and its index buffer, so they follow the GPU hero build and the interpolated display frames, and the level
//! shader's own lighting lights them. Split view: the draw hangs on `splitview::fx` (`Gpu::fur_draw`), so the Classic half has no fur.
//!
//! The game says which of the hero's textures are fur (`pack::Pack::fur`, engine/furspec.rs; R&C1: rac1/hero_fur.rs). Scene build
//! (`pick::pending`) only keeps a copy of the marker draws' texels, and only for a game with a fur spec; everything else (texture
//! match by content hash, masks, noise, buffers, bind groups) is built by `pick::realise` on the first frame fur is actually drawn,
//! so Fur Off / Lighting Classic / R&C2 cost nothing at load, and switching Fur on in the menu mid-level builds it then. The hero's
//! afterimage copies (the tail of the marker model, `pick::ghost_cut`) get no shells, hidden or not. Without
//! a spec, a match, Lighting Enhanced and a Fur level, nothing runs per frame but one `Option` test. The pipeline is built on the
//! first frame that needs it, inside the build guard (`pipe_guard::Effect::Fur`).
//!
//! Modules: `pick` (texture to part, normal welding, the lazy build), `params` (per-part uniform values), `pipes` (shader module,
//! pipelines, bind group layout), `noise` (the strand coverage texture); `fur.wgsl` is the shader source appended to the level shader.

use super::*;
use crate::furspec::FurSpec;
use std::sync::atomic::{AtomicU16, Ordering};
use std::sync::OnceLock;

mod noise;
mod params;
mod pick;
mod pipes;
#[cfg(test)]
mod tests;

pub(crate) use pick::pending;

/// The fur of one scene's marker: what scene build kept (`pick::pending`) and the GPU side, built on first use (`pick::realise`).
pub(crate) struct FurScene {
	/// Per marker draw: its texture key and texels.
	draws: Vec<Option<(u32, crate::level::Texture)>>,
	/// Per marker draw: how many of its indices get fur shells, the ones before the first face of the afterimage tail
	/// (`pick::ghost_cut`); empty = all of them (the draw list did not match the face list).
	cut: Vec<u32>,
	gpu: OnceLock<Option<FurMarker>>,
}

/// The built fur of a marker: the part of each marker draw (index into the spec's parts + 1; 0 = no fur), the per-part uniforms
/// (one 256-byte slot per part, dynamic offset) and the bind group (parts, the marker's vertex buffer as storage, the weld table).
pub(crate) struct FurMarker {
	part: Vec<u8>,
	params: wgpu::Buffer,
	bind: Vec<Option<wgpu::BindGroup>>,
	weld: bool,
	/// The (level, style) the `params` buffer holds (0 = not written yet), so a steady frame uploads nothing.
	written: AtomicU16,
	_keep: Vec<wgpu::Buffer>,
}

/// The fur pipelines of the window (`Gpu::fur`): built once, on the first frame with fur.
#[derive(Default)]
pub(crate) struct FurGpu {
	pipes: OnceLock<Option<pipes::Pipes>>,
	/// The bind group layout of group 2, shared by the pipeline and every scene's bind groups.
	layout: OnceLock<wgpu::BindGroupLayout>,
}

/// The effective level and style (true = Fluffy): Graphics, overridden by RAC_FUR=off|low|medium|high and RAC_FUR_STYLE=soft|fluffy
/// (tests; the old name "long" = fluffy).
pub(crate) fn level(g: crate::gfx::Graphics) -> (u8, bool) {
	static ENV: OnceLock<(Option<u8>, Option<bool>)> = OnceLock::new();
	let (q, fluffy) = *ENV.get_or_init(|| {
		let q = std::env::var("RAC_FUR").ok().and_then(|v| ["off", "low", "medium", "high"].iter().position(|n| v.eq_ignore_ascii_case(n)).map(|p| p as u8));
		let fluffy = std::env::var("RAC_FUR_STYLE").ok().and_then(|v| ["soft", "fluffy", "long"].iter().position(|n| v.eq_ignore_ascii_case(n)).map(|p| p > 0));
		(q, fluffy)
	});
	(q.unwrap_or(g.fur_q).min(3), fluffy.unwrap_or(g.fur_long))
}

/// Fur shells are drawn only for a shown marker outside the warp frame whose hero is not collapsed to the feet (hidden hero with a
/// loose wrench kept: its collapsed vertices have up-normals, the shells would show as slivers at his feet).
fn fur_wanted(marker_shown: bool, warp: bool, collapsed: bool) -> bool {
	marker_shown && !warp && !collapsed
}

#[cfg(test)]
mod wanted_tests {
	use super::fur_wanted;
	#[test]
	fn no_fur_on_the_collapsed_hero() {
		assert!(fur_wanted(true, false, false), "the shown hero has fur");
		assert!(!fur_wanted(true, false, true), "collapsed (hidden hero, wrench in view): no shells");
		assert!(!fur_wanted(false, false, false), "no marker");
		assert!(!fur_wanted(true, true, false), "warp frame");
	}
}

impl Gpu {
	/// The fur shells of the marker, blended over the finished world pass (called at the end of `pass_world`; `warp` = the warp
	/// frame, which hides the marker). Leaves the fur pipeline, group 2 and the marker's buffers set: every later draw of the level
	/// pass sets its own pipeline, group 1 and buffers (read 2026-10-31: oceans, world quads, art, shop models, particles), group 0
	/// is the same `ubind` this sets, and no later pipeline reads group 2.
	pub(in crate::viewer) fn fur_draw(&self, pass: &mut wgpu::RenderPass<'_>, scene: &Scene, warp: bool) {
		let Some(fs) = scene.marker_fur.as_ref().filter(|_| fur_wanted(scene.marker_shown, warp, self.marker_fur_off)) else { return };
		let g = crate::gfx::graphics();
		let (q, fluffy) = level(g);
		let Some(spec) = (crate::pack::get().fur)().filter(|_| crate::splitview::fx(g.enhanced() && q > 0) && !pipe_guard::off(pipe_guard::Effect::Fur)) else { return };
		let layout = self.fur.layout.get_or_init(|| pipes::layout(&self.device));
		let Some(p) = self.fur.pipes.get_or_init(|| pipe_guard::guard(&self.device, pipe_guard::Effect::Fur, || pipes::Pipes::new(&self.device, &self.pipeline, &self.tex_layout, layout, self.config.format, self.depth_format))).as_ref() else { return };
		let Some(fm) = fs.gpu.get_or_init(|| pick::realise(&self.device, &self.queue, fs, &scene.marker, &scene.marker_model, layout)).as_ref() else { return };
		let key = 1 | (q as u16) << 1 | (fluffy as u16) << 4;
		if fm.written.swap(key, Ordering::Relaxed) != key {
			self.queue.write_buffer(&fm.params, 0, &params::block(spec, q, fluffy, fm.weld));
		}
		pass.set_pipeline(&p.pipeline);
		pass.set_bind_group(0, &self.ubind, &[]);
		pass.set_vertex_buffer(0, scene.marker.vbuf.slice(..));
		pass.set_index_buffer(scene.marker.ibuf.slice(..), wgpu::IndexFormat::Uint32);
		let shells = crate::furspec::shells(q, fluffy);
		for (di, ((d, &part), bind)) in scene.marker.draws.iter().zip(&fm.part).zip(&fm.bind).enumerate() {
			let Some(bind) = bind.as_ref().filter(|_| part != 0) else { continue };
			// The hero's afterimage copies (hidden or not) are the tail of each draw's index range: no shells for them.
			let count = pick::fur_count(&fs.cut, di, d.count);
			if count == 0 {
				continue;
			}
			pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
			pass.set_bind_group(2, bind, &[(part as u32 - 1) * params::SLOT]);
			pass.draw_indexed(d.start..d.start + count, 0, 0..shells);
		}
	}
}

/// The spec of the running game, if it has fur (`Pack::fur`).
pub(crate) fn spec() -> Option<&'static FurSpec> {
	(crate::pack::get().fur)()
}
