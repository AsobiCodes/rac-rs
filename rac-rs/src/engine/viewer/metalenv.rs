//! [PC] Graphics -> Effects -> Metal On: what the metal look (metalfx.wgsl) reflects. Nothing here runs with Metal Off, Lighting
//! Classic or a game pack without metal textures (`u.keep_tail.y` = 0). The resources below are tiny; Metal Off allocates only
//! 1 x 1 stand-ins (`Dummy`) for the level-shader bind groups (group 0, bindings 4..8). The real set (`Real`) is made on the first
//! frame with Metal On (`MetalEnv::alloc`, prepare.rs, which then rebinds the groups that hold the stand-ins) and kept; the
//! reflection source (`Ssr`) is remade when the render size changes (a window resize, the render scale), with the same rebinds.
//!
//! - Environment: the level's retail sky shells drawn into six 64 x 64 cube faces (the faces and matrices of
//!   volclouds/skymodel.rs, with or without Volumetric clouds), on a new level and then every `EVERY` frames, plus a 5-level mip
//!   chain (rough metal reads blurrier levels). Six tiny sky passes and 24 tiny mip passes per capture.
//! - Screen-space reflection source: after the post chain's scene and linear depth are final (passes.rs, before the grade),
//!   one pass at half the render size (`ssr_size`; the Steam Deck's 640 x 400) keeps this frame's colour and depth with its
//!   view-projection, five tiny passes build the colour's mip chain (`SSR_MIPS`: glossy reflections read blurrier levels) and four
//!   the depth's nearest-depth chain (`HIZ`); the next frame's metal pixels walk their reflected ray through it (a previous-frame
//!   reflection: no extra target or pass inside the level pass).

use super::*;
use std::cell::{Cell, OnceCell, RefCell};

#[path = "metalplanar.rs"]
mod planar; // [PC] planar reflection of flat metal floors
pub(crate) use planar::{Floors, Keys};
#[path = "metalcontact.rs"]
mod contact; // [PC] contact reflections: the planar mirror fades with the reflection distance

/// Captured face size and mip levels (metalfx.wgsl `MK_MIPS` = the last level).
const FACE: u32 = 64;
const MIPS: u32 = 5;
/// Frames between two sky captures (as the cloud sky: some retail skies turn).
const EVERY: u32 = 30;
/// Mip levels of the reflection source's colour (metalfx.wgsl `metal_ssr` reads blurrier levels for hits further from the surface).
const SSR_MIPS: u32 = 6;
const SSR_DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
/// Levels of the reflection depth's nearest-depth chain (half the render size: the Deck's 640 x 400 down to 40 x 25; metalfx.wgsl `MK_HIZ_TOP` = the last): the march
/// skips whole cells the ray stays in front of, so it visits every texel it could meet without per-pixel samples (no grain).
pub(super) const HIZ: u32 = 5;

/// The reflection source's size for a render of w x h: half of it, rounded up to a multiple of 16 so every `HIZ` level halves
/// exactly (Steam Deck 1280 x 800 -> 640 x 400, 1920 x 1080 -> 960 x 544). Its texels are the reflection's stair steps, so a
/// fixed 640 x 400 showed them blown up 3x at 2000 x 1125. Read by uv: any window aspect works. At least 32 x 32 (all `SSR_MIPS`
/// colour levels fit). Remade when the render size changes (`ssr_rebuild`).
fn ssr_size(w: u32, h: u32) -> (u32, u32) {
	let r = |v: u32| (v / 2).div_ceil(16).max(2) * 16;
	(r(w), r(h))
}

/// The reflection source's new size when it must be (re)made: Metal on and none yet or of another size; Off never.
fn ssr_rebuild(have: Option<(u32, u32)>, on: bool, w: u32, h: u32) -> Option<(u32, u32)> {
	let want = ssr_size(w, h);
	(on && have != Some(want)).then_some(want)
}

