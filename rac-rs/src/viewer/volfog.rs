//! [PC] Volumetric fog (Split view: `Input::new` is None under the Classic pass's `gfx`, `frame.rs` keeps the retail fog there) (Pause -> Options -> Graphics -> Fog Low / Medium / High, with Lighting Enhanced; retail/pcext.rs
//! `graphics`; no retail counterpart). Classic draws the retail fog alone and nothing here runs.
//!
//! A ray march at half or quarter size (the post chain's linear depth grid): every pixel steps along its view ray to the scene (at
//! most `Params::cap` units) on a jittered logarithmic spacing, integrating the air's thickness (`density.rs`: height fog that
//! pools low, noise wisps drifting with the wind, Kerwan's chasm fog sheet as a real volume) into in-scattered light and
//! transmittance (`scatter.rs`: the sun's phase function, the sun's visibility from the cascaded shadow map so light shafts
//! stand where light passes between trees and buildings, an ambient term tinted by the level's retail fog colour). The result is
//! accumulated over frames (`temporal.rs`: reprojection, neighbourhood clamp) and applied to the picture with a depth-aware
//! upsample: scene * transmittance + scattered light, after the water pass, before bloom and the grade.
//!
//! Faithful base: the amount comes from the level's own retail fog (`density::level_params`); a level without retail fog gets none.
//! Shared with a later cloud layer: `noise.rs` (the 3D noise volume), `scatter.rs`, `temporal.rs`; the fog only adds `density.rs`.
//! Quality (what runs; `quality`): the grid is a divisor of the half-size linear depth grid. High = 32 steps, divisor 2 (a quarter of
//! the picture, the same grid as Medium; RAC_FOG_DIV=1 gives the half-size grid, gfxq.rs); Medium = 20 steps, divisor 2; Low = 12
//! steps, divisor 2 and no sun shadow map (mist without shafts). Low and Medium draw 0.6 and 0.8 of the amount.

use super::*;

mod density;
pub(in crate::viewer) mod floor;
pub(in crate::viewer) mod frame;
mod gputime;
pub(in crate::viewer) mod noise;
pub(in crate::viewer) mod scatter;
pub(in crate::viewer) mod soft;
pub(in crate::viewer) mod temporal;

pub(super) use frame::{fade_retail_fog, retail_fog_scale, wants_ocean_heights, Plan};

/// The device features the fog asks for: timestamps (RAC_FOG_LOG) and the forced early depth test (`soft::level_source`).
pub(super) fn gpu_features(adapter: &wgpu::Adapter) -> wgpu::Features {
	gputime::features(adapter) | soft::features(adapter)
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// The jitter counter of the march (`scatter.rs`: `fract(ign + 0.618034 * march.z)`) wraps here, like the clouds' (volclouds.rs, % 8): an f32
/// counter that grows without end loses the fractional part of the product after a few hours. 1000 x 0.618034 = 618.034, so the
/// wrap moves the offset by 0.034 only, and the product stays below 618 (an f32 step of 6e-5).
const JITTER_PERIOD: u32 = 1000;

/// What a Fog level costs: march steps, the grid's divisor of the linear depth grid, and whether the sun's shadow map is read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Quality {
	pub steps: f32,
	pub div: u32,
	pub shadows: bool,
	/// Share of the fog amount this level draws (Low and Medium are lighter than High).
	pub amount: f32,
}

/// The settings of a Graphics -> Fog level (`Graphics::fog_q`: 0 Classic, 1 Low, 2 Medium, anything else High); None = off.
pub(super) const fn quality(level: u8) -> Option<Quality> {
	match level {
		0 => None,
		1 => Some(Quality { steps: 12.0, div: 2, shadows: false, amount: 0.6 }),
		2 => Some(Quality { steps: 20.0, div: 2, shadows: true, amount: 0.8 }),
		_ => Some(Quality { steps: 32.0, div: 2, shadows: true, amount: 1.0 }),
	}
}

