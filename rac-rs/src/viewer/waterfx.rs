//! [PC] Water Enhanced (Pause -> Options -> Graphics -> Water, with Lighting Enhanced; retail/pcext.rs `graphics`; no retail
//! counterpart). The water itself stays retail: its meshes, wave motion, textures and blending are drawn by the level pass as
//! before, and Water Classic leaves the frame untouched.
//!
//! The level pass hands over the vertex and index buffers of every water draw it made (Novalis and Drek fleet water quads,
//! the Hoven queue-C pools, the Aridia / Kerwan / Batalia / Oltanis oceans and the Rilgar / Umbris / Gaspar RAM oceans). Then:
//! - `fs_pos`: those meshes again, into a full-size world-position target (nearest surface wins; a fragment hidden behind the
//!   level's depth is dropped, so whatever stands in front of the water keeps it);
//! - `fs_water`, inside the post chain after the occlusion and before the bloom (so glints bloom), on water pixels only: a
//!   normal from the surface's own shape (smoothed over a few pixels, so the mesh's triangles do not show) plus ten animated
//!   wave trains on world XY that fade out before they get finer than a few pixels; a refraction wobble of the picture under
//!   it; moving caustic light on the bed seen through blended water (and faintly on opaque water); a depth tint; screen-space
//!   reflections (a ray along the smooth surface marched against the full-size depth from a dithered start, refined on the
//!   crossing, read softened by its length, bent by the ripples, fading at screen edges and far hits to the fog colour as the
//!   far air), weighted by Fresnel with a minimum sheen so the surface shows from straight above, keeping the water's own
//!   hue; a sun glint with sparkles along the key light (`sunshadow::key_light`); foam where the water meets the ground
//!   (shallow water from the depth under it, or the edge where geometry cuts the surface); contact rings (`Rings`: ripples
//!   and a froth crest spreading from where the hero or a falling particle meets still water); a soft mask edge.
//!   A camera below the surface leaves the pixel retail, so underwater views stay as they were.
//! Gaspar's RAM ocean is lava (`LAVA_LEVEL`): it takes `LAVA_SLOT`, and `fs_water` gives it the lava look (`lava`) instead.
//! Aridia's mud (its ocean) and Orxon's toxic sludge (level terrain, `terrain_liquid`) take `MUD_SLOT` / `SLUDGE_SLOT`: the
//! mud and sludge looks.

use super::*;

const SHADER: &str = r#"
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