/// Group 0 bindings 4..8 of the level shader: the sky faces, their sampler, the reflection colour and depth, the sun map's gather
/// sampler (metalfx.wgsl `skyvis_sun_gap`).
pub(super) const LAYOUT_ENTRIES: [wgpu::BindGroupLayoutEntry; 6] = [
	wgpu::BindGroupLayoutEntry { binding: 4, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None },
	wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
	wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
	wgpu::BindGroupLayoutEntry { binding: 7, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
	wgpu::BindGroupLayoutEntry { binding: 8, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering), count: None },
	// The planar floor reflection (metalplanar.rs; the level pass's group only, a 1 x 1 stand-in elsewhere).
	wgpu::BindGroupLayoutEntry { binding: 9, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
];

/// The mip and reflection-source passes.
pub(super) const BLIT: &str = r#"
struct VO { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@group(0) @binding(0) var src: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
@group(0) @binding(2) var lin: texture_2d<f32>;
@vertex fn vs_full(@builtin(vertex_index) k: u32) -> VO {
	let x = f32((k << 1u) & 2u);
	let y = f32(k & 2u);
	var o: VO;
	o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
	o.uv = vec2<f32>(x, y);
	return o;
}
// One mip level: the bilinear read at the texel centre of the level below is its 2 x 2 mean.
@fragment fn fs_mip(v: VO) -> @location(0) vec4<f32> {
	return textureSampleLevel(src, samp, v.uv, 0.0);
}
struct SsrOut { @location(0) c: vec4<f32>, @location(1) d: vec4<f32> };
// The reflection source: the scene's colour (filtered down) with alpha 1 where the camera saw the sky (the clear depth, at or past
// metalfx.wgsl MK_SKY_Z: the colour's mip chain averages it into the sky share metalfx.wgsl `skyvis_cone` reads) and the half-size
// linear view depth (nearest).
@fragment fn fs_ssr(v: VO) -> SsrOut {
	var o: SsrOut;
	let dim = vec2<f32>(textureDimensions(lin));
	let z = textureLoad(lin, vec2<i32>(clamp(v.uv * dim, vec2<f32>(0.0), dim - 1.0)), 0).r;
	o.c = vec4<f32>(textureSampleLevel(src, samp, v.uv, 0.0).rgb, select(0.0, 1.0, z >= 700.0));
	o.d = vec4<f32>(z, 0.0, 0.0, 0.0);
	return o;
}
// One level of the reflection depth's nearest-depth chain (`lin` = the level below): the nearest of its 2 x 2 block.
@fragment fn fs_hiz(v: VO) -> @location(0) vec4<f32> {
	let p = vec2<i32>(v.pos.xy) * 2;
	let a = min(textureLoad(lin, p, 0).r, textureLoad(lin, p + vec2<i32>(1, 0), 0).r);
	let b = min(textureLoad(lin, p + vec2<i32>(0, 1), 0).r, textureLoad(lin, p + vec2<i32>(1, 1), 0).r);
	return vec4<f32>(min(a, b), 0.0, 0.0, 0.0);
}
"#;

/// What group 0 bindings 4, 6 and 7 hold until Metal is first switched On (A9): 1 x 1 textures of the same formats and dimensions, so
/// Metal Off (and a game pack without metal textures) never allocates the real set.
struct Dummy {
	env_view: wgpu::TextureView,
	ssr_c: wgpu::TextureView,
	ssr_d: wgpu::TextureView,
}

impl Dummy {
	fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Dummy {
		let tex =|label, format| device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] });
		let env_view = tex("metal env off", format).create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
		Dummy { env_view, ssr_c: tex("metal ssr colour off", format).create_view(&Default::default()), ssr_d: tex("metal ssr depth off", SSR_DEPTH).create_view(&Default::default()) }
	}
}

/// The real set, made on the first frame with Metal On (`MetalEnv::alloc`).
struct Real {
	env: wgpu::Texture,
	env_view: wgpu::TextureView,
	msaa: wgpu::TextureView,
	depth: wgpu::TextureView,
	/// Resolve target of the sky passes (the bound `env` must not be a target while group 0 holds it), copied into mip 0.
	stage: wgpu::Texture,
	stage_faces: Vec<wgpu::TextureView>,
	ubufs: Vec<wgpu::Buffer>,
	/// The face passes' group 0 (one per `ubufs` entry), made on the first capture: everything it holds (the uniforms, the sun's
	/// cascades, this set's views) lives as long as this set.
	cap_groups: OnceCell<Vec<wgpu::BindGroup>>,
	mip: wgpu::RenderPipeline,
	mip_layout: wgpu::BindGroupLayout,
	/// Per level 1.. and face: (source view one level down, target view).
	mips: Vec<(wgpu::BindGroup, wgpu::TextureView)>,
	ssr: wgpu::RenderPipeline,
	ssr_layout: wgpu::BindGroupLayout,
	hiz: wgpu::RenderPipeline,
	hiz_layout: wgpu::BindGroupLayout,
	/// The reflection-source bind group and the post targets' generation it reads.
	ssr_group: RefCell<Option<(u64, wgpu::BindGroup)>>,
}

/// The reflection source at one size (`ssr_size`), remade when the render size changes (`MetalEnv::alloc`).
struct Ssr {
	/// All `SSR_MIPS` colour levels (bound to the level shader), level 0 alone (the source pass's target) and per level 1..: (level
	/// below, target level).
	c: wgpu::TextureView,
	c0: wgpu::TextureView,
	cmips: Vec<(wgpu::BindGroup, wgpu::TextureView)>,
	/// All `HIZ` levels (bound to the level shader) and level 0 alone (the source pass's target).
	d: wgpu::TextureView,
	d0: wgpu::TextureView,
	/// Per depth level 1..: (level below, target level).
	hiz_steps: Vec<(wgpu::BindGroup, wgpu::TextureView)>,
}

