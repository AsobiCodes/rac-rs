//! [PC] Heat haze (v0.5.1; Lighting Enhanced with Graphics -> Fire Medium / High; Fire Low / Off and Lighting Classic = no pass,
//! the picture unchanged): the air over the frame's fires and over lava wobbles. Sources are what the engine already knows:
//! `FireFx::cores` (firefx.rs: level fires, flame sprites, burning enemies, fireballs, the Pyrocitor's jet) and the lava oceans
//! (`waterfx::lava_ocean_box`: the pack's liquid table says which are lava; no lava for a game that names none). The lava boxes come
//! from the ocean draws themselves, so they need no Water Enhanced (review S4); lava drawn as level terrain has no box and no haze.
//! `plan` (pure) turns them into heat columns over each fire (radius and height from the fire's size) and hot layers over each
//! lava box, keeps what is on screen within `FADE_FAR` of the eye, the nearest first, and projects them to one screen rect.
//! `HeatHaze::run` copies that rect of the scene (plus a margin) and draws `heathaze.wgsl` scissored to it, in the post chain's `mid`
//! (pass_post.rs: after the water, smoke, fog and underwater passes, before bloom and the grade; the HUD is drawn later and never
//! moves). A pixel moves by the heat its view ray crosses BEFORE it reaches the surface, so anything in front of a source stays put.
//! The wobble runs on the fire's game clock (`FireFx::shown`, firefx_clock.rs): it stands still in a pause and under the Quick
//! Select wheel like the flames, and its frequencies are whole cycles per `firefx_clock::PERIOD`, so the wrap is seamless (review B1).
//! Upscalers (review S5): the wobble has no motion vectors. FSR 3 gets the hazed pixels in its reactive mask (`mask`, read by
//! upscale/ffx_reactive.wgsl: less history there, no smear); DLSS / DLAA have no such input and may soften the wobble a little.
//! No source near enough on screen = no copy, no pass (zero cost). The pipeline is built under `pipe_guard` (Effect::HeatHaze) at
//! the first frame it could draw (passes.rs `warm`); a refusal turns the haze off. The scene copy is freed after `IDLE_FREE` frames
//! without the pass or on another level. `RAC_HAZE=0` = off; RAC_TUNE `haze_amp` scales the offset.
//! Split view: `wanted` gates with `splitview::fx`, so the Classic pass has no haze.

use super::pipe_guard::{self, Effect};
use super::Camera;

/// The most fire columns (High; Medium takes the nearest `Quality::cols`) and lava boxes per frame.
pub(super) const MAX_COLS: usize = 16;
pub(super) const MAX_LAVA: usize = 4;
/// Distance from the eye (world units) where the haze starts to fade, and where it is gone.
pub(super) const FADE_NEAR: f32 = 12.0;
pub(super) const FADE_FAR: f32 = 45.0;
/// Near the eye: no haze closer than `EYE_NONE`, the full haze from `EYE_FULL` (world units).
const EYE_NONE: f32 = 1.5;
const EYE_FULL: f32 = 4.0;
/// The largest heat column (radius, height): the Pyrocitor's jet and big fireballs give large cores.
const COL_MAX_R: f32 = 1.4;
const COL_MAX_H: f32 = 4.0;
/// Over lava: the hot layer's height and the path through it that gives the full wobble.
const LAVA_H: f32 = 1.6;
const LAVA_PATH: f32 = 7.0;
/// Closest view depth of the box part that is projected (the box is clipped there).
const NEAR: f32 = 0.05;
const FORMAT_LIN: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// The FSR reactive input of the hazed pixels (render size; upscale/ffx_reactive.wgsl reads it).
pub(super) const FORMAT_MASK: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
/// Frames without the pass after which the scene copy is freed (N4).
const IDLE_FREE: u32 = 300;

/// The pass's level (Graphics -> Fire): the largest offset in pixels at 1080 lines and the most fire columns.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Quality {
	pub amp: f32,
	pub cols: usize,
}

/// Fire Medium = a lighter wobble over the 8 nearest fires, High = the full one over 16; Low / Off = none.
pub(super) const fn quality(fire_level: u8) -> Option<Quality> {
	match fire_level {
		2 => Some(Quality { amp: 1.4, cols: 8 }),
		3 => Some(Quality { amp: 2.0, cols: MAX_COLS }),
		_ => None,
	}
}

/// `RAC_HAZE=0`: never (dev A/B); anything else or unset: with the Fire row.
pub(super) fn allowed() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_HAZE").map_or(true, |v| v.trim() != "0"))
}

