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
//! Gaspar's RAM ocean is lava: no water pass there (`LAVA_LEVEL`).

use super::*;

const SHADER: &str = r#"
struct WU { vp: mat4x4<f32>, eye: vec4<f32>, sun: vec4<f32>, proj: vec4<f32>, size: vec4<f32>, look: vec4<f32>, fog: vec4<f32>, mirror: vec4<f32>, flows: array<vec4<f32>, 64>, rn: vec4<f32>, rings: array<vec4<f32>, 32> };
@group(0) @binding(0) var<uniform> w: WU;
@group(0) @binding(1) var depth_ms: texture_depth_multisampled_2d;
@group(0) @binding(2) var scene: texture_2d<f32>;
@group(0) @binding(3) var lin: texture_2d<f32>;
@group(0) @binding(4) var wpos: texture_2d<f32>;
@group(0) @binding(5) var lsamp: sampler;
@group(0) @binding(6) var mirror: texture_2d<f32>;
// The particle pass's glow target (partfx.rs): its alpha is the particles' cover (w.mirror.z = 1 with Particles Enhanced).
@group(0) @binding(7) var glow: texture_2d<f32>;
// The water bed (mirror.rs `draw_bed`; half size): the level without its water, and its depth.
@group(0) @binding(8) var bed_tex: texture_2d<f32>;
@group(0) @binding(9) var bed_depth: texture_depth_multisampled_2d;

// Clear shallows: absorption per world unit of path through the water (`fs_water`).
const ABSORB: f32 = 0.12;

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
		if (abs(p.z - a.z) > 0.3 || abs(x) > 3.0 || d < 1e-4) { continue; }
		let env = exp(-x * x);
		let sl = b.x * (cos(2.4 * x) - 0.83 * x * sin(2.4 * x)) * env * clamp(b.y / (fp * 2.0), 0.0, 1.0);
		s += vec3<f32>(dv / d * sl, b.z * exp(-x * x * 4.0));
	}
	return s;
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
	for (var i = 0; i < 32; i++) {
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
			return vec4<f32>(hit.xy, b, edge_fade(hit.xy) * (1.0 - smoothstep(0.55, 1.0, f32(i) / 32.0)));
		}
		prev = t;
		t = t * 1.3 + 0.05;
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

