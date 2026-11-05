//! [PC] Render scale (v0.4.1, players asked for a resolution option): the game draws at a percentage of the window size into an
//! offscreen colour picture, then one extra pass copies it onto the window image (linear filter, rscale.wgsl).
//! `gpu.config` keeps meaning "the size everything is drawn at" (about 40 sites read it), so with a scale it holds the RENDER
//! size; the swapchain is configured at the WINDOW size (`window_config`) and the shown size lives here (`Gpu::win_size`: the
//! window, or with an Aspect ratio box the box: the mouse, the menus; the window itself, for the swapchain and the screenshot
//! readback, is `Gpu::real_size`, gpu/aspect_pass.rs). At 100 % there is no offscreen picture and no extra pass: the frame takes the same
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

/// The render scale in percent: RAC_RENDER_SCALE (50, "75%") wins, else the menu value (the pack's `render_scale_pct`; R&C1
/// rac1/retail/pcdisp.rs, default 100).
pub fn wanted_pct() -> u32 {
	static ENV: OnceLock<Option<u32>> = OnceLock::new();
	ENV.get_or_init(|| parse_pct(&std::env::var("RAC_RENDER_SCALE").ok()?)).unwrap_or_else(|| clamp_pct((crate::pack::get().render_scale_pct)()))
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
	// [PC] Aspect ratio (aspect_pass.rs): the sizes are planned from the box inside the window, as `rscale_plan` does.
	let boxed = crate::viewer::aspect_box::fit(win, crate::viewer::aspect_box::current()).size();
	(config.width, config.height) = planned_size(boxed, device.limits().max_texture_dimension_2d);
	win
}

/// The render size for a window size: the upscaler's (upscale/slot.rs `active`: RAC_UPSCALE or the Upscaler row) while one runs,
/// else the render scale's.
fn planned_size(win: (u32, u32), max: u32) -> (u32, u32) {
	plan(win, crate::viewer::upscale::slot::active(), wanted_pct(), max)
}

/// `planned_size` for an upscaler setting `ups` and a render scale `pct`.
fn plan(win: (u32, u32), ups: Option<Upscale>, pct: u32, max: u32) -> (u32, u32) {
	match ups {
		Some(s) => s.render_size(win, max),
		None => render_size(win, pct, max),
	}
}

type Upscale = crate::viewer::upscale::slot::Setting;

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
	/// The upscaler setting the sizes were planned for (`rscale_sync` plans again when `slot::active` differs: a menu change).
	ups: Option<Upscale>,
	/// [PC] `ups` minus a pipe-guard refusal: the upscaler the sizes were planned WITH, and the one the frame runs (upscale.rs
	/// `upscale_begin`, `hud_height`). The game thread changes the option at any time; a fresh `slot::active` read later in the
	/// frame drew the HUD pass at the old render size into the new window-size picture (player crash 2026-11-03, FSR 1 -> Off).
	planned: Option<Upscale>,
	/// [PC] The render size `rscale_plan` returned (`rscale_sync` plans again when `config` holds another one).
	render: (u32, u32),
	target: Option<Target>,
	pipe: Option<Pipe>,
	/// [PC] Aspect ratio: the window size, the box and its picture (aspect_pass.rs); `win` above is the box.
	pub(super) abox: super::aspect_pass::ABox,
}

impl RScale {
	pub(in crate::viewer) fn scaled(&self) -> bool {
		self.target.is_some()
	}

