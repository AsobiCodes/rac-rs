//! [PC] Enhanced image (Pause -> Options -> Graphics -> Lighting Enhanced, retail/pcext.rs `graphics`; no retail
//! counterpart). With Lighting Classic nothing here runs and the frame is drawn exactly as before.
//!
//! The level pass resolves its MSAA colour into `scene` instead of the window; then, before any HUD or menu is drawn:
//! - `fs_lin`: the closest of each 2x2 block of the depth target (sample 0) as linear view depth, half size;
//! - `fs_ao` (Ambient occlusion On): screen-space ambient occlusion from that depth (normals from the depth, 12 samples on a
//!   rotated spiral within 1 world unit, distance falloff), then a depth-aware blur across and down;
//! - Water Enhanced (waterfx.rs): the water pixels shaded over the scene;
//! - Global illumination (Low / Medium / High): one bounce of light between nearby surfaces, in screen space on a quarter-size
//!   (High: half-size) grid: `fs_gsrc` keeps each cell's colour (linear) and surface normal (from the half-size depth), `fs_gi`
//!   gathers a few cells on a rotated spiral within a few world units, each weighted by how the two surfaces face each other
//!   and by distance (no visibility test, no ray tracing), then a depth-aware blur across and down; `fs_comp` lights the
//!   surface's own colour with it (depth-aware upsample). The water pass's pixels (water, waterfalls, the underwater look)
//!   take none: their look stays the water pass's;
//! - bloom: the bright part of the picture (a soft threshold on luminance; sky pixels count a quarter, so skies do not wash
//!   out) plus, with Particles Enhanced, the additive particles' glow target (partfx.rs) down a chain of five half sizes and back up with a tent filter;
//! - Sun rays On: `fs_rmask` keeps the bright sky around the key light's screen position (half size), `fs_rays` blurs it
//!   radially towards that point (48 taps, decaying), so light shafts stream past whatever stands in front of the sky;
//! - `fs_comp`, its own pass into every sample of the MSAA target: contrast-adaptive sharpening of the scene, the occlusion
//!   (depth-aware upsample), the bloom, then a filmic grade in linear light (gentle contrast around mid grey, a soft shoulder
//!   above 0.75 so no white clips hard, vibrance: dull colours up to x1.15) back to the gamma-space target the GS look is drawn in.
//! The HUD pass then loads that target and draws the HUD, the retail overlay (menus, text, fades and screen filters), the
//! item previews and the movies over it, untouched. Cost at 2560x1440 (RTX 5070 Ti): see `HANDOFF.md`.

use super::*;

const POST_SHADER: &str = r#"
struct P { size: vec4<f32>, half: vec4<f32>, proj: vec4<f32>, params: vec4<f32>, grade: vec4<f32>, extra: vec4<f32>, sun: vec4<f32>, cam: array<vec4<f32>, 4>, fxl: array<vec4<f32>, 16>, q: vec4<f32>, gi: vec4<f32>, gi2: vec4<f32>, gi3: vec4<f32> };
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var depth_ms: texture_depth_multisampled_2d;
@group(0) @binding(3) var lin: texture_2d<f32>;
@group(0) @binding(4) var ao_in: texture_2d<f32>;
@group(0) @binding(5) var bloom_in: texture_2d<f32>;
@group(0) @binding(6) var lsamp: sampler;
@group(0) @binding(7) var glow_in: texture_2d<f32>;
@group(0) @binding(8) var rays_in: texture_2d<f32>;
// [PC] Global illumination: the gathered bounce light (blur input / the composite's), the water pass's position target and the
// scene as it was before the water pass (both read by the composite only while the water pass ran this frame: p.gi.w = 1).
@group(0) @binding(9) var gi_in: texture_2d<f32>;
@group(0) @binding(10) var wpos_in: texture_2d<f32>;
@group(0) @binding(11) var prew_in: texture_2d<f32>;