/// The uniform block (`scatter::BLOCK`, struct F), field for field.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Uniform {
	cam: [[f32; 4]; 4],
	prev: [[f32; 4]; 4],
	sun_vp: [[[f32; 4]; 4]; 3],
	sun: [f32; 4],
	sun_bias: [f32; 4],
	sun_texel: [f32; 4],
	sun_q: [f32; 4],
	fog_col: [f32; 4],
	sun_col: [f32; 4],
	amb_col: [f32; 4],
	dens: [f32; 4],
	sheet: [f32; 4],
	wind: [f32; 4],
	look: [f32; 4],
	size: [f32; 4],
	march: [f32; 4],
	proj: [f32; 4],
	lin: [f32; 4],
	phase: [f32; 4],
	floor_c: [f32; 4],
}

/// The fog result is applied to the picture: `rgb` added, the picture times `a` (the transmittance), the nearest few fog pixels
/// weighted by how close their depth is to the pixel's own (no halo round near edges). Its second target is the cover (`1 - a`).
const APPLY: &str = r#"
struct Applied {
	@location(0) fog: vec4<f32>,
	@location(1) cover: vec4<f32>,
}
// The share of a surface `dist` away that retail's GS fog covered and the fade (frame.rs `fade_retail_fog`) took away, given back
// over the last 30% of the fog's ramp: 0 nearer, the whole faded share at its end (where retail stops drawing: the mesh edges).
fn far_fog(dist: f32) -> f32 {
	let gs = vec4<f32>(f.prev[0].w, f.prev[1].w, f.prev[2].w, f.prev[3].w);
	if (gs.w <= 0.0 || gs.w >= 1.0 || gs.z >= 254.5 || f.phase.w <= 0.0) { return 0.0; }
	let raw = clamp(gs.x + dist * gs.y, gs.z, 255.0);
	let full = raw / 255.0;
	let faded = 1.0 - gs.w * (1.0 - full);
	return (1.0 - full / faded) * smoothstep(0.7, 1.0, (255.0 - raw) / (255.0 - gs.z));
}

// How close (1 next to it .. 0 from 96 pixels) a surface pixel at `pos` lies under a void pixel that looks down past the edge of the
// level's mesh (`floor_past_edge`): a few taps straight up the depth target, the nearest void one decides.
fn under_edge(pos: vec2<f32>) -> f32 {
	let lin_dim = vec2<i32>(f.lin.xy);
	for (var k = 0; k < 6; k++) {
		let up = 3.0 * pow(2.0, f32(k));
		let q = vec2<f32>(pos.x, pos.y - up);
		if (q.y < 0.0) { return 0.0; }
		if (textureLoad(lin, min(vec2<i32>(q * 0.5), lin_dim - vec2<i32>(1)), 0).r >= f.proj.w * 0.98) {
			let pd = vec2<f32>(q.x / f.lin.x - 1.0, 1.0 - q.y / f.lin.y);
			let d = f.cam[1].xyz + f.cam[2].xyz * pd.x + f.cam[3].xyz * pd.y;
			if (d.z / length(d) > -0.005) { return 0.0; }
			return floor_past_edge(d, floor_ray(d).x) * (1.0 - smoothstep(12.0, 96.0, up));
		}
	}
	return 0.0;
}

// The fog of the four nearest fog pixels at full-size pixel `pos`, re-fitted to the view depth `zc`.
fn fog_taps(pos: vec2<f32>, zc: f32) -> vec4<f32> {
	let lin_dim = vec2<i32>(f.lin.xy);
	let div = i32(f.lin.z);
	let fdim = vec2<i32>(f.size.xy);
	let fp = pos * 0.5 / f.lin.z - vec2<f32>(0.5);
	let i0 = vec2<i32>(floor(fp));
	let fr = fp - floor(fp);
	var acc = vec4<f32>(0.0);
	var wsum = 0.0;
	for (var k = 0; k < 4; k++) {
		let o = vec2<i32>(k & 1, k >> 1);
		let ci = clamp(i0 + o, vec2<i32>(0), fdim - vec2<i32>(1));
		let zt = textureLoad(lin, min(ci * div, lin_dim - vec2<i32>(1)), 0).r;
		let wb = select(1.0 - fr.x, fr.x, o.x == 1) * select(1.0 - fr.y, fr.y, o.y == 1);
		// Depth-corrected: the tap's fog is re-fitted to this pixel's depth (a homogeneous medium: transmittance ^ depth ratio), so
		// the fog meets near rock and far air without a stair-step line, and the weights can be wide.
		let w = wb * exp(-abs(zt - zc) / (0.25 * zc + 0.6)) + 1.0e-4;
		let tap = textureLoad(cur, ci, 0);
		let r = clamp(min(zc, f.march.y) / max(min(zt, f.march.y), 0.05), 0.2, 5.0);
		let ta = clamp(tap.a, 0.001, 0.999);
		let a2 = pow(ta, r);
		acc += vec4<f32>(tap.rgb * ((1.0 - a2) / (1.0 - ta)), a2) * w;
		wsum += w;
	}
	return acc / wsum;
}

