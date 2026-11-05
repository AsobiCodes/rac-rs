// [PC] Fire, ground fire after explosions (firefx_blast_gpu.rs): a scorch mark with a bed of embers, drawn as a decal into the depth the
// level wrote (group 2). The world point of each pixel is rebuilt from that depth, so the mark follows slopes and steps with no geometry
// of its own and does not paint walls, mobys or the hero. Group 0 (the level's block) is not used: the screen rectangle of each patch
// comes from the CPU (firefx_blast_gpu.rs `screen_rect`), the decal's block is at group 1 and the soft-particle block of partfx.rs at
// group 2 (the same bind group the particle pass sets).

// head: (patch count, seconds inside PERIOD, scorch darkness, clock blend)
// p: four rows per patch k: (centre xyz, radius), (normal xyz, seed), (flame, scorch, 0, 0), (screen rectangle lo.x, lo.y, hi.x, hi.y in NDC)
// inv0..inv2: the inverse of the x, y, w rows of view_proj (xyz) with their offsets in .w   hero: (feet xyz, protect radius or 0)
// tune: (glow, plane tolerance, 0, 0)
struct GU { head: vec4<f32>, p: array<vec4<f32>, 64>, inv0: vec4<f32>, inv1: vec4<f32>, inv2: vec4<f32>, hero: vec4<f32>, tune: vec4<f32> };
@group(1) @binding(0) var<uniform> g: GU;

struct SoftU { p: vec4<f32>, n: vec4<f32>, l: array<vec4<f32>, 16> };
@group(2) @binding(0) var<uniform> soft: SoftU;
@group(2) @binding(1) var soft_depth: texture_depth_multisampled_2d;

const PERIOD: f32 = 512.0;

// ---- noise: a copy of firefx.wgsl (a test compares the two blocks) ----
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
// ---- end of the noise copy ----

// Heat 0..1 -> colour: dark red, red, orange, yellow, white-yellow (a copy of firefx.wgsl).
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
// ---- end of the ramp copy ----

struct DIn {
	@builtin(position) pos: vec4<f32>,
	@location(0) @interpolate(flat) idx: u32,
};

// One instance per patch: two triangles over the patch's screen rectangle (made on the CPU; a patch off screen has no instance).
@vertex fn vs_decal(@builtin(vertex_index) vi: u32, @builtin(instance_index) ii: u32) -> DIn {
	var o: DIn;
	o.idx = ii;
	let rect = g.p[ii * 4u + 3u];
	var cx = array<f32, 6>(0.0, 1.0, 1.0, 0.0, 1.0, 0.0);
	var cy = array<f32, 6>(0.0, 0.0, 1.0, 0.0, 1.0, 1.0);
	o.pos = vec4<f32>(mix(rect.x, rect.z, cx[vi]), mix(rect.y, rect.w, cy[vi]), 0.5, 1.0);
	return o;
}

// The world point of pixel `ip` and its view depth w in .w (-1 = sky), rebuilt from the level's depth.
fn world_at(ip: vec2<i32>, dims: vec2<f32>) -> vec4<f32> {
	let zs = textureLoad(soft_depth, ip, 0);
	let n = soft.p.x;
	let f = soft.p.y;
	let w = n * f / (f - zs * (f - n));
	let xn = (f32(ip.x) + 0.5) / dims.x * 2.0 - 1.0;
	let yn = 1.0 - (f32(ip.y) + 0.5) / dims.y * 2.0;
	let v = vec3<f32>(xn * w - g.inv0.w, yn * w - g.inv1.w, w - g.inv2.w);
	return vec4<f32>(dot(g.inv0.xyz, v), dot(g.inv1.xyz, v), dot(g.inv2.xyz, v), select(w, -1.0, zs >= 0.99999));
}

// The embers: two layers of noise drifting against each other, so the bed churns instead of sliding (0..1).
fn embers_at(q: vec2<f32>, seed: u32, t: f32) -> f32 {
	let a = fbm(q * 2.2 + vec2<f32>(t * 0.21, t * 0.13), 3, seed + 31u);
	let b = fbm(q * 3.7 + vec2<f32>(-t * 0.17, t * 0.29), 3, seed + 47u);
	return smoothstep(0.30, 0.78, a * 0.55 + b * 0.55);
}