struct VO { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> VO {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	var o: VO;
	o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
	o.uv = vec2<f32>(x, y);
	return o;
}

// p.proj = (tan_half * aspect, tan_half, near, far).
fn linear_depth(d: f32) -> f32 {
	let n = p.proj.z;
	let f = p.proj.w;
	return n * f / (f - d * (f - n));
}

fn sky(z: f32) -> bool {
	return z >= p.proj.w * 0.98;
}

@fragment fn fs_lin(v: VO) -> @location(0) vec4<f32> {
	let full = vec2<i32>(textureDimensions(depth_ms));
	let c = vec2<i32>(v.pos.xy) * 2;
	var d = 1.0;
	for (var k = 0; k < 4; k++) {
		let q = min(c + vec2<i32>(k & 1, k >> 1), full - vec2<i32>(1));
		d = min(d, textureLoad(depth_ms, q, 0));
	}
	if (d >= 1.0) { return vec4<f32>(p.proj.w, 0.0, 0.0, 0.0); }
	return vec4<f32>(linear_depth(d), 0.0, 0.0, 0.0);
}

// View-space position of half-size pixel `c` (camera looks down -z).
fn view_pos(c: vec2<i32>) -> vec3<f32> {
	let hs = vec2<i32>(textureDimensions(lin));
	let q = clamp(c, vec2<i32>(0), hs - vec2<i32>(1));
	let z = textureLoad(lin, q, 0).r;
	let uv = (vec2<f32>(q) + 0.5) / vec2<f32>(hs);
	let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
	return vec3<f32>(ndc.x * p.proj.x * z, ndc.y * p.proj.y * z, -z);
}

fn ign(x: vec2<f32>) -> f32 {
	return fract(52.9829189 * fract(dot(x, vec2<f32>(0.06711056, 0.00583715))));
}

// p.params.x = occlusion strength, p.grade.w = world radius.
@fragment fn fs_ao(v: VO) -> @location(0) vec4<f32> {
	let c = vec2<i32>(v.pos.xy);
	let P = view_pos(c);
	if (sky(-P.z)) { return vec4<f32>(1.0); }
	// Normal from the depth: per axis the neighbour with the smaller step (no smearing across silhouettes).
	let r = view_pos(c + vec2<i32>(1, 0)) - P;
	let l = P - view_pos(c - vec2<i32>(1, 0));
	let d = view_pos(c + vec2<i32>(0, 1)) - P;
	let u = P - view_pos(c - vec2<i32>(0, 1));
	let dx = select(l, r, abs(r.z) < abs(l.z));
	let dy = select(u, d, abs(d.z) < abs(u.z));
	var n = normalize(cross(dy, dx));
	if (dot(n, P) > 0.0) { n = -n; }
	let R = p.grade.w;
	// Pixels per world unit at this depth, on the half-size target.
	let ppu = p.half.y * 0.5 / (p.proj.y * -P.z);
	// Far away the radius shrinks under a few pixels: the occlusion fades out instead of turning into noise.
	let fade = smoothstep(2.0, 6.0, R * ppu);
	if (fade <= 0.0) { return vec4<f32>(1.0); }
	let rad = clamp(R * ppu, 3.0, p.q.y);
	let rot = ign(v.pos.xy) * 6.2831853;
	var occ = 0.0;
	let N = i32(p.q.x);
	for (var i = 0; i < N; i++) {
		let a = rot + f32(i) * 2.3999632;
		let t = sqrt((f32(i) + 0.5) / f32(N)) * rad;
		let o = vec2<i32>(round(vec2<f32>(cos(a), sin(a)) * t));
		let S = view_pos(c + o);
		let w = S - P;
		let d2 = dot(w, w);
		let nd = dot(n, w) * inverseSqrt(max(d2, 1e-6));
		let fall = clamp(1.0 - d2 / (R * R), 0.0, 1.0);
		occ += max(nd - 0.1, 0.0) * fall;
	}
	let a = clamp(1.0 - fade * p.params.x * occ / f32(N) * 2.0, 0.3, 1.0);
	return vec4<f32>(a, 0.0, 0.0, 0.0);
}

// Depth-aware 7-tap blur of the occlusion; p.grade.z picks the axis (0 across, 1 down).
@fragment fn fs_blur(v: VO) -> @location(0) vec4<f32> {
	let c = vec2<i32>(v.pos.xy);
	let hs = vec2<i32>(textureDimensions(lin));
	let axis = select(vec2<i32>(1, 0), vec2<i32>(0, 1), p.grade.z > 0.5);
	let z0 = textureLoad(lin, c, 0).r;
	var sum = 0.0;
	var wsum = 0.0;
	for (var k = -3; k <= 3; k++) {
		let q = clamp(c + axis * k, vec2<i32>(0), hs - vec2<i32>(1));
		let z = textureLoad(lin, q, 0).r;
		let w = exp(-f32(k * k) / 8.0) * max(0.0, 1.0 - abs(z - z0) / (0.05 * z0 + 0.05));
		sum += textureLoad(ao_in, q, 0).r * w;
		wsum += w;
	}
	return vec4<f32>(sum / max(wsum, 1e-4), 0.0, 0.0, 0.0);
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
	return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

// [PC] Global illumination. p.gi = (strength in the composite, world radius, taps, water mask valid); p.gi2 = (albedo: 0 the
// lit colour .. 1 its hue, floor, largest radius in grid cells, half-size texels per grid cell); p.gi3 = (blur depth
// tolerance, cap of the gathered light, 0, 0). A grid cell stands for half-size texel `q * s` (its depth, position and normal).
fn gi_normal(h: vec2<i32>) -> vec3<f32> {
	// As `fs_ao`: per axis the neighbour with the smaller step (no smearing across silhouettes), facing the eye.
	let P = view_pos(h);
	let r = view_pos(h + vec2<i32>(1, 0)) - P;
	let l = P - view_pos(h - vec2<i32>(1, 0));
	let d = view_pos(h + vec2<i32>(0, 1)) - P;
	let u = P - view_pos(h - vec2<i32>(0, 1));
	let dx = select(l, r, abs(r.z) < abs(l.z));
	let dy = select(u, d, abs(d.z) < abs(u.z));
	var n = cross(dy, dx);
	if (dot(n, n) < 1e-12) { return vec3<f32>(0.0, 0.0, 1.0); }
	n = normalize(n);
	if (dot(n, P) > 0.0) { n = -n; }
	return n;
}

// A moby (Ratchet, enemies, crates, pickups) leaves alpha 0 in the post scene, every other pixel 1 (viewer.rs `fs`; a vegetation leaf's
// soft edge at least 0.78, from its alpha-to-coverage): the share (0 .. 1) of a resolved pixel (4 samples: alpha 1 - k / 4) that is moby.
fn moby_cover(a: f32) -> f32 {
	return clamp((0.78 - a) / 0.5, 0.0, 1.0);
}

struct GSO { @location(0) rad: vec4<f32>, @location(1) nrm: vec4<f32> };

// Each grid cell: the scene's linear colour over the cell (one filtered tap in its middle) and the view-space normal; sky
// cells: alpha 0 (they send no light and receive none).
@fragment fn fs_gsrc(v: VO) -> GSO {
	let q = vec2<i32>(v.pos.xy);
	let s = i32(p.gi2.w);
	let hs = vec2<i32>(textureDimensions(lin));
	let h = min(q * s, hs - vec2<i32>(1));
	var o: GSO;
	o.rad = vec4<f32>(0.0);
	o.nrm = vec4<f32>(0.0);
	if (sky(textureLoad(lin, h, 0).r)) { return o; }
	let uv = (vec2<f32>(q * s * 2) + f32(s)) * p.size.zw;
	// A cell the cell's middle or corners show a moby in is left empty, like sky: no light from it, none for it.
	let ho = f32(s) * p.size.zw;
	var amin = textureSampleLevel(scene, lsamp, uv, 0.0).a;
	for (var k = 0; k < 4; k++) {
		let sg = vec2<f32>(select(-1.0, 1.0, (k & 1) == 1), select(-1.0, 1.0, (k & 2) == 2));
		amin = min(amin, textureSampleLevel(scene, lsamp, uv + sg * ho, 0.0).a);
	}
	if (moby_cover(amin) > 0.0) { return o; }
	o.rad = vec4<f32>(to_linear(textureSampleLevel(scene, lsamp, uv, 0.0).rgb), 1.0);
	o.nrm = vec4<f32>(gi_normal(h) * 0.5 + 0.5, 1.0);
	return o;
}

// The bounce light reaching each grid cell: taps on a rotated spiral within the world radius (at least 1.5 cells, at most
// p.gi2.z), each sending its colour times the receiver's facing, the sender's facing (a surface turned away sends nothing: no
// halo behind objects) and a distance falloff. Inputs: cell colours in bloom_in, normals in ao_in.
@fragment fn fs_gi(v: VO) -> @location(0) vec4<f32> {
	let q = vec2<i32>(v.pos.xy);
	let s = i32(p.gi2.w);
	let gd = vec2<i32>(textureDimensions(ao_in));
	let nr = textureLoad(ao_in, q, 0);
	if (nr.a < 0.5) { return vec4<f32>(0.0); }
	let n = normalize(nr.xyz * 2.0 - 1.0);
	let P = view_pos(q * s);
	let R = p.gi.y;
	// Grid cells per world unit at this depth.
	let ppu = p.half.y * 0.5 / (p.proj.y * -P.z * f32(s));
	let rad = clamp(R * ppu, 1.5, p.gi2.z);
	let rot = ign(v.pos.xy) * 6.2831853;
	let N = i32(p.gi.z);
	var acc = vec3<f32>(0.0);
	for (var i = 0; i < N; i++) {
		let a = rot + f32(i) * 2.3999632;
		let t = sqrt((f32(i) + 0.5) / f32(N)) * rad;
		let o = vec2<i32>(round(vec2<f32>(cos(a), sin(a)) * t));
		let qs = q + o;
		if (all(o == vec2<i32>(0)) || any(qs < vec2<i32>(0)) || any(qs >= gd)) { continue; }
		let ns = textureLoad(ao_in, qs, 0);
		if (ns.a < 0.5) { continue; }
		let w = view_pos(qs * s) - P;
		let d2 = dot(w, w);
		let dir = w * inverseSqrt(max(d2, 1e-6));
		let cr = max(dot(n, dir), 0.0);
		let cs = max(dot(ns.xyz * 2.0 - 1.0, -dir), 0.0);
		let fall = clamp(1.0 - d2 / (R * R), 0.0, 1.0);
		acc += textureLoad(bloom_in, qs, 0).rgb * (cr * cs * fall);
	}
	return vec4<f32>(acc * (2.0 / f32(N)), 1.0);
}

// Depth-aware 7-tap blur of the bounce light on the grid; p.grade.z picks the axis (0 across, 1 down).
@fragment fn fs_giblur(v: VO) -> @location(0) vec4<f32> {
	let q = vec2<i32>(v.pos.xy);
	let s = i32(p.gi2.w);
	let gd = vec2<i32>(textureDimensions(gi_in));
	let hs = vec2<i32>(textureDimensions(lin));
	let axis = select(vec2<i32>(1, 0), vec2<i32>(0, 1), p.grade.z > 0.5);
	let z0 = textureLoad(lin, min(q * s, hs - vec2<i32>(1)), 0).r;
	var sum = vec3<f32>(0.0);
	var wsum = 0.0;
	for (var k = -3; k <= 3; k++) {
		let qq = clamp(q + axis * k, vec2<i32>(0), gd - vec2<i32>(1));
		let z = textureLoad(lin, min(qq * s, hs - vec2<i32>(1)), 0).r;
		let w = exp(-f32(k * k) / 8.0) * max(0.0, 1.0 - abs(z - z0) / (p.gi3.x * z0 + 0.05));
		sum += textureLoad(gi_in, qq, 0).rgb * w;
		wsum += w;
	}
	return vec4<f32>(sum / max(wsum, 1e-4), 1.0);
}

// The bounce light at full-size pixel `c0` of linear depth `z`: the grid cells around it, those whose depth is closest weighing most.
fn gi_at(c0: vec2<i32>, z: f32) -> vec3<f32> {
	let s = i32(p.gi2.w);
	let gd = vec2<i32>(textureDimensions(gi_in));
	let hs = vec2<i32>(textureDimensions(lin));
	// Cell q's value sits at full-size coordinate 2 q s + 1 (the middle of half-size texel q s).
	let h = (vec2<f32>(c0) - 0.5) / f32(2 * s);
	let b = vec2<i32>(floor(h));
	let f = h - floor(h);
	var sum = vec3<f32>(0.0);
	var wsum = 0.0;
	for (var k = 0; k < 4; k++) {
		let o = vec2<i32>(k & 1, k >> 1);
		let q = clamp(b + o, vec2<i32>(0), gd - vec2<i32>(1));
		let bw = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
		let zq = textureLoad(lin, min(q * s, hs - vec2<i32>(1)), 0).r;
		let w = bw * (1.0 / (abs(zq - z) / (0.02 * z + 0.02) + 0.05)) + 1e-4;
		sum += textureLoad(gi_in, q, 0).rgb * w;
		wsum += w;
	}
	return sum / wsum;
}

// Whether the water pass drew pixel `c0` this frame (a water surface in its position target, or any pixel it changed: the
// water, its waterfalls, the underwater look): no bounce light there.
fn gi_skip(c0: vec2<i32>) -> bool {
	if (p.gi.w < 0.5) { return false; }
	if (textureLoad(wpos_in, c0, 0).a > 0.5) { return true; }
	return any(textureLoad(prew_in, c0, 0).rgb != textureLoad(scene, c0, 0).rgb);
}

fn luma(c: vec3<f32>) -> f32 {
	return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// The first bloom level: the scene's bright part (soft knee on linear luminance; p.params.w = threshold), sky a quarter.
@fragment fn fs_bright(v: VO) -> @location(0) vec4<f32> {
	let px = p.size.zw;
	var acc = vec3<f32>(0.0);
	var wsum = 0.0;
	for (var k = 0; k < 4; k++) {
		let o = vec2<f32>(f32(k & 1) - 0.5, f32(k >> 1) - 0.5) * px;
		let c = to_linear(textureSampleLevel(scene, lsamp, v.uv + o, 0.0).rgb);
		// Karis average: a lone bright pixel does not flicker.
		let w = 1.0 / (1.0 + luma(c));
		acc += c * w;
		wsum += w;
	}
	var c = acc / wsum;
	let l = luma(c);
	let t = p.params.w;
	let knee = t * 0.5;
	var s = clamp(l - t + knee, 0.0, 2.0 * knee);
	s = s * s / (4.0 * knee + 1e-4);
	let k = max(s, l - t) / max(l, 1e-4);
	c *= k;
	let z = textureLoad(lin, clamp(vec2<i32>(v.pos.xy), vec2<i32>(0), vec2<i32>(textureDimensions(lin)) - vec2<i32>(1)), 0).r;
	if (sky(z)) { c *= 0.25; }
	// p.extra.x: the particle glow's gain (0 = Particles Classic: the target is not read).
	if (p.extra.x > 0.0) {
		// In linear light like the scene, so a blast's glow adds to it instead of washing its colours out.
		c += to_linear(textureSampleLevel(glow_in, lsamp, v.uv, 0.0).rgb) * p.extra.x;
	}
	return vec4<f32>(c, 1.0);
}

// p.sun = (key light's screen uv, ray strength, 0): the sky's bright part near it (sky pixels only, so the ground and every
// object in front of the sky block the light).
@fragment fn fs_rmask(v: VO) -> @location(0) vec4<f32> {
	let z = textureLoad(lin, clamp(vec2<i32>(v.pos.xy), vec2<i32>(0), vec2<i32>(textureDimensions(lin)) - vec2<i32>(1)), 0).r;
	if (!sky(z)) { return vec4<f32>(0.0); }
	let c = to_linear(textureSampleLevel(scene, lsamp, v.uv, 0.0).rgb);
	let d = length((v.uv - p.sun.xy) * vec2<f32>(p.size.x * p.size.w, 1.0));
	let near_sun = exp(-d * d * 5.0);
	return vec4<f32>(c * smoothstep(0.2, 0.7, luma(c)) * near_sun, 1.0);
}

// Radial blur of the mask (in bloom_in) towards the key light, dithered start against banding.
@fragment fn fs_rays(v: VO) -> @location(0) vec4<f32> {
	// p.q.z = taps (48 = High, the picture before the levels existed); fewer taps cover the same span with a steeper decay per tap
	// and the same total weight, so the shafts keep their brightness.
	let n = i32(p.q.z);
	let delta = (v.uv - p.sun.xy) * (0.85 / f32(n));
	var uv = v.uv - delta * ign(v.pos.xy);
	var acc = vec3<f32>(0.0);
	var w = 1.0;
	var decay = 0.955;
	var gain = 1.0;
	if (n < 48) {
		decay = pow(0.955, 48.0 / f32(n));
		gain = (1.0 - decay) / 0.045;
	}
	for (var i = 0; i < n; i++) {
		acc += textureSampleLevel(bloom_in, lsamp, uv, 0.0).rgb * w;
		w *= decay;
		uv -= delta;
	}
	return vec4<f32>(acc / 14.0 * gain, 1.0);
}

// 13-tap downsample (the larger level in bloom_in).
@fragment fn fs_down(v: VO) -> @location(0) vec4<f32> {
	let t = 1.0 / vec2<f32>(textureDimensions(bloom_in));
	let a = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(-2.0, -2.0), 0.0).rgb;
	let b = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(0.0, -2.0), 0.0).rgb;
	let c = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(2.0, -2.0), 0.0).rgb;
	let d = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(-2.0, 0.0), 0.0).rgb;
	let e = textureSampleLevel(bloom_in, lsamp, v.uv, 0.0).rgb;
	let f = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(2.0, 0.0), 0.0).rgb;
	let g = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(-2.0, 2.0), 0.0).rgb;
	let h = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(0.0, 2.0), 0.0).rgb;
	let i = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(2.0, 2.0), 0.0).rgb;
	let j = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(-1.0, -1.0), 0.0).rgb;
	let k = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(1.0, -1.0), 0.0).rgb;
	let l = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(-1.0, 1.0), 0.0).rgb;
	let m = textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(1.0, 1.0), 0.0).rgb;
	var o = e * 0.125 + (a + c + g + i) * 0.03125 + (b + d + f + h) * 0.0625 + (j + k + l + m) * 0.125;
	return vec4<f32>(o, 1.0);
}