// w.eye = (eye, seconds); w.sun = (to the key light, glint strength); w.look = (ripples, refraction in world units, minimum
// reflectance, foam); w.fog = (fog colour in linear light, reflection strength).
@fragment fn fs_water(v: VO) -> @location(0) vec4<f32> {
	let c = vec2<i32>(v.pos.xy);
	let wp = wpos_at(c);
	if (wp.a < 0.5) { discard; }
	let p = wp.xyz;
	let eye = w.eye.xyz;
	// Underwater (the camera below this surface): the retail picture.
	if (eye.z < p.z - 0.02) { discard; }
	let te = eye - p;
	let dist = max(length(te), 1e-3);
	let view = te / dist;
	let time = w.eye.w;
	let jit = dither(v.pos.xy);
	let uv0 = (vec2<f32>(c) + 0.5) * w.size.zw;
	let orig = to_linear(textureSampleLevel(scene, lsamp, uv0, 0.0).rgb);
	// World size of one pixel here (wider on a grazing view of the surface).
	let fp = dist * 2.0 * w.proj.y * w.size.w / max(view.z, 0.2);
	// Flowing water (a river's scrolling texture, `WaterFx::strip_flows`): the ripples and caustics drift downstream with it.
	// w.flows[slot] = (flow x, flow y, clear (0 = water retail draws opaque: no clear shallows), 0).
	let fl = w.flows[min(u32(wp.a - 0.5), 63u)];
	let flow = fl.xy;
	let q = p.xy - flow * time;
	let rg = rings_at(p, fp);
	let rs = ripple(q, time, fp) * w.look.x + rg.xy;
	let g = geo_normal(c, p, dist);
	let n = normalize(g + vec3<f32>(-rs.x, -rs.y, 0.0));
	let ndv = clamp(dot(n, view), 0.0, 1.0);
	let fresnel = max(0.02 + 0.98 * pow(1.0 - ndv, 5.0), w.look.z);
	let sun_on = clamp(w.sun.w / 2.2, 0.0, 1.0);

	// What lies under the surface: the level's depth behind the water pixel (blended water) gives the bed and its depth.
	let zw = project(p).z;
	let zs = scene_z(uv0);
	let blended = zs > zw + 0.05 && zs < w.proj.w * 0.98;
	let bed = eye + (p - eye) * (zs / zw);
	let thick = select(0.0, clamp(p.z - bed.z, 0.0, 40.0), blended);

	// Refraction: the picture under the surface, shifted by the ripple slope (more through deeper water); only from water.
	let push = w.look.y * (0.5 + min(thick, 4.0) * 0.25);
	var uv = project(p + vec3<f32>(rs * push, 0.0)).xy;
	if (wpos_at(vec2<i32>(uv * w.size.xy)).a < 0.5) {
		uv = mix(uv0, uv, 0.4);
		if (wpos_at(vec2<i32>(uv * w.size.xy)).a < 0.5) { uv = uv0; }
	}
	var base = to_linear(textureSampleLevel(scene, lsamp, uv, 0.0).rgb);

	// Caustics: moving light lines on the bed (on the surface itself where the water hides its bed), brightening the colour
	// already there so the water keeps its tint; strongest in sunlight and shallow water.
	let cp = select(q + rs * 0.6, bed.xy - flow * time + rs * (0.6 + thick * 0.3), blended);
	let cfp = select(fp, fp * zs / zw, blended);
	let cstr = (0.25 + 0.75 * sun_on) * select(0.18, 0.8 * exp(-thick * 0.22), blended);
	base += base * (caustic(cp, time, cfp) - 0.3) * cstr;
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
		buv = uv0;
		zb = linear_depth(textureLoad(bed_depth, clamp(vec2<i32>(buv * vec2<f32>(bd)), vec2<i32>(0), bd - vec2<i32>(1)), 0));
	}
	if (zb > zw + 0.02 && zb < w.proj.w * 0.98 && w.mirror.w > 0.0) {
		let bp = eye + (p - eye) * (zb / zw);
		let path = (zb / zw - 1.0) * dist;
		let hue = base / max(max(base.r, max(base.g, base.b)), 1e-3);
		var shallow = to_linear(textureSampleLevel(bed_tex, lsamp, buv, 0.0).rgb) * mix(vec3<f32>(1.0), hue, 0.6);
		let bt = clamp(p.z - bp.z, 0.0, 40.0);
		shallow += shallow * (caustic(bp.xy - flow * time + rs * (0.6 + bt * 0.3), time, fp * zb / zw) - 0.3) * (0.25 + 0.75 * sun_on) * 0.9 * exp(-bt * 0.2);
		base = mix(base, shallow, exp(-path * ABSORB) * w.mirror.w * fl.z);
	}

	// Reflection: the ray follows the smooth surface (so neighbouring pixels agree), the ripples then bend where it is read
	// and the full ripple normal picks the far air; softened more the further the reflected thing is from the water.
	var rr = reflect(-view, normalize(g + vec3<f32>(-rs.x, -rs.y, 0.0) * 0.15));
	rr = normalize(vec3<f32>(rr.xy, max(rr.z, 0.04)));
	var r = reflect(-view, n);
	r = normalize(vec3<f32>(r.xy, max(r.z, 0.04)));
	let hit = reflect_ray(p + g * 0.02, rr, jit);
	var refl = env(r, base);
	// How sure the reflection is (a mirror image or a screen-space hit): sure reflections may mirror more at grazing views.
	var sure = hit.w;
	if (hit.w > 0.0) {
		let bend = project(p + vec3<f32>(-rs * 0.5 * min(hit.z, 4.0), 0.0)).xy - project(p).xy;
		let rad = 0.002 + 0.014 * hit.z / (hit.z + dist);
		let huv = clamp(hit.xy + bend, vec2<f32>(0.0), vec2<f32>(1.0));
		refl = mix(refl, soft_scene(huv, rad, jit), hit.w);
	}
	// Flat water on the mirrored plane (mirror.rs; w.mirror = (height, on)): the true mirror image at this pixel, bent by the
	// ripples and softened by the pixel's footprint, in place of the screen-space reflection.
	if (w.mirror.y > 0.5 && abs(p.z - w.mirror.x) < 0.4 && g.z > 0.96) {
		let muv = uv0 + project(p + vec3<f32>(-rs * 0.25, 0.0)).xy - project(p).xy;
		let mr = (0.5 + min(fp * 4.0, 2.0)) * w.size.zw * 2.0;
		let a = jit * 6.2832;
		let o = vec2<f32>(cos(a), sin(a));
		var m = vec3<f32>(0.0);
		for (var k = 0; k < 4; k++) {
			let ok = select(o, vec2<f32>(-o.y, o.x), (k & 1) == 1) * select(1.0, -1.0, k >= 2);
			m += to_linear(textureSampleLevel(mirror, lsamp, clamp(muv + ok * mr, vec2<f32>(0.0), vec2<f32>(1.0)), 0.0).rgb);
		}
		refl = m * 0.25;
		sure = 1.0;
	}
	var col = mix(base, refl, clamp(fresnel * w.fog.w, 0.0, mix(0.65, 0.75, sure)));
	// Keep the water's own hue (a reflection of grey walls or fog must not grey a teal pool); less so for a sure reflection.
	let mx = max(base.r, max(base.g, base.b));
	let sat = (mx - min(base.r, min(base.g, base.b))) / max(mx, 1e-4);
	col = mix(col, base * (luma(col) / max(luma(base), 1e-4)), 0.4 * sat * (1.0 - 0.3 * sure));

	// Sun glint and sparkles along the key light (none with the light below the horizon: w.sun.w = 0): a sharp highlight
	// that softens with distance instead of flickering.
	let l = w.sun.xyz;
	let hv = normalize(l + view);
	let sn = normalize(n + vec3<f32>(-sparkle_slope(q, time, fp), 0.0));
	let e = 900.0 / (1.0 + fp * 60.0);
	let twinkle = 0.25 + 0.75 * smoothstep(0.7, 0.95, vnoise(q * 5.0 + vec2<f32>(time * 1.3, -time * 0.9)));
	col += vec3<f32>(1.0, 0.97, 0.9) * pow(max(dot(sn, hv), 0.0), e) * (e + 8.0) * 0.0004 * twinkle * w.sun.w;

	// Foam where the water meets the ground.
	var foam = 0.0;
	if (blended) {
		// Blended water over the ground: shallow near the shore (the water's own depth there, so a pool wall seen at a grazing
		// angle keeps a thin line instead of a broad band).
		foam = 1.0 - smoothstep(0.0, 0.3, thick);
	} else {
		// The water wrote its own depth: geometry cutting the surface at the same depth marks the shore.
		let px = clamp(0.18 / (zw * 2.0 * w.proj.y * w.size.w), 1.5, 9.0);
		var hits = 0.0;
		for (var k = 0; k < 8; k++) {
			let a = (f32(k) + jit) * 0.785398;
			let q = c + vec2<i32>(vec2<f32>(cos(a), sin(a)) * px);
			if (wpos_at(q).a < 0.5) {
				let zq = scene_z((vec2<f32>(q) + 0.5) * w.size.zw);
				if (abs(zq - zw) < 0.06 * zw + 0.25) { hits += 1.0; }
			}
		}
		foam = clamp(hits / 3.0, 0.0, 1.0);
	}
	let breakup = vnoise(p.xy * 2.7 + vec2<f32>(time * 0.35, time * -0.2)) * 0.6 + vnoise(p.xy * 6.1 - vec2<f32>(time * 0.5, 0.0)) * 0.4;
	// Soft and in the water's own colour: a lighter froth, not a white rim.
	foam = smoothstep(0.45, 0.95, foam * (0.3 + 1.1 * breakup)) * w.look.w * 0.55;
	col = mix(col, min(base * 1.5 + vec3<f32>(0.05), vec3<f32>(0.7)), foam);
	// The rings' froth crest: whiter than the shore foam while young, broken up the same way.
	col = mix(col, min(base * 1.8 + vec3<f32>(0.12), vec3<f32>(0.85)), clamp(rg.z * (0.3 + 0.9 * breakup), 0.0, 0.8));

	// Soft mask edge: a pixel with little water around blends back towards the retail picture.
	var around = 0.0;
	let ring = array<vec2<i32>, 8>(vec2<i32>(-1, -1), vec2<i32>(0, -1), vec2<i32>(1, -1), vec2<i32>(-1, 0), vec2<i32>(1, 0), vec2<i32>(-1, 1), vec2<i32>(0, 1), vec2<i32>(1, 1));
	for (var k = 0; k < 8; k++) {
		if (wpos_at(c + ring[k]).a > 0.5) { around += 1.0; }
	}
	col = mix(orig, col, smoothstep(2.0, 7.0, around));
	// Particles over the water (a blast, smoke, splashes) stay as drawn: the shade goes back to the picture under their cover.
	col = mix(col, orig, clamp(textureLoad(glow, c, 0).a * w.mirror.z * 1.5, 0.0, 1.0));
	return vec4<f32>(pow(clamp(col, vec3<f32>(0.0), vec3<f32>(1.0)), vec3<f32>(1.0 / 2.2)), 1.0);
}
"#;

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

