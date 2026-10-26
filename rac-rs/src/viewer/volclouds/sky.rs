//! The WGSL of the new sky: the reduction of the captured retail sky (six cube faces, `skymodel.rs`) into the sky model map, the
//! backdrop drawn in the level pass in place of the dropped shells, and the model lookup the clouds share.
//!
//! The model map (`MAP_W` x `SECTORS + 1` texels, Rgba16Float, gamma-space colours like the scene): column = sine of the elevation
//! from -0.2 to 1, row = the azimuth (a full turn from +x, so skies that differ round the horizon keep it). Each texel is the mean of the most
//! saturated samples of the retail sky in a window around it (painted clouds and small landmarks fall out, the palette stays). The last row holds
//! single values read with textureLoad: (0) toward the sun xyz, w = 1 when the new sky draws a sun disc; (1) the sun's colour;
//! (2) the zenith, (3) the horizon, (4) the colour below the horizon. A graded level (`Look::pale`, Novalis after its 2016 look)
//! has the backdrop desaturated, tinted, lifted and hazier toward the horizon (`grade`); the lighting values are graded untinted.

/// The model lookup; `SKYTEX` / `SKYSAMP` are replaced with the module's binding names.
pub const LOOKUP: &str = r#"
const MAP_W: f32 = 48.0;
const SECTORS: f32 = 24.0;
fn sky_info(i: i32) -> vec4<f32> {
	return textureLoad(SKYTEX, vec2<i32>(i, i32(SECTORS)), 0);
}
// The model's colour (gamma space) seen along unit direction d (`sun` unused: the map holds absolute azimuths). Bilinear by hand,
// so the azimuth wraps round.
fn sky_model(d: vec3<f32>, sun: vec3<f32>) -> vec3<f32> {
	let x = (clamp(d.z, -0.2, 1.0) + 0.2) / 1.2 * (MAP_W - 1.0);
	let az = fract(atan2(d.y, d.x) / 6.2831853 + 1.0) * SECTORS - 0.5;
	let r0 = floor(az);
	let fy = az - r0;
	let y0 = (i32(r0) + i32(SECTORS)) % i32(SECTORS);
	let y1 = (y0 + 1) % i32(SECTORS);
	let x0 = i32(floor(x));
	let x1 = min(x0 + 1, i32(MAP_W) - 1);
	let fx = x - floor(x);
	let a = mix(textureLoad(SKYTEX, vec2<i32>(x0, y0), 0).rgb, textureLoad(SKYTEX, vec2<i32>(x1, y0), 0).rgb, fx);
	let b = mix(textureLoad(SKYTEX, vec2<i32>(x0, y1), 0).rgb, textureLoad(SKYTEX, vec2<i32>(x1, y1), 0).rgb, fx);
	return mix(a, b, fy);
}
// The sun disc and its glow (gamma space, added over the model) along d; strength 0 without a disc.
fn sky_sun(d: vec3<f32>, sun: vec3<f32>, col: vec3<f32>, disc: f32) -> vec3<f32> {
	let c = dot(d, sun);
	let ang = acos(clamp(c, -1.0, 1.0));
	let r = 0.012;
	let core = 1.0 - smoothstep(r * 0.85, r, ang);
	let limb = mix(0.75, 1.0, sqrt(max(1.0 - (ang / r) * (ang / r), 0.0)));
	let glow = exp(-ang * 18.0) * 0.35 + exp(-ang * 4.0) * 0.12;
	return (col * core * limb * 1.4 + col * glow) * disc;
}
"#;

/// The reduction of the six captured faces into the model map, and the dev lat-long view of the faces (`skydump.rs`).
pub const REDUCE: &str = r#"
struct R { sun: vec4<f32>, p: vec4<f32> };
@group(0) @binding(0) var<uniform> r: R;
@group(0) @binding(1) var faces: texture_2d_array<f32>;
@group(0) @binding(2) var fsamp: sampler;

struct VO { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };
@vertex fn vs_full(@builtin(vertex_index) i: u32) -> VO {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	var o: VO;
	o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
	o.uv = vec2<f32>(x, y);
	return o;
}