// 9-tap tent upsample of the smaller level (bloom_in), added onto the larger one by the blend.
@fragment fn fs_up(v: VO) -> @location(0) vec4<f32> {
	let t = 1.0 / vec2<f32>(textureDimensions(bloom_in));
	var o = textureSampleLevel(bloom_in, lsamp, v.uv, 0.0).rgb * 4.0;
	o += (textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(-1.0, 0.0), 0.0).rgb + textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(1.0, 0.0), 0.0).rgb + textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(0.0, -1.0), 0.0).rgb + textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(0.0, 1.0), 0.0).rgb) * 2.0;
	o += textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(-1.0, -1.0), 0.0).rgb + textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(1.0, -1.0), 0.0).rgb + textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(-1.0, 1.0), 0.0).rgb + textureSampleLevel(bloom_in, lsamp, v.uv + t * vec2<f32>(1.0, 1.0), 0.0).rgb;
	return vec4<f32>(o / 16.0, 1.0);
}

// Linear-light tone curve: contrast around mid grey (p.grade.x = exponent) and a soft shoulder above 0.75 to 1.
fn tone(x: vec3<f32>) -> vec3<f32> {
	let y = 0.18 * pow(max(x, vec3<f32>(0.0)) / 0.18, vec3<f32>(p.grade.x));
	let t = 0.75;
	let s = t + (1.0 - t) * (1.0 - exp(-(y - t) / (1.0 - t)));
	return select(y, s, y > vec3<f32>(t));
}

// The occlusion at full size: the half-size taps whose depth is closest to this pixel's weigh most.
fn ao_at(v: VO, z: f32) -> f32 {
	let hs = vec2<i32>(textureDimensions(lin));
	let h = v.pos.xy * 0.5 - 0.5;
	let b = vec2<i32>(floor(h));
	let f = h - floor(h);
	var sum = 0.0;
	var wsum = 0.0;
	for (var k = 0; k < 4; k++) {
		let o = vec2<i32>(k & 1, k >> 1);
		let q = clamp(b + o, vec2<i32>(0), hs - vec2<i32>(1));
		let bw = select(1.0 - f.x, f.x, o.x == 1) * select(1.0 - f.y, f.y, o.y == 1);
		let zq = textureLoad(lin, q, 0).r;
		let w = bw * (1.0 / (abs(zq - z) / (0.02 * z + 0.02) + 0.05)) + 1e-4;
		sum += textureLoad(ao_in, q, 0).r * w;
		wsum += w;
	}
	return sum / wsum;
}