fn hash(p: vec2<f32>) -> f32 {
	let q = fract(p * vec2<f32>(123.34, 456.21));
	let r = q + dot(q, q + 45.32);
	return fract(r.x * r.y);
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
// world size of one pixel there: a train finer than about three pixels fades out instead of aliasing into blocks.
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
		let keep = 1.0 - smoothstep(0.12, 0.35, fp * f * 0.159);
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

// Animated caustic light on world XY `p` (mean about 0.35); `fp` = world size of a pixel (fine lines fade to their mean far off).
fn caustic(p: vec2<f32>, t: f32, fp: f32) -> f32 {
	let q = p * 0.75 + (vec2<f32>(vnoise(p * 0.45 + t * 0.1), vnoise(p * 0.45 + vec2<f32>(3.7, 9.1) - t * 0.1)) - 0.5) * 0.8;
	let a = 1.0 - smoothstep(0.0, 0.2, cells(q, t));
	let b = 1.0 - smoothstep(0.0, 0.24, cells(q * 1.37 + vec2<f32>(4.1, 7.3), t * 1.2));
	let c = a * a * 0.6 + b * b * 0.45 + a * b * 0.9;
	return mix(c, 0.35, smoothstep(0.06, 0.25, fp));
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

// [PC] Long swells on open sea (`WaterDraw::sea`) away from the hero: three long trains (44, 27 and 17 units), slow as deep
// water waves of that length, rising and falling in broad groups. A slope only: the surface (and what stands on it) stays
// retail.
const SWELL: f32 = 0.16;
const SWELL_FROM: f32 = 30.0;
const SWELL_TO: f32 = 80.0;
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
// [PC] At world XY: the distance (world units, bilinear) to the nearest cell with ground at the surface (`SHORE_SHADER`
// `cs_shore_v`; SHORE_FAR = none near). Where no ground is seen under the water (shallows over a level with no floor there, a
// bank seen from above: Pokitaru's beaches) this is all that tells the shore is near.
const SHORE_FAR: f32 = 11.0;
fn shore_dist(xy: vec2<f32>) -> f32 {
	let n = i32(w.gmap.w);
	let f = (xy - w.gmap.xy) / w.gmap.z - 0.5;
	let i = vec2<i32>(floor(f));
	let fr = f - floor(f);
	var d = vec4<f32>(SHORE_FAR);
	for (var k = 0; k < 4; k++) {
		let c = i + vec2<i32>(k & 1, k >> 1);
		if (all(c >= vec2<i32>(0)) && all(c < vec2<i32>(n))) {
			let key = shore[3 * n * n + c.y * n + c.x];
			if (key != 0u) { d[k] = bitcast<f32>(key) - 1.0; }
		}
	}
	return mix(mix(d.x, d.y, fr.x), mix(d.z, d.w, fr.x), fr.y) * w.gmap.z;
}

// How much the waves rise at the flat point f (f.z = the sea plane): (geometry, shading), 0..1. Calm where the ground comes
// within SHORE_TO under the surface or above it (shore map: no wave through the sand, no gap at a wall), round the hero in or
// at the water (he swims at retail's flat plane), and for the geometry only near a low camera and far off (the shading
// carries the far waves).
const SHORE_FROM: f32 = 0.1;
const SHORE_TO: f32 = 1.6;
// [PC] Where no ground is seen under the water: calm by the distance to the shore (`shore_dist`), from SHORE_D0 to SHORE_D1.
const SHORE_D0: f32 = 3.5;
const SHORE_D1: f32 = 9.5;
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
const HERO_CALM0: f32 = 1.2;
const HERO_CALM1: f32 = 5.0;
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
	let sh = mix(smoothstep(SHORE_D0, SHORE_D1, shore_dist(f.xy)), smoothstep(SHORE_FROM, SHORE_TO, f.z - s.x), smoothstep(0.0, 0.25, s.y));
	var hero = 1.0;
	if (w.hero.w > 0.5 && w.hero.z > f.z - HERO_DEEP && w.hero.z < f.z + HERO_HIGH) {
		hero = smoothstep(HERO_CALM0, HERO_CALM1, length(f.xy - w.hero.xy));
	}
	let low = 1.0 - smoothstep(CAM_LOW0, CAM_LOW1, w.eye.z - f.z);
	let cam = mix(1.0, smoothstep(CAM_CALM0, CAM_CALM1, length(f.xy - w.eye.xy)), low);
	let far = 1.0 - smoothstep(GEO_FAR0, GEO_FAR1, length(f - w.eye.xyz));
	return vec2<f32>(sh * hero * cam * far, sh * mix(0.5, 1.0, hero));
}

// The projected grid: w.ggrid = (columns, rows, cell px), w.gridb = (first corner, step) in NDC; vertex i + j (columns + 1).
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
	let fpg = dist * 2.0 * w.proj.y * w.size.w * w.ggrid.z;
	let am = calm(f).x;
	var h = 0.0;
	if (am > 0.001) {
		h = clamp(sea_w(f.xy, w.eye.w, fpg, fpg, geo_an(f - eye), i32(w.wave.x)).hg * am, -GEO_HMAX, GEO_HMAX);
	}
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

// The raised surface into the position target where its flat point is visible flat sea of this plane and nothing of the level
// stands in front of it (the retail sea's own depth, where it writes one, does not count). a = 1 + class bits (64 / 128 sea,
// 512 geometric) + 0.45 x the height (-GEO_HMAX..GEO_HMAX as 0..1), read back in `fs_water`.
@fragment fn fs_grid(v: GO) -> @location(0) vec4<f32> {
	let gz = w.geo[v.g];
	let dim = vec2<i32>(textureDimensions(seamask));
	let m = textureLoad(seamask, clamp(vec2<i32>(v.flat_px), vec2<i32>(0), dim - vec2<i32>(1)), 0).r;
	// (Retail's sea is long triangles of slightly different heights, Pokitaru: up to about a unit apart.)
	if (abs(m - gz.x) > GEO_MATCH) { discard; }
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

// Is the picture at uv water (geometric sea: the flat sea mask, as the picture holds the flat sea)?
fn wet(uv: vec2<f32>, geo: bool) -> bool {
	let q = vec2<i32>(uv * w.size.xy);
	if (geo) {
		let dim = vec2<i32>(textureDimensions(seamask));
		return textureLoad(seamask, clamp(q, vec2<i32>(0), dim - vec2<i32>(1)), 0).r > -1e8;
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

// Back to the frame: a pixel with little water around blends towards the retail picture, and particles over the water (a
// blast, smoke, splashes) stay as drawn.
fn finish(c: vec2<i32>, orig: vec3<f32>, shaded: vec3<f32>, geo: bool) -> vec4<f32> {
	var around = 0.0;
	let ring = array<vec2<i32>, 8>(vec2<i32>(-1, -1), vec2<i32>(0, -1), vec2<i32>(1, -1), vec2<i32>(-1, 0), vec2<i32>(1, 0), vec2<i32>(-1, 1), vec2<i32>(0, 1), vec2<i32>(1, 1));
	for (var k = 0; k < 8; k++) {
		// [PC] Geometric sea: a hole of the grid (`geo_gap`) is water too.
		if (wpos_at(c + ring[k]).a > 0.5 || (geo && geo_gap(c + ring[k]).a > 0.5)) { around += 1.0; }
	}
	var col = mix(orig, shaded, smoothstep(2.0, 7.0, around));
	col = mix(col, orig, clamp(textureLoad(glow, c, 0).a * w.mirror.z * 1.5, 0.0, 1.0));
	return vec4<f32>(pow(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2)), 1.0);
}

// w.eye = (eye, seconds); w.sun = (to the key light, glint strength); w.look = (ripples, refraction in world units, minimum
// reflectance, foam); w.fog = (fog colour in linear light, reflection strength).
@fragment fn fs_water(v: VO) -> @location(0) vec4<f32> {
	let c = vec2<i32>(v.pos.xy);
	var wp = wpos_at(c);
	if (wp.a < 0.5) { wp = geo_gap(c); }
	if (wp.a < 0.5) { discard; }
	let p = wp.xyz;
	let eye = w.eye.xyz;
	// [PC] Geometric sea waves (`fs_grid`, bit 512): p is the raised surface; its flat point pf (retail's plane under or over it,
	// the height from the fraction of a) is where the picture and the bed under the water are read.
	let geo = (u32(wp.a - 0.5) & 512u) != 0u;
	let pf = vec3<f32>(p.xy, p.z - select(0.0, (fract(wp.a) / 0.45 - 0.5) * 2.0 * GEO_HMAX, geo));
	// Underwater (the camera below this surface): the retail picture.
	if (eye.z < pf.z - 0.02) { discard; }
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
	let jit = dither(v.pos.xy);
	let uv0 = (vec2<f32>(c) + 0.5) * w.size.zw;
	let orig = to_linear(textureSampleLevel(scene, lsamp, uv0, 0.0).rgb);
	// Where the picture under the surface is read (geometric sea: at the flat point). A trough by the shore can have its flat point
	// behind the rock standing at the waterline: the picture there is the rock, and the water showed dark specks along every shore
	// (user screenshot 2026-10-19); the pixel's own picture then.
	let uvf = project(pf).xy;
	let uvb = select(uv0, uvf, geo && wet(uvf, true));
	// World size of one pixel here (wider on a grazing view of the surface).
	let ds = smoothstep(FAR_FROM, FAR_TO, dist);
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
	// swells on open sea away from the hero.
	// A phased river's flow jumps between its quads and phases: its wakes stay put.
	let wk = select(wake_at(p, fp, select(flow, vec2<f32>(0.0), phased), time), vec3<f32>(0.0), frozen);
	let ch = select(churn_at(p), 0.0, frozen);
	let rapid = smoothstep(RAPID_FROM, RAPID_TO, length(flow));
	rs += wk.xy;
	if (ch + rapid > 0.01) {
		rs += ripple(q * 1.9 + vec2<f32>(11.3, 4.7), time * 1.8, rfp) * (TURB * ch + RAPID_CHOP * rapid);
	}
	// Sea state (Water Medium / High): wind waves on open sea and lakes, a little calmer round the hero (his rings stay readable).
	var wh = 0.0;
	var wsd = vec2<f32>(0.0);
	// The geometric sea: the slope of the trains the grid carries (the surface the reflection ray follows).
	var gpart = vec2<f32>(0.0);
	let waved = w.wave.x > 0.5 && (sea || lake) && !iced && !frozen;
	if (geo) {
		let am = calm(pf);
		let df = max(length(eye - pf), 1e-3);
		let fpi = df * 2.0 * w.proj.y * w.size.w;
		let sw = sea_w(p.xy, time, fpi, fpi * w.ggrid.z, geo_an(pf - eye), i32(w.wave.x));
		wsd = sw.g * am.y;
		gpart = sw.gg * am.x;
		rs += wsd;
		wh = sw.h / max(sw.hw, 1e-4) * am.y;
	} else if (waved) {
		let hd = length(p.xy - w.hero.xy);
		let near = select(1.0, mix(0.5, 1.0, smoothstep(select(1.0, 3.0, sea), select(8.0, 25.0, sea), hd)), w.hero.w > 0.5);
		let wsl = waves(p.xy, time, rfp, select(WAVE_LAKE_SCALE, 1.0, sea), i32(select(min(w.wave.x, 4.0), w.wave.x, sea)));
		wsd = wsl.yz * (select(WAVE_LAKE_AMP, 1.0, sea) * near * w.wave.z);
		rs += wsd;
		wh = wsl.x * near;
	} else if (sea && !iced) {
		let away = select(1.0, smoothstep(SWELL_FROM, SWELL_TO, length(p.xy - w.hero.xy)), w.hero.w > 0.5);
		if (away > 0.0) { rs += swell(p.xy, time) * away; }
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
	// depth behind distant water blinks between bed and none), so the bed path fades out from FAR_FROM to FAR_TO (`bk`: 1 =
	// the bed path, 0 = the opaque one), and far water looks as water retail draws opaque [C: flick.py on camera pans of
	// fullgame1 0331 (Batalia sea), fg1/5_0216 and 5_0246 (Rilgar), 2026-10-16: with the bed path off past 40 units the far
	// flicker fell from 8.0 / 7.3 / 8.1 to 3.2 / 1.0 / 1.5 (Classic 1.9 / 0.5 / 0.2); with the clear shallows off it stayed].
	let zw = project(pf).z;
	let zs = scene_z(uvb);
	// [PC] Geometric sea: the picture at the flat point holds retail's flat sea, which wrote its depth. Read a fraction of a pixel
	// off pf, that depth lay a little behind or before zw (the grazing slope across a pixel), `zs > zw + 0.05` came and went with
	// the waves and the near sea switched between the bed path (a bed on the sea itself: depth 0, full shore foam, caustics) and
	// the opaque one in contour bands that changed every frame (ragged bands, thin streaks; 2026-10-20). The sea itself is no bed.
	let blended = fl.z > 0.5 && ds < 1.0 && zs > zw + 0.05 && zs < w.proj.w * 0.98 && !(geo && on_flat_sea(uvb, zs));
	let bk = select(0.0, 1.0 - ds, blended);
	let bed = eye + (pf - eye) * (zs / zw);
	let depth = select(0.0, clamp(pf.z - bed.z, 0.0, 40.0), blended);
	let thick = depth * bk;

	// Refraction: the picture under the surface, shifted by the ripple slope (more through deeper water); only from water.
	let push = w.look.y * (0.5 + min(thick, 4.0) * 0.25) * select(1.0, COLD_PUSH, cold > 0.0);
	// (Geometric sea: the shift capped at GEO_PUSH_MAX, so steep waves do not swirl the bed or pull in what stands by the shore.)
	var shift = rs * push;
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
	let cstr = (0.25 + 0.75 * sun_on) * select(mix(0.18, 0.8 * exp(-thick * 0.22), bk), 0.8 * exp(-thick * 0.22), bk >= 1.0);
	base += base * (cl - 0.3) * cstr;
	// Depth tint: deeper water turns towards a darker, richer version of its own colour.
	let lb = luma(base);
	let deep = max(vec3<f32>(lb) + (base - vec3<f32>(lb)) * 1.4, vec3<f32>(0.0)) * 0.7;
	base = mix(base, deep, (1.0 - exp(-thick * 0.12)) * 0.5);

	// Clear shallows: the bed (the level without its water, mirror.rs `draw_bed`) through the surface at the refracted uv, tinted
	// by the water's own hue and lit by the caustics; the path through the water absorbs it into the retail colour above, so
	// deep water (and water with nothing under it) keeps its own colour. w.mirror.w = how clear (CLEAR).
	let bd = vec2<i32>(textureDimensions(bed_depth));
	var buv = uv;
	var zb = linear_depth(textureLoad(bed_depth, clamp(vec2<i32>(buv * vec2<f32>(bd)), vec2<i32>(0), bd - vec2<i32>(1)), 0));
	if (zb < zw + 0.02) {
		// The refracted point lies in front of the water (something standing in it): read straight through instead.
		buv = uvb;
		zb = linear_depth(textureLoad(bed_depth, clamp(vec2<i32>(buv * vec2<f32>(bd)), vec2<i32>(0), bd - vec2<i32>(1)), 0));
	}
	if (zb > zw + 0.02 && zb < w.proj.w * 0.98 && w.mirror.w > 0.0) {
		let bp = eye + (pf - eye) * (zb / zw);
		let path = (zb / zw - 1.0) * length(pf - eye);
		let hue = base / max(max(base.r, max(base.g, base.b)), 1e-3);
		var shallow = to_linear(textureSampleLevel(bed_tex, lsamp, buv, 0.0).rgb) * mix(vec3<f32>(1.0), hue, 0.6);
		let bt = clamp(pf.z - bp.z, 0.0, 40.0);
		shallow += shallow * (caustic(bp.xy - ft + rs * (0.6 + bt * 0.3), time, fp * zb / zw) - 0.3) * (0.25 + 0.75 * sun_on) * 0.9 * exp(-bt * 0.2);
		base = mix(base, shallow, exp(-path * ABSORB) * w.mirror.w * fl.z);
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
		let muv = uvb + project(p + vec3<f32>(-(rs + wsd * WAVE_BEND) * 0.25 * (1.0 - FAR_BEND * ds), 0.0)).xy - project(p).xy;
		let mr = (0.5 + min(fp * 4.0, 2.0)) * w.size.zw * 2.0 * (1.0 + FAR_BLUR * ds);
		let a = jit * 6.2832;
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
	var col = mix(base, refl, clamp(fresnel * w.fog.w, 0.0, mix(0.65, 0.75, sure)));
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
	col += vec3<f32>(1.0, 0.97, 0.9) * pow(max(dot(sn, hv), 0.0), e) * (e + 8.0) * 0.0004 * twinkle * w.sun.w;
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
		col *= max(1.0 + select(WAVE_FACE, GEO_FACE, geo) * dot(wsd, lh) + select(0.0, WAVE_SHADE * wh, w.wave.y > 0.5), 0.0);
	}

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
			surf = (1.0 - smoothstep(0.2 * reach, reach, shore_dist(pf.xy))) * (1.0 - seen);
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
	foam = smoothstep(0.45, 0.95, foam * (0.3 + 1.1 * breakup) * (1.0 + 0.8 * rapid + ch)) * w.look.w * 0.55;
	if (cold >= 3.0) {
		// Cold water: ice forming where it meets the ground nearby (far off the shore test spreads over the cliffs), pale.
		col = mix(col, mix(base, COLD_ICE, 0.6), clamp(foam * COLD_RIM * (1.0 - smoothstep(0.04, 0.1, fp)), 0.0, 1.0));
	} else {
		col = mix(col, min(base * 1.5 + vec3<f32>(0.05), vec3<f32>(0.7)), foam * FOAM_SHORE);
	}
	if (surf > 0.0) {
		col = mix(col, min(base * 1.7 + vec3<f32>(0.12), vec3<f32>(0.88)), smoothstep(0.4, 0.9, surf * (0.25 + 1.1 * breakup)) * SURF_ALPHA * FOAM_SHORE);
	}
	// The rings' froth crest: whiter than the shore foam while young, broken up the same way.
	col = mix(col, min(base * 1.8 + vec3<f32>(0.12), vec3<f32>(0.85)), clamp(rg.z * (0.3 + 0.9 * breakup), 0.0, 0.8));
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

/// [PC] Geometric sea waves: the shore map. Every SHORE_STRIDE-th pixel of the water bed's depth (mirror.rs `draw_bed`: the
/// level without its water) as a world point; a point at most SHORE_CAP above the highest sea plane raises its cell of a
/// camera-centred SHORE_N x SHORE_N grid of SHORE_CELL units to its height (ordered key, atomic max; 0 = not seen). So a cell
/// holds the sea bed, or the sand, wall or rock that comes up to or through the surface there (`calm`). [PC] The buffer's
/// second quarter marks the cells with ground from SHORE_NEAR under the highest sea plane to SHORE_HIGH above it, the third
/// and fourth make the distance to them (`cs_shore_h`, `cs_shore_v`; `shore_dist`).
const SHORE_SHADER: &str = r#"
struct SU { ivp: mat4x4<f32>, map: vec4<f32>, size: vec4<f32> };
@group(0) @binding(0) var<uniform> s: SU;
@group(0) @binding(1) var bed: texture_depth_multisampled_2d;
@group(0) @binding(2) var<storage, read_write> shore: array<atomic<u32>>;
@compute @workgroup_size(8, 8) fn cs_shore(@builtin(global_invocation_id) id: vec3<u32>) {
	let dim = textureDimensions(bed);
	let px = id.xy * u32(s.size.x);
	if (any(px >= dim)) { return; }
	let d = textureLoad(bed, vec2<i32>(px), 0);
	if (d >= 0.99999) { return; }
	let uv = (vec2<f32>(px) + 0.5) / vec2<f32>(dim);
	let q = s.ivp * vec4<f32>(uv.x * 2.0 - 1.0, 1.0 - uv.y * 2.0, d, 1.0);
	let p = q.xyz / q.w;
	let n = i32(s.size.y);
	let c = vec2<i32>(floor((p.xy - s.map.xy) / s.map.z));
	if (any(c < vec2<i32>(0)) || any(c >= vec2<i32>(n))) { return; }
	let i = c.y * n + c.x;
	// [PC] Ground at the surface (s.size.z..w: a little under the highest sea plane to well above it, a bank seen from above
	// with its waterline hidden under its edge) marks its cell for the distance map (`cs_shore_h`).
	if (p.z >= s.size.z && p.z <= s.size.w && atomicLoad(&shore[n * n + i]) == 0u) { atomicStore(&shore[n * n + i], 1u); }
	if (p.z > s.map.w) { return; }
	let b = bitcast<u32>(p.z);
	let k = select(b | 0x80000000u, ~b, (b & 0x80000000u) != 0u);
	// (Most pixels of a near cell hold no new maximum: a plain read first keeps them off the atomic.)
	if (atomicLoad(&shore[i]) >= k) { return; }
	atomicMax(&shore[i], k);
}
// [PC] The distance map, an exact distance transform over SHORE_R cells in two passes: per cell the distance (cells) along its
// row to the nearest marked cell (SHORE_R + 1 = none), then the nearest over its column (centre to centre), stored + 1 as f32
// bits (0 = not made).
const SHORE_R: i32 = 10;
@compute @workgroup_size(8, 8) fn cs_shore_h(@builtin(global_invocation_id) id: vec3<u32>) {
	let n = i32(s.size.y);
	let c = vec2<i32>(id.xy);
	if (any(c >= vec2<i32>(n))) { return; }
	var best = SHORE_R + 1;
	for (var k = -SHORE_R; k <= SHORE_R; k++) {
		let x = c.x + k;
		if (x >= 0 && x < n && abs(k) < best && atomicLoad(&shore[n * n + c.y * n + x]) != 0u) { best = abs(k); }
	}
	atomicStore(&shore[2 * n * n + c.y * n + c.x], u32(best));
}
@compute @workgroup_size(8, 8) fn cs_shore_v(@builtin(global_invocation_id) id: vec3<u32>) {
	let n = i32(s.size.y);
	let c = vec2<i32>(id.xy);
	if (any(c >= vec2<i32>(n))) { return; }
	var d2 = f32((SHORE_R + 1) * (SHORE_R + 1));
	for (var k = -SHORE_R; k <= SHORE_R; k++) {
		let y = c.y + k;
		if (y >= 0 && y < n) {
			let h = f32(atomicLoad(&shore[2 * n * n + y * n + c.x]));
			d2 = min(d2, h * h + f32(k * k));
		}
	}
	atomicStore(&shore[3 * n * n + c.y * n + c.x], bitcast<u32>(sqrt(d2) + 1.0));
}
"#;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ShoreU {
	ivp: [[f32; 4]; 4],
	/// Origin x, y, cell size, highest height kept.
	map: [f32; 4],
	/// Stride (bed pixels), cells across.
	size: [f32; 4],
}

/// [PC] Geometric sea waves: sea planes drawn at most; the shore map's cells across, cell size (units), bed pixel stride and how
/// far above the sea a point still counts as ground at the surface.
const GEO_PLANES: usize = 4;
const SHORE_N: u32 = 256;
const SHORE_CELL: f32 = 1.0;
const SHORE_STRIDE: u32 = 2;
const SHORE_CAP: f32 = 1.0;
const SHORE_NEAR: f32 = 0.3;
const SHORE_HIGH: f32 = 3.5;
/// A moby that moved (units, or turned: radians) within SHORE_STILL frames (about a second) stays out of the shore map.
const SHORE_MOVED: f32 = 1e-3;
const SHORE_STILL: u32 = 50;
/// The projected grid's margins beyond the screen (NDC: sides, bottom, top): crests rising in from below the screen's edge.
const GRID_MARGIN: [f32; 3] = [0.06, 0.35, 0.12];

/// [PC] Geometric sea waves by Graphics level of Water (`vs_grid`): grid cell (px), first train's length (units) and steepness,
/// steepness growth per train, height gain (x the RAC_WAVES gain); none at Low or with RAC_WAVES=off.
#[derive(Clone, Copy)]
struct GeoLook {
	cell: f32,
	len: f32,
	sig: f32,
	sigq: f32,
	amp: f32,
}

fn geo_look(level: u8) -> Option<GeoLook> {
	if wave_look(level)[0] == 0.0 {
		return None;
	}
	let mut g = match level {
		2 => GeoLook { cell: 12.0, len: 12.0, sig: 0.2, sigq: 1.15, amp: 0.3 },
		_ => GeoLook { cell: 8.0, len: 14.0, sig: 0.24, sigq: 1.18, amp: 0.5 },
	};
	g.amp *= wave_look(level)[2];
	Some(g)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct WaterU {
	vp: [[f32; 4]; 4],
	eye: [f32; 4],
	sun: [f32; 4],
	proj: [f32; 4],
	size: [f32; 4],
	look: [f32; 4],
	fog: [f32; 4],
	mirror: [f32; 4],
	flows: [[f32; 4]; FLOWS],
	rn: [f32; 4],
	rings: [[f32; 4]; 2 * RINGS],
	trail: [[f32; 4]; 2 * TRAIL],
	wbox: [f32; 4],
	churn: [[f32; 4]; 2 * CHURN],
	hero: [f32; 4],
	/// Sea state: trains (0 = off), High (shade and whitecaps), slope gain.
	wave: [f32; 4],
	/// [PC] Geometric sea waves (`vs_grid`): vp's inverse; the planes (height, class bits); the shore map (origin x, y, cell,
	/// cells across); the grid (columns, rows, cell px) and its first corner and step in NDC; the trains (first steepness,
	/// growth per train, height gain, first length).
	ivp: [[f32; 4]; 4],
	geo: [[f32; 4]; GEO_PLANES],
	gmap: [f32; 4],
	ggrid: [f32; 4],
	gridb: [f32; 4],
	wgeo: [f32; 4],
	/// [PC] The 2nd and 3rd mirrored planes (Water High; `mirror` holds the 1st): their heights, the number of planes mirrored.
	mirror2: [f32; 4],
}

/// The look's numbers (one tuning for every level).
const RIPPLES: f32 = 1.0;
/// How far (world units, per unit of ripple slope) the picture under the surface shifts.
const REFRACTION: f32 = 0.5;
/// The least reflectance, above the physical Fresnel (so the surface reads from straight above too).
const SHEEN: f32 = 0.1;
const FOAM: f32 = 0.4;
const REFLECTION: f32 = 1.0;
const GLINT: f32 = 2.2;
/// How clear the water is over its bed (0 = the retail colour throughout).
const CLEAR: f32 = 1.0;

/// Gaspar (its "ocean" is lava): its RAM ocean takes `LAVA_SLOT` (`WaterDraw::liquid`), the lava look of `fs_water`.
pub(super) const LAVA_LEVEL: usize = 9;

/// The liquids that are no water, by level: the slot (`fs_water`'s look) their ocean draws take (`WaterDraw::liquid`). Gaspar's
/// RAM ocean is lava; Aridia's ocean (its "level ocean", the brown plane at z 24.75 [C: RAC_WATER_LOG at fg1/2_0091 / 2_0138,
/// 2026-10-16: its only water draw]) is the mud the hero sinks in (decomp/notes/water.md ARI-50: Aridia has no water faces).
fn liquid_slot(level: usize) -> Option<u32> {
	match level {
		LAVA_LEVEL => Some(LAVA_SLOT),
		2 => Some(MUD_SLOT),
		_ => None,
	}
}

/// A liquid drawn as level terrain, by level: its terrain texture index (tfrag face key) and slot. Orxon's green toxic sea is
/// terrain texture 12 (flat faces, z 27-62; it goes when that texture is left out, and retail draws it still: Classic fields
/// 60 / 80 / 100 at fullgame1 0520 differ in no sludge pixel) [C: the terrain textures logged and left out one at a time, 2026-10-16].
pub(super) fn terrain_liquid(level: usize) -> Option<(u32, u32)> {
	match level {
		10 => Some((12, SLUDGE_SLOT)),
		_ => None,
	}
}

/// The fx textures (level fx table index, `WorldQuad.tex`) of a sea drawn as effect quads: Pokitaru's sea is queue-A
/// callback 0x30a238 output (decomp/notes/ocean.md); at fg1/11_0449 it is fx texture 22 (15 fans out to the horizon) under
/// the additive layers 40 and 41 (flat at z 223.16 around the beach) [C: RAC_WATER_LOG bounds and RAC_DRAWSKIP=quads, 2026-10-15].
/// The queue-C pool strips (`Gpu.pool_draws`) that are water, by fx texture: Hoven's door pools (FUN_002e6ae0: tex 0x2c at FIX
/// 0x80, its second layer 0x2d at FIX 0x40) and its abyss water (tex 0x2c, 0x2f and 0x30; `cold_pool` gives 0x2f / 0x30 the cold look). Orxon's tex 41 (FIX
/// 32) and Batalia's tex 59 (additive, FIX 0) are no water surfaces.
/// (corrected 2026-10-16: was "tex 47 / 48 strips are the snow"; at fg1/12_0554 they are drawn but hidden under the ground,
/// in the abyss they are its teal wave plane [C: RAC_DRAWSKIP=pools on play/compare/Hoven.racpad field 1950].)
pub(super) fn pool_water_tex(level: usize) -> &'static [usize] {
	match level {
		12 => &[44, 45, 47, 48],
		_ => &[],
	}
}

/// Hoven's abyss water, the queue-C callback FUN_0030b798 (overlay.elf.c 165949-166004): tex 0x2c strips while the camera is
/// inside its cuboid and the wave plane tex 0x2f, both at FIX 0x7f (ALPHA 0x7f00000064), then tex 0x30 additive (Cs * FIX +
/// Cd). The two wave-plane layers get the cold look (`COLD_SLOT`). The tex 0x2c sheet is the flooded floor around the doors
/// (fg1/12_0582, plane z 45) and keeps the door pools' look, as the door pools themselves (tex 0x2c FIX 0x80, 0x2d FIX 0x40).
pub(super) fn cold_pool(level: usize, tex: usize, add: bool, fix: u8) -> bool {
	level == 12 && (!add && fix == 127 && tex == 47 || add && tex == 48)
}

/// [PC] Hoven's frozen floor (user 2026-10-16 at fg1/12_0582: "the ice here should be more static, currently it moves like
/// water"): the door puddles (FUN_002e6ae0 / FUN_002bbb40: tex 0x2c FIX 0x80, then tex 0x2d FIX 0x40) and the tex 0x2c FIX 0x7f
/// sheet around the doors (plane z 45) keep their Enhanced look held still: `FROZEN_SLOT` for the opaque layers,
/// `FROZEN_CLEAR_SLOT` for the FIX 0x40 one. Retail blends the puddle texture over time itself (Classic fields 60 / 100
/// differ there), which stays. The abyss (`cold_pool`, tex 0x2f / 0x30) stays liquid.
pub(super) fn frozen_pool(level: usize, tex: usize, add: bool, fix: u8) -> Option<u32> {
	match (level, add, tex, fix) {
		(12, false, 44, 127..=128) => Some(FROZEN_SLOT),
		(12, false, 45, 64) => Some(FROZEN_CLEAR_SLOT),
		_ => None,
	}
}

/// Quartu's tank water (the flooded round tank room, decomp/notes/draw_coverage.md 6h: queue B callbacks 0x2ea5c0 / 0x2ed510 /
/// 0x2ed8f0, a wobbling grid drawn twice at ALPHA 0x44) is fx texture 45 (646 quads, z 39.40-39.60 at fullgame2 0058)
/// [C: RAC_WATER_LOG fx quad bounds, 2026-10-16].
pub(super) fn sea_fx_tex(level: usize) -> &'static [usize] {
	match level {
		11 => &[22, 40, 41],
		15 => &[45],
		_ => &[],
	}
}

/// [PC] A pond drawn as effect quads, by level: its fx textures. Pokitaru's waterfall ponds (fullgame1 0447, z 231-233 at
/// x 445-476 y 549-572) are fx texture 44 (non-additive) under the additive layer 43 [C: RAC_DRAWSKIP=quads leaves them out,
/// fx quad bounds logged, 2026-10-17]. They get their own draw: their plane is not the sea's.
pub(super) fn pond_fx_tex(level: usize) -> &'static [usize] {
	match level {
		11 => &[43, 44],
		_ => &[],
	}
}

/// [PC] Falling water drawn as effect quads, by level: its fx textures. Pokitaru's waterfalls are fx texture 42 (additive,
/// z 222-252 at fullgame1 0447) [C: as `pond_fx_tex`]. Their feet churn the water (`WaterFx::sheet_falls`).
pub(super) fn fall_fx_tex(level: usize) -> &'static [usize] {
	match level {
		11 => &[42],
		_ => &[],
	}
}

/// [PC] The foot of a steep quad (its bottom edge's middle) as a waterfall foot, one per CHURN_CELL.
fn push_foot(falls: &mut Vec<[f32; 3]>, pos: &[[f32; 3]; 4]) {
	let mut v = *pos;
	v.sort_by(|a, b| a[2].total_cmp(&b[2]));
	let f = [(v[0][0] + v[1][0]) * 0.5, (v[0][1] + v[1][1]) * 0.5, (v[0][2] + v[1][2]) * 0.5];
	if !falls.iter().any(|g| (g[0] - f[0]).abs() < CHURN_CELL && (g[1] - f[1]).abs() < CHURN_CELL && (g[2] - f[2]).abs() < 1.0) {
		falls.push(f);
	}
}

/// [PC] How far from a waterfall's brink `shape_faces` looks (world units); farther points count as this far.
const BRINK_REACH: f32 = 4.0;

/// [PC] A sheet face's normal and its length, from its corners p0, p1 and the last; None when degenerate.
fn face_normal(c: &[[f32; 3]]) -> Option<([f32; 3], f32)> {
	let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
	let (a, b) = (sub(c[1], c[0]), sub(c[c.len() - 1], c[0]));
	let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
	let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
	if nl < 1e-9 { None } else { Some((n, nl)) }
}

/// [PC] Shapes Pokitaru's waterfall sheets (`sheet_falls`; retail's quads, in fours, are triangles of ribbons with p3 == p2)
/// for `fs_fall`: each vertex's `light` = [steepness (0 flat .. 1 straight down) averaged over the faces sharing the vertex,
/// signed distance d from the nearest brink (where the sheet tips from flat to steep going down; < 0 on the water running to
/// it, > 0 down the fall, +-BRINK_REACH beyond), 1, 1 on the flat skirt at a foot (water falls onto it) else 0]. Interpolated over each face, so the bend over the edge is shaded
/// smoothly instead of facet by facet.
/// `pos`: the sheet's vertices, each once; `faces`: their corner ids in fours (p3 == p2: a triangle); `steep`: each vertex's
/// steepness. Sets the `light` of every vertex of `faces`.
fn shape_faces(pos: &[[f32; 3]], faces: &[[u32; 4]], steep: &[f32], light: &mut [[f32; 4]]) {
	use std::collections::HashMap;
	let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
	let corners = |f: &[u32; 4]| -> std::ops::Range<usize> { if f[3] == f[2] { 0..3 } else { 0..4 } };
	let mut used = vec![false; pos.len()];
	for f in faces {
		for k in corners(f) {
			used[f[k] as usize] = true;
		}
	}
	// Brink points: on each edge from a flat corner down to a steep one, where the steepness crosses 0.5; not where falling
	// water comes down from above, straight or slanting (a step in a fall, the skirt at a foot: retail's sheets reach on below
	// the pond surface).
	let cell = |p: [f32; 3]| ((p[0] / BRINK_REACH).floor() as i32, (p[1] / BRINK_REACH).floor() as i32);
	let mut steeps: HashMap<(i32, i32), Vec<[f32; 3]>> = HashMap::new();
	for (i, &p) in pos.iter().enumerate() {
		if used[i] && steep[i] >= 0.5 {
			steeps.entry(cell(p)).or_default().push(p);
		}
	}
	let fed = |m: [f32; 3]| {
		let (cx, cy) = cell(m);
		(-1..=1).any(|dx| (-1..=1).any(|dy| steeps.get(&(cx + dx, cy + dy)).into_iter().flatten().any(|q| q[2] > m[2] + 0.5 && (q[0] - m[0]).hypot(q[1] - m[1]) < 1.5 + (q[2] - m[2]))))
	};
	let mut brinks: HashMap<(i32, i32), Vec<[f32; 3]>> = HashMap::new();
	for face in faces {
		let c = &face[corners(face)];
		for k in 0..c.len() {
			let (ia, ib) = (c[k] as usize, c[(k + 1) % c.len()] as usize);
			let (a, b, sa, sb) = (pos[ia], pos[ib], steep[ia], steep[ib]);
			if (sa < 0.5) == (sb < 0.5) {
				continue;
			}
			let (f, s, sf, ss) = if sa < 0.5 { (a, b, sa, sb) } else { (b, a, sb, sa) };
			if f[2] <= s[2] {
				continue;
			}
			let t = (0.5 - sf) / (ss - sf);
			let m = [f[0] + (s[0] - f[0]) * t, f[1] + (s[1] - f[1]) * t, f[2] + (s[2] - f[2]) * t];
			if !fed(m) {
				brinks.entry(cell(m)).or_default().push(m);
			}
		}
	}
	for (i, &p) in pos.iter().enumerate() {
		if !used[i] {
			continue;
		}
		let st = steep[i];
		let (cx, cy) = cell(p);
		let mut best: Option<(f32, [f32; 3])> = None;
		for dx in -1..=1 {
			for dy in -1..=1 {
				for b in brinks.get(&(cx + dx, cy + dy)).into_iter().flatten() {
					let d = sub(p, *b);
					let d = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
					if d < BRINK_REACH && best.map_or(true, |(e, _)| d < e) {
						best = Some((d, *b));
					}
				}
			}
		}
		// Far from any brink: steep is down the fall; flat is the skirt at a foot when water falls onto it, else upstream.
		let (d, foot) = match best {
			Some((d, b)) if p[2] < b[2] - 0.05 => (d, 0.0),
			Some((d, _)) => (-d, 0.0),
			None if st >= 0.5 => (BRINK_REACH, 0.0),
			None if fed(p) => (BRINK_REACH, 1.0),
			None => (-BRINK_REACH, 0.0),
		};
		light[i] = [st, d, 1.0, foot];
	}
}

/// [PC] `shape_faces` on one frame's sheet quads alone (RAC_FALL_OLD, the shape before `SheetMem`): corners at one position
/// (to 1/256) are one vertex.
fn shape_sheets(verts: &mut [Vertex]) {
	use std::collections::HashMap;
	let key = |p: [f32; 3]| ((p[0] * 256.0).round() as i32, (p[1] * 256.0).round() as i32, (p[2] * 256.0).round() as i32);
	let mut index: HashMap<(i32, i32, i32), u32> = HashMap::new();
	let mut pos: Vec<[f32; 3]> = Vec::new();
	let ids: Vec<u32> = verts
		.iter()
		.map(|v| {
			*index.entry(key(v.pos)).or_insert_with(|| {
				pos.push(v.pos);
				pos.len() as u32 - 1
			})
		})
		.collect();
	let faces: Vec<[u32; 4]> = ids.chunks_exact(4).map(|q| [q[0], q[1], q[2], q[3]]).collect();
	let mut sum = vec![(0.0f32, 0.0f32); pos.len()];
	for f in &faces {
		let c: Vec<u32> = if f[3] == f[2] { f[..3].to_vec() } else { f.to_vec() };
		let cp: Vec<[f32; 3]> = c.iter().map(|&i| pos[i as usize]).collect();
		if let Some((n, nl)) = face_normal(&cp) {
			let st = 1.0 - n[2].abs() / nl;
			for &i in &c {
				sum[i as usize].0 += st;
				sum[i as usize].1 += 1.0;
			}
		}
	}
	let steep: Vec<f32> = sum.iter().map(|&(s, n)| if n > 0.0 { s / n } else { 0.0 }).collect();
	let mut light = vec![[0.0f32; 4]; pos.len()];
	shape_faces(&pos, &faces, &steep, &mut light);
	for (v, &i) in verts.iter_mut().zip(&ids) {
		v.light = light[i as usize];
	}
}

/// [PC] How close (world units) two reconstructed sheet vertices are to count as one (`SheetMem`): retail's positions are rebuilt from
/// GS depth, so a vertex wobbles by up to about 0.1 from frame to frame; distinct vertices lie at least 0.37 apart.
const SHEET_SNAP: f32 = 0.18;

/// [PC] Pokitaru's waterfall sheets over time: unique vertices (the first position seen, within SHEET_SNAP), the triangles seen so far,
/// and the shape (`shape_faces`) and feet of that whole set, recomputed only when a new triangle shows up. Each vertex's steepness
/// and the feet are summed up as triangles come, so a recompute shapes each vertex once (not once per triangle corner: the
/// camera turning brings new triangles nearly every frame, 2026-10-19).
/// Per frame it looks up some 5000 corners (1300 quads at Pokitaru's bay): `memo` remembers each exact position's vertex, the
/// grid search checks only the cells SHEET_SNAP reaches into, and the maps hash with `FastHash` (with std's hasher and all 27
/// cells per lookup this took ~2 ms a frame on a fast PC, 2026-10-19).
#[derive(Default)]
struct SheetMem {
	verts: Vec<[f32; 3]>,
	light: Vec<[f32; 4]>,
	grid: FastMap<(i32, i32, i32), Vec<u32>>,
	known: std::collections::HashSet<[u32; 3], std::hash::BuildHasherDefault<FastHash>>,
	/// Exact position (bits) -> its vertex (`find`); cleared when a vertex is added (the nearest may change).
	memo: FastMap<[u32; 3], u32>,
	/// Corner ids in fours, p3 == p2.
	faces: Vec<[u32; 4]>,
	/// Per vertex: the steepness of its triangles summed, and their count.
	steep: Vec<(f32, f32)>,
	/// The feet of the steep triangles in the order seen (`push_foot`), and the lowest of them (`keep_lowest`).
	feet_seen: Vec<[f32; 3]>,
	feet: Vec<[f32; 3]>,
}

impl SheetMem {
	const CELL: f32 = 0.5;

	fn cell(p: [f32; 3]) -> (i32, i32, i32) {
		((p[0] / Self::CELL).floor() as i32, (p[1] / Self::CELL).floor() as i32, (p[2] / Self::CELL).floor() as i32)
	}

	/// The vertex within SHEET_SNAP of `p` (the nearest), a new one if none.
	fn id(&mut self, p: [f32; 3]) -> u32 {
		if let Some(i) = self.find(p) {
			return i;
		}
		let (cx, cy, cz) = Self::cell(p);
		let i = self.verts.len() as u32;
		self.verts.push(p);
		self.light.push([0.0; 4]);
		self.steep.push((0.0, 0.0));
		self.grid.entry((cx, cy, cz)).or_default().push(i);
		self.memo.clear();
		i
	}

	/// The vertex within SHEET_SNAP of `p` (the nearest), remembered per exact position.
	fn find(&mut self, p: [f32; 3]) -> Option<u32> {
		let key = [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
		if let Some(&i) = self.memo.get(&key) {
			return Some(i);
		}
		let i = self.search(p)?;
		// Positions that never settle (`observe`'s runaway reset) must not grow it without end.
		if self.memo.len() > 100_000 {
			self.memo.clear();
		}
		self.memo.insert(key, i);
		Some(i)
	}

	/// The cells next to p's (-1, 0, 1 per axis) that SHEET_SNAP reaches into.
	fn reach(v: f32) -> std::ops::RangeInclusive<i32> {
		let f = v / Self::CELL - (v / Self::CELL).floor();
		// A little wider than SHEET_SNAP / CELL so rounding never skips a cell that holds the nearest vertex.
		let r = SHEET_SNAP / Self::CELL + 1e-3;
		(if f < r { -1 } else { 0 })..=(if f > 1.0 - r { 1 } else { 0 })
	}

	/// The vertex within SHEET_SNAP of `p` (the nearest), searched in the grid.
	fn search(&self, p: [f32; 3]) -> Option<u32> {
		let (cx, cy, cz) = Self::cell(p);
		let mut best: Option<(f32, u32)> = None;
		for dx in Self::reach(p[0]) {
			for dy in Self::reach(p[1]) {
				for dz in Self::reach(p[2]) {
					for &i in self.grid.get(&(cx + dx, cy + dy, cz + dz)).into_iter().flatten() {
						let q = self.verts[i as usize];
						let d = (q[0] - p[0]).hypot(q[1] - p[1]).hypot(q[2] - p[2]);
						if d < SHEET_SNAP && best.map_or(true, |(e, _)| d < e) {
							best = Some((d, i));
						}
					}
				}
			}
		}
		best.map(|(_, i)| i)
	}

	/// [PC] Widescreen refill (retail/efx_draw.rs `FX_REFILL`): of `cand` (triangles in fours, p3 == p2), the ones that are a sheet
	/// triangle seen kicked before and not among `drawn` (this frame's kicked vertices, after `observe`), with their `light`.
	/// `drawn`: this frame's kicked triangles as `observe` returned their vertices.
	fn refill(&mut self, drawn: &[u32], cand: &[Vertex]) -> Vec<Vertex> {
		let tri = |known: &std::collections::HashSet<[u32; 3], _>, mut f: [u32; 3]| -> Option<[u32; 3]> {
			f.sort_unstable();
			(f[0] != f[1] && f[1] != f[2] && known.contains(&f)).then_some(f)
		};
		let shown: std::collections::HashSet<[u32; 3], std::hash::BuildHasherDefault<FastHash>> = drawn.chunks_exact(4).filter_map(|q| tri(&self.known, [q[0], q[1], q[2]])).collect();
		let mut out = Vec::new();
		for q in cand.chunks_exact(4) {
			let (Some(a), Some(b), Some(c)) = (self.find(q[0].pos), self.find(q[1].pos), self.find(q[2].pos)) else { continue };
			if tri(&self.known, [a, b, c]).is_some_and(|f| !shown.contains(&f)) {
				let ids = [a, b, c, c];
				out.extend(q.iter().zip(ids).map(|(v, i)| Vertex { light: self.light[i as usize], ..*v }));
			}
		}
		out
	}

	/// This frame's sheet vertices (triangles in fours, p3 == p2): adds the new triangles, and sets each vertex's `light`.
	/// Returns each vertex's id.
	fn observe(&mut self, verts: &mut [Vertex]) -> Vec<u32> {
		// Pokitaru's sheets are the same in every visit, so nothing is forgotten (a runaway set is: positions that never settle).
		if self.verts.len() > 20_000 {
			*self = SheetMem::default();
		}
		let ids: Vec<u32> = verts.iter().map(|v| self.id(v.pos)).collect();
		let mut dirty = false;
		for q in ids.chunks_exact(4) {
			let mut f = [q[0], q[1], q[2]];
			f.sort_unstable();
			if self.known.insert(f) {
				let pos = [self.verts[q[0] as usize], self.verts[q[1] as usize], self.verts[q[2] as usize], self.verts[q[2] as usize]];
				if let Some((n, nl)) = face_normal(&pos[..3]) {
					let st = 1.0 - n[2].abs() / nl;
					for &i in &q[..3] {
						let s = &mut self.steep[i as usize];
						s.0 += st;
						s.1 += 1.0;
					}
					if n[2].abs() < 0.5 * nl {
						push_foot(&mut self.feet_seen, &pos);
					}
				}
				self.faces.push([q[0], q[1], q[2], q[2]]);
				dirty = true;
			}
		}
		if dirty {
			let steep: Vec<f32> = self.steep.iter().map(|&(s, n)| if n > 0.0 { s / n } else { 0.0 }).collect();
			shape_faces(&self.verts, &self.faces, &steep, &mut self.light);
			self.feet = self.feet_seen.clone();
			keep_lowest(&mut self.feet);
		}
		for (v, &i) in verts.iter_mut().zip(&ids) {
			v.light = self.light[i as usize];
		}
		ids
	}
}

/// [PC] A fast hasher for small integer keys (rustc's Fx: rotate, xor, multiply); `SheetMem`'s per-frame lookups.
#[derive(Default, Clone, Copy)]
struct FastHash(u64);

impl FastHash {
	fn add(&mut self, x: u64) {
		self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x517c_c1b7_2722_0a95);
	}
}

impl std::hash::Hasher for FastHash {
	fn finish(&self) -> u64 {
		self.0
	}
	fn write(&mut self, b: &[u8]) {
		for c in b.chunks(8) {
			let mut w = [0u8; 8];
			w[..c.len()].copy_from_slice(c);
			self.add(u64::from_le_bytes(w));
		}
	}
	fn write_u32(&mut self, x: u32) {
		self.add(x as u64);
	}
	fn write_i32(&mut self, x: i32) {
		self.add(x as u32 as u64);
	}
	fn write_u64(&mut self, x: u64) {
		self.add(x);
	}
	fn write_usize(&mut self, x: usize) {
		self.add(x as u64);
	}
}

type FastMap<K, V> = std::collections::HashMap<K, V, std::hash::BuildHasherDefault<FastHash>>;

/// [PC] Only the lowest foot of a falling sheet meets the water below it.
fn keep_lowest(falls: &mut Vec<[f32; 3]>) {
	let all = std::mem::take(falls);
	falls.extend(all.iter().copied().filter(|f| !all.iter().any(|g| g[2] < f[2] - 0.5 && (g[0] - f[0]).powi(2) + (g[1] - f[1]).powi(2) < 9.0)));
}

/// A river drawn as effect quads, by level: its fx texture. Batalia's jungle river is fx texture 58 (non-additive quads from
/// the waterfall down to the sea, z 14-31, sloped) under the additive layers 60 and 61 [C: RAC_WATER_LOG bounds at fullgame1
/// 0345 / 0348, 2026-10-16]; retail scrolls its uv the same step every tick on every quad (`WaterFx::quad_flows`).
pub(super) fn river_fx_tex(level: usize) -> Option<usize> {
	match level {
		8 => Some(58),
		_ => None,
	}
}

/// River quad flows (`WaterFx::quad_flows`): directions and speed steps (world units per second) the quads share slots by.
/// How clear a river of effect quads is over its bed (flows z; under 0.5: no bed path): retail paints its light pattern
/// on the surface, which stays.
const RIVER_CLEAR: f32 = 0.25;
const RIVER_DIRS: f32 = 12.0;
const RIVER_SPEED_STEP: f32 = 0.5;

const WPOS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// One water draw of the level pass.
pub(super) struct WaterDraw {
	/// Vertex buffer (`Vertex` layout), index buffer (u32), index count.
	pub vb: wgpu::Buffer,
	pub ib: wgpu::Buffer,
	pub n: u32,
	/// The first index (`range`: a slice of a shared batch).
	pub first: u32,
	/// Its flat plane, if any (`mirror::plane_of`).
	pub plane: super::mirror::Plane,
	/// Its slot in this frame's flow table (`WaterFx::strip_flows`), 0 = still water.
	pub flow: u32,
	/// What drew it (the water log, RAC_WATER_LOG).
	pub kind: &'static str,
	/// [PC] Its box (min x, y, z, max x, y, z) for the wakes and the churn (`Wakes`); none for a slice of a shared batch.
	pub bounds: Option<[f32; 6]>,
}

impl WaterDraw {
	pub(super) fn new(vb: wgpu::Buffer, ib: wgpu::Buffer, n: u32, pos: impl Iterator<Item = [f32; 3]> + Clone) -> WaterDraw {
		let mut b = [f32::MAX, f32::MAX, f32::MAX, f32::MIN, f32::MIN, f32::MIN];
		for p in pos.clone() {
			for k in 0..3 {
				b[k] = b[k].min(p[k]);
				b[k + 3] = b[k + 3].max(p[k]);
			}
		}
		WaterDraw { vb, ib, n, first: 0, plane: super::mirror::plane_of(pos), flow: 0, kind: "", bounds: (b[0] <= b[3]).then_some(b) }
	}

	/// Indices `first..first + n` of a shared batch (`terrain_liquid`): its slot, no mirror plane.
	pub(super) fn range(vb: wgpu::Buffer, ib: wgpu::Buffer, first: u32, n: u32, slot: u32) -> WaterDraw {
		WaterDraw { vb, ib, n, first, plane: None, flow: slot, kind: "", bounds: None }
	}

	/// [PC] Open sea (`swell`): flat water whose box spans at least `SEA_SIZE` (the oceans and Pokitaru's sea, not a pool).
	fn sea(&self) -> bool {
		self.plane.is_some_and(|b| (b[3] - b[1]).max(b[4] - b[2]) >= SEA_SIZE) && self.flow == 0 && self.kind != "strip"
	}

	/// [PC] A lake (`waves`): flat still water, spanning at least `LAKE_SIZE` but no open sea.
	fn lake(&self) -> bool {
		self.plane.is_some_and(|b| {
			let span = (b[3] - b[1]).max(b[4] - b[2]);
			span >= LAKE_SIZE && span < SEA_SIZE
		}) && self.flow == 0 && self.kind != "strip"
	}

	/// [PC] Its box for the wakes (`Wakes`): water only (not lava, mud or sludge), its flat plane's height when it has one.
	fn wake_box(&self) -> Option<[f32; 6]> {
		if self.flow >= LAST_FLOW && self.flow != OPAQUE_SLOT && self.flow != COLD_SLOT {
			return None;
		}
		match (self.plane, self.bounds) {
			(Some(p), _) => Some([p[1], p[2], p[0], p[3], p[4], p[0]]),
			(None, b) => b,
		}
	}

	pub(super) fn kind(mut self, kind: &'static str) -> WaterDraw {
		self.kind = kind;
		self
	}

	/// A flowing river group (`WaterFx::quad_flows`): its flow slot, and no mirror plane (the river slopes).
	pub(super) fn flowing(mut self, slot: u32) -> WaterDraw {
		self.flow = slot;
		self.plane = None;
		self
	}

	/// [PC] Water retail draws milky (Pokitaru's waterfall ponds, `pond_fx_tex`): `OPAQUE_SLOT` (its own colour, no clear
	/// shallows), keeping its mirror plane.
	pub(super) fn opaque(mut self) -> WaterDraw {
		self.flow = OPAQUE_SLOT;
		self
	}

	/// An ocean that is no water (`liquid_slot`: Gaspar's lava, Aridia's mud): its slot, and no mirror plane.
	pub(super) fn liquid(mut self, level: usize) -> WaterDraw {
		if let Some(slot) = liquid_slot(level) {
			self.flow = slot;
			self.plane = None;
		}
		self
	}
}

/// Contact rings at most; the hero's ring spacing moving and standing still (s); a particle ring's spacing (world units, s).
const RINGS: usize = 16;
const HERO_GAP: f32 = 0.3;
const IDLE_GAP: f32 = 1.4;
const PART_GAP: (f32, f32) = (0.5, 0.3);

/// [PC] Contact rings: ripples spreading from where something meets still water (a flat `WaterDraw::plane`). The hero wading
/// or swimming leaves a ring with a froth crest every `HERO_GAP` s while moving (a faint one every `IDLE_GAP` s standing
/// still), and the retail pool particles that touch the surface (splashes, drops, spray) leave smaller ones. Contacts are
/// taken once per game field (`feed`), so a paused game leaves none. Retail draws no such rings; the particles stay as drawn.
#[derive(Default)]
struct Rings {
	live: Vec<Ring>,
	/// The last field's contacts not yet placed (point, hero).
	contacts: Vec<([f32; 3], bool)>,
	frame: u32,
	hero_last: Option<[f32; 3]>,
	hero_t: f32,
}

#[derive(Clone, Copy, Debug)]
struct Ring {
	p: [f32; 3],
	born: f32,
	slope: f32,
	froth: f32,
	life: f32,
	/// [PC] Heavy splashes (`Wakes`): how far and fast it spreads (1 = a contact ring) and the foam left inside it.
	size: f32,
	patch: f32,
}

impl Rings {
	/// This field's contact candidates: the hero's feet and the pool particles; nothing new while the field count stands still.
	fn feed(&mut self, frame: u32, hero: Option<[f32; 3]>, parts: impl Iterator<Item = [f32; 3]>) {
		if frame == self.frame {
			return;
		}
		self.frame = frame;
		self.contacts.clear();
		self.contacts.extend(hero.map(|h| (h, true)));
		self.contacts.extend(parts.take(256).map(|p| (p, false)));
	}

	/// Places the waiting contacts on the flat water `planes` (`WaterDraw::plane`; the hero from 1.6 below to 0.25 above one, a particle within
	/// 0.4 of it, inside its box) at time `now` (s), drops spent rings, and gives the shader rows and count.
	fn update(&mut self, now: f32, planes: impl Iterator<Item = [f32; 5]>) -> ([[f32; 4]; 2 * RINGS], usize) {
		self.live.retain(|r| now - r.born < r.life && now >= r.born - 1.0);
		let planes: Vec<[f32; 5]> = planes.collect();
		for (p, hero) in std::mem::take(&mut self.contacts) {
			let (lo, hi) = if hero { (-1.6, 0.25) } else { (-0.4, 0.4) };
			let on = planes.iter().find(|b| p[0] >= b[1] - 0.3 && p[0] <= b[3] + 0.3 && p[1] >= b[2] - 0.3 && p[1] <= b[4] + 0.3 && p[2] - b[0] >= lo && p[2] - b[0] <= hi);
			let Some(b) = on else {
				if hero {
					self.hero_last = None;
				}
				continue;
			};
			let c = [p[0], p[1], b[0]];
			if hero {
				let moved = self.hero_last.map_or(1.0, |q| ((q[0] - p[0]).powi(2) + (q[1] - p[1]).powi(2)).sqrt());
				self.hero_last = Some(p);
				let (gap, slope, froth) = if moved > 0.02 { (HERO_GAP, 0.8, 0.9) } else { (IDLE_GAP, 0.3, 0.2) };
				if now - self.hero_t < gap && now >= self.hero_t {
					continue;
				}
				self.hero_t = now;
				self.push(Ring { p: c, born: now, slope, froth, life: 1.8, size: 1.0, patch: 0.0 }, true);
			} else {
				let near = |r: &Ring| (r.p[0] - c[0]).powi(2) + (r.p[1] - c[1]).powi(2) < PART_GAP.0 * PART_GAP.0 && now - r.born < PART_GAP.1;
				if !self.live.iter().any(near) {
					self.push(Ring { p: c, born: now, slope: 0.4, froth: 0.6, life: 1.2, size: 1.0, patch: 0.0 }, false);
				}
			}
		}
		let mut rows = [[0.0; 4]; 2 * RINGS];
		let mut k = 0;
		for r in self.live.iter().filter(|r| now >= r.born) {
			let age = now - r.born;
			let a = age / r.size.sqrt();
			let rad = (0.15 + 1.3 * a - 0.15 * a * a) * r.size;
			let fade = (1.0 - age / r.life).powi(2);
			rows[2 * k] = [r.p[0], r.p[1], r.p[2], rad];
			rows[2 * k + 1] = [r.slope * fade / (1.0 + 0.5 * rad / r.size), (0.22 + 0.15 * a) * r.size.sqrt(), r.froth * (1.0 - a / 0.9).max(0.0), r.patch * fade];
			k += 1;
		}
		(rows, k)
	}

	/// [PC] Heavy splashes (`Wakes::update`: something came down into still water fast, or a burst of particles hit it): two
	/// big rings, the second a moment later, with a froth crest and foam left where it went in.
	fn splash(&mut self, now: f32, p: [f32; 3], size: f32) {
		self.push(Ring { p, born: now, slope: 1.5 * size.min(1.2), froth: 1.0, life: 2.4 + size, size: 1.0 + size, patch: 0.8 }, true);
		self.push(Ring { p, born: now + 0.22, slope: 0.8 * size.min(1.2), froth: 0.7, life: 2.0 + size, size: 0.6 + 0.6 * size, patch: 0.0 }, true);
	}

	/// A new ring; when full, the hero's replaces the oldest and a particle's is dropped.
	fn push(&mut self, r: Ring, hero: bool) {
		if self.live.len() < RINGS {
			self.live.push(r);
		} else if hero {
			if let Some(o) = self.live.iter_mut().min_by(|a, b| a.born.total_cmp(&b.born)) {
				*o = r;
			}
		}
	}
}

/// Wake points at most (each mover's head and the path behind it), the spacing of a path's points (s), how long a wake lasts
/// (s), the movers followed besides the hero, a mover's wake strength (the hero's is 1) and the field id of the hero.
const TRAIL: usize = 48;
const TRAIL_GAP: f32 = 0.1;
const WAKE_LIFE: f32 = 2.6;
const MOVERS: usize = 6;
const MOVER_WAKE: f32 = 0.85;
/// A moby's wake starts this share of its bounding radius wide each side, at most MOVER_HALF_MAX.
const MOVER_HALF: f32 = 0.6;
const MOVER_HALF_MAX: f32 = 5.0;
const HERO_ID: u32 = u32::MAX;
/// Churn spots at most, the churn grid (world units), the churn's time constant (s) and the particle hits per second at which
/// a spot starts and is full.
const CHURN: usize = 12;
/// The pool particles the churn cells look at each field.
const PARTS: usize = 2048;
const CHURN_CELL: f32 = 2.0;
const CHURN_TAU: f32 = 2.0;
const CHURN_RATE: (f32, f32) = (60.0, 400.0);
/// The particle hits of one field in one churn cell that make a heavy splash (on top of the spot's own rate).
const BURST: f32 = 10.0;
/// Particles this close to a mover (world units) are its own spray.
const CHURN_SKIP: f32 = 2.5;
/// A waterfall foot's steady churn (hits per second).
const FALL_RATE: f32 = 600.0;
/// [PC] How strongly a river's steep quads (`quad_flows`, opaque in retail) take the falling water (`fs_fall`'s vertex alpha
/// scale: 1.4 = full).
const RIVER_FALL_ALPHA: f32 = 1.4;
/// [PC] A river strip's triangles this steep (normal z under this share of its length, about 40 degrees) take the falling
/// water; the steeper ones (under 0.5, 60 degrees) are the fall whose foot churns. Gentler slopes (Novalis's rapids) keep
/// the approved river look.
const FALL_STEEP: f32 = 0.75;
/// [PC] Over each waterfall foot (`fs_fall`): FALL_PUFFS mist puffs (half size FALL_PUFF_SIZE world units), FALL_DROPS spray
/// drops (half width FALL_DROP_SIZE); on each foot (at most FALL_BOILS) a boiling foam disc (radius FALL_BOIL_SIZE).
const FALL_PUFFS: usize = 6;
const FALL_PUFF_SIZE: f32 = 3.0;
const FALL_DROPS: usize = 20;
const FALL_DROP_SIZE: f32 = 0.04;
const FALL_BOIL_SIZE: f32 = 2.5;
const FALL_BOILS: usize = 48;
/// [PC] A waterfall foot this far (world units) under a flat water's plane is moved up onto it.
const FALL_DEEP: f32 = 4.0;
/// [PC] The waterfall feet in one square this wide (world units) share one set; at most FALL_SPOTS sets a frame.
const FALL_PUFF_MERGE: f32 = 6.0;
const FALL_SPOTS: usize = 16;
/// A flat water's span (world units) from which it is open sea (`WaterDraw::sea`).
const SEA_SIZE: f32 = 120.0;
/// (Lakes start at 48: the 32-unit tiled tanks and pools of Pokitaru / Oltanis stay still like retail.)
const LAKE_SIZE: f32 = 48.0;

/// [PC] Wakes, churn and heavy splashes (Water Enhanced; retail draws none of them, its own splash particles stay as drawn).
/// Every game field (`feed`) the hero's feet and the drawn mobys are matched against the water boxes (`WaterDraw::wake_box`;
/// the hero from 1.6 below to 0.25 above the surface, as `Rings`). A mover in the water leaves a path of points (`TRAIL_GAP`
/// apart, kept `WAKE_LIFE` s); `fs_water` draws a V behind it from that path (`wake_at`). Something that comes down into
/// flat water fast (the hero jumping in, a falling moby) makes a heavy splash (`Rings::splash`). The pool particles that
/// hit the water fill a grid of churn cells (hits per second, `CHURN_TAU` s memory): where they keep falling (the spray at a
/// waterfall's foot, rapids throwing spray) the water churns (`churn_at`: turbulence and foam), and a sudden burst of hits
/// (a blast in the water) is a heavy splash too.
#[derive(Default)]
struct Wakes {
	tracks: Vec<Track>,
	/// The last field's movers not yet placed (id, position, size) and pool particles, the field count, and its time step (s).
	cands: Vec<(u32, [f32; 3], f32)>,
	parts: Vec<[f32; 3]>,
	frame: u32,
	dt: f32,
	/// The hero's feet (`swell` stays off near him).
	hero: Option<[f32; 3]>,
	cells: std::collections::HashMap<(i32, i32), Cell>,
}

struct Track {
	id: u32,
	/// Its path, newest first: (point on the surface, time, motion direction, speed).
	pts: std::collections::VecDeque<([f32; 3], f32, [f32; 2], f32)>,
	/// Where it is now while it moves in the water (point, direction, speed).
	head: Option<([f32; 3], [f32; 2], f32)>,
	last: Option<[f32; 3]>,
	/// Its wake's half width at the mover.
	half: f32,
	/// Last field it was over the water and clear of it (a splash: it came down into it).
	above: bool,
	seen: f32,
}

#[derive(Clone, Copy, Default)]
struct Cell {
	/// Hits per second (decaying), and their position sums decaying the same way.
	rate: f32,
	sum: [f32; 3],
	/// The last burst's time.
	burst: f32,
}

impl Wakes {
	/// This field's movers (the hero's feet, the drawn mobys) and pool particles; nothing new while the field count stands still.
	fn feed(&mut self, frame: u32, hero: Option<[f32; 3]>, movers: &[(u32, [f32; 3], f32)], parts: &[[f32; 3]]) {
		if frame == self.frame {
			return;
		}
		let step = frame.wrapping_sub(self.frame);
		self.dt = if (1..=5).contains(&step) { step as f32 / 50.0 } else { 0.0 };
		self.frame = frame;
		self.hero = hero;
		self.cands.clear();
		self.cands.extend(hero.map(|h| (HERO_ID, h, 0.0)));
		// What rides with the hero (Clank, the held item) leaves no wake of its own.
		let near = |p: &[f32; 3]| hero.is_some_and(|h| (h[0] - p[0]).powi(2) + (h[1] - p[1]).powi(2) + (h[2] - p[2]).powi(2) < 2.25);
		self.cands.extend(movers.iter().filter(|m| !near(&m.1)).copied());
		self.parts.clear();
		self.parts.extend_from_slice(parts);
	}

	/// Places the waiting movers and particles on the water `boxes` at time `now` (s): the shader's rows, and the heavy
	/// splashes to make (flat water only: point on the surface, size).
	fn update(&mut self, now: f32, boxes: &[[f32; 6]], falls: &[[f32; 3]]) -> (WakeRows, Vec<([f32; 3], f32)>) {
		let mut splashes = Vec::new();
		let dt = std::mem::take(&mut self.dt);
		// A new field: a mover not seen in it (gone, hidden) has no head any more.
		if !self.cands.is_empty() {
			for t in &mut self.tracks {
				t.head = None;
			}
		}
		let inside = |b: &[f32; 6], p: [f32; 3], m: f32| p[0] >= b[0] - m && p[0] <= b[3] + m && p[1] >= b[1] - m && p[1] <= b[4] + m;
		for (id, p, size) in std::mem::take(&mut self.cands) {
			// The hero's feet from 1.6 below to 0.25 above the surface; a moby's origin (a raft's, a fish's) from 2 below to 1.5 above.
			let (lo, hi) = if id == HERO_ID { (1.6, 0.25) } else { (2.0, 1.5) };
			let wet = boxes.iter().find(|b| inside(b, p, 0.3) && p[2] >= b[2] - lo && p[2] <= b[5] + hi);
			let k = match self.tracks.iter().position(|t| t.id == id) {
				Some(k) => k,
				None if wet.is_some() && (id == HERO_ID || self.tracks.iter().filter(|t| t.id != HERO_ID).count() < MOVERS) => {
					self.tracks.push(Track { id, pts: Default::default(), head: None, last: None, half: 0.3, above: false, seen: now });
					self.tracks.len() - 1
				}
				None => continue,
			};
			let t = &mut self.tracks[k];
			// The hero's is narrow; a moby's starts at about its width (its bounding sphere, a raft's spans its hull).
			t.half = if id == HERO_ID { 0.3 } else { (size * MOVER_HALF).clamp(0.5, MOVER_HALF_MAX) };
			let v = match t.last {
				Some(l) if dt > 0.0 => [(p[0] - l[0]) / dt, (p[1] - l[1]) / dt, (p[2] - l[2]) / dt],
				_ => [0.0; 3],
			};
			// A jump (a respawn, a cut) is no motion.
			let v = if v[0].abs() + v[1].abs() + v[2].abs() > 60.0 { [0.0; 3] } else { v };
			let speed = (v[0] * v[0] + v[1] * v[1]).sqrt();
			let dir = if speed > 1e-3 { [v[0] / speed, v[1] / speed] } else { [0.0, 0.0] };
			t.head = None;
			if let Some(b) = wet {
				if t.above && v[2] < -4.0 && b[2] == b[5] {
					splashes.push(([p[0], p[1], b[2]], (-v[2] / 8.0).clamp(0.6, 1.6) * if id == HERO_ID { 1.0 } else { MOVER_WAKE }));
				}
				if speed > 0.3 {
					// The wake lies on the surface: a flat water's height, else the hero's feet raised to about the waterline
					// (a moby's origin) within the water's box.
					let surf = if b[2] == b[5] { b[2] } else { (p[2] + if id == HERO_ID { 0.7 } else { 0.0 }).clamp(b[2], b[5]) };
					let w = [p[0], p[1], surf];
					if t.pts.front().is_none_or(|q| now - q.1 >= TRAIL_GAP || now < q.1) {
						t.pts.push_front((w, now, dir, speed));
					}
					t.head = Some((w, dir, speed));
				}
			}
			t.above = wet.is_none() && boxes.iter().any(|b| inside(b, p, 0.0) && p[2] > b[5] + 0.25 && p[2] < b[5] + 8.0);
			t.last = Some(p);
			t.seen = now;
		}
		for t in &mut self.tracks {
			while t.pts.back().is_some_and(|q| now - q.1 > WAKE_LIFE || now < q.1 - 1.0) {
				t.pts.pop_back();
			}
		}
		self.tracks.retain(|t| !t.pts.is_empty() || t.head.is_some() || now - t.seen < 1.0);
		// The churn cells: decay, then this field's hits (a particle within the water's box, from its surface to 1 above it;
		// bubbles rising under it do not count).
		if dt > 0.0 {
			let keep = (-dt / CHURN_TAU).exp();
			for c in self.cells.values_mut() {
				c.rate *= keep;
				c.sum = c.sum.map(|s| s * keep);
			}
			let mut field: std::collections::HashMap<(i32, i32), (f32, [f32; 3])> = std::collections::HashMap::new();
			// The spray a mover throws up (the hero swimming) is its wake's, not churn.
			let heads: Vec<[f32; 3]> = self.tracks.iter().filter_map(|t| t.head.map(|h| h.0)).chain(self.hero).collect();
			for &p in self.parts.iter().filter(|p| !heads.iter().any(|h| (h[0] - p[0]).powi(2) + (h[1] - p[1]).powi(2) < CHURN_SKIP * CHURN_SKIP)) {
				if let Some(b) = boxes.iter().find(|b| inside(b, p, 0.3) && p[2] >= b[2] - 0.15 && p[2] <= b[5] + 1.0) {
					let e = field.entry(((p[0] / CHURN_CELL).floor() as i32, (p[1] / CHURN_CELL).floor() as i32)).or_default();
					e.0 += 1.0;
					e.1 = [e.1[0] + p[0], e.1[1] + p[1], e.1[2] + p[2].min(b[5])];
				}
			}
			// A waterfall's foot on the water churns steadily, spray or not.
			for &f in falls {
				if let Some(b) = boxes.iter().find(|b| inside(b, f, 0.5) && f[2] >= b[2] - 1.0 && f[2] <= b[5] + 2.0) {
					let k = FALL_RATE * dt;
					let e = field.entry(((f[0] / CHURN_CELL).floor() as i32, (f[1] / CHURN_CELL).floor() as i32)).or_default();
					e.0 += k;
					e.1 = [e.1[0] + f[0] * k, e.1[1] + f[1] * k, e.1[2] + f[2].min(b[5]) * k];
				}
			}
			for (key, (n, s)) in field {
				let c = self.cells.entry(key).or_default();
				if n >= BURST + 4.0 * c.rate * dt && now - c.burst > 1.0 {
					c.burst = now;
					let at = [s[0] / n, s[1] / n, s[2] / n];
					if let Some(b) = boxes.iter().find(|b| b[2] == b[5] && inside(b, at, 0.0) && (at[2] - b[2]).abs() < 1.0) {
						splashes.push(([at[0], at[1], b[2]], (n / 20.0).clamp(0.6, 1.6)));
					}
				}
				c.rate += n / CHURN_TAU;
				c.sum = [c.sum[0] + s[0] / CHURN_TAU, c.sum[1] + s[1] / CHURN_TAU, c.sum[2] + s[2] / CHURN_TAU];
			}
			self.cells.retain(|_, c| c.rate > 0.05);
		}
		(self.rows(now), splashes)
	}

	/// The shader's rows: per wake point (x, y, surface z, age), (half width at the mover, 0, speed, strength when the next
	/// point is the same mover's, else 0), their box grown by the widest wake and the furthest drift, and per churn spot (x, y, z, radius),
	/// (strength, 0, 0, 0).
	fn rows(&self, now: f32) -> WakeRows {
		let mut r = WakeRows { trail: [[0.0; 4]; 2 * TRAIL], n: 0, wbox: [0.0; 4], churn: [[0.0; 4]; 2 * CHURN], cn: 0 };
		let mut bx = [f32::MAX, f32::MAX, f32::MIN, f32::MIN];
		for t in &self.tracks {
			let (str, half) = (if t.id == HERO_ID { 1.0 } else { MOVER_WAKE }, t.half);
			let head = t.head.map(|(p, d, s)| (p, now, d, s));
			let pts: Vec<_> = head.into_iter().chain(t.pts.iter().copied().filter(|q| head.is_none_or(|h| q.0 != h.0))).collect();
			if pts.len() < 2 || r.n + 2 > TRAIL {
				continue;
			}
			let take = pts.len().min(TRAIL - r.n);
			for (k, q) in pts.iter().take(take).enumerate() {
				let age = (now - q.1).max(0.0);
				r.trail[2 * r.n] = [q.0[0], q.0[1], q.0[2], age];
				r.trail[2 * r.n + 1] = [half, 0.0, q.3, if k + 1 == take { 0.0 } else { str }];
				let grow = half + 1.3 + age * (0.35 * q.3).min(2.2);
				bx = [bx[0].min(q.0[0] - grow), bx[1].min(q.0[1] - grow), bx[2].max(q.0[0] + grow), bx[3].max(q.0[1] + grow)];
				r.n += 1;
			}
		}
		if r.n > 0 {
			let g = 6.0 * WAKE_LIFE;
			r.wbox = [bx[0] - g, bx[1] - g, bx[2] + g, bx[3] + g];
		}
		let mut cells: Vec<_> = self.cells.values().filter(|c| c.rate >= CHURN_RATE.0).collect();
		cells.sort_by(|a, b| b.rate.total_cmp(&a.rate));
		for c in cells.into_iter().take(CHURN) {
			let at = c.sum.map(|v| v / c.rate);
			r.churn[2 * r.cn] = [at[0], at[1], at[2], CHURN_CELL * 1.4];
			r.churn[2 * r.cn + 1] = [((c.rate - CHURN_RATE.0) / (CHURN_RATE.1 - CHURN_RATE.0)).clamp(0.0, 1.0), 0.0, 0.0, 0.0];
			r.cn += 1;
		}
		r
	}
}

/// What `Wakes` gives the shader (`WaterU`).
struct WakeRows {
	trail: [[f32; 4]; 2 * TRAIL],
	n: usize,
	wbox: [f32; 4],
	churn: [[f32; 4]; 2 * CHURN],
	cn: usize,
}

/// Flow table slots (slot 0 = still water).
const FLOWS: usize = 64;
/// The flow slot of still water that retail draws opaque (Hoven's FIX 128 pools): ripples and reflections, no clear shallows.
pub(super) const OPAQUE_SLOT: u32 = FLOWS as u32 - 1;
/// The flow slot of Hoven's abyss water (`cold_pool`): opaque like `OPAQUE_SLOT`, with the cold look `COLD_LOOK`.
pub(super) const COLD_SLOT: u32 = FLOWS as u32 - 2;
/// The flow slots of Hoven's frozen floor (`frozen_pool`), held still (flows w = -1): opaque like `OPAQUE_SLOT`, and clear
/// like slot 0.
pub(super) const FROZEN_SLOT: u32 = FLOWS as u32 - 3;
pub(super) const FROZEN_CLEAR_SLOT: u32 = FLOWS as u32 - 4;
/// The flow slot of Gaspar's lava (`WaterDraw::liquid`, flows w = -2): `fs_water`'s lava look.
pub(super) const LAVA_SLOT: u32 = FLOWS as u32 - 5;
/// The flow slots of Aridia's mud (`liquid_slot`, flows w = -4) and Orxon's toxic sludge (`terrain_liquid`, flows w = -5):
/// `fs_water`'s mud and sludge looks.
pub(super) const MUD_SLOT: u32 = FLOWS as u32 - 6;
pub(super) const SLUDGE_SLOT: u32 = FLOWS as u32 - 7;
/// The lowest reserved slot: flowing water takes the slots below it.
const LAST_FLOW: u32 = SLUDGE_SLOT;
/// [PC] The cold look of Hoven's abyss (`fs_water`): 1 = long slow swells and a higher reflectance floor (what stands in the
/// water mirrors), 2 = also a cool light on the swell and a moonlit shimmer, 3 = also an icy rim where it meets the ground.
const COLD_LOOK: f32 = 2.0;

/// Levels whose world strips (`Gpu::strip_draws`) are water: Novalis, whose strip classes 676 / 678 / 761 / 1225 are its
/// river, waterfall pool and cascade pools (retail/queue_q1.rs `STRIP_FN`; decomp/notes/water.md NOV-23.1).
pub(super) fn strips_are_water(level: usize) -> bool {
	level == 1
}

/// Whether a world strip's water (Novalis' river and pools) drifts its ripples with the strip's texture scroll. Off: the
/// ripples stand in world space, so neighbouring pieces match and nothing jumps; retail's own scroll stays in the picture.
const STRIP_FLOW: bool = false;

/// A world strip's texture drift: the mean of its uv last frame and the smoothed uv step per game tick.
#[derive(Clone, Copy, Default)]
struct Drift {
	mean: [f32; 2],
	step: [f32; 2],
	seen: bool,
}

struct Targets {
	key: (u32, u32, u64),
	wpos: wgpu::TextureView,
	wdepth: wgpu::TextureView,
	copy: wgpu::Texture,
	g_pos: wgpu::BindGroup,
	g_shade: wgpu::BindGroup,
	/// The planar mirrors (mirror.rs), half size: MSAA colour and depth (shared by the passes, each clears them), the resolve
	/// (`MAX_MIRRORS` layers, read by `fs_water` as an array) and its single-layer views for the passes.
	mirror_ms: wgpu::TextureView,
	mirror_layers: Vec<wgpu::TextureView>,
	mirror_depth: wgpu::TextureView,
	/// The water bed (mirror.rs `draw_bed`), half size: MSAA colour, its resolve and MSAA depth (both read by `fs_water`).
	bed_ms: wgpu::TextureView,
	bed: wgpu::TextureView,
	bed_depth: wgpu::TextureView,
	/// [PC] Geometric sea waves: the flat sea mask (`fs_mask`), the grid's group (with the mask) and the shore map's group.
	seamask: wgpu::TextureView,
	g_grid: wgpu::BindGroup,
	g_shore: wgpu::BindGroup,
}


pub(super) struct WaterFx {
	layout: wgpu::BindGroupLayout,
	ubuf: wgpu::Buffer,
	sampler: wgpu::Sampler,
	p_pos: wgpu::RenderPipeline,
	p_shade: wgpu::RenderPipeline,
	p_ao_sea: wgpu::RenderPipeline,
	dummy: wgpu::TextureView,
	/// The same as an array view (the mirror slot).
	dummy_arr: wgpu::TextureView,
	dummy_r32: wgpu::TextureView,
	format: wgpu::TextureFormat,
	depth_format: wgpu::TextureFormat,
	start: std::time::Instant,
	t: Option<Targets>,
	drift: std::collections::HashMap<(u32, u32, u32, u32), Drift>,
	flows: Vec<[f32; 2]>,
	/// The river quads' uv last frame by the bits of their first corner, and the smoothed uv step per tick (`quad_flows`).
	river_uv: std::collections::HashMap<[u32; 3], [f32; 2]>,
	river_step: [f32; 2],
	/// This frame's flows are river quads (`quad_flows`): they flow in two phases (flows w = -3).
	phased: bool,
	rings: Rings,
	wakes: Wakes,
	/// [PC] This frame's waterfall feet (`quad_flows`: the bottom of each steep river quad) and whether `quad_flows` ran.
	falls: Vec<[f32; 3]>,
	falls_fresh: bool,
	/// [PC] This frame's waterfall sheets (`sheet_falls`) and, made in `positions`, their draw with the mist puffs (`fs_fall`).
	p_fall: wgpu::RenderPipeline,
	/// Group 1 of `p_fall` on a level without a waterfall texture (`fall_fx_tex`).
	fall_dummy: wgpu::BindGroup,
	fall_verts: Vec<Vertex>,
	fall: Option<(wgpu::Buffer, wgpu::Buffer, u32)>,
	/// [PC] Pokitaru's sheets as seen over time (`SheetMem`): their shape does not depend on which triangles retail drew this frame.
	sheet_mem: SheetMem,
	/// The level `sheet_mem` was filled in.
	sheet_level: usize,
	/// [PC] Geometric sea waves: the mask, grid and shore map pipelines, the shore map's layout, buffer and uniforms, the grid's
	/// index buffer (and its columns, rows), and this frame's grid draw (planes, indices) for `sea_waves`.
	p_mask: wgpu::RenderPipeline,
	p_grid: wgpu::RenderPipeline,
	p_shore: wgpu::ComputePipeline,
	p_shore_h: wgpu::ComputePipeline,
	p_shore_v: wgpu::ComputePipeline,
	shore_layout: wgpu::BindGroupLayout,
	shore: wgpu::Buffer,
	shore_u: wgpu::Buffer,
	grid_ib: Option<(wgpu::Buffer, u32, u32)>,
	geo_draw: Option<(u32, u32)>,
	/// [PC] Geometric sea: per prop slot its last place and turn and the frames since it last moved (`track_props`), and the
	/// slots that moved within SHORE_STILL frames (`shore_movers`).
	shore_still: Vec<([f32; 6], u32)>,
	shore_movers: Vec<usize>,
}

/// What the shading pass needs of the frame.
pub(super) struct WaterFrame {
	pub vp: [[f32; 4]; 4],
	pub eye: [f32; 3],
	pub proj: [f32; 4],
	/// The key light's travel direction and brightness (`sunshadow::key_light`), if any.
	pub key: Option<([f32; 3], f32)>,
	/// The level's fog colour (gamma-space 0..1).
	pub fog: [f32; 3],
	/// Graphics level of Water (1 Low, 2 Medium, otherwise High): `march_steps`.
	pub level: u8,
	/// Graphics -> Waves (retail/pcext.rs `Graphics::waves`): the geometric sea waves (`geo_look`); Off keeps the flat sea.
	pub waves: bool,
}

/// Steps of the reflection ray march (`reflect_ray`) by Graphics level; High = the 32 of before the levels.
pub(super) const fn march_steps(level: u8) -> f32 {
	match level {
		1 => 16.0,
		2 => 24.0,
		_ => 32.0,
	}
}

/// [PC] Sea state by Graphics level of Water (`waves`): 1 Low = none (the flat retail surface with its ripples), 2 Medium = four
/// trains, no shade or whitecaps, 3 High = seven trains with both. RAC_WAVES=off (dev aid): none at any level; RAC_WAVES=N: slope gain N.
fn wave_look(level: u8) -> [f32; 4] {
	static GAIN: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
	let gain = *GAIN.get_or_init(|| match std::env::var("RAC_WAVES") {
		Ok(v) if v == "off" => 0.0,
		Ok(v) => v.parse::<f32>().unwrap_or(1.0),
		Err(_) => 1.0,
	});
	if gain == 0.0 {
		return [0.0; 4];
	}
	// RAC_WAVE_CAPT=x (dev aid): whitecap threshold shift.
	static CAPT: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
	let capt = *CAPT.get_or_init(|| std::env::var("RAC_WAVE_CAPT").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0));
	match level {
		1 => [0.0; 4],
		2 => [4.0, 0.0, gain, 0.0],
		_ => [7.0, 1.0, gain, capt],
	}
}