/// The pass may draw at Fire level `fire_level` (quality, `RAC_HAZE`, not refused by the driver): the sources are only gathered then.
pub(super) fn wanted(fire_level: u8) -> Option<Quality> {
	quality(fire_level).filter(|_| crate::splitview::fx(allowed() && !pipe_guard::off(Effect::HeatHaze)))
}

/// The wobble's clock: seconds into the fire clock's period at the fire's game time `shown` (`FireFx::shown`).
pub(super) fn clock(shown: f64) -> f32 {
	super::firefx_clock::Clock::game(shown).t
}

/// The camera as the pass sees it: eye, unit forward / right / up, the tangents of half the view and the target size.
#[derive(Clone, Copy, Debug)]
pub(super) struct View {
	pub eye: [f32; 3],
	pub f: [f32; 3],
	pub r: [f32; 3],
	pub u: [f32; 3],
	pub tx: f32,
	pub ty: f32,
	pub w: u32,
	pub h: u32,
}

impl View {
	pub(super) fn of(cam: &Camera, aspect: f32, w: u32, h: u32) -> View {
		let f = super::forward(cam);
		let r = super::normalise(super::cross(f, cam.up));
		let u = super::cross(r, f);
		View { eye: cam.pos, f, r, u, tx: cam.tan_half * aspect, ty: cam.tan_half, w, h }
	}
}

/// One frame's sources and the screen rect (x0, y0, x1, y1 in pixels, x1 / y1 exclusive) the pass draws in.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Plan {
	pub cols: Vec<[[f32; 4]; 2]>,
	pub lava: Vec<[[f32; 4]; 2]>,
	pub rect: [u32; 4],
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// Distance from `p` to the box `lo..hi` (0 inside).
fn box_dist(p: [f32; 3], lo: [f32; 3], hi: [f32; 3]) -> f32 {
	let d: [f32; 3] = std::array::from_fn(|i| (lo[i] - p[i]).max(p[i] - hi[i]).max(0.0));
	dot(d, d).sqrt()
}

/// The heat column over a fire core (centre, horizontal radius, half height; firefx_vol.rs `core`): foot at the core's centre,
/// wider than the flame, reaching well above it; bigger fires a little stronger. Rows as `heathaze.wgsl` reads them.
pub(super) fn column(c: [f32; 3], r: f32, hz: f32) -> [[f32; 4]; 2] {
	let (r, hz) = (r.max(0.05), hz.max(0.05));
	let rad = (1.3 * r + 0.15).min(COL_MAX_R);
	let height = (2.0 * hz + 1.5 * r + 0.5).min(COL_MAX_H);
	let gain = (r / 0.5).clamp(0.4, 1.0);
	[[c[0], c[1], c[2], rad], [height, gain, 0.0, 0.0]]
}

/// The column's box (min, max corner).
fn column_box(row: &[[f32; 4]; 2]) -> ([f32; 3], [f32; 3]) {
	let ([x, y, z, rad], h) = (row[0], row[1][0]);
	([x - rad, y - rad, z], [x + rad, y + rad, z + h])
}

/// The hot layer over a lava box (min x, y, z, max x, y, z), cut to the part within `FADE_FAR` of the eye (None: nothing near).
pub(super) fn lava_layer(b: [f32; 6], eye: [f32; 3]) -> Option<[[f32; 4]; 2]> {
	let top = b[5];
	let (x0, y0) = (b[0].max(eye[0] - FADE_FAR), b[1].max(eye[1] - FADE_FAR));
	let (x1, y1) = (b[3].min(eye[0] + FADE_FAR), b[4].min(eye[1] + FADE_FAR));
	if x1 <= x0 || y1 <= y0 || box_dist(eye, [x0, y0, top - 0.05], [x1, y1, top + LAVA_H]) > FADE_FAR {
		return None;
	}
	Some([[x0, y0, x1, y1], [top, LAVA_H, LAVA_PATH, 1.0]])
}

fn lava_box(row: &[[f32; 4]; 2]) -> ([f32; 3], [f32; 3]) {
	let ([x0, y0, x1, y1], [top, h, ..]) = (row[0], row[1]);
	([x0, y0, top - 0.05], [x1, y1, top + h])
}