// [PC] Particles Enhanced: the world position seen through half-size depth texel `q` at linear depth `z`
// (p.cam = eye, forward, right x horizontal tan, up x vertical tan).
fn half_pos(q: vec2<i32>, z: f32) -> vec3<f32> {
	let uv = (vec2<f32>(q) + 0.5) / vec2<f32>(textureDimensions(lin));
	return p.cam[0].xyz + z * (p.cam[1].xyz + (uv.x * 2.0 - 1.0) * p.cam[2].xyz + (1.0 - uv.y * 2.0) * p.cam[3].xyz);
}

// The effect lights (partfx.rs `lights`; p.extra.y in use, p.fxl = (position, reach), (rgb, 0)) on the surface at full-size
// pixel `c0`: its normal from the half-size depth (the flatter neighbour on each axis, so silhouettes keep their own
// side), half-wrapped facing, reach falloff squared. Sky: none.
fn fx_light(c0: vec2<i32>) -> vec3<f32> {
	let hs = vec2<i32>(textureDimensions(lin));
	let q = min(c0 / 2, hs - vec2<i32>(1));
	let z = textureLoad(lin, q, 0).r;
	if (sky(z)) { return vec3<f32>(0.0); }
	let pc = half_pos(q, z);
	let qx0 = max(q - vec2<i32>(1, 0), vec2<i32>(0));
	let qx1 = min(q + vec2<i32>(1, 0), hs - vec2<i32>(1));
	let qy0 = max(q - vec2<i32>(0, 1), vec2<i32>(0));
	let qy1 = min(q + vec2<i32>(0, 1), hs - vec2<i32>(1));
	let zx0 = textureLoad(lin, qx0, 0).r;
	let zx1 = textureLoad(lin, qx1, 0).r;
	let zy0 = textureLoad(lin, qy0, 0).r;
	let zy1 = textureLoad(lin, qy1, 0).r;
	let dx = select(half_pos(qx1, zx1) - pc, pc - half_pos(qx0, zx0), abs(zx0 - z) < abs(zx1 - z));
	let dy = select(half_pos(qy1, zy1) - pc, pc - half_pos(qy0, zy0), abs(zy0 - z) < abs(zy1 - z));
	var n = cross(dx, dy);
	if (dot(n, n) < 1e-12) { n = -p.cam[1].xyz; } else { n = normalize(n); }
	if (dot(n, p.cam[0].xyz - pc) < 0.0) { n = -n; }
	var acc = vec3<f32>(0.0);
	for (var k = 0u; k < u32(p.extra.y); k++) {
		let a = p.fxl[2u * k];
		let d = a.xyz - pc;
		let dist = length(d);
		if (dist >= a.w) { continue; }
		let f = 1.0 - dist / a.w;
		let face = clamp(dot(n, d / max(dist, 1e-4)) * 0.6 + 0.4, 0.0, 1.0);
		acc += p.fxl[2u * k + 1u].rgb * f * f * face;
	}
	return acc;
}

// Heat shimmer (p.extra.w = strength in pixels at 1080 lines, p.extra.z = seconds): the air above warm glow (the particle glow
// target's red over blue, gathered from below, where the heat rises from) wobbles the picture; pixels the particles cover
// themselves stay put. Returns the uv offset (0 = none).
fn shimmer(v: VO) -> vec2<f32> {
	if (p.extra.w <= 0.0) { return vec2<f32>(0.0); }
	var heat = 0.0;
	for (var k = 1; k <= 5; k++) {
		let g = textureSampleLevel(glow_in, lsamp, v.uv + vec2<f32>(0.0, f32(k) * 0.012), 0.0).rgb;
		heat += max(g.r - g.b, 0.0) * (1.0 - f32(k) * 0.15);
	}
	heat = clamp(heat, 0.0, 1.0) * (1.0 - clamp(textureLoad(glow_in, vec2<i32>(v.pos.xy), 0).a * 2.0, 0.0, 1.0));
	if (heat <= 0.0) { return vec2<f32>(0.0); }
	let t = p.extra.z;
	let wob = vec2<f32>(sin(v.uv.y * 95.0 - t * 9.0 + sin(v.uv.x * 41.0 + t * 2.0) * 2.0), 0.6 * cos(v.uv.x * 73.0 + t * 7.0 + sin(v.uv.y * 37.0) * 2.0));
	return wob * heat * p.extra.w * (p.size.y / 1080.0) * p.size.zw;
}

// One scene texel at `c` (clamped), or, while the shimmer moves it (`off` != 0), the filtered scene at its centre + `off`.
fn scene_at(c: vec2<i32>, dim: vec2<i32>, off: vec2<f32>) -> vec3<f32> {
	let q = clamp(c, vec2<i32>(0), dim - vec2<i32>(1));
	if (all(off == vec2<f32>(0.0))) { return textureLoad(scene, q, 0).rgb; }
	return textureSampleLevel(scene, lsamp, (vec2<f32>(q) + 0.5) * p.size.zw + off, 0.0).rgb;
}