pub(in crate::viewer) struct MetalEnv {
	device: wgpu::Device,
	format: wgpu::TextureFormat,
	depth_format: wgpu::TextureFormat,
	samp: wgpu::Sampler,
	/// Binding 8: the sun map's gather (nearest; a depth texture takes no filtering sampler).
	nsamp: wgpu::Sampler,
	dummy: Dummy,
	/// Empty until the first frame with Metal On (`alloc`): Metal Off allocates nothing but `dummy`.
	real: OnceCell<Real>,
	/// The reflection source and its size, made with `real` and remade on a new render size.
	ssr: Option<((u32, u32), Ssr)>,
	/// (level, frames since) of the last sky capture.
	captured: Cell<Option<(usize, u32)>>,
	/// (level, view-projection) of the reflection source kept last frame.
	prev: Cell<Option<(usize, [[f32; 4]; 4])>>,
	/// The planar floor reflection's targets (made with `ssr`, same size), this frame's floor height and its mirror pass's uniforms.
	planar: Option<planar::Planar>,
	plane: Cell<Option<f32>>,
	/// The planar weight's fade-in (metalplanar.rs `fade`): last frame's weight and time.
	planar_fade: Cell<(f32, Option<std::time::Instant>)>,
	planar_mirror: RefCell<Option<super::mirror::Mirror>>,
}

/// Builds `make()` into `cell` on the first call with `on` set; true = just built (the caller rebinds). Off never builds.
fn build_on_first_on<T>(cell: &OnceCell<T>, on: bool, make: impl FnOnce() -> T) -> bool {
	if !on || cell.get().is_some() {
		return false;
	}
	cell.set(make()).is_ok()
}

impl MetalEnv {
	/// Only the 1 x 1 stand-ins and the sampler; the real set comes with `alloc`.
	pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> MetalEnv {
		let samp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("metal env"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, mipmap_filter: wgpu::MipmapFilterMode::Linear, ..Default::default() });
		let nsamp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("metal gather"), ..Default::default() });
		MetalEnv { device: device.clone(), format, depth_format, samp, nsamp, dummy: Dummy::new(device, format), real: OnceCell::new(), ssr: None, captured: Cell::new(None), prev: Cell::new(None), planar: None, plane: Cell::new(None), planar_fade: Cell::new((0.0, None)), planar_mirror: RefCell::new(None) }
	}

	/// The first frame with Metal `on` makes the real set, and a frame with a new render size `w` x `h` (the post chain's) a new
	/// reflection source (`ssr_rebuild`; the kept one is forgotten). True = made: every group 0 that holds `entries()` must be rebound.
	pub(super) fn alloc(&mut self, on: bool, w: u32, h: u32) -> bool {
		let first = build_on_first_on(&self.real, on, || Real::new(&self.device, self.format, self.depth_format, &self.samp));
		// The planar pass's group holds `entries()`: remade on its next pass.
		if first {
			self.planar_mirror.get_mut().take();
		}
		let Some(size) = ssr_rebuild(self.ssr.as_ref().map(|s| s.0), on, w, h) else { return first };
		let Some(real) = self.real.get_mut() else { return first };
		// The face passes' groups hold `entries()`: remade on the next capture.
		real.cap_groups.take();
		self.ssr = Some((size, Ssr::new(&self.device, real, self.format, &self.samp, size)));
		self.planar = Some(planar::Planar::new(&self.device, &real.mip_layout, &self.samp, (self.format, self.depth_format), size));
		self.planar_mirror.get_mut().take();
		self.prev.set(None);
		true
	}

	/// Whether the real set exists (never before the first frame with Metal On).
	#[cfg(test)]
	fn allocated(&self) -> bool {
		self.real.get().is_some()
	}
}