// The fog over a surface at view depth `z` (`dist` from the eye) along `d`: the haze `r`, the fog floor in front of it and retail's
// faded far fog given back near the mesh edge.
fn fog_surface(pos: vec2<f32>, r: vec4<f32>, d: vec3<f32>, z: f32, dist: f32) -> vec4<f32> {
	let eye = f.cam[0].xyz;
	let pe = eye + d * z;
	let tf = exp(-floor_tau(eye, pe, floor_top(pe)));
	var back = far_fog(dist);
	if (back > 0.0) { back *= under_edge(pos); }
	return vec4<f32>(r.rgb + r.a * ((1.0 - tf) * floor_col() + tf * back * f.fog_col.rgb), r.a * tf * (1.0 - back));
}

@fragment fn fs_apply(v: VO) -> Applied {
	let lin_dim = vec2<i32>(f.lin.xy);
	var zc = textureLoad(lin, min(vec2<i32>(v.pos.xy * 0.5), lin_dim - vec2<i32>(1)), 0).r;
	// Under a blended no-Z texel (soft.rs) what lies behind it is read at full size (the half-size grid keeps the nearest of four
	// pixels: the texel's own mesh next to it).
	let sf = soft_surface(v.pos.xy);
	if (sf.y > 0.0) {
		let dz = textureLoad(depth_ms, min(vec2<i32>(v.pos.xy), vec2<i32>(textureDimensions(depth_ms)) - vec2<i32>(1)), 0);
		zc = select(f.proj.z * f.proj.w / (f.proj.w - dz * (f.proj.w - f.proj.z)), f.proj.w, dz >= 1.0);
	}
	let r = fog_taps(v.pos.xy, zc);
	// The fog floor (floor.rs) behind the haze: the solid retail-coloured fog under the ground, along the pixel's whole ray (a sky
	// pixel looking down: far out), ending at a water surface (the sea keeps its look). `back`: the share of the picture retail's
	// fog colour takes besides: a void pixel looking down past the edge of the level's mesh (the world past it reads as far fog), and
	// retail's own far fog that the volume faded, given back near its end under such a void (`under_edge`), so the mesh edge
	// meets it (user 2026-10-25).
	let pd = vec2<f32>(v.pos.x / f.lin.x - 1.0, 1.0 - v.pos.y / f.lin.y);
	let d = f.cam[1].xyz + f.cam[2].xyz * pd.x + f.cam[3].xyz * pd.y;
	let eye = f.cam[0].xyz;
	let wdim = vec2<i32>(textureDimensions(wpos));
	let wp = textureLoad(wpos, min(vec2<i32>(v.pos.xy), wdim - vec2<i32>(1)), 0);
	var o: vec4<f32>;
	if (wp.a >= 0.5) {
		var back = far_fog(distance(eye, wp.xyz));
		if (back > 0.0) { back *= under_edge(v.pos.xy); }
		o = vec4<f32>(r.rgb + r.a * back * f.fog_col.rgb, r.a * (1.0 - back));
	} else if (zc >= f.proj.w * 0.98) {
		let fr = floor_ray(d);
		let tf = exp(-floor_tau(eye, eye + d * 4000.0, fr.yz));
		let back = floor_past_edge(d, fr.x) * smoothstep(-0.005, -0.04, d.z / length(d));
		o = vec4<f32>(r.rgb + r.a * ((1.0 - tf) * floor_col() + tf * back * f.fog_col.rgb), r.a * tf * (1.0 - back));
	} else {
		o = fog_surface(v.pos.xy, r, d, zc, distance(eye, eye + d * zc));
	}
	// A blended texel that writes no Z in front (soft.rs: the alpha-faded bottoms of cliffs, pillars and buildings): fogged at its
	// own depth, weighted by its alpha, as retail fades it into the clear colour, instead of the void's deep haze and floor.
	let zs = sf.x / length(d);
	if (sf.y > 0.0 && zs < zc * 0.99 && (wp.a < 0.5 || sf.x < distance(eye, wp.xyz))) {
		o = mix(o, fog_surface(v.pos.xy, fog_taps(v.pos.xy, zs), d, zs, sf.x), sf.y);
	}
	return Applied(o, vec4<f32>(1.0 - o.a));
}
"#;