// p.params: (occlusion strength, bloom strength, sharpening, bloom threshold); p.grade: (contrast, saturation, blur axis,
// occlusion radius). Occlusion off: p.params.x = 0 (the half-size occlusion is not read).
@fragment fn fs_comp(v: VO) -> @location(0) vec4<f32> {
	let dim = vec2<i32>(textureDimensions(scene));
	let c0 = vec2<i32>(v.pos.xy);
	let off = shimmer(v);
	let e = scene_at(c0, dim, off);
	let n = scene_at(c0 + vec2<i32>(0, -1), dim, off);
	let s = scene_at(c0 + vec2<i32>(0, 1), dim, off);
	let w = scene_at(c0 + vec2<i32>(-1, 0), dim, off);
	let ea = scene_at(c0 + vec2<i32>(1, 0), dim, off);
	// Contrast-adaptive sharpening (the weight shrinks where the neighbourhood already has contrast).
	let mn = min(e, min(min(n, s), min(w, ea)));
	let mx = max(e, max(max(n, s), max(w, ea)));
	let amp = sqrt(clamp(min(mn, 1.0 - mx) / max(mx, vec3<f32>(1e-4)), vec3<f32>(0.0), vec3<f32>(1.0)));
	let peak = -1.0 / mix(8.0, 5.0, p.params.z);
	let wt = amp * peak;
	var c = clamp((e + (n + s + w + ea) * wt) / (1.0 + 4.0 * wt), vec3<f32>(0.0), vec3<f32>(1.0));
	var l = to_linear(c);
	if (p.gi.x > 0.0) {
		// Global illumination: the bounce light on the surface's own colour (p.gi2.x: towards its hue, so surfaces in shade
		// catch it too) plus a faint floor; occluded with the rest below. Not on sky or the water pass's pixels.
		let hs = vec2<i32>(textureDimensions(lin));
		let z = textureLoad(lin, min(c0 / 2, hs - vec2<i32>(1)), 0).r;
		// Mobys (Ratchet, enemies, crates, pickups: `moby_cover`) catch none; an edge pixel catches the share that is not moby.
		let mine = 1.0 - moby_cover(textureLoad(scene, c0, 0).a);
		if (!sky(z) && !gi_skip(c0) && mine > 0.0) {
			let mx = max(l.r, max(l.g, l.b));
			let alb = mix(l, l * (0.5 / max(mx, 0.1)), p.gi2.x);
			// The gathered light, softly held under p.gi3.y (linear: a bright object right next to a surface tints it, never lights it up).
			let gs = gi_at(c0, z) * p.gi.x;
			l += gs / (1.0 + max(gs.r, max(gs.g, gs.b)) / p.gi3.y) * (alb + vec3<f32>(p.gi2.y)) * mine;
		}
	}
	if (p.params.x > 0.0) {
		let hs = vec2<i32>(textureDimensions(lin));
		let z = textureLoad(lin, min(c0 / 2, hs - vec2<i32>(1)), 0).r;
		if (!sky(z)) {
			l *= ao_at(v, z);
		}
	}
	if (p.extra.y > 0.0) {
		// The effect lights add to what the surface already shows (a faint floor so black walls still catch some),
		// not on the particles' own pixels.
		l += (l + vec3<f32>(0.02)) * fx_light(c0) * (1.0 - clamp(textureLoad(glow_in, c0, 0).a, 0.0, 1.0));
	}
	l += textureSampleLevel(bloom_in, lsamp, v.uv, 0.0).rgb * p.params.y;
	if (p.sun.z > 0.0) {
		l += textureSampleLevel(rays_in, lsamp, v.uv, 0.0).rgb * p.sun.z;
	}
	l = tone(l);
	// Vibrance: dull colours gain up to p.grade.y - 1, already saturated ones (yellow shirts, lava) barely move.
	let y = luma(l);
	let mx3 = max(l.r, max(l.g, l.b));
	let sat = (mx3 - min(l.r, min(l.g, l.b))) / max(mx3, 1e-4);
	let k = 1.0 + (p.grade.y - 1.0) * (1.0 - sat);
	l = max(vec3<f32>(y) + (l - vec3<f32>(y)) * k, vec3<f32>(0.0));
	if (p.half.w < 0.0) {
		// Dev view (RAC_POST_VIEW=ao): the occlusion alone.
		let hs = vec2<i32>(textureDimensions(lin));
		let z = textureLoad(lin, min(c0 / 2, hs - vec2<i32>(1)), 0).r;
		return vec4<f32>(vec3<f32>(select(ao_at(v, z), 1.0, sky(z))), 1.0);
	}
	return vec4<f32>(pow(min(l, vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2)), 1.0);
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostU {
	size: [f32; 4],
	half: [f32; 4],
	proj: [f32; 4],
	params: [f32; 4],
	grade: [f32; 4],
	extra: [f32; 4],
	sun: [f32; 4],
	/// [PC] Particles Enhanced: eye, forward, right x horizontal tan, up x vertical tan; the effect lights (partfx.rs `FxLights::rows`).
	cam: [[f32; 4]; 4],
	fxl: [[f32; 4]; 2 * super::partfx::FX_LIGHTS],
	/// [PC] Quality levels (Graphics -> Low / Medium / High): x = occlusion taps, y = the occlusion's largest screen radius (half-size pixels).
	q: [f32; 4],
	/// [PC] Global illumination (`fs_gi`; the shader's `P` names each lane): strength (0 = off), world radius, taps, water mask valid;
	/// albedo towards the hue, floor, largest radius in grid cells, half-size texels per grid cell; blur depth tolerance, cap.
	gi: [f32; 4],
	gi2: [f32; 4],
	gi3: [f32; 4],
}

/// Global illumination by Graphics level (1 Low, 2 Medium, otherwise High): (taps per cell, world radius, largest radius in grid
/// cells at 720 lines, grid size divisor of the window: 4 = quarter size, 2 = half size). The largest radius scales with the
/// window height: a bigger window gathers over the same share of the picture, so things close to the camera do not bounce
/// more. Every level gathers over the same world radius: the levels differ in grid size and noise, not in brightness. GPU time at 1280x720 (RTX 5070 Ti, 2026-10-19): Low ~0.025 ms,
/// Medium ~0.07 ms, High 0.09-0.16 ms, plus ~0.005 ms in the composite.
const fn gi_quality(level: u8) -> (f32, f32, f32, u32) {
	match level {
		1 => (6.0, 4.0, 10.0, 4),
		2 => (8.0, 4.0, 14.0, 4),
		_ => (12.0, 4.0, 32.0, 2),
	}
}

/// Ambient occlusion by Graphics level (1 Low, 2 Medium, otherwise High): (taps per pixel, largest radius in half-size pixels).
/// High is the picture before the levels existed (12 taps, 48 px). The half-size target and the two blur passes are the same.
const fn ao_quality(level: u8) -> (f32, f32) {
	match level {
		1 => (5.0, 24.0),
		2 => (8.0, 36.0),
		_ => (12.0, 48.0),
	}
}

/// Sun rays by Graphics level (1 Low, 2 Medium, otherwise High): taps of the radial blur in `fs_rays` (High = the 48 of before the levels).
const fn sun_taps(level: u8) -> f32 {
	match level {
		1 => 16.0,
		2 => 32.0,
		_ => 48.0,
	}
}

/// The look's numbers (one tuning for every level).
const AO_STRENGTH: f32 = 1.8;
const AO_RADIUS: f32 = 1.0;
const BLOOM_STRENGTH: f32 = 0.12;
const BLOOM_THRESHOLD: f32 = 0.7;
const SHARPEN: f32 = 0.35;
const CONTRAST: f32 = 1.08;
const SATURATION: f32 = 1.15;
/// Particles Enhanced: the additive particles' glow into the bloom (on top of the thresholded scene).
const GLOW_GAIN: f32 = 1.6;
/// Sun rays On: the rays' strength (times the key light's on-screen fade).
const RAYS: f32 = 0.55;
/// [PC] Heat shimmer strength (pixels at 1080 lines; `fs_comp` `shimmer`).
const SHIMMER: f32 = 2.5;
/// [PC] Global illumination: the bounce light's strength, how far the lit surface's colour goes towards its hue (0 the colour
/// as lit, 1 its hue at half brightness), the floor every surface catches, the blur's depth tolerance (fraction of the depth)
/// and the soft cap of the gathered light (linear; tuned on Kerwan, Pokitaru, Eudora, Blarg and Novalis, 2026-10-19; cap 1.0 ->
/// 0.2 after the user's playtest: Ratchet close to the camera lit the sand under his feet too much).
const GI_STRENGTH: f32 = 48.0;
const GI_ALBEDO: f32 = 0.5;
const GI_FLOOR: f32 = 0.02;
const GI_BLUR_TOL: f32 = 0.05;
const GI_CAP: f32 = 0.2;
/// Grid formats: the cells' colour (linear) and normal, the bounce light.
const GI_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const GI_NORMAL_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// Seconds since the first call (the shimmer's clock), wrapped so the shader's sines keep their precision.
fn seconds() -> f32 {
	static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
	(T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() % 600.0) as f32
}

/// Sun rays: the direction towards the sun level `level`'s sky paints (sky mesh axes = world axes, Z up), or None for a
/// sky without one. Measured from the sky shells (centroid of the brightest texels in a one-off dump of every level's sky, 2026-10-15): the day
/// skies of Novalis, Kerwan, Eudora, Pokitaru and Kalebo III share one dome (shell 0) whose bright glow is centred at
/// azimuth 0, elevation 36 degrees; Veldin's dusk sun sits on the horizon band (shell 4); Gemlik's red sun is a disc
/// (shell 2). The night, space, storm and lava skies have none.
pub(super) fn sky_sun(level: usize) -> Option<[f32; 3]> {
	match level {
		0 => Some([0.9888, 0.0174, 0.1478]),
		1 | 3 | 4 | 11 | 16 => Some([0.8068, 0.0031, 0.5906]),
		13 => Some([-0.3961, 0.8970, 0.1917]),
		_ => None,
	}
}

/// Sun rays: the sun's screen position (uv) and strength for travel direction `dir` (towards the ground) of brightness
/// `luma`, seen through `vp` from `eye`; None with the light below the horizon or behind the camera. The strength fades out
/// as the point leaves the screen (rays from just outside the frame still stream in).
pub(super) fn sun_on_screen(vp: &[[f32; 4]; 4], eye: [f32; 3], dir: [f32; 3], luma: f32) -> Option<([f32; 2], f32)> {
	if dir[2] > -0.05 {
		return None;
	}
	let q = [eye[0] - dir[0] * 100.0, eye[1] - dir[1] * 100.0, eye[2] - dir[2] * 100.0, 1.0];
	let c: [f32; 4] = [0, 1, 2, 3].map(|r| (0..4).map(|k| vp[k][r] * q[k]).sum());
	if c[3] <= 1e-3 {
		return None;
	}
	let (x, y) = (c[0] / c[3], c[1] / c[3]);
	let off = (x.abs().max(y.abs()) - 1.0).max(0.0);
	let s = (1.0 - off / 0.6).clamp(0.0, 1.0) * ((luma - 0.1) / 0.5).clamp(0.0, 1.0) * ((-dir[2] - 0.05) / 0.15).clamp(0.0, 1.0);
	(s > 0.0).then_some(([x * 0.5 + 0.5, 0.5 - y * 0.5], s))
}
/// Dev only: RAC_POST_VIEW=ao shows the occlusion alone.
fn view_ao() -> bool {
	static V: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*V.get_or_init(|| std::env::var("RAC_POST_VIEW").is_ok_and(|v| v == "ao"))
}

/// Bloom levels: 1/2 .. 1/32 of the window.
const LEVELS: usize = 5;

struct Targets {
	size: (u32, u32),
	/// Tells these targets from earlier ones (waterfx.rs binds `scene` and `lin`).
	generation: u64,
	scene: wgpu::TextureView,
	/// The particle glow (partfx.rs resolves into it).
	glow: wgpu::TextureView,
	lin: wgpu::TextureView,
	ao: [wgpu::TextureView; 2],
	bloom: Vec<wgpu::TextureView>,
	/// Bind groups: lin, ao, blur across, blur down, bright, down[1..], up[..], comp.
	g_lin: wgpu::BindGroup,
	g_ao: wgpu::BindGroup,
	g_blur: [wgpu::BindGroup; 2],
	g_bright: wgpu::BindGroup,
	g_down: Vec<wgpu::BindGroup>,
	g_up: Vec<wgpu::BindGroup>,
	g_comp: wgpu::BindGroup,
	/// Sun rays: the mask and the blurred rays (half size), their bind groups.
	rays: [wgpu::TextureView; 2],
	g_rmask: wgpu::BindGroup,
	g_rays: wgpu::BindGroup,
	/// The depth target's depth aspect (binding 2 of every group).
	depth_only: wgpu::TextureView,
}

/// [PC] Global illumination's grid targets (made with the first frame that asks for them, kept while the level stays on).
struct GiTargets {
	/// Window size, grid divisor, the `Targets` generation they bind.
	key: (u32, u32, u32, u64),
	/// The cells' colour, the cells' normal, the bounce light and its blur's middle step.
	src: [wgpu::TextureView; 2],
	gi: [wgpu::TextureView; 2],
	g_src: wgpu::BindGroup,
	g_gi: wgpu::BindGroup,
	g_blur: [wgpu::BindGroup; 2],
	g_comp: wgpu::BindGroup,
	/// The composite's group with the water pass's position target and pre-water scene copy (`bind_water`), and those two.
	g_comp_w: Option<(wgpu::TextureView, wgpu::Texture, wgpu::BindGroup)>,
}

/// The views of one post bind group; None = the 1x1 placeholder of that binding.
#[derive(Clone, Copy, Default)]
struct Views<'a> {
	lin: Option<&'a wgpu::TextureView>,
	ao: Option<&'a wgpu::TextureView>,
	bloom: Option<&'a wgpu::TextureView>,
	glow: Option<&'a wgpu::TextureView>,
	rays: Option<&'a wgpu::TextureView>,
	gi: Option<&'a wgpu::TextureView>,
	wpos: Option<&'a wgpu::TextureView>,
	prew: Option<&'a wgpu::TextureView>,
}

