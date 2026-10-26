//! The water shader (WGSL, one string): the sea, wave, lava, mud, sludge and waterfall looks, `fs_pos`, `fs_water` and
//! `fs_fall`. It is a single string literal (assembled in `setup::new`, checked by the `tests` below).

pub(super) const SHADER: &str = r#"
struct WU { vp: mat4x4<f32>, eye: vec4<f32>, sun: vec4<f32>, proj: vec4<f32>, size: vec4<f32>, look: vec4<f32>, fog: vec4<f32>, mirror: vec4<f32>, flows: array<vec4<f32>, 64>, rn: vec4<f32>, rings: array<vec4<f32>, 32>, trail: array<vec4<f32>, 96>, wbox: vec4<f32>, churn: array<vec4<f32>, 24>, hero: vec4<f32>, wave: vec4<f32>, ivp: mat4x4<f32>, geo: array<vec4<f32>, 4>, gmap: vec4<f32>, ggrid: vec4<f32>, gridb: vec4<f32>, wgeo: vec4<f32>, mirror2: vec4<f32> };
@group(0) @binding(0) var<uniform> w: WU;
@group(0) @binding(1) var depth_ms: texture_depth_multisampled_2d;
@group(0) @binding(2) var scene: texture_2d<f32>;
@group(0) @binding(3) var lin: texture_2d<f32>;
@group(0) @binding(4) var wpos: texture_2d<f32>;
@group(0) @binding(5) var lsamp: sampler;
// The planar mirrors (mirror.rs; half size), one layer per mirrored plane: heights w.mirror.x, w.mirror2.x, w.mirror2.y.
@group(0) @binding(6) var mirror: texture_2d_array<f32>;
// The particle pass's glow target (partfx.rs): its alpha is the particles' cover (w.mirror.z = 1 with Particles Enhanced).
@group(0) @binding(7) var glow: texture_2d<f32>;
// The water bed (mirror.rs `draw_bed`; half size): the level without its water, and its depth.
@group(0) @binding(8) var bed_tex: texture_2d<f32>;
@group(0) @binding(9) var bed_depth: texture_depth_multisampled_2d;
// [PC] Geometric sea waves: the flat sea's height per pixel (`fs_mask`, -1e9 = none) and the shore map (`SHORE_SHADER`).
@group(0) @binding(10) var seamask: texture_2d<f32>;
@group(0) @binding(11) var<storage, read> shore: array<u32>;

// Clear shallows: absorption per world unit of path through the water (`fs_water`).
const ABSORB: f32 = 0.12;
// Cold water (`fs_water`, w.flows[COLD_SLOT].w = the look 1..3): the swell's world scale (under 1 = longer waves), pace and
// slope, and the pixel size its finest waves fade at as if far off (no fine chop); the least reflectance and how dark a
// reflection may turn the water (the dark cave above must not dim the teal); how far the swell moves the water's own
// picture; light on the swell and the shimmer; the ice colour (linear light) and its rim.
const COLD_SCALE: f32 = 0.55;
const COLD_PACE: f32 = 0.5;
const COLD_SWELL: f32 = 2.2;
const COLD_CALM: f32 = 0.12;
const COLD_SHEEN: f32 = 0.3;
const COLD_FLOOR: f32 = 0.85;
const COLD_PUSH: f32 = 1.8;
const COLD_SHADE: f32 = 1.0;
const COLD_SHIMMER: f32 = 0.6;
const COLD_ICE: vec3<f32> = vec3<f32>(0.55, 0.75, 0.80);
const COLD_RIM: f32 = 1.4;

// Far water (user 2026-10-16: "the water surfaces flicker when u move the camera, especially those more far away"): past
// FAR_FROM world units the ripples fade sooner (as if a pixel were 1 + FAR_RIPPLE times wider), the pixel footprint follows a
// grazing view further (FAR_GRAZE, was 0.2 everywhere), and the mirror / screen-space reflection is bent less by the ripples
// (FAR_BEND) and read over a wider disc (FAR_BLUR), all reaching full strength at FAR_TO. Nearer water keeps its look exactly.
// [C: flick.py on play/compare Hoven / Pokitaru / Novalis windows, 2026-10-16: with the ripples off the far flicker fell most,
// the planar mirror and the footprint clamp next.]
const FAR_FROM: f32 = 25.0;
// [PC] Turbulence at churn spots and in rapids (flow speeds RAPID_FROM..RAPID_TO units/s), and their foam.
const TURB: f32 = 1.2;
const RAPID_FROM: f32 = 3.5;
const RAPID_TO: f32 = 6.0;
const RAPID_CHOP: f32 = 0.6;
const CHURN_FOAM: f32 = 0.9;
const RAPID_FOAM: f32 = 0.3;
// [PC] Foam switched off for now (user 2026-10-21: it still flickered): the shore foam and surf where the water meets the ground
// or objects (FOAM_SHORE) and the white of wakes (FOAM_WAKE). 1.0 brings each back. The ice rim on cold water, splash rings,
// whitewater at churn spots and rapids, whitecaps and the waterfalls' boil stay.
const FOAM_SHORE: f32 = 0.0;
const FOAM_WAKE: f32 = 0.0;
// A river of effect quads (`WaterFx::quad_flows`, flows w = -3) flows in two phases FLOW_CYCLE seconds long, each restarting
// while the other one shows, so neighbouring quads of different flow drift apart by at most one cycle's travel.
const FLOW_CYCLE: f32 = 1.6;
const FAR_TO: f32 = 50.0;
const FAR_GRAZE: f32 = 0.02;
const FAR_RIPPLE: f32 = 1.5;
const FAR_BEND: f32 = 0.6;
const FAR_BLUR: f32 = 2.0;
// Open sea: the screen-space occlusion fades out from SEA_AO_FROM to SEA_AO_TO world units from the eye (`fs_ao_sea`).
const SEA_AO_FROM: f32 = 2.0;
const SEA_AO_TO: f32 = 6.0;

// w.proj = (tan_half * aspect, tan_half, near, far).
fn linear_depth(d: f32) -> f32 {
	let n = w.proj.z;
	let f = w.proj.w;
	return n * f / (f - d * (f - n));
}

struct PO { @builtin(position) pos: vec4<f32>, @location(0) world: vec3<f32>, @location(1) @interpolate(flat) flow: u32 };

// The instance index is the draw's flow slot (w.flows; 0 = still water).
@vertex fn vs_pos(@location(0) p: vec3<f32>, @builtin(instance_index) k: u32) -> PO {
	var o: PO;
	o.pos = w.vp * vec4<f32>(p, 1.0);
	o.world = p;
	o.flow = k;
	return o;
}

// The surface's world position (a = 1 + flow slot), unless the level's depth here is nearer (something stands in front of
// the water).
@fragment fn fs_pos(v: PO) -> @location(0) vec4<f32> {
	let zs = linear_depth(textureLoad(depth_ms, vec2<i32>(v.pos.xy), 0));
	let zw = linear_depth(v.pos.z);
	if (zw > zs * 1.004 + 0.03) { discard; }
	return vec4<f32>(v.world, 1.0 + f32(v.flow));
}