/// Gaspar (its "ocean" is lava).
pub(super) const LAVA_LEVEL: usize = 9;

/// The fx textures (level fx table index, `WorldQuad.tex`) of a sea drawn as effect quads: Pokitaru's sea is queue-A
/// callback 0x30a238 output (decomp/notes/ocean.md); at fg1/11_0449 it is fx texture 22 (15 fans out to the horizon) under
/// the additive layers 40 and 41 (flat at z 223.16 around the beach) [C: RAC_WATER_LOG bounds and RAC_DRAWSKIP=quads, 2026-10-15].
/// The queue-C pool strips (`Gpu.pool_draws`) that are water, by fx texture: Hoven's abyss water (0x30b798: tex 0x2c and its
/// FIX 64 second layer 0x2d). Its tex 47 / 48 strips are the snow, Orxon's tex 41 (FIX 32) and Batalia's tex 59 (additive,
/// FIX 0) are no water surfaces.
pub(super) fn pool_water_tex(level: usize) -> &'static [usize] {
	match level {
		12 => &[44, 45],
		_ => &[],
	}
}

pub(super) fn sea_fx_tex(level: usize) -> &'static [usize] {
	match level {
		11 => &[22, 40, 41],
		_ => &[],
	}
}

const WPOS_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;

/// One water draw of the level pass.
pub(super) struct WaterDraw {
	/// Vertex buffer (`Vertex` layout), index buffer (u32), index count.
	pub vb: wgpu::Buffer,
	pub ib: wgpu::Buffer,
	pub n: u32,
	/// Its flat plane, if any (`mirror::plane_of`).
	pub plane: super::mirror::Plane,
	/// Its slot in this frame's flow table (`WaterFx::strip_flows`), 0 = still water.
	pub flow: u32,
	/// What drew it (the water log, RAC_WATER_LOG).
	pub kind: &'static str,
}

