//! The GPU side of the backdrop (numbers and easing: `backdrop.rs`). The window image is copied into level 0 of a mip chain `m`,
//! halved `k` times level by level (each a render pass of 2x2 averages), blurred along x into `a` and along y into `b` at that small
//! size, and `b` is drawn back up under the scrim by the main UI pass (`bg_comp`). `a` and `b` are half the window size (levels
//! 0.. are the sizes of `m`'s levels 1..), so the blur chain never costs a full-size texture. Made on the first menu frame for the
//! window size, dropped when the menu closes or the window changes size.
use super::backdrop::{kernel, MAX_K};
use super::gpu_pipes::{bind, Params, Pipes};
use crate::fmv::present::QueueWrites;

pub struct BackRes {
	pub win: (u32, u32),
	/// The most halvings this window size allows (levels of `m` minus one, at most `MAX_K`).
	pub k_max: u32,
	m: wgpu::Texture,
	m_views: Vec<wgpu::TextureView>,
	a_views: Vec<wgpu::TextureView>,
	b_views: Vec<wgpu::TextureView>,
	bg_down: Vec<wgpu::BindGroup>,
	bg_h: Vec<wgpu::BindGroup>,
	bg_v: Vec<wgpu::BindGroup>,
	bg_comp: Vec<wgpu::BindGroup>,
	u_h: wgpu::Buffer,
	u_v: wgpu::Buffer,
	u_comp: wgpu::Buffer,
}

fn level(v: u32, k: u32) -> u32 {
	(v >> k).max(1)
}

fn tex(device: &wgpu::Device, label: &str, size: (u32, u32), levels: u32, format: wgpu::TextureFormat, extra: wgpu::TextureUsages) -> wgpu::Texture {
	device.create_texture(&wgpu::TextureDescriptor {
		label: Some(label),
		size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
		mip_level_count: levels,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT | extra,
		view_formats: &[],
	})
}

fn views(t: &wgpu::Texture, n: u32) -> Vec<wgpu::TextureView> {
	(0..n).map(|i| t.create_view(&wgpu::TextureViewDescriptor { base_mip_level: i, mip_level_count: Some(1), ..Default::default() })).collect()
}

fn uniform(device: &wgpu::Device, label: &str) -> wgpu::Buffer {
	device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: std::mem::size_of::<Params>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })
}

fn pass(enc: &mut wgpu::CommandEncoder, label: &str, view: &wgpu::TextureView, pipe: &wgpu::RenderPipeline, bg: &wgpu::BindGroup) {
	let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
		label: Some(label),
		color_attachments: &[Some(wgpu::RenderPassColorAttachment { view, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
		depth_stencil_attachment: None,
		timestamp_writes: None,
		occlusion_query_set: None,
		multiview_mask: None,
	});
	p.set_pipeline(pipe);
	p.set_bind_group(0, bg, &[]);
	p.draw(0..3, 0..1);
}

impl BackRes {
	/// None when the window is too small to halve (under 2 px).
	pub fn new(device: &wgpu::Device, pipes: &Pipes, format: wgpu::TextureFormat, win: (u32, u32)) -> Option<BackRes> {
		let big = win.0.max(win.1);
		if win.0.min(win.1) < 2 {
			return None;
		}
		let max_levels = 32 - big.leading_zeros(); // floor(log2) + 1
		let k_max = (max_levels - 1).min(MAX_K);
		let m = tex(device, "ui backdrop", win, k_max + 1, format, wgpu::TextureUsages::COPY_DST);
		let half = (level(win.0, 1), level(win.1, 1));
		let a = tex(device, "ui blur a", half, k_max, format, wgpu::TextureUsages::empty());
		let b = tex(device, "ui blur b", half, k_max, format, wgpu::TextureUsages::empty());
		let (m_views, a_views, b_views) = (views(&m, k_max + 1), views(&a, k_max), views(&b, k_max));
		let (u_h, u_v, u_comp) = (uniform(device, "ui blur x"), uniform(device, "ui blur y"), uniform(device, "ui backdrop"));
		// Pass i reads level i-1 and writes level i; blur k reads m[k] -> a[k-1], then a[k-1] -> b[k-1]; the composite reads b[k-1].
		let bg_down = (1..=k_max).map(|i| bind(device, pipes, &m_views[i as usize - 1], &u_comp)).collect();
		let bg_h = (1..=k_max).map(|k| bind(device, pipes, &m_views[k as usize], &u_h)).collect();
		let bg_v = (1..=k_max).map(|k| bind(device, pipes, &a_views[k as usize - 1], &u_v)).collect();
		let bg_comp = (1..=k_max).map(|k| bind(device, pipes, &b_views[k as usize - 1], &u_comp)).collect();
		Some(BackRes { win, k_max, m, m_views, a_views, b_views, bg_down, bg_h, bg_v, bg_comp, u_h, u_v, u_comp })
	}

	/// The composite's bind group for blur level `k` (1..=k_max), drawn by the main pass.
	pub fn comp_group(&self, k: u32) -> &wgpu::BindGroup {
		&self.bg_comp[k as usize - 1]
	}

	/// Copies `window` into the chain, halves it `k` times and blurs it with a Gaussian of `sigma` texels (of level `k`).
	/// Also writes the composite's uniforms (`scrim` rgba 0..1, `dither` strength).
	#[allow(clippy::too_many_arguments)]
	pub fn encode(&self, enc: &mut wgpu::CommandEncoder, q: &dyn QueueWrites, pipes: &Pipes, window: &wgpu::Texture, k: u32, sigma: f32, scrim: [f32; 4], dither: f32) {
		let k = k.clamp(1, self.k_max);
		enc.copy_texture_to_texture(window.as_image_copy(), self.m.as_image_copy(), wgpu::Extent3d { width: self.win.0, height: self.win.1, depth_or_array_layers: 1 });
		for i in 1..=k {
			pass(enc, "ui down", &self.m_views[i as usize], &pipes.down, &self.bg_down[i as usize - 1]);
		}
		let w = kernel(sigma);
		let weights = |step: [f32; 2]| Params { a: [step[0], step[1], 0.0, 0.0], w0: [w[0], w[1], w[2], w[3]], w1: [w[4], w[5], w[6], w[7]], scrim: [0.0; 4] };
		let (kw, kh) = (level(self.win.0, k) as f32, level(self.win.1, k) as f32);
		q.write_buffer(&self.u_h, 0, bytemuck::bytes_of(&weights([1.0 / kw, 0.0])));
		q.write_buffer(&self.u_v, 0, bytemuck::bytes_of(&weights([0.0, 1.0 / kh])));
		q.write_buffer(&self.u_comp, 0, bytemuck::bytes_of(&Params { a: [0.0, 0.0, 0.0, dither], scrim, ..Default::default() }));
		let ki = k as usize - 1;
		pass(enc, "ui blur x", &self.a_views[ki], &pipes.blur, &self.bg_h[ki]);
		pass(enc, "ui blur y", &self.b_views[ki], &pipes.blur, &self.bg_v[ki]);
	}
}
