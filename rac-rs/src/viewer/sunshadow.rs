//! [PC] Dynamic shadows (Pause -> Options -> Graphics -> Dynamic shadows On, with Lighting Enhanced; retail/pcext.rs
//! `graphics`; no retail counterpart). Off, or with Lighting Classic, nothing here runs.
//!
//! - The light: the level's own outdoor light, light row 0 (the pair a hero or moby outside every light zone takes; on all 19
//!   planets that is the sun), its brighter direction. One light for the whole level: following the hero's zone light (a
//!   hangar's lamp is 90 degrees off Veldin's sun) turned every shadow round as he walked through a doorway. Should the row
//!   change while the level runs, the light turns over about three quarters of a second (time based; a big turn also softens
//!   the shadows while it lasts); a new level starts at its light. It is raised to at least `MIN_RISE` above the horizon (a low
//!   key light, Veldin's 20 degrees, threw shadows almost three times an object's height across the whole view). The strength
//!   follows the light's brightness: a level without a real key light (space, ship scenes) gets weak or no shadows. A light
//!   from below the horizon casts none.
//! - Three cascades (view depth up to 12, 36 and 110 world units), 2048 x 2048 each, every one fitted to the bounding sphere of
//!   its slice of the view (the same size whatever way the camera turns) and moved in whole texels (no shimmer while the camera
//!   moves). Each reaches 200 units towards the light, so buildings behind the camera still shade the ground in front of it.
//! - Casters: the static world (terrain fragments and ties with the LOD mesh the frame drew them with, so a surface never
//!   shadows itself through a coarser copy of its own mesh; undrawn ones with the full mesh; the merged world) culled per cascade by their
//!   bounding spheres, and every moby of this frame (the props batch as `vs_props` poses it, the hero, debris and the clipped
//!   props; not see-through shells such as blast domes and glows, nor a moby faded below alpha 0x60). Depth, cut by texel alpha
//!   where the main pass cuts (shrubs, terrain below 96/128, ties below 64/128: a see-through overlay casts nothing), both
//!   faces, slope-scaled depth bias; one multi-draw per batch per cascade.
//! - Receivers (viewer.rs SHADER `sun_occ`): every opaque world, shrub, terrain and moby fragment, 4x4 hardware PCF, the light's
//!   offset by 2 texels against acne. Baked world colours already hold 2002's painted shadows, so a fragment darkens by
//!   STRENGTH x how lit its baked colour is (a painted shadow takes no second one); a moby keeps its ambient and loses its
//!   directional light. The retail shadow volumes (shadowvol.rs) are not drawn while this runs (no double shadows).

use super::*;

pub(super) const CASCADES: usize = 3;
/// Shadow map size per cascade (the allocation; Low and Medium draw into the corner of it, see `Quality`).
pub(super) const RES: u32 = 2048;
/// Far end of each cascade (view depth, world units) at High.
const SPLITS: [f32; CASCADES] = [12.0, 36.0, 110.0];

/// Graphics -> Dynamic shadows Low / Medium / High: what the level costs the GPU. High is the whole map (`RES`, three cascades
/// out to `SPLITS`, 4x4 hardware PCF taps) = the picture before the levels existed. Low and Medium render into the corner
/// `res` x `res` of the same map (the receivers scale their map uv by `res / RES`, `write_uniforms`), so a change of level
/// needs no new texture or bind group; a smaller map also means fewer casters (the cascades reach less far), fewer cascades
/// (Low: two, the third is not drawn or read) and a smaller PCF kernel (`pcf` x `pcf` compare taps per receiver pixel).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Quality {
	/// Texels per cascade side actually drawn and read.
	pub res: u32,
	/// Cascades drawn and read (the last one fades out at its rim).
	pub cascades: usize,
	pub splits: [f32; CASCADES],
	/// PCF kernel side (taps = pcf^2).
	pub pcf: u32,
}

/// The shadow settings of a Graphics level (`Graphics::shadows_q`: 1 Low, 2 Medium, anything else High).
pub(super) const fn quality(level: u8) -> Quality {
	match level {
		1 => Quality { res: 1024, cascades: 2, splits: [16.0, 56.0, 56.0], pcf: 2 },
		2 => Quality { res: 1536, cascades: 3, splits: [12.0, 36.0, 80.0], pcf: 3 },
		_ => Quality { res: RES, cascades: CASCADES, splits: SPLITS, pcf: 4 },
	}
}
/// How far each cascade reaches towards the light beyond its sphere (world units).
const REACH: f32 = 200.0;
/// The darkest a fully lit surface gets in a shadow (fraction of its light removed).
const STRENGTH: f32 = 0.6;
/// The lowest the light may stand (sine of its elevation, 35 degrees): a lower key light keeps its compass heading but is raised.
const MIN_RISE: f32 = 0.5736;
/// Time constant of the light's turn towards a new zone's (seconds).
const TURN_TAU: f32 = 0.25;

