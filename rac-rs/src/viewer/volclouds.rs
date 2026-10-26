//! [PC] Volumetric clouds (Pause -> Options -> Graphics -> Effects -> Volumetric clouds Low / Medium / High, with Lighting Enhanced;
//! retail/pcext.rs `graphics`; no retail counterpart). Classic draws the retail sky alone and nothing here runs.
//!
//! On a level with an open sky (`params::look`) the painted sky gives way to a new one: `skymodel.rs` captures the retail sky and
//! reduces it to a smooth model that keeps the planet's palette (`sky.rs`), the backdrop drawn from it replaces the painted shells
//! (landmark shells stay: `params::sky_mask`), and real 3D clouds fill a spherical shell above the level (`layer.rs`: coherent cloud
//! fields from a weather map, the four cloud genera of `genera.rs` (cumulus to cumulonimbus towers and a stratocumulus deck in the
//! shell, an altocumulus band and the cirrus / cirrocumulus veil above it), lit by the sky's sun with a silver lining, drifting and
//! evolving; noise made on the GPU by `noise3d.rs`). The clouds are marched on a coarse grid (a quarter to an eighth of the
//! picture) before the level pass, accumulated over frames, and laid over the backdrop right after the sky shells (`apply`): every
//! world draw, effect, particle and smoke column of the level pass then lands on top of them, as on the painted sky. The fog pass
//! later puts its haze over the clouds too.

use super::*;

mod genera;
mod gputime;
mod input;
mod layer;
mod noise3d;
mod params;
mod sky;
mod skydump;
pub(in crate::viewer) mod skymodel;
#[cfg(test)]
mod tests;

pub(super) use input::Input;

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// Weight of the current frame in the history (lower than the fog's: the sub-pixel jitter is averaged over more frames, so cloud
/// edges stay still).
const BLEND: f32 = 0.08;
/// The sub-pixel offsets of the grid rays, one per frame (Halton 2, 3; grid pixels).
const JITTER: [[f32; 2]; 8] = [[0.0, -0.1667], [-0.25, 0.1667], [0.25, -0.3889], [-0.375, -0.0556], [0.125, 0.2778], [-0.125, -0.2778], [0.375, 0.0556], [-0.4375, 0.3889]];

/// The uniform block (layer.rs `C`, field for field; the meaning of each field is documented there).
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
	cam: [[f32; 4]; 4],
	prev: [[f32; 4]; 4],
	sun: [f32; 4],
	shell: [f32; 4],
	cover: [f32; 4],
	scale: [f32; 4],
	wind: [f32; 4],
	light: [f32; 4],
	haze: [f32; 4],
	size: [f32; 4],
	march: [f32; 4],
	lin: [f32; 4],
	tint: [f32; 4],
	dusk: [f32; 4],
	genus: [f32; 4],
	alt: [f32; 4],
	jit: [f32; 4],
	flow: [f32; 4],
	form: [f32; 4],
	reuse: [f32; 4],
}

struct Pipes {
	cloud: wgpu::RenderPipeline,
	resolve: wgpu::RenderPipeline,
	apply: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	lsamp: wgpu::Sampler,
	dummy: wgpu::TextureView,
}

struct Targets {
	key: (u32, u32, u64),
	cur: wgpu::TextureView,
	hist: [wgpu::TextureView; 2],
}

pub(super) struct VolClouds {
	format: wgpu::TextureFormat,
	depth_format: wgpu::TextureFormat,
	pipes: Option<Pipes>,
	noise: Option<noise3d::Noise>,
	ubuf: Option<wgpu::Buffer>,
	t: Option<Targets>,
	/// The previous frame's camera rows and clock, the frame count, which history texture is the newer.
	prev: Option<[[f32; 4]; 4]>,
	prev_time: f32,
	frames: u32,
	flip: usize,
	level: usize,
	/// The level's reference height the cloud shell sits on (the camera's z when the level began, raised when the camera climbs).
	ref_z: f32,
	/// The GPU timer (RAC_FOG_LOG=1), made at the first run.
	timer: Option<Option<gputime::GpuTimer>>,
	/// The last frame marched nothing (`gfxq::cloud_reuse`): the next one marches.
	skipped: bool,
	/// This frame's apply step (the bind group of the newest history), set by `march`, drawn by `apply` in the level pass.
	ready: Option<wgpu::BindGroup>,
	/// The new sky (captured model, backdrop, which shells to drop).
	pub(in crate::viewer) sky: skymodel::SkyModel,
}

