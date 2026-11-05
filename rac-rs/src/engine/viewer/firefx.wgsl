// [PC] Fire (firefx.rs): procedural flames. Prefix of the level shader's struct U (view_proj first), the fire's own uniforms at group 1
// and the soft-particle block of partfx.rs at group 2 (the same bind group the particle pass sets).
struct U { view_proj: mat4x4<f32>, sky_view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;

// t: (seconds, noise octaves, embers per emitter, ember size)   k: (jet flow m/s, turbulence, core, glow)
// m: (retail tint mix, smoke, ember brightness, clock blend)   right / up: camera axes, w = (jet cooling, jet warp)   e: emitters, two rows each:
// (base xyz, flame height), (seed, width, strength, 0)
struct FireU { t: vec4<f32>, k: vec4<f32>, m: vec4<f32>, right: vec4<f32>, up: vec4<f32>, e: array<vec4<f32>, 256> };
@group(1) @binding(0) var<uniform> fu: FireU;

struct SoftU { p: vec4<f32>, n: vec4<f32>, l: array<vec4<f32>, 16> };
@group(2) @binding(0) var<uniform> soft: SoftU;
@group(2) @binding(1) var soft_depth: texture_depth_multisampled_2d;

// The depth rebuilt from w (viewer.rs world_clip): the fire's depth must match the level's.
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

// Depth fade where the fire meets a wall or the ground (partfx.rs; soft.p = near, far, fade, glow).
fn soft_fade(pos: vec4<f32>) -> f32 {
	let n = soft.p.x;
	let f = soft.p.y;
	let zs = textureLoad(soft_depth, vec2<i32>(pos.xy), 0);
	let ls = n * f / (f - zs * (f - n));
	let lf = n * f / (f - pos.z * (f - n));
	return clamp((ls - lf) / soft.p.z, 0.0, 1.0);
}

// ---- the clock (firefx_clock.rs): fu.t.x = seconds inside PERIOD; the last seconds blend (fu.m.w) the pattern of t - PERIOD in ----
const PERIOD: f32 = 512.0;
const TAU: f32 = 6.2831853;

// ---- noise: integer hash, so the pattern never repeats within a period ----
fn hash11(n: u32) -> u32 {
	let s = n * 747796405u + 2891336453u;
	let w = ((s >> ((s >> 28u) + 4u)) ^ s) * 277803737u;
	return (w >> 22u) ^ w;
}

fn hash2(p: vec2<i32>, seed: u32) -> f32 {
	let h = hash11(u32(p.x) * 1973u + u32(p.y) * 9277u + seed * 26699u + 911u);
	return f32(h) * (1.0 / 4294967295.0);
}

fn vnoise(p: vec2<f32>, seed: u32) -> f32 {
	let i = floor(p);
	let f = fract(p);
	let w = f * f * (3.0 - 2.0 * f);
	let ip = vec2<i32>(i);
	let a = hash2(ip, seed);
	let b = hash2(ip + vec2<i32>(1, 0), seed);
	let c = hash2(ip + vec2<i32>(0, 1), seed);
	let d = hash2(ip + vec2<i32>(1, 1), seed);
	return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

fn fbm(p: vec2<f32>, oct: i32, seed: u32) -> f32 {
	var a = 0.5;
	var s = 0.0;
	var q = p;
	for (var i = 0; i < oct; i++) {
		s += a * vnoise(q, seed + u32(i) * 17u);
		q = q * 2.03 + vec2<f32>(17.1, 9.3);
		a *= 0.5;
	}
	return s / (1.0 - exp2(-f32(oct)));
}

// ---- the flame ----
struct FIn {
	@builtin(position) pos: vec4<f32>,
	@location(0) uv: vec2<f32>,
	@location(1) tint: vec3<f32>,
	@location(2) fade: f32,
	@location(3) @interpolate(flat) info: vec4<f32>,
	// A jet puff's metres: (half length, jet length, 0, 0).
	@location(4) @interpolate(flat) ext: vec4<f32>,
};

struct FireOut { @location(0) c: vec4<f32>, @location(1) glow: vec4<f32> };

@vertex fn vs_fire(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>, @location(4) l: vec4<f32>) -> FIn {
	var o: FIn;
	o.pos = world_clip(u.view_proj, p);
	o.uv = uv;
	o.tint = c.rgb;
	o.fade = c.a;
	// info: (seed, aspect or jet position 0..1, kind, jet half width)   kind: 0 level fire, 1 flame, 2 fireball, 3 jet puff, 4 spark
	o.info = vec4<f32>(l.x, l.y, l.w, l.z);
	o.ext = vec4<f32>(n, 0.0);
	return o;
}

// Heat 0..1 -> colour: dark red, red, orange, yellow, white-yellow.
fn ramp(T: f32) -> vec3<f32> {
	let a = vec3<f32>(0.30, 0.02, 0.0);
	let b = vec3<f32>(1.0, 0.10, 0.015);
	let c = vec3<f32>(1.0, 0.42, 0.05);
	let d = vec3<f32>(1.0, 0.80, 0.22);
	let e = vec3<f32>(1.0, 0.96, 0.72);
	var col = mix(a, b, smoothstep(0.0, 0.22, T));
	col = mix(col, c, smoothstep(0.2, 0.5, T));
	col = mix(col, d, smoothstep(0.45, 0.75, T));
	return mix(col, e, smoothstep(0.72, 1.0, T));
}

// A tongue of flame: x -1..1 across, h 0 (base) .. 1 (top), aspect = width / height. Returns (heat, smoke density).
fn flame_at(x: f32, h: f32, seed: f32, aspect: f32, t: f32) -> vec2<f32> {
	let oct = i32(fu.t.y);
	let sd = u32(seed * 4096.0);
	let turb = fu.k.y;
	let hc = clamp(h, 0.0, 1.0);
	// Noise space: square cells in world units; the pattern scrolls up with the heat and is warped twice by slower noise.
	let q = vec2<f32>(x * 0.5 * aspect * 2.6, hc * 2.6 - t * 1.15);
	let w1 = fbm(q * 0.9 + vec2<f32>(3.7, -t * 0.35), 2, sd + 1u);
	let w2 = fbm(q * 0.9 + vec2<f32>(-8.1 + t * 0.3, 1.7), 2, sd + 2u);
	let wq = q + (vec2<f32>(w1, w2) - 0.5) * (0.9 * turb) * (0.25 + hc * 1.2);
	let n = fbm(wq * 1.35 + vec2<f32>(0.0, -t * 0.45), oct, sd);
	let sway = (fbm(vec2<f32>(hc * 1.7 - t * 0.8, 4.0), 2, sd + 9u) - 0.5) * 0.75 * hc * turb;
	let hw = 0.9 * pow(max(1.0 - hc, 0.0), 0.62) + 0.04;
	let xs = (x - sway) / hw;
	let body = clamp(1.0 - xs * xs, 0.0, 1.0);
	let fall = 1.0 - hc;
	let hot = body * (0.45 + 0.95 * n) * (0.35 + fall);
	let T = clamp(hot * 1.15 - hc * hc * 0.65 - (1.0 - n) * 0.25 * hc, 0.0, 1.0);
	var dens = clamp(1.0 - xs * xs * 0.45, 0.0, 1.0) * (0.5 + n) * (1.0 - hc * 0.5);
	// Nothing at the quad's border: the flame and its smoke fade out before it.
	let edge = (1.0 - smoothstep(0.78, 1.0, abs(x))) * (1.0 - smoothstep(0.86, 1.0, h)) * smoothstep(-0.06, 0.03, h);
	return vec2<f32>(T * edge, dens * edge);
}

// A fireball: p -1..1 from the centre. Returns (heat, smoke density).
fn burst_at(p: vec2<f32>, seed: f32, fade: f32, t: f32) -> vec2<f32> {
	let oct = i32(fu.t.y);
	let sd = u32(seed * 4096.0);
	let turb = fu.k.y;
	let r = length(p);
	let q = p * 1.8 + vec2<f32>(seed * 13.0, -t * 1.1);
	let w = vec2<f32>(fbm(q + vec2<f32>(4.1, t * 0.5), 2, sd + 1u), fbm(q + vec2<f32>(-3.3, 2.2 - t * 0.4), 2, sd + 2u));
	let n = fbm(q + (w - 0.5) * 1.1 * turb, oct, sd);
	let rr = r + (n - 0.5) * 0.7 * turb;
	let shell = 1.0 - smoothstep(0.35, 1.0, rr);
	let edge = 1.0 - smoothstep(0.8, 1.0, r);
	let T = clamp(shell * (0.5 + 0.9 * (1.0 - r)) * (0.5 + n) * (0.35 + 0.65 * fade), 0.0, 1.0) * edge;
	let dens = (1.0 - smoothstep(0.55, 1.0, r + (n - 0.5) * 0.5)) * edge;
	return vec2<f32>(T, dens);
}

// The clock wraps every PERIOD: in its last seconds the picture eases into the one the wrap lands on, so nothing jumps.
fn flame(x: f32, h: f32, seed: f32, aspect: f32) -> vec2<f32> {
	var r = flame_at(x, h, seed, aspect, fu.t.x);
	if (fu.m.w > 0.0) {
		r = mix(r, flame_at(x, h, seed, aspect, fu.t.x - PERIOD), fu.m.w);
	}
	return r;
}

fn burst(p: vec2<f32>, seed: f32, fade: f32) -> vec2<f32> {
	var r = burst_at(p, seed, fade, fu.t.x);
	if (fu.m.w > 0.0) {
		r = mix(r, burst_at(p, seed, fade, fu.t.x - PERIOD), fu.m.w);
	}
	return r;
}

// A puff of a flame jet (the Pyrocitor's stream, firefx_jet_quads.rs): p -1..1 (x across, y along toward the tip), s = 0 (nozzle) .. 1 (tip)
// of the whole jet, W / L the puff's half width / half length (m), len the jet's length (m). The noise runs in world metres along the
// jet and flows toward the tip, so neighbouring puffs show one continuous pattern. Returns (heat, smoke density).
fn jet_at(p: vec2<f32>, seed: f32, s0: f32, W: f32, L: f32, len: f32, t: f32) -> vec2<f32> {
	let oct = i32(fu.t.y);
	let sd = u32(seed * 4096.0);
	let turb = fu.up.w;
	let s = clamp(s0 + p.y * L / max(len, 1.0), 0.0, 1.0);
	let q = vec2<f32>(p.x * W * 1.7, (s0 * len + p.y * L) * 0.85 - t * fu.k.x);
	// One domain warp (Low: oct <= 2), a second one on top from Medium.
	let w1 = vec2<f32>(fbm(q * 0.9 + vec2<f32>(3.7, -t * 0.35), 2, sd + 1u), fbm(q * 0.9 + vec2<f32>(-8.1 + t * 0.3, 1.7), 2, sd + 2u));
	var wq = q + (w1 - 0.5) * (0.8 * turb) * (0.3 + 0.9 * s);
	if (oct > 2) {
		let w2 = vec2<f32>(fbm(wq * 1.7 + vec2<f32>(1.3, -t * 0.6), 2, sd + 3u), fbm(wq * 1.7 + vec2<f32>(-5.2 + t * 0.5, 7.7), 2, sd + 4u));
		wq = wq + (w2 - 0.5) * (0.5 * turb);
	}
	let n = fbm(wq * 1.35 + vec2<f32>(0.0, -t * 0.45), oct, sd);
	let r = length(p);
	let rr = r + (n - 0.5) * 0.6 * turb;
	let shell = 1.0 - smoothstep(0.3, 1.0, rr);
	// Nothing at the quad's border: the neighbours overlap, so the chain has no seams.
	let edge = 1.0 - smoothstep(0.78, 1.0, r);
	let cool = clamp(s * fu.right.w, 0.0, 1.0);
	let T = clamp(shell * (0.3 + 0.95 * (1.0 - cool)) * (0.45 + 0.9 * n), 0.0, 1.0) * edge;
	// Smoke only toward the tips (dark red ends of the jet).
	let dens = (1.0 - smoothstep(0.55, 1.0, rr)) * edge * smoothstep(0.35, 1.0, s) * (0.5 + n);
	return vec2<f32>(T, dens);
}

// A spark of a jet: a small glowing dot, hottest in the middle. Returns (heat, smoke density).
fn spark_at(p: vec2<f32>) -> vec2<f32> {
	return vec2<f32>(pow(clamp(1.0 - length(p), 0.0, 1.0), 1.6), 0.0);
}

fn jet(p: vec2<f32>, seed: f32, s: f32, W: f32, L: f32, len: f32) -> vec2<f32> {
	var r = jet_at(p, seed, s, W, L, len, fu.t.x);
	if (fu.m.w > 0.0) {
		r = mix(r, jet_at(p, seed, s, W, L, len, fu.t.x - PERIOD), fu.m.w);
	}
	return r;
}

fn shade(T: f32, dens: f32, tint: vec3<f32>, fade: f32, pos: vec4<f32>) -> FireOut {
	let sf = soft_fade(pos);
	let tn = tint / max(max(tint.r, max(tint.g, tint.b)), 0.001);
	let col = ramp(T) * mix(vec3<f32>(1.0), tn, fu.m.x * (1.0 - T));
	let emis = col * (0.35 + 1.65 * T * fu.k.z);
	let fa = smoothstep(0.04, 0.45, T) * 0.9;
	let sa = clamp(dens * (1.0 - T) * 0.4 * fu.m.y, 0.0, 0.6);
	let v = fade * sf;
	let cover = (1.0 - (1.0 - fa) * (1.0 - sa)) * v;
	let smoke_rgb = vec3<f32>(0.05, 0.04, 0.035);
	// Premultiplied: the flame adds its light, the smoke darkens what is behind it.
	var o: FireOut;
	o.c = vec4<f32>(emis * (fa * v) + smoke_rgb * (sa * (1.0 - fa) * v), cover);
	o.glow = vec4<f32>(clamp(emis * (fa * v) * soft.p.w * fu.k.w * 0.6, vec3<f32>(0.0), vec3<f32>(1.0)), cover);
	return o;
}

@fragment fn fs_fire(i: FIn) -> FireOut {
	var r: vec2<f32>;
	if (i.info.z > 3.5) {
		r = spark_at(i.uv * 2.0 - 1.0);
	} else if (i.info.z > 2.5) {
		r = jet(i.uv * 2.0 - 1.0, i.info.x, i.info.y, i.info.w, i.ext.x, i.ext.y);
	} else if (i.info.z > 1.5) {
		r = burst(i.uv * 2.0 - 1.0, i.info.x, i.fade);
	} else {
		r = flame(i.uv.x * 2.0 - 1.0, i.uv.y, i.info.x, i.info.y);
	}
	if (r.x <= 0.002 && r.y <= 0.01) {
		discard;
	}
	// A sprite's alpha is its age / intensity: a dying flame cools as well as thins.
	var fade = i.fade;
	var T = r.x;
	if (i.info.z > 0.5) {
		T = T * (0.4 + 0.6 * clamp(fade, 0.0, 1.0));
		fade = pow(clamp(fade, 0.0, 1.0), 0.7);
	}
	return shade(T, r.y, i.tint, fade, i.pos);
}

// ---- embers: drawn from the emitters alone, no CPU particles; instance = emitter * per + ember ----
struct EIn { @builtin(position) pos: vec4<f32>, @location(0) q: vec2<f32>, @location(1) col: vec4<f32> };

@vertex fn vs_ember(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> EIn {
	var o: EIn;
	let per = u32(max(fu.t.z, 1.0));
	let ei = ii / per;
	let j = ii % per;
	let a = fu.e[2u * ei];
	let b = fu.e[2u * ei + 1u];
	let sd = u32(b.x * 4096.0) * 131u + j * 7919u;
	let r0 = hash2(vec2<i32>(i32(j), 3), sd);
	let r1 = hash2(vec2<i32>(i32(j), 5), sd);
	let r2 = hash2(vec2<i32>(i32(j), 7), sd);
	let r3 = hash2(vec2<i32>(i32(j), 11), sd);
	let life = 1.3 + 1.7 * r0;
	// Whole cycles per period (rounded) for the climb, the wobble and the flicker: seamless at the wrap, and no huge sin arguments.
	let cyc = fu.t.x * (1.0 / PERIOD);
	let age = fract(max(round(PERIOD / life), 1.0) * cyc + r1);
	let ang = r2 * 6.2831853;
	let rad = (r3 - 0.5) * b.y * 0.8;
	let hgt = a.w;
	var pos = a.xyz + vec3<f32>(cos(ang), sin(ang), 0.0) * rad;
	pos.z += hgt * (0.25 + 2.6 * pow(age, 0.85));
	let wob = sin(TAU * fract(round((2.0 + 3.0 * r1) * PERIOD / TAU) * cyc + r2 * 20.0 / TAU)) * 0.18 * hgt * age;
	pos += vec3<f32>(cos(ang + 1.6), sin(ang + 1.6), 0.0) * wob + vec3<f32>(0.35, 0.15, 0.0) * hgt * age * age * 0.6;
	let s = fu.t.w * (0.03 + 0.04 * r2) * (1.0 - 0.6 * age) * (0.6 + 0.2 * sqrt(hgt));
	var corners = array<vec2<f32>, 6>(vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0));
	let c = corners[vi];
	let world = pos + (fu.right.xyz * c.x + fu.up.xyz * c.y) * s;
	o.pos = world_clip(u.view_proj, world);
	o.q = c;
	let flick = 0.65 + 0.35 * sin(TAU * fract(round((9.0 + 8.0 * r0) * PERIOD / TAU) * cyc + r3 * 40.0 / TAU));
	let inten = (1.0 - age) * smoothstep(0.0, 0.08, age) * flick * b.z;
	o.col = vec4<f32>(mix(vec3<f32>(1.0, 0.78, 0.28), vec3<f32>(0.9, 0.16, 0.02), age), inten);
	return o;
}

@fragment fn fs_ember(i: EIn) -> FireOut {
	let d = length(i.q);
	if (d >= 1.0 || i.col.a <= 0.001) {
		discard;
	}
	let v = (1.0 - smoothstep(0.2, 1.0, d)) * i.col.a * soft_fade(i.pos);
	var o: FireOut;
	// Alpha 0: purely additive over the picture.
	o.c = vec4<f32>(i.col.rgb * v * 1.6, 0.0);
	o.glow = vec4<f32>(clamp(i.col.rgb * v * soft.p.w * fu.m.z, vec3<f32>(0.0), vec3<f32>(1.0)), 0.0);
	return o;
}