impl Real {
	fn new(device: &wgpu::Device, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat, samp: &wgpu::Sampler) -> Real {
		let tex = |label, w: u32, h: u32, layers: u32, mips: u32, samples: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| make_tex(device, label, (w, h, layers), mips, samples, format, usage);
		let (ra, tb) = (wgpu::TextureUsages::RENDER_ATTACHMENT, wgpu::TextureUsages::TEXTURE_BINDING);
		let env = tex("metal env", FACE, FACE, 6, MIPS, 1, format, ra | tb | wgpu::TextureUsages::COPY_DST);
		let env_view = env.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
		let msaa = tex("metal env msaa", FACE, FACE, 1, 1, crate::viewer::targets::MSAA, format, ra).create_view(&Default::default());
		let depth = tex("metal env depth", FACE, FACE, 1, 1, crate::viewer::targets::MSAA, depth_format, ra).create_view(&Default::default());
		let stage = tex("metal env stage", FACE, FACE, 6, 1, 1, format, ra | wgpu::TextureUsages::COPY_SRC);
		let stage_faces = (0..6).map(|k| layer(&stage, k, 0)).collect();
		let ubufs = (0..6).map(|_| device.create_buffer(&wgpu::BufferDescriptor { label: Some("metal env uniforms"), size: std::mem::size_of::<Uniforms>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })).collect();
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("metal env"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("metal env", BLIT.into())) });
		let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None };
		let src = entry(0, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false });
		let sampler = entry(1, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering));
		let mip_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("metal mip"), entries: &[src, sampler] });
		let lin = entry(2, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false });
		let ssr_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("metal ssr"), entries: &[src, sampler, lin] });
		let pipe = |layout: &wgpu::BindGroupLayout, fs: &str, targets: &[Option<wgpu::ColorTargetState>]| {
			let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("metal env"), bind_group_layouts: &[Some(layout)], immediate_size: 0 });
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some("metal env"),
				layout: Some(&pl),
				vertex: wgpu::VertexState { module: &module, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
				fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some(fs), compilation_options: Default::default(), targets }),
				primitive: Default::default(),
				depth_stencil: None,
				multisample: Default::default(),
				multiview_mask: None,
				cache: None,
			})
		};
		let target = |format| Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL });
		let mip = pipe(&mip_layout, "fs_mip", &[target(format)]);
		let ssr = pipe(&ssr_layout, "fs_ssr", &[target(format), target(SSR_DEPTH)]);
		let mips = (1..MIPS).flat_map(|m| (0..6).map(move |k| (k, m))).map(|(k, m)| mip_step(device, &mip_layout, samp, &env, k, m)).collect();
		let hiz_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor { label: Some("metal hiz"), entries: &[lin] });
		let hiz = pipe(&hiz_layout, "fs_hiz", &[target(SSR_DEPTH)]);
		Real { env, env_view, msaa, depth, stage, stage_faces, ubufs, cap_groups: OnceCell::new(), mip, mip_layout, mips, ssr, ssr_layout, hiz, hiz_layout, ssr_group: RefCell::new(None) }
	}
}

impl Ssr {
	fn new(device: &wgpu::Device, re: &Real, format: wgpu::TextureFormat, samp: &wgpu::Sampler, (sw, sh): (u32, u32)) -> Ssr {
		let (ra, tb) = (wgpu::TextureUsages::RENDER_ATTACHMENT, wgpu::TextureUsages::TEXTURE_BINDING);
		let ct = make_tex(device, "metal ssr colour", (sw, sh, 1), SSR_MIPS, 1, format, ra | tb);
		let cmips = (1..SSR_MIPS).map(|m| mip_step(device, &re.mip_layout, samp, &ct, 0, m)).collect();
		let dt = make_tex(device, "metal ssr depth", (sw, sh, 1), HIZ, 1, SSR_DEPTH, ra | tb);
		let hiz_steps = (1..HIZ)
			.map(|m| {
				let from = layer(&dt, 0, m - 1);
				let group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("metal hiz"), layout: &re.hiz_layout, entries: &[wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&from) }] });
				(group, layer(&dt, 0, m))
			})
			.collect();
		Ssr { c: ct.create_view(&Default::default()), c0: layer(&ct, 0, 0), cmips, d: dt.create_view(&Default::default()), d0: layer(&dt, 0, 0), hiz_steps }
	}
}

fn make_tex(device: &wgpu::Device, label: &str, (w, h, layers): (u32, u32, u32), mips: u32, samples: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages) -> wgpu::Texture {
	device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: layers }, mip_level_count: mips, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
}

/// One layer `k` at mip `m` of `t` as a 2D view.
fn layer(t: &wgpu::Texture, k: u32, m: u32) -> wgpu::TextureView {
	t.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: k, array_layer_count: Some(1), base_mip_level: m, mip_level_count: Some(1), ..Default::default() })
}

/// The mip pass of layer `k`, level `m` of `t`: (its group reading level m - 1, its target view).
fn mip_step(device: &wgpu::Device, layout: &wgpu::BindGroupLayout, samp: &wgpu::Sampler, t: &wgpu::Texture, k: u32, m: u32) -> (wgpu::BindGroup, wgpu::TextureView) {
	let from = layer(t, k, m - 1);
	let group = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("metal mip"), layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&from) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(samp) }] });
	(group, layer(t, k, m))
}

impl MetalEnv {
	/// Group 0 bindings 4..9 (`LAYOUT_ENTRIES`) of every pass but the level pass: the planar floor reflection is a stand-in (the mirror
	/// passes draw into it).
	pub(super) fn entries(&self) -> [wgpu::BindGroupEntry<'_>; 6] {
		self.entries_for(false)
	}