struct Pipes {
	march: wgpu::RenderPipeline,
	resolve: wgpu::RenderPipeline,
	apply: wgpu::RenderPipeline,
	layout: wgpu::BindGroupLayout,
	noise: wgpu::TextureView,
	nsamp: wgpu::Sampler,
	lsamp: wgpu::Sampler,
	dummy: wgpu::TextureView,
}

struct Targets {
	key: (u32, u32, u64),
	cur: wgpu::TextureView,
	hist: [wgpu::TextureView; 2],
}

/// The resources the pass's bind groups hold besides the targets' history views (wgpu handles compare by identity): the groups are
/// rebuilt only when one of them is a different object, not every frame.
#[derive(PartialEq)]
struct BindKey {
	lin: wgpu::TextureView,
	sun_map: wgpu::TextureView,
	water: Option<wgpu::TextureView>,
	gmap: Option<wgpu::TextureView>,
	soft: wgpu::Buffer,
	depth: wgpu::TextureView,
	cur: wgpu::TextureView,
}

/// The three bind groups (march, accumulate, apply) for each history flip, built on first use under `key`.
struct Binds {
	key: BindKey,
	groups: [Option<[wgpu::BindGroup; 3]>; 2],
}

/// The inputs of one frame.
pub(super) struct Input<'a> {
	pub cam: &'a Camera,
	pub aspect: f32,
	pub scene: &'a wgpu::TextureView,
	pub lin: &'a wgpu::TextureView,
	/// The post targets' generation and the full picture size (the grid follows them).
	pub generation: u64,
	pub size: (u32, u32),
	pub far: f32,
	pub sun_map: &'a wgpu::TextureView,
	pub sun_cmp: &'a wgpu::Sampler,
	pub sun: Option<&'a super::sunshadow::SunFrame>,
	pub level: usize,
	/// The level's retail fog (near, far, intensity near, intensity far) and colour (0..1).
	pub fog: [f32; 4],
	pub fog_col: [f32; 3],
	/// The key light's colour (0..1 display space).
	pub light: [f32; 3],
	pub sheet_z: Option<f32>,
	/// Water height on a level without air haze (Novalis): a thin mist lies on it (`density::level_params`).
	pub mist_z: Option<f32>,
	pub quality: u8,
	pub time: f32,
	/// The water pass's position target when it drew water this frame (`WaterFx::gi_mask`): the fog ends at the water surface.
	pub water: Option<&'a wgpu::TextureView>,
	/// The level's fog floor (`floor::current`): solid fog under its lowest ground.
	pub floor: Option<floor::LevelFloor>,
	/// The post's fog cover target (`Post::fog_cover`): `fs_apply` writes how much of each pixel the fog hides (post.rs fades the
	/// occlusion and the bounce light by it).
	pub cover: &'a wgpu::TextureView,
}

pub(super) struct VolFog {
	format: wgpu::TextureFormat,
	pipes: Option<Pipes>,
	ubuf: Option<wgpu::Buffer>,
	t: Option<Targets>,
	/// The bind groups of the last frames (`BindKey`).
	binds: Option<Binds>,
	/// (height, patches) per descriptor of the whole ocean (`Ocean::heights`), refreshed by `Gpu::set_ocean_heights` for the levels that
	/// want it (`wants_ocean_heights`): `Plan::new` (frame.rs) reads it for the fog sheet and the water mist.
	pub(in crate::viewer) ocean_heights: Vec<(f32, usize)>,
	/// The previous frame's camera rows, the frame count since the history was valid, which history texture is the newer.
	prev: Option<[[f32; 4]; 4]>,
	frames: u32,
	flip: usize,
	level: usize,
	ground: f32,
	/// The fog floor's clear-top map texture with its generation and origin (`floor::ground_texture`).
	gmap: (u32, Option<(wgpu::TextureView, [f32; 2])>),
	last_time: f32,
	/// GPU timestamps of the three passes (RAC_FOG_LOG=1, `gputime.rs`).
	timer: Option<gputime::GpuTimer>,
	/// The blended no-Z texels' distance and alpha (`soft.rs`), written by the level pass, read by `fs_apply`.
	pub(in crate::viewer) soft: soft::Soft,
}