impl WaterFx {
	pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat, tex_layout: &wgpu::BindGroupLayout) -> WaterFx {
		// RAC_WATER_DBG=N (dev aid, off by default): fs_water returns one of its inputs as a colour (seam hunting).
		let dbg = match std::env::var("RAC_WATER_DBG").ok().and_then(|v| v.parse::<u32>().ok()) {
			Some(1) => "return vec4<f32>(clamp(thick / 4.0, 0.0, 1.0), select(0.0, 1.0, blended), fract((p.z - 223.0) * 50.0), 1.0);",
			Some(2) => "return vec4<f32>(g.x * 4.0 + 0.5, g.y * 4.0 + 0.5, g.z, 1.0);",
			Some(3) => "return vec4<f32>((uv - uv0) * 60.0 + vec2<f32>(0.5), clamp((zs - zw) / 2.0, 0.0, 1.0), 1.0);",
			Some(4) => "return vec4<f32>(rs * 3.0 + vec2<f32>(0.5), 0.5, 1.0);",
			Some(5) => "return vec4<f32>(orig, 1.0);",
			_ => "",
		};
		let src = SHADER.replace("//DBGHOOK", dbg);
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water fx"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("water fx", src.into())) });
		let tex = |binding: u32, sample_type: wgpu::TextureSampleType, multisampled: bool| wgpu::BindGroupLayoutEntry {
			binding,
			visibility: wgpu::ShaderStages::FRAGMENT,
			ty: wgpu::BindingType::Texture { sample_type, view_dimension: wgpu::TextureViewDimension::D2, multisampled },
			count: None,
		};
		let unfiltered = wgpu::TextureSampleType::Float { filterable: false };
		let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("water fx"),
			entries: &[
				wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
				tex(1, wgpu::TextureSampleType::Depth, true),
				tex(2, wgpu::TextureSampleType::Float { filterable: true }, false),
				tex(3, unfiltered, false),
				tex(4, unfiltered, false),
				wgpu::BindGroupLayoutEntry { binding: 5, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
				wgpu::BindGroupLayoutEntry { binding: 6, visibility: wgpu::ShaderStages::FRAGMENT, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false }, count: None },
				tex(7, unfiltered, false),
				tex(8, wgpu::TextureSampleType::Float { filterable: true }, false),
				tex(9, wgpu::TextureSampleType::Depth, true),
				tex(10, unfiltered, false),
				wgpu::BindGroupLayoutEntry { binding: 11, visibility: wgpu::ShaderStages::VERTEX_FRAGMENT, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: true }, has_dynamic_offset: false, min_binding_size: None }, count: None },
			],
		});
		let pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water fx"), bind_group_layouts: &[Some(&layout)], immediate_size: 0 });
		let p_pos = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water positions"),
			layout: Some(&pl),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_pos"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &[wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }] })],
			},
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_pos"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: WPOS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let p_shade = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water shade"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_water"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let p_ao_sea = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water sea occlusion"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_full"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs_ao_sea"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R8Unorm, blend: Some(wgpu::BlendState::ALPHA_BLENDING), write_mask: wgpu::ColorWrites::ALL })],
			}),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		// [PC] Geometric sea waves: the flat sea mask, the projected grid, the shore map.
		let p_mask = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water sea mask"),
			layout: Some(&pl),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_pos"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout { array_stride: std::mem::size_of::<Vertex>() as u64, step_mode: wgpu::VertexStepMode::Vertex, attributes: &[wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }] })],
			},
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_mask"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: wgpu::TextureFormat::R32Float, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let p_grid = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water sea grid"),
			layout: Some(&pl),
			vertex: wgpu::VertexState { module: &shader, entry_point: Some("vs_grid"), compilation_options: Default::default(), buffers: &[] },
			fragment: Some(wgpu::FragmentState { module: &shader, entry_point: Some("fs_grid"), compilation_options: Default::default(), targets: &[Some(wgpu::ColorTargetState { format: WPOS_FORMAT, blend: None, write_mask: wgpu::ColorWrites::ALL })] }),
			primitive: Default::default(),
			depth_stencil: Some(wgpu::DepthStencilState { format: wgpu::TextureFormat::Depth32Float, depth_write_enabled: Some(true), depth_compare: Some(wgpu::CompareFunction::Less), stencil: Default::default(), bias: Default::default() }),
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let shore_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water shore map"), source: wgpu::ShaderSource::Wgsl(crate::devtune::wgsl("water shore map", SHORE_SHADER.into())) });
		let shore_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
			label: Some("water shore map"),
			entries: &[
				wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None }, count: None },
				wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Depth, view_dimension: wgpu::TextureViewDimension::D2, multisampled: true }, count: None },
				wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::COMPUTE, ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Storage { read_only: false }, has_dynamic_offset: false, min_binding_size: None }, count: None },
			],
		});
		let shore_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water shore map"), bind_group_layouts: &[Some(&shore_layout)], immediate_size: 0 });
		let shore_pipe = |entry: &str| {
			device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
				label: Some("water shore map"),
				layout: Some(&shore_pl),
				module: &shore_shader,
				entry_point: Some(entry),
				compilation_options: Default::default(),
				cache: None,
			})
		};
		let (p_shore, p_shore_h, p_shore_v) = (shore_pipe("cs_shore"), shore_pipe("cs_shore_h"), shore_pipe("cs_shore_v"));
		// The waterfalls also see retail's waterfall texture (group 1, a level texture's bind group).
		let pl_fall = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("water falls"), bind_group_layouts: &[Some(&layout), Some(tex_layout)], immediate_size: 0 });
		let p_fall = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
			label: Some("water falls"),
			layout: Some(&pl_fall),
			vertex: wgpu::VertexState {
				module: &shader,
				entry_point: Some("vs_fall"),
				compilation_options: Default::default(),
				buffers: &[Some(wgpu::VertexBufferLayout {
					array_stride: std::mem::size_of::<Vertex>() as u64,
					step_mode: wgpu::VertexStepMode::Vertex,
					attributes: &wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x2, 2 => Float32x3, 3 => Float32x4, 4 => Float32x4],
				})],
			},
			fragment: Some(wgpu::FragmentState {
				module: &shader,
				entry_point: Some("fs_fall"),
				compilation_options: Default::default(),
				targets: &[Some(wgpu::ColorTargetState {
					format,
					blend: Some(wgpu::BlendState {
						color: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::One, dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha, operation: wgpu::BlendOperation::Add },
						alpha: wgpu::BlendComponent { src_factor: wgpu::BlendFactor::Zero, dst_factor: wgpu::BlendFactor::One, operation: wgpu::BlendOperation::Add },
					}),
					write_mask: wgpu::ColorWrites::ALL,
				})],
			}),
			primitive: Default::default(),
			depth_stencil: None,
			multisample: Default::default(),
			multiview_mask: None,
			cache: None,
		});
		let fall_dummy = {
			let view = device
				.create_texture(&wgpu::TextureDescriptor { label: Some("water falls dummy"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format: wgpu::TextureFormat::Rgba8Unorm, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] })
				.create_view(&Default::default());
			let sampler = device.create_sampler(&Default::default());
			device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("water falls dummy"), layout: tex_layout, entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }, wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&sampler) }] })
		};
		let small = |format| {
			device
				.create_texture(&wgpu::TextureDescriptor { label: Some("water fx dummy"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] })
				.create_view(&Default::default())
		};
		WaterFx {
			layout,
			ubuf: device.create_buffer(&wgpu::BufferDescriptor { label: Some("water fx uniforms"), size: std::mem::size_of::<WaterU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
			sampler: device.create_sampler(&wgpu::SamplerDescriptor { label: Some("water fx linear"), mag_filter: wgpu::FilterMode::Linear, min_filter: wgpu::FilterMode::Linear, address_mode_u: wgpu::AddressMode::ClampToEdge, address_mode_v: wgpu::AddressMode::ClampToEdge, ..Default::default() }),
			p_pos,
			p_shade,
			p_ao_sea,
			dummy: small(format),
			dummy_arr: device
				.create_texture(&wgpu::TextureDescriptor { label: Some("water fx dummy array"), size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage: wgpu::TextureUsages::TEXTURE_BINDING, view_formats: &[] })
				.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() }),
			dummy_r32: small(WPOS_FORMAT),
			format,
			depth_format,
			start: std::time::Instant::now(),
			t: None,
			drift: std::collections::HashMap::new(),
			flows: vec![[0.0; 2]],
			river_uv: std::collections::HashMap::new(),
			river_step: [0.0; 2],
			phased: false,
			rings: Rings::default(),
			wakes: Wakes::default(),
			falls: Vec::new(),
			falls_fresh: false,
			p_fall,
			fall_dummy,
			fall_verts: Vec::new(),
			fall: None,
			sheet_mem: SheetMem::default(),
			sheet_level: usize::MAX,
			p_mask,
			p_grid,
			p_shore,
			p_shore_h,
			p_shore_v,
			shore_layout,
			shore: device.create_buffer(&wgpu::BufferDescriptor { label: Some("water shore map"), size: (4 * SHORE_N * SHORE_N * 4) as u64, usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
			shore_u: device.create_buffer(&wgpu::BufferDescriptor { label: Some("water shore map"), size: std::mem::size_of::<ShoreU>() as u64, usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
			grid_ib: None,
			geo_draw: None,
			shore_still: Vec::new(),
			shore_movers: Vec::new(),
		}
	}

	/// The targets for the post's current ones (`post::Post::water_inputs`): (re)made when the window or the post targets change.
	fn prepare(&mut self, device: &wgpu::Device, depth: &wgpu::TextureView, scene: &wgpu::TextureView, lin: &wgpu::TextureView, glow: &wgpu::TextureView, generation: u64) {
		let tex = scene.texture();
		let (w, h) = (tex.width(), tex.height());
		if self.t.as_ref().is_some_and(|t| t.key == (w, h, generation)) {
			return;
		}
		let make = |label, format, usage| {
			device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 }, mip_level_count: 1, sample_count: 1, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] })
		};
		let wpos = make("water positions", WPOS_FORMAT, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
		let wdepth = make("water depth", wgpu::TextureFormat::Depth32Float, wgpu::TextureUsages::RENDER_ATTACHMENT).create_view(&Default::default());
		let copy = make("water scene copy", self.format, wgpu::TextureUsages::COPY_DST | wgpu::TextureUsages::TEXTURE_BINDING);
		let copy_v = copy.create_view(&Default::default());
		let depth_only = depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		let half = |label, format, samples, usage| {
			let size = wgpu::Extent3d { width: (w / 2).max(1), height: (h / 2).max(1), depth_or_array_layers: 1 };
			device.create_texture(&wgpu::TextureDescriptor { label: Some(label), size, mip_level_count: 1, sample_count: samples, dimension: wgpu::TextureDimension::D2, format, usage, view_formats: &[] }).create_view(&Default::default())
		};
		let mirror_ms = half("water mirror msaa", self.format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT);
		let mirror_tex = device.create_texture(&wgpu::TextureDescriptor {
			label: Some("water mirror"),
			size: wgpu::Extent3d { width: (w / 2).max(1), height: (h / 2).max(1), depth_or_array_layers: super::mirror::MAX_MIRRORS as u32 },
			mip_level_count: 1,
			sample_count: 1,
			dimension: wgpu::TextureDimension::D2,
			format: self.format,
			usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
			view_formats: &[],
		});
		let mirror = mirror_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2Array), ..Default::default() });
		let mirror_layers: Vec<wgpu::TextureView> = (0..super::mirror::MAX_MIRRORS as u32)
			.map(|k| mirror_tex.create_view(&wgpu::TextureViewDescriptor { dimension: Some(wgpu::TextureViewDimension::D2), base_array_layer: k, array_layer_count: Some(1), ..Default::default() }))
			.collect();
		let mirror_depth = half("water mirror depth", self.depth_format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT);
		let bed_ms = half("water bed msaa", self.format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT);
		let bed = half("water bed", self.format, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
		let bed_depth = half("water bed depth", self.depth_format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
		let bed_depth_only = bed_depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		let seamask = make("water sea mask", wgpu::TextureFormat::R32Float, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING).create_view(&Default::default());
		let group = |scene_v: &wgpu::TextureView, lin_v: &wgpu::TextureView, wpos_v: &wgpu::TextureView, mirror_v: &wgpu::TextureView, glow_v: &wgpu::TextureView, bed_v: &wgpu::TextureView, mask_v: &wgpu::TextureView| {
			device.create_bind_group(&wgpu::BindGroupDescriptor {
				label: Some("water fx"),
				layout: &self.layout,
				entries: &[
					wgpu::BindGroupEntry { binding: 0, resource: self.ubuf.as_entire_binding() },
					wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&depth_only) },
					wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(scene_v) },
					wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(lin_v) },
					wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::TextureView(wpos_v) },
					wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::Sampler(&self.sampler) },
					wgpu::BindGroupEntry { binding: 6, resource: wgpu::BindingResource::TextureView(mirror_v) },
					wgpu::BindGroupEntry { binding: 7, resource: wgpu::BindingResource::TextureView(glow_v) },
					wgpu::BindGroupEntry { binding: 8, resource: wgpu::BindingResource::TextureView(bed_v) },
					wgpu::BindGroupEntry { binding: 9, resource: wgpu::BindingResource::TextureView(&bed_depth_only) },
					wgpu::BindGroupEntry { binding: 10, resource: wgpu::BindingResource::TextureView(mask_v) },
					wgpu::BindGroupEntry { binding: 11, resource: self.shore.as_entire_binding() },
				],
			})
		};
		let g_pos = group(&self.dummy, &self.dummy_r32, &self.dummy_r32, &self.dummy_arr, &self.dummy, &self.dummy, &self.dummy_r32);
		let g_grid = group(&self.dummy, &self.dummy_r32, &self.dummy_r32, &self.dummy_arr, &self.dummy, &self.dummy, &seamask);
		let g_shade = group(&copy_v, lin, &wpos, &mirror, glow, &bed, &seamask);
		let g_shore = device.create_bind_group(&wgpu::BindGroupDescriptor {
			label: Some("water shore map"),
			layout: &self.shore_layout,
			entries: &[
				wgpu::BindGroupEntry { binding: 0, resource: self.shore_u.as_entire_binding() },
				wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&bed_depth_only) },
				wgpu::BindGroupEntry { binding: 2, resource: self.shore.as_entire_binding() },
			],
		});
		self.t = Some(Targets { key: (w, h, generation), wpos, wdepth, copy, g_pos, g_shade, mirror_ms, mirror_layers, mirror_depth, bed_ms, bed, bed_depth, seamask, g_grid, g_shore });
	}

	/// After the level pass: the water meshes `draws` into the position target. Call before the post chain; `depth` is the level
	/// pass's MSAA depth, `scene` / `lin` / `glow` / `generation` the post's targets (`post::Post::water_inputs`); `parts`: the
	/// particle pass ran (Particles Enhanced), so the glow target holds their cover.
	#[allow(clippy::too_many_arguments)]
	pub(super) fn positions(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, depth: &wgpu::TextureView, (scene, lin, glow, generation): (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView, u64), f: &WaterFrame, draws: &[WaterDraw], mirrors: &[f32], parts: bool) {
		self.prepare(device, depth, scene, lin, glow, generation);
		let Some(t) = &self.t else { return };
		let (w, h) = (t.key.0 as f32, t.key.1 as f32);
		let (sun, glint) = match f.key {
			// Travelling down (from above the horizon): the glint points back up the light.
			Some((d, luma)) if d[2] < -0.05 => ([-d[0], -d[1], -d[2]], GLINT * ((luma - 0.1) / 0.5).clamp(0.0, 1.0)),
			_ => ([0.0, 0.0, 1.0], 0.0),
		};
		let fog = f.fog.map(|c| c.max(0.0).powf(2.2));
		// RAC_WATER_TIME=T (dev aid): the water's clock held at T seconds (same-time A/B pictures).
		static FIXED: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
		let now = FIXED.get_or_init(|| std::env::var("RAC_WATER_TIME").ok().and_then(|v| v.parse::<f32>().ok())).unwrap_or_else(|| self.start.elapsed().as_secs_f32());
		let boxes: Vec<[f32; 6]> = draws.iter().filter_map(|d| d.wake_box()).collect();
		if !std::mem::take(&mut self.falls_fresh) {
			self.falls.clear();
		}
		let (wake, splashes) = self.wakes.update(now, &boxes, &self.falls);
		for (p, size) in splashes {
			self.rings.splash(now, p, size);
		}
		let (rings, rn) = self.rings.update(now, draws.iter().filter_map(|d| d.plane));
		let hero = self.wakes.hero;
		// [PC] Geometric sea waves: the sea planes over which the camera is (one projected grid each, `sea_waves`), the grid
		// and the shore map round the camera.
		let look = geo_look(f.level).filter(|_| f.waves);
		let mut planes: Vec<[f32; 4]> = Vec::new();
		if look.is_some() {
			for d in draws.iter().filter(|d| d.sea()) {
				let z = d.plane.map_or(0.0, |b| b[0]);
				if f.eye[2] <= z + 0.05 {
					continue;
				}
				let bits = if d.kind == "sea fx quads" { 128.0 } else { 64.0 } + 512.0;
				let full = planes.len() >= GEO_PLANES;
				match planes.iter_mut().find(|p| (p[0] - z).abs() < 1.5) {
					Some(p) => p[1] = p[1].max(bits),
					None if !full => planes.push([z, bits, 0.0, 0.0]),
					None => {}
				}
			}
		}
		let in_geo = |d: &WaterDraw| d.sea() && d.plane.is_some_and(|b| planes.iter().any(|p| (p[0] - b[0]).abs() < 1.5));
		let g = look.unwrap_or(GeoLook { cell: 8.0, len: 9.0, sig: 0.0, sigq: 1.0, amp: 0.0 });
		let [mx, mb, mt] = GRID_MARGIN;
		let cols = ((w * (1.0 + mx) / g.cell).ceil() as u32).max(1);
		let rows = ((h * (1.0 + 0.5 * (mb + mt)) / g.cell).ceil() as u32).max(1);
		let ivp = super::mirror::inverse(&f.vp).unwrap_or(f.vp);
		let origin = [((f.eye[0] / SHORE_CELL).floor() - (SHORE_N / 2) as f32) * SHORE_CELL, ((f.eye[1] / SHORE_CELL).floor() - (SHORE_N / 2) as f32) * SHORE_CELL];
		let u = WaterU {
			vp: f.vp,
			eye: [f.eye[0], f.eye[1], f.eye[2], now % 3600.0],
			sun: [sun[0], sun[1], sun[2], glint],
			proj: f.proj,
			size: [w, h, 1.0 / w, 1.0 / h],
			look: [RIPPLES, REFRACTION, SHEEN, FOAM],
			fog: [fog[0], fog[1], fog[2], REFLECTION],
			mirror: [mirrors.first().copied().unwrap_or(0.0), !mirrors.is_empty() as u32 as f32, parts as u32 as f32, CLEAR],
			flows: std::array::from_fn(|k| if k == OPAQUE_SLOT as usize { [0.0; 4] } else if k == COLD_SLOT as usize { [0.0, 0.0, 0.0, COLD_LOOK] } else if k == FROZEN_SLOT as usize { [0.0, 0.0, 0.0, -1.0] } else if k == FROZEN_CLEAR_SLOT as usize { [0.0, 0.0, 1.0, -1.0] } else if k == LAVA_SLOT as usize { [0.0, 0.0, 0.0, -2.0] } else if k == MUD_SLOT as usize { [0.0, 0.0, 0.0, -4.0] } else if k == SLUDGE_SLOT as usize { [0.0, 0.0, 0.0, -5.0] } else { self.flows.get(k).map_or([0.0, 0.0, 1.0, 0.0], |f| if self.phased && k > 0 { [f[0], f[1], RIVER_CLEAR, -3.0] } else { [f[0], f[1], 1.0, 0.0] }) }),
			rn: [rn as f32, wake.n as f32, wake.cn as f32, march_steps(f.level)],
			rings,
			trail: wake.trail,
			wbox: wake.wbox,
			churn: wake.churn,
			hero: hero.map_or([0.0; 4], |h| [h[0], h[1], h[2], 1.0]),
			wave: wave_look(f.level),
			ivp,
			geo: std::array::from_fn(|k| planes.get(k).copied().unwrap_or([0.0; 4])),
			gmap: [origin[0], origin[1], SHORE_CELL, SHORE_N as f32],
			ggrid: [cols as f32, rows as f32, g.cell, 0.0],
			gridb: [-1.0 - mx, -1.0 - mb, (2.0 + 2.0 * mx) / cols as f32, (2.0 + mb + mt) / rows as f32],
			wgeo: [g.sig, g.sigq, g.amp, g.len],
			mirror2: [mirrors.get(1).copied().unwrap_or(0.0), mirrors.get(2).copied().unwrap_or(0.0), mirrors.len() as f32, 0.0],
		};
		queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));
		self.geo_draw = None;
		if !planes.is_empty() {
			let top = planes.iter().map(|p| p[0]).fold(f32::MIN, f32::max);
			queue.write_buffer(&self.shore_u, 0, bytemuck::bytes_of(&ShoreU { ivp, map: [origin[0], origin[1], SHORE_CELL, top + SHORE_CAP], size: [SHORE_STRIDE as f32, SHORE_N as f32, top - SHORE_NEAR, top + SHORE_HIGH] }));
			if self.grid_ib.as_ref().is_none_or(|g| (g.1, g.2) != (cols, rows)) {
				let vc = cols + 1;
				let idx: Vec<u32> = (0..rows).flat_map(|j| (0..cols).flat_map(move |i| {
					let a = j * vc + i;
					[a, a + 1, a + vc, a + 1, a + vc + 1, a + vc]
				})).collect();
				let ib = device.create_buffer(&wgpu::BufferDescriptor { label: Some("water sea grid"), size: std::mem::size_of_val(&idx[..]) as u64, usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
				queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
				self.grid_ib = Some((ib, cols, rows));
			}
			self.geo_draw = Some((planes.len() as u32, cols * rows * 6));
			// The flat sea where it is visible: its height per pixel (`fs_mask`).
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("water sea mask"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.seamask, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: -1e9, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_pipeline(&self.p_mask);
			pass.set_bind_group(0, &t.g_pos, &[]);
			for d in draws.iter().filter(|d| in_geo(d)) {
				pass.set_vertex_buffer(0, d.vb.slice(..));
				pass.set_index_buffer(d.ib.slice(..), wgpu::IndexFormat::Uint32);
				pass.draw_indexed(d.first..d.first + d.n, 0, 0..1);
			}
		}
		// [PC] The waterfalls (`fs_fall`): the sheets, then over their feet the foam discs (hiding where a sheet meets the
		// water), the mist and the spray.
		let mut fv = std::mem::take(&mut self.fall_verts);
		self.fall = None;
		if !fv.is_empty() {
			// Each foot on the water it falls into (retail's sheets reach on under it: the nearest flat water at or above it);
			// one deeper under another water (Pokitaru's pond: its sheets reach on down to the sea under it) is hidden.
			let planes: Vec<[f32; 5]> = draws.iter().filter_map(|d| d.plane).collect();
			let feet: Vec<[f32; 3]> = self
				.falls
				.iter()
				.filter_map(|&f| {
					let on = |b: &&[f32; 5], m: f32| f[0] >= b[1] - m && f[0] <= b[3] + m && f[1] >= b[2] - m && f[1] <= b[4] + m;
					if planes.iter().any(|b| on(&b, 0.0) && b[0] > f[2] + FALL_DEEP) {
						return None;
					}
					let z = planes.iter().filter(|b| on(b, 1.0) && b[0] >= f[2] - 0.5 && b[0] <= f[2] + FALL_DEEP).map(|b| b[0]).min_by(f32::total_cmp);
					Some([f[0], f[1], z.unwrap_or(f[2])])
				})
				.collect();
			let quad = |pos: [f32; 3], kind: f32, size: f32, seed: f32| [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].map(|uv| Vertex { pos, uv, normal: [kind, size, seed], colour: [1.0; 4], light: [0.0; 4] });
			let seed_of = |key: (i32, i32, i32)| ((key.0.wrapping_mul(73856093) ^ key.1.wrapping_mul(19349663) ^ key.2.wrapping_mul(83492791)) & 1023) as f32 / 1024.0;
			let key_of = |f: &[f32; 3], cell: f32| ((f[0] / cell).floor() as i32, (f[1] / cell).floor() as i32, (f[2] / 2.0).floor() as i32);
			let mut over = Vec::new();
			// A foam disc on every foot (seeded by its churn cell, so it holds still whatever order retail draws the sheets in).
			for f in feet.iter().take(FALL_BOILS) {
				over.extend(quad(*f, 3.0, FALL_BOIL_SIZE, seed_of(key_of(f, CHURN_CELL))));
			}
			// The feet in one FALL_PUFF_MERGE square share one set of mist and spray (a wide fall has a foot every churn cell).
			let mut spots: std::collections::BTreeMap<(i32, i32, i32), ([f32; 3], f32)> = std::collections::BTreeMap::new();
			for f in &feet {
				let e = spots.entry(key_of(f, FALL_PUFF_MERGE)).or_insert(([0.0; 3], 0.0));
				e.0 = [e.0[0] + f[0], e.0[1] + f[1], e.0[2] + f[2]];
				e.1 += 1.0;
			}
			for (key, (sum, n)) in spots.iter().take(FALL_SPOTS) {
				let f = sum.map(|v| v / n);
				let seed = seed_of(*key);
				for k in 0..FALL_PUFFS {
					over.extend(quad(f, 1.0, FALL_PUFF_SIZE, (seed + k as f32 * 0.618034).fract()));
				}
				for k in 0..FALL_DROPS {
					over.extend(quad(f, 2.0, FALL_DROP_SIZE, (seed * 3.7 + k as f32 * 0.381966).fract()));
				}
			}
			fv.append(&mut over);
			let idx: Vec<u32> = (0..fv.len() as u32 / 4).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
			let vb = device.create_buffer(&wgpu::BufferDescriptor { label: Some("water falls"), size: std::mem::size_of_val(&fv[..]) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
			queue.write_buffer(&vb, 0, bytemuck::cast_slice(&fv));
			let ib = device.create_buffer(&wgpu::BufferDescriptor { label: Some("water falls"), size: std::mem::size_of_val(&idx[..]) as u64, usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
			queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
			self.fall = Some((vb, ib, idx.len() as u32));
		}
		fv.clear();
		self.fall_verts = fv;
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("water positions"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.wpos, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
			// (Kept for the geometric sea's grid, `sea_waves`.)
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.wdepth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: if planes.is_empty() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store } }), stencil_ops: None }),
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.p_pos);
		pass.set_bind_group(0, &t.g_pos, &[]);
		// RAC_WATER_CLASS=1 (dev aid): the sea / lake class of every water draw, every 100th call.
		static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
		if std::env::var("RAC_WATER_CLASS").is_ok() && CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 100 == 50 {
			for d in draws {
				eprintln!("waterclass: kind {:?} n {} flow {} z {:?} span {:?} sea {} lake {} box {:?} eye {:?} hero {:?}", d.kind, d.n, d.flow, d.plane.map(|b| b[0]), d.plane.map(|b| (b[3] - b[1]).max(b[4] - b[2])), d.sea(), d.lake(), d.plane, f.eye, hero);
			}
		}
		// (The geometric sea is the grid's, `sea_waves`.)
		for d in draws.iter().filter(|d| !in_geo(d)) {
			pass.set_vertex_buffer(0, d.vb.slice(..));
			pass.set_index_buffer(d.ib.slice(..), wgpu::IndexFormat::Uint32);
			// The instance is the flow slot, plus FLOWS on open sea (`swell`).
			// Plus another FLOWS for the sea drawn as effect quads (`fs_ao_sea`).
			// Plus 256 on a lake (`WaterDraw::lake`).
			let k = d.flow + if d.sea() { FLOWS as u32 * if d.kind == "sea fx quads" { 2 } else { 1 } } else if d.lake() { 256 } else { 0 };
			pass.draw_indexed(d.first..d.first + d.n, 0, k..k + 1);
		}
	}

	/// This field's contact candidates for the rings (`Rings::feed`) and the wakes (`Wakes::feed`): the game's field count, the
	/// hero's feet, pool particles, and the drawn mobys (id, position).
	pub(super) fn contacts(&mut self, frame: u32, hero: Option<[f32; 3]>, parts: impl Iterator<Item = [f32; 3]>, movers: &[(u32, [f32; 3], f32)]) {
		if frame == self.wakes.frame {
			return;
		}
		// The rings look at the first 256 (v0.2.1); the churn cells at up to PARTS (a waterfall's spray comes late in the list).
		let parts: Vec<[f32; 3]> = parts.take(PARTS).collect();
		self.rings.feed(frame, hero, parts.iter().take(256).copied());
		self.wakes.feed(frame, hero, movers, &parts);
	}

	/// Before the level pass, on a level whose strips are water: the flow slot of each strip (0 = still), from the drift of
	/// its texture. Retail scrolls a river strip's uv a fixed step per game tick (FUN_002b9010, strips.rs `strip_scroll_step`);
	/// the step is measured on the strip's mean uv (frames where it does not move fall between two ticks, a jump past a third
	/// of the texture is the scroll's wrap) and turned into world units per second through the strip's uv-to-world map (a
	/// least-squares fit over its vertices). The ripples then drift the way the texture does, at its speed (only with
	/// `STRIP_FLOW`, off since 2026-10-20: the per-piece flows tore seams and jumped). The step also gives a waterfall's
	/// streak direction, so the drift is kept per strip identity, not per list index.
	pub(super) fn strip_flows(&mut self, strips: &[crate::particles::WorldStrip]) -> Vec<u32> {
		self.flows.truncate(1);
		self.phased = false;
		self.falls.clear();
		self.falls_fresh = true;
		let mut slots = Vec::with_capacity(strips.len());
		// [PC] A strip's drift is kept by what the strip is (its first vertex's x / y, its vertex count and its rank among strips
		// with the same ones: a record's two uv sets share positions), not by its place in this frame's list. The list changes
		// from frame to frame (a class not drawn this tick, strips from other callbacks), and by index a strip was compared with
		// another one's uv: garbage steps, rivers and waterfalls that turned and sped off (user 2026-10-20, Novalis).
		if self.drift.len() > 512 {
			self.drift.clear();
		}
		let mut rank: std::collections::HashMap<(u32, u32, u32), u32> = std::collections::HashMap::new();
		for s in strips {
			let at = s.pos.first().map_or([0; 2], |p| [p[0].to_bits(), p[1].to_bits()]);
			let r = rank.entry((at[0], at[1], s.pos.len() as u32)).or_insert(0);
			let d = self.drift.entry((at[0], at[1], s.pos.len() as u32, *r)).or_default();
			*r += 1;
			let n = s.uv.len().max(1) as f32;
			let mean = s.uv.iter().fold([0.0f32; 2], |a, u| [a[0] + u[0] / n, a[1] + u[1] / n]);
			let step = [mean[0] - d.mean[0], mean[1] - d.mean[1]];
			let size = step[0].abs().max(step[1].abs());
			if d.seen && size > 1e-6 && size < 0.3 {
				d.step = if d.step == [0.0; 2] { step } else { [d.step[0] * 0.8 + step[0] * 0.2, d.step[1] * 0.8 + step[1] * 0.2] };
			}
			d.mean = mean;
			d.seen = true;
			// [PC] Steep triangles are a waterfall (Novalis: the river's drop into the lower pool, z 60 -> 39): they are drawn
			// again with falling streaks (`fs_fall`) along the way retail scrolls them, and the foot of the steepest churns the
			// water (`Wakes`).
			let along_u = d.step[0].abs() > d.step[1].abs();
			let sign = if along_u { d.step[0] } else { d.step[1] }.signum();
			for k in 2..s.pos.len().min(s.uv.len()) {
				let t = [s.pos[k - 2], s.pos[k - 1], s.pos[k], s.pos[k]];
				let (a, b) = ([t[1][0] - t[0][0], t[1][1] - t[0][1], t[1][2] - t[0][2]], [t[2][0] - t[0][0], t[2][1] - t[0][1], t[2][2] - t[0][2]]);
				let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
				let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
				if nl >= 1e-9 && n[2].abs() < FALL_STEEP * nl {
					if n[2].abs() < 0.5 * nl {
						push_foot(&mut self.falls, &t);
					}
					let uv = [s.uv[k - 2], s.uv[k - 1], s.uv[k], s.uv[k]];
					self.fall_verts.extend((0..4).map(|j| Vertex { pos: t[j], uv: uv[j], normal: [0.0, sign, along_u as u32 as f32], colour: [0.5, 0.5, 0.5, RIVER_FALL_ALPHA], light: [0.0; 4] }));
				}
			}
			// [PC] No ripple flow on strips (STRIP_FLOW): each piece got its own flow, so neighbouring pieces drifted their ripples
			// apart (seams), and a piece's flow jumped in direction and speed with the measured step.
			let flow = STRIP_FLOW.then(|| uv_map(&s.pos, &s.uv)).flatten().map(|inv| {
				// A texture feature at uv u sits at x = inv (u - c): it moves by -inv (uv step) per tick, 50 ticks a second.
				let v = [-(inv[0][0] * d.step[0] + inv[0][1] * d.step[1]) * 50.0, -(inv[1][0] * d.step[0] + inv[1][1] * d.step[1]) * 50.0];
				let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
				if l > 6.0 { [v[0] * 6.0 / l, v[1] * 6.0 / l] } else { v }
			});
			match flow.filter(|v| v[0].abs() + v[1].abs() > 1e-3 && self.flows.len() < LAST_FLOW as usize) {
				Some(v) => {
					slots.push(self.flows.len() as u32);
					self.flows.push(v);
				}
				None => slots.push(0),
			}
		}
		keep_lowest(&mut self.falls);
		slots
	}

	/// Before the level pass, on a level with a river of effect quads (`river_fx_tex`): the river quads grouped by flow slot
	/// (slot, indices into `quads`). The texture scrolls the same uv step per tick on every quad, so the step is the median uv
	/// change of the quads drawn at the same place last frame (no change: between two ticks; a change past 0.3 is the wrap),
	/// smoothed as `strip_flows` does. Each quad's flow is then that step through its own uv-to-world map, rounded to
	/// `RIVER_DIRS` directions and `RIVER_SPEED_STEP` speeds so the quads share a few slots. Steep quads (the waterfall, normal
	/// z under 0.5) are left out and keep their retail look.
	pub(super) fn quad_flows(&mut self, quads: &[([[f32; 3]; 4], [[f32; 2]; 4])]) -> Vec<(u32, Vec<usize>)> {
		self.flows.truncate(1);
		self.phased = true;
		self.falls.clear();
		self.falls_fresh = true;
		let key = |p: [f32; 3]| [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
		let (mut du, mut dv) = (Vec::new(), Vec::new());
		for (pos, uv) in quads {
			if let Some(o) = self.river_uv.get(&key(pos[0])) {
				let d = [uv[0][0] - o[0], uv[0][1] - o[1]];
				if d[0].abs() < 0.3 && d[1].abs() < 0.3 {
					du.push(d[0]);
					dv.push(d[1]);
				}
			}
		}
		if !du.is_empty() {
			du.sort_by(f32::total_cmp);
			dv.sort_by(f32::total_cmp);
			let step = [du[du.len() / 2], dv[dv.len() / 2]];
			if step[0].abs().max(step[1].abs()) > 1e-6 {
				let d = self.river_step;
				self.river_step = if d == [0.0; 2] { step } else { [d[0] * 0.8 + step[0] * 0.2, d[1] * 0.8 + step[1] * 0.2] };
			}
		}
		self.river_uv.clear();
		self.river_uv.extend(quads.iter().map(|(pos, uv)| (key(pos[0]), uv[0])));
		let st = self.river_step;
		let mut keys: std::collections::BTreeMap<(i32, i32), u32> = std::collections::BTreeMap::new();
		let mut groups: Vec<(u32, Vec<usize>)> = Vec::new();
		for (i, (pos, uv)) in quads.iter().enumerate() {
			let (a, b) = ([pos[1][0] - pos[0][0], pos[1][1] - pos[0][1], pos[1][2] - pos[0][2]], [pos[3][0] - pos[0][0], pos[3][1] - pos[0][1], pos[3][2] - pos[0][2]]);
			let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
			let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
			if nl < 1e-9 || n[2].abs() < 0.5 * nl {
				// [PC] A steep quad's bottom edge is a waterfall's foot (`Wakes` churn), one per CHURN_CELL, and the quad is drawn
				// again with falling streaks (`fs_fall`) along the way retail scrolls it.
				if nl >= 1e-9 {
					push_foot(&mut self.falls, pos);
					let along_u = st[0].abs() > st[1].abs();
					let sign = if along_u { st[0] } else { st[1] }.signum();
					self.fall_verts.extend((0..4).map(|k| Vertex { pos: pos[k], uv: uv[k], normal: [0.0, sign, along_u as u32 as f32], colour: [0.5, 0.5, 0.5, RIVER_FALL_ALPHA], light: [0.0; 4] }));
				}
				continue;
			}
			let flow = uv_map(&pos[..], &uv[..]).map(|inv| {
				let v = [-(inv[0][0] * st[0] + inv[0][1] * st[1]) * 50.0, -(inv[1][0] * st[0] + inv[1][1] * st[1]) * 50.0];
				let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
				if l > 6.0 { [v[0] * 6.0 / l, v[1] * 6.0 / l] } else { v }
			});
			let slot = match flow.map(|v| (v[1].atan2(v[0]), (v[0] * v[0] + v[1] * v[1]).sqrt())).filter(|f| f.1 >= RIVER_SPEED_STEP * 0.5) {
				Some((ang, sp)) => {
					let k = ((ang / std::f32::consts::TAU * RIVER_DIRS).round() as i32).rem_euclid(RIVER_DIRS as i32);
					let q = (sp / RIVER_SPEED_STEP).round() as i32;
					match keys.get(&(k, q)) {
						Some(&s) => s,
						None if self.flows.len() < LAST_FLOW as usize => {
							let (ang, sp) = (k as f32 * std::f32::consts::TAU / RIVER_DIRS, q as f32 * RIVER_SPEED_STEP);
							let s = self.flows.len() as u32;
							self.flows.push([ang.cos() * sp, ang.sin() * sp]);
							keys.insert((k, q), s);
							s
						}
						None => 0,
					}
				}
				None => 0,
			};
			match groups.iter_mut().find(|g| g.0 == slot) {
				Some(g) => g.1.push(i),
				None => groups.push((slot, vec![i])),
			}
		}
		keep_lowest(&mut self.falls);
		groups
	}

	/// [PC] Waterfall sheets drawn as effect quads (`fall_fx_tex`, Pokitaru): the foot of each steep quad churns the water it
	/// falls into, as a river's waterfall does (`quad_flows`), and the sheets are drawn as falling water (`fs_fall`) instead of
	/// retail's own draw (gpu.rs leaves them out of the level pass).
	/// After `quad_flows`, whose feet it keeps. Each quad: its corners' position, uv and colour.
	/// `refill`: a widescreen refill candidate (retail/efx_draw.rs `FX_REFILL`), drawn only as a known sheet triangle (`SheetMem::refill`).
	pub(super) fn sheet_falls(&mut self, level: usize, quads: impl Iterator<Item = ([[f32; 3]; 4], [[f32; 2]; 4], [[f32; 4]; 4], bool)>) {
		// The sheets seen so far belong to one level: another level (or the first visit of this run's level) starts empty.
		if self.sheet_level != level {
			self.sheet_level = level;
			self.sheet_mem = SheetMem::default();
		}
		if !self.falls_fresh {
			self.falls.clear();
		}
		self.falls_fresh = true;
		let start = self.fall_verts.len();
		// RAC_FALL_LOG=path (dev): appends this frame's sheet quads (positions, uv, colour) for flicker analysis.
		let (refill, quads): (Vec<_>, Vec<_>) = quads.partition(|q| q.3);
		let quads: Vec<_> = quads.into_iter().map(|(p, u, c, _)| (p, u, c)).collect();
		static OLD: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
		// RAC_FALL_OLD=1 (dev): the per-frame shape of before, for A/B pictures.
		let old = *OLD.get_or_init(|| std::env::var("RAC_FALL_OLD").is_ok());
		static LOG: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
		let log = LOG.get_or_init(|| std::env::var("RAC_FALL_LOG").ok());
		if let Some(path) = log {
			use std::io::Write;
			static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
			let n = FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
			if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
				let _ = writeln!(f, "FRAME {n} quads {}", quads.len());
				for (pos, uv, colour) in &quads {
					let _ = writeln!(f, "{:?} {:?} {:?}", pos, uv, colour);
				}
			}
		}
		for (pos, uv, colour) in quads {
			// Retail scrolls their v up 0.01 a tick [C: quad dump at fullgame1 0447, 2026-10-17].
			// n.y = 2: `fs_fall` builds the streaks from retail's texture.
			self.fall_verts.extend((0..4).map(|k| Vertex { pos: pos[k], uv: uv[k], normal: [0.0, 2.0, 0.0], colour: colour[k], light: [0.0; 4] }));
			if old {
				let (a, b) = ([pos[1][0] - pos[0][0], pos[1][1] - pos[0][1], pos[1][2] - pos[0][2]], [pos[3][0] - pos[0][0], pos[3][1] - pos[0][1], pos[3][2] - pos[0][2]]);
				let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
				let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
				if nl >= 1e-9 && n[2].abs() < 0.5 * nl {
					push_foot(&mut self.falls, &pos);
				}
			}
		}
		if old {
			keep_lowest(&mut self.falls);
			shape_sheets(&mut self.fall_verts[start..]);
		} else {
			// The shape and the feet come from every triangle seen so far, not from the ones retail happened to draw this frame
			// (it leaves some out with the camera: their neighbours' steepness and brinks, and so the look, popped from frame to frame).
			let drawn = self.sheet_mem.observe(&mut self.fall_verts[start..]);
			let cand: Vec<Vertex> = refill.iter().flat_map(|(pos, uv, colour, _)| (0..4).map(move |k| Vertex { pos: pos[k], uv: uv[k], normal: [0.0, 2.0, 0.0], colour: colour[k], light: [0.0; 4] })).collect();
			let more = self.sheet_mem.refill(&drawn, &cand);
			self.fall_verts.extend(more);
			for f in &self.sheet_mem.feet {
				if !self.falls.iter().any(|g| (g[0] - f[0]).abs() < CHURN_CELL && (g[1] - f[1]).abs() < CHURN_CELL && (g[2] - f[2]).abs() < 1.0) {
					self.falls.push(*f);
				}
			}
			keep_lowest(&mut self.falls);
		}
		if let Some(path) = log {
			use std::io::Write;
			if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
				for v in &self.fall_verts[start..] {
					let _ = writeln!(f, "SHAPE {:?} {:?}", v.pos, v.light);
				}
			}
		}
	}

	/// [PC] Before `strip_flows` / `quad_flows` / `sheet_falls`: drops waterfall sheets a frame without a water pass left.
	pub(super) fn clear_falls(&mut self) {
		self.fall_verts.clear();
	}

	/// [PC] Geometric sea: this frame's props (`Gpu::set_props`). A moby that moved or turned within SHORE_STILL frames is a mover:
	/// the bed draws it after the shore map (mirror.rs `draw_bed`), so a fish or a boat in the sea makes no surf of its own
	/// (its wake marks it; its bobbing made the surf grow and vanish frame to frame) while a moby that keeps still (a post, a
	/// hull, a crate) keeps its surf.
	pub(super) fn track_props(&mut self, props: &[crate::game::Prop]) {
		self.shore_movers.clear();
		self.shore_still.resize(props.len(), ([f32::NAN; 6], 0));
		for (pi, p) in props.iter().enumerate() {
			let now = [p.pos[0], p.pos[1], p.pos[2], p.rot[0], p.rot[1], p.rot[2]];
			let e = &mut self.shore_still[pi];
			e.1 = if now.iter().zip(&e.0).all(|(a, b)| (a - b).abs() <= SHORE_MOVED) { e.1.saturating_add(1) } else { 0 };
			e.0 = now;
			if p.shown && e.1 < SHORE_STILL {
				self.shore_movers.push(pi);
			}
		}
	}

	/// [PC] Geometric sea: the prop slots the bed draws after the shore map (`track_props`); none without sea waves.
	pub(super) fn shore_movers(&self) -> &[usize] {
		if self.geo_draw.is_some() && crate::devtune::f32("SHORE_MOVERS", 1.0) > 0.5 { &self.shore_movers } else { &[] }
	}

	/// [PC] This frame has waterfall sheets to draw (the water pass runs for them even with no water in view).
	pub(super) fn has_falls(&self) -> bool {
		!self.fall_verts.is_empty()
	}

	/// The targets (MSAA colour, resolve layer `k`, MSAA depth) of mirror pass `k` for `Gpu::draw_mirror`; after `positions`.
	pub(super) fn mirror_target(&self, k: usize) -> Option<(&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView)> {
		self.t.as_ref().and_then(|t| Some((&t.mirror_ms, t.mirror_layers.get(k)?, &t.mirror_depth)))
	}

	/// [PC] Geometric sea waves, after `positions` and `Gpu::draw_bed`: the shore map from the bed's depth, then the projected
	/// grid of each sea plane into the position target (nearest surface wins against the other water).
	pub(super) fn sea_waves(&self, enc: &mut wgpu::CommandEncoder) {
		let (Some(t), Some((planes, n)), Some((ib, ..))) = (&self.t, self.geo_draw, &self.grid_ib) else { return };
		enc.clear_buffer(&self.shore, 0, None);
		{
			let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("water shore map"), timestamp_writes: None });
			cp.set_pipeline(&self.p_shore);
			cp.set_bind_group(0, &t.g_shore, &[]);
			let (bw, bh) = ((t.key.0 / 2).max(1).div_ceil(SHORE_STRIDE), (t.key.1 / 2).max(1).div_ceil(SHORE_STRIDE));
			cp.dispatch_workgroups(bw.div_ceil(8), bh.div_ceil(8), 1);
			// [PC] The distance map (`cs_shore_h`, `cs_shore_v`).
			for p in [&self.p_shore_h, &self.p_shore_v] {
				cp.set_pipeline(p);
				cp.set_bind_group(0, &t.g_shore, &[]);
				cp.dispatch_workgroups(SHORE_N.div_ceil(8), SHORE_N.div_ceil(8), 1);
			}
		}
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("water sea grid"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.wpos, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.wdepth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard }), stencil_ops: None }),
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.p_grid);
		pass.set_bind_group(0, &t.g_grid, &[]);
		pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
		pass.draw_indexed(0..n, 0, 0..planes);
	}

	/// The water bed's targets (MSAA colour, resolve, MSAA depth) for `Gpu::draw_bed`; after `positions`.
	pub(super) fn bed_target(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView)> {
		self.t.as_ref().map(|t| (&t.bed_ms, &t.bed, &t.bed_depth))
	}

	/// [PC] Global illumination (post.rs `bind_water`): the position target and the scene copy `shade` takes before it draws.
	pub(super) fn gi_mask(&self) -> Option<(&wgpu::TextureView, &wgpu::Texture)> {
		self.t.as_ref().map(|t| (&t.wpos, &t.copy))
	}

	/// Inside the post chain (after the occlusion, before the bloom): shades the water pixels of the post scene `scene`;
	/// `fall_tex`: the bind group of the level's waterfall texture (`fall_fx_tex`), if any.
	pub(super) fn shade(&self, enc: &mut wgpu::CommandEncoder, scene: &wgpu::TextureView, ao: Option<&wgpu::TextureView>, fall_tex: Option<&wgpu::BindGroup>) {
		let Some(t) = &self.t else { return };
		// [PC] No screen-space occlusion on the far open sea (`fs_ao_sea`). RAC_SEA_AO=1 (dev aid): keep it (A/B).
		static KEEP: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
		if let (Some(ao), false) = (ao, *KEEP.get_or_init(|| std::env::var("RAC_SEA_AO").is_ok_and(|v| v == "1"))) {
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("water sea occlusion"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: ao, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: None,
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_pipeline(&self.p_ao_sea);
			pass.set_bind_group(0, &t.g_shade, &[]);
			pass.draw(0..3, 0..1);
		}
		enc.copy_texture_to_texture(scene.texture().as_image_copy(), t.copy.as_image_copy(), t.copy.size());
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("water shade"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: scene, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: None,
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.p_shade);
		pass.set_bind_group(0, &t.g_shade, &[]);
		pass.draw(0..3, 0..1);
		if let Some((vb, ib, n)) = &self.fall {
			pass.set_pipeline(&self.p_fall);
			pass.set_bind_group(1, fall_tex.unwrap_or(&self.fall_dummy), &[]);
			pass.set_vertex_buffer(0, vb.slice(..));
			pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
			pass.draw_indexed(0..*n, 0, 0..1);
		}
	}
}