	/// Group 0 bindings 4..9 (`LAYOUT_ENTRIES`): the real set once it exists, else the 1 x 1 stand-ins; `planar` = the level pass's
	/// group (binding 9 the planar floor reflection).
	pub(super) fn entries_for(&self, planar: bool) -> [wgpu::BindGroupEntry<'_>; 6] {
		let pl = self.planar.as_ref().filter(|_| planar).map_or(&self.dummy.ssr_c, |p| &p.all);
		let env = self.real.get().map_or(&self.dummy.env_view, |r| &r.env_view);
		let (c, d) = self.ssr.as_ref().map_or((&self.dummy.ssr_c, &self.dummy.ssr_d), |(_, s)| (&s.c, &s.d));
		[
			wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(env) },
			wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.samp) },
			wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(c) },
			wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(d) },
			wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&self.nsamp) },
			wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(pl) },
		]
	}

	/// The metal block of the level uniforms (after `keep_tail.y`, the on switch, is set): knobs, sources ready, last frame's
	/// view-projection. Off: left zero, and the kept reflection source is forgotten (A6, `block`).
	pub(super) fn write_uniforms(&self, u: &mut Uniforms, level: usize) {
		if let Some(m) = block(u.keep_tail[1] > 0.0, level, self.captured.get(), &self.prev, knob("metal_ssr", 1.0) > 0.0, off_forgets()) {
			u.metal = m;
		}
	}
}

/// The metal uniform block of one frame, None while Metal is off (the block stays zero). Off also forgets the kept reflection
/// source: it goes stale while no capture runs (Classic lighting has no post chain), and a stale one must not be marched
/// through when Metal comes back On (A6). `ssr` = the `metal_ssr` knob is on; `forget` = `off_forgets()`.
fn block(on: bool, level: usize, captured: Option<(usize, u32)>, prev: &Cell<Option<(usize, [[f32; 4]; 4])>>, ssr: bool, forget: bool) -> Option<[[f32; 4]; 14]> {
	if !on {
		if forget {
			prev.set(None);
		}
		return None;
	}
	let env = captured.is_some_and(|(l, _)| l == level);
	let kept = prev.get().filter(|(l, _)| *l == level && ssr);
	Some(knobs(env, kept.map(|p| p.1)))
}

/// Whether a pass with Metal off forgets the kept sky faces and reflection source: not in the Classic pass of a split frame
/// (splitview.rs), which runs with Metal off between two Enhanced passes; forgetting there left the Enhanced half without either
/// (test build 9: F10 switched the reflections off on the Enhanced half too).
fn off_forgets() -> bool {
	!crate::splitview::classic()
}

fn knob(name: &str, default: f32) -> f32 {
	crate::devtune::f32(name, default)
}

/// The uniform block: every look number is a RAC_TUNE knob (ENV.md `metal_*` rows); `prev` = the reflection source's camera.
pub(super) fn knobs(env: bool, prev: Option<[[f32; 4]; 4]>) -> [[f32; 4]; 14] {
	// m[12]: the planar floor reflection (`MetalEnv::plan_planar`), zero here.
	// m[13]: contact reflections (metalcontact.rs `knobs`): near, far, extra blur in mips per unit of reflection distance.
	let mut m = [[0.0; 4]; 14];
	// Softer, blurrier reflections (user 2026-11-03, Blarg window bands): strength = the Metal reflections slider (default 75 %),
	// rougher polished / brushed / painted kinds (rougher = wider sky-visibility lobe, so far window posts no longer print bands on
	// the floor); brushed / painted stay under metal_ssr_rough (0.5) so they keep the screen-space reflection.
	m[0] = [knob("metal_refl", 1.0) * crate::metal_option::reflect_pct() as f32 / 100.0, knob("metal_f0", 0.04), knob("metal_hue", 0.35), knob("metal_spec", 0.8)];
	m[1] = [knob("metal_rough_polished", 0.4), knob("metal_rough_mirror", 0.05), knob("metal_rough_brushed", 0.48), knob("metal_rough_painted", 0.45)];
	// The walk's cell cap: 16 (user 2026-11-04 "the metal shader drops fps hard"): the rays that meet something meet it within 8 cells
	// (16 / 12 / 8 vs 64 change < 0.01 % of the pixels by more than 8 / 255 at 1280 x 720; Kalebo, Novalis, Pokitaru), the rest crawl
	// to the cap and miss; level pass 2.57 -> 1.52 ms (Metal off 0.37).
	m[2] = [knob("metal_equip", 0.5), env as u8 as f32, prev.is_some() as u8 as f32, knob("metal_ssr_steps", 16.0)];
	m[3] = [knob("metal_ssr_dist", 14.0), knob("metal_ssr_thick", 0.4), knob("metal_spec_floor", 0.08), knob("metal_dyn_spec", 0.6)];
	if let Some(vp) = prev {
		m[4..8].copy_from_slice(&vp);
	}
	m[8] = [knob("metal_metalness_polished", 1.0), knob("metal_metalness_mirror", 1.0), knob("metal_metalness_brushed", 0.9), knob("metal_metalness_painted", 0.3)];
	m[9] = [knob("metal_sky", 1.0), knob("metal_ssr_edge", 0.08), knob("metal_ssr_rough", 0.5), knob("metal_ground", 0.8)];
	// Glossy, not a mirror (user 2026-11-03): strength down / grazing, own-colour tint, texture break-up; sky blur, sky occlusion,
	// glossy cone per roughness, least reflection blur (mips).
	m[10] = [knob("metal_refl_down", 0.45), knob("metal_refl_graze", 0.85), knob("metal_tint", 0.6), knob("metal_texbreak", 0.5)];
	m[11] = [knob("metal_gloss_sky", 2.0), knob("metal_occl", 1.0), knob("metal_gloss", 0.8), knob("metal_gloss_min", 2.0)];
	m[13] = contact::knobs();
	m
}