/// How far the view may turn between two marches of the clouds when a frame in between reuses the last result (degrees), and how
/// far the camera may move (world units).
const CLOUD_REUSE_DEG: f32 = 1.0;
const CLOUD_REUSE_MOVE: f32 = 3.0;

/// Whether the view `now` is within `CLOUD_REUSE_DEG` (forward and right directions) and `CLOUD_REUSE_MOVE` of the view `prev`
/// (camera rows of `volfog::cam_rows`: position, forward, right * tan_x, up * tan_y).
fn view_close(prev: &[[f32; 4]; 4], now: &[[f32; 4]; 4]) -> bool {
	let unit_dot = |a: [f32; 4], b: [f32; 4]| {
		let (la, lb) = ((a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-9), (b[0] * b[0] + b[1] * b[1] + b[2] * b[2]).sqrt().max(1e-9));
		(a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) / (la * lb)
	};
	let min_cos = CLOUD_REUSE_DEG.to_radians().cos();
	let moved = ((0..3).map(|k| (prev[0][k] - now[0][k]).powi(2)).sum::<f32>()).sqrt();
	unit_dot(prev[1], now[1]) >= min_cos && unit_dot(prev[2], now[2]) >= min_cos && moved <= CLOUD_REUSE_MOVE
}

impl VolClouds {
	pub(super) fn new(format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> VolClouds {
		VolClouds { format, depth_format, pipes: None, noise: None, ubuf: None, t: None, prev: None, prev_time: 0.0, frames: 0, flip: 0, level: usize::MAX, ref_z: 0.0, timer: None, ready: None, skipped: false, sky: Default::default() }
	}

	/// The whole WGSL of the pass.
	fn source() -> String {
		format!("{}{}{}{}", layer::BLOCK, sky::LOOKUP.replace("SKYTEX", "skymap").replace("SKYSAMP", "lsamp"), genera::GENERA, layer::CLOUDS)
	}

	fn build(&self, device: &wgpu::Device) -> Pipes {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("volclouds"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("volclouds", Self::source().into())) });
		let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None };
		let tex = |filterable: bool, view_dimension| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension, multisampled: false };
		let (d2, d3) = (wgpu::TextureViewDimension::D2, wgpu::TextureViewDimension::D3);
		let filt = wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering);
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("volclouds"),
			entries: &[
				entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
				entry(2, tex(true, d3)),
				entry(3, tex(true, d3)),
				entry(4, tex(true, d2)),
				entry(5, filt),
				entry(6, tex(true, d2)),
				entry(7, tex(true, d2)),
				entry(8, filt),
				entry(9, tex(true, d2)),
			],
		});
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("volclouds"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let make = |fs: &'static str, target: wgpu::TextureFormat, blend: Option<wgpu::BlendState>, level: bool| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(fs),
				layout: Some(&pl),
				vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
				fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: target, blend, write_mask: wgpu::ColorWrites::ALL })] }),
				primitive: Default::default(),
				// The apply step draws inside the level pass (4x MSAA, its depth target untouched).
				depth_stencil: level.then(|| wgpu::DepthStencilState { format: self.depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
				multisample: wgpu::MultisampleState { count: if level { crate::viewer::targets::MSAA } else { 1 }, ..Default::default() },
				multiview_mask: None,
				cache: None,
			})
		};
		// The picture times the transmittance (the fragment's alpha) plus the scattered light.
		let over = wgpu::BlendState {
			color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::SrcAlpha, operation: wgpu::BlendOperation::Add },
			alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add },
		};
		let lsamp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("volclouds linear"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
		let dummy = device
			.create_texture(&wgpu::TextureDescriptor {
				label: Some("volclouds dummy"),
				size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: FORMAT,
				usage: wgpu::TextureUsages::TEXTURE_BINDING,
				view_formats: &[],
			})
			.create_view(&Default::default());
		Pipes { cloud: make("fs_cloud", FORMAT, None, false), resolve: make("fs_resolve", FORMAT, None, false), apply: make("fs_apply_clouds", self.format, Some(over), true), layout, lsamp, dummy }
	}

	fn targets(&self, device: &wgpu::Device, inp: &Input, div: u32) -> Targets {
		let (fw, fh) = (inp.size.0.div_ceil(div).max(1), inp.size.1.div_ceil(div).max(1));
		let make = |label| {
			device
				.create_texture(&wgpu::TextureDescriptor {
					label: Some(label),
					size: wgpu::Extent3d { width: fw, height: fh, depth_or_array_layers: 1 },
					mip_level_count: 1,
					sample_count: 1,
					dimension: wgpu::TextureDimension::D2,
					format: FORMAT,
					usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
					view_formats: &[],
				})
				.create_view(&Default::default())
		};
		Targets { key: (fw, fh, inp.generation), cur: make("volclouds now"), hist: [make("volclouds history 0"), make("volclouds history 1")] }
	}

	/// The uniform of this frame; `fresh` = no usable history (the current frame counts fully). RAC_TUNE cl_* knobs (ENV.md).
	fn uniform(&self, inp: &Input, q: params::Quality, look: params::Look, sun: [f32; 3], fresh: bool, fsize: (u32, u32)) -> Uniform {
		let t = |name, d| crate::devtune::f32(name, d);
		let cam = super::volfog::cam_rows(inp.cam, inp.aspect);
		let mut u: Uniform = Zeroable::zeroed();
		u.cam = cam;
		u.prev = self.prev.unwrap_or(cam);
		u.sun = [sun[0], sun[1], sun[2], t("cl_sun", look.sun)];
		let radius = t("cl_radius", 120000.0);
		u.shell = [radius, radius + t("cl_base", look.base), radius + t("cl_top", look.top), self.ref_z - radius];
		let c = |v: f32, name| (v + t(name, 0.0)).clamp(0.0, 1.0);
		u.cover = [c(look.cu, "cl_cu"), c(look.sc, "cl_sc"), t("cl_sigma", 0.08) * look.density, t("cl_eros", 0.7)];
		u.scale = [t("cl_weather", 14000.0 * look.size), t("cl_shape", 3000.0 * look.size), t("cl_detail", 250.0), t("cl_evolve", 0.0025)];
		let (speed, heading) = (t("cl_wind", look.wind), t("cl_heading", look.heading));
		u.wind = [heading.cos() * speed * inp.time, heading.sin() * speed * inp.time, inp.time, t("cl_shear", 500.0)];
		// The wind's drift since the last frame (the reprojection follows the moving clouds).
		let dt = if fresh { 0.0 } else { inp.time - self.prev_time };
		let j = JITTER[(self.frames % 8) as usize];
		u.jit = [j[0], j[1], heading.cos() * speed * dt, heading.sin() * speed * dt];
		u.genus = [c(look.ac, "cl_ac"), c(look.ci, "cl_ci"), c(look.cc, "cl_cc"), c(look.tower, "cl_tower")];
		u.alt = [radius + t("cl_mid", look.mid), t("cl_midthick", 420.0), radius + t("cl_high", look.high), t("cl_pale", look.pale)];
		let thick = (u.shell[2] - u.shell[1]).max(1.0);
		u.flow = [heading.cos(), heading.sin(), (t("cl_scthick", 750.0) / thick).min(1.0), t("cl_roll", 1500.0 * look.size)];
		u.light = [t("cl_g", 0.55), t("cl_gback", -0.25), t("cl_back", 0.3), t("cl_powder", 0.6)];
		u.haze = [t("cl_haze", look.haze), t("cl_reach", 90000.0), t("cl_amb", look.amb), t("cl_expo", look.expo)];
		u.size = [fsize.0 as f32, fsize.1 as f32, 1.0 / fsize.0 as f32, 1.0 / fsize.1 as f32];
		// Frame index mod 8: the along-ray offset's period, in step with JITTER (also keeps f32 exact in long sessions).
		u.march = [q.steps, q.light, (self.frames % 8) as f32, if fresh { 1.0 } else { t("cl_blend", BLEND) }];
		u.lin = [q.mid, t("cl_lsig", 0.4), q.div as f32, inp.far];
		let key = look.tint.unwrap_or(inp.light);
		let w = key[0].max(key[1]).max(key[2]).max(0.05);
		u.tint = [key[0] / w, key[1] / w, key[2] / w, t("cl_hue", look.hue)];
		let dusk = t("cl_dusk", look.dusk);
		u.dusk = [dusk, t("cl_rim", dusk * look.rim), t("cl_ms", 0.25 + 0.45 * dusk + look.soft), t("cl_damb", look.dusk_amb)];
		u.form = [t("cl_form", look.form), t("cl_lining", look.lining), t("cl_under", look.under), t("cl_feather", look.feather)];
		u
	}

	/// Follows the level's reference height (the shell stays above a climbing camera, slowly so the history follows).
	fn follow(&mut self, inp: &Input, look: params::Look) {
		let z = inp.cam.pos[2];
		if self.level != inp.level || (self.prev.is_none() && self.frames == 0) {
			self.ref_z = z;
			self.level = inp.level;
		} else if z > self.ref_z + look.base * 0.4 {
			self.ref_z += (z - look.base * 0.4 - self.ref_z) * 0.02;
		}
	}

	/// Marches and accumulates this frame's clouds (before the level pass; `apply` then lays them over the backdrop). False (and the
	/// history dropped) when the level has none, the new sky is off or the noise is still being made.
	pub(super) fn march(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, inp: &Input) -> bool {
		self.ready = None;
		let model = self.sky.model().map(|(s, m)| (s, m.clone()));
		let (Some(mut q), Some(look), Some((sun, skymap)), true) = (params::quality(inp.quality), params::look(inp.level), model, inp.size.0 > 0) else {
			self.prev = None;
			return false;
		};
		if let (3.., Some(div)) = (inp.quality, super::gfxq::cloud_div()) {
			q.div = div; // RAC_CLOUD_DIV (gfxq.rs)
		}
		let _s = super::profile::span("volclouds");
		let cu = (look.cu + crate::devtune::f32("cl_cu", 0.0)).clamp(0.0, 1.0);
		let cov = [cu, (cu + crate::devtune::f32("cl_farcov", look.farcov)).min(1.0)];
		if !self.noise.get_or_insert_with(|| noise3d::Noise::new(device)).step(queue, enc, cov) {
			self.prev = None;
			return false;
		}
		if self.pipes.is_none() {
			self.pipes = Some(self.build(device));
		}
		let timer = self.timer.get_or_insert_with(|| gputime::GpuTimer::new(device, queue));
		if let Some(tm) = timer.as_mut() {
			tm.poll(device);
		}
		let key_ok = self.t.as_ref().is_some_and(|t| t.key.2 == inp.generation && t.key.0 == inp.size.0.div_ceil(q.div) && t.key.1 == inp.size.1.div_ceil(q.div));
		if !key_ok {
			self.t = Some(self.targets(device, inp, q.div));
			self.prev = None;
		}
		self.follow(inp, look);
		let fresh = self.prev.is_none();
		let (read, write) = (self.flip, 1 - self.flip);
		let ubuf = self.ubuf.get_or_insert_with(|| device.create_buffer(&wgpu::BufferDescriptor { label: Some("volclouds"), size: std::mem::size_of::<Uniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })).clone();
		let Some(fsize) = self.t.as_ref().map(|t| (t.key.0, t.key.1)) else { return false };
		// RAC_CLOUD_REUSE: every other frame marches nothing while the view has turned less than `CLOUD_REUSE_DEG` since the last march.
		let reuse = super::gfxq::cloud_reuse() && !fresh && !self.skipped && self.prev.as_ref().is_some_and(|p| view_close(p, &super::volfog::cam_rows(inp.cam, inp.aspect)));
		let mut u = self.uniform(inp, q, look, sun, fresh, fsize);
		u.reuse[0] = if reuse { 1.0 } else { 0.0 };
		queue.write_buffer(&ubuf, 0, bytemuck::bytes_of(&u));
		let (Some(pipes), Some(t), Some(noise)) = (&self.pipes, &self.t, &self.noise) else { return false };
		let group = |cur: &wgpu::TextureView, hist: &wgpu::TextureView| {
			let view = wgpu::BindingResource::TextureView;
			device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("volclouds"),
				layout: &pipes.layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 2, resource: view(&noise.shape) },
					wgpu::BindGroupEntry { binding: 3, resource: view(&noise.detail) },
					wgpu::BindGroupEntry { binding: 4, resource: view(&noise.weather) },
					wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&noise.sampler) },
					wgpu::BindGroupEntry { binding: 6, resource: view(cur) },
					wgpu::BindGroupEntry { binding: 7, resource: view(hist) },
					wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&pipes.lsamp) },
					wgpu::BindGroupEntry { binding: 9, resource: view(&skymap) },
				],
			})
		};
		self.skipped = reuse;
		if reuse {
			self.ready = Some(group(&t.hist[self.flip], &pipes.dummy));
			return true;
		}
		let tm = self.timer.as_ref().and_then(|t| t.as_ref());
		let pass = |enc: &mut wgpu::CommandEncoder, i: u32, label: &str, target: &wgpu::TextureView, pipeline: &wgpu::RenderPipeline, g: &wgpu::BindGroup| {
			let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some(label),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: tm.and_then(|tm| tm.writes(i)).or_else(|| crate::viewer::passtime::rw(&format!("cloud {label}"))),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			p.set_pipeline(pipeline);
			p.set_bind_group(0, g, &[]);
			p.draw(0..3, 0..1);
		};
		pass(enc, 0, "volclouds march", &t.cur, &pipes.cloud, &group(&pipes.dummy, &pipes.dummy));
		pass(enc, 1, "volclouds accumulate", &t.hist[write], &pipes.resolve, &group(&t.cur, &t.hist[read]));
		self.ready = Some(group(&t.hist[write], &pipes.dummy));
		if let Some(tm) = self.timer.as_mut().and_then(|t| t.as_mut()) {
			tm.finish(enc);
		}
		self.prev = Some(super::volfog::cam_rows(inp.cam, inp.aspect));
		self.prev_time = inp.time;
		self.flip = write;
		self.frames = self.frames.wrapping_add(1);
		true
	}

	/// No clouds this frame (Classic, under water, warp): nothing to apply, the history is dropped.
	pub(super) fn idle(&mut self) {
		if crate::retail::splitview::classic() {
			return; // the Classic pass of a split frame must not drop the Enhanced pass's history
		}
		self.ready = None;
		self.prev = None;
	}

	/// Lays this frame's clouds over the backdrop and the kept sky shells, inside the level pass right after the sky (group 0 is the
	/// clouds' own here: the caller sets the level uniforms back). False when there is nothing to apply.
	pub(super) fn apply(&self, pass: &mut wgpu::RenderPass<'_>) -> bool {
		let (Some(g), Some(pipes)) = (&self.ready, &self.pipes) else { return false };
		if !crate::retail::splitview::fx(true) {
			return false; // the Classic half has no clouds
		}
		pass.set_pipeline(&pipes.apply);
		pass.set_bind_group(0, g, &[]);
		pass.draw(0..3, 0..1);
		true
	}
}