const SUN_SHADER: &str = r#"
struct SU { vp: mat4x4<f32> };
@group(0) @binding(0) var<uniform> su: SU;

@vertex fn vs_sh(@location(0) p: vec3<f32>) -> @builtin(position) vec4<f32> {
	return su.vp * vec4<f32>(p, 1.0);
}

// vs_props' placement (viewer.rs SHADER), position only; a hidden or faded slot casts nothing.
struct PropInst { m0: vec4<f32>, m1: vec4<f32>, m2: vec4<f32>, m3: vec4<f32>, light: vec4<f32>, tint: vec4<f32>, ptint: vec4<f32>, a: vec4<f32>, k: vec4<u32> };
@group(1) @binding(0) var<storage, read> insts: array<PropInst>;
@group(1) @binding(1) var<storage, read> poses: array<f32>;

@vertex fn vs_sh_props(@location(0) p: vec3<f32>, @location(5) sj: vec2<u32>) -> @builtin(position) vec4<f32> {
	let s = insts[sj.x];
	let j = sj.y;
	let kind = s.k.x & 0xffu;
	var q = p;
	if (j < s.k.w) {
		let b = (s.k.z + j) * 6u;
		q = vec3<f32>(poses[b], poses[b + 1u], poses[b + 2u]);
	}
	// A faded moby (draw alpha below 0x60, `moby_alpha` in viewer.rs: an explosion or glow fading out) casts nothing.
	let alpha = 128.0 - round(abs(s.a.y - floor(s.a.y + 0.5)) * 512.0);
	if ((kind == 1u || kind == 2u) && (s.k.x & 0x100u) == 0u && alpha >= 96.0) {
		var pos = q + s.m3.xyz;
		if (kind == 1u) {
			let a = q * s.a.x;
			pos = s.m0.xyz * a.x + s.m1.xyz * a.y + s.m2.xyz * a.z + s.m3.xyz;
		}
		return su.vp * vec4<f32>(pos, 1.0);
	}
	return vec4<f32>(0.0, 0.0, -2.0, 1.0);
}
"#;

/// Shrubs, terrain and ties cast with their texel alpha: the bindless texture array (bindless.rs), slot in the low 16 bits of
/// the first instance (`bindless::first_instance`). A shrub texel below alpha 0x30 / 0x80 casts nothing (the leaves' holes); a
/// terrain texel below 0x60 and a tie texel below 0x40 neither, the alphas from which the main pass writes depth (`fs_tfrag`,
/// `fs_tie`): a see-through overlay (a sand patch laid over the ground, its outside clear) shadowed the ground right under it
/// in a fine ripple and cut other shadows off at its invisible edge.
fn shrub_shader() -> String {
	format!(
		r#"enable wgpu_binding_array;
struct SU {{ vp: mat4x4<f32> }};
@group(0) @binding(0) var<uniform> su: SU;
@group(1) @binding(0) var texs: binding_array<texture_2d<f32>, {}>;
@group(1) @binding(1) var samp: sampler;
struct SOut {{ @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32>, @location(1) @interpolate(flat) ti: u32 }};

@vertex fn vs_sh_shrub(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @builtin(instance_index) inst: u32) -> SOut {{
	var o: SOut;
	o.pos = su.vp * vec4<f32>(p, 1.0);
	o.uv = uv;
	o.ti = inst & 0xffffu;
	return o;
}}

@fragment fn fs_sh_shrub(i: SOut) {{
	if (textureSampleLevel(texs[i.ti], samp, i.uv, 0.0).a < 0.375) {{ discard; }}
}}

@fragment fn fs_sh_tfrag(i: SOut) {{
	if (textureSampleLevel(texs[i.ti], samp, i.uv, 0.0).a * 128.0 < 95.5) {{ discard; }}
}}

@fragment fn fs_sh_tie(i: SOut) {{
	if (textureSampleLevel(texs[i.ti], samp, i.uv, 0.0).a * 128.0 < 63.5) {{ discard; }}
}}
"#,
		super::bindless::TEX_CAP
	)
}

/// One cascade: its light view-projection and the light-space box it covers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Cascade {
	pub vp: [[f32; 4]; 4],
	/// World units per shadow texel.
	pub texel: f32,
	/// Light basis (right, up, travel direction) and the box: x, y within `r` of (cx, cy), z (along the light) in zmin..zmax.
	basis: [[f32; 3]; 3],
	c: [f32; 2],
	r: f32,
	z: [f32; 2],
}

fn dot3(a: [f32; 3], b: [f32; 3]) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm3(v: [f32; 3]) -> [f32; 3] {
	let l = dot3(v, v).sqrt().max(1e-9);
	[v[0] / l, v[1] / l, v[2] / l]
}

