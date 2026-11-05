//! [PC] The achievement icon atlas on the GPU (engine/achievements/icons.rs): decoded and uploaded ONCE, at the first frame that draws
//! a quad with `ICON_TEX` (the unlock pop-up or the list), with the scene's linear sampler and mips (the icons are drawn 64 px at 1080
//! lines from 128 px tiles). `pass_hud` draws the quads with the plain HUD pipelines. No new shader: DX12 and Vulkan share the path.

use super::*;

/// The atlas bind group, once made. `tried` stops a failed build (a bad icon file, logged once) from repeating every frame.
#[derive(Default)]
pub(in crate::viewer) struct AchIcons {
	pub(super) bind: Option<wgpu::BindGroup>,
	tried: bool,
}

impl Gpu {
	/// Makes the atlas when this frame's overlay holds an icon quad and it was not tried yet.
	pub(super) fn ach_icons_ensure(&mut self) {
		if self.ach_icons.tried || !crate::achievements::available() || !self.hud.iter().any(|q| q.tex == crate::achievements::ICON_TEX) {
			return;
		}
		self.ach_icons.tried = true;
		let _s = super::profile::span("ach_icons");
		if let Some(t) = crate::achievements::atlas() {
			self.ach_icons.bind = Some(self.upload(Some(&t), &self.sampler));
		}
	}
}
