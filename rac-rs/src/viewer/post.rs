//! [PC] Enhanced image (Pause -> Options -> Graphics -> Lighting Enhanced, retail/pcext.rs `graphics`; no retail
//! counterpart). With Lighting Classic nothing here runs and the frame is drawn exactly as before.
//!
//! The level pass resolves its MSAA colour into `scene` instead of the window; then, before any HUD or menu is drawn:
//! - `fs_lin`: the closest of each 2x2 block of the depth target (sample 0) as linear view depth, half size;
//! - `fs_ao` (Ambient occlusion On): screen-space ambient occlusion from that depth (normals from the depth, 12 samples on a
//!   rotated spiral within 1 world unit, distance falloff), then a depth-aware blur across and down;
//! - Water Enhanced (waterfx.rs): the water pixels shaded over the scene;
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
struct P { size: vec4<f32>, half: vec4<f32>, proj: vec4<f32>, params: vec4<f32>, grade: vec4<f32>, extra: vec4<f32>, sun: vec4<f32>, cam: array<vec4<f32>, 4>, fxl: array<vec4<f32>, 16> };
@group(0) @binding(0) var<uniform> p: P;
@group(0) @binding(1) var scene: texture_2d<f32>;
@group(0) @binding(2) var depth_ms: texture_depth_multisampled_2d;
@group(0) @binding(3) var lin: texture_2d<f32>;
@group(0) @binding(4) var ao_in: texture_2d<f32>;
@group(0) @binding(5) var bloom_in: texture_2d<f32>;
@group(0) @binding(6) var lsamp: sampler;
@group(0) @binding(7) var glow_in: texture_2d<f32>;
@group(0) @binding(8) var rays_in: texture_2d<f32>;

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
	let rad = clamp(R * ppu, 3.0, 48.0);
	let rot = ign(v.pos.xy) * 6.2831853;
	var occ = 0.0;
	let N = 12;
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
	let n = 48;
	let delta = (v.uv - p.sun.xy) * (0.85 / f32(n));
	var uv = v.uv - delta * ign(v.pos.xy);
	var acc = vec3<f32>(0.0);
	var w = 1.0;
	for (var i = 0; i < n; i++) {
		acc += textureSampleLevel(bloom_in, lsamp, uv, 0.0).rgb * w;
		w *= 0.955;
		uv -= delta;
	}
	return vec4<f32>(acc / 14.0, 1.0);
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
	/// A 1x1 placeholder for bindings a pass does not read.
	dummy: wgpu::TextureView,
	dummy_r32: wgpu::TextureView,
	format: wgpu::TextureFormat,
	t: Option<Targets>,
}

const LIN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Float;
const AO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;
const BLOOM_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