/// The world-from-uv map of a strip: the least-squares fit uv = A (x, y) + c over its vertices, inverted (rows of A^-1); none
/// when the fit is degenerate.
fn uv_map(pos: &[[f32; 3]], uv: &[[f32; 2]]) -> Option<[[f32; 2]; 2]> {
	let n = pos.len().min(uv.len());
	if n < 3 {
		return None;
	}
	let k = n as f64;
	let (mx, my) = (pos[..n].iter().map(|p| p[0] as f64).sum::<f64>() / k, pos[..n].iter().map(|p| p[1] as f64).sum::<f64>() / k);
	let (mu, mv) = (uv[..n].iter().map(|u| u[0] as f64).sum::<f64>() / k, uv[..n].iter().map(|u| u[1] as f64).sum::<f64>() / k);
	let (mut sxx, mut sxy, mut syy, mut sux, mut suy, mut svx, mut svy) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
	for (p, u) in pos[..n].iter().zip(&uv[..n]) {
		let (x, y, a, b) = (p[0] as f64 - mx, p[1] as f64 - my, u[0] as f64 - mu, u[1] as f64 - mv);
		sxx += x * x;
		sxy += x * y;
		syy += y * y;
		sux += a * x;
		suy += a * y;
		svx += b * x;
		svy += b * y;
	}
	let det = sxx * syy - sxy * sxy;
	if det.abs() < 1e-9 {
		return None;
	}
	// A = S_uv,xy S_xy,xy^-1 (2x2).
	let ixx = [[syy / det, -sxy / det], [-sxy / det, sxx / det]];
	let a = [[sux * ixx[0][0] + suy * ixx[1][0], sux * ixx[0][1] + suy * ixx[1][1]], [svx * ixx[0][0] + svy * ixx[1][0], svx * ixx[0][1] + svy * ixx[1][1]]];
	let da = a[0][0] * a[1][1] - a[0][1] * a[1][0];
	if da.abs() < 1e-12 {
		return None;
	}
	Some([[(a[1][1] / da) as f32, (-a[0][1] / da) as f32], [(-a[1][0] / da) as f32, (a[0][0] / da) as f32]])
}

