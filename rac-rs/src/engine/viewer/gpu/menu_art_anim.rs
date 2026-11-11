//! Animated menu art (lane gmap, 2026-11-06): a modern menu page's painting (`MENU_ART_TEX`, `Gpu::set_menu_art`) can be drawn by this
//! small pipeline instead of the plain HUD one (`MENU_ART_ANIM_TEX`, default off, no game named here). The pack hands over the
//! painting's facts as an [`ArtAnim`] each display frame:
//! - a disc (centre, two axes in texels, tilt) that turns RIGIDLY in its own plane: the fragment maps its place on the tilted
//!   ellipse to a circle, turns it by `angle` and maps back, then mixes that with the untouched painting through a feathered ring, so
//!   the picture's edges and the far starfield never move. The pack wraps `angle`, so there is no winding and no jump;
//! - the core glow breathes (`core_gain`, a brightness multiplier inside `core_radius` of the disc);
//! - stars twinkle: the painting's ALPHA channel is a star map (255 = not a star, 1..=254 = a star's code; the pipeline is opaque, so
//!   alpha is free), each code gives its own phase and rate, weighted by the texel's brightness so a star's halo falls off.
//! The disc taps are bicubic (Catmull-Rom, 9 bilinear fetches) so the slow turn does not breathe in sharpness. Cost: one quad, ~30
//! texture fetches per pixel of the painting.

use super::*;

/// The period (s) the caller wraps `ArtAnim::time` at; the shader's twinkle rates are whole cycles per this period (`WRAP` there).
pub const TWINKLE_WRAP_S: f64 = 600.0;

/// The animation of one frame (all angles in radians, `time` in seconds, wrapped by the caller).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ArtAnim {
	/// Disc centre as a fraction of the picture.
	pub centre: [f32; 2],
	/// Disc semi-axes in texels (the ellipse where the feather ends).
	pub axes: [f32; 2],
	/// Tilt of the ellipse's first axis from the picture's x axis.
	pub tilt: f32,
	/// How far the disc has turned (any value; the shader only uses its sine and cosine).
	pub angle: f32,
	/// Width of the feather ring, as a fraction of the disc radius (the turned picture shows fully inside `1 - feather`).
	pub feather: f32,
	/// Brightness multiplier of the core glow (1 = as painted).
	pub core_gain: f32,
	/// Radius of the core glow in disc units (1 = the ellipse).
	pub core_radius: f32,
	/// Display time in seconds (twinkle phase).
	pub time: f32,
	/// Strength of the twinkle (0 = off).
	pub twinkle: f32,
}

/// Rotation of the disc: the angle for a time `t` (s) at one full turn per `period` (s), wrapped to 0..2pi so it never grows.
pub fn turn_angle(t: f64, period: f64) -> f32 {
	if period <= 0.0 {
		return 0.0;
	}
	(t / period).fract().rem_euclid(1.0) as f32 * std::f32::consts::TAU
}

/// The feather weight of the turned picture at disc radius `r` (1 inside `1 - feather`, 0 at `1`, smooth between).
pub fn feather_weight(r: f32, feather: f32) -> f32 {
	let f = feather.clamp(0.01, 1.0);
	let x = ((1.0 - r) / f).clamp(0.0, 1.0);
	x * x * (3.0 - 2.0 * x)
}

