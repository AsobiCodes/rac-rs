//! [PC] Water Low / Medium / High: the underwater look (no retail counterpart; Water Classic and Off draw the retail picture).
//!
//! Retail has no underwater geometry pass: while the camera water flag (0x167414, retail.rs `SwapWords::under`) is set it swaps
//! the fog words (near 0, far 32 m, a dark teal or blue) and draws one flat tint sprite over the frame (water.md NOV-28). The
//! sprite sits in the retail overlay, so it stays; this pass works on the picture below it (before bloom and the grade; the HUD
//! and the sprite are drawn after it, so they are never touched):
//! - Low: distance fog in the water's colour and per-channel absorption along the part of each view ray that lies in the water
//!   (the ray from the eye to the surface it sees, or to its first crossing of the water plane: a camera at the surface gets a
//!   natural waterline split, a camera above the surface gets nothing here: the water pass shades that). One depth load, one
//!   scene load per pixel.
//! - Medium: plus light that fades with the depth below the surface, and animated light caustics on every surface below it
//!   (one ridge layer, the world position from the depth, the sun's direction bent into the water).
//! - High: caustics of two layers with a warp, light shafts from the surface, and a gentle wobble of the whole picture.
//! The amount (`Under::k`) ramps on the wall clock when the flag flips, so nothing flashes (the retail tint sprite itself switches
//! in one field).

use super::*;

const SHADER: &str = r#"
struct U {
	eye: vec4<f32>,
	fwd: vec4<f32>,
	right: vec4<f32>,
	up: vec4<f32>,
	size: vec4<f32>,
	proj: vec4<f32>,
	wcol: vec4<f32>,
	light: vec4<f32>,
};
// eye.w = the surface height; light.w = 1 when the surface is known; proj = (near, far, amount, seconds); wcol.w = level (1..3).
@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var depth_ms: texture_depth_multisampled_2d;
@group(0) @binding(3) var lsamp: sampler;

// Fog per metre of water path (u.size.x), caustic gain (size.y), shaft gain (size.z), wobble (size.w): `TUNE`. Absorption per channel
// (red goes first), the light's fall per metre of depth:
const ABSORB: vec3<f32> = vec3<f32>(0.060, 0.020, 0.014);
const LIGHT_FALL: f32 = 0.045;
const PATH_MAX: f32 = 40.0;
const HUE_MIX: f32 = 0.7;
const HAZE: f32 = 0.30;
const HAZE_MIX: f32 = 0.35;
const CAUSTIC_SCALE: f32 = 0.85;
const CAUSTIC_FALL: f32 = 0.055;