impl Cascade {
	/// The cascade for the view slice `near..far` of a camera at `pos` looking along `fwd` (vertical half-angle tangent
	/// `tan_half`, `aspect` width / height), lit along `dir` (the light's travel direction), on a `res`-texel map.
	pub(super) fn fit(pos: [f32; 3], fwd: [f32; 3], tan_half: f32, aspect: f32, near: f32, far: f32, dir: [f32; 3], res: u32) -> Cascade {
		// Bounding sphere of the slice: on the view axis where the near and far corners are equally far (k = corner spread^2).
		let k = tan_half * tan_half * (1.0 + aspect * aspect);
		let t = ((far + near) * (1.0 + k) / 2.0).min(far);
		let r = ((far - t) * (far - t) + far * far * k).sqrt().max(((t - near) * (t - near) + near * near * k).sqrt());
		let centre = [pos[0] + fwd[0] * t, pos[1] + fwd[1] * t, pos[2] + fwd[2] * t];
		let d = norm3(dir);
		let up_ref = if d[2].abs() < 0.99 { [0.0, 0.0, 1.0] } else { [1.0, 0.0, 0.0] };
		let right = norm3([d[1] * up_ref[2] - d[2] * up_ref[1], d[2] * up_ref[0] - d[0] * up_ref[2], d[0] * up_ref[1] - d[1] * up_ref[0]]);
		let up = [right[1] * d[2] - right[2] * d[1], right[2] * d[0] - right[0] * d[2], right[0] * d[1] - right[1] * d[0]];
		// Whole texels only: the map slides under the world instead of resampling it.
		let texel = 2.0 * r / res as f32;
		let snap = |v: f32| (v / texel).floor() * texel;
		let (cx, cy, cz) = (snap(dot3(centre, right)), snap(dot3(centre, up)), dot3(centre, d));
		let (zmin, zmax) = (cz - r - REACH, cz + r);
		let zr = zmax - zmin;
		let vp = [
			[right[0] / r, up[0] / r, d[0] / zr, 0.0],
			[right[1] / r, up[1] / r, d[1] / zr, 0.0],
			[right[2] / r, up[2] / r, d[2] / zr, 0.0],
			[-cx / r, -cy / r, -zmin / zr, 1.0],
		];
		Cascade { vp, texel, basis: [right, up, d], c: [cx, cy], r, z: [zmin, zmax] }
	}

	/// Whether a sphere can cast into the cascade.
	pub(super) fn touches(&self, centre: [f32; 3], radius: f32) -> bool {
		let [right, up, d] = self.basis;
		(dot3(centre, right) - self.c[0]).abs() <= self.r + radius && (dot3(centre, up) - self.c[1]).abs() <= self.r + radius && {
			let z = dot3(centre, d);
			z >= self.z[0] - radius && z <= self.z[1] + radius
		}
	}
}

/// This frame's light: travel direction, strength (0..STRENGTH) and the cascades.
pub(super) struct SunFrame {
	pub dir: [f32; 3],
	pub strength: f32,
	pub cascades: [Cascade; CASCADES],
	pub q: Quality,
}

/// Writes this frame's shadow uniforms (`Uniforms::sun_*`): cascade matrices, (dir, strength), per cascade the receiver depth
/// bias (one texel's worth of depth) and map size, texel sizes. None leaves them zero: the shaders skip every shadow read.
pub(super) fn write_uniforms(u: &mut Uniforms, f: Option<&SunFrame>) {
	let Some(f) = f else { return };
	u.sun_vp = f.cascades.map(|c| c.vp);
	u.sun = [f.dir[0], f.dir[1], f.dir[2], f.strength];
	let c = &f.cascades;
	let bias = |k: usize| c[k].texel / (c[k].z[1] - c[k].z[0]);
	u.sun_bias = [bias(0), bias(1), bias(2), RES as f32];
	u.sun_texel = [c[0].texel, c[1].texel, c[2].texel, 0.0];
	// Receivers: PCF kernel side, map uv scale (the drawn corner of the map; 1.0 at High), cascades read.
	u.sun_q = [f.q.pcf as f32, f.q.res as f32 / RES as f32, f.q.cascades as f32, 0.0];
}

/// The key light of a blended light pair: rows as `Gpu::light_rows` gives them (colour A + wrap, to-light A, colour B + wrap,
/// to-light B), slots `a` / `b` mixed by `t` as `slot_blend` does. Returns (travel direction, brightness), None without a lit
/// direction.
pub(super) fn key_light(rows: &[[[f32; 4]; 4]; 16], a: usize, b: usize, t: f32) -> Option<([f32; 3], f32)> {
	let (ra, rb) = (rows[a.min(15)], rows[b.min(15)]);
	let mix = |x: [f32; 4], y: [f32; 4]| [0, 1, 2, 3].map(|k| x[k] + (y[k] - x[k]) * t);
	let luma = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
	let pairs = [(mix(ra[0], rb[0]), mix(ra[1], rb[1])), (mix(ra[2], rb[2]), mix(ra[3], rb[3]))];
	pairs
		.into_iter()
		.filter(|(_, to)| dot3([to[0], to[1], to[2]], [to[0], to[1], to[2]]) > 1e-6)
		.map(|(c, to)| (norm3([-to[0], -to[1], -to[2]]), luma(c)))
		.max_by(|x, y| x.1.total_cmp(&y.1))
}