/// The shader. WGSL, validated by the test below.
pub(super) const SHADER: &str = "
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var samp: sampler;
const WRAP: f32 = 600.0;
struct P { a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32> };
@group(1) @binding(0) var<uniform> u: P;
struct V { @builtin(position) p: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>) -> V {
	var o: V;
	o.p = vec4<f32>(p.xy, 0.0, 1.0);
	o.uv = uv;
	return o;
}
fn cr(uv: vec2<f32>, sz: vec2<f32>) -> vec3<f32> {
	let pos = uv * sz;
	let c = floor(pos - vec2<f32>(0.5)) + vec2<f32>(0.5);
	let f = pos - c;
	let w0 = f * (vec2<f32>(-0.5) + f * (vec2<f32>(1.0) - 0.5 * f));
	let w1 = vec2<f32>(1.0) + f * f * (vec2<f32>(-2.5) + 1.5 * f);
	let w2 = f * (vec2<f32>(0.5) + f * (vec2<f32>(2.0) - 1.5 * f));
	let w3 = f * f * (vec2<f32>(-0.5) + 0.5 * f);
	let w12 = w1 + w2;
	let t0 = (c - vec2<f32>(1.0)) / sz;
	let t3 = (c + vec2<f32>(2.0)) / sz;
	let t12 = (c + w2 / w12) / sz;
	var s = vec3<f32>(0.0);
	s += textureSampleLevel(tex, samp, vec2<f32>(t0.x, t0.y), 0.0).rgb * (w0.x * w0.y);
	s += textureSampleLevel(tex, samp, vec2<f32>(t12.x, t0.y), 0.0).rgb * (w12.x * w0.y);
	s += textureSampleLevel(tex, samp, vec2<f32>(t3.x, t0.y), 0.0).rgb * (w3.x * w0.y);
	s += textureSampleLevel(tex, samp, vec2<f32>(t0.x, t12.y), 0.0).rgb * (w0.x * w12.y);
	s += textureSampleLevel(tex, samp, vec2<f32>(t12.x, t12.y), 0.0).rgb * (w12.x * w12.y);
	s += textureSampleLevel(tex, samp, vec2<f32>(t3.x, t12.y), 0.0).rgb * (w3.x * w12.y);
	s += textureSampleLevel(tex, samp, vec2<f32>(t0.x, t3.y), 0.0).rgb * (w0.x * w3.y);
	s += textureSampleLevel(tex, samp, vec2<f32>(t12.x, t3.y), 0.0).rgb * (w12.x * w3.y);
	s += textureSampleLevel(tex, samp, vec2<f32>(t3.x, t3.y), 0.0).rgb * (w3.x * w3.y);
	return max(s, vec3<f32>(0.0));
}
// The picture at `uv` with the star twinkle applied.
fn shade(uv: vec2<f32>, sz: vec2<f32>) -> vec3<f32> {
	var c = cr(uv, sz);
	let ti = vec2<i32>(clamp(uv * sz, vec2<f32>(0.0), sz - vec2<f32>(1.0)));
	let code = round(textureLoad(tex, ti, 0).a * 255.0);
	if (code > 0.5 && code < 254.5 && u.c.y > 0.0) {
		let ph = fract(code * 0.618034) * 6.2831853;
		// Both rates are whole cycles per WRAP seconds (the caller wraps `time` at WRAP): no jump when the clock wraps.
		let rate = round((0.9 + fract(code * 0.7548776) * 1.6) * WRAP / 6.2831853) * 6.2831853 / WRAP;
		let rate2 = round(rate * 2.3 * WRAP / 6.2831853) * 6.2831853 / WRAP;
		let w = smoothstep(0.30, 0.65, max(c.r, max(c.g, c.b)));
		let k = 0.6 * sin(u.c.x * rate + ph) + 0.4 * sin(u.c.x * rate2 + ph * 1.7);
		c = c * (1.0 + u.c.y * w * k);
	}
	return c;
}
@fragment fn fs(v: V) -> @location(0) vec4<f32> {
	let sz = vec2<f32>(textureDimensions(tex));
	let q = (v.uv - u.a.xy) * sz;
	let ct = cos(u.b.x);
	let st = sin(u.b.x);
	// Into the disc plane: undo the tilt, squash the ellipse to a circle.
	let e = vec2<f32>(ct * q.x + st * q.y, -st * q.x + ct * q.y);
	let d = e / u.a.zw;
	let r = length(d);
	let ca = cos(u.b.y);
	let sa = sin(u.b.y);
	let d2 = vec2<f32>(ca * d.x + sa * d.y, -sa * d.x + ca * d.y);
	let e2 = d2 * u.a.zw;
	let q2 = vec2<f32>(ct * e2.x - st * e2.y, st * e2.x + ct * e2.y);
	let uv2 = u.a.xy + q2 / sz;
	let f = clamp((1.0 - r) / max(u.b.z, 0.01), 0.0, 1.0);
	let m = f * f * (3.0 - 2.0 * f);
	var c = shade(v.uv, sz);
	if (m > 0.0) {
		c = mix(c, shade(uv2, sz), m);
	}
	// The core breathes (disc units; a soft Gaussian).
	let g = exp(-(r * r) / (u.c.z * u.c.z));
	c = c * (1.0 + (u.b.w - 1.0) * g);
	return vec4<f32>(c, 1.0);
}
";