struct VO { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> VO {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	var o: VO;
	o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
	o.uv = vec2<f32>(x, y);
	return o;
}

fn linear_depth(d: f32) -> f32 {
	let n = u.proj.x;
	let f = u.proj.y;
	return n * f / (f - d * (f - n));
}

fn hash2(i: vec2<i32>) -> f32 {
	var h = bitcast<u32>(i.x) * 374761393u + bitcast<u32>(i.y) * 668265263u;
	h = (h ^ (h >> 13u)) * 1274126177u;
	h = h ^ (h >> 16u);
	return f32(h & 0xffffffu) / 16777215.0;
}

fn vnoise(p: vec2<f32>) -> f32 {
	let i = vec2<i32>(floor(p));
	let f = fract(p);
	let s = f * f * (3.0 - 2.0 * f);
	let a = hash2(i);
	let b = hash2(i + vec2<i32>(1, 0));
	let c = hash2(i + vec2<i32>(0, 1));
	let d = hash2(i + vec2<i32>(1, 1));
	return mix(mix(a, b, s.x), mix(c, d, s.x), s.y);
}

// A thin bright network where the noise crosses its middle value.
fn ridge(p: vec2<f32>) -> f32 {
	return 1.0 - abs(2.0 * vnoise(p) - 1.0);
}

// The light the surface focuses onto a point whose surface-side position is `s` (world xy); `hi` = two layers.
fn caustic(s: vec2<f32>, t: f32, hi: bool) -> f32 {
	let q0 = s * CAUSTIC_SCALE;
	let w = vec2<f32>(vnoise(q0 * 0.7 + vec2<f32>(t * 0.17, 3.1)), vnoise(q0 * 0.7 + vec2<f32>(7.7, -t * 0.15)));
	let q = q0 + (w - vec2<f32>(0.5)) * 1.1;
	let r1 = ridge(q + vec2<f32>(t * 0.11, t * 0.07));
	if (!hi) { return pow(r1, 6.0) * 1.7; }
	let r2 = ridge(vec2<f32>(q.y * 1.37 - q.x * 0.31, q.x * 1.29 + q.y * 0.43) - vec2<f32>(t * 0.09, -t * 0.12));
	return pow(0.5 * (r1 + r2), 7.0) * 3.2;
}

fn ign(x: vec2<f32>) -> f32 {
	return fract(52.9829189 * fract(dot(x, vec2<f32>(0.06711056, 0.00583715))));
}

@fragment fn fs_under(v: VO) -> @location(0) vec4<f32> {
	let c0 = vec2<i32>(v.pos.xy);
	let level = u.wcol.w;
	let t = u.proj.w;
	let k = u.proj.z;
	let uv = v.uv;
	// The world position of this pixel (derivatives first: uniform control flow).
	let dd = textureLoad(depth_ms, c0, 0);
	let sky = dd >= 1.0;
	let z = select(linear_depth(dd), u.proj.y, sky);
	let dir = u.fwd.xyz + (uv.x * 2.0 - 1.0) * u.right.xyz + (1.0 - uv.y * 2.0) * u.up.xyz;
	let pw = u.eye.xyz + z * dir;
	let dx = dpdx(pw);
	let dy = dpdy(pw);
	let edge = smoothstep(0.015, 0.05, fwidth(z) / max(z, 0.2));
	// High: the whole picture wobbles a little (a few pixels at most).
	var off = vec2<f32>(0.0);
	if (level >= 3.0) {
		off = u.size.w * vec2<f32>(sin(uv.y * 31.0 + t * 1.7) + 0.6 * sin(uv.y * 11.0 - t * 1.1), sin(uv.x * 27.0 + t * 1.3) + 0.6 * sin(uv.x * 9.0 - t * 0.9)) * k;
	}
	var col = textureSampleLevel(scene, lsamp, clamp(uv + off, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb;
	let orig = textureLoad(scene, c0, 0).rgb;

	// The part of the ray inside the water.
	let dl = length(dir);
	let dist = z * dl;
	let rd = dir / dl;
	let known = u.light.w > 0.5;
	let surf = u.eye.w;
	var path = dist;
	if (known) {
		if (u.eye.z < surf) {
			if (rd.z > 0.001) { path = min(dist, (surf - u.eye.z) / rd.z); }
		} else {
			path = 0.0;
		}
	}
	path = min(path, PATH_MAX);
	if (path <= 0.0) { return vec4<f32>(orig, 1.0); }

	// How much light there is at the eye's depth.
	var lit = 0.7;
	if (known) { lit = exp(-max(surf - u.eye.z, 0.0) * LIGHT_FALL * u.up.w); }
	// Caustics on the surfaces below the water (Medium, High).
	if (level >= 2.0 && known && !sky && pw.z < surf) {
		let ds = surf - pw.z;
		var nf = normalize(cross(dx, dy));
		if (dot(nf, rd) > 0.0) { nf = -nf; }
		let lw = u.light.xyz;
		let s = pw.xy - lw.xy * (ds / max(-lw.z, 0.25));
		let facing = clamp(dot(nf, -lw) * 0.85 + 0.15, 0.0, 1.0);
		let c = caustic(s, t, level >= 3.0) * facing * (1.0 - edge) * exp(-ds * CAUSTIC_FALL);
		col += (col * 0.9 + vec3<f32>(0.02) + u.wcol.rgb * 0.15) * c * u.size.y * lit;
	}
	// Along the water path: the colours drift to the water's hue at the picture's own brightness (retail's fog is already in the
	// picture and is weak for its far end, so a veil of the dark fog colour would only make the view murkier), the red goes first,
	// and a light haze of the water's colour (HAZE) gives depth.
	let fogf = 1.0 - exp(-path * u.size.x);
	let wl = max(dot(u.wcol.rgb, vec3<f32>(0.2126, 0.7152, 0.0722)), 0.02);
	let hue = min(u.wcol.rgb / wl, vec3<f32>(4.0));
	let cl = dot(col, vec3<f32>(0.2126, 0.7152, 0.0722));
	col = mix(col, hue * cl, fogf * HUE_MIX);
	col = col * exp(-path * ABSORB * u.fwd.w);
	let veil = hue * HAZE * (0.35 + 0.65 * lit);
	col = mix(col, veil, fogf * HAZE_MIX);
	// High: faint light shafts from the surface (bands across the view, in the distance, stronger towards the surface).
	if (level >= 3.0 && known) {
		let sh = vnoise(vec2<f32>(dot(dir.xy, vec2<f32>(0.7, 0.7)) * 9.0 / max(abs(dir.z) + 0.6, 0.6) + t * 0.12, t * 0.05));
		col += hue * u.size.z * smoothstep(0.45, 0.9, sh) * lit * clamp(rd.z * 1.5 + 0.3, 0.0, 1.0) * min(fogf * 2.5, 1.0);
	}
	col += (ign(vec2<f32>(c0)) - 0.5) / 255.0;
	return vec4<f32>(mix(orig, clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), k), 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct UnderU {
	eye: [f32; 4],
	fwd: [f32; 4],
	right: [f32; 4],
	up: [f32; 4],
	size: [f32; 4],
	proj: [f32; 4],
	wcol: [f32; 4],
	light: [f32; 4],
}

/// What the pass needs of the frame (gpu.rs fills it from the camera, the retail swap words and the water draws).
pub(super) struct UnderFrame {
	pub eye: [f32; 3],
	pub fwd: [f32; 3],
	/// Right x horizontal tan, up x vertical tan (as post.rs `cam`).
	pub right: [f32; 3],
	pub up: [f32; 3],
	pub near_far: [f32; 2],
	/// The retail camera water flag.
	pub flag: bool,
	/// The water surface height above the camera, when known.
	pub surf: Option<f32>,
	/// The water's colour (the retail underwater fog colour, gamma 0..1).
	pub colour: [f32; 3],
	/// The key light's travel direction.
	pub light: Option<[f32; 3]>,
	/// Water level: 1 Low, 2 Medium, 3 High.
	pub level: u8,
}

pub(super) struct Under {
	layout: wgpu::BindGroupLayout,
	pipeline: wgpu::RenderPipeline,
	sampler: wgpu::Sampler,
	ubuf: wgpu::Buffer,
	copy: Option<(wgpu::Texture, (u32, u32))>,
	format: wgpu::TextureFormat,
	start: std::time::Instant,
	last: std::time::Instant,
	/// The amount, 0..1: ramps up and down on the wall clock.
	k: f32,
	/// The last surface height found while the flag was set (kept while the flag stays set).
	surf: Option<f32>,
}

/// Fog per metre, caustic gain, shaft gain, wobble (uv), absorption scale, light-fall scale: the look's tuning. [PC] Dev aid: RAC_UNDER_TUNE="d,c,s,w,absorb,lightfall" overrides it.
const TUNE: [f32; 6] = [0.03, 1.15, 0.16, 0.0016, 0.1, 1.0];

fn tune() -> [f32; 6] {
	static T: std::sync::OnceLock<[f32; 6]> = std::sync::OnceLock::new();
	*T.get_or_init(|| {
		let mut t = TUNE;
		if let Ok(v) = std::env::var("RAC_UNDER_TUNE") {
			for (i, x) in v.split(',').enumerate().take(6) {
				if let Ok(x) = x.trim().parse::<f32>() {
					t[i] = x;
				}
			}
		}
		t
	})
}

/// Seconds to fade the look in and out when the flag flips.
const FADE_IN: f32 = 0.45;
const FADE_OUT: f32 = 0.3;

impl Under {
	pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Under {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("underwater"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("underwater", SHADER.into())) });
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("underwater"),
			entries: &[
				wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
				wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2, multisampled: false }, count: None },
				wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: true }, count: None },
				wgpu::BindGroupLayoutEntry { binding: 3, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
			],
		});
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("underwater"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("underwater"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: None,
			multisample: wgpu::MultisampleState::default(),
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_under"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			multiview_mask: None,
			cache: None,
		});
		let sampler = device.create_sampler(&wgpu::SamplerDescriptor { mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() });
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("underwater"), size: std::mem::size_of::<UnderU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let now = std::time::Instant::now();
		Under { layout, pipeline, sampler, ubuf, copy: None, format, start: now, last: now, k: 0.0, surf: None }
	}

	/// The amount for this frame (call every frame the water pass is on): ramps towards the flag.
	fn ramp(&mut self, flag: bool) {
		let now = std::time::Instant::now();
		let dt = now.duration_since(self.last).as_secs_f32().min(0.1);
		self.last = now;
		self.k = if flag { (self.k + dt / FADE_IN).min(1.0) } else { (self.k - dt / FADE_OUT).max(0.0) };
		if !flag && self.k == 0.0 {
			self.surf = None;
		}
	}

	/// The look is on or still fading out (as of the last `run`).
	pub(super) fn active(&self) -> bool {
		self.k > 0.0
	}

	/// The pass: the picture in `scene` below the retail HUD, with `depth` (the level's MSAA depth, sample 0).
	pub(super) fn run(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, scene: &wgpu::TextureView, depth: &wgpu::TextureView, f: &UnderFrame) {
		self.ramp(f.flag);
		if self.k <= 0.0 {
			return;
		}
		if f.surf.is_some() {
			self.surf = f.surf;
		}
		let surf = f.surf.or(self.surf);
		let size = scene.texture().size();
		let dim = (size.width, size.height);
		if self.copy.as_ref().is_none_or(|c| c.1 != dim || c.0.format() != scene.texture().format()) {
			let tex = device.create_texture(&wgpu::TextureDescriptor {
				label: Some("underwater copy"),
				size: wgpu::Extent3d { width: dim.0, height: dim.1, depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: self.format,
				usage: wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING,
				view_formats: &[],
			});
			self.copy = Some((tex, dim));
		}
		let Some((copy, _)) = &self.copy else { return };
		let l = f.light.map_or([0.25, 0.15, -1.0], |d| d);
		// The sun bent into the water: towards straight down, normalised.
		let l = [l[0] * 0.45, l[1] * 0.45, l[2] * 0.45 - 0.55];
		let n = (l[0] * l[0] + l[1] * l[1] + l[2] * l[2]).sqrt().max(1e-4);
		let u = UnderU {
			eye: [f.eye[0], f.eye[1], f.eye[2], surf.unwrap_or(0.0)],
			fwd: [f.fwd[0], f.fwd[1], f.fwd[2], tune()[4]],
			right: [f.right[0], f.right[1], f.right[2], 0.0],
			up: [f.up[0], f.up[1], f.up[2], tune()[5]],
			size: [tune()[0], tune()[1], tune()[2], tune()[3]],
			proj: [f.near_far[0], f.near_far[1], self.k * self.k * (3.0 - 2.0 * self.k), self.start.elapsed().as_secs_f32()],
			wcol: [f.colour[0], f.colour[1], f.colour[2], f.level as f32],
			light: [l[0] / n, l[1] / n, l[2] / n, if surf.is_some() { 1.0 } else { 0.0 }],
		};
		queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));
		enc.copy_texture_to_texture(scene.texture().as_image_copy(), copy.as_image_copy(), copy.size());
		let copy_view = copy.create_view(&Default::default());
		let depth_only = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("underwater"),
			layout: &self.layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&copy_view) },
				wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&depth_only) },
				wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::Sampler(&self.sampler) },
			],
		});
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("underwater"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: scene, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.pipeline);
		pass.set_bind_group(0, &group, &[]);
		pass.draw(0..3, 0..1);
	}
}