#[cfg(test)]
mod tests {
	/// A strip whose uv is (x / 4, y / 8): the world-from-uv map is diag(4, 8).
	#[test]
	fn uv_map_inverts_the_fit() {
		let pos = [[0.0, 0.0, 0.0], [4.0, 0.0, 0.0], [0.0, 8.0, 0.0], [4.0, 8.0, 1.0]];
		let uv = [[0.0, 0.0], [1.0, 0.0], [0.0, 1.0], [1.0, 1.0]];
		let m = super::uv_map(&pos, &uv).unwrap();
		assert!((m[0][0] - 4.0).abs() < 1e-4 && (m[1][1] - 8.0).abs() < 1e-4 && m[0][1].abs() < 1e-4 && m[1][0].abs() < 1e-4);
	}

	/// Rings: the hero in a pool leaves one at the surface height, a particle far from any water none, a paused game
	/// (same field) nothing new, and spent rings go.
	#[test]
	fn rings_follow_contacts_on_still_water() {
		let pool = [2.0, 0.0, 0.0, 10.0, 10.0];
		let mut r = super::Rings::default();
		r.feed(1, Some([5.0, 5.0, 1.5]), [[5.0, 5.0, 9.0], [7.0, 7.0, 2.2]].into_iter());
		let (rows, n) = r.update(10.0, [pool].into_iter());
		assert_eq!(n, 2);
		assert_eq!(&rows[0][..3], &[5.0, 5.0, 2.0]);
		assert!(rows[1][0] > 0.0 && rows[1][2] > 0.0);
		assert_eq!(&rows[2][..3], &[7.0, 7.0, 2.0]);
		r.feed(1, Some([6.0, 5.0, 1.5]), std::iter::empty());
		assert_eq!(r.update(10.5, [pool].into_iter()).1, 2);
		r.feed(2, None, std::iter::empty());
		assert_eq!(r.update(13.0, [pool].into_iter()).1, 0);
	}

	/// The shader compiles (naga) with both entry pairs.
	#[test]
	fn water_shader_parses() {
		let m = naga::front::wgsl::parse_str(super::SHADER).expect("water shader");
		let names: Vec<_> = m.entry_points.iter().map(|e| e.name.as_str()).collect();
		for n in ["vs_pos", "fs_pos", "vs_full", "fs_water"] {
			assert!(names.contains(&n), "{n}");
		}
		let mut v = naga::valid::Validator::new(naga::valid::ValidationFlags::all(), naga::valid::Capabilities::all());
		v.validate(&m).expect("water shader validates");
	}
}
