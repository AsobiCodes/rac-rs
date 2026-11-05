//! The level pass's render pass (moved out of `draw_passes`, a pure move: the attachments and clear values are unchanged).

use super::*;

impl Gpu {
	/// Begins the level pass: the MSAA colour target resolving into the post scene (or the frame), depth and stencil cleared.
	/// `flags` = (post_on, part_on, water_on, rs): whether the colour result is stored.
	pub(super) fn begin_level_pass<'e>(&self, enc: &'e mut wgpu::CommandEncoder, scene: &Scene, view: &wgpu::TextureView, flags: (bool, bool, bool, bool)) -> wgpu::RenderPass<'e> {
		let (post_on, part_on, water_on, rs) = flags;
		enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("level"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: &self.msaa,
				resolve_target: Some(self.post.scene().filter(|_| post_on).unwrap_or(&view)),
				depth_slice: None,
				ops: wgpu::Operations {
					load: wgpu::LoadOp::Clear(self.warp_fx.as_ref().filter(|_| scene.warp_shown).map_or(scene.clear, |w| w.clear)),
					store: if part_on || water_on || rs { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard },
				},
			})],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
				view: &self.depth,
				depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
				// BRIDGE-64a: the shadow-volume counter starts at 128 every frame.
				stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(super::shadowvol::STENCIL_CLEAR), store: wgpu::StoreOp::Discard }),
			}),
			timestamp_writes: crate::viewer::passtime::rw("level"),
			occlusion_query_set: None,
			multiview_mask: None,
		})
	}
}
