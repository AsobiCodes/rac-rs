//! Render targets and buffers: 4x MSAA / depth views, mip chain, buffer creation, the shop item-panel target
//! and the `Quad.tex` keys that select the panel / overlay targets.

use super::*;

/// 4x MSAA for the scene pass (port extra: modern display, no PS2 aliasing).
pub(super) const MSAA: u32 = 4;

/// REN-9b: the shop item-panel render target (retail 512x128 at GS 0x3be000): 4x MSAA colour and depth resolved into a
/// sampled texture. `bind` samples the resolved colour; the HUD draws it for `Quad.tex == ITEM_PANEL_TEX`.
pub(super) struct ItemPanel {
	pub(super) msaa: wgpu::TextureView,
	pub(super) depth: wgpu::TextureView,
	pub(super) resolved: wgpu::TextureView,
	pub(super) bind: wgpu::BindGroup,
}
/// REN-9b: the shop's item poses split into world mobies and the panel-1 item (never both).
pub(super) fn split_item_poses(poses: &[game::vendor::ItemPose]) -> (Vec<(u32, [f32; 16])>, Vec<(u32, [f32; 16])>) {
	let pick = |dest| poses.iter().filter(|p| p.dest == dest).map(|p| (p.class, place(p.pos, p.rot))).collect();
	(pick(game::vendor::ItemDest::World), pick(game::vendor::ItemDest::Panel))
}
/// Target size in texels (TW 9, TH 7).
pub(super) const ITEM_PANEL_SIZE: (u32, u32) = (512, 128);
/// `Quad.tex` value that selects the item-panel target instead of an overlay texture.
pub(crate) const ITEM_PANEL_TEX: usize = usize::MAX;
/// BRIDGE-36 (b): `Quad.tex` selecting the purchase popup target.
pub(crate) const POPUP_PANEL_TEX: usize = usize::MAX - 1;
/// BRIDGE-37 (b): `Quad.tex` selecting the shop portrait target.
pub(crate) const PORTRAIT_PANEL_TEX: usize = usize::MAX - 2;
/// `Quad.tex` selecting the retail GS overlay (retail.rs): drawn with out = rgb + a * scene.
pub(crate) const RETAIL_OV_TEX: usize = usize::MAX - 3;
/// `Quad.tex` selecting the retail screen filter's untextured pipeline (retail/queue_b.rs `screen_quads`): GS ALPHA
/// 0x42 out = Cd - Cs (colour premultiplied by As on the CPU), clamped at 0 per channel.
pub(crate) const RETAIL_FILTER_TEX: usize = usize::MAX - 4;

/// The multisampled colour target, resolved into the swapchain image.
pub(super) fn msaa_view(device: &wgpu::Device, format: wgpu::TextureFormat, w: u32, h: u32) -> wgpu::TextureView {
	device
		.create_texture(&wgpu::TextureDescriptor {
			label: Some("msaa"),
			size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: MSAA,
			dimension: wgpu::TextureDimension::D2,
			format,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
			view_formats: &[],
		})
		.create_view(&Default::default())
}

/// Box-filtered mip chain below `t` (level 0 excluded); alpha is averaged too.
pub(super) fn mip_chain(t: &Texture) -> Vec<(u32, u32, Vec<[u8; 4]>)> {
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

pub(super) fn depth_view(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::TextureView {
	device
		.create_texture(&wgpu::TextureDescriptor {
			label: Some("depth"),
			size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
			mip_level_count: 1,
			sample_count: MSAA,
			dimension: wgpu::TextureDimension::D2,
			format,
			// [PC] Sampled by the Enhanced image (post.rs): its occlusion reads the level's depth.
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		})
		.create_view(&Default::default())
}

pub(super) fn buffer(device: &wgpu::Device, label: &str, data: &[u8], usage: wgpu::BufferUsages) -> wgpu::Buffer {
	let buf = device.create_buffer(&wgpu::BufferDescriptor {
		label: Some(label),
		size: (data.len() as u64).max(4).next_multiple_of(4),
		usage: usage | wgpu::BufferUsages::COPY_DST,
		mapped_at_creation: false,
	});
	buf
}
