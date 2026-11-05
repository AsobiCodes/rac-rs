// Modern UI (src/engine/ui): the backdrop passes (downsample, separable blur, composite with the scrim) and the instanced
// rounded rectangles / atlas quads of the draw list. One bind group layout for every pass: texture, sampler, Params.
// Colours are gamma-space values (the swapchain is the non-sRGB format, like the rest of the renderer) and are blended as such,
// the way a browser does. Dev: RAC_UI_WGSL=<file> loads this shader from disk (ENV.md).
struct Params {
	a: vec4<f32>,      // inst: window size in px (xy), text gamma (z); blur: texel step (xy); composite: dither strength (w)
	w0: vec4<f32>,     // blur weights for offsets 0..3
	w1: vec4<f32>,     // blur weights for offsets 4..7
	scrim: vec4<f32>,  // composite: scrim colour (rgb) and alpha
}
@group(0) @binding(0) var tex: texture_2d<f32>;
@group(0) @binding(1) var smp: sampler;
@group(0) @binding(2) var<uniform> p: Params;

// Instance kinds (ui/instances.rs K_RRECT, K_TEXT).
const K_RRECT: f32 = 0.0;
const K_TEXT: f32 = 1.0;
const K_IMAGE: f32 = 2.0;

// ---- backdrop -------------------------------------------------------------------------------------------------------------

struct FullOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) uv: vec2<f32>,
}

@vertex
fn vs_full(@builtin(vertex_index) i: u32) -> FullOut {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	var o: FullOut;
	o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
	o.uv = vec2<f32>(x, y);
	return o;
}

fn tap(uv: vec2<f32>) -> vec3<f32> {
	return textureSampleLevel(tex, smp, uv, 0.0).rgb;
}

// Halves the picture: the sample sits on the corner between four source texels, an exact 2x2 average.
@fragment
fn fs_down(v: FullOut) -> @location(0) vec4<f32> {
	return vec4<f32>(tap(v.uv), 1.0);
}

// One axis of the Gaussian: 15 taps, the weights (sum 1) come from the CPU (backdrop.rs `kernel`).
@fragment
fn fs_blur(v: FullOut) -> @location(0) vec4<f32> {
	let s = p.a.xy;
	var c = tap(v.uv) * p.w0.x;
	c += (tap(v.uv + s) + tap(v.uv - s)) * p.w0.y;
	c += (tap(v.uv + s * 2.0) + tap(v.uv - s * 2.0)) * p.w0.z;
	c += (tap(v.uv + s * 3.0) + tap(v.uv - s * 3.0)) * p.w0.w;
	c += (tap(v.uv + s * 4.0) + tap(v.uv - s * 4.0)) * p.w1.x;
	c += (tap(v.uv + s * 5.0) + tap(v.uv - s * 5.0)) * p.w1.y;
	c += (tap(v.uv + s * 6.0) + tap(v.uv - s * 6.0)) * p.w1.z;
	c += (tap(v.uv + s * 7.0) + tap(v.uv - s * 7.0)) * p.w1.w;
	return vec4<f32>(c, 1.0);
}

fn dither(q: vec2<f32>) -> f32 {
	return fract(sin(dot(q, vec2<f32>(12.9898, 78.233))) * 43758.5453) - 0.5;
}

// The blurred picture (bilinear up from the small level) under the scrim. A fixed noise of half a level hides banding.
@fragment
fn fs_comp(v: FullOut) -> @location(0) vec4<f32> {
	let c = mix(tap(v.uv), p.scrim.rgb, p.scrim.a);
	return vec4<f32>(c + vec3<f32>(dither(v.pos.xy) * p.a.w / 255.0), 1.0);
}

// ---- instances ------------------------------------------------------------------------------------------------------------

struct InstIn {
	@location(0) rect: vec4<f32>,
	@location(1) fill: vec4<f32>,
	@location(2) border: vec4<f32>,
	@location(3) q: vec4<f32>,     // corner radius, border width, kind, -
	@location(4) uv: vec4<f32>,    // text: atlas x, y of the bitmap
	@location(5) clip: vec4<f32>,  // x, y, w, h; w < 0 = none
	@location(6) clipq: vec4<f32>, // corner radius
}

