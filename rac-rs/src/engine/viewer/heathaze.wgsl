// [PC] Heat haze (heathaze.rs): the scene inside the sources' screen rect, moved by a small rising noise as strong as the heat
// the pixel's view ray crosses before it hits the surface. One full-screen triangle, scissored to the rect; the source is a copy of
// the scene (`src`), the depth the half-size linear view depth of the post chain (`lin`, post.rs `fs_lin`; sky = the far plane).
// FXC-safe: no local arrays, no texture reads or textureDimensions inside the loops (only uniform array reads).
// Second target: FSR 3's reactive input (heathaze.rs `take_mask`, upscale/ffx_reactive.wgsl): the heat of the pixel, 0 elsewhere.

struct U {
	// (width, height, 1 / width, 1 / height)
	size: vec4<f32>,
	// eye, forward, right x horizontal tan, up x vertical tan (world space, Z up)
	cam: array<vec4<f32>, 4>,
	// (seconds into the fire clock's period (firefx_clock.rs PERIOD), largest offset in pixels, fire columns, lava boxes)
	prm: vec4<f32>,
	// (distance where the far fade starts, where the haze is gone, eye distance with no haze, eye distance with the full haze)
	fade: vec4<f32>,
	// per column: (x, y, foot z, radius), (height, gain, -, -)
	cols: array<vec4<f32>, 32>,
	// per lava box: (min x, min y, max x, max y), (top, hot layer height, path length for full heat, gain)
	lava: array<vec4<f32>, 8>,
};

@group(0) @binding(0) var<uniform> u: U;
@group(0) @binding(1) var src: texture_2d<f32>;
@group(0) @binding(2) var lin: texture_2d<f32>;
@group(0) @binding(3) var samp: sampler;

struct VO {
	@builtin(position) pos: vec4<f32>,
};

struct FO {
	@location(0) c: vec4<f32>,
	@location(1) heat: vec4<f32>,
};

// The wobble's angular speeds: whole cycles per PERIOD (firefx_clock.rs; heathaze.rs `clock` wraps there), so the pattern just
// before the wrap is the one just after it (review B1). Near 5.5, 1.1, 9.0, 4.3 and 1.7 rad/s.
const PERIOD: f32 = 512.0;
const W_STEP: f32 = 6.2831853 / PERIOD;
const W_RISE: f32 = 448.0 * W_STEP;
const W_SWAY: f32 = 90.0 * W_STEP;
const W_FAST: f32 = 733.0 * W_STEP;
const W_SIDE: f32 = 350.0 * W_STEP;
const W_SLOW: f32 = 139.0 * W_STEP;

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> VO {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	var o: VO;
	o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
	return o;
}

fn depth_at(c: vec2<i32>, hs: vec2<i32>) -> f32 {
	return textureLoad(lin, clamp(c / 2, vec2<i32>(0), hs - vec2<i32>(1)), 0).r;
}

// Far from the eye the haze fades out (u.fade.x .. u.fade.y); right at the eye too (u.fade.z .. u.fade.w): hot air the camera
// stands in must not wobble the whole picture.
fn fade(dist: f32) -> f32 {
	return (1.0 - smoothstep(u.fade.x, u.fade.y, dist)) * smoothstep(u.fade.z, u.fade.w, dist);
}

// The heat of column `a`/`b` at ray depth `t` (view depth units; `dl` = world length of one unit).
fn col_at(a: vec4<f32>, b: vec4<f32>, eye: vec3<f32>, d: vec3<f32>, t: f32, dl: f32) -> f32 {
	let q = eye + d * t;
	let r = length(q.xy - a.xy) / a.w;
	let h = (q.z - a.z) / b.x;
	if (r >= 1.0 || h <= 0.0 || h >= 1.0) { return 0.0; }
	let rad = 1.0 - r * r;
	let up = 1.0 - h;
	return rad * rad * smoothstep(0.0, 0.2, h) * up * up * b.y * fade(t * dl);
}