// The captured retail sky along direction d (the faces' cameras: skymodel.rs `FACES`).
fn cube(d: vec3<f32>) -> vec3<f32> {
	let a = abs(d);
	var k = 0;
	var fw = vec3<f32>(1.0, 0.0, 0.0);
	var rt = vec3<f32>(0.0, -1.0, 0.0);
	var up = vec3<f32>(0.0, 0.0, 1.0);
	if (a.x >= a.y && a.x >= a.z) {
		if (d.x > 0.0) { k = 0; fw = vec3<f32>(1.0, 0.0, 0.0); rt = vec3<f32>(0.0, -1.0, 0.0); }
		else { k = 1; fw = vec3<f32>(-1.0, 0.0, 0.0); rt = vec3<f32>(0.0, 1.0, 0.0); }
	} else if (a.y >= a.z) {
		if (d.y > 0.0) { k = 2; fw = vec3<f32>(0.0, 1.0, 0.0); rt = vec3<f32>(1.0, 0.0, 0.0); }
		else { k = 3; fw = vec3<f32>(0.0, -1.0, 0.0); rt = vec3<f32>(-1.0, 0.0, 0.0); }
	} else {
		up = vec3<f32>(0.0, 1.0, 0.0);
		if (d.z > 0.0) { k = 4; fw = vec3<f32>(0.0, 0.0, 1.0); rt = vec3<f32>(-1.0, 0.0, 0.0); }
		else { k = 5; fw = vec3<f32>(0.0, 0.0, -1.0); rt = vec3<f32>(1.0, 0.0, 0.0); }
	}
	let z = dot(d, fw);
	let n = vec2<f32>(dot(d, rt) / z, dot(d, up) / z);
	return textureSampleLevel(faces, fsamp, vec2<f32>(n.x * 0.5 + 0.5, 0.5 - n.y * 0.5), k, 0.0).rgb;
}

fn dir_of(s: f32, az: f32) -> vec3<f32> {
	let ce = sqrt(max(1.0 - s * s, 0.0));
	return vec3<f32>(ce * cos(az), ce * sin(az), s);
}

fn luma(c: vec3<f32>) -> f32 { return dot(c, vec3<f32>(0.299, 0.587, 0.114)); }

// The level's grade (`params::Look::pale`, r.p.w) of the model's colour c at the sine of the elevation s: softer (less saturated,
// lifted; the backdrop also a little greener, `tint`) and paler toward the horizon (haze).
fn grade(c: vec3<f32>, s: f32, tint: f32) -> vec3<f32> {
	let k = r.p.w;
	if (k <= 0.0) { return c; }
	var g = mix(vec3<f32>(luma(c)), c, 1.0 - 0.4 * k) * mix(vec3<f32>(1.0), vec3<f32>(0.97, 1.08, 0.92), k * tint);
	g += (1.0 - g) * 0.05 * k;
	let hz = k * 0.6 * (1.0 - smoothstep(0.0, 0.25, s));
	return mix(g, g + (1.0 - g) * 0.35, hz);
}

const N: i32 = 96;
// The share of samples the sky texel keeps: the most saturated ones (painted clouds are paler than the sky they sit in, so on a
// cloudy band the sky still wins), averaged.
const KEEP: f32 = 0.5;
fn score(c: vec3<f32>) -> f32 {
	let mx = max(c.r, max(c.g, c.b));
	let mn = min(c.r, min(c.g, c.b));
	// Very dark samples (the clear colour where no shell covers) count less.
	return (mx - mn) / max(mx, 0.02) * smoothstep(0.02, 0.12, mx);
}
fn robust(cols: ptr<function, array<vec3<f32>, 96>>) -> vec3<f32> {
	// The score threshold: the lowest score with more than (1 - KEEP) N samples at or below it (ties count).
	var cut = 1.0e9;
	let want = i32(f32(N) * (1.0 - KEEP));
	for (var i = 0; i < N; i++) {
		let si = score((*cols)[i]);
		var below = 0;
		for (var j = 0; j < N; j++) {
			below += select(0, 1, score((*cols)[j]) <= si);
		}
		if (below > want && si < cut) { cut = si; }
	}
	var acc = vec3<f32>(0.0);
	var w = 0.0;
	for (var i = 0; i < N; i++) {
		let c = (*cols)[i];
		let k = select(0.0, 1.0, score(c) >= cut);
		acc += c * k;
		w += k;
	}
	return acc / max(w, 1.0);
}