/// The screen rect of a world box, clamped to the screen: None when it lies wholly outside one plane of the view frustum (behind
/// the near plane or past a side: also a box beside and behind the eye, review S3). A box reaching behind the near plane is clipped
/// there (its edges cut at `NEAR`), so the rect stays tight; one round the eye covers the screen.
pub(super) fn screen_rect(v: &View, lo: [f32; 3], hi: [f32; 3]) -> Option<[f32; 4]> {
	let c: [[f32; 3]; 8] = std::array::from_fn(|k| {
		let p = [if k & 1 == 0 { lo[0] } else { hi[0] }, if k & 2 == 0 { lo[1] } else { hi[1] }, if k & 4 == 0 { lo[2] } else { hi[2] }];
		let q = [p[0] - v.eye[0], p[1] - v.eye[1], p[2] - v.eye[2]];
		[dot(q, v.r), dot(q, v.u), dot(q, v.f)]
	});
	let all = |t: &dyn Fn(&[f32; 3]) -> bool| c.iter().all(t);
	if all(&|p| p[2] < NEAR) || all(&|p| p[0] > p[2] * v.tx) || all(&|p| -p[0] > p[2] * v.tx) || all(&|p| p[1] > p[2] * v.ty) || all(&|p| -p[1] > p[2] * v.ty) {
		return None;
	}
	let mut r = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
	let mut add = |p: [f32; 3]| {
		let (nx, ny) = ((p[0] / (p[2] * v.tx)).clamp(-1.0, 1.0), (p[1] / (p[2] * v.ty)).clamp(-1.0, 1.0));
		r = [r[0].min(nx), r[1].min(ny), r[2].max(nx), r[3].max(ny)];
	};
	for (a, pa) in c.iter().enumerate() {
		if pa[2] >= NEAR {
			add(*pa);
		}
		for bit in [1, 2, 4] {
			let pb = c[a | bit];
			if a & bit == 0 && (pa[2] < NEAR) != (pb[2] < NEAR) {
				let s = (NEAR - pa[2]) / (pb[2] - pa[2]);
				add(std::array::from_fn(|i| pa[i] + (pb[i] - pa[i]) * s));
			}
		}
	}
	if r[2] <= r[0] || r[3] <= r[1] {
		return None;
	}
	let (w, h) = (v.w as f32, v.h as f32);
	Some([(r[0] * 0.5 + 0.5) * w, (0.5 - r[3] * 0.5) * h, (r[2] * 0.5 + 0.5) * w, (0.5 - r[1] * 0.5) * h])
}

/// Pixels round the rect for the largest offset `amp_px` (window pixels) and the filter.
pub(super) fn margin(amp_px: f32) -> u32 {
	amp_px.ceil() as u32 + 3
}

/// The sources on screen within `FADE_FAR`, the nearest `keep` (distance, row, screen rect).
fn nearest_on_screen(v: &View, rows: impl Iterator<Item = ([[f32; 4]; 2], ([f32; 3], [f32; 3]))>, keep: usize) -> Vec<(f32, [[f32; 4]; 2], [f32; 4])> {
	let mut on: Vec<_> = rows
		.filter_map(|(row, (lo, hi))| {
			let d = box_dist(v.eye, lo, hi);
			(d < FADE_FAR).then(|| screen_rect(v, lo, hi)).flatten().map(|s| (d, row, s))
		})
		.collect();
	on.sort_by(|a, b| a.0.total_cmp(&b.0));
	on.truncate(keep);
	on
}