pub(super) fn cam_rows(cam: &Camera, aspect: f32) -> [[f32; 4]; 4] {
	let f = forward(cam);
	let r = normalise(cross(f, cam.up));
	let u = cross(r, f);
	let (tx, ty) = (cam.tan_half * aspect, cam.tan_half);
	[[cam.pos[0], cam.pos[1], cam.pos[2], 0.0], [f[0], f[1], f[2], 0.0], [r[0] * tx, r[1] * tx, r[2] * tx, 0.0], [u[0] * ty, u[1] * ty, u[2] * ty, 0.0]]
}

impl VolFog {
	/// A frame on which the pass does not draw (`Plan::new` is None: Classic, under water, a level without a volume): the bind-group
	/// cache lets go of the full-size views it holds (MSAA depth, `lin`, the soft buffer), so a resize or a render scale change
	/// while the fog is off does not keep the old ones alive. One `is_some` check per frame while nothing is cached.
	pub(super) fn idle(&mut self) {
		if self.binds.is_some() {
			self.binds = None;
		}
	}

	/// `ulayout`: the level shader's group 0 (`soft.rs` rebinds it with the no-Z texel buffer).
	pub(super) fn new(format: wgpu::TextureFormat, ulayout: &wgpu::BindGroupLayout) -> VolFog {
		VolFog { format, pipes: None, ubuf: None, t: None, binds: None, ocean_heights: Vec::new(), prev: None, frames: 0, flip: 0, level: usize::MAX, ground: 0.0, gmap: (u32::MAX, None), last_time: 0.0, timer: None, soft: soft::Soft::new(ulayout) }
	}

	/// The whole WGSL of the pass.
	fn source() -> String {
		format!("{}{}{}{}{}{}{}", scatter::BLOCK, density::FOG_DENSITY, floor::FLOOR, scatter::MARCH, temporal::RESOLVE, soft::READ, APPLY)
	}