/// The height of the nearest flat water plane above `eye` whose box holds it (the water draws of the frame).
pub(super) fn plane_above(draws: &[waterfx::WaterDraw], eye: [f32; 3]) -> Option<f32> {
	draws
		.iter()
		.filter_map(|d| d.plane)
		.filter(|b| b[0] > eye[2] && eye[0] >= b[1] && eye[0] <= b[3] && eye[1] >= b[2] && eye[1] <= b[4])
		.map(|b| b[0])
		.min_by(|a, b| a.total_cmp(b))
}

/// The frame's inputs (`UnderFrame`) from the camera and the retail words. `far`: the world far plane the depth was drawn with
/// (`Gpu::world_far`).
#[allow(clippy::too_many_arguments)]
pub(super) fn frame(cam: &Camera, aspect: f32, flag: bool, hero_surf: Option<f32>, draws: &[waterfx::WaterDraw], colour: [f32; 3], light: Option<[f32; 3]>, level: u8, far: f32) -> UnderFrame {
	let f = forward(cam);
	let r = normalise(cross(f, cam.up));
	let u = cross(r, f);
	let (tx, ty) = (cam.tan_half * aspect, cam.tan_half);
	// The hero's own water height first (exact: retail's water face under the hero), else the nearest plane above the eye.
	let surf = hero_surf.filter(|&s| s > cam.pos[2] - 0.3).or_else(|| plane_above(draws, cam.pos));
	UnderFrame { eye: cam.pos, fwd: f, right: r.map(|c| c * tx), up: u.map(|c| c * ty), near_far: [RETAIL_NEAR, far], flag, surf, colour, light, level }
}