/// The frame's plan: the nearest `q.cols` fire columns and the nearest `MAX_LAVA` lava layers that are on screen within `FADE_FAR`
/// (screen test first, then the nearest: review S2), and their joined screen rect grown by `margin`. None = nothing to draw.
pub(super) fn plan(v: &View, cores: &[([f32; 3], f32, f32)], lava: &[[f32; 6]], q: Quality) -> Option<Plan> {
	let cols = nearest_on_screen(v, cores.iter().map(|&(c, r, hz)| column(c, r, hz)).map(|row| (row, column_box(&row))), q.cols.min(MAX_COLS));
	let lava = nearest_on_screen(v, lava.iter().filter_map(|&b| lava_layer(b, v.eye)).map(|row| (row, lava_box(&row))), MAX_LAVA);
	let rect = cols.iter().chain(lava.iter()).fold([f32::MAX, f32::MAX, f32::MIN, f32::MIN], |r, (_, _, s)| [r[0].min(s[0]), r[1].min(s[1]), r[2].max(s[2]), r[3].max(s[3])]);
	if rect[2] <= rect[0] || rect[3] <= rect[1] {
		return None;
	}
	let m = margin(q.amp * v.h as f32 / 1080.0) as f32;
	let clampx = |x: f32, hi: u32| (x.max(0.0) as u32).min(hi);
	let r = [clampx(rect[0] - m, v.w), clampx(rect[1] - m, v.h), clampx((rect[2] + m).ceil(), v.w), clampx((rect[3] + m).ceil(), v.h)];
	if r[2] <= r[0] || r[3] <= r[1] {
		return None;
	}
	Some(Plan { cols: cols.into_iter().map(|c| c.1).collect(), lava: lava.into_iter().map(|l| l.1).collect(), rect: r })
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct HazeU {
	size: [f32; 4],
	cam: [[f32; 4]; 4],
	prm: [f32; 4],
	fade: [f32; 4],
	cols: [[f32; 4]; 2 * MAX_COLS],
	lava: [[f32; 4]; 2 * MAX_LAVA],
}

fn uniform(v: &View, p: &Plan, secs: f32, amp_px: f32) -> HazeU {
	let mut u = HazeU {
		size: [v.w as f32, v.h as f32, 1.0 / v.w as f32, 1.0 / v.h as f32],
		cam: [
			[v.eye[0], v.eye[1], v.eye[2], 0.0],
			[v.f[0], v.f[1], v.f[2], 0.0],
			[v.r[0] * v.tx, v.r[1] * v.tx, v.r[2] * v.tx, 0.0],
			[v.u[0] * v.ty, v.u[1] * v.ty, v.u[2] * v.ty, 0.0],
		],
		prm: [secs, amp_px, p.cols.len() as f32, p.lava.len() as f32],
		fade: [FADE_NEAR, FADE_FAR, EYE_NONE, EYE_FULL],
		cols: [[0.0; 4]; 2 * MAX_COLS],
		lava: [[0.0; 4]; 2 * MAX_LAVA],
	};
	for (k, c) in p.cols.iter().take(MAX_COLS).enumerate() {
		u.cols[2 * k] = c[0];
		u.cols[2 * k + 1] = c[1];
	}
	for (k, l) in p.lava.iter().take(MAX_LAVA).enumerate() {
		u.lava[2 * k] = l[0];
		u.lava[2 * k + 1] = l[1];
	}
	u
}

pub(super) const SHADER: &str = include_str!("heathaze.wgsl");

/// One frame's sources (pass_post.rs): the Fire row level (0 under water), the fire cores, the lava boxes, the wobble's clock
/// (`clock`) and the level (another level frees the scene copy).
pub(super) struct Sources<'a> {
	pub fire_level: u8,
	pub cores: &'a [([f32; 3], f32, f32)],
	pub lava: &'a [[f32; 6]],
	pub t: f32,
	pub level: usize,
}

struct Pipes {
	format: wgpu::TextureFormat,
	pipe: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	ubuf: wgpu::Buffer,
	samp: wgpu::Sampler,
}

/// The scene copy (size of the scene), the FSR mask and the bind group of the copy with the post targets of `generation`.
struct SceneCopy {
	tex: wgpu::Texture,
	view: wgpu::TextureView,
	mask: wgpu::TextureView,
	group: Option<(u64, wgpu::BindGroup)>,
}

/// The pass's GPU side: pipeline (`warm`), the scene copy (made per target size, freed when idle).
#[derive(Default)]
pub(crate) struct HeatHaze {
	pipes: Option<Pipes>,
	copy: Option<SceneCopy>,
	idle: u32,
	level: usize,
	/// The FSR mask holds this frame's haze (`mask`).
	drawn: bool,
}