	fn build(&self, device: &wgpu::Device, queue: &wgpu::Queue) -> Pipes {
		let src = Self::source();
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("volfog"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("volfog", src.into())) });
		let entry = |binding: u32, ty: wgpu::BindingType| wgpu::BindGroupLayoutEntry { binding, visibility: wgpu::ShaderStages::FRAGMENT, ty, count: None };
		let tex2 = |filterable: bool| wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false };
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("volfog"),
			entries: &[
				entry(0, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }),
				entry(1, tex2(false)),
				entry(2, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false }),
				entry(3, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
				entry(4, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }),
				entry(5, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Comparison)),
				entry(6, tex2(true)),
				entry(7, tex2(true)),
				entry(8, wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering)),
				entry(9, tex2(false)),
				entry(10, tex2(false)),
				entry(11, wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }),
				entry(12, wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: true }),
			],
		});
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("volfog"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		// The apply pass also writes the cover target; the picture keeps its alpha (post.rs reads the mobys' cover from it).
		let make = |fs: &'static str, target: wgpu::TextureFormat, blend: Option<wgpu::BlendState>| {
			let apply = fs == "fs_apply";
			let mask = if apply { wgpu::ColorWrites::COLOR } else { wgpu::ColorWrites::ALL };
			let cover = apply.then_some(wgpu::ColorTargetState { format: super::post::FOG_COVER_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL });
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(fs),
				layout: Some(&pl),
				vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
				fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: target, blend, write_mask: mask }), cover][..if apply { 2 } else { 1 }] }),
				primitive: Default::default(),
				depth_stencil: None,
				multisample: Default::default(),
				multiview_mask: None,
				cache: None,
			})
		};
		// The picture times the transmittance (the fragment's alpha) plus the scattered light.
		let over = wgpu::BlendState {
			color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::SrcAlpha, operation: wgpu::BlendOperation::Add },
			alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::Zero, operation: wgpu::BlendOperation::Add },
		};
		let (noise, nsamp) = noise::upload(device, queue);
		let lsamp = device.create_sampler(&wgpu::SamplerDescriptor { label: Some("volfog linear"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() });
		let dummy = device
			.create_texture(&wgpu::TextureDescriptor {
				label: Some("volfog dummy"),
				size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: FORMAT,
				usage: wgpu::TextureUsages::TEXTURE_BINDING,
				view_formats: &[],
			})
			.create_view(&Default::default());
		Pipes { march: make("fs_march", FORMAT, None), resolve: make("fs_resolve", FORMAT, None), apply: make("fs_apply", self.format, Some(over)), layout, noise, nsamp, lsamp, dummy }
	}

	fn targets(&self, device: &wgpu::Device, inp: &Input, div: u32) -> Targets {
		let (fw, fh) = (inp.size.0.div_ceil(2).div_ceil(div).max(1), inp.size.1.div_ceil(2).div_ceil(div).max(1));
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
		Targets { key: (fw, fh, inp.generation), cur: make("volfog now"), hist: [make("volfog history 0"), make("volfog history 1")] }
	}

	/// The uniform of this frame; `fresh` = no usable history (the current frame counts fully).
	fn uniform(&self, inp: &Input, q: Quality, p: &density::Params, fresh: bool, fsize: (u32, u32)) -> Uniform {
		let floor = inp.floor.map(|fl| fl.at());
		let cam = cam_rows(inp.cam, inp.aspect);
		let mut u: Uniform = Zeroable::zeroed();
		u.cam = cam;
		u.prev = self.prev.unwrap_or(cam);
		// The sun's cascades exactly as the world shaders read them (sunshadow.rs `write_uniforms`).
		let mut su: Uniforms = Zeroable::zeroed();
		super::sunshadow::write_uniforms(&mut su, inp.sun);
		(u.sun_vp, u.sun, u.sun_bias, u.sun_texel, u.sun_q) = (su.sun_vp, su.sun, su.sun_bias, su.sun_texel, su.sun_q);
		u.sun_q[3] = if q.shadows && inp.sun.is_some() { 1.0 } else { 0.0 };
		if inp.sun.is_none() {
			// No key light this frame: a fixed direction for the phase only, no shafts.
			u.sun = [-0.4, -0.3, -0.85, 0.0];
		}
		// The glow round the sun follows the sun the level's sky paints (post.rs `sky_sun`), so the fog does not make a second sun; the
		// shadow map (shafts) keeps the key light. A level whose sky paints no sun uses the key light.
		u.phase = match super::post::sky_sun(inp.level) {
			Some(to) if crate::devtune::f32("fog_skyphase", 1.0) > 0.5 => [-to[0], -to[1], -to[2], 0.0],
			_ => [u.sun[0], u.sun[1], u.sun[2], 0.0],
		};
		if frame::fog_log() {
			eprintln!("volfog: key light travel dir {:?} strength {:.2}; sky sun travel dir {:?}", &u.sun[..3], u.sun[3], &u.phase[..3]);
		}
		let t = |name, d| crate::devtune::f32(name, d);
		let lc = inp.light;
		u.fog_col = [inp.fog_col[0], inp.fog_col[1], inp.fog_col[2], 1.0];
		u.sun_col = [lc[0], lc[1], lc[2], t("fog_g", 0.7)];
		let amb = t("fog_amb", 0.65);
		let sky = t("fog_sky", 0.12);
		u.amb_col = [inp.fog_col[0] * amb + lc[0] * sky, inp.fog_col[1] * amb + lc[1] * sky, inp.fog_col[2] * amb + lc[2] * sky, 0.0];
		u.dens = [p.sigma * t("fog_k", 1.0), p.height * t("fog_h", 1.0), self.ground, p.haze];
		if let Some(s) = p.sheet {
			// The thick part stands over the retail sheet, dense with a short soft top: 22 units over a water mist, 6 over Kerwan's sheet
			// (user 2026-10-25: at 22 it "covers whole buildings"; the fog floor, floor.rs, hides the tower bottoms at the sheet);
			// fog_sheet_lift / fog_sheet_h / fog_sheet tune it.
			u.sheet = [s.z + t("fog_sheet_lift", if inp.sheet_z.is_some() { 6.0 } else { 22.0 }), s.height * t("fog_sheet_h", if inp.sheet_z.is_some() { 0.4 } else { 0.6 }), s.sigma * t("fog_sheet", 3.0), 1.0];
		}
		u.wind = [0.8, 0.35, t("fog_wind", 0.7), inp.time];
		u.look = [t("fog_wisp", 0.85), t("fog_scale", 46.0), t("fog_detail", 17.0), t("fog_sun", 0.4)];
		u.size = [fsize.0 as f32, fsize.1 as f32, 1.0 / fsize.0 as f32, 1.0 / fsize.1 as f32];
		u.march = [q.steps, p.cap.min(inp.far), (self.frames % JITTER_PERIOD) as f32, if fresh { 1.0 } else { temporal::BLEND }];
		u.proj = [inp.cam.tan_half * inp.aspect, inp.cam.tan_half, RETAIL_NEAR, inp.far];
		let g = inp.fog;
		let slope = (g[3] - g[2]) / (g[1] - g[0]).max(0.001);
		// Retail's GS fog unfaded in the previous camera's unused w slots (`gs` in APPLY): intercept, slope per unit, lowest value, fade.
		let gs = [g[2] - g[0] * slope, slope, g[2].min(g[3]), crate::retail::pcext::fogopt::gs_scale()];
		(0..4).for_each(|k| u.prev[k][3] = gs[k]);
		u.lin = [inp.size.0.div_ceil(2) as f32, inp.size.1.div_ceil(2) as f32, q.div as f32, t("fog_deep", 6.0)];
		if let Some(fl) = floor {
			// The fog floor (floor.rs) in slots the fog leaves unused: top z, soft height, extinction.
			(u.fog_col[3], u.amb_col[3], u.phase[3]) = (fl.z, fl.height, fl.sigma);
			let at = self.gmap.1.as_ref().map_or([0.0; 2], |g| g.1);
			(u.cam[1][3], u.cam[2][3], u.cam[3][3]) = (at[0], at[1], inp.floor.map_or(fl.z, |l| l.clear));
			let c = inp.floor.map_or(inp.fog_col, |l| l.colour(inp.fog_col));
			u.floor_c = [c[0], c[1], c[2], 0.0];
		}
		u
	}

	/// Marches, accumulates and applies the fog into `inp.scene`. False (and the history dropped) when the level has none. `depth`: the
	/// level's MSAA depth (sample 0 tells `fs_apply` what lies behind a blended no-Z texel, `soft.rs`).
	pub(super) fn run(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, inp: &Input, depth: &wgpu::TextureView) -> bool {
		let (Some(mut q), true) = (quality(inp.quality), inp.size.0 > 0) else { return false };
		if let (3.., Some(div)) = (inp.quality, super::gfxq::fog_div()) {
			q.div = div; // RAC_FOG_DIV (gfxq.rs): an override, unset = `quality`
		}
		let params = density::level_params(inp.level, inp.fog, inp.sheet_z, inp.mist_z, crate::devtune::f32("fog_amount", 0.8) * q.amount);
		if frame::fog_log() {
			eprintln!("volfog: run quality {} params {params:?} sheet_z {:?} floor {:?} size {:?}", inp.quality, inp.sheet_z, inp.floor, inp.size);
		}
		let Some(params) = params else {
			self.prev = None;
			return false;
		};
		let _s = super::profile::span("volfog");
		if let Some(tm) = &mut self.timer {
			tm.poll(device);
		}
		if self.pipes.is_none() {
			// [PC] pipe_guard.rs: a driver that refuses the fog shader (DX12 FXC) turns the fog off, it does not crash the game.
			let Some(p) = super::pipe_guard::guard(device, super::pipe_guard::Effect::VolFog, || self.build(device, queue)) else { return false };
			self.pipes = Some(p);
			self.timer = gputime::GpuTimer::new(device, queue);
		}
		let key_ok = self.t.as_ref().is_some_and(|t| t.key.2 == inp.generation && t.key.0 == inp.size.0.div_ceil(2).div_ceil(q.div) && t.key.1 == inp.size.1.div_ceil(2).div_ceil(q.div));
		if !key_ok {
			self.t = Some(self.targets(device, inp, q.div));
			self.prev = None;
		}
		let dt = (inp.time - self.last_time).clamp(0.0, 0.1);
		self.last_time = inp.time;
		let want = (inp.cam.pos[2] - 3.0).min(crate::devtune::f32("fog_cap", density::pool_cap(inp.level)));
		if let Some(g) = floor::ground_texture(device, queue, self.gmap.0) {
			self.gmap = g;
		}
		if self.level != inp.level || self.prev.is_none() {
			self.ground = want;
			self.level = inp.level;
		} else {
			self.ground += (want - self.ground) * (1.0 - (-dt / 1.5).exp());
		}
		if let (true, Some(fl)) = (frame::fog_log(), inp.floor) {
			eprintln!("volfog: floor clear {:.2} soft {:.1} cam {:?}", fl.clear, fl.soft, inp.cam.pos);
		}
		let fresh = self.prev.is_none();
		let (read, write) = (self.flip, 1 - self.flip);
		let ubuf = self.ubuf.get_or_insert_with(|| device.create_buffer(&wgpu::BufferDescriptor { label: Some("volfog"), size: std::mem::size_of::<Uniform>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false })).clone();
		let Some(fsize) = self.t.as_ref().map(|t| (t.key.0, t.key.1)) else { return false };
		let u = self.uniform(inp, q, &params, fresh, fsize);
		queue.write_buffer(&ubuf, 0, bytemuck::bytes_of(&u));
		let (Some(pipes), Some(t), Some(soft)) = (&self.pipes, &self.t, self.soft.buffer()) else { return false };
		let key = BindKey { lin: inp.lin.clone(), sun_map: inp.sun_map.clone(), water: inp.water.cloned(), gmap: self.gmap.1.as_ref().map(|g| g.0.clone()), soft: soft.clone(), depth: depth.clone(), cur: t.cur.clone() };
		let mut binds = self.binds.take().filter(|b| b.key == key).unwrap_or(Binds { key, groups: [None, None] });
		let group = |depth: &wgpu::TextureView, cur: &wgpu::TextureView, hist: &wgpu::TextureView| {
			device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("volfog"),
				layout: &pipes.layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(inp.lin) },
					wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&pipes.noise) },
					wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&pipes.nsamp) },
					wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(inp.sun_map) },
					wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(inp.sun_cmp) },
					wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(cur) },
					wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(hist) },
					wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::Sampler(&pipes.lsamp) },
					wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(inp.water.unwrap_or(&pipes.dummy)) },
					wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(self.gmap.1.as_ref().map_or(&pipes.dummy, |g| &g.0)) },
					wgpu::BindGroupEntry { binding: 11, resource: soft.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 12, resource: wgpu::BindingResource::TextureView(depth) },
				],
			})
		};
		let timer = &self.timer;
		let pass = |enc: &mut wgpu::CommandEncoder, i: u32, label: &str, target: &wgpu::TextureView, pipeline: &wgpu::RenderPipeline, g: &wgpu::BindGroup, load: wgpu::LoadOp<wgpu::Color>| {
			let at = |view, load| Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } });
			let both = [at(target, load), at(inp.cover, wgpu::LoadOp::Clear(wgpu::Color::BLACK))];
			let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some(label),
				color_attachments: &both[..if i == 2 { 2 } else { 1 }],
				depth_stencil_attachment: None,
				timestamp_writes: timer.as_ref().and_then(|tm| tm.writes(i)).or_else(|| crate::viewer::passtime::rw(label)),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			p.set_pipeline(pipeline);
			p.set_bind_group(0, g, &[]);
			p.draw(0..3, 0..1);
		};
		let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
		let groups = binds.groups[read].get_or_insert_with(|| {
			let dv = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
			[group(&dv, &pipes.dummy, &pipes.dummy), group(&dv, &t.cur, &t.hist[read]), group(&dv, &t.hist[write], &pipes.dummy)]
		});
		pass(enc, 0, "volfog march", &t.cur, &pipes.march, &groups[0], clear);
		pass(enc, 1, "volfog accumulate", &t.hist[write], &pipes.resolve, &groups[1], clear);
		pass(enc, 2, "volfog apply", inp.scene, &pipes.apply, &groups[2], wgpu::LoadOp::Load);
		if let Some(tm) = &mut self.timer {
			tm.finish(enc);
		}
		self.binds = Some(binds);
		self.prev = Some(cam_rows(inp.cam, inp.aspect));
		self.flip = write;
		self.frames = self.frames.wrapping_add(1);
		true
	}
}

#[cfg(test)]
mod tests;