pub(super) struct Post {
	layout: wgpu::BindGroupLayout,
	/// Uniform buffers: the shared one and the blur's second axis.
	ubuf: wgpu::Buffer,
	ubuf_v: wgpu::Buffer,
	sampler: wgpu::Sampler,
	p_lin: wgpu::RenderPipeline,
	p_ao: wgpu::RenderPipeline,
	p_blur: wgpu::RenderPipeline,
	p_bright: wgpu::RenderPipeline,
	p_down: wgpu::RenderPipeline,
	p_up: wgpu::RenderPipeline,
	p_comp: wgpu::RenderPipeline,
	p_rmask: wgpu::RenderPipeline,
	p_rays: wgpu::RenderPipeline,
	/// [PC] Global illumination: cells, gather, blur.
	p_gsrc: wgpu::RenderPipeline,
	p_gi: wgpu::RenderPipeline,
	p_giblur: wgpu::RenderPipeline,
	/// A 1x1 placeholder for bindings a pass does not read.
	dummy: wgpu::TextureView,
	dummy_r32: wgpu::TextureView,
	format: wgpu::TextureFormat,
	t: Option<Targets>,
	gi: Option<GiTargets>,
	/// This frame's water pass ran: the composite reads its targets (`bind_water`).
	gi_water: bool,
}