impl WaterDraw {
	pub(super) fn new(vb: wgpu::Buffer, ib: wgpu::Buffer, n: u32, pos: impl Iterator<Item = [f32; 3]> + Clone) -> WaterDraw {
		WaterDraw { vb, ib, n, plane: super::mirror::plane_of(pos), flow: 0, kind: "" }
	}

	pub(super) fn kind(mut self, kind: &'static str) -> WaterDraw {
		self.kind = kind;
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
		self.live.retain(|r| now - r.born < r.life && now >= r.born);
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
				self.push(Ring { p: c, born: now, slope, froth, life: 1.8 }, true);
			} else {
				let near = |r: &Ring| (r.p[0] - c[0]).powi(2) + (r.p[1] - c[1]).powi(2) < PART_GAP.0 * PART_GAP.0 && now - r.born < PART_GAP.1;
				if !self.live.iter().any(near) {
					self.push(Ring { p: c, born: now, slope: 0.4, froth: 0.6, life: 1.2 }, false);
				}
			}
		}
		let mut rows = [[0.0; 4]; 2 * RINGS];
		for (k, r) in self.live.iter().enumerate() {
			let age = now - r.born;
			let rad = 0.15 + 1.3 * age - 0.15 * age * age;
			let fade = (1.0 - age / r.life).powi(2);
			rows[2 * k] = [r.p[0], r.p[1], r.p[2], rad];
			rows[2 * k + 1] = [r.slope * fade / (1.0 + 0.5 * rad), 0.22 + 0.15 * age, r.froth * (1.0 - age / 0.9).max(0.0), 0.0];
		}
		(rows, self.live.len())
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