impl HeatHaze {
	fn pipelines(device: &wgpu::Device, format: wgpu::TextureFormat) -> Pipes {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("heat haze"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("heat haze", SHADER.into())) });
		let tex = |binding, filterable| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false },
			count: None,
		};
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("heat haze"),
			entries: &[
				wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
				tex(1, true),
				tex(2, false),
				wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
			],
		});
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("heat haze"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let targets = [Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL }), Some(wgpu::ColorTargetState { format: FORMAT_MASK, blend: None, write_mask: wgpu::ColorWrites::ALL })];
		let pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("heat haze"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_haze"), compilation_options: Default::default(), targets: &targets }),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("heat haze"), size: std::mem::size_of::<HazeU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let samp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("heat haze"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
		Pipes { format, pipe, layout, ubuf, samp }
	}

	/// Builds the pipeline for scene format `format` at the first frame the haze could draw (passes.rs; Fire Medium / High), not at
	/// the first fire mid-game. A driver that refuses it turns the haze off for the run (pipe_guard.rs).
	pub(super) fn warm(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat) {
		if self.pipes.is_some() || wanted(crate::fire_option::level()).is_none() {
			return;
		}
		self.pipes = pipe_guard::guard(device, Effect::HeatHaze, || Self::pipelines(device, format));
	}

	/// The scene copy and the FSR mask for a `w` x `h` scene (remade when the size changes; the bind group with them).
	fn copy_for(&mut self, device: &wgpu::Device, format: wgpu::TextureFormat, w: u32, h: u32) -> &mut SceneCopy {
		if self.copy.as_ref().is_some_and(|c| c.tex.width() != w || c.tex.height() != h) {
			self.copy = None;
		}
		self.copy.get_or_insert_with(|| {
			let make = |label, format, usage| {
				let size = wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 };
				device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
			};
			let tex = make("heat haze scene copy", format, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST);
			let mask = make("heat haze mask", FORMAT_MASK, wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
			SceneCopy { view: tex.create_view(&Default::default()), tex, mask, group: None }
		})
	}

	/// A frame without the pass: after `IDLE_FREE` of them the scene copy is freed (it is made again at the next pass).
	fn idle(&mut self) -> bool {
		self.drawn = false;
		self.idle = self.idle.saturating_add(1);
		if self.idle >= IDLE_FREE {
			self.copy = None;
		}
		false
	}

	/// FSR 3's reactive input of this frame (render size): the hazed pixels, or None when the haze did not draw since the last call.
	pub(super) fn take_mask(&mut self) -> Option<wgpu::TextureView> {
		self.copy.as_ref().filter(|_| std::mem::take(&mut self.drawn)).map(|c| c.mask.clone())
	}

	/// One frame's haze into `scene` (inside post.rs `encode`'s `mid`; `lin` = its half-size linear depth, already drawn, with the
	/// post targets' generation). `cam` / `aspect`: the level pass's projection. Returns false when nothing was drawn.
	#[allow(clippy::too_many_arguments)]
	pub(super) fn run(&mut self, device: &wgpu::Device, queue: &super::GatedQueue, enc: &mut wgpu::CommandEncoder, scene: &wgpu::TextureView, (lin, generation): (&wgpu::TextureView, u64), cam: &Camera, aspect: f32, s: &Sources<'_>) -> bool {
		if s.level != self.level {
			(self.level, self.copy) = (s.level, None);
		}
		let Some(q) = wanted(s.fire_level) else { return self.idle() };
		let st = scene.texture();
		let (w, h) = (st.width(), st.height());
		if lin.texture().format() != FORMAT_LIN {
			return self.idle();
		}
		let v = View::of(cam, aspect, w, h);
		let Some(p) = plan(&v, s.cores, s.lava, q) else { return self.idle() };
		self.warm(device, st.format());
		let Some(pp) = self.pipes.as_ref().filter(|pp| pp.format == st.format()) else { return self.idle() };
		let amp_px = q.amp * crate::devtune::f32("haze_amp", 1.0) * h as f32 / 1080.0;
		queue.write_buffer(&pp.ubuf, 0, bytemuck::bytes_of(&uniform(&v, &p, s.t, amp_px)));
		let (pipe, layout, ubuf, samp) = (pp.pipe.clone(), pp.layout.clone(), pp.ubuf.clone(), pp.samp.clone());
		let c = self.copy_for(device, st.format(), w, h);
		// The copy reaches one margin past the drawn rect (the samples move at most `amp_px`).
		let m = margin(amp_px);
		let [x0, y0, x1, y1] = p.rect;
		let (cx0, cy0, cx1, cy1) = (x0.saturating_sub(m), y0.saturating_sub(m), (x1 + m).min(w), (y1 + m).min(h));
		let mut from = st.as_image_copy();
		from.origin = wgpu::Origin3d { x: cx0, y: cy0, z: 0 };
		let mut to = c.tex.as_image_copy();
		to.origin = from.origin;
		enc.copy_texture_to_texture(from, to, wgpu::Extent3d { width: cx1 - cx0, height: cy1 - cy0, depth_or_array_layers: 1 });
		// Cached while the copy and the post targets stay (N3).
		if c.group.as_ref().is_none_or(|g| g.0 != generation) {
			let entries = [
				wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&c.view) },
				wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(lin) },
				wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&samp) },
			];
			c.group = Some((generation, device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("heat haze"), layout: &layout, entries: &entries })));
		}
		let att = |view, load| Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } });
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("heat haze"),
			color_attachments: &[att(scene, wgpu::LoadOp::Load), att(&c.mask, wgpu::LoadOp::Clear(wgpu::Color::BLACK))],
			depth_stencil_attachment: None,
			timestamp_writes: super::passtime::rw("heat haze"),
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&pipe);
		pass.set_bind_group(0, &c.group.as_ref().expect("made above").1, &[]);
		pass.set_scissor_rect(x0, y0, x1 - x0, y1 - y0);
		pass.draw(0..3, 0..1);
		drop(pass);
		(self.idle, self.drawn) = (0, true);
		true
	}
}

#[cfg(test)]
mod tests;