const LIN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl Post {
	pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Post {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("post"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("post", POST_SHADER.into())) });
		let tex = |binding: u32, sample_type: wgpu::TextureSampleType, multisampled: bool| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled },
			count: None,
		};
		let float = wgpu::TextureSampleType::Float { filterable: true };
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("post"),
			entries: &[
				wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
				tex(1, float, false),
				tex(2, wgpu::TextureSampleType::Depth, true),
				tex(3, wgpu::TextureSampleType::Float { filterable: false }, false),
				tex(4, wgpu::TextureSampleType::Float { filterable: false }, false),
				tex(5, float, false),
				wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
				tex(7, float, false),
				tex(8, float, false),
				tex(9, wgpu::TextureSampleType::Float { filterable: false }, false),
				tex(10, wgpu::TextureSampleType::Float { filterable: false }, false),
				tex(11, wgpu::TextureSampleType::Float { filterable: false }, false),
			],
		});
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("post"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let make = |fs: &str, target: wgpu::TextureFormat, blend: Option<wgpu::BlendState>, ms: bool| {
			device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
				label: Some(fs),
				layout: Some(&pl),
				vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
				fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some(fs), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: target, blend, write_mask: wgpu::ColorWrites::ALL })] }),
				primitive: Default::default(),
				depth_stencil: None,
				// The composite writes every sample of the MSAA target the HUD pass then loads.
				multisample: wgpu::MultisampleState { count: if ms { MSAA } else { 1 }, ..Default::default() },
				multiview_mask: None,
				cache: None,
			})
		};
		let add = wgpu::BlendState { color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add }, alpha: wgpu::BlendComponent::REPLACE };
		// [PC] Global illumination's cells: colour and normal in one pass (two targets).
		let p_gsrc = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("fs_gsrc"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs_gsrc"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState { format: GI_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL }), Some(wgpu::ColorTargetState { format: GI_NORMAL_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })],
			}),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let ubuf = |label| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: std::mem::size_of::<PostU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
		let small = |format| {
			device
				.create_texture(&wgpu::TextureDescriptor { label: Some("post dummy"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] })
				.create_view(&Default::default())
		};
		Post {
			p_lin: make("fs_lin", LIN_FORMAT, None, false),
			p_ao: make("fs_ao", AO_FORMAT, None, false),
			p_blur: make("fs_blur", AO_FORMAT, None, false),
			p_bright: make("fs_bright", BLOOM_FORMAT, None, false),
			p_down: make("fs_down", BLOOM_FORMAT, None, false),
			p_up: make("fs_up", BLOOM_FORMAT, Some(add), false),
			p_comp: make("fs_comp", format, None, true),
			p_rmask: make("fs_rmask", BLOOM_FORMAT, None, false),
			p_rays: make("fs_rays", BLOOM_FORMAT, None, false),
			p_gsrc,
			p_gi: make("fs_gi", GI_FORMAT, None, false),
			p_giblur: make("fs_giblur", GI_FORMAT, None, false),
			layout,
			ubuf: ubuf("post uniforms"),
			ubuf_v: ubuf("post uniforms (blur down)"),
			sampler: device.create_sampler(&wgpu::SamplerDescriptor { label: Some("post linear"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() }),
			dummy: small(BLOOM_FORMAT),
			dummy_r32: small(LIN_FORMAT),
			format,
			t: None,
			gi: None,
			gi_water: false,
		}
	}

	/// Makes the targets for a `w` x `h` frame (created or resized as the window is); `depth` is the level pass's MSAA depth target.
	/// `gi_level` > 0 (Global illumination Low / Medium / High): also the grid targets of that level's size.
	pub(super) fn prepare(&mut self, device: &wgpu::Device, w: u32, h: u32, depth: &wgpu::TextureView, gi_level: u8) {
		if self.t.as_ref().is_none_or(|t| t.size != (w, h)) {
			self.t = Some(self.targets(device, w, h, depth));
		}
		self.gi_water = false;
		if gi_level > 0 {
			let Some(t) = &self.t else { return };
			let key = (w, h, gi_quality(gi_level).3, t.generation);
			if self.gi.as_ref().is_none_or(|g| g.key != key) {
				self.gi = Some(self.gi_targets(device, t, key));
			}
		}
	}

	/// Global illumination, after the water pass's positions: `mask` = its position target and its copy of the scene before
	/// it shaded (waterfx.rs `gi_mask`) when it runs this frame, so the composite leaves its pixels alone.
	pub(super) fn bind_water(&mut self, device: &wgpu::Device, mask: Option<(&wgpu::TextureView, &wgpu::Texture)>) {
		self.gi_water = false;
		let (Some(t), Some(g), Some((wpos, copy))) = (&self.t, &self.gi, mask) else { return };
		if g.g_comp_w.as_ref().is_none_or(|(v, c, _)| v != wpos || c != copy) {
			let prew = copy.create_view(&Default::default());
			let group = self.bind(device, &self.ubuf, t, &t.scene, Views { lin: Some(&t.lin), ao: Some(&t.ao[0]), bloom: Some(&t.bloom[0]), glow: Some(&t.glow), rays: Some(&t.rays[1]), gi: Some(&g.gi[0]), wpos: Some(wpos), prew: Some(&prew) });
			if let Some(g) = &mut self.gi {
				g.g_comp_w = Some((wpos.clone(), copy.clone(), group));
			}
		}
		self.gi_water = true;
	}

	/// One post bind group: uniforms `buf`, `scene` and the views `v` (placeholders for the rest).
	fn bind(&self, device: &wgpu::Device, buf: &wgpu::Buffer, t: &Targets, scene: &wgpu::TextureView, v: Views) -> wgpu::BindGroup {
		self.bind_d(device, buf, &t.depth_only, scene, v)
	}

	fn bind_d(&self, device: &wgpu::Device, buf: &wgpu::Buffer, depth_only: &wgpu::TextureView, scene: &wgpu::TextureView, v: Views) -> wgpu::BindGroup {
		let (d, d32) = (&self.dummy, &self.dummy_r32);
		fn tv(binding: u32, view: &wgpu::TextureView) -> wgpu::BindGroupEntry<'_> {
			wgpu::BindGroupEntry { binding, resource: wgpu::BindingResource::TextureView(view) }
		}
		device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("post"),
			layout: &self.layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
				tv(1, scene),
				tv(2, depth_only),
				tv(3, v.lin.unwrap_or(d32)),
				tv(4, v.ao.unwrap_or(d32)),
				tv(5, v.bloom.unwrap_or(d)),
				wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&self.sampler) },
				tv(7, v.glow.unwrap_or(d)),
				tv(8, v.rays.unwrap_or(d)),
				tv(9, v.gi.unwrap_or(d)),
				tv(10, v.wpos.unwrap_or(d32)),
				tv(11, v.prew.unwrap_or(d)),
			],
		})
	}

	/// The grid targets and their bind groups for `key` (`prepare`).
	fn gi_targets(&self, device: &wgpu::Device, t: &Targets, key: (u32, u32, u32, u64)) -> GiTargets {
		let (w, h, div, _) = key;
		let (gw, gh) = (w.div_ceil(div), h.div_ceil(div));
		let make = |label, format| {
			device
				.create_texture(&wgpu::TextureDescriptor {
					label: Some(label),
					size: wgpu::Extent3d { width: gw.max(1), height: gh.max(1), depth_or_array_layers: 1 },
					mip_level_count: 1,
					sample_count: 1,
					dimension: wgpu::TextureDimension::D2,
					format,
					usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
					view_formats: &[],
				})
				.create_view(&Default::default())
		};
		let src = [make("post gi cells", GI_FORMAT), make("post gi normals", GI_NORMAL_FORMAT)];
		let gi = [make("post gi", GI_FORMAT), make("post gi blur", GI_FORMAT)];
		let lin = Some(&t.lin);
		GiTargets {
			key,
			g_src: self.bind(device, &self.ubuf, t, &t.scene, Views { lin, ..Default::default() }),
			g_gi: self.bind(device, &self.ubuf, t, &t.scene, Views { lin, ao: Some(&src[1]), bloom: Some(&src[0]), ..Default::default() }),
			g_blur: [self.bind(device, &self.ubuf, t, &t.scene, Views { lin, gi: Some(&gi[0]), ..Default::default() }), self.bind(device, &self.ubuf_v, t, &t.scene, Views { lin, gi: Some(&gi[1]), ..Default::default() })],
			g_comp: self.bind(device, &self.ubuf, t, &t.scene, Views { lin, ao: Some(&t.ao[0]), bloom: Some(&t.bloom[0]), glow: Some(&t.glow), rays: Some(&t.rays[1]), gi: Some(&gi[0]), ..Default::default() }),
			g_comp_w: None,
			src,
			gi,
		}
	}

	/// The level pass's resolve target (after `prepare`).
	pub(super) fn scene(&self) -> Option<&wgpu::TextureView> {
		self.t.as_ref().map(|t| &t.scene)
	}

	/// The scene, the half-size linear depth and the targets' generation, for the water pass (after `prepare`).
	pub(super) fn glow(&self) -> Option<&wgpu::TextureView> {
		self.t.as_ref().map(|t| &t.glow)
	}

	pub(super) fn water_inputs(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView, u64)> {
		self.t.as_ref().map(|t| (&t.scene, &t.lin, &t.glow, t.generation))
	}

	/// Drops the size-dependent targets (the depth target they read was recreated).
	pub(super) fn reset(&mut self) {
		self.t = None;
	}

	fn targets(&self, device: &wgpu::Device, w: u32, h: u32, depth: &wgpu::TextureView) -> Targets {
		let make = |label, w: u32, h: u32, format, mips: u32| {
			device.create_texture(&wgpu::TextureDescriptor {
				label: Some(label),
				size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
				mip_level_count: mips,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format,
				usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
				view_formats: &[],
			})
		};
		let (hw, hh) = (w.div_ceil(2), h.div_ceil(2));
		let scene = device
			.create_texture(&wgpu::TextureDescriptor {
				label: Some("post scene"),
				size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
				mip_level_count: 1,
				sample_count: 1,
				dimension: wgpu::TextureDimension::D2,
				format: self.format,
				// COPY_SRC: the water pass reads a copy of it (waterfx.rs).
				usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_SRC,
				view_formats: &[],
			})
			.create_view(&Default::default());
		let glow = make("post particle glow", w, h, super::partfx::GLOW_FORMAT, 1).create_view(&Default::default());
		let lin = make("post depth", hw, hh, LIN_FORMAT, 1).create_view(&Default::default());
		let ao = [make("post ao", hw, hh, AO_FORMAT, 1).create_view(&Default::default()), make("post ao blur", hw, hh, AO_FORMAT, 1).create_view(&Default::default())];
		let bloom_tex = make("post bloom", hw, hh, BLOOM_FORMAT, LEVELS as u32);
		let bloom: Vec<wgpu::TextureView> = (0..LEVELS as u32).map(|m| bloom_tex.create_view(&wgpu::TextureViewDescriptor { base_mip_level: m, mip_level_count: Some(1), ..Default::default() })).collect();
		let depth_only = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		let rays = [make("post ray mask", hw, hh, BLOOM_FORMAT, 1).create_view(&Default::default()), make("post rays", hw, hh, BLOOM_FORMAT, 1).create_view(&Default::default())];
		let group = |buf: &wgpu::Buffer, v: Views| self.bind_d(device, buf, &depth_only, &scene, v);
		let l = Some(&lin);
		Targets {
			g_lin: group(&self.ubuf, Views::default()),
			g_ao: group(&self.ubuf, Views { lin: l, ..Default::default() }),
			g_blur: [group(&self.ubuf, Views { lin: l, ao: Some(&ao[0]), ..Default::default() }), group(&self.ubuf_v, Views { lin: l, ao: Some(&ao[1]), ..Default::default() })],
			g_bright: group(&self.ubuf, Views { lin: l, glow: Some(&glow), ..Default::default() }),
			g_down: (1..LEVELS).map(|m| group(&self.ubuf, Views { lin: l, bloom: Some(&bloom[m - 1]), ..Default::default() })).collect(),
			g_up: (1..LEVELS).map(|m| group(&self.ubuf, Views { lin: l, bloom: Some(&bloom[m]), ..Default::default() })).collect(),
			g_comp: group(&self.ubuf, Views { lin: l, ao: Some(&ao[0]), bloom: Some(&bloom[0]), glow: Some(&glow), rays: Some(&rays[1]), ..Default::default() }),
			g_rmask: group(&self.ubuf, Views { lin: l, ..Default::default() }),
			g_rays: group(&self.ubuf, Views { lin: l, bloom: Some(&rays[0]), ..Default::default() }),
			size: (w, h),
			generation: {
				static GEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
				GEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
			},
			scene,
			glow,
			lin,
			ao,
			bloom,
			rays,
			depth_only,
		}
	}

	/// After the level pass: the half-size depth, the occlusion (when on), `mid` (the water pass: it may draw into the scene),
	/// the bloom chain (with the particle glow when `glow`) and the sun rays (`sun` = the key light's screen uv and strength).
	/// `cam` / `aspect` are the level pass's projection.
	#[allow(clippy::too_many_arguments)]
	/// `fx`: the effect lights (with `glow`). `gi_level` > 0: the global illumination after `mid` (as given to `prepare`). `far`: the
	/// world far plane the depth was drawn with (`Gpu::world_far`).
	pub(super) fn encode(&self, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, cam: &Camera, aspect: f32, ao_level: u8, sun_level: u8, part_level: u8, glow: bool, fx: &super::partfx::FxLights, sun: Option<([f32; 2], f32)>, gi_level: u8, far: f32, mid: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView, Option<&wgpu::TextureView>)) {
		let Some(t) = &self.t else { return };
		let gi = self.gi_on(t, gi_level);
		let ao = ao_level > 0;
		let (w, h) = t.size;
		let (hw, hh) = (w.div_ceil(2) as f32, h.div_ceil(2) as f32);
		let (ao_taps, ao_reach) = ao_quality(ao_level);
		let mut u = PostU {
			size: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
			half: [hw, hh, 1.0 / hw, if view_ao() { -1.0 } else { 1.0 / hh }],
			proj: [cam.tan_half * aspect, cam.tan_half, RETAIL_NEAR, far],
			params: [if ao { AO_STRENGTH } else { 0.0 }, BLOOM_STRENGTH, SHARPEN, BLOOM_THRESHOLD],
			grade: [CONTRAST, SATURATION, 0.0, AO_RADIUS],
			extra: match glow {
				true => [GLOW_GAIN, fx.n as f32, seconds(), if super::partfx::FX_ON && part_level >= 2 { SHIMMER } else { 0.0 }],
				false => [0.0; 4],
			},
			sun: sun.map_or([0.0; 4], |(uv, s)| [uv[0], uv[1], s * RAYS, 0.0]),
			cam: {
				let f = forward(cam);
				let r = normalise(cross(f, cam.up));
				let u = cross(r, f);
				let (tx, ty) = (cam.tan_half * aspect, cam.tan_half);
				[[cam.pos[0], cam.pos[1], cam.pos[2], 0.0], [f[0], f[1], f[2], 0.0], [r[0] * tx, r[1] * tx, r[2] * tx, 0.0], [u[0] * ty, u[1] * ty, u[2] * ty, 0.0]]
			},
			fxl: fx.rows(),
			q: [ao_taps, ao_reach, sun_taps(sun_level), 0.0],
			gi: [0.0; 4],
			gi2: [0.0; 4],
			gi3: [0.0; 4],
		};
		if gi.is_some() {
			let (taps, reach, cells, div) = gi_quality(gi_level);
			u.gi = [GI_STRENGTH, reach, taps, if self.gi_water { 1.0 } else { 0.0 }];
			u.gi2 = [GI_ALBEDO, GI_FLOOR, cells * h as f32 / 720.0, (div / 2) as f32];
			u.gi3 = [GI_BLUR_TOL, GI_CAP, 0.0, 0.0];
		}
		queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));
		u.grade[2] = 1.0;
		queue.write_buffer(&self.ubuf_v, 0, bytemuck::bytes_of(&u));
		let run = |enc: &mut wgpu::CommandEncoder, label: &str, target: &wgpu::TextureView, pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup, load: wgpu::LoadOp<wgpu::Color>| {
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some(label),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: target, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load, store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_pipeline(pipeline);
			pass.set_bind_group(0, group, &[]);
			pass.draw(0..3, 0..1);
		};
		let clear = wgpu::LoadOp::Clear(wgpu::Color::BLACK);
		run(enc, "post depth", &t.lin, &self.p_lin, &t.g_lin, clear);
		if ao {
			run(enc, "post ao", &t.ao[0], &self.p_ao, &t.g_ao, clear);
			run(enc, "post ao across", &t.ao[1], &self.p_blur, &t.g_blur[0], clear);
			run(enc, "post ao down", &t.ao[0], &self.p_blur, &t.g_blur[1], clear);
		}
		mid(enc, &t.scene, ao.then_some(&t.ao[0]));
		if let Some(g) = gi {
			// The grid's cells (colour and normal at once), the gather, its blur across and down.
			let at = |view| Some(wgpu::RenderPassColorAttachment { view, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: clear, store: wgpu::StoreOp::Store } });
			let pass = |enc: &mut wgpu::CommandEncoder, label: &str, targets: &[Option<wgpu::RenderPassColorAttachment>], pipeline: &wgpu::RenderPipeline, group: &wgpu::BindGroup| {
				let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some(label), color_attachments: targets, depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None });
				pass.set_pipeline(pipeline);
				pass.set_bind_group(0, group, &[]);
				pass.draw(0..3, 0..1);
			};
			pass(enc, "post gi cells", &[at(&g.src[0]), at(&g.src[1])], &self.p_gsrc, &g.g_src);
			pass(enc, "post gi", &[at(&g.gi[0])], &self.p_gi, &g.g_gi);
			pass(enc, "post gi across", &[at(&g.gi[1])], &self.p_giblur, &g.g_blur[0]);
			pass(enc, "post gi down", &[at(&g.gi[0])], &self.p_giblur, &g.g_blur[1]);
		}
		run(enc, "post bright", &t.bloom[0], &self.p_bright, &t.g_bright, clear);
		for m in 1..LEVELS {
			run(enc, "post bloom down", &t.bloom[m], &self.p_down, &t.g_down[m - 1], clear);
		}
		for m in (1..LEVELS).rev() {
			run(enc, "post bloom up", &t.bloom[m - 1], &self.p_up, &t.g_up[m - 1], wgpu::LoadOp::Load);
		}
		if sun.is_some() {
			run(enc, "post ray mask", &t.rays[0], &self.p_rmask, &t.g_rmask, clear);
			run(enc, "post rays", &t.rays[1], &self.p_rays, &t.g_rays, clear);
		}
	}

	/// The graded picture into every sample of the MSAA target (`msaa`), stored for the HUD pass to load (`gi_level` as for `encode`).
	pub(super) fn composite(&self, enc: &mut wgpu::CommandEncoder, msaa: &wgpu::TextureView, gi_level: u8) {
		let Some(t) = &self.t else { return };
		let group = match self.gi_on(t, gi_level) {
			Some(g) => match (&g.g_comp_w, self.gi_water) {
				(Some((_, _, w)), true) => w,
				_ => &g.g_comp,
			},
			None => &t.g_comp,
		};
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("post composite"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: msaa, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.p_comp);
		pass.set_bind_group(0, group, &[]);
		pass.draw(0..3, 0..1);
	}

	/// The grid targets when global illumination runs this frame (level `gi_level`, made by `prepare` for these targets).
	fn gi_on(&self, t: &Targets, gi_level: u8) -> Option<&GiTargets> {
		self.gi.as_ref().filter(|g| gi_level > 0 && g.key.3 == t.generation && g.key.2 == gi_quality(gi_level).3)
	}
}