/// The pipeline and its uniform, made at startup (`Gpu::warm_menu_art_anim`) or at the first use.
pub(in crate::viewer) struct ArtPipe {
	pipeline: wgpu::RenderPipeline,
	ubuf: wgpu::Buffer,
	bind: wgpu::BindGroup,
}

/// The animation state of the menu art: this frame's parameters (None = the plain painting) and the pipeline.
#[derive(Default)]
pub(in crate::viewer) struct MenuArtAnim {
	pub(in crate::viewer) anim: Option<ArtAnim>,
	pipe: Option<ArtPipe>,
}

impl ArtPipe {
	fn new(device: &wgpu::Device, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat, tex_layout: &wgpu::BindGroupLayout) -> ArtPipe {
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("menu art anim"),
			entries: &[wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None }],
		});
		let module = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("menu art anim"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("menu art anim"), bind_group_layouts: &[Some(tex_layout), Some(&layout)], immediate_size: 0 });
		let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("menu art anim"),
			layout: Some(&pl),
			vertex: wgpu::VertexState {
				module: &module,
				entry_point: Some("vs"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2] })],
			},
			fragment: Some(wgpu::FragmentState { module: &module, entry_point: Some("fs"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: wgpu::PrimitiveState::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: depth_format, depth_write_enabled: Some(false), depth_compare: Some(wgpu::CompareFunction::Always), stencil: Default::default(), bias: Default::default() }),
			multisample: wgpu::MultisampleState { count: msaa(), ..Default::default() },
			multiview_mask: None,
			cache: None,
		});
		let ubuf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("menu art anim"), size: 64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let bind = device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("menu art anim"), layout: &layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() }] });
		ArtPipe { pipeline, ubuf, bind }
	}
}

impl Gpu {
	/// Builds the animation pipeline through the build guard (`pipe_guard::Effect::MenuArt`) unless it exists. A driver refusal
	/// switches the effect off for the run and `draw_menu_art_anim` then returns false: the painting is drawn plain. Called with the
	/// other UI pipelines at startup (setup.rs), and again (cheaply) by `set_menu_art_anim`.
	pub fn warm_menu_art_anim(&mut self) {
		if self.menu_art_anim.pipe.is_none() {
			let (device, format, depth, layout) = (&self.device, self.config.format, self.depth_format, &self.tex_layout);
			self.menu_art_anim.pipe = crate::viewer::pipe_guard::guard(device, crate::viewer::pipe_guard::Effect::MenuArt, || ArtPipe::new(device, format, depth, layout));
		}
	}

	/// This frame's animation of the menu art, or None for the plain painting. Call every display frame while the art is up; the
	/// pipeline is normally made at startup (a refused build is never retried).
	pub fn set_menu_art_anim(&mut self, anim: Option<ArtAnim>) {
		if anim.is_some() {
			self.warm_menu_art_anim();
		}
		self.menu_art_anim.anim = anim;
	}