impl Gpu {
	/// Before the level pass with Metal On: the sky faces of a new level, then every `EVERY` frames.
	pub(in crate::viewer) fn metal_capture_env(&self, enc: &mut wgpu::CommandEncoder, scene: &Scene, u: &Uniforms) {
		let me = &self.metalenv;
		if u.keep_tail[1] <= 0.0 {
			if off_forgets() {
				me.captured.set(None);
			}
			return;
		}
		let Some(re) = me.real.get() else { return }; // (`alloc` ran in prepare.rs: this is only the frame before it)
		match me.captured.get() {
			Some((l, age)) if l == self.level && age + 1 < EVERY => return me.captured.set(Some((l, age + 1))),
			_ => {}
		}
		let Some(sky) = scene.sky.as_ref().filter(|s| !s.draws.is_empty()) else {
			// No sky shells: the shader reads the ground colours (env not ready).
			me.captured.set(None);
			return;
		};
		let groups = re.cap_groups.get_or_init(|| {
			let layout = self.pipeline.get_bind_group_layout(0);
			re.ubufs.iter().map(|b| self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("metal env capture"), layout: &layout, entries: &self.sun.entries(b, me) })).collect()
		});
		for (k, bind) in groups.iter().enumerate() {
			let mut mu = *u;
			mu.sky_view_proj = super::volclouds::skymodel::face_matrix(k);
			self.queue.write_buffer(&re.ubufs[k], 0, bytemuck::bytes_of(&mu));
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("metal env capture"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &re.msaa, resolve_target: Some(&re.stage_faces[k]), depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(scene.clear), store: wgpu::StoreOp::Discard } })],
				depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
					view: &re.depth,
					depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
					stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(crate::viewer::shadowvol::STENCIL_CLEAR), store: wgpu::StoreOp::Discard }),
				}),
				timestamp_writes: crate::viewer::passtime::rw("metal env"),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_bind_group(0, bind, &[]);
			pass.set_pipeline(&self.sky_pipeline);
			pass.set_vertex_buffer(0, sky.vbuf.slice(..));
			pass.set_index_buffer(sky.ibuf.slice(..), wgpu::IndexFormat::Uint32);
			for d in &sky.draws {
				pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
				pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
			}
		}
		enc.copy_texture_to_texture(re.stage.as_image_copy(), re.env.as_image_copy(), wgpu::Extent3d { width: FACE, height: FACE, depth_or_array_layers: 6 });
		for (group, target) in &re.mips {
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("metal env mip"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_pipeline(&re.mip);
			pass.set_bind_group(0, group, &[]);
			pass.draw(0..3, 0..1);
		}
		me.captured.set(Some((self.level, 0)));
	}

	/// After the post chain's scene and depth are final (before the grade): keeps them as the next frame's reflection source.
	pub(in crate::viewer) fn metal_capture_ssr(&self, enc: &mut wgpu::CommandEncoder, u: &Uniforms) {
		let me = &self.metalenv;
		let inputs = self.post.water_inputs().filter(|_| u.keep_tail[1] > 0.0 && knob("metal_ssr", 1.0) > 0.0);
		let Some((scene, lin, _, generation)) = inputs else {
			if off_forgets() {
				me.prev.set(None);
			}
			return;
		};
		let (Some(re), Some((_, ss))) = (me.real.get(), me.ssr.as_ref()) else {
			me.prev.set(None);
			return;
		};
		let mut slot = re.ssr_group.borrow_mut();
		if slot.as_ref().is_none_or(|(g, _)| *g != generation) {
			let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("metal ssr"),
				layout: &re.ssr_layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(scene) },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&me.samp) },
					wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(lin) },
				],
			});
			*slot = Some((generation, group));
		}
		let Some((_, group)) = slot.as_ref() else { return };
		let attach = |view| Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } });
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("metal ssr source"),
			color_attachments: &[attach(&ss.c0), attach(&ss.d0)],
			depth_stencil_attachment: None,
			timestamp_writes: crate::viewer::passtime::rw("metal ssr"),
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&re.ssr);
		pass.set_bind_group(0, group, &[]);
		pass.draw(0..3, 0..1);
		drop(pass);
		let steps = ss.cmips.iter().map(|s| (&re.mip, s, "metal ssr colour mip")).chain(ss.hiz_steps.iter().map(|s| (&re.hiz, s, "metal ssr depth chain")));
		for (pipe, (from, target), label) in steps {
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some(label),
				color_attachments: &[attach(target)],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_pipeline(pipe);
			pass.set_bind_group(0, from, &[]);
			pass.draw(0..3, 0..1);
		}
		me.prev.set(Some((self.level, u.view_proj)));
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	fn validate(src: &str) -> naga::Module {
		let m = naga::front::wgsl::parse_str(src).unwrap_or_else(|e| panic!("{}", e.emit_to_string(src)));
		naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all()).validate(&m).expect("WGSL validates");
		m
	}
	#[test]
	fn blit_shader_validates() {
		validate(BLIT);
	}
	/// DX12 parity: the level shader with the metal part must also go through naga's HLSL writer (wgpu's dx12 feature, on by default
	/// on Windows, builds naga's `hlsl-out`; no extra crate). It does not run FXC; the picture pass checks that on a DX12 run.
	#[cfg(windows)]
	#[test]
	fn level_shader_writes_hlsl() {
		let src = super::super::volfog::soft::level_source(super::super::SHADER, false);
		let out = crate::viewer::metalpipe::hlsl(&src, true);
		assert!(out.contains("metal_look") && out.contains("metal_ssr"), "the metal function is in the HLSL");
		// R24 M3: the Off set (MK_ON = false) writes too; naga only turns the override into a constant (`if (!false)` returns at
		// metal_look's first line), the driver's compiler drops the dead metal code.
		let off = crate::viewer::metalpipe::hlsl(&src, false);
		assert!(off.contains("metal_look"), "the Off set writes");
	}
	#[test]
	fn knobs_default_and_off_block() {
		let m = knobs(false, None);
		assert_eq!(m[2][1], 0.0, "no sky captured");
		assert_eq!(m[2][2], 0.0, "no reflection source");
		assert!(m[4..8].iter().all(|c| *c == [0.0; 4]));
		assert!(m[1][1] < m[1][0] && m[1][0] < m[1][2], "mirror < polished < brushed");
		let vp = [[1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0], [9.0, 10.0, 11.0, 12.0], [13.0, 14.0, 15.0, 16.0]];
		let m = knobs(true, Some(vp));
		assert_eq!((m[2][1], m[2][2]), (1.0, 1.0));
		assert_eq!(&m[4..8], &vp[..]);
	}
	const VP: [[f32; 4]; 4] = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
	/// A6: Metal Off forgets the kept reflection source, so coming back On (after Classic frames that ran no capture) does not march
	/// through a stale one; the next On frame has no source until the capture of that frame has run.
	#[test]
	fn off_forgets_the_kept_reflection_source() {
		let prev = Cell::new(Some((3usize, VP)));
		let m = block(true, 3, Some((3, 0)), &prev, true, true).expect("on");
		assert_eq!((m[2][1], m[2][2]), (1.0, 1.0), "sky ready, source of this level used");
		assert_eq!(block(false, 3, Some((3, 0)), &prev, true, true), None, "off: the block stays zero");
		assert_eq!(prev.get(), None, "off cleared the kept source");
		let m = block(true, 3, Some((3, 0)), &prev, true, true).expect("on again");
		assert_eq!((m[2][1], m[2][2]), (1.0, 0.0), "on again: no stale source");
	}
	#[test]
	fn the_kept_source_is_only_used_on_its_own_level_and_with_the_knob() {
		let prev = Cell::new(Some((3usize, VP)));
		assert_eq!(block(true, 4, None, &prev, true, true).expect("on")[2][2], 0.0, "another level");
		assert_eq!(block(true, 3, None, &prev, false, true).expect("on")[2][2], 0.0, "metal_ssr = 0");
		assert_eq!(prev.get(), Some((3, VP)), "on does not touch the kept source");
		assert_eq!(block(true, 3, None, &prev, true, true).expect("on")[2][1], 0.0, "no sky captured");
	}
	/// Split view: the Classic pass of a split frame (Metal off) keeps the Enhanced pass's sky faces and reflection source, so the
	/// Enhanced half shows what Enhanced shows without the split; outside that pass Off still forgets them (A6).
	#[test]
	fn split_classic_pass_keeps_the_enhanced_sources() {
		crate::splitview::set_classic(true);
		let in_classic = off_forgets();
		crate::splitview::set_classic(false);
		assert!(!in_classic && off_forgets());
		let prev = Cell::new(Some((3usize, VP)));
		assert_eq!(block(false, 3, Some((3, 0)), &prev, true, in_classic), None, "Classic pass: the block stays zero");
		assert_eq!(prev.get(), Some((3, VP)), "and the kept source stays");
		let m = block(true, 3, Some((3, 0)), &prev, true, off_forgets()).expect("Enhanced pass");
		assert_eq!((m[2][1], m[2][2]), (1.0, 1.0), "the Enhanced half reads the sky faces and the reflection source");
	}
	/// A9: nothing of the real set is made until the first frame with Metal On.
	#[test]
	fn off_never_builds_the_real_set() {
		let cell: OnceCell<u32> = OnceCell::new();
		let mut built = 0;
		for _ in 0..1000 {
			assert!(!build_on_first_on(&cell, false, || { built += 1; 7 }));
		}
		assert_eq!((built, cell.get()), (0, None));
		assert!(build_on_first_on(&cell, true, || { built += 1; 7 }), "the first On frame builds");
		assert!(!build_on_first_on(&cell, true, || { built += 1; 8 }), "and only once");
		assert!(!build_on_first_on(&cell, false, || { built += 1; 9 }), "Off afterwards keeps it (the groups hold it)");
		assert_eq!((built, cell.get()), (1, Some(&7)));
	}
	/// The same on a real adapter (needs a GPU): `cargo test metal_env_on_adapter -- --ignored`.
	#[test]
	#[ignore]
	fn metal_env_on_adapter() {
		let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
		let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions::default())).expect("adapter");
		let (device, _queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default())).expect("device");
		let mut me = MetalEnv::new(&device, wgpu::TextureFormat::Rgba8UnormSrgb, wgpu::TextureFormat::Depth24PlusStencil8);
		assert!(!me.allocated() && !me.alloc(false, 1280, 800) && !me.allocated(), "Off allocates nothing");
		assert!(me.alloc(true, 1280, 800) && me.allocated(), "the first On frame allocates");
		assert!(!me.alloc(true, 1280, 800), "once per size");
		assert!(me.alloc(true, 1920, 1080), "a new render size remakes the reflection source");
	}
	/// v4 (orchestrator review of v3): the source follows the render size; the Deck keeps 640 x 400; every `HIZ` level halves exactly.
	#[test]
	fn ssr_source_is_half_the_render_size() {
		assert_eq!(ssr_size(1280, 800), (640, 400));
		assert_eq!(ssr_size(1920, 1080), (960, 544));
		assert_eq!(ssr_size(2000, 1125), (1008, 576));
		assert_eq!(ssr_size(1, 1), (32, 32), "every colour mip fits");
		assert!(32 >> (SSR_MIPS - 1) >= 1);
		for (w, h) in [(1280, 720), (1920, 1080), (2560, 1440), (3840, 2160), (2000, 1125)] {
			let (sw, sh) = ssr_size(w, h);
			assert!(sw % (1 << (HIZ - 1)) == 0 && sh % (1 << (HIZ - 1)) == 0 && sw * 2 >= w && sh * 2 >= h);
		}
	}
	/// R24 M1: a window resize or a new render scale remakes the reflection source at the new size (it kept the first size).
	#[test]
	fn the_reflection_source_follows_a_resize() {
		assert_eq!(ssr_rebuild(None, true, 1280, 800), Some((640, 400)), "the first On frame");
		assert_eq!(ssr_rebuild(Some((640, 400)), true, 1280, 800), None, "same size: kept");
		assert_eq!(ssr_rebuild(Some((640, 400)), true, 1920, 1080), Some((960, 544)), "resized: remade");
		assert_eq!(ssr_rebuild(None, false, 1280, 800), None, "Off never");
		assert_eq!(ssr_rebuild(Some((640, 400)), false, 1920, 1080), None, "Off keeps the old one");
	}
	#[test]
	fn layout_entries_follow_the_level_bindings() {
		let b: Vec<u32> = LAYOUT_ENTRIES.iter().map(|e| e.binding).collect();
		assert_eq!(b, [4, 5, 6, 7, 8, 9]);
		for name in ["@group(0) @binding(4) var metal_env", "@group(0) @binding(5) var metal_samp", "@group(0) @binding(6) var metal_ssr_c", "@group(0) @binding(7) var metal_ssr_d", "@group(0) @binding(8) var metal_nsamp", "@group(0) @binding(9) var metal_pl"] {
			assert!(super::super::metalfx::METAL_WGSL.contains(name), "{name}");
		}
	}
}