/// Flow table slots (slot 0 = still water).
const FLOWS: usize = 64;
/// The flow slot of still water that retail draws opaque (Hoven's FIX 128 pools): ripples and reflections, no clear shallows.
pub(super) const OPAQUE_SLOT: u32 = FLOWS as u32 - 1;

/// Levels whose world strips (`Gpu::strip_draws`) are water: Novalis, whose strip classes 676 / 678 / 761 / 1225 are its
/// river, waterfall pool and cascade pools (retail/queue_q1.rs `STRIP_FN`; decomp/notes/water.md NOV-23.1).
pub(super) fn strips_are_water(level: usize) -> bool {
	level == 1
}

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
	/// The planar mirror (mirror.rs), half size: MSAA colour, its resolve (read by `fs_water`), MSAA depth.
	mirror_ms: wgpu::TextureView,
	mirror: wgpu::TextureView,
	mirror_depth: wgpu::TextureView,
	/// The water bed (mirror.rs `draw_bed`), half size: MSAA colour, its resolve and MSAA depth (both read by `fs_water`).
	bed_ms: wgpu::TextureView,
	bed: wgpu::TextureView,
	bed_depth: wgpu::TextureView,
}

pub(super) struct WaterFx {
	layout: wgpu::BindGroupLayout,
	ubuf: wgpu::Buffer,
	sampler: wgpu::Sampler,
	p_pos: wgpu::RenderPipeline,
	p_shade: wgpu::RenderPipeline,
	dummy: wgpu::TextureView,
	dummy_r32: wgpu::TextureView,
	format: wgpu::TextureFormat,
	depth_format: wgpu::TextureFormat,
	start: std::time::Instant,
	t: Option<Targets>,
	drift: Vec<Drift>,
	flows: Vec<[f32; 2]>,
	rings: Rings,
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
}

