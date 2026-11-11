//! Render targets and buffers: 4x MSAA / depth views, mip chain, buffer creation, the shop item-panel target
//! and the `Quad.tex` keys that select the panel / overlay targets.

use super::*;

/// 4x MSAA for the scene pass (port extra: modern display, no PS2 aliasing); 1 sample in the upscaler cut (scene_msaa.rs).
pub(crate) use super::scene_msaa::msaa;

/// REN-9b: the shop item-panel render target (retail 512x128 at GS 0x3be000): 4x MSAA colour and depth resolved into a
/// sampled texture. `bind` samples the resolved colour; the HUD draws it for `Quad.tex == ITEM_PANEL_TEX`.
pub(crate) struct ItemPanel {
	pub(super) msaa: wgpu::TextureView,
	pub(super) depth: wgpu::TextureView,
	pub(super) resolved: wgpu::TextureView,
	pub(super) bind: wgpu::BindGroup,
}
/// REN-9b: the shop's item poses split into world mobies and the panel-1 item (never both).
pub fn split_item_poses(poses: &[ItemPose]) -> (Vec<(u32, [f32; 16])>, Vec<(u32, [f32; 16])>) {
	let pick = |dest| poses.iter().filter(|p| p.dest == dest).map(|p| (p.class, place(p.pos, p.rot))).collect();
	(pick(ItemDest::World), pick(ItemDest::Panel))
}
/// Target size in texels (TW 9, TH 7).
pub(crate) const ITEM_PANEL_SIZE: (u32, u32) = (512, 128);
/// `Quad.tex` value that selects the item-panel target instead of an overlay texture.
pub const ITEM_PANEL_TEX: usize = usize::MAX;
/// BRIDGE-36 (b): `Quad.tex` selecting the purchase popup target.
pub const POPUP_PANEL_TEX: usize = usize::MAX - 1;
/// BRIDGE-37 (b): `Quad.tex` selecting the shop portrait target.
pub const PORTRAIT_PANEL_TEX: usize = usize::MAX - 2;
/// `Quad.tex` selecting the retail GS overlay (retail.rs): drawn with out = rgb + a * scene.
pub const RETAIL_OV_TEX: usize = usize::MAX - 3;
/// `Quad.tex` selecting the retail screen filter's untextured pipeline (retail/queue_b.rs `screen_quads`): GS ALPHA
/// 0x42 out = Cd - Cs (colour premultiplied by As on the CPU), clamped at 0 per channel.
pub const RETAIL_FILTER_TEX: usize = usize::MAX - 4;
/// `Quad.tex` selecting the sharp HUD picture (`Gpu::set_retail_hd`: the HUD drawn at the window's resolution by the GS capture,
/// R&C2): the same blend as `RETAIL_OV_TEX` (out = rgb + a * what is below), no effect depth, drawn after the overlay quad.
pub const RETAIL_HD_TEX: usize = usize::MAX - 5;
/// `Quad.tex` selecting the menu art picture (`Gpu::set_menu_art`: a full-page painting a modern menu page lays over the world, opaque,
/// texture colour times the quad's colour; a pack hands the picture over, the engine only draws it).
pub const MENU_ART_TEX: usize = usize::MAX - 6;
/// `Quad.tex` selecting the menu art picture drawn by the animation pipeline (gpu/menu_art_anim.rs `set_menu_art_anim`): the same
/// painting as `MENU_ART_TEX`, with its disc turning, core breathing and stars twinkling. Falls back to the plain draw without a set
/// animation.
pub const MENU_ART_ANIM_TEX: usize = usize::MAX - 7;

/// The multisampled colour target, resolved into the swapchain image.
pub(crate) fn msaa_view(device: &wgpu::Device, format: wgpu::TextureFormat, w: u32, h: u32) -> wgpu::TextureView {
	device
		.create_texture(&wgpu::TextureDescriptor {
			label: Some("msaa"),
			size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: msaa(),
			dimension: wgpu::TextureDimension::D2,
			format,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | super::scene_msaa::copy_usage(), // lane noaa: resolve copies at 1 sample
			view_formats: &[],
		})
		.create_view(&Default::default())
}

/// Box-filtered mip chain below `t` (level 0 excluded); alpha is averaged too.
pub(crate) fn mip_chain(t: &Texture) -> Vec<(u32, u32, Vec<[u8; 4]>)> {
	let mut out = Vec::new();
	let (mut w, mut h, mut px) = (t.w as u32, t.h as u32, t.rgba.clone());
	while w > 1 || h > 1 {
		let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
		let mut next = Vec::with_capacity((nw * nh) as usize);
		for y in 0..nh {
			for x in 0..nw {
				let mut acc = [0u32; 4];
				for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
					let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
					let c = px[(sy * w + sx) as usize];
					for i in 0..4 {
						acc[i] += c[i] as u32;
					}
				}
				next.push(acc.map(|v| ((v + 2) / 4) as u8));
			}
		}
		out.push((nw, nh, next.clone()));
		(w, h, px) = (nw, nh, next);
	}
	out
}

pub(crate) fn depth_view(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::TextureView {
	device
		.create_texture(&wgpu::TextureDescriptor {
			label: Some("depth"),
			size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: msaa(),
			dimension: wgpu::TextureDimension::D2,
			format,
			// [PC] Sampled by the Enhanced image (post.rs): its occlusion reads the level's depth.
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		})
		.create_view(&Default::default())
}

pub(crate) fn buffer(device: &wgpu::Device, label: &str, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
	let buf = device.create_buffer(&wgpu::BufferDescriptor {
		label: Some(label),
		size: (data.len() as u64).max(4).next_multiple_of(4),
		usage: usage | wgpu::BufferUsages::COPY_DST,
		mapped_at_creation: false,
	});
	buf
}

/// NOV-13.3: one shop model slot: class, world position and Euler rotation (x, y, z; applied x, then y, then z).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ItemPose {
	pub class: u32,
	pub pos: crate::physics::V3,
	pub rot: crate::physics::V3,
	pub dest: ItemDest,
}

/// REN-9: where a shop model is drawn. `World` = the B (ammo row) or C (weapon row) model in the world pass; `Panel`
/// = the A model, rendered into the offscreen item-panel target and shown on panel 1 (BRIDGE-19), never a world moby.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemDest {
	World,
	Panel,
}