	/// The upscaler the sizes were planned with (None: no upscaler, or the driver refused it).
	pub(in crate::viewer) fn planned(&self) -> Option<Upscale> {
		self.planned
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
	/// The window's size in pixels (what mouse positions and the menus use); `config` is the render size. [PC] With an Aspect ratio
	/// box it is the box (aspect_pass.rs): the window itself is `real_size`.
	pub fn win_size(&self) -> (u32, u32) {
		let w = self.rscale.win;
		if w.0 == 0 { (self.config.width, self.config.height) } else { w }
	}

	/// The offscreen view the frame is drawn into (None at 100 %: the window image itself).
	pub(super) fn rscale_view(&self) -> Option<wgpu::TextureView> {
		self.rscale.target.as_ref().map(|t| t.view.clone())
	}

	/// Window size `real` and the percentage in effect: the render size for them, with the offscreen picture made or dropped.
	pub(super) fn rscale_plan(&mut self, real: (u32, u32)) -> (u32, u32) {
		let win = self.abox_plan(real); // [PC] Aspect ratio (aspect_pass.rs): the box inside the window is the window from here on
		let pct = wanted_pct();
		// [PC] The upscaler (upscale/slot.rs): its pipelines are built (pipe-guarded) before the sizes are planned, so a refusal
		// plans the plain sizes here; `ups` keeps the setting asked for, so a refusal also differs at the next `rscale_sync` (which
		// plans again: `Gpu::new` does not use this size).
		let ups = crate::viewer::upscale::slot::active();
		self.upscale_build(ups.is_some());
		// ONE read of the option for the whole plan (`ups`, minus a refusal of the build just now): the sizes, the offscreen copy and
		// the frame's slot all follow it, whatever the menu sets meanwhile (the next `rscale_sync` plans again).
		let planned = crate::viewer::upscale::slot::active_of(ups, crate::viewer::pipe_guard::off(crate::viewer::pipe_guard::Effect::Upscale));
		let render = plan(win, planned, pct, self.device.limits().max_texture_dimension_2d);
		self.rscale.win = win;
		self.rscale.pct = pct;
		self.rscale.ups = ups;
		self.rscale.planned = planned;
		self.rscale.render = render;
		// [PC] The upscaler makes the window-size picture, no offscreen copy.
		if render == win || planned.is_some() {
			self.rscale.target = None;
		} else {
			let format = self.config.format;
			// [PC] pipe_guard.rs: a driver that refuses the blit shader (DX12 FXC) draws at the window size, no crash.
			if self.rscale.pipe.is_none() {
				self.rscale.pipe = crate::viewer::pipe_guard::guard(&self.device, crate::viewer::pipe_guard::Effect::RenderScale, || make_pipe(&self.device, format));
			}
			let Some(pipe) = &self.rscale.pipe else {
				self.rscale.target = None;
				self.rscale.render = win;
				return win;
			};
			self.rscale.target = Some(make_target(&self.device, pipe, format, render));
		}
		render
	}

	/// First thing in a frame: a changed scale or upscaler (menu or env) re-plans the sizes at once, and so does a `config` that is
	/// not the planned render size (`Gpu::new` sizes its targets from `start_size`'s own reads).
	pub fn rscale_sync(&mut self) {
		let stale = (self.config.width, self.config.height) != self.rscale.render;
		if self.rscale.pct != 0 && (stale || self.rscale.pct != wanted_pct() || self.rscale.ups != crate::viewer::upscale::slot::active() || self.abox_changed()) {
			let win = self.real_size();
			self.resize(win.0, win.1);
		}
	}

	/// The swapchain at the window size (the render size lives in `config`).
	pub(super) fn reconfigure_surface(&self) {
		super::setup::configure_surface(&self.surface, &self.device, &window_config(&self.config, self.real_size()));
	}

	/// Copies the offscreen picture onto `dst`, the window image or, with an Aspect ratio box, the box picture (nothing at 100 %).
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

	/// pipe_guard.rs `Effect::Upscale`: a refused upscaler plans the render scale's size (the plain picture), an accepted one its
	/// own render size; the menu's FSR Performance halves each side.
	#[test]
	fn a_refused_upscaler_plans_the_render_scale_size() {
		use crate::viewer::upscale::slot::active_of;
		let fsr = crate::viewer::upscale::slot::parse(Some("fsr:performance"));
		assert_eq!(plan((1920, 1080), active_of(fsr, false), 100, 8192), (960, 540));
		assert_eq!(plan((1920, 1080), active_of(fsr, true), 100, 8192), (1920, 1080), "refused: the window size");
		assert_eq!(plan((1920, 1080), active_of(fsr, true), 75, 8192), (1440, 810), "refused: the render scale applies");
		assert_eq!(plan((1920, 1080), None, 100, 8192), (1920, 1080));
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
		let _g = crate::pcopts::globals_guard();
		if std::env::var_os("RAC_RENDER_SCALE").is_none() {
			assert_eq!(wanted_pct(), clamp_pct((crate::pack::get().render_scale_pct)()));
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
