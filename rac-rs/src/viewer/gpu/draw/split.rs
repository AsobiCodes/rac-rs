//! [PC] Split view: the split itself (retail/splitview.rs, Debug mode F10), the Classic half of the frame. `split_classic` records and submits a full
//! second pass of the frame into an offscreen target while the split switch is Classic (`splitview`) (so `pcext::graphics()` answers Classic
//! and the volumetric clouds, fog and history of the Enhanced pass stay untouched); `split_extras` then adds the quads that lay
//! that picture over the left of the divider, the divider and the labels to the Enhanced pass's HUD (`pass_hud` draws SPLIT_TEX).
//! With a texture pack the Classic pass draws the disc's original textures (`swap_originals`, gpu/split_orig.rs).
//! The Classic pass is submitted before the Enhanced one records: both share the uniform, tie and argument buffers.

use super::*;
use std::cell::RefCell;

/// The offscreen Classic picture: its size and format, the render view and the bind group the HUD pass samples.
struct Target {
	key: (u32, u32, wgpu::TextureFormat),
	view: wgpu::TextureView,
	bind: wgpu::BindGroup,
}

thread_local! {
	static TARGET: RefCell<Option<Target>> = const { RefCell::new(None) };
	/// The Classic picture of this frame is ready (set by `split_classic`, taken by `Gpu::draw`).
	static READY: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Takes this frame's "Classic picture ready" flag.
pub(super) fn take_ready() -> bool {
	READY.with(|r| r.replace(false))
}

/// The render view of the Classic picture (None before the first Classic pass).
pub(super) fn view() -> Option<wgpu::TextureView> {
	TARGET.with(|t| t.borrow().as_ref().map(|t| t.view.clone()))
}

/// The bind group of the Classic picture for the HUD pass.
pub(super) fn bind() -> Option<wgpu::BindGroup> {
	TARGET.with(|t| t.borrow().as_ref().map(|t| t.bind.clone()))
}

impl Gpu {
	/// The Classic half of this frame, when the split view is on and the two halves would differ (`take_ready` says if the
	/// picture is ready). `redo` re-runs what the handler computed with the player's draw distance (the props pick, `set_props`
	/// with the view): once with the Classic flag up before the pass, once more after it so the Enhanced pass sees the real pick.
	pub(in crate::viewer) fn split_classic(&mut self, cam: &Camera, redo: &mut dyn FnMut(&mut Gpu)) {
		use crate::retail::splitview as sv;
		if !sv::active() || self.scene.is_none() || !(sv::differs(crate::retail::pcext::graphics(), self.modern_ok) || (self.modern_ok && self.has_originals())) {
			return;
		}
		let _s = super::profile::span("d_split");
		self.split_target();
		sv::set_classic(true);
		sv::set_redo(true);
		redo(self);
		sv::set_redo(false);
		self.swap_originals(); // texture pack: the disc's own textures in the Classic half (gpu/split_orig.rs)
		let done = self.split_classic_pass(cam).is_some();
		self.swap_originals();
		sv::set_classic(false);
		sv::set_redo(true);
		redo(self);
		sv::set_redo(false);
		READY.with(|r| r.set(done));
	}

	/// Split re-run: the clip view (props clipper, far plane) of the drawn camera under the current pass's draw distance.
	pub(in crate::viewer) fn redo_clip_view(&mut self, cam: &Camera) {
		let (aspect, wfar) = (self.config.width as f32 / self.config.height.max(1) as f32, self.world_far());
		if let Some(scene) = self.scene.as_mut().filter(|s| s.clip_view.is_some()) {
			scene.clip_view = Some(crate::viewer::clip::ClipView::new(cam, aspect).with_far(wfar));
		}
	}

	/// (Re)makes the offscreen target at the window size.
	fn split_target(&self) {
		let key = (self.config.width.max(1), self.config.height.max(1), self.config.format);
		if TARGET.with(|t| t.borrow().as_ref().is_some_and(|t| t.key == key)) {
			return;
		}
		let tex = self.device.create_texture(&wgpu::TextureDescriptor {
			label: Some("split classic"),
			size: wgpu::Extent3d { width: key.0, height: key.1, depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: key.2,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		});
		let view = tex.create_view(&Default::default());
		let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("split classic"),
			layout: &self.tex_layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.nearest) },
			],
		});
		TARGET.with(|t| *t.borrow_mut() = Some(Target { key, view, bind }));
	}

	/// The frame's steps with the Classic flag up, ending in a submit of its own (no acquire, no present).
	fn split_classic_pass(&mut self, cam: &Camera) -> Option<()> {
		let dc = self.frame_uniforms(cam)?;
		let mut dfx = self.frame_effects(cam, &dc)?;
		let (mut dt, warp_hud) = self.frame_target(cam, &dc)?;
		let db = self.frame_buffers(warp_hud)?;
		self.draw_passes(cam, &dc, &mut dfx, &mut dt, &db)?;
		let DrawTarget { enc, cuts, .. } = dt;
		cuts.submit(self.queue.sync(), enc.finish());
		Some(())
	}

	/// The split view's quads for the Enhanced pass (nothing when it is off).
	pub(super) fn split_extras(&self, out: &mut Vec<Quad>, classic_ready: bool) {
		crate::retail::splitview::extras(out, self.config.width as f32, self.config.height as f32, classic_ready);
	}
}