/// Shadow strength for a key light of brightness `luma` travelling along `dir`: none from below the horizon or for a dim light.
pub(super) fn strength(dir: [f32; 3], luma: f32) -> f32 {
	let down = (-dir[2]).clamp(0.0, 1.0);
	let lit = ((luma - 0.1) / 0.4).clamp(0.0, 1.0);
	STRENGTH * lit * (down / 0.15).min(1.0)
}

/// A light travelling along `d` raised to at least `MIN_RISE` above the horizon, compass heading kept (a light from below the
/// horizon, or straight down, is left as it is).
pub(super) fn raise(d: [f32; 3]) -> [f32; 3] {
	let down = -d[2];
	let h = (d[0] * d[0] + d[1] * d[1]).sqrt();
	if down <= 0.0 || down >= MIN_RISE || h < 1e-6 {
		return d;
	}
	let k = (1.0 - MIN_RISE * MIN_RISE).sqrt() / h;
	[d[0] * k, d[1] * k, -MIN_RISE]
}

/// Turns unit vector `a` towards unit vector `b` by the fraction `k` of the angle between them (along the great circle; through
/// straight down when they are opposite).
pub(super) fn turn(a: [f32; 3], b: [f32; 3], k: f32) -> [f32; 3] {
	let c = dot3(a, b).clamp(-1.0, 1.0);
	if c > 0.99999 {
		return b;
	}
	if c < -0.999 {
		let mid = turn(a, [0.0, 0.0, -1.0], 0.999);
		return turn(mid, b, (2.0 * k - 1.0).max(0.0));
	}
	let th = c.acos();
	let (wa, wb) = (((1.0 - k) * th).sin(), (k * th).sin());
	norm3([0, 1, 2].map(|i| a[i] * wa + b[i] * wb))
}

/// A batch the shadow pass draws: which buffers, which pipeline.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Src {
	World,
	Tfrags,
	Ties,
	Props,
	Marker,
	Clip,
	Debris,
	Shrubs,
}

pub(super) struct SunShadow {
	tex: wgpu::Texture,
	layers: [wgpu::TextureView; CASCADES],
	/// The whole array, sampled by the world shaders (bound in the level's group 0 with `cmp`).
	pub array: wgpu::TextureView,
	pub cmp: wgpu::Sampler,
	ubuf: [wgpu::Buffer; CASCADES],
	ubind: [wgpu::BindGroup; CASCADES],
	pipeline: wgpu::RenderPipeline,
	pipeline_props: Option<wgpu::RenderPipeline>,
	/// Texel-alpha casters (`shrub_shader`): shrubs, terrain, ties.
	pipeline_shrub: Option<wgpu::RenderPipeline>,
	pipeline_tfrag: Option<wgpu::RenderPipeline>,
	pipeline_tie: Option<wgpu::RenderPipeline>,
	indirect: Option<wgpu::Buffer>,
	cap: usize,
	/// The eased key light (direction before `raise`, strength) and the scene it belongs to (a new level snaps), and when it
	/// was last eased.
	dir: Option<([f32; 3], f32)>,
	scene_id: usize,
	last: Option<std::time::Instant>,
}