struct VO { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

// [PC] The open sea drawn as retail effect quads (Pokitaru; wpos.a >= 128.5: `WaterDraw::sea` plus 64): no screen-space occlusion on the sea beyond a few units. It is computed from the level's
// depth, where the retail sea (long overlapping triangles of slightly different heights, tex 22 and the layers 40 / 41)
// has creases along the triangles' intersections: the occlusion drew them as thin straight dark lines and curves (Pokitaru).
// Runs on the half-size occlusion target (blend: over, white with alpha k) before the shading.
@fragment fn fs_ao_sea(v: VO) -> @location(0) vec4<f32> {
	let c = vec2<i32>(v.pos.xy) * 2;
	var k = 0.0;
	for (var i = 0; i < 4; i++) {
		let wp = wpos_at(c + vec2<i32>(i & 1, i >> 1));
		if ((u32(wp.a - 0.5) & 128u) != 0u) { k += 0.25 * smoothstep(SEA_AO_FROM, SEA_AO_TO, length(w.eye.xyz - wp.xyz)); }
	}
	if (k <= 0.0) { discard; }
	return vec4<f32>(1.0, 1.0, 1.0, k);
}

@vertex fn vs_full(@builtin(vertex_index) i: u32) -> VO {
	let x = f32((i << 1u) & 2u);
	let y = f32(i & 2u);
	var o: VO;
	o.pos = vec4<f32>(x * 2.0 - 1.0, 1.0 - y * 2.0, 0.0, 1.0);
	o.uv = vec2<f32>(x, y);
	return o;
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
	return pow(max(c, vec3<f32>(0.0)), vec3<f32>(2.2));
}

fn luma(c: vec3<f32>) -> f32 {
	return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// [PC] An INTEGER hash of the cell floor(p): the old float hash (fract of products of fract, ~1e4 big) came out a little different
// when a compiler fused the same cell's corner as `i + 1` than as a neighbour's `i`, so the noise jumped along straight world
// lines (a 1-px seam across Pokitaru's pool, Agent 12). Same 0..1 spread, exact on every GPU.
fn hash(p: vec2<f32>) -> f32 {
	return lhash(vec2<i32>(floor(p)));
}

// Smooth value noise, 0..1.
fn vnoise(p: vec2<f32>) -> f32 {
	let i = floor(p);
	let f = p - i;
	let u = f * f * (3.0 - 2.0 * f);
	let a = hash(i);
	let b = hash(i + vec2<f32>(1.0, 0.0));
	let c = hash(i + vec2<f32>(0.0, 1.0));
	let d = hash(i + vec2<f32>(1.0, 1.0));
	return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

// Per-pixel dither 0..1 (interleaved gradient noise): staggers the reflection march between neighbours, so no step bands.
fn dither(c: vec2<f32>) -> f32 {
	return fract(52.9829189 * fract(dot(c, vec2<f32>(0.06711056, 0.00583715))));
}

// The ripple slope (d height / d x, d height / d y) at world XY `p`, time `t`: ten wave trains, each turned by the golden
// angle and 1.33 times finer than the last, on a slowly drifting warped domain (no grid, no repeat on screen). `fp` is the
// world size of one pixel there: a train finer than about 12-50 pixels fades out (not 3-8: those shimmered when the view moved).
fn ripple(p: vec2<f32>, t: f32, fp: f32) -> vec2<f32> {
	let warp = vec2<f32>(vnoise(p * 0.09 + vec2<f32>(t * 0.02, 0.0)), vnoise(p * 0.09 + vec2<f32>(5.2, 1.3 - t * 0.015))) * 3.0;
	let q = p + warp;
	var g = vec2<f32>(0.0);
	var f = 1.6;
	var a = 0.055;
	for (var k = 0; k < 10; k++) {
		let ang = f32(k) * 2.39996 + 0.4;
		let d = vec2<f32>(cos(ang), sin(ang));
		let ph = dot(d, q) * f + t * sqrt(9.8 * f) * 0.35 + f32(k) * 1.7;
		let keep = 1.0 - smoothstep(0.02, 0.08, fp * f * 0.159);
		g += d * cos(ph) * a * f * keep;
		f *= 1.33;
		a *= 0.72;
	}
	return g;
}

// Fine sparkle slope for the sun glint only (twinkles), same construction, finer and steeper.
fn sparkle_slope(p: vec2<f32>, t: f32, fp: f32) -> vec2<f32> {
	var g = vec2<f32>(0.0);
	var f = 9.0;
	for (var k = 0; k < 4; k++) {
		let ang = f32(k) * 2.39996 + 1.1;
		let d = vec2<f32>(cos(ang), sin(ang));
		let ph = dot(d, p) * f + t * (3.1 + f32(k) * 0.9);
		g += d * cos(ph) * 0.05 * (1.0 - smoothstep(0.12, 0.35, fp * f * 0.159));
		f *= 1.41;
	}
	return g;
}

// Caustic cells: the gap between the nearest and second-nearest of a jittered moving point set (bright lines on cell borders).
fn cells(p: vec2<f32>, t: f32) -> f32 {
	let i = floor(p);
	let f = p - i;
	var d1 = 9.0;
	var d2 = 9.0;
	for (var y = -1; y <= 1; y++) {
		for (var x = -1; x <= 1; x++) {
			let o = vec2<f32>(f32(x), f32(y));
			let h = vec2<f32>(hash(i + o), hash(i + o + vec2<f32>(17.3, 3.1)));
			let pt = o + 0.5 + 0.4 * sin(vec2<f32>(t * 0.7, t * 0.55) + 6.2831 * h);
			let d = length(pt - f);
			if (d < d1) {
				d2 = d1;
				d1 = d;
			} else if (d < d2) {
				d2 = d;
			}
		}
	}
	return d2 - d1;
}

// Animated caustic light on world XY `p` (mean about 0.35); `fp` = world size of a pixel (lines widen with it, then fade to their mean).
fn caustic(p: vec2<f32>, t: f32, fp: f32) -> f32 {
	let q = p * 0.75 + (vec2<f32>(vnoise(p * 0.45 + t * 0.1), vnoise(p * 0.45 + vec2<f32>(3.7, 9.1) - t * 0.1)) - 0.5) * 0.8;
	let a = 1.0 - smoothstep(0.0, sqrt(0.04 + 20.25 * fp * fp), cells(q, t));
	let b = 1.0 - smoothstep(0.0, sqrt(0.0576 + 38.07 * fp * fp), cells(q * 1.37 + vec2<f32>(4.1, 7.3), t * 1.2));
	let c = a * a * 0.6 + b * b * 0.45 + a * b * 0.9;
	return mix(c, 0.35, smoothstep(0.03, 0.16, fp));
}

// Contact rings (`Rings`; w.rn.x in use, per ring (centre, radius), (slope, width, froth, 0)): a wave packet travelling out
// from its centre on that plane only. xy = its slope (faded where a pixel spans the packet), z = its froth crest.
fn rings_at(p: vec3<f32>, fp: f32) -> vec3<f32> {
	var s = vec3<f32>(0.0);
	for (var k = 0u; k < u32(w.rn.x); k++) {
		let a = w.rings[2u * k];
		let b = w.rings[2u * k + 1u];
		let dv = p.xy - a.xy;
		let d = length(dv);
		let x = (d - a.w) / b.y;
		// [PC] A heavy splash (`Rings::splash`, b.w > 0) leaves foam inside its ring.
		if (b.w > 0.0 && abs(p.z - a.z) <= 0.3) { s.z += b.w * (1.0 - smoothstep(0.25 * a.w, 0.85 * a.w, d)); }
		if (abs(p.z - a.z) > 0.3 || abs(x) > 3.0 || d < 1e-4) { continue; }
		let env = exp(-x * x);
		let sl = b.x * (cos(2.4 * x) - 0.83 * x * sin(2.4 * x)) * env * clamp(b.y / (fp * 2.0), 0.0, 1.0);
		s += vec3<f32>(dv / d * sl, b.z * exp(-x * x * 4.0));
	}
	return s;
}

// [PC] Wakes (`Wakes`; w.rn.y points, per point (x, y, surface z, age s), (half width at the mover, 0, speed, strength when
// the next point is the same mover's, else 0); w.wbox = their box). Each stretch of a mover's path gives the distance across
// it and the age and speed there: two arms open behind the mover at Kelvin's angle (half width at the mover + 0.35 v age, at
// most 2.2 a second), the water between them is choppy and foams just behind it, and the whole wake drifts downstream with
// the water's flow (for WAKE_DRIFT s). The stretches are blended (foam: the most of them; slope: their weighted mean), so
// a turn leaves no seam where one stretch meets the next. xy = slope, z = foam.
const WAKE_LIFE: f32 = 2.6;
const WAKE_SLOPE: f32 = 1.4;
const WAKE_CHOP: f32 = 0.5;
const WAKE_FOAM: f32 = 1.0;
const WAKE_BAND: f32 = 2.0;
const WAKE_ARM_FOAM: f32 = 1.3;
// A wake drifts with the water's flow for its first WAKE_DRIFT seconds only: the flow here is the pixel's own, so on a fast
// river (Novalis' cave strips) a longer drift drew the trail far downstream of the mover in streaks, torn where the flow
// changes between strips (user 2026-10-17, going upstream).
const WAKE_DRIFT: f32 = 0.35;
fn wake_at(p: vec3<f32>, fp: f32, flow: vec2<f32>, t: f32) -> vec3<f32> {
	let n = u32(w.rn.y);
	if (n < 2u || p.x < w.wbox.x || p.y < w.wbox.y || p.x > w.wbox.z || p.y > w.wbox.w) { return vec3<f32>(0.0); }
	var sl = vec2<f32>(0.0);
	var wsum = 0.0;
	var foam = 0.0;
	var mid = 0.0;
	for (var k = 0u; k + 1u < n; k++) {
		let ab = w.trail[2u * k + 1u];
		if (ab.w <= 0.0) { continue; }
		let a = w.trail[2u * k];
		let dz = abs(p.z - a.z);
		if (dz > WAKE_BAND) { continue; }
		let b = w.trail[2u * k + 2u];
		let pa = a.xy + flow * min(a.w, WAKE_DRIFT);
		let e = b.xy + flow * min(b.w, WAKE_DRIFT) - pa;
		let el = max(dot(e, e), 1e-6);
		let tr = dot(p.xy - pa, e) / el;
		let hh = clamp(tr, 0.0, 1.0);
		let bd = p.xy - (pa + e * hh);
		let dd = dot(bd, bd);
		let reach = ab.x + max(a.w, b.w) * 2.2 + 1.5;
		if (dd > reach * reach) { continue; }
		let age = mix(a.w, b.w, hh);
		let v = clamp(mix(ab.z, w.trail[2u * k + 3u].z, hh), 0.0, 8.0);
		// A sloping river: the wake fades out above and below the mover's water instead of ending at a quad's edge.
		let str = ab.w * (1.0 - smoothstep(WAKE_BAND * 0.4, WAKE_BAND, dz)) * smoothstep(0.3, 2.0, v);
		let life = clamp(1.0 - age / WAKE_LIFE, 0.0, 1.0);
		let r = sqrt(dd);
		let half = ab.x + age * min(0.35 * v, 2.2);
		let wid = 0.16 + 0.12 * age;
		// Past a stretch's ends its wake fades out (no round caps: no wake ahead of the mover, no rings where it went slowly);
		// the next stretch carries it on.
		let over = max(max(-tr, tr - 1.0), 0.0) * sqrt(el);
		let cap = 1.0 - smoothstep(0.0, 0.2 + 0.25 * half, over);
		let fade = life * life * str * cap;
		let away = select(bd / max(r, 1e-4), vec2<f32>(0.0), r < 1e-4);
		let x = (r - half) / wid;
		let x2 = (r - half * 0.55) / (wid * 0.8);
		// A pixel wider than the arm sees it fade out instead of flickering.
		let keep = clamp(wid / (fp * 2.0), 0.0, 1.0);
		let g = exp(-x * x);
		let g2 = exp(-x2 * x2);
		let arms = (cos(2.4 * x) - 0.83 * x * sin(2.4 * x)) * g + 0.45 * (cos(2.4 * x2) - 0.83 * x2 * sin(2.4 * x2)) * g2;
		let wk = (g + g2 + 0.05) * fade * keep;
		sl += away * arms * WAKE_SLOPE * fade * keep * wk;
		wsum += wk;
		mid = max(mid, (1.0 - smoothstep(half * 0.35, half * 0.85, r)) * fade * keep);
		let core = 1.0 - smoothstep(ab.x * 0.6 + age * 0.2, ab.x * 1.6 + age * 0.45, r);
		// The foam outlives the slope: it fades with the wake's life, not its square.
		foam = max(foam, (core * exp(-age * 0.8) * WAKE_FOAM + exp(-x * x * 2.5) * WAKE_ARM_FOAM * exp(-age * 0.45)) * life * str * cap);
	}
	if (wsum <= 0.0 && foam <= 0.0) { return vec3<f32>(0.0); }
	sl = sl / max(wsum, 1e-4);
	let q = p.xy - flow * t;
	// The chop (0.32 units across) fades out where a pixel spans half of it, so it never shimmers far off.
	let cfine = 1.0 - smoothstep(0.08, 0.16, fp);
	if (mid * cfine > 0.0) {
		let chop = vec2<f32>(lnoise(q * 3.1 + vec2<f32>(t * 1.7, 0.0)), lnoise(q * 3.1 + vec2<f32>(7.3, -t * 1.7))) - 0.5;
		sl += chop * WAKE_CHOP * mid * cfine;
	}
	return vec3<f32>(sl, foam);
}

// [PC] Churn (`Wakes`; w.rn.z spots, per spot (x, y, z, radius), (strength, 0, 0, 0)): where retail's spray keeps hitting the
// water (a waterfall's foot, rapids). 0..1.
fn churn_at(p: vec3<f32>) -> f32 {
	var c = 0.0;
	for (var k = 0u; k < u32(w.rn.z); k++) {
		let a = w.churn[2u * k];
		if (abs(p.z - a.z) > 2.0) { continue; }
		c += w.churn[2u * k + 1u].x * (1.0 - smoothstep(a.w * 0.3, a.w, length(p.xy - a.xy)));
	}
	return min(c, 1.0);
}

// [PC] Long swells on open sea (`WaterDraw::sea`) three long trains (44, 27 and 17 units), slow as deep
// water waves of that length, rising and falling in broad groups. A slope only: the surface (and what stands on it) stays
// retail.
const SWELL: f32 = 0.16;
fn swell(p: vec2<f32>, t: f32) -> vec2<f32> {
	var g = vec2<f32>(0.0);
	var f = 6.2832 / 44.0;
	var a = SWELL;
	for (var k = 0; k < 3; k++) {
		let ang = 0.6 + f32(k) * 0.45;
		let d = vec2<f32>(cos(ang), sin(ang));
		let ph = dot(d, p) * f + t * sqrt(9.8 * f) * 0.6 + f32(k) * 2.1;
		g += d * cos(ph) * a;
		f *= 1.6;
		a *= 0.62;
	}
	return g * (0.55 + 0.9 * vnoise(p * 0.012 + vec2<f32>(t * 0.01, 0.0)));
}

// [PC] Sea state (Water Medium / High; w.wave.x = trains, 0 = off): layered wind waves on open sea (`sea`) and lakes. Trains of
// WAVE_L0 * scale units (each 0.63 times the last, turned a little off the wind heading WAVE_WIND, slow as deep water waves,
// rising and falling in gusts), each a sharpened `exp(sin)` crest with broad troughs that also drags the next train's phase
// sideways (choppy, no grid). Slopes only: the surface stays retail's. Returns (height 0.. about +-0.4, slope x, slope y); a
// train finer than about three pixels fades out (`fp`).
const WAVE_WIND: f32 = 0.6;
const WAVE_L0: f32 = 48.0;
const WAVE_S0: f32 = 0.2;
const WAVE_PACE: f32 = 0.5;
const WAVE_DRAG: f32 = 0.19;
// Lakes: the trains' scale (shorter) and slope; the crest shade; whitecaps (crest height from..to, strength).
const WAVE_LAKE_SCALE: f32 = 0.3;
const WAVE_LAKE_AMP: f32 = 0.5;
const WAVE_SHADE: f32 = 0.5;
const WAVE_FACE: f32 = 1.6;
// (The geometric sea's true slopes are steeper than the W2 look's: its faces less.)
const GEO_FACE: f32 = 0.9;
const WAVE_BEND: f32 = 3.0;
const WAVE_CAP_FROM: f32 = 0.20;
const WAVE_CAP_TO: f32 = 0.32;
const WAVE_CAP: f32 = 0.5;
const WAVE_CAP_LUMA: f32 = 0.12;
fn waves(p0: vec2<f32>, t: f32, fp: f32, scale: f32, n: i32) -> vec3<f32> {
	var p = p0;
	var h = 0.0;
	var hw = 0.0;
	var g = vec2<f32>(0.0);
	var len = WAVE_L0 * scale;
	var s = WAVE_S0;
	for (var k = 0; k < n; k++) {
		let kk = 6.2832 / len;
		let ang = WAVE_WIND + (fract(f32(k) * 0.618 + 0.3) - 0.5) * 2.0 * (0.25 + 0.2 * f32(k));
		let d = vec2<f32>(cos(ang), sin(ang));
		let x = dot(d, p) * kk + t * sqrt(9.8 * kk) * WAVE_PACE + f32(k) * 1.9;
		let e = exp(sin(x) - 1.0);
		let dx = e * cos(x);
		let keep = 1.0 - smoothstep(0.12, 0.35, fp * kk * 0.159);
		let gust = (0.55 + 0.9 * lnoise(p0 * (0.25 / len) + vec2<f32>(f32(k) * 13.1, t * 0.02))) * keep;
		let a = s / kk;
		g += d * (dx * 1.9 * s * gust);
		h += (e - 0.4658) * a * gust;
		hw += a;
		p += d * (dx * a * WAVE_DRAG * gust);
		len *= 0.63;
		s *= 0.82;
	}
	return vec3<f32>(h / hw, g);
}

// [PC] Whitewater: soft foam patches breaking up in three sizes, flowing with the water; far off, their mean. (lnoise: its
// integer hash keeps far-from-origin levels such as Pokitaru free of blocks.)
fn lace(q: vec2<f32>, t: f32, fp: f32) -> f32 {
	let n = lnoise(q * 1.6 + vec2<f32>(t * 0.35, -t * 0.2)) * 0.55 + lnoise(q * 4.1 - vec2<f32>(t * 0.6, 0.0)) * 0.3 + lnoise(q * 9.3 + vec2<f32>(0.0, t)) * 0.15 * (1.0 - smoothstep(0.03, 0.1, fp));
	return mix(smoothstep(0.42, 0.72, n), 0.45, smoothstep(0.08, 0.3, fp));
}

// [PC] Geometric sea waves (Water Medium / High on open sea, `vs_grid`): a real height field in world units with its exact slope.
// Train k is w.wgeo.w * GEO_RATIO^k units long and w.wgeo.x * w.wgeo.y^k steep (short trains steeper), turned off the wind heading
// WAVE_WIND, slow as deep water waves, a sharpened `exp(sin)` crest with broad troughs, in gusts (`lnoise_d`), each dragging the
// next train's phase sideways (the drag's own slope included, so shading and geometry agree). Heights x w.wgeo.z. `fpp` / `fpg`:
// the world size of a pixel / of a grid cell there across the view; `an` = (the view's heading on the water, 1 / sin^2 of its
// slope - 1): the footprint is that much longer along the view, so a train fades by its own heading (crests running toward the
// camera stay readable far out on a grazing view). A train finer than a few footprints fades out of the shading / the geometry.
// Returns (h, slope) of the shading trains, (hg, gg) of the geometry trains, and hw (sum of the trains' heights, for `wh`).
const GEO_RATIO: f32 = 0.73;
const GEO_DRAG: f32 = 0.014;
// Crest sharpness (exp(GEO_SHARP (sin - 1)); GEO_MEAN its mean, I0(k) e^-k) and the trains' spread off the wind (rad, first, per train).
const GEO_SHARP: f32 = 1.5;
const GEO_MEAN: f32 = 0.3674;
const GEO_SPREAD: f32 = 0.22;
const GEO_SPREAD_K: f32 = 0.12;
// Each train's length off the plain GEO_RATIO step (+- half of this), so no one spacing repeats down the trains.
const GEO_LEN_JIT: f32 = 0.35;
// Slow patches of calm and rough sea over all trains: amplitude GEO_PATCH_LO..GEO_PATCH_HI, patches ~1 / GEO_PATCH_S units.
const GEO_PATCH_S: f32 = 0.016;
const GEO_PATCH_LO: f32 = 0.3;
const GEO_PATCH_HI: f32 = 1.6;
struct SW { h: f32, g: vec2<f32>, hg: f32, gg: vec2<f32>, hw: f32 };
// `an` of `sea_w` for a view vector v (eye to the water); the slope floored at GEO_GRAZE.
const GEO_GRAZE: f32 = 0.05;
// The geometric sea's refraction shift at most (world units, `fs_water`).
const GEO_PUSH_MAX: f32 = 0.35;
fn geo_an(v: vec3<f32>) -> vec3<f32> {
	let l = max(length(v), 1e-3);
	let s = max(abs(v.z) / l, GEO_GRAZE);
	return vec3<f32>(normalize(v.xy + vec2<f32>(1e-6, 0.0)), 1.0 / (s * s) - 1.0);
}
fn sea_w(p0: vec2<f32>, t: f32, fpp: f32, fpg: f32, an: vec3<f32>, n: i32) -> SW {
	var o: SW;
	var p = p0;
	var j0 = vec2<f32>(1.0, 0.0);
	var j1 = vec2<f32>(0.0, 1.0);
	var len = w.wgeo.w;
	var sig = w.wgeo.x;
	for (var k = 0; k < n; k++) {
		let kk = 6.2832 / len;
		// Close to the wind for the long trains (long crests), wider for the chop.
		let ang = WAVE_WIND + (fract(f32(k) * 0.618034 + 0.31) - 0.5) * 2.0 * (GEO_SPREAD + GEO_SPREAD_K * f32(k));
		let d = vec2<f32>(cos(ang), sin(ang));
		let x = dot(d, p) * kk + t * sqrt(9.8 * kk) * WAVE_PACE + f32(k) * 1.9;
		let sx = sin(x);
		let cx = cos(x) * GEO_SHARP;
		let e = exp((sx - 1.0) * GEO_SHARP);
		let a = sig / kk;
		let gs = 0.25 / len;
		let ln = lnoise_d(p0 * gs + vec2<f32>(f32(k) * 13.1, t * 0.02));
		let gu = 0.55 + 0.9 * ln.x;
		let dgu = ln.yz * (0.9 * gs);
		// d x / d p0 through the drags so far (j0, j1: d p / d p0.x, d p0.y).
		let dx = vec2<f32>(dot(j0, d), dot(j1, d)) * kk;
		let hk = (e - GEO_MEAN) * a;
		let hh = hk * gu;
		let dh = dx * (e * cx * a * gu) + dgu * hk;
		let dv = dot(d, an.xy);
		let st = sqrt(1.0 + dv * dv * an.z) / len;
		let kp = 1.0 - smoothstep(0.12, 0.35, fpp * st);
		let kg = 1.0 - smoothstep(0.12, 0.35, fpg * st);
		o.h += hh * kp;
		o.g += dh * kp;
		o.hg += hh * kg;
		o.gg += dh * kg;
		o.hw += a;
		let m = e * cx * GEO_DRAG * len * gu;
		let dm = (dx * (e * (cx * cx - sx * GEO_SHARP) * gu) + dgu * (e * cx)) * (GEO_DRAG * len);
		p += d * m;
		j0 += d * dm.x;
		j1 += d * dm.y;
		len *= GEO_RATIO * (1.0 + GEO_LEN_JIT * (fract(f32(k) * 0.7549 + 0.17) - 0.5));
		sig *= w.wgeo.y;
	}
	// The calm / rough patches (their own slope included).
	let pn = lnoise_d(p0 * GEO_PATCH_S + vec2<f32>(31.7, t * 0.004));
	let pa = mix(GEO_PATCH_LO, GEO_PATCH_HI, pn.x);
	let dpa = pn.yz * ((GEO_PATCH_HI - GEO_PATCH_LO) * GEO_PATCH_S);
	o.g = o.g * pa + dpa * o.h;
	o.gg = o.gg * pa + dpa * o.hg;
	o.h *= pa;
	o.hg *= pa;
	o.h *= w.wgeo.z;
	o.g *= w.wgeo.z;
	o.hg *= w.wgeo.z;
	o.gg *= w.wgeo.z;
	o.hw *= w.wgeo.z;
	return o;
}

// The shore map (`SHORE_SHADER`: per cell the highest ground at most SHORE_CAP above the water, as an ordered key, 0 = not seen).
fn zkey_dec(k: u32) -> f32 {
	return bitcast<f32>(select(~k, k & 0x7fffffffu, (k & 0x80000000u) != 0u));
}
// At world XY: (that ground's height, how much of it was seen 0..1), bilinear over the seen cells.
fn shore_at(xy: vec2<f32>) -> vec2<f32> {
	let n = i32(w.gmap.w);
	let f = (xy - w.gmap.xy) / w.gmap.z - 0.5;
	let i = vec2<i32>(floor(f));
	let fr = f - floor(f);
	var acc = 0.0;
	var ws = 0.0;
	for (var k = 0; k < 4; k++) {
		let o = vec2<i32>(k & 1, k >> 1);
		let c = i + o;
		if (all(c >= vec2<i32>(0)) && all(c < vec2<i32>(n))) {
			let key = shore[c.y * n + c.x];
			if (key != 0u) {
				let wt = mix(1.0 - fr.x, fr.x, f32(o.x)) * mix(1.0 - fr.y, fr.y, f32(o.y));
				acc += zkey_dec(key) * wt;
				ws += wt;
			}
		}
	}
	return vec2<f32>(acc / max(ws, 1e-6), ws);
}

// How much the waves rise at the flat point f (f.z = the sea plane): (geometry, shading), 0..1. Calm where the ground comes
// within SHORE_TO under the surface or above it (shore map: no wave through the sand, no gap at a wall), and for the geometry only near a low camera and far off (the shading
// carries the far waves).
const SHORE_FROM: f32 = 0.1;
const SHORE_TO: f32 = 1.6;
// [PC] Where no ground is seen under the water: calm by the distance to low shore (`shore_dist` y: a beach's waterline band),
// from SHORE_D0 to SHORE_D1; by the distance to any ground at the surface (x: a dock's rim, a wall) only WALL_D0 to WALL_D1, so
// the waves come up to a dock (they stopped ~10 units short of the user's round dock, 2026-10-24). `shore_calm` (shore_map.rs).
const SHORE_D0: f32 = 3.5;
const SHORE_D1: f32 = 9.5;
const WALL_D0: f32 = 0.5;
const WALL_D1: f32 = 2.5;
// The geometric sea's surf (`fs_water`): band width on the map (world units), how far it reaches out and back with the waves
// (a share of the width), ground at most this deep under the surface, how opaque, and the clear circle round the swimming hero
// (the shore map holds him as ground).
const SURF_W: f32 = 0.8;
const SURF_PULSE: f32 = 0.35;
const SURF_DEEP: f32 = 1.5;
const SURF_ALPHA: f32 = 0.7;
const SURF_HERO0: f32 = 0.7;
const SURF_HERO1: f32 = 1.6;
// The breakup noise (the broken-up look of surf, foam, rings and wakes) is band-limited by the pixel's world size `fp`: an octave with
// under about two pixels per wavelength aliases (on a far or grazing view neighbouring pixels read unrelated noise, so the ragged
// foam edge changed with every camera move); each octave fades out to BREAKUP_MID (a little under the noise's mean 0.5: the foam's
// share stays as it was) between the sizes in `fs_water`. Near water is untouched.
const BREAKUP_MID: f32 = 0.43;
const HERO_DEEP: f32 = 3.0;
const HERO_HIGH: f32 = 1.5;
const CAM_LOW0: f32 = 1.5;
const CAM_LOW1: f32 = 4.0;
const CAM_CALM0: f32 = 1.0;
const CAM_CALM1: f32 = 6.0;
const GEO_FAR0: f32 = 160.0;
const GEO_FAR1: f32 = 320.0;
const GEO_FAR_T: f32 = 3000.0;
// Heights the position target's alpha can carry (`fs_grid`).
const GEO_HMAX: f32 = 3.0;
// How far a flat sea pixel's height may be from its plane's (`fs_grid`; planes closer than this are one, `positions`).
const GEO_MATCH: f32 = 1.5;
fn calm(f: vec3<f32>) -> vec2<f32> {
	let s = shore_at(f.xy);
	let sd = shore_dist(f.xy);
	let sh = shore_calm(sd, smoothstep(SHORE_FROM, SHORE_TO, f.z - s.x), s.y);
	let low = 1.0 - smoothstep(CAM_LOW0, CAM_LOW1, w.eye.z - f.z);
	let cam = mix(1.0, smoothstep(CAM_CALM0, CAM_CALM1, length(f.xy - w.eye.xy)), low);
	let far = 1.0 - smoothstep(GEO_FAR0, GEO_FAR1, length(f - w.eye.xyz));
	return vec2<f32>(sh * cam * far, sh);
}

// The projected grid: w.ggrid = (columns, rows, cell px, band px: the cell the wave band limit is made for), w.gridb = (first corner, step) in NDC; vertex i + j (columns + 1).
// Each vertex is cast along its view ray onto the plane w.geo[instance].x (a ray missing it: far out along its heading) and
// raised by the waves there. `flat_px`: the pixel of that flat point (`fs_grid` keeps only what lies over the visible flat sea).
struct GO { @builtin(position) pos: vec4<f32>, @location(0) world: vec3<f32>, @location(1) flat_px: vec2<f32>, @location(2) h: f32, @location(3) @interpolate(flat) g: u32 };
@vertex fn vs_grid(@builtin(vertex_index) vi: u32, @builtin(instance_index) gi: u32) -> GO {
	let vc = u32(w.ggrid.x) + 1u;
	let ndc = w.gridb.xy + vec2<f32>(f32(vi % vc), f32(vi / vc)) * w.gridb.zw;
	let z0 = w.geo[gi].x;
	let eye = w.eye.xyz;
	let a = w.ivp * vec4<f32>(ndc, 0.5, 1.0);
	let dir = normalize(a.xyz / a.w - eye);
	let down = z0 - eye.z;
	var f = vec3<f32>(eye.xy + normalize(dir.xy + vec2<f32>(1e-6, 0.0)) * GEO_FAR_T, z0);
	if (dir.z * down > 0.0) {
		let t = down / dir.z;
		if (t < GEO_FAR_T) { f = vec3<f32>(eye.xy + dir.xy * t, z0); }
	}
	let dist = max(length(f - eye), 1e-3);
	let fpg = dist * 2.0 * w.proj.y * w.size.w * w.ggrid.w;
	let h = grid_wave(f, ndc, z0, gi, fpg); // [PC] 0 for a vertex no kept fragment depends on (dry_tiles.rs)
	let pw = vec3<f32>(f.xy, z0 + h);
	var o: GO;
	o.pos = w.vp * vec4<f32>(pw, 1.0);
	o.world = pw;
	o.flat_px = (ndc * vec2<f32>(0.5, -0.5) + 0.5) * w.size.xy;
	o.h = h;
	o.g = gi;
	return o;
}

// The view depth of the plane z = z0 along pixel px's ray (1e9: the ray misses it).
fn plane_depth(px: vec2<f32>, z0: f32) -> f32 {
	let ndc = vec2<f32>(px.x * w.size.z * 2.0 - 1.0, 1.0 - px.y * w.size.w * 2.0);
	let a = w.ivp * vec4<f32>(ndc, 0.5, 1.0);
	let dir = a.xyz / a.w - w.eye.xyz;
	let t = (z0 - w.eye.z) / dir.z;
	if (t <= 0.0) { return 1e9; }
	return project(w.eye.xyz + dir * t).z;
}

// The raised surface into the position target where its flat point is visible flat sea of this plane, or flat sea hidden by
// something standing in it (the hero: a crest in front of him covers him, user 2026-10-24), and nothing of the level stands in front of
// the raised point (the retail sea's own depth does not count). a = 1 + class bits (64 / 128 sea, 512 geometric) + 0.45 x the height (-GEO_HMAX..GEO_HMAX as 0..1), read back in `fs_water`.
@fragment fn fs_grid(v: GO) -> @location(0) vec4<f32> {
	let gz = w.geo[v.g];
	let dim = vec2<i32>(textureDimensions(seamask)); let fc = clamp(vec2<i32>(v.flat_px), vec2<i32>(0), dim - vec2<i32>(1));
	let m = textureLoad(seamask, fc, 0).r; let zp0 = plane_depth(v.flat_px, gz.x); let zf = linear_depth(textureLoad(depth_ms, fc, 0));
	// (Retail's sea: long triangles up to about a unit apart, Pokitaru.) A stander: before the flat point, its point there under 2 GEO_HMAX over the plane.
	if (abs(m - gz.x) > GEO_MATCH && !(m < -1e8 && hero_pixel(fc, zf, gz.x))) { discard; }
	let zs = linear_depth(textureLoad(depth_ms, vec2<i32>(v.pos.xy), 0));
	let zw = linear_depth(v.pos.z);
	if (zw > zs * 1.004 + 0.03) {
		let zp = plane_depth(v.pos.xy, gz.x);
		if (abs(zs - zp) > zp * 0.004 + 0.05) { discard; }
	}
	return vec4<f32>(v.world, 1.0 + gz.y + 0.45 * clamp(v.h / (2.0 * GEO_HMAX) + 0.5, 0.0, 1.0));
}

// The flat sea meshes into the sea mask: their height where they are visible (as `fs_pos`).
@fragment fn fs_mask(v: PO) -> @location(0) vec4<f32> {
	let zs = linear_depth(textureLoad(depth_ms, vec2<i32>(v.pos.xy), 0));
	let zw = linear_depth(v.pos.z);
	if (zw > zs * 1.004 + 0.03) { discard; }
	return vec4<f32>(v.world.z, 0.0, 0.0, 1.0);
}

// [PC] Geometric sea: a pixel of visible flat sea the raised surface left uncovered (a trough at a waterline drops below the
// flat sea there): the flat sea's point on this pixel's ray as the grid would write it (height 0), so `fs_water` shades it
// instead of leaving retail's picture (thin slivers along every shore, user screenshot 2026-10-19). a = 0: none.
fn geo_gap(c: vec2<i32>) -> vec4<f32> {
	let dim = vec2<i32>(textureDimensions(seamask));
	let m = textureLoad(seamask, clamp(c, vec2<i32>(0), dim - vec2<i32>(1)), 0).r;
	if (m < -1e8) { return vec4<f32>(0.0); }
	var bits = 0.0;
	for (var k = 0; k < 4; k++) {
		if (w.geo[k].y > 0.5 && abs(m - w.geo[k].x) <= GEO_MATCH) {
			bits = w.geo[k].y;
			break;
		}
	}
	if (bits < 0.5) { return vec4<f32>(0.0); }
	let px = vec2<f32>(c) + 0.5;
	let ndc = vec2<f32>(px.x * w.size.z * 2.0 - 1.0, 1.0 - px.y * w.size.w * 2.0);
	let a = w.ivp * vec4<f32>(ndc, 0.5, 1.0);
	let dir = a.xyz / a.w - w.eye.xyz;
	let t = (m - w.eye.z) / dir.z;
	if (t <= 0.0) { return vec4<f32>(0.0); }
	return vec4<f32>(w.eye.xyz + dir * t, 1.0 + bits + 0.45 * 0.5);
}

// Is the picture at uv water (geometric sea: the flat sea mask, as the picture holds the flat sea)? [PC] Geometric sea: all four
// texels the bilinear read blends: a crest by a dock's rim reads the picture displaced towards the rim (the flat point under a
// raised point lies pixels away), and one rim texel in the blend made a stippled dark band along it (2026-10-24).
fn wet(uv: vec2<f32>, geo: bool) -> bool {
	let q = vec2<i32>(uv * w.size.xy);
	if (geo) {
		let dim = vec2<i32>(textureDimensions(seamask));
		let q0 = vec2<i32>(floor(uv * w.size.xy - 0.5));
		for (var k = 0; k < 4; k++) {
			if (textureLoad(seamask, clamp(q0 + vec2<i32>(k & 1, k >> 1), vec2<i32>(0), dim - vec2<i32>(1)), 0).r < -1e8) { return false; }
		}
		return true;
	}
	return wpos_at(q).a > 0.5;
}

// [PC] Geometric sea: the linear depth of the plane z = m on the ray through screen point px (pixels).
fn sea_ray_z(px: vec2<f32>, m: f32) -> f32 {
	let ndc = vec2<f32>(px.x * w.size.z * 2.0 - 1.0, 1.0 - px.y * w.size.w * 2.0);
	let a = w.ivp * vec4<f32>(ndc, 0.5, 1.0);
	let dir = a.xyz / a.w - w.eye.xyz;
	return project(w.eye.xyz + dir * ((m - w.eye.z) / dir.z)).z;
}

// [PC] Geometric sea: is the level's depth zs at uv retail's flat sea itself (it wrote its depth there), not a bed under it?
// Within one pixel's slope of the flat sea's depth at that pixel (the depth is read at a sample, not at the pixel's centre).
fn on_flat_sea(uv: vec2<f32>, zs: f32) -> bool {
	let q = vec2<i32>(uv * w.size.xy);
	let g = geo_gap(q);
	if (g.a < 0.5) { return false; }
	let px = vec2<f32>(q) + 0.5;
	let z0 = sea_ray_z(px, g.z);
	let slope = abs(sea_ray_z(px + vec2<f32>(1.0, 0.0), g.z) - z0) + abs(sea_ray_z(px + vec2<f32>(0.0, 1.0), g.z) - z0);
	return abs(zs - z0) < 0.05 + slope;
}

fn wpos_at(c: vec2<i32>) -> vec4<f32> {
	let dim = vec2<i32>(textureDimensions(wpos));
	return textureLoad(wpos, clamp(c, vec2<i32>(0), dim - vec2<i32>(1)), 0);
}

// The level's linear depth at screen uv, full size (sample 0 of the MSAA depth).
fn scene_z(uv: vec2<f32>) -> f32 {
	let d = vec2<i32>(textureDimensions(depth_ms));
	let q = clamp(vec2<i32>(uv * vec2<f32>(d)), vec2<i32>(0), d - vec2<i32>(1));
	return linear_depth(textureLoad(depth_ms, q, 0));
}

// The surface's own large-scale normal (a sloped river, a waterfall's lip) from the positions 4 and 11 pixels around, averaged;
// flat up where too little water is around. The retail wave meshes (Pokitaru's pool: 1-unit tiles bobbing a few hundredths)
// tilt each tile by a few degrees, and following that split the water into tiles that each reflected their own patch of sky,
// so tilts under about 8 degrees count as flat (the ripples carry the motion) and only real slopes bend the surface.
fn geo_normal(c: vec2<i32>, p: vec3<f32>, dist: f32) -> vec3<f32> {
	var acc = vec3<f32>(0.0);
	let lim = 0.05 * dist + 1.0;
	for (var k = 0; k < 2; k++) {
		let r = select(4, 11, k == 1);
		let xp = wpos_at(c + vec2<i32>(r, 0));
		let xn = wpos_at(c - vec2<i32>(r, 0));
		let yp = wpos_at(c + vec2<i32>(0, r));
		let yn = wpos_at(c - vec2<i32>(0, r));
		let okxp = xp.a > 0.5 && length(xp.xyz - p) < lim;
		let okxn = xn.a > 0.5 && length(xn.xyz - p) < lim;
		let okyp = yp.a > 0.5 && length(yp.xyz - p) < lim;
		let okyn = yn.a > 0.5 && length(yn.xyz - p) < lim;
		if (!(okxp || okxn) || !(okyp || okyn)) { continue; }
		let dx = select(select(p - xn.xyz, xp.xyz - p, okxp), xp.xyz - xn.xyz, okxp && okxn);
		let dy = select(select(p - yn.xyz, yp.xyz - p, okyp), yp.xyz - yn.xyz, okyp && okyn);
		var n = normalize(cross(dx, dy) + vec3<f32>(0.0, 0.0, 1e-6));
		if (n.z < 0.0) { n = -n; }
		if (n.z > 0.6) { acc += n; }
	}
	if (acc.z <= 0.0) { return vec3<f32>(0.0, 0.0, 1.0); }
	let t = normalize(acc);
	return normalize(mix(vec3<f32>(0.0, 0.0, 1.0), t, 0.75 * smoothstep(0.99, 0.95, t.z)));
}

// World point -> (screen uv, view depth).
fn project(q: vec3<f32>) -> vec3<f32> {
	let k = w.vp * vec4<f32>(q, 1.0);
	return vec3<f32>(k.xy / k.w * vec2<f32>(0.5, -0.5) + 0.5, k.w);
}

fn on_screen(s: vec3<f32>) -> bool {
	return s.z > w.proj.z && all(s.xy >= vec2<f32>(0.0)) && all(s.xy <= vec2<f32>(1.0));
}

fn edge_fade(uv: vec2<f32>) -> f32 {
	let e = min(uv, 1.0 - uv);
	return smoothstep(0.0, 0.12, min(e.x, e.y));
}

// Screen-space reflection of ray `p` + t `r` against the full-size depth: (hit uv, ray length, confidence 0..1). The march
// grows its step (32 steps reach the far plane) from a per-pixel dithered start, then halves in on the crossing; a ray that
// passes behind a thin object keeps going. Confidence fades at the screen edges and for late (far) hits.
fn reflect_ray(p: vec3<f32>, r: vec3<f32>, jit: f32) -> vec4<f32> {
	var t = 0.12 * (1.0 + jit);
	var prev = 0.0;
	// w.rn.w = steps (32 = High; fewer steps grow faster, 1.3^(32/n) per step, so the ray still reaches as far).
	let n = i32(w.rn.w);
	var grow = 1.3;
	var add = 0.05;
	if (n < 32) {
		grow = pow(1.3, 32.0 / f32(n));
		add = (grow - 1.0) / 6.0;
	}
	for (var i = 0; i < n; i++) {
		let s = project(p + r * t);
		if (!on_screen(s)) { break; }
		let sz = scene_z(s.xy);
		let dz = s.z - sz;
		if (dz > 0.02 && sz < w.proj.w * 0.98 && dz < max(0.5, (t - prev) * 1.5)) {
			var a = prev;
			var b = t;
			for (var j = 0; j < 6; j++) {
				let m = (a + b) * 0.5;
				let sm = project(p + r * m);
				if (sm.z > scene_z(sm.xy)) { b = m; } else { a = m; }
			}
			let hit = project(p + r * b);
			return vec4<f32>(hit.xy, b, edge_fade(hit.xy) * (1.0 - smoothstep(0.55, 1.0, f32(i) / f32(n))));
		}
		prev = t;
		t = t * grow + add;
	}
	return vec4<f32>(0.5, 0.5, t, 0.0);
}

// The scene at `uv`, softened over a disc of radius `rad` (uv): six taps turned by the pixel's dither.
fn soft_scene(uv: vec2<f32>, rad: f32, jit: f32) -> vec3<f32> {
	var acc = to_linear(textureSampleLevel(scene, lsamp, uv, 0.0).rgb) * 0.25;
	for (var k = 0; k < 6; k++) {
		let a = (f32(k) + jit) * 1.0472;
		let o = vec2<f32>(cos(a), sin(a) * w.size.x * w.size.w) * rad * (0.55 + 0.45 * f32(k & 1));
		acc += to_linear(textureSampleLevel(scene, lsamp, clamp(uv + o, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb) * 0.125;
	}
	return acc;
}

// What a ray that finds nothing on screen reflects: halfway between the water's own colour and the fog colour as the far
// air, brighter towards the horizon, never much brighter than the water itself (so a dark pool stays dark and the sea keeps
// its colour).
fn env(r: vec3<f32>, base: vec3<f32>) -> vec3<f32> {
	let h = 1.0 - clamp(r.z, 0.0, 1.0);
	let cap = clamp((luma(base) + 0.04) / (luma(w.fog.rgb) + 0.04) * 1.6, 0.15, 1.0);
	return mix(base * 1.1, w.fog.rgb * cap, 0.45) * (0.8 + 0.4 * h * h);
}

// Lava (Gaspar's RAM ocean, `LAVA_SLOT`: w.flows[..].w = -2; user 2026-10-16: "it doesnt really match the quality on hoven"):
// retail's own animated lava picture, moved a little by a heat shimmer, under a molten skin with slow swells (lit by the key
// light, glossy: a Fresnel reflection of what stands around), dark crust plates drifting with the flow in broad fields and
// breaking up into small floes at their edges, with glowing cracks between them, bubbles that swell and pop on the open
// lava, and a white-hot rim with a glow where the lava meets rock. All in world space and time; every fine part fades to its
// mean before it gets finer than a few pixels (far lava keeps a calm glow instead of flickering).
const LAVA_HOT: vec3<f32> = vec3<f32>(1.0, 0.42, 0.07);
const LAVA_WHITE: vec3<f32> = vec3<f32>(1.0, 0.82, 0.42);
const LAVA_CRUST: vec3<f32> = vec3<f32>(0.02, 0.008, 0.005);
// Crust plates per world unit; the crust fields' share (higher = less crust); the flow (world units a second).
const LAVA_PLATE: f32 = 0.32;
const LAVA_COVER: f32 = 0.62;
const LAVA_FLOW: vec2<f32> = vec2<f32>(0.06, 0.035);
// The swell's slope, the molten skin's reflectance, the shore rim, the bubbles, the heat shimmer (world units).
const LAVA_SWELL: f32 = 0.4;
const LAVA_GLOSS: f32 = 0.5;
const LAVA_RIM: f32 = 1.0;
// How far from the rock the rim reaches (world units); the lava depth over the rock where the rim and the glow around it
// fade out; deeper (or nothing under the lava) counts as LAVA_DEEP.
const LAVA_SHORE: f32 = 0.2;
const LAVA_RIM_DEPTH: f32 = 0.18;
const LAVA_HALO_DEPTH: f32 = 0.7;
const LAVA_DEEP: f32 = 4.0;
// How bright the lava glows along a plate's edge, and how far (cell units) that glow reaches into the open lava.
const LAVA_CRACK: f32 = 0.8;
const LAVA_EDGE: f32 = 0.07;
const LAVA_BUBBLE: f32 = 1.0;
const LAVA_SHIMMER: f32 = 0.16;

// Integer lattice hash (Gaspar's large world coordinates break a fract(sin) hash under the GPU's fast maths).
fn lhash_u(i: vec2<i32>) -> u32 {
	var n = bitcast<u32>(i.x) * 1597334677u ^ bitcast<u32>(i.y) * 3812015801u;
	n = (n ^ (n >> 16u)) * 2246822519u;
	n = (n ^ (n >> 13u)) * 3266489917u;
	return n ^ (n >> 16u);
}
fn lhash(i: vec2<i32>) -> f32 {
	return f32(lhash_u(i) >> 8u) / 16777215.0;
}
fn lhash2(i: vec2<i32>) -> vec2<f32> {
	let n = lhash_u(i);
	return vec2<f32>(f32(n & 0xffffu), f32(n >> 16u)) / 65535.0;
}
// Value noise 0..1 with its slope: (value, d/dx, d/dy).
fn lnoise_d(p: vec2<f32>) -> vec3<f32> {
	let fl = floor(p);
	let i = vec2<i32>(fl);
	let f = p - fl;
	let u = f * f * (3.0 - 2.0 * f);
	let du = 6.0 * f * (1.0 - f);
	let a = lhash(i);
	let b = lhash(i + vec2<i32>(1, 0));
	let c = lhash(i + vec2<i32>(0, 1));
	let d = lhash(i + vec2<i32>(1, 1));
	let k = a - b - c + d;
	return vec3<f32>(a + (b - a) * u.x + (c - a) * u.y + k * u.x * u.y, du.x * ((b - a) + k * u.y), du.y * ((c - a) + k * u.x));
}
fn lnoise(p: vec2<f32>) -> f32 {
	return lnoise_d(p).x;
}

// The molten skin's slope: three octaves of flowing noise (swells about 6, 2.6 and 1.1 units long), each fading out before
// it gets finer than a few pixels (`fp` = world size of a pixel).
fn lava_slope(p: vec2<f32>, t: f32, fp: f32) -> vec2<f32> {
	var g = vec2<f32>(0.0);
	var f = 0.17;
	var a = 1.0;
	for (var k = 0; k < 3; k++) {
		let n = lnoise_d((p - LAVA_FLOW * t * (1.0 + 0.4 * f32(k))) * f + vec2<f32>(f32(k) * 17.3, f32(k) * 5.9));
		g += n.yz * f * a * (1.0 - smoothstep(0.12, 0.35, fp * f));
		f *= 2.3;
		a *= 0.5;
	}
	return g * LAVA_SWELL;
}

// Crust plates: a jittered cell set (cell units). x = the gap to the crack (second nearest minus nearest), y = the plate's
// own value.
fn lava_plates(q: vec2<f32>, t: f32) -> vec2<f32> {
	let fl = floor(q);
	let i = vec2<i32>(fl);
	let f = q - fl;
	var d1 = 9.0;
	var d2 = 9.0;
	var id = 0.0;
	for (var y = -1; y <= 1; y++) {
		for (var x = -1; x <= 1; x++) {
			let o = vec2<i32>(x, y);
			let h = lhash2(i + o);
			let pt = vec2<f32>(o) + 0.5 + 0.36 * sin(vec2<f32>(t * 0.06, t * 0.045) + 6.2831 * h);
			let d = length(pt - f);
			if (d < d1) {
				d2 = d1;
				d1 = d;
				id = h.x;
			} else if (d < d2) {
				d2 = d;
			}
		}
	}
	return vec2<f32>(d2 - d1, id);
}

// A bubble on the open lava (cells about 2.9 units; four in ten bubble, each on its own beat): x = the dome (0..1) while it
// swells, y = the ring of its pop.
fn lava_bubble(p: vec2<f32>, t: f32) -> vec2<f32> {
	let q = p * 0.35;
	let fl = floor(q);
	let i = vec2<i32>(fl);
	let h = lhash2(i + vec2<i32>(311, 97));
	if (h.x > 0.4) { return vec2<f32>(0.0); }
	let ph = fract(t / (2.5 + 10.0 * h.x) + h.y);
	let d = length(q - fl - (0.3 + 0.4 * lhash2(i + vec2<i32>(91, 37))));
	if (ph < 0.3) {
		let r = 0.2 * smoothstep(0.0, 1.0, ph / 0.3);
		return vec2<f32>(1.0 - smoothstep(r * 0.6, r + 0.01, d), 0.0);
	}
	if (ph < 0.42) {
		let k = (ph - 0.3) / 0.12;
		return vec2<f32>(0.0, (1.0 - smoothstep(0.0, 0.04 + 0.04 * k, abs(d - 0.2 - 0.22 * k))) * (1.0 - k));
	}
	return vec2<f32>(0.0);
}

// The lava's colour (linear light) at water pixel `c` (world point `p`).
fn lava(c: vec2<i32>, p: vec3<f32>, view: vec3<f32>, dist: f32, fp: f32, jit: f32, uv0: vec2<f32>) -> vec3<f32> {
	let t = w.eye.w;
	let sun_on = clamp(w.sun.w / 2.2, 0.0, 1.0);
	// The key light, else light from high up behind the view (crust and swells stay shaped at night or in shade).
	let hd = normalize(vec2<f32>(view.x, view.y) + vec2<f32>(1e-4, 0.0));
	let l = select(normalize(vec3<f32>(hd * 0.5, 0.86)), w.sun.xyz, w.sun.w > 0.0);
	let s = lava_slope(p.xy, t, fp);
	let n = normalize(vec3<f32>(-s, 1.0));

	// Heat shimmer: the picture under the skin moves a little with the swell and a slow flowing wobble (only from lava).
	let wob = vec2<f32>(lnoise(p.xy * 0.35 + vec2<f32>(t * 0.25, 0.0)), lnoise(p.xy * 0.35 + vec2<f32>(7.1, 2.3 - t * 0.22))) - 0.5;
	var uv = project(p + vec3<f32>(wob * 2.0 * LAVA_SHIMMER + s * 0.3, 0.0)).xy;
	if (wpos_at(vec2<i32>(uv * w.size.xy)).a < 0.5) { uv = uv0; }
	let base = to_linear(textureSampleLevel(scene, lsamp, uv, 0.0).rgb);

	// Molten: retail's colour, a slow heat pulse rolling across it, the swells lit (brighter facing the light, its mean kept).
	let pulse = 0.5 + 0.5 * sin(t * 0.8 + dot(p.xy, vec2<f32>(0.07, 0.05)) + lnoise(p.xy * 0.04) * 6.2832);
	var molten = base * (1.05 + 0.3 * pulse) * max(1.0 + 1.2 * (dot(n, l) - l.z), 0.3);

	// Crust: broad fields drifting with the flow; inside one, plates with thin cracks; at its edge they shrink into floes.
	let cq = (p.xy - LAVA_FLOW * t) * 0.022;
	let zone = smoothstep(LAVA_COVER - 0.1, LAVA_COVER + 0.12, lnoise(cq) * 0.65 + lnoise(cq * 2.7 + vec2<f32>(4.1, 8.3)) * 0.35);
	let warp = vec2<f32>(lnoise(p.xy * 0.045 + vec2<f32>(1.7, 0.0)), lnoise(p.xy * 0.045 + vec2<f32>(0.0, 6.4))) - 0.5;
	let pl = lava_plates((p.xy - LAVA_FLOW * t) * LAVA_PLATE + warp * 2.0, t);
	let cw = mix(0.55, 0.06, zone) + pl.y * 0.05;
	let aa = fp * LAVA_PLATE * 1.5;
	let far = smoothstep(0.08, 0.35, aa);
	let on = smoothstep(0.0, 0.25, zone);
	let plate = mix(smoothstep(cw, cw + max(aa, 0.04), pl.x), clamp(1.0 - cw * 1.7, 0.0, 1.0), far) * on;
	// The glow along a plate's edge: in a thin crack all of it, in the open lava a band fading out from the floe.
	let crack = mix(exp(-max(cw - pl.x, 0.0) / (LAVA_EDGE + aa)), 0.25, far) * on * (1.0 - plate);
	// Rough crust: lit by the light, a dull red where it borders a crack, a faint glow through it.
	let rough = lnoise_d(p.xy * 1.6).yz * 1.6 * 0.35 * (1.0 - smoothstep(0.1, 0.35, fp * 1.6));
	let nc = normalize(vec3<f32>(-s * 0.5 - rough, 1.0));
	let ember = lnoise(p.xy * 0.9 + vec2<f32>(t * 0.05, 0.0)) * (1.0 - smoothstep(0.1, 0.4, fp * 0.9));
	var crust = LAVA_CRUST * (0.5 + 1.0 * max(dot(nc, l), 0.0) * (0.4 + 0.6 * sun_on)) + base * (0.015 + 0.14 * ember * ember * ember);
	let hem = 1.0 - smoothstep(cw, cw + 0.06 + aa, pl.x);
	crust = mix(crust, base * 0.2, hem * hem * (1.0 - far));

	// Gloss: the reflection of what stands around (screen space, else the far air) on the molten skin, little on the crust.
	let ndv = clamp(dot(n, view), 0.0, 1.0);
	let fres = 0.04 + 0.96 * pow(1.0 - ndv, 5.0);
	var r = reflect(-view, n);
	r = normalize(vec3<f32>(r.xy, max(r.z, 0.04)));
	let hit = reflect_ray(p + vec3<f32>(0.0, 0.0, 0.02), r, jit);
	var refl = env(r, w.fog.rgb);
	if (hit.w > 0.0) {
		refl = mix(refl, soft_scene(hit.xy, (0.003 + 0.014 * hit.z / (hit.z + dist)) * (1.0 + FAR_BLUR * smoothstep(FAR_FROM, FAR_TO, dist)), jit), hit.w);
	}
	molten += refl * fres * LAVA_GLOSS;
	crust += refl * fres * 0.1;
	// The key light's sheen on the swells (broad, warm, softening with distance).
	let hv = normalize(l + view);
	let e = 60.0 / (1.0 + fp * 20.0);
	molten += vec3<f32>(1.0, 0.85, 0.6) * pow(max(dot(n, hv), 0.0), e) * (e + 8.0) * 0.004 * (0.3 + 0.7 * sun_on);

	var col = mix(molten, crust, plate);
	col += (LAVA_HOT + LAVA_WHITE * crack * crack * crack * 0.6) * crack * LAVA_CRACK;

	// Bubbles on the open lava: a darker skin swelling, a bright rim, then a hot ring as it pops.
	let bub = lava_bubble(p.xy, t) * (1.0 - smoothstep(0.04, 0.15, fp)) * (1.0 - plate) * LAVA_BUBBLE;
	col = mix(col, col * 0.5 + LAVA_CRUST, bub.x * 0.7);
	col += LAVA_WHITE * bub.x * (1.0 - bub.x) * 1.6 + LAVA_HOT * bub.y * 0.6;

	// Where the lava meets rock: a white-hot rim where geometry cuts the surface nearby, and a broader glow where the lava runs
	// shallow over the rock under it. The rim's taps sit at fixed world offsets around the point on the lava plane (twelve
	// directions, two radii up to LAVA_SHORE, at least 1.5 pixels out), each weighted by how near the depth there is to the
	// tap's own, so they stay on the same ground while the camera moves. The glow reads the bed (the level drawn without the
	// lava, mirror.rs `draw_bed`) bilinearly, so it follows the rock smoothly. Before, eight taps at fixed pixel offsets made
	// both: they stair-stepped and crawled at every shore while the camera crept [C: flick.py on fg1/9_0362 fields 60-84,
	// 2026-10-16: lava flicker 6.5 with them, 4.1 now, 3.8 with no rim at all; the rest is the bobbing lava rocks, whose rim
	// moves with them].
	var h1 = 0.0;
	let rs0 = max(LAVA_SHORE, fp * 3.0);
	for (var k = 0; k < 12; k++) {
		let a = f32(k) * 0.523599 + 0.2618;
		let d = vec2<f32>(cos(a), sin(a));
		for (var j = 0; j < 2; j++) {
			let sq = project(vec3<f32>(p.xy + d * rs0 * (0.5 + 0.5 * f32(j)), p.z));
			let q = vec2<i32>(sq.xy * w.size.xy);
			if (on_screen(sq) && wpos_at(q).a < 0.5) {
				let zq = scene_z((vec2<f32>(q) + 0.5) * w.size.zw);
				let tol = 0.06 * sq.z + 0.25;
				h1 += (1.0 - smoothstep(0.5 * tol, tol, abs(zq - sq.z))) * (1.0 - 0.4 * f32(j));
			}
		}
	}
	let zw = project(p).z;
	let bd = vec2<i32>(textureDimensions(bed_depth));
	let bf = uv0 * vec2<f32>(bd) - 0.5;
	let b0 = vec2<i32>(floor(bf));
	let bw = bf - floor(bf);
	var dep = 0.0;
	for (var k = 0; k < 4; k++) {
		let o = vec2<i32>(k & 1, k >> 1);
		let zb = linear_depth(textureLoad(bed_depth, clamp(b0 + o, vec2<i32>(0), bd - vec2<i32>(1)), 0));
		let dk = select(LAVA_DEEP, clamp(p.z - (w.eye.z + (p.z - w.eye.z) * zb / zw), 0.0, LAVA_DEEP), zb > zw && zb < w.proj.w * 0.98);
		dep += dk * select(1.0 - bw.x, bw.x, o.x == 1) * select(1.0 - bw.y, bw.y, o.y == 1);
	}
	let flick = 0.7 + 0.3 * lnoise(p.xy * 1.2 + vec2<f32>(t * 0.7, -t * 0.5));
	let rim = max(smoothstep(0.0, 2.5, h1), 1.0 - smoothstep(0.0, LAVA_RIM_DEPTH, dep)) * LAVA_RIM;
	let halo = max(rim * 0.6, 1.0 - smoothstep(0.0, LAVA_HALO_DEPTH, dep)) * LAVA_RIM;
	col = mix(col, max(molten, LAVA_HOT), halo * 0.35);
	col += (LAVA_HOT * 0.4 + LAVA_WHITE * 0.25 * flick * flick) * rim * flick;
	return col;
}

// Mud (Aridia's ocean, `MUD_SLOT`: w.flows[..].w = -4; the brown quicksand the hero sinks in): retail's own picture under a
// slow churn (it oozes a little with broad low swells, lit by the key light), darker wet patches drifting across it, a dim
// soft sheen of the air at a grazing view (no mirror: mud reflects nothing sharp), and slow bubbles that swell and pop.
const MUD_SWELL: f32 = 0.35;
const MUD_OOZE: f32 = 0.1;
const MUD_FLOW: vec2<f32> = vec2<f32>(0.018, 0.011);
const MUD_SHEEN: f32 = 0.3;
const MUD_BUBBLE: f32 = 0.8;

// Slow churn slope: two octaves of drifting noise (swells about 7 and 3 units), each fading out before it gets finer than a
// few pixels.
fn mud_slope(p: vec2<f32>, t: f32, fp: f32) -> vec2<f32> {
	var g = vec2<f32>(0.0);
	var f = 0.14;
	var a = 1.0;
	for (var k = 0; k < 2; k++) {
		let n = lnoise_d((p - MUD_FLOW * t * (1.0 - 1.6 * f32(k))) * f + vec2<f32>(f32(k) * 13.1, f32(k) * 3.7 + t * 0.01));
		g += n.yz * f * a * (1.0 - smoothstep(0.12, 0.35, fp * f));
		f *= 2.3;
		a *= 0.55;
	}
	return g * MUD_SWELL;
}

fn mud(c: vec2<i32>, p: vec3<f32>, view: vec3<f32>, dist: f32, fp: f32, uv0: vec2<f32>) -> vec3<f32> {
	let t = w.eye.w;
	let sun_on = clamp(w.sun.w / 2.2, 0.0, 1.0);
	let hd = normalize(vec2<f32>(view.x, view.y) + vec2<f32>(1e-4, 0.0));
	let l = select(normalize(vec3<f32>(hd * 0.5, 0.86)), w.sun.xyz, w.sun.w > 0.0);
	let s = mud_slope(p.xy, t, fp);
	let n = normalize(vec3<f32>(-s, 1.0));
	// The ooze: the picture moves a little with a slow wobble and the swell (only from mud).
	let wob = vec2<f32>(lnoise(p.xy * 0.2 + vec2<f32>(t * 0.04, 0.0)), lnoise(p.xy * 0.2 + vec2<f32>(5.3, 1.1 - t * 0.035))) - 0.5;
	var uv = project(p + vec3<f32>(wob * 2.0 * MUD_OOZE + s * 0.2, 0.0)).xy;
	if (wpos_at(vec2<i32>(uv * w.size.xy)).a < 0.5) { uv = uv0; }
	let base = to_linear(textureSampleLevel(scene, lsamp, uv, 0.0).rgb);
	// Lit swells (the mean kept) and darker wet patches.
	let wet = smoothstep(0.45, 0.75, lnoise((p.xy - MUD_FLOW * t * 2.0) * 0.08 + vec2<f32>(2.1, 7.7)));
	var col = base * max(1.0 + 0.9 * (dot(n, l) - l.z), 0.5) * (1.0 - 0.18 * wet);
	// The sheen: the air (env) blurred, only at a grazing view, stronger on the wet patches; a broad dull highlight.
	let ndv = clamp(dot(n, view), 0.0, 1.0);
	let fres = 0.02 + 0.98 * pow(1.0 - ndv, 5.0);
	var r = reflect(-view, n);
	r = normalize(vec3<f32>(r.xy, max(r.z, 0.04)));
	col += env(r, base) * fres * MUD_SHEEN * (0.5 + 0.5 * wet);
	let hv = normalize(l + view);
	let e = 18.0 / (1.0 + fp * 10.0);
	col += vec3<f32>(1.0, 0.9, 0.75) * pow(max(dot(n, hv), 0.0), e) * (e + 8.0) * 0.0015 * (0.3 + 0.7 * sun_on) * (0.4 + 0.6 * wet);
	// Bubbles: a dome swelling (lit on top, its rim darker), then a dark ring as it pops; slower than lava's.
	let bub = lava_bubble(p.xy + vec2<f32>(57.0, 23.0), t * 0.45) * (1.0 - smoothstep(0.04, 0.15, fp)) * MUD_BUBBLE;
	col = mix(col, col * 1.25 + base * 0.1, bub.x * (1.0 - bub.x) * 2.0);
	col *= 1.0 - 0.35 * bub.y - 0.15 * bub.x * bub.x;
	return col;
}

// Toxic sludge (Orxon's green sea, level terrain texture 12, `SLUDGE_SLOT`: w.flows[..].w = -5): retail's own picture, thick:
// slow heavy swells under a glossy skin (a soft reflection of what stands around that darkens the bright goo where rock
// mirrors in it, broad highlights of the light), a glow rolling through it in broad veins (the goo away from one goes duller),
// and domes that swell slowly, glint and pop into a ring. Retail draws it still; all the motion here is slow.
const SLUDGE_SWELL: f32 = 0.9;
const SLUDGE_FLOW: vec2<f32> = vec2<f32>(0.03, -0.02);
const SLUDGE_GLOW: f32 = 0.35;
const SLUDGE_GLOSS: f32 = 0.55;
const SLUDGE_BUBBLE: f32 = 1.0;

// A sludge dome (cells 3 units; seven in ten swell, each on its own beat of 5-12 s): x, y = its slope, z = the ring left by
// its pop (0..1). The dome grows to 0.9 units across over most of its beat, then pops.
fn sludge_bubble(p: vec2<f32>, t: f32) -> vec3<f32> {
	let q = p / 3.0;
	let fl = floor(q);
	let i = vec2<i32>(fl);
	let h = lhash2(i + vec2<i32>(733, 151));
	if (h.x > 0.7) { return vec3<f32>(0.0); }
	let ph = fract(t / (5.0 + 10.0 * h.x) + h.y);
	let o = (q - fl - (0.3 + 0.4 * lhash2(i + vec2<i32>(29, 613)))) * 3.0;
	let d = length(o);
	if (ph < 0.8) {
		let r = 0.45 * smoothstep(0.0, 0.8, ph);
		if (d >= r) { return vec3<f32>(0.0); }
		// A cap of a sphere of radius r: slope o / sqrt(r^2 - d^2), clipped at its foot.
		let s = o / max(sqrt(r * r - d * d), r * 0.25) * smoothstep(0.0, 0.1, ph);
		return vec3<f32>(s * 0.6, 0.0);
	}
	let k = (ph - 0.8) / 0.2;
	return vec3<f32>(0.0, 0.0, (1.0 - smoothstep(0.0, 0.08 + 0.1 * k, abs(d - 0.45 - 0.6 * k))) * (1.0 - k));
}

fn sludge(c: vec2<i32>, p: vec3<f32>, view: vec3<f32>, dist: f32, fp: f32, jit: f32, uv0: vec2<f32>) -> vec3<f32> {
	let t = w.eye.w;
	let sun_on = clamp(w.sun.w / 2.2, 0.0, 1.0);
	let hd = normalize(vec2<f32>(view.x, view.y) + vec2<f32>(1e-4, 0.0));
	let l = select(normalize(vec3<f32>(hd * 0.5, 0.86)), w.sun.xyz, w.sun.w > 0.0);
	var g = vec2<f32>(0.0);
	var f = 0.15;
	var a = 1.0;
	for (var k = 0; k < 3; k++) {
		let nd = lnoise_d((p.xy - SLUDGE_FLOW * t * (1.0 + 0.5 * f32(k))) * f + vec2<f32>(f32(k) * 9.7, t * 0.02 + f32(k) * 4.3));
		g += (nd.yz - 0.0) * f * a * (1.0 - smoothstep(0.12, 0.35, fp * f));
		f *= 2.2;
		a *= 0.55;
	}
	// The domes, fading out before they get finer than a few pixels.
	let bf = 1.0 - smoothstep(0.08, 0.25, fp);
	let bub = sludge_bubble(p.xy, t) * bf * SLUDGE_BUBBLE;
	let s = g * SLUDGE_SWELL + bub.xy;
	let n = normalize(vec3<f32>(-s, 1.0));
	// The picture under the skin moves a little with the swell.
	var uv = project(p + vec3<f32>(s * 0.25, 0.0)).xy;
	if (wpos_at(vec2<i32>(uv * w.size.xy)).a < 0.5) { uv = uv0; }
	let base = to_linear(textureSampleLevel(scene, lsamp, uv, 0.0).rgb);
	// The glow: broad veins drifting slowly through it with a slow pulse; away from them the goo is duller. The swells lit
	// (darker troughs: retail's sludge is near white in places, so the light can only shape it by taking away).
	let vq = (p.xy - SLUDGE_FLOW * t * 1.5) * 0.07;
	let vein = 1.0 - abs(lnoise(vq + vec2<f32>(lnoise(vq * 0.5 + vec2<f32>(t * 0.01, 3.0)) * 2.0, 0.0)) * 2.0 - 1.0);
	let glow = smoothstep(0.35, 1.0, vein) * (0.85 + 0.15 * sin(t * 0.6 + dot(p.xy, vec2<f32>(0.05, 0.08))));
	let shade = clamp(1.0 + 1.6 * (dot(n, l) - l.z), 0.3, 1.15);
	var col = base * shade * mix(0.62, 1.0, glow) + base * glow * SLUDGE_GLOW * 0.3;
	// Gloss: a soft reflection of what stands around (else the air), mixed in: rock mirrored in the goo darkens it.
	let ndv = clamp(dot(n, view), 0.0, 1.0);
	let fres = 0.05 + 0.95 * pow(1.0 - ndv, 5.0);
	var r = reflect(-view, n);
	r = normalize(vec3<f32>(r.xy, max(r.z, 0.04)));
	let hit = reflect_ray(p + vec3<f32>(0.0, 0.0, 0.02), r, jit);
	var refl = env(r, base * 0.6);
	if (hit.w > 0.0) {
		refl = mix(refl, soft_scene(hit.xy, (0.006 + 0.016 * hit.z / (hit.z + dist)) * (1.0 + FAR_BLUR * smoothstep(FAR_FROM, FAR_TO, dist)), jit), hit.w);
	}
	col = mix(col, refl, clamp(fres * SLUDGE_GLOSS * 2.0, 0.0, 0.6));
	// Broad highlights of the light on the swells and domes (a light from high behind the view when there is no key light).
	let hv = normalize(l + view);
	let e = 30.0 / (1.0 + fp * 12.0);
	col += vec3<f32>(0.85, 1.0, 0.7) * pow(max(dot(n, hv), 0.0), e) * (e + 8.0) * 0.01 * (0.4 + 0.6 * sun_on);
	// A pop leaves a dark ring spreading, bright at its crest.
	col = mix(col, col * 0.45, bub.z * 0.7) + base * bub.z * bub.z * 0.5;
	return col;
}

// w.eye = (eye, seconds); w.sun = (to the key light, glint strength); w.look = (ripples, refraction in world units, minimum
// reflectance, foam); w.fog = (fog colour in linear light, reflection strength).
@fragment fn fs_water(v: VO) -> @location(0) vec4<f32> {
	let c = vec2<i32>(v.pos.xy);
	var wp = wpos_at(c);
	if (wp.a < 0.5) { wp = geo_fill(c); }
	if (wp.a < 0.5) { discard; }
	let p = wp.xyz;
	let eye = w.eye.xyz;
	// [PC] Geometric sea waves (`fs_grid`, bit 512): p is the raised surface; its flat point pf (retail's plane under or over it,
	// the height from the fraction of a) is where the picture and the bed under the water are read.
	let geo = (u32(wp.a - 0.5) & 512u) != 0u;
	let pf = vec3<f32>(p.xy, p.z - select(0.0, (fract(wp.a) / 0.45 - 0.5) * 2.0 * GEO_HMAX, geo));
	// Underwater (the camera below this surface): the retail picture.
	if (eye.z < pf.z - 0.02) { if (under_ok(wp.a)) { return under_side(c, p, pf, geo, (u32(wp.a - 0.5) & 192u) != 0u); } discard; }
	let te = eye - p;
	let dist = max(length(te), 1e-3);
	let view = te / dist;
	// Frozen sheets (Hoven's ice floor, `frozen_pool`, w.flows[..].w = -1): the same look held still (no ripple, caustic or
	// sparkle motion, no contact rings); retail's own texture motion stays in the picture.
	// The slot (a = 1 + slot, plus FLOWS on open sea: `WaterDraw::sea`).
	let slot = u32(wp.a - 0.5) & 63u;
	let sea = (u32(wp.a - 0.5) & 192u) != 0u; // (bit 128: the effect-quad sea)
	let lake = (u32(wp.a - 0.5) & 256u) != 0u; // a still lake (`WaterDraw::lake`)
	let fw = w.flows[slot].w;
	let frozen = fw < -0.5 && fw > -1.5;
	let time = select(w.eye.w, 0.0, frozen);
	let jit = dither(v.pos.xy); let svis = sky_vis(pf, jit);
	let uv0 = (vec2<f32>(c) + 0.5) * w.size.zw;
	let orig = to_linear(textureSampleLevel(scene, lsamp, uv0, 0.0).rgb);
	// Where the picture under the surface is read (geometric sea: at the flat point). A trough by the shore can have its flat point
	// behind the rock standing at the waterline: the picture there is the rock, and the water showed dark specks along every shore
	// (user screenshot 2026-10-19); the pixel's own picture then.
	let uvf = project(pf).xy;
	let uvb = sea_pic(select(uv0, uvf, geo && wet(uvf, true)), geo);
	// World size of one pixel here (wider on a grazing view of the surface).
	let fdist = far_dist(dist, view.z, geo);
	let ds = smoothstep(FAR_FROM, FAR_TO, fdist);
	let fp = dist * 2.0 * w.proj.y * w.size.w / max(view.z, mix(0.2, FAR_GRAZE, ds));
	if (fw < -1.5 && fw > -2.5) {
		return finish(c, orig, lava(c, p, view, dist, fp, jit, uv0), false);
	}
	if (fw < -3.5 && fw > -4.5) {
		return finish(c, orig, mud(c, p, view, dist, fp, uv0), false);
	}
	if (fw < -4.5 && fw > -5.5) {
		return finish(c, orig, sludge(c, p, view, dist, fp, jit, uv0), false);
	}
	let phased = fw < -2.5;
	// Flowing water (a river's scrolling texture, `WaterFx::strip_flows`): the ripples and caustics drift downstream with it.
	// w.flows[slot] = (flow x, flow y, clear (0 = water retail draws opaque: no clear shallows), cold look (0 = none)).
	let fl = w.flows[slot];
	let flow = fl.xy;
	// Cold water (Hoven's abyss, `COLD_SLOT`): long slow heavy swells that move its own picture further, a higher reflectance
	// floor (what stands in it mirrors); look 2+ lights the swell with a cool light and adds a moonlit shimmer, look 3 an icy
	// rim where the water meets the ground nearby.
	let cold = fl.w;
	// The caustics of a phased river stay in place (they would jump with the phases).
	let ft = select(flow * time, vec2<f32>(0.0), phased);
	let q = p.xy - ft;
	let rg = select(rings_at(p, fp), vec3<f32>(0.0), frozen);
	let iced = cold > 0.0;
	let sc = select(1.0, COLD_SCALE, iced);
	let rfp = max(fp * sc * (1.0 + FAR_RIPPLE * ds), select(0.0, COLD_CALM, iced));
	var rw = ripple(q * sc, time * select(1.0, COLD_PACE, iced), rfp);
	if (phased) {
		let t0 = fract(time / FLOW_CYCLE);
		let t1 = fract(t0 + 0.5);
		let w0 = 1.0 - abs(1.0 - 2.0 * t0);
		rw = mix(ripple(p.xy - flow * (t1 * FLOW_CYCLE) + vec2<f32>(3.7, 1.9), time, rfp), ripple(p.xy - flow * (t0 * FLOW_CYCLE), time, rfp), w0);
		// Two half-weight patterns cancel a little: back to one pattern's strength.
		rw *= 1.0 / sqrt(w0 * w0 + (1.0 - w0) * (1.0 - w0));
	}
	var rs = rw * (w.look.x * select(1.0, COLD_SWELL, iced)) + rg.xy;
	// [PC] Livelier water: wakes behind movers, turbulence where spray keeps falling (churn) and in fast flows (rapids), long
	// swells on open sea.
	// A phased river's flow jumps between its quads and phases: its wakes stay put.
	let wk = select(wake_at(p, fp, select(flow, vec2<f32>(0.0), phased), time), vec3<f32>(0.0), frozen);
	let ch = select(churn_at(p), 0.0, frozen);
	let rapid = smoothstep(RAPID_FROM, RAPID_TO, length(flow));
	rs += wk.xy;
	if (ch + rapid > 0.01) {
		rs += ripple(q * 1.9 + vec2<f32>(11.3, 4.7), time * 1.8, rfp) * (TURB * ch + RAPID_CHOP * rapid);
	}
	// Sea state (Water Medium / High): wind waves on open sea and lakes.
	var wh = 0.0;
	var wsd = vec2<f32>(0.0);
	// The geometric sea: the slope of the trains the grid carries (the surface the reflection ray follows).
	var gpart = vec2<f32>(0.0);
	let waved = w.wave.x > 0.5 && (sea || lake) && !iced && !frozen;
	if (geo) {
		let am = calm(pf);
		let df = max(length(eye - pf), 1e-3);
		let fpi = df * 2.0 * w.proj.y * w.size.w;
		let sw = sea_w(p.xy, time, fpi * (1.0 + select(3.0, SEA_TRAIN_FAR, geo) * smoothstep(15.0, 40.0, fdist)), fpi * (1.0 + 3.0 * smoothstep(15.0, 40.0, fdist)) * w.ggrid.w, geo_an(pf - eye), i32(w.wave.x));
		wsd = sw.g * am.y;
		gpart = sw.gg * am.x;
		rs += wsd;
		wh = sw.h / max(sw.hw, 1e-4) * am.y;
	} else if (waved) {
		let wsl = waves(p.xy, time, rfp, select(WAVE_LAKE_SCALE, 1.0, sea), i32(select(min(w.wave.x, 4.0), w.wave.x, sea)));
		wsd = wsl.yz * (select(WAVE_LAKE_AMP, 1.0, sea) * w.wave.z);
		rs += wsd;
		wh = wsl.x;
	} else if (sea && !iced) {
		rs += swell(p.xy, time);
	}
	var g = vec3<f32>(0.0, 0.0, 1.0);
	if (!geo) { g = geo_normal(c, p, dist); }
	let n = normalize(g + vec3<f32>(-rs.x, -rs.y, 0.0));
	let ndv = clamp(dot(n, view), 0.0, 1.0);
	let fresnel = max(0.02 + 0.98 * pow(1.0 - ndv, 5.0), select(w.look.z, COLD_SHEEN, cold > 0.0));
	let sun_on = clamp(w.sun.w / 2.2, 0.0, 1.0);

	// What lies under the surface: the level's depth behind the water pixel (blended water) gives the bed and its depth. Water
	// retail draws (near) opaque (fl.z = 0: Hoven's abyss and floor sheets at FIX 0x7f, the FIX 0x80 pools) has no bed: the
	// depth behind Hoven's abyss comes and goes from one frame to the next, and the whole sheet blinked between two looks
	// [C: play/compare/Hoven.racpad fields 1711-1713, 2026-10-16]. The same holds far off for all blended water (the level's
	// depth behind distant water blinks between bed and none), so the bed path fades out by `far_dist` (far_look.rs) from FAR_FROM
	// to FAR_TO (`bk`: 1 = the bed path, 0 = the opaque one) [C: flick.py on camera pans of fullgame1 0331 (Batalia sea), fg1/5_0216
	// and 5_0246 (Rilgar), 2026-10-16: far flicker 8.0 / 7.3 / 8.1 -> 3.2 / 1.0 / 1.5 (Classic 1.9 / 0.5 / 0.2)].
	let zw = project(pf).z;
	let zs = scene_z(uvb);
	// [PC] Geometric sea: the picture at the flat point holds retail's flat sea, which wrote its depth. Read a fraction of a pixel
	// off pf, that depth lay a little behind or before zw (the grazing slope across a pixel), `zs > zw + 0.05` came and went with
	// the waves and the near sea switched between the bed path (a bed on the sea itself: depth 0, full shore foam, caustics) and
	// the opaque one in contour bands that changed every frame (ragged bands, thin streaks; 2026-10-20). The sea itself is no bed.
	let blended = fl.z > 0.5 && ds < 1.0 && zs > zw + 0.05 && (geo || zs < w.proj.w * 0.98) && !(geo && on_flat_sea(uvb, zs));
	let bk = select(0.0, 1.0 - ds, blended);
	let bed = bed_at(eye, pf, zs, zw, geo);
	let depth = select(0.0, clamp(pf.z - bed.z, 0.0, 40.0), blended);
	let thick = depth * bk;
	let sfade = select(1.0, shore_fade(project(pf).xy, pf, zw), w.mirror.w > 0.0 && fl.z > 0.5 && geo);

	// Refraction: the picture under the surface, shifted by the ripple slope (more through deeper water); only from water.
	let push = w.look.y * (0.5 + min(thick, 4.0) * 0.25) * select(1.0, COLD_PUSH, cold > 0.0);
	// (Geometric sea: the shift capped at GEO_PUSH_MAX, so steep waves do not swirl the bed or pull in what stands by the shore.)
	var shift = rs * push * sfade / (1.0 + max(fp - 0.02, 0.0) * 25.0);
	if (geo) { shift *= min(1.0, GEO_PUSH_MAX / max(length(shift), 1e-4)); }
	var uv = project(pf + vec3<f32>(shift, 0.0)).xy;
	if (!wet(uv, geo)) {
		uv = mix(uvb, uv, 0.4);
		if (!wet(uv, geo)) { uv = uvb; }
	}
	//DBGHOOK
	var base = to_linear(textureSampleLevel(scene, lsamp, uv, 0.0).rgb);

	// Caustics: moving light lines on the bed (on the surface itself where the water hides its bed), brightening the colour
	// already there so the water keeps its tint; strongest in sunlight and shallow water.
	let cb = bed.xy - ft + rs * (0.6 + thick * 0.3);
	var cl: f32;
	if (bk >= 1.0) {
		cl = caustic(cb, time, fp * zs / zw);
	} else {
		cl = caustic(q + rs * 0.6, time, fp);
		if (bk > 0.0) {
			cl = mix(cl, caustic(cb, time, fp * zs / zw), bk);
		}
	}
	let cstr = (0.25 + 0.75 * sun_on) * svis * select(mix(0.18, 0.8 * exp(-thick * 0.22), bk), 0.8 * exp(-thick * 0.22), bk >= 1.0);
	base += base * (cl - 0.3) * cstr;
	// Depth tint: deeper water turns towards a darker, richer version of its own colour.
	let lb = luma(base);
	let deep = max(vec3<f32>(lb) + (base - vec3<f32>(lb)) * 1.4, vec3<f32>(0.0)) * 0.7;
	base = mix(base, deep, (1.0 - exp(-thick * 0.12)) * 0.5);

	// Clear shallows: the bed (the level without its water, mirror.rs `draw_bed`) through the surface at the refracted uv, tinted
	// by the water's own hue and lit by the caustics; the path through the water absorbs it into the retail colour above, so
	// deep water (and water with nothing under it) keeps its own colour. w.mirror.w = how clear (CLEAR).
	// [PC] The bed is read bilinear and the shallows reach depth 0 at the waterline (shore_fade.rs).
	let buv = bed_uv(uv, uvb, zw, geo);
	let zb = bed_lz(buv, geo);
	if (zb < w.proj.w * 0.98 && w.mirror.w > 0.0 && shallows_k(zb, pf, zw, geo) > 0.0) {
		let bp = eye + (pf - eye) * (max(zb, zw) / zw);
		let path = max(zb / zw - 1.0, 0.0) * length(pf - eye);
		let hue = base / max(max(base.r, max(base.g, base.b)), 1e-3);
		var shallow = to_linear(textureSampleLevel(bed_tex, lsamp, buv, 0.0).rgb) * mix(vec3<f32>(1.0), hue, 0.6);
		let bt = clamp(pf.z - bp.z, 0.0, 40.0);
		shallow += shallow * (caustic(bp.xy - ft + rs * (0.6 + bt * 0.3), time, fp * zb / zw) - 0.3) * (0.25 + 0.75 * sun_on) * svis * 0.9 * exp(-bt * 0.2);
		base = mix(base, shallow, exp(-path * ABSORB) * w.mirror.w * fl.z * shallows_k(zb, pf, zw, geo));
	}

	// Reflection: the ray follows the smooth surface (so neighbouring pixels agree), the ripples then bend where it is read
	// and the full ripple normal picks the far air; softened more the further the reflected thing is from the water.
	let gsm = normalize(g + vec3<f32>(-gpart, 0.0));
	var rr = reflect(-view, normalize(gsm + vec3<f32>(-(rs - gpart), 0.0) * 0.15));
	rr = normalize(vec3<f32>(rr.xy, max(rr.z, 0.04)));
	var r = reflect(-view, n);
	r = normalize(vec3<f32>(r.xy, max(r.z, 0.04)));
	let hit = reflect_ray(p + gsm * 0.02, rr, jit);
	var refl = env(r, base);
	// How sure the reflection is (a mirror image or a screen-space hit): sure reflections may mirror more at grazing views.
	var sure = hit.w;
	if (hit.w > 0.0) {
		let bend = project(p + vec3<f32>(-(rs + wsd * WAVE_BEND) * 0.5 * min(hit.z, 4.0) * (1.0 - FAR_BEND * ds), 0.0)).xy - project(p).xy;
		let rad = (0.002 + 0.014 * hit.z / (hit.z + dist)) * (1.0 + FAR_BLUR * ds);
		let huv = clamp(hit.xy + bend, vec2<f32>(0.0), vec2<f32>(1.0));
		refl = mix(refl, soft_scene(huv, rad, jit), hit.w);
	}
	// Flat water on a mirrored plane (mirror.rs; w.mirror = (height, on), w.mirror2 = (2nd height, 3rd height, planes mirrored)):
	// the true mirror image at this pixel, bent by the ripples and softened by the pixel's footprint, in place of the
	// screen-space reflection. The layer is the one whose plane lies within 0.4 of the pixel (the nearest if two do).
	var ml = -1;
	var mbest = 0.4;
	if (w.mirror.y > 0.5 && g.z > 0.96) {
		if (abs(pf.z - w.mirror.x) < mbest) {
			ml = 0;
			mbest = abs(pf.z - w.mirror.x);
		}
		if (w.mirror2.z > 1.5 && abs(pf.z - w.mirror2.x) < mbest) {
			ml = 1;
			mbest = abs(pf.z - w.mirror2.x);
		}
		if (w.mirror2.z > 2.5 && abs(pf.z - w.mirror2.y) < mbest) {
			ml = 2;
			mbest = abs(pf.z - w.mirror2.y);
		}
	}
	if (ml >= 0) {
		// (Geometric sea: read at the flat point's pixel, where the mirror image of this spot lies.)
		let muv = uvb + mirror_push(c) + project(p + vec3<f32>(-(rs + wsd * WAVE_BEND) * 0.25 * (1.0 - FAR_BEND * ds), 0.0)).xy - project(p).xy;
		let mr = (0.5 + min(fp * 4.0, 2.0)) * w.size.zw * 2.0 * (1.0 + FAR_BLUR * ds);
		let a = mix(jit, 0.5, smoothstep(12.0, 30.0, dist)) * 6.2832; // far: no dither (it showed as a diagonal hatch on Kalebo's sea)
		let o = vec2<f32>(cos(a), sin(a));
		var m = vec3<f32>(0.0);
		for (var k = 0; k < 4; k++) {
			let ok = select(o, vec2<f32>(-o.y, o.x), (k & 1) == 1) * select(1.0, -1.0, k >= 2);
			m += to_linear(textureSampleLevel(mirror, lsamp, clamp(muv + ok * mr, vec2<f32>(0.0), vec2<f32>(1.0)), ml, 0.0).rgb);
		}
		refl = m * 0.25;
		sure = 1.0;
	}
	if (cold > 0.0) {
		refl = max(refl, base * COLD_FLOOR);
	}
	var col = mix(base, refl, clamp(steep_fresnel(fresnel, view.z, ndv, geo) * w.fog.w, 0.0, mix(0.65, 0.75, sure)));
	// Keep the water's own hue (a reflection of grey walls or fog must not grey a teal pool); less so for a sure reflection.
	let mx = max(base.r, max(base.g, base.b));
	let sat = (mx - min(base.r, min(base.g, base.b))) / max(mx, 1e-4);
	col = mix(col, base * (luma(col) / max(luma(base), 1e-4)), 0.4 * sat * (1.0 - 0.3 * sure));
	// [PC] Milky water (`OPAQUE_SLOT`: flows (0, 0, 0, 0), Pokitaru's waterfall ponds) scatters the light under its surface: a
	// reflection only brightens it (the mirrored dark foot of a rock standing in it drew a black band round it, user 2026-10-17).
	if (fl.z < 0.01 && fl.w == 0.0) {
		col = max(col, base);
	}

	// Sun glint and sparkles along the key light (none with the light below the horizon: w.sun.w = 0): a sharp highlight
	// that softens with distance instead of flickering.
	let l = w.sun.xyz;
	let hv = normalize(l + view);
	let sn = normalize(n + vec3<f32>(-sparkle_slope(q, time, fp), 0.0));
	let e = 900.0 / (1.0 + fp * 60.0);
	let twinkle = 0.25 + 0.75 * smoothstep(0.7, 0.95, vnoise(q * 5.0 + vec2<f32>(time * 1.3, -time * 0.9)));
	col += vec3<f32>(1.0, 0.97, 0.9) * pow(max(dot(sn, hv), 0.0), e) * (e + 8.0) * 0.0004 * twinkle * w.sun.w * svis;
	if (cold >= 2.0) {
		// Cold water: a cool light from beyond the water, 25 degrees up along the view's heading (no light of the level reaches
		// the cave). The swell brightens where it faces it and darkens where it turns away (its mean stays the water's colour),
		// and a soft moonlit shimmer runs along the waves.
		let hd = normalize(vec2<f32>(-view.x, -view.y) + vec2<f32>(1e-4, 0.0));
		let lc = vec3<f32>(hd * 0.906, 0.423);
		col *= max(1.0 + COLD_SHADE * (dot(n, lc) - dot(g, lc)), 0.0);
		let ec = 60.0 / (1.0 + fp * 20.0);
		col += vec3<f32>(0.7, 0.9, 1.0) * pow(max(dot(n, normalize(lc + view)), 0.0), ec) * (ec + 8.0) * 0.0004 * (0.5 + 0.5 * twinkle) * COLD_SHIMMER;
	}

	// Sea state: the wave faces turned to the light (the key light, else from behind the view) are brighter, those turned away
	// darker (the mean stays the water's colour); High: crests catch a little more light than troughs.
	if (waved) {
		let lh = select(normalize(-view.xy + vec2<f32>(1e-4, 0.0)), normalize(w.sun.xy + vec2<f32>(1e-4, 0.0)), w.sun.w > 0.0);
		col *= max(1.0 + select(WAVE_FACE, GEO_FACE, geo) * steep_face(view.z, geo) * dot(wsd, lh) + select(0.0, WAVE_SHADE * wh, w.wave.y > 0.5) + steep_crest(view.z, wh, geo), 0.0);
	}

	col = shore_bed(col, uv0, sfade);
	// Foam where the water meets the ground.
	var foam = 0.0;
	if (bk > 0.0) {
		// Blended water over the ground: shallow near the shore (the water's own depth there, so a pool wall seen at a grazing
		// angle keeps a thin line instead of a broad band).
		foam = 1.0 - smoothstep(0.0, 0.3, depth);
	}
	if (bk < 1.0) {
		// The water wrote its own depth: geometry cutting the surface at the same depth marks the shore.
		let px = clamp(0.18 / (zw * 2.0 * w.proj.y * w.size.w), 1.5, 9.0);
		var hits = 0.0;
		for (var k = 0; k < 8; k++) {
			let a = (f32(k) + jit) * 0.785398;
			let q = c + vec2<i32>(vec2<f32>(cos(a), sin(a)) * px);
			// [PC] Geometric sea: a hole of the projected grid (a fold at a crest; `geo_gap` fills it) is water, not ground: counted
			// as ground it drew foam contour lines that moved with the crests every frame (thin flickering streaks, ragged shore
			// bands, 2026-10-20).
			if (wpos_at(q).a < 0.5 && !(geo && geo_gap(q).a > 0.5)) {
				let zq = scene_z((vec2<f32>(q) + 0.5) * w.size.zw);
				if (abs(zq - zw) < 0.06 * zw + 0.25) { hits += 1.0; }
			}
		}
		foam = mix(clamp(hits / 3.0, 0.0, 1.0), foam, bk);
	}
	// [PC] Geometric sea: surf along every shore of the shore map (beaches, rocks, posts: ground up to SURF_DEEP under the surface or
	// above it), SURF_W wide on the map whatever the slope (a gentle beach would foam far out by depth alone), reaching out and
	// back with the waves.
	var surf = 0.0;
	if (geo) {
		let s = shore_at(pf.xy);
		let under = pf.z - s.x;
		// [PC] Where no ground is seen under the water, by the distance to the shore (the seen cells' weight drew square patches).
		let seen = smoothstep(0.0, 0.25, s.y);
		let reach = SURF_W * (1.0 + SURF_PULSE * clamp(wh, -1.0, 1.0));
		if (seen < 1.0) {
			surf = (1.0 - smoothstep(0.2 * reach, reach, shore_dist(pf.xy).x)) * (1.0 - seen);
		}
		if (s.y > 0.0 && under < SURF_DEEP) {
			let e = vec2<f32>(w.gmap.z, 0.0);
			let gr = vec2<f32>(shore_at(pf.xy + e.xy).x - shore_at(pf.xy - e.xy).x, shore_at(pf.xy + e.yx).x - shore_at(pf.xy - e.yx).x) / (2.0 * w.gmap.z);
			let off = max(under, 0.0) / max(length(gr), 0.05);
			surf += (1.0 - smoothstep(0.2 * reach, reach, off)) * (1.0 - smoothstep(0.6 * SURF_DEEP, SURF_DEEP, under)) * seen;
		}
		if (surf > 0.0) {
			if (w.hero.w > 0.5 && w.hero.z > pf.z - HERO_DEEP && w.hero.z < pf.z + HERO_HIGH) {
				surf *= smoothstep(SURF_HERO0, SURF_HERO1, length(pf.xy - w.hero.xy));
			}
		}
	}
	let bw0 = 1.0 - smoothstep(0.15, 0.5, fp);
	let bw1 = 1.0 - smoothstep(0.06, 0.22, fp);
	let breakup = mix(BREAKUP_MID, vnoise(p.xy * 2.7 + vec2<f32>(time * 0.35, time * -0.2)), bw0) * 0.6 + mix(BREAKUP_MID, vnoise(p.xy * 6.1 - vec2<f32>(time * 0.5, 0.0)), bw1) * 0.4;
	// Soft and in the water's own colour: a lighter froth, not a white rim.
	// [PC] Rocks and banks in a fast flow or under falling spray foam more.
	foam = smoothstep(0.45, 0.95, foam * sfade * (0.3 + 1.1 * breakup) * (1.0 + 0.8 * rapid + ch)) * w.look.w * 0.55;
	if (cold >= 3.0) {
		// Cold water: ice forming where it meets the ground nearby (far off the shore test spreads over the cliffs), pale.
		col = mix(col, mix(base, COLD_ICE, 0.6), clamp(foam * COLD_RIM * (1.0 - smoothstep(0.04, 0.1, fp)), 0.0, 1.0));
	} else {
		col = mix(col, min(base * 1.5 + vec3<f32>(0.05), vec3<f32>(0.7)), foam * FOAM_SHORE);
	}
	if (surf > 0.0) {
		col = mix(col, min(base * 1.7 + vec3<f32>(0.12), vec3<f32>(0.88)), smoothstep(0.4, 0.9, surf * sfade * (0.25 + 1.1 * breakup)) * SURF_ALPHA * FOAM_SHORE);
	}
	// The rings' froth crest: whiter than the shore foam while young, broken up the same way.
	col = mix(col, min(base * 1.8 + vec3<f32>(0.12), vec3<f32>(0.85)), clamp(rg.z * (0.3 + 0.9 * breakup), 0.0, 0.8) * mix(0.3, 1.0, smoothstep(0.1, 0.4, dot(base, vec3<f32>(0.2126, 0.7152, 0.0722)))));
	// [PC] Churned water: the wake's foam, whitewater lace where spray keeps falling and streaks along a fast flow; it drifts
	// with the water and fades.
	let fd = normalize(flow + vec2<f32>(1e-5, 0.0));
	let streak = smoothstep(0.5, 0.85, lnoise(vec2<f32>(dot(q, fd) * 0.5, dot(q, vec2<f32>(-fd.y, fd.x)) * 3.0)));
	let white = wk.z * (0.6 + 0.6 * breakup) * FOAM_WAKE + ch * CHURN_FOAM + rapid * RAPID_FOAM * streak;
	if (white > 0.01) {
		let lc = lace(q, time, fp);
		// On bright water (Pokitaru's milky ponds) the brightened colour is no whiter than the water: the foam goes white there.
		let fc = mix(min(base * 1.6 + vec3<f32>(0.22), vec3<f32>(0.9)), vec3<f32>(0.95), smoothstep(0.3, 0.6, dot(base, vec3<f32>(0.2126, 0.7152, 0.0722))));
		col = mix(col, fc, clamp(white * (0.25 + 0.95 * lc), 0.0, 0.85));
	}

	// Sea state, High, Pokitaru's open sea only (the effect-quad sea, not rivers, rain water or other seas): thin sparse whitecaps on
	// the very highest crests of the big waves, only on bright water, thinning out far off.
	let bl = dot(base, vec3<f32>(0.2126, 0.7152, 0.0722));
	if (waved && (u32(wp.a - 0.5) & 128u) != 0u && w.wave.y > 0.5) {
		let cap = smoothstep(WAVE_CAP_FROM + w.wave.w, WAVE_CAP_TO + w.wave.w, wh) * (1.0 - ds) * lace(q, time * 0.5, fp) * smoothstep(WAVE_CAP_LUMA, WAVE_CAP_LUMA + 0.15, bl);
		let fc = mix(min(base * 1.5 + vec3<f32>(0.06), vec3<f32>(0.9)), vec3<f32>(0.95), smoothstep(0.3, 0.6, bl));
		col = mix(col, fc, clamp(cap * WAVE_CAP, 0.0, 0.5));
	}

	// Soft mask edge and particle cover (`finish`).
	return finish(c, orig, col, geo);
}
// [PC] Waterfalls (`WaterFx::sheet_falls`, `fs_fall`), drawn over the shaded picture. Pokitaru's sheets replace retail's own
// (additive, glassy) draw; Novalis's steep strips and Batalia's steep river quads are drawn over theirs. The body: aerated
// white water in three layers of falling streaks (FALL_V units a second down the sheet, FALL_L1..3 cells a unit along,
// FALL_ACROSS cells a uv unit across), thin and see-through between them (FALL_THIN), torn into strands where retail's own
// cover fades out at a ribbon's side (FALL_FRAY); a glassy bright band where it bends over the lip (FALL_LIP); white froth
// within FALL_FOOT units above a churning foot, and foam where the sheet spreads flat over the water. At each foot: a
// boiling foam disc drifting outward (FALL_BOIL), mist rising and drifting away (FALL_MIST) and spray drops thrown out in
// arcs (FALL_DROP_*).
const FALL_ACROSS: f32 = 40.0;
const FALL_V: f32 = 7.0;
const FALL_L1: f32 = 0.22;
const FALL_L2: f32 = 0.6;
const FALL_L3: f32 = 1.6;
const FALL_THIN: f32 = 0.35;
const FALL_THICK: f32 = 0.97;
const FALL_AER: vec2<f32> = vec2<f32>(0.3, 0.75);
const FALL_FRAY: f32 = 0.3;
const FALL_LIP: f32 = 0.35;
const FALL_FOOT: f32 = 3.0;
const FALL_MIST: f32 = 0.1;
const FALL_MIST_LIFE: f32 = 4.5;
const FALL_MIST_RISE: f32 = 5.0;
const FALL_MIST_DRIFT: f32 = 3.0;
const FALL_BOIL: f32 = 0.8;
const FALL_BOIL_RATE: f32 = 0.35;
const FALL_DROP_LIFE: f32 = 0.9;
const FALL_DROP_SPEED: f32 = 2.0;
const FALL_DROP_UP: f32 = 3.5;
const FALL_DROP_BLUR: f32 = 0.02;
const FALL_DROP_A: f32 = 0.5;
const FALL_G: f32 = 9.8;
const FALL_NEAR: vec2<f32> = vec2<f32>(3.0, 10.0);
const FALL_WHITE: vec3<f32> = vec3<f32>(0.88, 0.95, 1.0);
const FALL_CLEAR: vec3<f32> = vec3<f32>(0.5, 0.68, 0.72);
const FALL_TEX_V: f32 = 0.45;
const FALL_TEX_GAIN: f32 = 1.3;
const FALL_GLOW: f32 = 0.15;
// [PC] Pokitaru's sheets (`shape_sheets`): retail's own flat look FALL_BRINK units (from..to) above the brink; thick smooth
// water bending over it (FALL_TONGUE colour, FALL_GLASS cover, FALL_SHEEN streaks of light, a FALL_CREST bright crest on
// the bend) that tears into white by FALL_ONSET units below it.
const FALL_BRINK: vec2<f32> = vec2<f32>(0.6, 2.0);
const FALL_TONGUE: vec3<f32> = vec3<f32>(0.42, 0.72, 0.7);
const FALL_GLASS: f32 = 0.7;
const FALL_SHEEN: f32 = 0.35;
const FALL_CREST: f32 = 0.45;
const FALL_ONSET: f32 = 1.2;

struct FO { @builtin(position) pos: vec4<f32>, @location(0) world: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) col: vec4<f32>, @location(3) @interpolate(flat) kind: vec3<f32>, @location(4) shape: vec4<f32> };
// Retail's waterfall texture (`fall_fx_tex`; a blank one elsewhere).
@group(1) @binding(0) var fall_tex: texture_2d<f32>;
@group(1) @binding(1) var fall_smp: sampler;

// A sheet vertex as retail draws it (n.x = 0; n.z = 1: its fall runs along u, not v), or a corner (uv -1..1) of one of a
// foot's quads at p (n.x = 1 mist puff, 2 spray drop, 3 boil disc; n.y its size, n.z its seed 0..1). Puffs and drops move
// here, looping on the clock: puffs rise and drift away growing, drops fly out and fall back, stretched along their motion.
// sh: a sheet vertex's shape (`shape_sheets`; z = 0 where unknown).
@vertex fn vs_fall(@location(0) p: vec3<f32>, @location(1) uv: vec2<f32>, @location(2) n: vec3<f32>, @location(3) c: vec4<f32>, @location(4) sh: vec4<f32>) -> FO {
	var q = p;
	var o: FO;
	o.col = c;
	let t = w.eye.w;
	if (n.x > 2.5) {
		q = p + vec3<f32>(uv * n.y, 0.03);
	} else if (n.x > 0.5) {
		let d = normalize(p - w.eye.xyz);
		var r = normalize(cross(d, vec3<f32>(0.0, 0.0, 1.0)) + vec3<f32>(1e-4, 0.0, 0.0));
		var up = cross(r, d);
		let ang = n.z * 6.2831 * 7.0;
		let dir = vec3<f32>(cos(ang), sin(ang), 0.0);
		if (n.x < 1.5) {
			let ph = fract(t / FALL_MIST_LIFE + n.z);
			let size = n.y * (0.6 + 0.8 * ph);
			q = p + dir * (0.5 + FALL_MIST_DRIFT * ph) + vec3<f32>(0.0, 0.0, FALL_MIST_RISE * ph) + (r * uv.x + up * uv.y) * size;
			o.col = vec4<f32>(c.rgb, sin(ph * 3.14159));
		} else {
			let life = FALL_DROP_LIFE * (0.7 + 0.6 * fract(n.z * 13.1));
			let ph = fract(t / life + n.z);
			let tt = ph * life;
			let v0 = dir * FALL_DROP_SPEED * (0.5 + fract(n.z * 7.7)) + vec3<f32>(0.0, 0.0, FALL_DROP_UP * (0.6 + 0.8 * fract(n.z * 3.3)));
			let at = p + v0 * tt + vec3<f32>(0.0, 0.0, -0.5 * FALL_G * tt * tt);
			let vel = v0 + vec3<f32>(0.0, 0.0, -FALL_G * tt);
			let ax = vel - d * dot(vel, d);
			let al = length(ax);
			if (al > 1e-3) {
				up = ax / al;
				r = normalize(cross(d, up));
			}
			q = at + r * uv.x * n.y + up * uv.y * (n.y + al * FALL_DROP_BLUR);
			// Gone once back under the water.
			o.col = vec4<f32>(c.rgb, (1.0 - ph) * step(p.z - 0.1, at.z));
		}
	}
	o.pos = w.vp * vec4<f32>(q, 1.0);
	o.world = q;
	o.uv = uv;
	o.kind = n;
	o.shape = sh;
	return o;
}

// Value noise repeating every `per` cells.
fn tnoise(p: vec2<f32>, per: vec2<f32>) -> f32 {
	let i = floor(p);
	let f = p - i;
	let u = f * f * (3.0 - 2.0 * f);
	let i0 = i - per * floor(i / per);
	let i1 = i0 + vec2<f32>(1.0) - per * floor((i0 + vec2<f32>(1.0)) / per);
	return mix(mix(hash(i0), hash(vec2<f32>(i1.x, i0.y)), u.x), mix(hash(vec2<f32>(i0.x, i1.y)), hash(i1), u.x), u.y);
}

// One streak layer: `l` cells a unit along, scrolling down at `v` units a second (the scroll wraps every 64 cells, where
// the noise repeats, so the clock never grows its coordinates).
fn fall_layer(x: f32, z: f32, l: f32, v: f32, t: f32) -> f32 {
	return tnoise(vec2<f32>(x, z * l + fract(t * v * l / 64.0) * 64.0), vec2<f32>(4096.0, 64.0));
}

// Foam on the boil disc, drifting out from its middle (two phases of FALL_BOIL_RATE a second, crossfaded).
fn boil_foam(xy: vec2<f32>, o: vec2<f32>, t: f32) -> f32 {
	let f0 = fract(t * FALL_BOIL_RATE);
	let f1 = fract(t * FALL_BOIL_RATE + 0.5);
	let a0 = vnoise(xy * 1.5 - o * f0 * 2.5) * 0.6 + vnoise(xy * 3.4 - o * f0 * 3.0 + 7.0) * 0.4;
	let a1 = vnoise(xy * 1.5 - o * f1 * 2.5 + 13.0) * 0.6 + vnoise(xy * 3.4 - o * f1 * 3.0 + 21.0) * 0.4;
	let w0 = 1.0 - abs(2.0 * f0 - 1.0);
	return a0 * w0 + a1 * (1.0 - w0);
}

// A waterfall point under the water drawn in front of it at pixel c (the water pass's positions).
fn fall_under(c: vec2<i32>, p: vec3<f32>, below: f32) -> bool {
	let wp = wpos_at(c);
	return wp.w > 0.5 && p.z < wp.z - below && distance(wp.xyz, w.eye.xyz) < distance(p, w.eye.xyz);
}

// Premultiplied over the picture.
@fragment fn fs_fall(v: FO) -> @location(0) vec4<f32> {
	let p = v.world;
	let nrm = cross(dpdx(p), dpdy(p));
	// Across the fall in cells, and how many cells / units a pixel spans (before any branch: derivatives).
	let gx = select(v.uv.x, v.uv.y, v.kind.z > 0.5) * FALL_ACROSS;
	let fwx = fwidth(gx);
	let fwz = fwidth(p.z);
	let c = vec2<i32>(v.pos.xy);
	let zs = linear_depth(textureLoad(depth_ms, c, 0));
	let zw = linear_depth(v.pos.z);
	let t = w.eye.w;
	// Retail's texture (sheets with n.y = 2) at its own scroll, and again finer and faster on top.
	let k1 = textureSample(fall_tex, fall_smp, v.uv);
	let k2 = textureSample(fall_tex, fall_smp, v.uv * vec2<f32>(1.6, 1.3) + vec2<f32>(0.37, fract(t * FALL_TEX_V)));
	if (v.kind.x > 2.5) {
		// Only on the water it lies on.
		let r = length(v.uv);
		let wp = wpos_at(c);
		if (r > 1.0 || zw > zs * 1.004 + 0.05 || wp.w < 0.5 || abs(wp.z - p.z) > 1.5) { discard; }
		let foam = smoothstep(0.4, 0.65, boil_foam(p.xy, v.uv / max(r, 1e-3), t) + (1.0 - r) * 0.45) * (1.0 - smoothstep(0.5, 1.0, r));
		let a = foam * FALL_BOIL * (0.5 + 0.5 * (1.0 - r));
		return vec4<f32>(FALL_WHITE * a, a);
	}
	if (v.kind.x > 1.5) {
		if (zw > zs || fall_under(c, p, 0.05)) { discard; }
		let a = (1.0 - v.uv.x * v.uv.x) * (1.0 - v.uv.y * v.uv.y) * v.col.a * FALL_DROP_A * smoothstep(1.0, 3.0, zw);
		return vec4<f32>(FALL_WHITE * a, a);
	}
	if (v.kind.x > 0.5) {
		let r2 = dot(v.uv, v.uv);
		if (r2 > 1.0 || zw > zs || fall_under(c, p, 0.3)) { discard; }
		// Soft where it meets the level, and gone right in front of the eye.
		let soft = clamp((zs - zw) / 2.0, 0.0, 1.0) * smoothstep(1.5, 6.0, zw);
		let billow = vnoise(v.uv * 1.7 + vec2<f32>(v.kind.z * 73.0, -t * 0.45)) * 0.6 + vnoise(v.uv * 3.9 + vec2<f32>(t * 0.2, v.kind.z * 31.0 - t * 0.8)) * 0.4;
		let a = (1.0 - r2) * (1.0 - r2) * soft * smoothstep(0.25, 0.75, billow) * FALL_MIST * v.col.a;
		return vec4<f32>(FALL_WHITE * a, a);
	}
	if (zw > zs * 1.004 + 0.05) { discard; }
	// 0 on a flat stretch of the sheet, 1 where it falls straight down.
	let s = 1.0 - abs(nrm.z) / max(length(nrm), 1e-9);
	let fall = smoothstep(0.5, 0.85, s);
	let flat = 1.0 - smoothstep(0.35, 0.6, s);
	let lip = smoothstep(0.35, 0.6, s) * (1.0 - smoothstep(0.7, 0.9, s));
	var foot = 0.0;
	for (var k = 0; k < i32(w.rn.z); k++) {
		let ch = w.churn[2 * k];
		let h = p.z - ch.z;
		let dh = length(p.xy - ch.xy);
		foot = max(foot, w.churn[2 * k + 1].x * (1.0 - smoothstep(0.0, FALL_FOOT, h)) * step(-0.5, h) * (1.0 - smoothstep(ch.w, ch.w + 2.5, dh)));
	}
	// Streaks finer than about two pixels fade to their mean instead of shimmering.
	let d2 = 1.0 - smoothstep(0.35, 0.8, max(fwx * 0.6, fwz * FALL_L2));
	let d3 = 1.0 - smoothstep(0.35, 0.8, max(fwx * 1.5, fwz * FALL_L3));
	let n1 = fall_layer(gx * 0.25, p.z, FALL_L1, FALL_V, t);
	let n2 = mix(0.5, fall_layer(gx * 0.6 + 17.0, p.z, FALL_L2, FALL_V * 1.25, t), d2);
	let n3 = mix(0.5, fall_layer(gx * 1.5 + 5.0, p.z, FALL_L3, FALL_V * 1.5, t), d3);
	let tl = (luma(k1.rgb) * min(k1.a * 2.0, 1.0) * 0.55 + luma(k2.rgb) * min(k2.a * 2.0, 1.0) * 0.45) * FALL_TEX_GAIN;
	// Retail's texture is mostly dark (made to be added): its streaks only shade a mostly solid body.
	let dens = select(n1 * 0.5 + n2 * 0.33 + n3 * 0.17, clamp(0.45 + 0.55 * smoothstep(0.05, 0.6, tl) + 0.35 * (n1 - 0.5), 0.0, 1.0), v.kind.y > 1.5);
	let a = clamp(v.col.a / 1.4, 0.0, 1.0);
	let lit = clamp(v.col.rgb * 2.0, vec3<f32>(0.0), vec3<f32>(1.25)) * 0.95;
	let under = select(1.0, 0.3, fall_under(c, p, 0.05));
	let near = mix(0.35, 1.0, smoothstep(FALL_NEAR.x, FALL_NEAR.y, zw)) * under;
	if (v.shape.z > 0.5) {
		// Pokitaru's sheets, shaped per vertex (`shape_sheets`): steepness and distance d from the brink (< 0 on the water
		// running to it) vary smoothly over the sheet, so its facets never show.
		let ss = v.shape.x;
		let d = v.shape.y;
		let sfall = smoothstep(0.45, 0.8, ss);
		let sflat = (1.0 - smoothstep(0.25, 0.5, ss)) * max(smoothstep(FALL_BRINK.x, FALL_BRINK.y, -d), v.shape.w);
		// The water tears up unevenly: in some columns the white starts right at the brink, in others lower down.
		let tear = tnoise(vec2<f32>(gx * 0.3, fract(t * 0.004) * 64.0), vec2<f32>(4096.0, 64.0));
		let onset = smoothstep(-0.5, FALL_ONSET, d + (tear - 0.5) * FALL_ONSET);
		// Over the brink: streaks of light running down the smooth water (along d), a bright crest on the bend.
		let sheen = smoothstep(0.6, 0.85, fall_layer(gx * 0.7 + 3.0, -d, 0.35, FALL_V * 0.4, t)) * d2;
		let crest = exp(-d * d * 6.0) * (0.6 + 0.4 * n2);
		let aer = clamp(sfall * smoothstep(FALL_AER.x, FALL_AER.y, dens) * onset + foot * 0.9, 0.0, 1.0);
		let cov = smoothstep(0.0, FALL_FRAY, a * 1.25 - (1.0 - dens) * 0.9 * onset + 0.15 + (1.0 - onset) * 0.3);
		// Between the white streaks: the thick smooth water near the brink, thin clear water further down.
		let glass = FALL_TONGUE * lit * (0.85 + 0.15 * n1) + FALL_WHITE * lit * (sheen * FALL_SHEEN + crest * FALL_CREST);
		let thin = mix(glass, FALL_CLEAR * lit * (0.85 + 0.15 * dens), onset);
		let col = mix(thin, FALL_WHITE * lit * (0.85 + 0.15 * dens), aer);
		let alpha = cov * mix(mix(FALL_GLASS + 0.15 * sheen + 0.15 * crest, FALL_THIN, onset), FALL_THICK, aer) * near;
		let sheet = vec4<f32>(col * alpha + FALL_WHITE * lit * dens * cov * FALL_GLOW * onset * under, alpha);
		return mix(sheet, vec4<f32>(k1.rgb * k1.a * v.col.rgb * v.col.a, 0.0), sflat);
	}
	// Retail's cover (0 at a ribbon's outer side) torn by the streaks; the lip holds together.
	let cov = smoothstep(0.0, FALL_FRAY, a * 1.25 - (1.0 - dens) * 0.9 + 0.15 + lip * 0.3);
	let aer = clamp(fall * smoothstep(FALL_AER.x, FALL_AER.y, dens) + lip * 0.15 + foot * 0.9, 0.0, 1.0);
	// Lit as retail tints the sheet (0x40 = 1.0, `lit`); the same on every facet, so the sheet's corners do not show.
	var col = mix(FALL_CLEAR, FALL_WHITE, aer) * lit * (0.85 + 0.15 * dens) + FALL_WHITE * lip * FALL_LIP * smoothstep(0.5, 0.8, dens);
	var alpha = cov * mix(FALL_THIN, FALL_THICK, aer);
	// Thinner right in front of the eye (walking through a fall), and seen through the water below its surface.
	alpha *= near;
	// Premultiplied, plus a little of retail's glow; where the sheet spreads flat over the water, retail's own additive
	// glow (`fs_part`: texture x vertex colour, added times its alpha; the foam disc covers the foot).
	let sheet = vec4<f32>(col * alpha + FALL_WHITE * lit * dens * cov * FALL_GLOW * under, alpha);
	let retail = vec4<f32>(select(vec3<f32>(dens * 0.5), k1.rgb * k1.a, v.kind.y > 1.5) * v.col.rgb * v.col.a, 0.0);
	return mix(sheet, retail, flat);
}
"#;