// Column `k`: the ray (up to the surface depth `z`) sampled where it passes closest to the column's axis and where it crosses a
// third of its height; the larger heat counts. A surface in front of the column ends the ray first: no heat.
fn col_heat(k: i32, eye: vec3<f32>, d: vec3<f32>, z: f32, dl: f32) -> f32 {
	let a = u.cols[2 * k];
	let b = u.cols[2 * k + 1];
	let dd = dot(d.xy, d.xy);
	var t1 = z;
	if (dd > 1e-8) { t1 = clamp(-dot(eye.xy - a.xy, d.xy) / dd, 0.0, z); }
	var t2 = t1;
	if (abs(d.z) > 1e-5) { t2 = clamp((a.z + 0.33 * b.x - eye.z) / d.z, 0.0, z); }
	return max(col_at(a, b, eye, d, t1, dl), col_at(a, b, eye, d, t2, dl));
}

// Lava box `k`: the length of the ray (up to the surface) inside the hot layer over the lava, weighted towards the lava.
fn lava_heat(k: i32, eye: vec3<f32>, d: vec3<f32>, z: f32, dl: f32) -> f32 {
	let a = u.lava[2 * k];
	let b = u.lava[2 * k + 1];
	let lo = vec3<f32>(a.x, a.y, b.x - 0.05);
	let hi = vec3<f32>(a.z, a.w, b.x + b.y);
	let safe = select(d, vec3<f32>(1e-6), abs(d) < vec3<f32>(1e-6));
	let t0 = (lo - eye) / safe;
	let t1 = (hi - eye) / safe;
	let tn = min(t0, t1);
	let tf = max(t0, t1);
	let ta = max(max(max(tn.x, tn.y), tn.z), 0.0);
	let tb = min(min(min(tf.x, tf.y), tf.z), z);
	if (tb <= ta) { return 0.0; }
	let low = min(eye.z + d.z * ta, eye.z + d.z * tb);
	let hl = clamp((low - b.x) / b.y, 0.0, 1.0);
	return clamp((tb - ta) * dl / b.z, 0.0, 1.0) * (1.0 - hl) * b.w * fade(ta * dl);
}

@fragment fn fs_haze(v: VO) -> FO {
	let c0 = vec2<i32>(v.pos.xy);
	let here = textureLoad(src, c0, 0);
	let hs = vec2<i32>(textureDimensions(lin));
	let uv = v.pos.xy * u.size.zw;
	let ndc = vec2<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0);
	let eye = u.cam[0].xyz;
	let d = u.cam[1].xyz + u.cam[2].xyz * ndc.x + u.cam[3].xyz * ndc.y;
	let dl = length(d);
	let z = depth_at(c0, hs);
	var heat = 0.0;
	let nc = i32(u.prm.z);
	for (var k = 0; k < nc; k++) {
		heat += col_heat(k, eye, d, z, dl);
	}
	let nl = i32(u.prm.w);
	for (var k = 0; k < nl; k++) {
		heat += lava_heat(k, eye, d, z, dl);
	}
	heat = min(heat, 1.0);
	if (heat < 0.002) { return FO(here, vec4<f32>(0.0)); }
	// Rising wobble in pixels at 1080 lines (the pattern keeps its look at any window size).
	let t = u.prm.x;
	let pp = v.pos.xy * (1080.0 / u.size.y);
	let n = vec2<f32>(
		sin(pp.y * 0.13 + t * W_RISE + sin(pp.x * 0.05 + t * W_SWAY) * 2.2) + 0.5 * sin(pp.y * 0.29 + pp.x * 0.07 + t * W_FAST),
		0.6 * cos(pp.x * 0.09 + pp.y * 0.06 + t * W_SIDE + sin(pp.y * 0.041 - t * W_SLOW) * 1.8));
	let off = n * (heat * u.prm.y / 1.5);
	// Never pull a pixel of something in front (closer) into the hazed one.
	let zs = depth_at(vec2<i32>(v.pos.xy + off), hs);
	let hv = vec4<f32>(heat, 0.0, 0.0, 1.0);
	if (zs < z - max(0.3, 0.08 * z)) { return FO(here, hv); }
	let c = textureSampleLevel(src, samp, (v.pos.xy + off) * u.size.zw, 0.0);
	return FO(vec4<f32>(c.rgb, here.a), hv);
}