	/// The animated painting's draw (pass_hud, `MENU_ART_ANIM_TEX`); false when there is nothing to draw it with. Leaves bind group
	/// 0 as the HUD draws had it.
	pub(super) fn draw_menu_art_anim(&self, pass: &mut wgpu::RenderPass<'_>, art: &wgpu::BindGroup, range: std::ops::Range<u32>) -> bool {
		let (Some(a), Some(p)) = (self.menu_art_anim.anim.as_ref(), self.menu_art_anim.pipe.as_ref()) else { return false };
		let u: [f32; 16] = [
			a.centre[0], a.centre[1], a.axes[0], a.axes[1],
			a.tilt, a.angle, a.feather, a.core_gain,
			a.time, a.twinkle, a.core_radius, 0.0,
			0.0, 0.0, 0.0, 0.0,
		];
		self.queue.write_buffer(&p.ubuf, 0, bytemuck::cast_slice(&u));
		pass.set_pipeline(&p.pipeline);
		pass.set_bind_group(0, art, &[]);
		pass.set_bind_group(1, &p.bind, &[]);
		pass.draw_indexed(range, 0, 0..1);
		pass.set_bind_group(0, &self.ubind, &[]);
		true
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The shader compiles and validates (naga).
	#[test]
	fn shader_validates() {
		let m = naga::front::wgsl::parse_str(SHADER).unwrap_or_else(|e| panic!("menu art anim: {}", e.emit_to_string(SHADER)));
		let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
		v.validate(&m).unwrap_or_else(|e| panic!("menu art anim validates: {e:?}"));
	}

	/// DX12 parity: the shader also goes through naga's HLSL writer (tb5 review N7).
	#[cfg(windows)]
	#[test]
	fn shader_writes_hlsl() {
		let h = crate::viewer::metalpipe::hlsl_set(SHADER, &[]);
		assert!(h.contains("fs"), "HLSL has the fragment entry");
	}

	/// The shader's const WRAP is the period the pack wraps the time at (tb5 review N5).
	#[test]
	fn the_shader_wrap_is_the_packs_wrap() {
		assert!(SHADER.contains(&format!("const WRAP: f32 = {:.1};", TWINKLE_WRAP_S)));
	}

	/// Mirror of the shader's twinkle: the factor at the wrap's two sides agrees for every star code (no jump every 600 s).
	#[test]
	fn the_twinkle_does_not_jump_when_the_clock_wraps() {
		let tau = std::f64::consts::TAU;
		let p = TWINKLE_WRAP_S;
		let k = |code: f64, t: f64| {
			let ph = (code * 0.618034).fract() * tau;
			let rate = ((0.9 + (code * 0.7548776).fract() * 1.6) * p / tau).round() * tau / p;
			let rate2 = (rate * 2.3 * p / tau).round() * tau / p;
			0.6 * (t * rate + ph).sin() + 0.4 * (t * rate2 + ph * 1.7).sin()
		};
		for code in 1..=254 {
			let (a, b) = (k(code as f64, p), k(code as f64, 0.0));
			assert!((a - b).abs() < 1e-6, "code {code}: {a} vs {b}");
		}
	}

	#[test]
	fn the_angle_wraps_without_a_jump() {
		let p = 480.0;
		assert_eq!(turn_angle(0.0, p), 0.0);
		assert!((turn_angle(p * 0.25, p) - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
		// Just before and just after a full turn the sine and cosine agree (no jump in the picture).
		let (a, b) = (turn_angle(p - 1e-3, p), turn_angle(p + 1e-3, p));
		assert!((a.sin() - b.sin()).abs() < 1e-4 && (a.cos() - b.cos()).abs() < 1e-4);
		// A long session stays inside one turn.
		assert!(turn_angle(1.0e7, p) < std::f32::consts::TAU);
		assert_eq!(turn_angle(5.0, 0.0), 0.0);
	}

	#[test]
	fn the_feather_is_one_inside_zero_outside_and_smooth() {
		assert_eq!(feather_weight(0.0, 0.4), 1.0);
		assert_eq!(feather_weight(0.6, 0.4), 1.0);
		assert_eq!(feather_weight(1.0, 0.4), 0.0);
		assert_eq!(feather_weight(1.7, 0.4), 0.0);
		let mut last = 1.0;
		for i in 0..=100 {
			let w = feather_weight(0.6 + 0.4 * i as f32 / 100.0, 0.4);
			assert!(w <= last + 1e-6, "monotone");
			last = w;
		}
		assert!((feather_weight(0.8, 0.4) - 0.5).abs() < 1e-6);
	}
}
