//! [PC] Render scale (v0.4.1, players asked for a resolution option): the game draws at a percentage of the window size into an
//! offscreen colour picture, then one extra pass copies it onto the window image (linear filter, rscale.wgsl).
//! `gpu.config` keeps meaning "the size everything is drawn at" (about 40 sites read it), so with a scale it holds the RENDER
//! size; the swapchain is configured at the WINDOW size (`window_config`) and the window size lives here (`Gpu::win_size`:
//! the mouse, the screenshot readback). At 100 % there is no offscreen picture and no extra pass: the frame takes the same
//! path as before. The ReShade add-on runs only at 100 % (its depth and picture are the window's own).
//! The wanted percentage comes from ONE place, `wanted_pct`: env RAC_RENDER_SCALE if set, else the menu value (the Render scale
//! row of the Graphics page, `retail::pcdisp::render_scale_pct`). A change applies at once (`Gpu::rscale_sync`, first thing in a frame).

use super::*;
use std::sync::OnceLock;

const SHADER: &str = include_str!("rscale.wgsl");

/// The percentages the menu offers.
pub const CHOICES: [u32; 6] = [50, 75, 100, 125, 150, 200];
/// What an env value or a menu value is clamped to (a typo never asks for a 1 px or a 40 000 px picture).
const MIN_PCT: u32 = 25;
const MAX_PCT: u32 = 400;

/// The render scale in percent: RAC_RENDER_SCALE (50, "75%") wins, else the menu value (retail/pcdisp.rs, default 100).
pub fn wanted_pct() -> u32 {
	static ENV: OnceLock<Option<u32>> = OnceLock::new();
	ENV.get_or_init(|| parse_pct(&std::env::var("RAC_RENDER_SCALE").ok()?)).unwrap_or_else(|| clamp_pct(crate::retail::pcdisp::render_scale_pct()))
}

fn parse_pct(s: &str) -> Option<u32> {
	s.trim().trim_end_matches('%').trim().parse::<u32>().ok().map(clamp_pct)
}

fn clamp_pct(p: u32) -> u32 {
	p.clamp(MIN_PCT, MAX_PCT)
}

/// The render size for a window size: round(w * pct / 100) x round(h * pct / 100), at least 1 px, and if the device cannot make a
/// texture that big (`max` = its max_texture_dimension_2d) both sides shrink by the same factor so the shape stays.
pub fn render_size(win: (u32, u32), pct: u32, max: u32) -> (u32, u32) {
	let side = |v: u32| ((v as u64 * pct as u64 + 50) / 100).max(1);
	let (mut w, mut h) = (side(win.0), side(win.1));
	let max = max.max(1) as u64;
	let big = w.max(h);
	if big > max {
		w = (w * max + big / 2) / big;
		h = (h * max + big / 2) / big;
	}
	(w.clamp(1, max) as u32, h.clamp(1, max) as u32)
}

/// Start of `Gpu::new`: the window size `config` was made with; `config` then takes the render size of the wanted scale.
pub(super) fn start_size(device: &wgpu::Device, config: &mut wgpu::SurfaceConfiguration) -> (u32, u32) {
	let win = (config.width, config.height);
	(config.width, config.height) = render_size(win, wanted_pct(), device.limits().max_texture_dimension_2d);
	win
}

/// The swapchain configuration: `config` (render size) at the window size.
pub fn window_config(config: &wgpu::SurfaceConfiguration, win: (u32, u32)) -> wgpu::SurfaceConfiguration {
	wgpu::SurfaceConfiguration { width: win.0.max(1), height: win.1.max(1), ..config.clone() }
}

struct Target {
	view: wgpu::TextureView,
	bind: wgpu::BindGroup,
}

struct Pipe {
	pipeline: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	sampler: wgpu::Sampler,
}

/// The Gpu's render-scale state: the window size, the percentage in effect, and (only while the render size differs from the
/// window size) the offscreen picture and the copy pipeline.
#[derive(Default)]
pub(in crate::viewer) struct RScale {
	win: (u32, u32),
	pct: u32,
	target: Option<Target>,
	pipe: Option<Pipe>,
}

impl RScale {
	pub(in crate::viewer) fn scaled(&self) -> bool {
		self.target.is_some()
	}
}

fn make_pipe(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipe {
	let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
		label: Some("render scale"),
		entries: &[
			wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
			wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
		],
	});
	let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("render scale"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
	let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("render scale"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("render scale", SHADER.into())) });
	let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
		label: Some("render scale"),
		layout: Some(&pl),
		vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_blit"), compilation_options: Default::default(), buffers: &[] },
		fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_blit"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
		primitive: Default::default(),
		depth_stencil: None,
		multisample: Default::default(),
		multiview_mask: None,
		cache: None,
	});
	let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
		label: Some("render scale"),
		address_mode_u: wgpu::AddressMode::ClampToEdge,
		address_mode_v: wgpu::AddressMode::ClampToEdge,
		mag_filter: wgpu::FilterMode::Linear,
		min_filter: wgpu::FilterMode::Linear,
		..Default::default()
	});
	Pipe { pipeline, layout, sampler }
}

fn make_target(device: &wgpu::Device, pipe: &Pipe, format: wgpu::TextureFormat, size: (u32, u32)) -> Target {
	let tex = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("render scale"),
		size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
		view_formats: &[],
	});
	let view = tex.create_view(&Default::default());
	let bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
		label: Some("render scale"),
		layout: &pipe.layout,
		entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&pipe.sampler) }],
	});
	Target { view, bind }
}