struct InstOut {
	@builtin(position) pos: vec4<f32>,
	@location(0) @interpolate(flat) rect: vec4<f32>,
	@location(1) @interpolate(flat) fill: vec4<f32>,
	@location(2) @interpolate(flat) border: vec4<f32>,
	@location(3) @interpolate(flat) q: vec4<f32>,
	@location(4) @interpolate(flat) uv: vec4<f32>,
	@location(5) @interpolate(flat) clip: vec4<f32>,
	@location(6) @interpolate(flat) clipr: f32,
}

@vertex
fn vs_inst(@builtin(vertex_index) i: u32, d: InstIn) -> InstOut {
	// Two triangles: corners (0,0) (1,0) (0,1) (0,1) (1,0) (1,1).
	let corner = vec2<f32>(f32((0x32u >> i) & 1u), f32((0x2Cu >> i) & 1u));
	// Rounded rectangles get a pixel of margin for the edge anti-aliasing; atlas quads are exact.
	let pad = select(0.0, 1.0, d.q.z < 0.5);
	let px = d.rect.xy - vec2<f32>(pad) + corner * (d.rect.zw + vec2<f32>(2.0 * pad));
	var o: InstOut;
	o.pos = vec4<f32>(px.x / p.a.x * 2.0 - 1.0, 1.0 - px.y / p.a.y * 2.0, 0.0, 1.0);
	o.rect = d.rect;
	o.fill = d.fill;
	o.border = d.border;
	o.q = d.q;
	o.uv = d.uv;
	o.clip = d.clip;
	o.clipr = d.clipq.x;
	return o;
}

fn cov(d: f32) -> f32 {
	return clamp(0.5 - d, 0.0, 1.0);
}

// Signed distance (px) to a rounded rectangle of half size h around the origin.
fn sd_rrect(c: vec2<f32>, h: vec2<f32>, r: f32) -> f32 {
	let q = abs(c) - h + vec2<f32>(r);
	return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - r;
}

@fragment
fn fs_inst(v: InstOut) -> @location(0) vec4<f32> {
	var o = vec4<f32>(0.0);
	if (v.q.z < 0.5) {
		// Rounded rectangle: the fill covers the whole shape, the border is a ring inside it, drawn over the fill.
		let h = v.rect.zw * 0.5;
		let sd = sd_rrect(v.pos.xy - (v.rect.xy + h), h, min(v.q.x, min(h.x, h.y)));
		let outer = cov(sd);
		let inner = cov(sd + v.q.y);
		let af = v.fill.a * outer;
		let ab = v.border.a * max(outer - inner, 0.0);
		o = vec4<f32>(v.border.rgb * ab + v.fill.rgb * af * (1.0 - ab), ab + af * (1.0 - ab));
	} else if (v.q.z > 1.5) {
		// Picture of the image atlas (RGBA, straight alpha): stretched over the quad, rounded corners, tinted by the fill colour.
		let h = v.rect.zw * 0.5;
		let t = (v.pos.xy - v.rect.xy) / v.rect.zw;
		let c = textureSampleLevel(tex, smp, mix(v.uv.xy, v.uv.zw, t), 0.0);
		let a = c.a * v.fill.a * cov(sd_rrect(v.pos.xy - (v.rect.xy + h), h, min(v.q.x, min(h.x, h.y))));
		o = vec4<f32>(c.rgb * v.fill.rgb * a, a);
	} else {
		// Atlas quad: coverage tinted by the fill colour; the quad is exactly the bitmap, one texel per pixel.
		let lp = vec2<i32>(floor(v.pos.xy - v.rect.xy));
		var a = textureLoad(tex, vec2<i32>(i32(v.uv.x), i32(v.uv.y)) + lp, 0).r;
		if (a > 0.0) {
			a = pow(a, p.a.z);
		}
		o = vec4<f32>(v.fill.rgb * (v.fill.a * a), v.fill.a * a);
	}
	if (v.clip.z >= 0.0) {
		let ch = v.clip.zw * 0.5;
		o = o * cov(sd_rrect(v.pos.xy - (v.clip.xy + ch), ch, v.clipr));
	}
	return o;
}