fn embers(q: vec2<f32>, seed: u32) -> f32 {
	var h = embers_at(q, seed, g.head.y);
	if (g.head.w > 0.0) {
		h = mix(h, embers_at(q, seed, g.head.y - PERIOD), g.head.w);
	}
	return h;
}

struct GOut { @location(0) c: vec4<f32>, @location(1) glow: vec4<f32> };

@fragment fn fs_decal(i: DIn) -> GOut {
	let k = i.idx * 4u;
	let pa = g.p[k];
	let pb = g.p[k + 1u];
	let pc = g.p[k + 2u];
	let r0 = pa.w;
	let n0 = normalize(pb.xyz);
	let seed = u32(pb.w * 4096.0);
	let ip = vec2<i32>(i.pos.xy);
	let dims = vec2<f32>(textureDimensions(soft_depth));
	let here = world_at(ip, dims);
	if (here.w < 0.0) {
		discard;
	}
	let d = here.xyz - pa.xyz;
	let h = dot(d, n0);
	let tol = g.tune.y * (0.3 + 0.12 * r0);
	if (abs(h) > 1.6 * tol) {
		discard;
	}
	// Far outside the mark nothing is visible whatever the ragged edge does (it moves r by 0.35 at most and the mark is gone at r = 1):
	// leave before the noise.
	let len = length(d - n0 * h) / r0;
	if (len > 1.36) {
		discard;
	}
	// A ragged edge: world-metre noise on the radius.
	let wob = (fbm(here.xy * 1.3 + vec2<f32>(pb.w * 40.0, 3.0), 2, seed) - 0.5) * 0.7;
	let r = len + wob;
	if (r > 1.2) {
		discard;
	}
	// The face under the pixel from its neighbours' points: it must agree with the patch's plane (walls, crate sides, the hero fall out).
	let lim = dims - vec2<f32>(1.0, 1.0);
	let px = world_at(vec2<i32>(min(vec2<f32>(ip) + vec2<f32>(1.0, 0.0), lim)), dims);
	let py = world_at(vec2<i32>(min(vec2<f32>(ip) + vec2<f32>(0.0, 1.0), lim)), dims);
	var agree = 0.0;
	if (px.w > 0.0 && py.w > 0.0 && abs(px.w - here.w) < 0.12 * here.w && abs(py.w - here.w) < 0.12 * here.w) {
		let cr = cross(px.xyz - here.xyz, py.xyz - here.xyz);
		if (dot(cr, cr) > 1.0e-12) {
			agree = abs(dot(normalize(cr), n0));
		}
	}
	var mask = smoothstep(0.55, 0.85, agree) * (1.0 - smoothstep(tol, 1.6 * tol, abs(h)));
	// The hero is in the sampled depth: nothing is painted on it (the ground at its feet is).
	if (g.hero.w > 0.0) {
		let dh = here.z - g.hero.z;
		let dxy = length(here.xy - g.hero.xy);
		mask = mask * (1.0 - smoothstep(0.06, 0.2, dh) * (1.0 - smoothstep(0.85 * g.hero.w, 1.2 * g.hero.w, dxy)));
	}
	if (mask <= 0.002) {
		discard;
	}
	let flame = pc.x;
	let scorch = pc.y;
	// The scorch: near black, dense in the middle, ragged and uneven.
	let edge = 1.0 - smoothstep(0.55, 1.0, r);
	let patchy = 0.55 + 0.45 * fbm(here.xy * 1.7 + vec2<f32>(pb.w * 17.0, 5.0), 3, seed + 5u);
	let a_s = clamp(scorch * mask * edge * patchy * g.head.z, 0.0, 1.0);
	// The embers: hottest in the middle, dying with the flame.
	let cen = 1.0 - smoothstep(0.1, 1.0, r);
	let heat = clamp(embers(here.xy, seed) * (0.4 + 0.8 * cen), 0.0, 1.0) * flame;
	let emis = ramp(heat * 0.9) * heat * 1.5 * g.tune.x * mask * (1.0 - smoothstep(0.7, 1.0, r));
	let scorch_rgb = vec3<f32>(0.02, 0.016, 0.012);
	var o: GOut;
	o.c = vec4<f32>(scorch_rgb * a_s + emis, a_s);
	o.glow = vec4<f32>(clamp(emis * soft.p.w * 0.6, vec3<f32>(0.0), vec3<f32>(1.0)), 0.0);
	return o;
}