impl SunShadow {
	/// `skin_layout`: the GPU-posed props' storage group (None: CPU posing); `array_layout`: the bindless texture array's
	/// layout when draws can carry their slot in the first instance (None: shrubs cast nothing, terrain and ties cast whole).
	pub(super) fn new(device: &wgpu::Device, skin_layout: Option<&wgpu::BindGroupLayout>, array_layout: Option<&wgpu::BindGroupLayout>) -> SunShadow {
		let tex = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("sun shadow map"),
			size: wgpu::Extent3d { width: RES, height: RES, depth_or_array_layers: CASCADES as u32 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: wgpu::TextureFormat::Depth32Float,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		});
		let layers = std::array::from_fn(|k| tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: k as u32, array_layer_count: Some(1), ..Default::default() }));
		let array = tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
		let cmp = device.create_sampler(&wgpu::SamplerDescriptor {
			label: Some("sun shadow compare"),
			mag_filter: wgpu::FilterMode::Linear,
			min_filter: wgpu::FilterMode::Linear,
			compare: Some(wgpu::CompareFunction::LessEqual),
			..Default::default()
		});
		let ulayout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("sun shadow uniforms"),
			entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
		});
		let ubuf: [wgpu::Buffer; CASCADES] = std::array::from_fn(|_| device.create_buffer(&wgpu::BufferDescriptor { label: Some("sun shadow cascade"), size: 64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
		let ubind = std::array::from_fn(|k| device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("sun shadow cascade"), layout: &ulayout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf[k].as_entire_binding() }] }));
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sun shadow"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("sun shadow", SUN_SHADER.into())) });
		let vertex = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3] };
		let sj = wgpu::VertexBufferLayout { array_stride: 8, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![5 => Uint32x2] };
		let make = |layouts: &[Option<&wgpu::BindGroupLayout>], entry: &str, buffers: &[Option<wgpu::VertexBufferLayout>]| {
			let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sun shadow"), bind_group_layouts: layouts, immediate_size: 0 });
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(entry),
				layout: Some(&pl),
				vertex: wgpu::VertexState { module: &shader, entry_point: Some(entry), compilation_options: Default::default(), buffers },
				fragment: None,
				primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
				depth_stencil: Some(wgpu::DepthStencilState {
					format: wgpu::TextureFormat::Depth32Float,
					depth_write_enabled: Some(true),
					depth_compare: Some(wgpu::CompareFunction::Less),
					stencil: Default::default(),
					bias: wgpu::DepthBiasState { constant: 0, slope_scale: 1.5, clamp: 0.0 },
				}),
				multisample: Default::default(),
				multiview_mask: None,
				cache: None,
			})
		};
		let pipeline = make(&[Some(&ulayout)], "vs_sh", &[Some(vertex.clone())]);
		let pipeline_props = skin_layout.map(|sl| make(&[Some(&ulayout), Some(sl)], "vs_sh_props", &[Some(vertex.clone()), Some(sj)]));
		let cut = array_layout.map(|al| {
			let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("sun shadow texel alpha"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("sun shadow texel alpha", shrub_shader().into())) });
			let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("sun shadow texel alpha"), bind_group_layouts: &[Some(&ulayout), Some(al)], immediate_size: 0 });
			(module, pl)
		});
		let make_cut = |fs: &str| cut.as_ref().map(|(module, pl)| {
			let vu = wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2] };
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(fs),
				layout: Some(pl),
				vertex: wgpu::VertexState { module, entry_point: Some("vs_sh_shrub"), compilation_options: Default::default(), buffers: &[Some(vu)] },
				fragment: Some(wgpu::FragmentState { module, entry_point: Some(fs), compilation_options: Default::default(), targets: &[] }),
				primitive: wgpu::PrimitiveState { cull_mode: None, ..Default::default() },
				depth_stencil: Some(wgpu::DepthStencilState {
					format: wgpu::TextureFormat::Depth32Float,
					depth_write_enabled: Some(true),
					depth_compare: Some(wgpu::CompareFunction::Less),
					stencil: Default::default(),
					bias: wgpu::DepthBiasState { constant: 0, slope_scale: 1.5, clamp: 0.0 },
				}),
				multisample: Default::default(),
				multiview_mask: None,
				cache: None,
			})
		});
		let (pipeline_shrub, pipeline_tfrag, pipeline_tie) = (make_cut("fs_sh_shrub"), make_cut("fs_sh_tfrag"), make_cut("fs_sh_tie"));
		SunShadow { tex, layers, array, cmp, ubuf, ubind, pipeline, pipeline_props, pipeline_shrub, pipeline_tfrag, pipeline_tie, indirect: None, cap: 0, dir: None, scene_id: 0, last: None }
	}

	/// The level's group-0 entries: `ubuf` at binding 0, the cascades and their compare sampler at 1 and 2.
	pub(super) fn entries<'a>(&'a self, ubuf: &'a wgpu::Buffer) -> [wgpu::BindGroupEntry<'a>; 3] {
		[
			wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
			wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&self.array) },
			wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&self.cmp) },
		]
	}

	/// This frame's light and cascades, or None when nothing casts (no lit key light). `rows` are the level's light rows;
	/// `scene_id` tells a new level (the light snaps instead of easing).
	pub(super) fn frame(&mut self, cam: &Camera, aspect: f32, rows: &[[[f32; 4]; 4]; 16], scene_id: usize, q: Quality) -> Option<SunFrame> {
		let now = std::time::Instant::now();
		let dt = self.last.map_or(0.0, |t| now.duration_since(t).as_secs_f32()).min(0.1);
		self.last = Some(now);
		let (target, luma) = match key_light(rows, 0, 0, 0.0) {
			Some(k) => k,
			// No lit key light: the shadows fade out along the last direction (none at all in a new level).
			None => match self.dir.filter(|_| self.scene_id == scene_id) {
				Some((d, _)) => (d, 0.0),
				None => return None,
			},
		};
		let want = strength(target, luma);
		// A changed light row turns the light over about three quarters of a second (time based, whatever the frame rate) instead
		// of snapping every shadow at once; a new level starts at its light.
		let (key, st) = match self.dir.filter(|_| self.scene_id == scene_id) {
			Some((d, s)) => {
				let k = 1.0 - (-dt / TURN_TAU).exp();
				(turn(d, target, k), s + (want - s) * k)
			}
			None => (target, want),
		};
		self.dir = Some((key, st));
		self.scene_id = scene_id;
		let dir = raise(key);
		// While a big turn is under way the shadows soften (half strength at a right angle).
		let s = st * (0.5 + 0.5 * dot3(key, target).clamp(0.0, 1.0));
		if s <= 0.0 {
			return None;
		}
		let fwd = forward(cam);
		let mut near = RETAIL_NEAR;
		let cascades = q.splits.map(|far| {
			let c = Cascade::fit(cam.pos, fwd, cam.tan_half, aspect, near, far, dir, q.res);
			near = far;
			c
		});
		Some(SunFrame { dir, strength: s, cascades, q })
	}

	/// Renders the three cascades of `f` from the scene's casters.
	/// `skin` is the GPU-posed props batch's bind group and slot attribute (None: the CPU poses the props in world space).
	pub(super) fn encode(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, scene: &Scene, f: &SunFrame, skin: Option<(&wgpu::BindGroup, Option<&wgpu::Buffer>)>) {
		let _s = super::profile::span("sun_shadow");
		for (k, c) in f.cascades.iter().enumerate().take(f.q.cascades) {
			queue.write_buffer(&self.ubuf[k], 0, bytemuck::bytes_of(&c.vp));
		}
		// Draw lists: per cascade, per batch, the merged index ranges of the casters.
		let mut args: Vec<[u32; 5]> = Vec::new();
		let mut runs: Vec<(usize, Src, u64, u32)> = Vec::new();
		let push = |args: &mut Vec<[u32; 5]>, runs: &mut Vec<(usize, Src, u64, u32)>, k: usize, src: Src, draws: &mut dyn Iterator<Item = (u32, u32, u32)>| {
			let first = args.len();
			for (start, count, fi) in draws {
				// Adjacent ranges (texture runs of one mesh) merge when they cast alike: depth only, or the same slot for texel alpha.
				let n = args.len();
				match args.last_mut().filter(|a| n > first && a[2] + a[0] == start && a[4] == fi) {
					Some(a) => a[0] += count,
					None => args.push([count, 1, start, 0, fi]),
				}
			}
			if args.len() > first {
				runs.push((k, src, (first * 20) as u64, (args.len() - first) as u32));
			}
		};
		// Terrain and ties carry their texture slot when they cast by texel alpha (`shrub_shader`).
		let cut = self.pipeline_tfrag.is_some() && scene.tex_array.is_some();
		let fi = |d: &Draw| if cut { super::bindless::first_instance(d) } else { 0 };
		for (k, c) in f.cascades.iter().enumerate().take(f.q.cascades) {
			// tfrag_ranges[i]: 0 far, 1 mid, 2 near. A fragment casts with the LOD `Gpu::cull_tfrags` drew it with this frame (its
			// receiving surface: a coarser mesh lying above the drawn one shadowed it in dots and jagged bands that came and went as
			// the camera moved); an undrawn one (off screen) with the near mesh.
			let tlod = |i: usize| match scene.tfrag_lod.get(i).copied() {
				Some(l) if l < 3 => l as usize,
				_ => 2,
			};
			let tf = scene.tfrag_set.iter().enumerate().filter(|(_, t)| c.touches(t.cull.centre, t.cull.radius)).flat_map(|(i, _)| {
				let lod = tlod(i);
				scene.tfrag_draws[scene.tfrag_ranges[i][lod].start as usize..scene.tfrag_ranges[i][lod].end as usize].iter().map(|d| (d.start, d.count, fi(d)))
			});
			push(&mut args, &mut runs, k, Src::Tfrags, &mut tf.collect::<Vec<_>>().into_iter());
			// Ties likewise: the LOD `Gpu::cull_ties` drew, LOD 0 when undrawn.
			let ilod = |i: usize| match scene.tie_lod.get(i).copied() {
				Some(l) if l < 3 => l as usize,
				_ => 0,
			};
			let ties = scene.tie_cull.iter().enumerate().filter(|(_, t)| c.touches(t.centre, t.radius)).flat_map(|(i, _)| {
				let lod = ilod(i);
				scene.tie_draws[scene.tie_ranges[i][lod].start as usize..scene.tie_ranges[i][lod].end as usize].iter().map(|d| (d.start, d.count, fi(d)))
			});
			push(&mut args, &mut runs, k, Src::Ties, &mut ties.collect::<Vec<_>>().into_iter());
			push(&mut args, &mut runs, k, Src::World, &mut scene.world.draws.iter().map(|d| (d.start, d.count, 0)));
			if self.pipeline_shrub.is_some() && scene.tex_array.is_some() {
				let sh = scene.shrub_cull.iter().enumerate().filter(|(_, s)| c.touches(s.centre, s.radius)).flat_map(|(i, _)| scene.shrubs.draws[scene.shrub_ranges[i].0.clone()].iter().map(|d| (d.start, d.count, super::bindless::first_instance(d))));
				push(&mut args, &mut runs, k, Src::Shrubs, &mut sh.collect::<Vec<_>>().into_iter());
			}
			if scene.props_shown {
				// See-through shells (blast domes, glows) cast nothing (`clear_tex`).
				let solid = |d: &&Draw| !scene.clear_tex.get(d.texture).copied().unwrap_or(false);
				push(&mut args, &mut runs, k, Src::Props, &mut scene.props.draws.iter().filter(solid).map(|d| (d.start, d.count, 0)));
				push(&mut args, &mut runs, k, Src::Clip, &mut scene.clip_batch.draws.iter().filter(solid).map(|d| (d.start, d.count, 0)));
				if let Some(b) = &scene.debris {
					push(&mut args, &mut runs, k, Src::Debris, &mut b.draws.iter().filter(solid).map(|d| (d.start, d.count, 0)));
				}
			}
			if scene.marker_shown {
				push(&mut args, &mut runs, k, Src::Marker, &mut scene.marker.draws.iter().map(|d| (d.start, d.count, 0)));
			}
		}
		if args.len() > self.cap {
			self.cap = args.len().next_power_of_two().max(4096);
			self.indirect = Some(device.create_buffer(&wgpu::BufferDescriptor { label: Some("sun shadow draws"), size: (self.cap * 20) as u64, usage: wgpu::BufferUsages::INDIRECT | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
		}
		let Some(ind) = &self.indirect else { return };
		if !args.is_empty() {
			queue.write_buffer(ind, 0, bytemuck::cast_slice(&args));
		}
		for k in 0..f.q.cascades {
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("sun shadow"),
				color_attachments: &[],
				depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &self.layers[k], depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }), stencil_ops: None }),
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			if f.q.res < RES {
				// Low / Medium: the corner of the map (the receivers read it scaled, `write_uniforms`).
				pass.set_viewport(0.0, 0.0, f.q.res as f32, f.q.res as f32, 0.0, 1.0);
			}
			pass.set_bind_group(0, &self.ubind[k], &[]);
			for &(_, src, off, n) in runs.iter().filter(|r| r.0 == k) {
				let batch = match src {
					Src::World => &scene.world,
					Src::Tfrags => &scene.tfrags,
					Src::Ties => &scene.ties,
					Src::Props => &scene.props,
					Src::Marker => &scene.marker,
					Src::Clip => &scene.clip_batch,
					Src::Shrubs => &scene.shrubs,
					Src::Debris => match &scene.debris {
						Some(b) => b,
						None => continue,
					},
				};
				match (src, &self.pipeline_props, skin) {
					(Src::Props, Some(p), Some((bind, Some(sj)))) => {
						pass.set_pipeline(p);
						pass.set_bind_group(1, bind, &[]);
						pass.set_vertex_buffer(1, sj.slice(..));
					}
					// GPU-posed props without their slot attribute: model space, nothing to cast.
					(Src::Props, _, Some(_)) => continue,
					(Src::Shrubs, _, _) => match (&self.pipeline_shrub, &scene.tex_array) {
						(Some(p), Some(arr)) => {
							pass.set_pipeline(p);
							pass.set_bind_group(1, arr, &[]);
						}
						_ => continue,
					},
					(Src::Tfrags | Src::Ties, _, _) if cut => {
						let p = if matches!(src, Src::Tfrags) { &self.pipeline_tfrag } else { &self.pipeline_tie };
						if let (Some(p), Some(arr)) = (p, &scene.tex_array) {
							pass.set_pipeline(p);
							pass.set_bind_group(1, arr, &[]);
						}
					}
					_ => pass.set_pipeline(&self.pipeline),
				}
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				pass.multi_draw_indexed_indirect(ind, off, n);
			}
		}
		let _ = &self.tex;
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn cascade_covers_its_slice_and_snaps_to_texels() {
		let (pos, fwd, dir) = ([100.0, 50.0, 10.0], norm3([1.0, 0.3, -0.2]), norm3([0.3, -0.5, -0.8]));
		let c = Cascade::fit(pos, fwd, 0.6, 16.0 / 9.0, 0.03, 12.0, dir, RES);
		// Every corner of the slice projects inside the map, depth inside 0..1.
		let up_ref = [0.0, 0.0, 1.0];
		let s = norm3([fwd[1] * up_ref[2] - fwd[2] * up_ref[1], fwd[2] * up_ref[0] - fwd[0] * up_ref[2], fwd[0] * up_ref[1] - fwd[1] * up_ref[0]]);
		let u = [s[1] * fwd[2] - s[2] * fwd[1], s[2] * fwd[0] - s[0] * fwd[2], s[0] * fwd[1] - s[1] * fwd[0]];
		for (depth, sx, sy) in [(0.03f32, 1.0f32, 1.0f32), (12.0, 1.0, 1.0), (12.0, -1.0, 1.0), (12.0, 1.0, -1.0), (12.0, -1.0, -1.0), (0.03, -1.0, -1.0)] {
			let (hx, hy) = (depth * 0.6 * 16.0 / 9.0 * sx, depth * 0.6 * sy);
			let p = [0, 1, 2].map(|k| pos[k] + fwd[k] * depth + s[k] * hx + u[k] * hy);
			let m = c.vp;
			let q = [0, 1, 2].map(|r| m[0][r] * p[0] + m[1][r] * p[1] + m[2][r] * p[2] + m[3][r]);
			assert!(q[0].abs() <= 1.0 && q[1].abs() <= 1.0, "corner {p:?} -> {q:?}");
			assert!((0.0..=1.0).contains(&q[2]), "depth {q:?}");
			assert!(c.touches(p, 0.0));
		}
		// Turning the camera keeps the sphere's size; moving it a little moves the map in whole texels.
		let turned = Cascade::fit(pos, norm3([-0.2, 1.0, 0.1]), 0.6, 16.0 / 9.0, 0.03, 12.0, dir, RES);
		assert!((turned.r - c.r).abs() < 1e-4 && (turned.texel - c.texel).abs() < 1e-7);
		let moved = Cascade::fit([pos[0] + 0.0137, pos[1], pos[2]], fwd, 0.6, 16.0 / 9.0, 0.03, 12.0, dir, RES);
		for k in 0..2 {
			let d = (moved.c[k] - c.c[k]) / c.texel;
			assert!((d - d.round()).abs() < 1e-2, "moved by {d} texels");
		}
		// A caster far towards the light still counts, one behind the receivers does not.
		let towards = [0, 1, 2].map(|k| pos[k] + fwd[k] * 6.0 - dir[k] * 150.0);
		let behind = [0, 1, 2].map(|k| pos[k] + fwd[k] * 6.0 + dir[k] * 150.0);
		assert!(c.touches(towards, 1.0));
		assert!(!c.touches(behind, 1.0));
	}

	#[test]
	fn key_light_and_strength() {
		let mut rows = [[[0.0f32; 4]; 4]; 16];
		// Slot 2: bright A from above (to-light +z), dim B sideways; slot 3: a dim light.
		rows[2] = [[0.9, 0.9, 0.8, 0.0], [0.0, 0.0, 1.0, 0.0], [0.1, 0.1, 0.1, 0.0], [1.0, 0.0, 0.0, 0.0]];
		rows[3] = [[0.05, 0.05, 0.05, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0; 4], [0.0; 4]];
		let (d, l) = key_light(&rows, 2, 2, 0.0).unwrap();
		assert_eq!(d, [0.0, 0.0, -1.0], "travels downwards");
		assert!(l > 0.8);
		assert!((strength(d, l) - STRENGTH).abs() < 1e-6);
		let (d3, l3) = key_light(&rows, 3, 3, 0.0).unwrap();
		assert_eq!(strength(d3, l3), 0.0, "a dim sideways light casts nothing");
		assert!(key_light(&[[[0.0; 4]; 4]; 16], 0, 0, 0.0).is_none(), "no light, no shadows");
		assert_eq!(strength([0.0, 0.0, 1.0], 1.0), 0.0, "from below the horizon");
	}

	#[test]
	fn low_light_is_raised_and_turns_smoothly() {
		// Veldin's key light (20 degrees up) keeps its heading at 35 degrees; a high one is left alone.
		let low = norm3([-0.39713123, 0.8516507, -0.3420202]);
		let r = raise(low);
		assert!((r[2] + MIN_RISE).abs() < 1e-6 && (dot3(r, r) - 1.0).abs() < 1e-5);
		assert!((r[1] / r[0] - low[1] / low[0]).abs() < 1e-4, "heading kept");
		assert_eq!(raise([0.0, 0.6, -0.8]), [0.0, 0.6, -0.8]);
		assert_eq!(raise([0.0, 0.9, 0.1]), [0.0, 0.9, 0.1], "below the horizon: left for `strength`");
		// The hangar's light (+x) from the outdoor one: each step is a small turn, the end is the target, never a jump.
		let inside = norm3([0.8838404, 0.35856837, -0.30042458]);
		let mut d = low;
		let mut biggest = 0.0f32;
		for _ in 0..60 {
			let n = turn(d, inside, 1.0 - (-1.0f32 / 60.0 / TURN_TAU).exp());
			biggest = biggest.max(dot3(d, n).clamp(-1.0, 1.0).acos());
			d = n;
		}
		assert!(dot3(d, inside) > 0.999, "arrives within a second");
		assert!(biggest < 0.2, "largest step {biggest} rad");
		// Opposite directions turn through straight down.
		let m = turn([1.0, 0.0, 0.0], [-1.0, 0.0, 0.0], 0.5);
		assert!(m[2] < -0.9, "{m:?}");
	}
}
