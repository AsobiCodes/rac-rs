//! HUD art textures and the retail overlay texture.

use super::*;

impl Gpu {
	/// Replaces the level's HUD art (overlay slots from 1; slot 0 stays the built-in font).
	pub(in crate::viewer) fn set_hud_art(&mut self, ts: &[Texture]) {
		self.hud_textures.truncate(1);
		for t in ts {
			let b = self.upload(Some(t), &self.nearest);
			self.hud_textures.push(b);
		}
	}

	/// Uploads `t` into HUD slot `slot` (None = a new slot after the others); returns the slot.
	pub(in crate::viewer) fn hud_texture(&mut self, slot: Option<usize>, t: &Texture) -> usize {
		let b = self.upload(Some(t), &self.nearest);
		match slot.filter(|&s| s < self.hud_textures.len()) {
			Some(s) => {
				self.hud_textures[s] = b;
				s
			}
			None => {
				self.hud_textures.push(b);
				self.hud_textures.len() - 1
			}
		}
	}

	/// Retail mode: replaces the GS overlay image (`w` x `rows` RGBA8) and takes the guard measured on it (`retail_ov_guard`). `live` =
	/// the picture is gameplay's (`hud_place::live`); any other picture (menus, shop, cutscenes, front end) holds no guard (hud_hold.rs).
	pub(in crate::viewer) fn set_retail_overlay(&mut self, w: u32, rows: u32, px: &[[u8; 4]], guard: crate::retail::HudGuard, live: bool) {
		let tex = self.overlay_texture(w, rows, guard, live);
		let _sw = super::profile::span("ov_write");
		self.queue.write_texture(
			tex.as_image_copy(),
			bytemuck::cast_slice(px),
			wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(rows) },
			wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
		);
	}

	/// Like `set_retail_overlay`, but the texture write runs on the `ov-write` helper (ovwrite.rs: staged before this frame's final
	/// submit, which waits for it); RAC_OV_ASYNC=0 writes here.
	pub(in crate::viewer) fn set_retail_overlay_async(&mut self, w: u32, rows: u32, px: Vec<[u8; 4]>, guard: crate::retail::HudGuard, live: bool) {
		let tex = self.overlay_texture(w, rows, guard, live);
		self.write_overlay(&tex, w, rows, px);
	}

	/// Takes the guard, (re)creates the overlay texture when the size changed and returns it (a cheap handle clone).
	fn overlay_texture(&mut self, w: u32, rows: u32, guard: crate::retail::HudGuard, live: bool) -> wgpu::Texture {
		// A write still on the helper (an earlier frame's) must be staged before the sync path or a new texture goes on.
		ovwrite::join_to(ovwrite::issued());
		self.retail_ov_guard.next(guard, crate::retail::disc::usa(), live);
		if !self.retail_ov.as_ref().is_some_and(|o| o.2 == w && o.3 == rows) {
			let tex = self.device.create_texture(&wgpu::TextureDescriptor {
				label: Some("retail overlay"),
				size: wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: wgpu::TextureFormat::Rgba8Unorm,
				usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
				view_formats: &[],
			});
			let view = tex.create_view(&Default::default());
			let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: None,
				layout: &self.tex_layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&self.clamp_sampler) },
				],
			});
			self.retail_ov = Some((tex, bind, w, rows, None));
		}
		self.retail_ov.as_ref().unwrap().0.clone()
	}

	/// Whether a retail overlay image has arrived.
	pub(in crate::viewer) fn has_retail_overlay(&self) -> bool {
		self.retail_ov.is_some()
	}
}