impl Gpu {
	/// The window's size in pixels (what mouse positions and the screenshot readback use); `config` is the render size.
	pub(in crate::viewer) fn win_size(&self) -> (u32, u32) {
		let w = self.rscale.win;
		if w.0 == 0 { (self.config.width, self.config.height) } else { w }
	}

	/// The offscreen view the frame is drawn into (None at 100 %: the window image itself).
	pub(super) fn rscale_view(&self) -> Option<wgpu::TextureView> {
		self.rscale.target.as_ref().map(|t| t.view.clone())
	}

	/// Window size `win` and the percentage in effect: the render size for them, with the offscreen picture made or dropped.
	pub(super) fn rscale_plan(&mut self, win: (u32, u32)) -> (u32, u32) {
		let pct = wanted_pct();
		let render = render_size(win, pct, self.device.limits().max_texture_dimension_2d);
		self.rscale.win = win;
		self.rscale.pct = pct;
		if render == win {
			self.rscale.target = None;
		} else {
			let format = self.config.format;
			// [PC] pipe_guard.rs: a driver that refuses the blit shader (DX12 FXC) draws at the window size, no crash.
			if self.rscale.pipe.is_none() {
				self.rscale.pipe = crate::viewer::pipe_guard::guard(&self.device, crate::viewer::pipe_guard::Effect::RenderScale, || make_pipe(&self.device, format));
			}
			let Some(pipe) = &self.rscale.pipe else {
				self.rscale.target = None;
				return win;
			};
			self.rscale.target = Some(make_target(&self.device, pipe, format, render));
		}
		render
	}

	/// First thing in a frame: a changed scale (menu or env) re-plans the sizes at once.
	pub(in crate::viewer) fn rscale_sync(&mut self) {
		if self.rscale.pct != 0 && self.rscale.pct != wanted_pct() {
			let win = self.win_size();
			self.resize(win.0, win.1);
		}
	}

	/// The swapchain at the window size (the render size lives in `config`).
	pub(super) fn reconfigure_surface(&self) {
		super::setup::configure_surface(&self.surface, &self.device, &window_config(&self.config, self.win_size()));
	}

	/// Copies the offscreen picture onto the window image `dst` (nothing at 100 %).
	pub(super) fn rscale_blit(&self, enc: &mut wgpu::CommandEncoder, dst: &wgpu::TextureView) {
		let (Some(t), Some(p)) = (&self.rscale.target, &self.rscale.pipe) else { return };
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("render scale"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: dst, depth_slice: None, resolve_target: None, ops: wgpu::Operations { load: crate::viewer::post::overwrite(), store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&p.pipeline);
		pass.set_bind_group(0, &t.bind, &[]);
		pass.draw(0..3, 0..1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn render_size_rounds_and_keeps_100_exact() {
		assert_eq!(render_size((1920, 1080), 100, 8192), (1920, 1080));
		assert_eq!(render_size((1920, 1080), 50, 8192), (960, 540));
		assert_eq!(render_size((1920, 1080), 75, 8192), (1440, 810));
		assert_eq!(render_size((1920, 1080), 200, 8192), (3840, 2160));
		assert_eq!(render_size((1281, 721), 50, 8192), (641, 361), "half rounds up");
		assert_eq!(render_size((1280, 720), 125, 8192), (1600, 900));
		assert_eq!(render_size((1, 1), 25, 8192), (1, 1), "never zero");
	}

	#[test]
	fn render_size_clamps_to_the_device_and_keeps_the_shape() {
		let (w, h) = render_size((3840, 2160), 400, 8192);
		assert_eq!(w, 8192);
		assert!((h as i64 - 4608).abs() <= 1, "16:9 kept: {h}");
		assert_eq!(render_size((3840, 2160), 200, 8192), (7680, 4320), "fits");
		assert_eq!(render_size((100, 100), 400, 256), (256, 256));
	}

	#[test]
	fn pct_parse_and_clamp() {
		assert_eq!(parse_pct("50"), Some(50));
		assert_eq!(parse_pct(" 75% "), Some(75));
		assert_eq!(parse_pct("1"), Some(MIN_PCT));
		assert_eq!(parse_pct("99999"), Some(MAX_PCT));
		assert_eq!(parse_pct("fifty"), None);
		assert!(CHOICES.iter().all(|&c| clamp_pct(c) == c));
	}

	#[test]
	fn menu_value_is_used_without_env() {
		let _g = crate::retail::pcopts::globals_guard();
		if std::env::var_os("RAC_RENDER_SCALE").is_none() {
			assert_eq!(wanted_pct(), clamp_pct(crate::retail::pcdisp::render_scale_pct()));
		}
	}

	#[test]
	fn window_config_only_changes_the_size() {
		let c = wgpu::SurfaceConfiguration { usage: wgpu::TextureUsages::RENDER_ATTACHMENT, format: wgpu::TextureFormat::Bgra8Unorm, width: 960, height: 540, present_mode: wgpu::PresentMode::Fifo, desired_maximum_frame_latency: 2, alpha_mode: wgpu::CompositeAlphaMode::Auto, color_space: Default::default(), view_formats: vec![] };
		let w = window_config(&c, (1920, 1080));
		assert_eq!((w.width, w.height, w.format), (1920, 1080, c.format));
		assert_eq!(window_config(&c, (0, 0)).width, 1);
	}

	#[test]
	fn shader_validates() {
		let m = naga::front::wgsl::parse_str(SHADER).expect("wgsl parse");
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("wgsl validate");
	}
}