impl Post {
	pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat) -> Post {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("post"), source: wgpu::ShaderSource::Wgsl(POST_SHADER.into()) });
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
			layout,
			ubuf: ubuf("post uniforms"),
			ubuf_v: ubuf("post uniforms (blur down)"),
			sampler: device.create_sampler(&wgpu::SamplerDescriptor { label: Some("post linear"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, ..Default::default() }),
			dummy: small(BLOOM_FORMAT),
			dummy_r32: small(LIN_FORMAT),
			format,
			t: None,
		}
	}

	/// Makes the targets for a `w` x `h` frame (created or resized as the window is); `depth` is the level pass's MSAA depth target.
	pub(super) fn prepare(&mut self, device: &wgpu::Device, w: u32, h: u32, depth: &wgpu::TextureView) {
		if self.t.as_ref().is_none_or(|t| t.size != (w, h)) {
			self.t = Some(self.targets(device, w, h, depth));
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
		let group_x = |buf: &wgpu::Buffer, scene_v: &wgpu::TextureView, lin_v: &wgpu::TextureView, ao_v: &wgpu::TextureView, bloom_v: &wgpu::TextureView, glow_v: &wgpu::TextureView, rays_v: &wgpu::TextureView| {
			device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("post"),
				layout: &self.layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(scene_v) },
					wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&depth_only) },
					wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(lin_v) },
					wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(ao_v) },
					wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(bloom_v) },
					wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::Sampler(&self.sampler) },
					wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(glow_v) },
					wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(rays_v) },
				],
			})
		};
		let group_g = |buf: &wgpu::Buffer, scene_v: &wgpu::TextureView, lin_v: &wgpu::TextureView, ao_v: &wgpu::TextureView, bloom_v: &wgpu::TextureView, glow_v: &wgpu::TextureView| group_x(buf, scene_v, lin_v, ao_v, bloom_v, glow_v, &self.dummy);
		let group = |buf: &wgpu::Buffer, scene_v: &wgpu::TextureView, lin_v: &wgpu::TextureView, ao_v: &wgpu::TextureView, bloom_v: &wgpu::TextureView| group_g(buf, scene_v, lin_v, ao_v, bloom_v, &self.dummy);
		let (d, d32) = (&self.dummy, &self.dummy_r32);
		Targets {
			g_lin: group(&self.ubuf, &scene, d32, d32, d),
			g_ao: group(&self.ubuf, &scene, &lin, d32, d),
			g_blur: [group(&self.ubuf, &scene, &lin, &ao[0], d), group(&self.ubuf_v, &scene, &lin, &ao[1], d)],
			g_bright: group_g(&self.ubuf, &scene, &lin, d32, d, &glow),
			g_down: (1..LEVELS).map(|m| group(&self.ubuf, &scene, &lin, d32, &bloom[m - 1])).collect(),
			g_up: (1..LEVELS).map(|m| group(&self.ubuf, &scene, &lin, d32, &bloom[m])).collect(),
			g_comp: group_x(&self.ubuf, &scene, &lin, &ao[0], &bloom[0], &glow, &rays[1]),
			g_rmask: group(&self.ubuf, &scene, &lin, d32, d),
			g_rays: group(&self.ubuf, &scene, &lin, d32, &rays[0]),
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
		}
	}

	/// After the level pass: the half-size depth, the occlusion (when on), `mid` (the water pass: it may draw into the scene),
	/// the bloom chain (with the particle glow when `glow`) and the sun rays (`sun` = the key light's screen uv and strength).
	/// `cam` / `aspect` are the level pass's projection.
	#[allow(clippy::too_many_arguments)]
	/// `fx`: the effect lights (with `glow`).
	pub(super) fn encode(&self, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, cam: &Camera, aspect: f32, ao: bool, glow: bool, fx: &super::partfx::FxLights, sun: Option<([f32; 2], f32)>, mid: impl FnOnce(&mut wgpu::CommandEncoder, &wgpu::TextureView)) {
		let Some(t) = &self.t else { return };
		let (w, h) = t.size;
		let (hw, hh) = (w.div_ceil(2) as f32, h.div_ceil(2) as f32);
		let mut u = PostU {
			size: [w as f32, h as f32, 1.0 / w as f32, 1.0 / h as f32],
			half: [hw, hh, 1.0 / hw, if view_ao() { -1.0 } else { 1.0 / hh }],
			proj: [cam.tan_half * aspect, cam.tan_half, RETAIL_NEAR, RETAIL_FAR],
			params: [if ao { AO_STRENGTH } else { 0.0 }, BLOOM_STRENGTH, SHARPEN, BLOOM_THRESHOLD],
			grade: [CONTRAST, SATURATION, 0.0, AO_RADIUS],
			extra: match glow {
				true => [GLOW_GAIN, fx.n as f32, seconds(), if super::partfx::FX_ON { SHIMMER } else { 0.0 }],
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
		};
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
		mid(enc, &t.scene);
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

	/// The graded picture into every sample of the MSAA target (`msaa`), stored for the HUD pass to load.
	pub(super) fn composite(&self, enc: &mut wgpu::CommandEncoder, msaa: &wgpu::TextureView) {
		let Some(t) = &self.t else { return };
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("post composite"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: msaa, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.p_comp);
		pass.set_bind_group(0, &t.g_comp, &[]);
		pass.draw(0..3, 0..1);
	}
}