impl WaterFx {
	pub(super) fn new(device: &wgpu::Device, format: wgpu::TextureFormat, depth_format: wgpu::TextureFormat) -> WaterFx {
		let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("water fx"), source: wgpu::ShaderSource::Wgsl(SHADER.into()) });
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
				tex(6, wgpu::TextureSampleType::Float { filterable: true }, false),
				tex(7, unfiltered, false),
				tex(8, wgpu::TextureSampleType::Float { filterable: true }, false),
				tex(9, wgpu::TextureSampleType::Depth, true),
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
			dummy: small(format),
			dummy_r32: small(WPOS_FORMAT),
			format,
			depth_format,
			start: std::time::Instant::now(),
			t: None,
			drift: Vec::new(),
			flows: vec![[0.0; 2]],
			rings: Rings::default(),
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
		let mirror = half("water mirror", self.format, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
		let mirror_depth = half("water mirror depth", self.depth_format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT);
		let bed_ms = half("water bed msaa", self.format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT);
		let bed = half("water bed", self.format, 1, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
		let bed_depth = half("water bed depth", self.depth_format, MSAA, wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING);
		let bed_depth_only = bed_depth.texture().create_view(&wgpu::TextureViewDescriptor { aspect: wgpu::TextureAspect::DepthOnly, ..Default::default() });
		let group = |scene_v: &wgpu::TextureView, lin_v: &wgpu::TextureView, wpos_v: &wgpu::TextureView, mirror_v: &wgpu::TextureView, glow_v: &wgpu::TextureView, bed_v: &wgpu::TextureView| {
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
				],
			})
		};
		let g_pos = group(&self.dummy, &self.dummy_r32, &self.dummy_r32, &self.dummy, &self.dummy, &self.dummy);
		let g_shade = group(&copy_v, lin, &wpos, &mirror, glow, &bed);
		self.t = Some(Targets { key: (w, h, generation), wpos, wdepth, copy, g_pos, g_shade, mirror_ms, mirror, mirror_depth, bed_ms, bed, bed_depth });
	}

	/// After the level pass: the water meshes `draws` into the position target. Call before the post chain; `depth` is the level
	/// pass's MSAA depth, `scene` / `lin` / `glow` / `generation` the post's targets (`post::Post::water_inputs`); `parts`: the
	/// particle pass ran (Particles Enhanced), so the glow target holds their cover.
	#[allow(clippy::too_many_arguments)]
	pub(super) fn positions(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, depth: &wgpu::TextureView, (scene, lin, glow, generation): (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView, u64), f: &WaterFrame, draws: &[WaterDraw], mirror: Option<f32>, parts: bool) {
		self.prepare(device, depth, scene, lin, glow, generation);
		let Some(t) = &self.t else { return };
		let (w, h) = (t.key.0 as f32, t.key.1 as f32);
		let (sun, glint) = match f.key {
			// Travelling down (from above the horizon): the glint points back up the light.
			Some((d, luma)) if d[2] < -0.05 => ([-d[0], -d[1], -d[2]], GLINT * ((luma - 0.1) / 0.5).clamp(0.0, 1.0)),
			_ => ([0.0, 0.0, 1.0], 0.0),
		};
		let fog = f.fog.map(|c| c.max(0.0).powf(2.2));
		let (rings, rn) = self.rings.update(self.start.elapsed().as_secs_f32(), draws.iter().filter_map(|d| d.plane));
		let u = WaterU {
			vp: f.vp,
			eye: [f.eye[0], f.eye[1], f.eye[2], self.start.elapsed().as_secs_f32() % 3600.0],
			sun: [sun[0], sun[1], sun[2], glint],
			proj: f.proj,
			size: [w, h, 1.0 / w, 1.0 / h],
			look: [RIPPLES, REFRACTION, SHEEN, FOAM],
			fog: [fog[0], fog[1], fog[2], REFLECTION],
			mirror: [mirror.unwrap_or(0.0), mirror.is_some() as u32 as f32, parts as u32 as f32, CLEAR],
			flows: std::array::from_fn(|k| if k == OPAQUE_SLOT as usize { [0.0; 4] } else { self.flows.get(k).map_or([0.0, 0.0, 1.0, 0.0], |f| [f[0], f[1], 1.0, 0.0]) }),
			rn: [rn as f32, 0.0, 0.0, 0.0],
			rings,
		};
		queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("water positions"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.wpos, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.wdepth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }), stencil_ops: None }),
			timestamp_writes: None,
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.p_pos);
		pass.set_bind_group(0, &t.g_pos, &[]);
		for d in draws {
			pass.set_vertex_buffer(0, d.vb.slice(..));
			pass.set_index_buffer(d.ib.slice(..), wgpu::IndexFormat::Uint32);
			pass.draw_indexed(0..d.n, 0, d.flow..d.flow + 1);
		}
	}

	/// This field's contact candidates for the rings (`Rings::feed`): the game's field count, the hero's feet, pool particles.
	pub(super) fn contacts(&mut self, frame: u32, hero: Option<[f32; 3]>, parts: impl Iterator<Item = [f32; 3]>) {
		self.rings.feed(frame, hero, parts);
	}

	/// Before the level pass, on a level whose strips are water: the flow slot of each strip (0 = still), from the drift of
	/// its texture. Retail scrolls a river strip's uv a fixed step per game tick (FUN_002b9010, strips.rs `strip_scroll_step`);
	/// the step is measured on the strip's mean uv (frames where it does not move fall between two ticks, a jump past a third
	/// of the texture is the scroll's wrap) and turned into world units per second through the strip's uv-to-world map (a
	/// least-squares fit over its vertices). The ripples then drift the way the texture does, at its speed.
	pub(super) fn strip_flows(&mut self, strips: &[crate::particles::WorldStrip]) -> Vec<u32> {
		self.drift.resize(strips.len(), Drift::default());
		self.flows.truncate(1);
		let mut slots = Vec::with_capacity(strips.len());
		for (s, d) in strips.iter().zip(self.drift.iter_mut()) {
			let n = s.uv.len().max(1) as f32;
			let mean = s.uv.iter().fold([0.0f32; 2], |a, u| [a[0] + u[0] / n, a[1] + u[1] / n]);
			let step = [mean[0] - d.mean[0], mean[1] - d.mean[1]];
			let size = step[0].abs().max(step[1].abs());
			if d.seen && size > 1e-6 && size < 0.3 {
				d.step = if d.step == [0.0; 2] { step } else { [d.step[0] * 0.8 + step[0] * 0.2, d.step[1] * 0.8 + step[1] * 0.2] };
			}
			d.mean = mean;
			d.seen = true;
			let flow = uv_map(&s.pos, &s.uv).map(|inv| {
				// A texture feature at uv u sits at x = inv (u - c): it moves by -inv (uv step) per tick, 50 ticks a second.
				let v = [-(inv[0][0] * d.step[0] + inv[0][1] * d.step[1]) * 50.0, -(inv[1][0] * d.step[0] + inv[1][1] * d.step[1]) * 50.0];
				let l = (v[0] * v[0] + v[1] * v[1]).sqrt();
				if l > 6.0 { [v[0] * 6.0 / l, v[1] * 6.0 / l] } else { v }
			});
			match flow.filter(|v| v[0].abs() + v[1].abs() > 1e-3 && self.flows.len() < OPAQUE_SLOT as usize) {
				Some(v) => {
					slots.push(self.flows.len() as u32);
					self.flows.push(v);
				}
				None => slots.push(0),
			}
		}
		slots
	}

	/// The planar mirror's targets (MSAA colour, resolve, MSAA depth) for `Gpu::draw_mirror`; after `positions`.
	pub(super) fn mirror_target(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView)> {
		self.t.as_ref().map(|t| (&t.mirror_ms, &t.mirror, &t.mirror_depth))
	}

	/// The water bed's targets (MSAA colour, resolve, MSAA depth) for `Gpu::draw_bed`; after `positions`.
	pub(super) fn bed_target(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView)> {
		self.t.as_ref().map(|t| (&t.bed_ms, &t.bed, &t.bed_depth))
	}

	/// Inside the post chain (after the occlusion, before the bloom): shades the water pixels of the post scene `scene`.
	pub(super) fn shade(&self, enc: &mut wgpu::CommandEncoder, scene: &wgpu::TextureView) {
		let Some(t) = &self.t else { return };
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
