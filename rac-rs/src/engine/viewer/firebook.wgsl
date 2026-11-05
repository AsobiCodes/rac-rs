// [PC] Fire flipbooks (firebook.rs): the level's fires (FireKind::Ground quads) as upright camera-facing sprites that play a looping
// flame animation. Atlas: 16 x 16 frames of 128 x 256 texels, row-major, top row first; R = temperature (the ramp's input), G = opacity;
// 256 frames one solver step apart (a 4 s loop, played at 64 fps), the flame's foot at the bottom centre of its frame. Premultiplied over the scene, emissive in HDR (the bloom
// and the tonemap treat it like the simulated fire), solid where G is high, soft against the scene's depth.
// Group 0 the level shader's block (view_proj), group 1 this pass, group 2 the soft-particle block of partfx.rs (scene depth).
struct U { view_proj: mat4x4<f32>, sky_view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;

// eye (xyz), right: the view's horizontal right (xyz), t: (loop phase in frames, brightness, glow gain, opacity gain),
// k: (-, soft m, sink share of the height, over: the share of the opacity that occludes what is behind (the rest only adds light))
struct BU { eye: vec4<f32>, right: vec4<f32>, t: vec4<f32>, k: vec4<f32> };
// a: (foot xyz, height m), b: (width m, frame offset, mirror +-1, fade), c: (tint rgb, -)
struct S { a: vec4<f32>, b: vec4<f32>, c: vec4<f32> };
@group(1) @binding(0) var<uniform> bu: BU;
@group(1) @binding(1) var<storage, read> spr: array<S>;
@group(1) @binding(2) var atlas: texture_2d<f32>;
@group(1) @binding(3) var asamp: sampler;

struct SoftU { p: vec4<f32>, n: vec4<f32>, l: array<vec4<f32>, 16> };
@group(2) @binding(0) var<uniform> soft: SoftU;
@group(2) @binding(1) var soft_depth: texture_depth_multisampled_2d;

const COLS: f32 = 16.0;
const FRAMES: f32 = 256.0;
// The frame's bottom this share of its height below the foot (the flame's root sits in the burning surface, no cut line).
const SINK: f32 = 0.08;
// The frame's bottom this share fades out (the generator's flat bed slab: no bright line along the burning edge).
const FOOT_FADE: f32 = 0.2;
// The soft base ellipse: centre height (uv.y), half height, peak opacity, temperature.
const BASE_Y: f32 = 0.9;
const BASE_R: f32 = 0.16;
const BASE_A: f32 = 0.55;
const BASE_T: f32 = 0.7;

// The depth rebuilt from w (viewer.rs world_clip, as firesim.wgsl).
fn world_clip(m: mat4x4<f32>, p: vec3<f32>) -> vec4<f32> {
	var c = m * vec4<f32>(p, 1.0);
	let wr = vec4<f32>(m[0][3], m[1][3], m[2][3], m[3][3]);
	let zr = vec4<f32>(m[0][2], m[1][2], m[2][2], m[3][2]);
	let ww = dot(wr.xyz, wr.xyz);
	if (ww > 1.0e-6) {
		let a = dot(zr.xyz, wr.xyz) / ww;
		let d = zr.xyz - a * wr.xyz;
		if (dot(d, d) <= 1.0e-10 * ww) {
			c.z = a * c.w + (zr.w - a * wr.w);
		}
	}
	return c;
}

struct VOut {
	@builtin(position) pos: vec4<f32>,
	// the sprite's own uv (0..1 across, 0 at the top .. 1 at the foot), mirrored
	@location(0) uv: vec2<f32>,
	// frame a, frame b, blend, fade
	@location(1) @interpolate(flat) f: vec4<f32>,
	@location(2) @interpolate(flat) tint: vec3<f32>,
};

@vertex fn vs_book(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> VOut {
	let s = spr[ii];
	var cx = array<f32, 6>(-1.0, 1.0, 1.0, -1.0, 1.0, -1.0);
	var cy = array<f32, 6>(0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
	let x = cx[vi];
	let y = cy[vi];
	let h = s.a.w;
	// The frame's bottom `sink` x h below the foot, its top still at the foot + h: the flame grows out of the surface it burns on.
	let sink = bu.k.z;
	let p = s.a.xyz + bu.right.xyz * (0.5 * x * s.b.x) + vec3<f32>(0.0, 0.0, (y * (1.0 + sink) - sink) * h);
	var o: VOut;
	o.pos = world_clip(u.view_proj, p);
	var uu = 0.5 * x + 0.5;
	if (s.b.z < 0.0) {
		uu = 1.0 - uu;
	}
	o.uv = vec2<f32>(uu, 1.0 - y);
	let tf = bu.t.x + s.b.y;
	let fa = floor(tf);
	o.f = vec4<f32>(fa - FRAMES * floor(fa / FRAMES), fa + 1.0 - FRAMES * floor((fa + 1.0) / FRAMES), tf - fa, s.b.w);
	o.tint = s.c.rgb;
	return o;
}

// One frame's texel at the sprite's uv, kept half a texel of the coarsest level (cells of 8 x 16) inside its cell: no level bleeds the
// neighbouring cell in (the bed slab of the frame above would draw a line on the top of the frame below).
fn frame_at(f: f32, uv: vec2<f32>) -> vec2<f32> {
	let col = f - COLS * floor(f / COLS);
	let row = floor(f / COLS);
	let q = clamp(uv, vec2<f32>(0.0625, 0.032), vec2<f32>(0.9375, 0.968));
	return textureSample(atlas, asamp, (vec2<f32>(col, row) + q) / COLS).rg;
}

// Temperature -> emission: agent A's ramp (FIREBOOK_A.md, pp/fb_ramp.txt; linear RGB, piecewise linear in t on t x RAMP_T), its 0.5 and
// 0.65 stops warmed to orange (firebook C, on the board: A's stops read red in game): orange body, yellow-white only at the roots, deep
// red tips. An HDR lift on the hottest part feeds the bloom.
const RAMP_T: f32 = 1.0;
fn ramp(t0: f32) -> vec3<f32> {
	var k = array<f32, 8>(0.0, 0.15, 0.3, 0.5, 0.65, 0.8, 0.9, 1.0);
	var c = array<vec3<f32>, 8>(vec3<f32>(0.0, 0.0, 0.0), vec3<f32>(0.12, 0.01, 0.002), vec3<f32>(0.45, 0.035, 0.005), vec3<f32>(1.0, 0.24, 0.016), vec3<f32>(1.0, 0.42, 0.045), vec3<f32>(1.0, 0.62, 0.12), vec3<f32>(1.0, 0.82, 0.35), vec3<f32>(1.0, 0.96, 0.78));
	let t = clamp(t0 * RAMP_T, 0.0, 1.0);
	var o = c[7];
	for (var i = 0; i < 7; i++) {
		if (t <= k[i + 1]) {
			o = mix(c[i], c[i + 1], (t - k[i]) / (k[i + 1] - k[i]));
			break;
		}
	}
	return o * (1.0 + 1.0 * smoothstep(0.75, 1.0, t));
}

// The game's own fire colour bends the ramp where it is not a warm orange (as firesim.wgsl `tinted`): the core stays light.
fn tinted(c: vec3<f32>, tint: vec3<f32>, t: f32) -> vec3<f32> {
	let m = max(max(tint.r, tint.g), max(tint.b, 1.0e-3));
	let off = clamp((1.0 - dot(normalize(tint + vec3<f32>(1.0e-4)), normalize(vec3<f32>(1.0, 0.5, 0.15)))) * 6.0, 0.0, 1.0);
	let luma = dot(c, vec3<f32>(0.3, 0.55, 0.15));
	return mix(c, tint / m * luma * 1.35 + vec3<f32>(smoothstep(0.8, 1.0, t) * 0.35), off);
}

// The frame's outer columns / top rows (the clamp zone of `frame_at`: its texel is repeated there) draw nothing, and the flame fades in just
// inside them. Where a flame of the atlas touches the frame's side the clamped column was smeared into a flat hard-edged bar sticking out of
// the fire (a fade inside the clamp zone left half of it: nfire25).
fn edge_window(uv: vec2<f32>) -> f32 {
	return smoothstep(0.0625, 0.125, uv.x) * smoothstep(0.9375, 0.875, uv.x) * smoothstep(0.032, 0.064, uv.y);
}

fn soft_fade(pos: vec4<f32>) -> f32 {
	let n = soft.p.x;
	let f = soft.p.y;
	let dfull = vec2<i32>(textureDimensions(soft_depth)) - vec2<i32>(1);
	let zs = textureLoad(soft_depth, clamp(vec2<i32>(pos.xy), vec2<i32>(0), dfull), 0);
	let ls = n * f / (f - zs * (f - n));
	let lf = n * f / (f - pos.z * (f - n));
	return clamp((ls - lf) / max(bu.k.y, 1.0e-3), 0.0, 1.0);
}

struct FireOut { @location(0) c: vec4<f32>, @location(1) glow: vec4<f32> };

// The flame's light (rgb, already x a) and its opacity a at this pixel (a < 0.004 discards).
fn flame(i: VOut) -> vec4<f32> {
	let s = mix(frame_at(i.f.x, i.uv), frame_at(i.f.y, i.uv), i.f.z);
	// Soft against the scene only up the flame: its root meets the roof / dome with no fade (no gap between surface and fire).
	let sf = i.f.w * mix(1.0, soft_fade(i.pos), smoothstep(0.85, 0.5, i.uv.y));
	// A soft glowing base under every flame (neighbouring sprites' bases overlap into one band, no comb of roots): an ellipse at the foot.
	let bd = length(vec2<f32>((i.uv.x - 0.5) * 2.0, (i.uv.y - BASE_Y) / BASE_R));
	let base = (1.0 - smoothstep(0.0, 1.0, bd)) * smoothstep(1.0, 0.9, i.uv.y) * BASE_A;
	let fa = clamp(s.g * bu.t.w, 0.0, 1.0) * smoothstep(1.0, 1.0 - FOOT_FADE, i.uv.y) * edge_window(i.uv);
	let a = max(fa, base) * sf;
	if (a < 0.004) {
		discard;
	}
	let st = mix(s.r, max(s.r, BASE_T), clamp(base - fa, 0.0, 1.0));
	let e = tinted(ramp(st), i.tint, st) * bu.t.y;
	return vec4<f32>(e * a, a);
}

fn glow_of(f: vec4<f32>) -> vec4<f32> {
	return vec4<f32>(clamp(f.rgb * soft.p.w * bu.t.z * 0.6, vec3<f32>(0.0), vec3<f32>(1.0)), f.a);
}

// The classic draw: premultiplied over the scene, far to near (the order matters where sprites overlap).
@fragment fn fs_book(i: VOut) -> FireOut {
	let f = flame(i);
	var o: FireOut;
	o.c = vec4<f32>(f.rgb, f.a * bu.k.w);
	o.glow = glow_of(f);
	return o;
}

// The order independent pair (any draw order gives the same picture): every sprite first darkens what is behind it by its occluding share
// (dst x (1 - a x over)), then every sprite adds its light. With nothing overlapping it is the classic draw's picture.
@fragment fn fs_dark(i: VOut) -> @location(0) vec4<f32> {
	let f = flame(i);
	return vec4<f32>(0.0, 0.0, 0.0, f.a * bu.k.w);
}

@fragment fn fs_light(i: VOut) -> FireOut {
	let f = flame(i);
	var o: FireOut;
	o.c = vec4<f32>(f.rgb, 0.0);
	o.glow = glow_of(f);
	return o;
}