@fragment fn fs_reduce(v: VO) -> @location(0) vec4<f32> {
	let c = vec2<i32>(v.pos.xy);
	let sun = r.sun.xyz;
	let w = r.p.x;
	let sectors = r.p.z;
	var cols: array<vec3<f32>, 96>;
	if (f32(c.y) < sectors) {
		let s0 = -0.2 + 1.2 * f32(c.x) / (w - 1.0);
		let sec = 6.2831853 / sectors;
		let a0 = sec * (f32(c.y) + 0.5);
		let ds = 1.2 / (w - 1.0);
		for (var i = 0; i < 8; i++) {
			for (var j = 0; j < 12; j++) {
				let s = clamp(s0 + ds * (f32(i) - 3.5) * 1.2, -0.25, 0.999);
				let a = a0 + (f32(j) - 5.5) * sec * 0.32;
				cols[i * 12 + j] = cube(dir_of(s, a));
			}
		}
		return vec4<f32>(grade(robust(&cols), s0, 1.0), 1.0);
	}
	// The single values.
	if (c.x == 0) { return r.sun; }
	if (c.x == 1) {
		// The sun's colour: the brightest of the samples round the painted sun, pulled toward white a little.
		var best = vec3<f32>(0.0);
		for (var i = 0; i < 64; i++) {
			let fi = f32(i);
			let rr = 0.06 * sqrt(fi / 64.0);
			let a = fi * 2.39996;
			let up = normalize(cross(sun, vec3<f32>(0.0, 0.0, 1.0)) + vec3<f32>(1.0e-4, 0.0, 0.0));
			let side = cross(up, sun);
			let s = cube(normalize(sun + (up * cos(a) + side * sin(a)) * rr));
			if (luma(s) > luma(best)) { best = s; }
		}
		return vec4<f32>(best, 1.0);
	}
	var band = vec2<f32>(0.85, 0.999);
	if (c.x == 3) { band = vec2<f32>(0.0, 0.08); }
	if (c.x == 4) { band = vec2<f32>(-0.2, -0.04); }
	for (var i = 0; i < 8; i++) {
		for (var j = 0; j < 12; j++) {
			let s = mix(band.x, band.y, (f32(i) + 0.5) / 8.0);
			cols[i * 12 + j] = cube(dir_of(s, f32(j) * 0.5236 + 0.13));
		}
	}
	return vec4<f32>(grade(robust(&cols), (band.x + band.y) * 0.5, 0.0), 1.0);
}

// Dev: the faces as a lat-long picture (x = azimuth from +x, y = elevation from +90 to -90 degrees).
@fragment fn fs_latlong(v: VO) -> @location(0) vec4<f32> {
	let az = v.uv.x * 6.2831853;
	let el = (0.5 - v.uv.y) * 3.14159265;
	return vec4<f32>(cube(vec3<f32>(cos(el) * cos(az), cos(el) * sin(az), sin(el))), 1.0);
}
"#;

/// The backdrop: a full-screen triangle at the far plane in the level pass (after `viewer::SHADER`'s bindings: `u` in group 0,
/// the model map as `tex` / `samp` in group 1). The view ray comes from `u.sky_view_proj` (the sky's rotation-only matrix; the
/// water mirror's flipped one works the same).
pub const BACKDROP: &str = r#"
struct BO { @builtin(position) pos: vec4<f32>, @location(0) ndc: vec2<f32> };
@vertex fn vs_backdrop(@builtin(vertex_index) i: u32) -> BO {
	let x = f32((i << 1u) & 2u) * 2.0 - 1.0;
	let y = 1.0 - f32(i & 2u) * 2.0;
	var o: BO;
	o.pos = vec4<f32>(x, y, 1.0, 1.0);
	o.ndc = vec2<f32>(x, y);
	return o;
}

@fragment fn fs_backdrop(i: BO) -> @location(0) vec4<f32> {
	let m = u.sky_view_proj;
	let a = vec3<f32>(m[0][0], m[1][0], m[2][0]);
	let b = vec3<f32>(m[0][1], m[1][1], m[2][1]);
	let c = vec3<f32>(m[0][3], m[1][3], m[2][3]);
	let det = dot(a, cross(b, c));
	let d = normalize((cross(b, c) * i.ndc.x + cross(c, a) * i.ndc.y + cross(a, b)) / det);
	let s = sky_info(0);
	var col = sky_model(d, s.xyz) + sky_sun(d, s.xyz, sky_info(1).rgb, s.w);
	// Dither (the target is 8 bits per channel: no bands in the smooth gradient).
	let n = fract(52.9829189 * fract(dot(i.pos.xy, vec2<f32>(0.06711056, 0.00583715))));
	col += (n - 0.5) / 255.0;
	return vec4<f32>(col, 1.0);
}
"#;
