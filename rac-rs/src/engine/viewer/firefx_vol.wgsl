// [PC] Fire volumes (firefx_vol.rs): a flame, a fireball or a slab of a flame jet as a real 3D volume, raymarched inside a box
// around it. Same group 0 (the level shader's struct U, view_proj first) and group 2 (the soft-particle block of partfx.rs: the scene
// depth) as firefx.wgsl.
struct U { view_proj: mat4x4<f32>, sky_view_proj: mat4x4<f32> };
@group(0) @binding(0) var<uniform> u: U;

// eye: (camera xyz, clock seconds 0..PERIOD)   fwd: (view forward xyz, 2 tan(half the vertical field of view): the pixel's angle x the
// target's height, `pix`; the march jitter is a FIXED per-pixel pattern on purpose: a per-frame one flickers 2-3x more with frame
// smoothing on, review R7-5)
// q: (march steps, noise taps 1..3, glow gain, core gain)   k: (turbulence, rise speed, density, retail tint mix)
// k2: (smoke, opacity, brightness, tongue cut)   j: the jet's (flow speed m/s, cooling, warp, core gain)
struct VolU { eye: vec4<f32>, fwd: vec4<f32>, q: vec4<f32>, k: vec4<f32>, k2: vec4<f32>, j: vec4<f32> };
@group(1) @binding(0) var<uniform> vu: VolU;
@group(1) @binding(1) var ntex: texture_3d<f32>;
@group(1) @binding(2) var nsamp: sampler;

struct SoftU { p: vec4<f32>, n: vec4<f32>, l: array<vec4<f32>, 16> };
@group(2) @binding(0) var<uniform> soft: SoftU;
@group(2) @binding(1) var soft_depth: texture_depth_multisampled_2d;

const PERIOD: f32 = 512.0;
// Flame noise: how much taller than wide a noise cell is (tongues), and how far the fbm is spread around 0.5 (tearing).
const STRETCH: f32 = 2.6;
const CONTRAST: f32 = 1.7;
// A flame's box half-width in foot radii: room for the sway and the torn tongues at the top.
const BOX: f32 = 1.6;
// The scene's surface ends the march softly only inside a fire: what lies behind it fades over SOFT x the volume's radius (at most
// SOFT_MAX m), scaled by how much the fire in front of the surface already covers the pixel (a hard end at SOFT_FRONT.x or less, the
// whole fade from .y on). So a ledge standing inside a big fire does not cut a hard-edged dark hole into it, while whatever stands in
// front of a volume (a pillar, the hero's arm before a jet, the ground at a fireball's rim) hides the fire behind it.
const SOFT: f32 = 0.4;
const SOFT_MAX: f32 = 0.5;
const SOFT_FRONT: vec2<f32> = vec2<f32>(0.1, 0.5);
// A flame's heat gain: the band-limited detail taps leave the flame's body smoother, which reads whiter; 0.95 restores the colour of
// the flame with every detail tap.
const FHEAT: f32 = 0.95;
// A noise tap fades in between these ratios of its finest cell to the march step (`band`).
const BAND_LO: f32 = 0.7;
const BAND_HI: f32 = 1.4;
// On Low a base tap reads a coarser mip once its finest cell is shorter than 1 / LOD_K march steps (`lod`).
const LOD_K: f32 = 1.4;
// The pixel's footprint limits the noise like the march step does (`pix`): a base tap reads a coarser mip once its finest cell is
// shorter than PIX_LOD pixels, a detail tap fades out by its finest cell over PIX_BAND pixels (BAND_LO..BAND_HI). Far away (a burning
// building across Novalis) the finest cells were a fraction of a pixel: they aliased, and every camera move re-sampled them, so the
// fire crawled and flickered. Up close the pixel is far smaller than the cells and the march step: the same picture as before.
const PIX_LOD: f32 = 3.0;
const PIX_BAND: f32 = 2.0;
// The jet: its marched cylinder in radii of the slab's wider end, how much longer its noise cells are along the stream than across,
// the noise spread, and the metres from the nozzle over which the cone opens to the stream's width.
const JBOX: f32 = 1.7;
const JSTRETCH: f32 = 2.2;
const JCONTRAST: f32 = 1.9;
const JOPEN: f32 = 1.2;
// The jet's warp flows at JWARP x the stream's speed (a little slower: the billows change shape as they travel), the detail at
// JDETAIL x (a little faster than the billows it rides on; much faster turns into boiling).
const JWARP: f32 = 0.8;
// The jet's noise tile across the stream (m; along it JSTRETCH x longer), x the width gain fire_jet_vw (row g.w).
const JTILE: f32 = 0.75;
// The jet's density over the flames', the width of its soft edge (in the field's units) and how much of its body burns out to soot.
const JDENS: f32 = 1.6;
const JEDGE: f32 = 0.1;
const JSOOT: f32 = 1.0;
const JOCC: f32 = 0.7;
// The jet's march steps over a flame's: High / Medium, Low (Low 24, Medium 36, High 60 across the full width).
const JSTEPS: f32 = 1.5;
const JSTEPS_LOW: f32 = 2.0;
const JDETAIL: f32 = 1.1;
// A fireball the eye stands in or at fades as a whole (from BALL_NEAR.y x its radius away down to BALL_NEAR.x x): a camera inside the
// big splash balls where a jet hits the ground saw a screen-wide flat red wall (the game's own sprites show nothing there). Near the eye
// a jet slab fades out over NEAR m (nothing within a quarter of it; the sprites it stands for had a near fade). Flames and fireballs
// keep their full body near the eye otherwise (a level fire close to the camera stays whole).
const NEAR: f32 = 1.5;
const BALL_NEAR: vec2<f32> = vec2<f32>(0.8, 1.6);

// The depth rebuilt from w (viewer.rs world_clip): the box's depth must match the level's.
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

struct VIn {
	@builtin(position) pos: vec4<f32>,
	@location(0) w: vec3<f32>,
	// a: (flame base / ball centre xyz, flame height)   b: (radius, seed, fade, kind: 0 flame, 1 fireball, 2 jet slab)   c: (tint rgb,
	// strength). A jet slab (firefx_jet_vol.rs `slabs`): a = (P0, r0), b = (chain end s, seed, alpha at P1, 2), c = (tint, alpha at P0),
	// d = (P1, r1), e = (plane normal at P0, s at P0), f = (plane normal at P1, s at P1), g = (jet direction, width gain fire_jet_vw:
	// the noise tile is JTILE x it); else zero.
	@location(1) @interpolate(flat) a: vec4<f32>,
	@location(2) @interpolate(flat) b: vec4<f32>,
	@location(3) @interpolate(flat) c: vec4<f32>,
	@location(4) @interpolate(flat) d: vec4<f32>,
	@location(5) @interpolate(flat) e: vec4<f32>,
	@location(6) @interpolate(flat) f: vec4<f32>,
	@location(7) @interpolate(flat) g: vec4<f32>,
};

struct FireOut { @location(0) c: vec4<f32>, @location(1) glow: vec4<f32> };

// The box the volume lives in: a fireball's cube, a flame's column (room for its sway, its foot a little under the base), a jet slab's
// axis grown by its marched cylinder.
fn box_lo(a: vec4<f32>, b: vec4<f32>, d: vec4<f32>) -> vec3<f32> {
	if (b.w > 1.5) {
		return min(a.xyz, d.xyz) - vec3<f32>(JBOX * max(a.w, d.w));
	}
	if (b.w > 0.5) {
		return a.xyz - vec3<f32>(b.x);
	}
	let m = b.x * BOX;
	return a.xyz - vec3<f32>(m, m, 0.1 * a.w);
}

fn box_hi(a: vec4<f32>, b: vec4<f32>, d: vec4<f32>) -> vec3<f32> {
	if (b.w > 1.5) {
		return max(a.xyz, d.xyz) + vec3<f32>(JBOX * max(a.w, d.w));
	}
	if (b.w > 0.5) {
		return a.xyz + vec3<f32>(b.x);
	}
	let m = b.x * BOX;
	return a.xyz + vec3<f32>(m, m, a.w);
}

// 36 corners (12 triangles) of the box; corner bits x = 1, y = 2, z = 4. No culling: the fragment keeps the far face itself.
const CUBE = array<u32, 36>(0u, 2u, 6u, 0u, 6u, 4u, 1u, 5u, 7u, 1u, 7u, 3u, 0u, 4u, 5u, 0u, 5u, 1u, 2u, 3u, 7u, 2u, 7u, 6u, 0u, 1u, 3u, 0u, 3u, 2u, 4u, 6u, 7u, 4u, 7u, 5u);

@vertex fn vs_vol(@builtin(vertex_index) vi: u32, @location(0) a: vec4<f32>, @location(1) b: vec4<f32>, @location(2) c: vec4<f32>, @location(3) d: vec4<f32>, @location(4) e: vec4<f32>, @location(5) f: vec4<f32>, @location(6) g: vec4<f32>) -> VIn {
	var cube = CUBE;
	let k = cube[vi % 36u];
	let lo = box_lo(a, b, d);
	let hi = box_hi(a, b, d);
	let w = vec3<f32>(select(lo.x, hi.x, (k & 1u) != 0u), select(lo.y, hi.y, (k & 2u) != 0u), select(lo.z, hi.z, (k & 4u) != 0u));
	var o: VIn;
	o.pos = world_clip(u.view_proj, w);
	o.w = w;
	o.a = a;
	o.b = b;
	o.c = c;
	o.d = d;
	o.e = e;
	o.f = f;
	o.g = g;
	return o;
}

// Heat 0..1 -> colour: deep red, red-orange, orange, yellow, white-yellow.
fn ramp(T: f32) -> vec3<f32> {
	let a = vec3<f32>(0.25, 0.015, 0.0);
	let b = vec3<f32>(0.95, 0.12, 0.01);
	let c = vec3<f32>(1.0, 0.40, 0.04);
	let d = vec3<f32>(1.0, 0.74, 0.20);
	let e = vec3<f32>(1.0, 0.95, 0.75);
	var col = mix(a, b, smoothstep(0.0, 0.25, T));
	col = mix(col, c, smoothstep(0.22, 0.5, T));
	col = mix(col, d, smoothstep(0.48, 0.78, T));
	return mix(col, e, smoothstep(0.78, 1.0, T));
}

fn nz(p: vec3<f32>) -> vec4<f32> {
	return textureSampleLevel(ntex, nsamp, p, 0.0);
}

// A base tap at mip level `lod` (firefx_vol.rs: the noise has 3 levels, each the 2x2x2 mean of the one above).
fn nzl(p: vec3<f32>, lod: f32) -> vec4<f32> {
	return textureSampleLevel(ntex, nsamp, p, lod);
}

// The mip level of a base tap whose finest cell is `cell` long at march step `dt` and pixel footprint `fp` (m). On Low (one tap) its
// long steps cannot resolve the finest cells, and sampled at random they turned into grain, so a coarser level averages them out (cell
// shorter than 1 / LOD_K steps). On every level a cell shorter than PIX_LOD pixels reads the coarser level too (far fires).
fn lod(cell: f32, dt: f32, fp: f32) -> f32 {
	let c = max(cell, 1.0e-4);
	let m = select(0.0, log2(LOD_K * dt / c), vu.q.y < 1.5);
	return clamp(max(m, log2(PIX_LOD * fp / c)), 0.0, 2.0);
}

// The pixel's footprint (m) at distance t along the ray: vu.fwd.w over the target's height (the scene depth's) per metre.
fn pix_scale() -> f32 {
	return vu.fwd.w / f32(textureDimensions(soft_depth).y);
}

// A scroll of `rate` noise tiles per second that comes back to the same place at the clock's wrap (a whole number of tiles a period).
// It jumps by one whole tile at its own wrap, so it must only offset a tap's coordinate directly, never be scaled with it (a tap at
// 2.31 x a scrolled point would jump by 2.31 tiles, a pop of its pattern every few seconds).
fn scroll(rate: f32) -> f32 {
	return fract(round(PERIOD * rate) * (vu.eye.w / PERIOD));
}

// The weight of a noise tap whose finest cell is `cell` long at march step `dt` and pixel footprint `fp`: 1 when the march and the
// pixel resolve it, 0 when it would alias into a fine fixed hatch (the step) or a crawling flicker (the pixel, far away).
fn band(cell: f32, dt: f32, fp: f32) -> f32 {
	return smoothstep(BAND_LO, BAND_HI, cell / max(max(dt, PIX_BAND * fp), 1.0e-4));
}

// The noise contrast's gain for a base tap whose finest cell is `cell` long at march step `dt`: 1 where the march resolves it (High and
// Medium), down to 0.8 on Low's coarse steps (with the coarser mips of `lod`; 0.6 flattened a jet's billows into a soft blob).
fn coarse(cell: f32, dt: f32) -> f32 {
	return mix(0.8, 1.0, smoothstep(0.25, 0.55, cell / max(dt, 1.0e-4)));
}

// The march's start offset at a pixel: a hash of the pixel and the volume's seed (white noise; fixed per pixel, review R7-5). The
// interleaved-gradient pattern used before has diagonal structure that showed as a fine hatch, and was the same for all overlapping
// volumes, so their errors added up instead of averaging out.
fn jitter(px: vec2<f32>, sd: f32) -> f32 {
	var h = (u32(px.x) * 1973u) ^ (u32(px.y) * 9277u) ^ (u32(sd * 65536.0) * 26699u) ^ 0x9e3779b9u;
	h = (h ^ (h >> 16u)) * 0x7feb352du;
	h = (h ^ (h >> 15u)) * 0x846ca68bu;
	h = h ^ (h >> 16u);
	return f32(h >> 8u) * (1.0 / 16777216.0);
}

fn seed_off(sd: f32) -> vec3<f32> {
	return vec3<f32>(fract(sd * 7.618034), fract(sd * 3.381966), fract(sd * 5.754878));
}

// The flame's field at p: (heat, density). Local frame: base a.xyz, height a.w, radius b.x at the foot; dt = the march step.
// d.x > 0: the flame's own noise (firefx_vol.rs `noise_of`): tile size d.x, scroll d.y tiles/s at fire speed 1, kept while its size
// changes; else both come from its size.
fn flame_at(p: vec3<f32>, a: vec4<f32>, b: vec4<f32>, d4: vec4<f32>, dt: f32, fp: f32) -> vec2<f32> {
	let R = b.x;
	let H = a.w;
	let lp = p - a.xyz;
	let hz = lp.z / H;
	let turb = vu.k.x;
	// Empty space (most of the box: room for the sway and the tongues) costs no noise taps: below the foot, above the tip, or so far
	// out that even the widest sway and the deepest noise bulge cannot reach (the density there is exactly 0, the same picture).
	if (hz <= -0.1 || hz >= 1.0) {
		return vec2<f32>(0.0);
	}
	let hc0 = clamp(hz, 0.0, 1.0);
	let reach = 0.95 * pow(1.0 - hc0, 0.7) + 0.05 + 0.5 * abs(vu.k2.w) * (0.35 + 1.5 * hc0) + 0.02;
	if (length(lp.xy) / R - 0.78 * hz * hz * abs(turb) >= reach) {
		return vec2<f32>(0.0);
	}
	// Noise tiles: across by the flame's width, stretched up so the billows become tongues; they rise with the heat.
	let own = d4.x > 0.0;
	let s = select(max(R * 1.3, 0.25), d4.x, own);
	let sv = s * STRETCH;
	let rise = vu.k.y * (0.9 + 0.25 * H / s);
	let rk = vu.k.y * d4.y;
	let nq = vec3<f32>(lp.x / s, lp.y / s, lp.z / sv) + seed_off(b.y);
	let wq = nq * 0.45 + vec3<f32>(0.37, 0.71, -scroll(select(rise * 0.3 / sv, rk * 0.3, own)));
	let wv = nzl(wq, lod(s * 0.139, dt, fp)).rgb - vec3<f32>(0.5);
	let wp = nq + wv * (1.1 * turb) * (0.25 + hz);
	let n0 = nzl(wp - vec3<f32>(0.0, 0.0, scroll(select(rise / sv, rk, own))), lod(s * 0.0625, dt, fp)).r;
	var n = n0;
	// The detail taps only where the march resolves them: a tap's finest noise cell (s / 37, s / 66 across) shorter than ~1.4 steps
	// would alias into a fixed fine hatch, so its weight fades out by the step length (the base tap's finest cell is s / 16). Each
	// tap scrolls on its own (`scroll`), at the speed it had riding on the base tap's scroll.
	let w1 = band(s * 0.027, dt, fp);
	if (vu.q.y > 1.5 && w1 > 0.0) {
		let d1 = nz(wp * 2.31 + vec3<f32>(0.5, 0.2, -scroll(select(rise * 3.71 / sv, rk * 3.71, own)))).b;
		n = mix(n0, n0 * 0.62 + d1 * 0.38, w1);
	}
	let w2 = band(s * 0.015, dt, fp);
	if (vu.q.y > 2.5 && w2 > 0.0) {
		let d2 = nz(wp * 4.1 + vec3<f32>(0.1, 0.8, -scroll(select(rise * 6.5 / sv, rk * 6.5, own)))).r;
		n = mix(n, n * 0.8 + d2 * 0.2, w2);
	}
	// Value-noise fbm sits near 0.5: spread it so the flame tears into separate tongues instead of a soft blob. Less so where the march
	// cannot resolve the base tap's finest cell (s / 16 shorter than ~0.5 step: Low): torn thin there, the flame turned to grain.
	n = clamp((n - 0.5) * CONTRAST * coarse(s * 0.0625, dt) + 0.5, 0.0, 1.0);
	// The whole flame leans and sways with height (the slow warp), the profile narrows to the tip.
	let sway = wv.xy * (hz * hz) * R * 1.1 * turb;
	let rad = length(lp.xy - sway) / R;
	let hc = clamp(hz, 0.0, 1.0);
	let prof = 0.95 * pow(1.0 - hc, 0.7) + 0.05;
	let cut = vu.k2.w * (0.35 + 1.5 * hc);
	let d = (prof - rad) + (n - 0.5) * cut;
	let foot = smoothstep(-0.1, 0.04, hz) * (1.0 - smoothstep(0.86, 1.0, hz));
	// Nothing reaches the marched column's wall (radius BOX R around the unswayed axis): a torn tongue swayed out that far would be
	// cut by it into a straight edge.
	let wall = 1.0 - smoothstep(BOX - 0.35, BOX - 0.05, length(lp.xy) / R);
	let dens = smoothstep(-0.02, 0.3, d) * foot * wall;
	// Heat: white-yellow in the core near the foot, orange in the body, red-orange at the torn edges and the tips.
	let core = smoothstep(0.35, 0.8, d) * (1.0 - 0.75 * hc);
	let T = clamp(max((d * 1.3 + 0.08) * (1.1 - 0.85 * hc) * (0.5 + 0.8 * n), core), 0.0, 1.0) * foot * FHEAT;
	return vec2<f32>(T, dens);
}

// A fireball's field at p: (heat, density). Centre a.xyz, radius b.x; b.z = its age (1 new, 0 dying); dt = the march step.
fn ball_at(p: vec3<f32>, a: vec4<f32>, b: vec4<f32>, dt: f32, fp: f32) -> vec2<f32> {
	let R = b.x;
	let lp = p - a.xyz;
	let r = length(lp) / R;
	if (r >= 1.0) {
		return vec2<f32>(0.0);
	}
	let s = max(R * 0.8, 0.2);
	let turb = vu.k.x;
	let wv = nzl((lp / s + seed_off(b.y)) * 0.5 + vec3<f32>(0.2, 0.6, -scroll(vu.k.y * 0.15 / s)), lod(s * 0.125, dt, fp)).rgb - vec3<f32>(0.5);
	let wp = lp / s + seed_off(b.y) + wv * 1.1 * turb;
	var n = nzl(wp - vec3<f32>(0.0, 0.0, scroll(vu.k.y * 0.5 / s)), lod(s * 0.0625, dt, fp)).r;
	let w1 = band(s * 0.026, dt, fp);
	if (vu.q.y > 1.5 && w1 > 0.0) {
		n = mix(n, n * 0.65 + nz(wp * 2.4 + vec3<f32>(0.3, 0.1, -scroll(vu.k.y * 1.2 / s))).b * 0.35, w1);
	}
	let fade = clamp(b.z, 0.0, 1.0);
	let d = (1.0 - r) - 0.1 + (n - 0.5) * 0.9 * turb - (1.0 - fade) * 0.25;
	let dens = smoothstep(0.0, 0.25, d) * (1.0 - smoothstep(0.85, 1.0, r));
	let T = clamp((d * 1.4 + 0.1) * (0.35 + 0.65 * fade) * (0.55 + 0.8 * n), 0.0, 1.0);
	return vec2<f32>(T, dens);
}

// A slab of a flame jet (rows as in VIn): (heat, density) at p. The axis runs P0 = a.xyz .. P1 = d.xyz; t (0..1) comes from
// the distances to the two end planes, so neighbouring slabs agree on their shared plane (no seam); sm = metres from the nozzle.
fn jet_at(p: vec3<f32>, a: vec4<f32>, b: vec4<f32>, c: vec4<f32>, d: vec4<f32>, e: vec4<f32>, f: vec4<f32>, g: vec4<f32>, dt: f32, fp: f32) -> vec2<f32> {
	let d0 = max(dot(p - a.xyz, e.xyz), 0.0);
	let d1 = max(dot(d.xyz - p, f.xyz), 0.0);
	let t = d0 / max(d0 + d1, 1.0e-5);
	let off = p - mix(a.xyz, d.xyz, t);
	let ra = length(off);
	let rm = max(a.w, d.w);
	if (ra >= rm * (JBOX - 0.05)) {
		return vec2<f32>(0.0);
	}
	let sm = mix(e.w, f.w, t);
	let sn = clamp(sm / b.x, 0.0, 1.0);
	// The cone opens from a narrow mouth at the nozzle to the stream's width.
	let R = max(mix(a.w, d.w, t) * mix(0.3, 1.0, smoothstep(0.0, JOPEN, sm)), 0.02);
	// The noise lives in the stream's own frame: along = metres from the nozzle along the chain, across = the offset from the axis in
	// a frame square to the jet's overall direction. It is continuous across the slabs, follows the stream's bends and flows only along
	// it; nothing in it depends on where the jet stands in the world (a world-space pattern far from the origin turned into a new
	// pattern on every frame the chain's direction or scale moved a little: the flame boiled). The frame turns continuously with W
	// (Frisvad's basis, singular only for a jet aimed straight down, W.z = -1): a switch of reference axis at a fixed angle made the
	// whole pattern jump in one frame when the aim crossed it (firefx_jet_vol.rs tests replicate these lines).
	let W = g.xyz;
	let fa = 1.0 / max(1.0 + W.z, 1.0e-6);
	let U1 = vec3<f32>(1.0 - W.x * W.x * fa, -W.x * W.y * fa, -W.x);
	let U2 = cross(W, U1);
	let S = JTILE * g.w;
	let rate = vu.j.x / (S * JSTRETCH);
	let q = vec3<f32>(dot(off, U1) / S, dot(off, U2) / S, sm / (S * JSTRETCH)) + seed_off(b.y) * 7.0;
	let wv = nzl(q * 0.45 + vec3<f32>(0.37, 0.71, 0.13 - scroll(rate * 0.45 * JWARP)), lod(S * 0.139, dt, fp)).rgb - vec3<f32>(0.5);
	let wp = q + wv * (1.3 * vu.j.z) * (0.35 + 0.9 * sn);
	var n = nzl(wp - vec3<f32>(0.0, 0.0, scroll(rate)), lod(S * 0.0625, dt, fp)).r;
	let w1 = band(S * 0.027, dt, fp);
	if (vu.q.y > 1.5 && w1 > 0.0) {
		n = mix(n, n * 0.62 + nz(wp * 2.31 + vec3<f32>(0.5, 0.2, 0.9 - scroll(rate * 2.31 * JDETAIL))).b * 0.38, w1);
	}
	n = clamp((n - 0.5) * JCONTRAST * coarse(S * 0.0625, dt) + 0.5, 0.0, 1.0);
	// Torn more toward the tip, where only the densest billows are left; nothing at the nozzle's plane, the chain's end or the wall.
	let dd = (1.0 - ra / R) + (n - 0.5) * (0.45 + 0.95 * sn) * vu.k2.w - 0.7 * smoothstep(0.55, 1.0, sn);
	// A piece cut off from the stream starts s0 = b.w - 2 metres down it (firefx_jet_vol.rs `slabs_into`): its start fades in over up
	// to a metre more instead of a nozzle's 0.1 m.
	let s0 = max(b.w - 2.0, 0.0);
	let ends = smoothstep(s0, s0 + 0.1 + 0.8 * min(s0, 1.0), sm) * (1.0 - smoothstep(0.9, 1.0, sm / b.x));
	let wall = 1.0 - smoothstep(JBOX - 0.35, JBOX - 0.05, ra / rm);
	// The edge is never sharper than the march resolves (Low's coarse steps turned a sharper one into grain).
	let dens = smoothstep(0.0, max(JEDGE, 1.2 * dt / rm), dd) * ends * wall * smoothstep(0.0, 0.35, mix(c.w, b.z, t)) * JDENS;
	// Heat: white-yellow along the axis near the nozzle, orange in the body, cooling to dark red at the tip; the warp's green makes
	// cooler streaks. Soot: from the body on, the warp's blue marks patches that burn out into dark smoke (they hide the flame behind).
	let cool = clamp(sn * vu.j.y, 0.0, 1.0);
	let core = smoothstep(0.3, 0.8, dd) * (1.0 - smoothstep(0.05, 0.45, sn)) * vu.j.w;
	let soot = smoothstep(0.0, 0.3, wv.b + 0.45 * sn - 0.32) * smoothstep(0.25, 0.7, sn) * JSOOT;
	let T = (dd * 1.3 + 0.1) * (1.1 - 0.85 * cool) * (0.45 + 0.8 * n) * (0.8 + 0.6 * wv.g) * (1.0 - soot);
	return vec2<f32>(clamp(max(T, core), 0.0, 1.0), dens);
}

// The ray's span (ta, tb) inside a flame's round column (around its axis, radius BOX R) or a fireball's sphere, and that radius: the
// steps go where the fire can be (a ray looking straight down the column keeps the box's segment).
fn clip_round(eye: vec3<f32>, rd: vec3<f32>, ta: f32, tb: f32, a: vec4<f32>, R: f32, ball: bool) -> vec3<f32> {
	let rr = select(R * BOX, R, ball);
	let keep = vec3<f32>(1.0, 1.0, select(0.0, 1.0, ball));
	let oq = (eye - a.xyz) * keep;
	let rq = rd * keep;
	let qa = dot(rq, rq);
	var s = vec3<f32>(ta, tb, rr);
	if (qa > 1.0e-4) {
		let qb = dot(oq, rq);
		let disc = qb * qb - qa * (dot(oq, oq) - rr * rr);
		if (disc <= 0.0) {
			return vec3<f32>(1.0, 0.0, rr);
		}
		let sq = sqrt(disc);
		s.x = max(s.x, (-qb - sq) / qa);
		s.y = min(s.y, (-qb + sq) / qa);
	}
	return s;
}

// The ray's span inside a jet slab: between its two end planes and inside the cylinder (radius JBOX x the wider end) round its axis.
fn clip_jet(eye: vec3<f32>, rd: vec3<f32>, ta: f32, tb: f32, a: vec4<f32>, d: vec4<f32>, e: vec4<f32>, f: vec4<f32>) -> vec3<f32> {
	let rr = JBOX * max(a.w, d.w);
	var s = vec3<f32>(ta, tb, rr);
	let none = vec3<f32>(1.0, 0.0, rr);
	// dot(p - P0, n0) >= 0 and dot(p - P1, n1) <= 0.
	let k0 = dot(rd, e.xyz);
	let h0 = dot(a.xyz - eye, e.xyz);
	if (abs(k0) < 1.0e-6) {
		if (h0 > 0.0) { return none; }
	} else if (k0 > 0.0) {
		s.x = max(s.x, h0 / k0);
	} else {
		s.y = min(s.y, h0 / k0);
	}
	let k1 = dot(rd, f.xyz);
	let h1 = dot(d.xyz - eye, f.xyz);
	if (abs(k1) < 1.0e-6) {
		if (h1 < 0.0) { return none; }
	} else if (k1 > 0.0) {
		s.y = min(s.y, h1 / k1);
	} else {
		s.x = max(s.x, h1 / k1);
	}
	let ax = d.xyz - a.xyz;
	let u = ax / max(length(ax), 1.0e-5);
	let o = eye - a.xyz;
	let oq = o - u * dot(o, u);
	let rq = rd - u * dot(rd, u);
	let qa = dot(rq, rq);
	if (qa > 1.0e-8) {
		let qb = dot(oq, rq);
		let disc = qb * qb - qa * (dot(oq, oq) - rr * rr);
		if (disc <= 0.0) { return none; }
		let sq = sqrt(disc);
		s.x = max(s.x, (-qb - sq) / qa);
		s.y = min(s.y, (-qb + sq) / qa);
	} else if (dot(oq, oq) >= rr * rr) {
		return none;
	}
	return s;
}

@fragment fn fs_vol(i: VIn) -> FireOut {
	let eye = vu.eye.xyz;
	let dv = i.w - eye;
	let tf = length(dv);
	let rd = dv / tf;
	let lo = box_lo(i.a, i.b, i.d);
	let hi = box_hi(i.a, i.b, i.d);
	let inv = vec3<f32>(1.0) / select(rd, vec3<f32>(1.0e-6), abs(rd) < vec3<f32>(1.0e-6));
	let t0 = (lo - eye) * inv;
	let t1 = (hi - eye) * inv;
	let tn = min(t0, t1);
	let tx = max(t0, t1);
	let ta0 = max(max(tn.x, tn.y), tn.z);
	let tb0 = min(min(tx.x, tx.y), tx.z);
	// One fragment per pixel: only the box's far face marches (the camera may stand inside the box; the winding does not matter).
	if (tf < 0.5 * (ta0 + tb0)) {
		discard;
	}
	let jet = i.b.w > 1.5;
	let ball = i.b.w > 0.5 && !jet;
	// The radius the steps, the density and the soft end go by: a flame's foot, a fireball's, a jet slab's wider end.
	let R = select(i.b.x, max(i.a.w, i.d.w), jet);
	let inside = select(1.0, smoothstep(BALL_NEAR.x * R, BALL_NEAR.y * R, length(eye - i.a.xyz)), ball);
	if (inside <= 0.001) {
		discard;
	}
	var span: vec3<f32>;
	if (jet) {
		span = clip_jet(eye, rd, max(ta0, 0.05), tb0, i.a, i.d, i.e, i.f);
	} else {
		span = clip_round(eye, rd, max(ta0, 0.05), tb0, i.a, R, ball);
	}
	let ta = span.x;
	let rr = span.z;
	// The scene's surface ts ends the ray, softly only inside a fire (SOFT): `sd` is the longest fade behind it, `sk` the one this ray
	// takes, set at its first step past the surface from how much the fire in front of it covers.
	let n = soft.p.x;
	let f = soft.p.y;
	let zs = textureLoad(soft_depth, vec2<i32>(i.pos.xy), 0);
	let ls = n * f / (f - zs * (f - n));
	let ts = ls / max(dot(rd, vu.fwd.xyz), 0.05);
	let sd = clamp(SOFT * R, 0.03, SOFT_MAX);
	let tb = min(span.y, ts + 3.0 * sd);
	if (tb <= ta) {
		discard;
	}
	// Steps by the length marched: the column's full width gets the quality's steps, a shorter chord (near an edge, cut by a wall)
	// fewer at the same spacing, so the spacing stays fine enough for the noise (coarse spacing shows as a fine hatch).
	// A jet slab takes JSTEPS x the steps (JSTEPS_LOW x on Low): its dense, sharp-edged flame needs the finer spacing (coarser shows as
	// grain); a jet is a few volumes, so this costs little.
	let qs = vu.q.x * select(1.0, select(JSTEPS, JSTEPS_LOW, vu.q.y < 1.5), jet);
	// The steps sit on planes fixed in the volume (counted from its centre's projection on the ray), at the full width's spacing (at
	// least 3 on a short chord): a moving eye moved the steps counted from the chord's start through the volume, so each frame sampled
	// other points of the noise and a far fire's flames changed shape on every frame (they flickered while the camera moved).
	// The steps always reach the chord's end: up to `cap` steps (3 full widths) at the planes' spacing, a longer chord (a jet seen along
	// its stream, a tall flame seen from above) at a coarser spacing over the whole of it, as before the fixed planes.
	let cap = 3.0 * max(qs, 3.0);
	let dt = max(min(2.0 * rr / max(qs, 3.0), (tb - ta) / 3.0), (tb - ta) / cap);
	let jit = jitter(i.pos.xy, i.b.y);
	let tc = dot(0.5 * (lo + hi) - eye, rd);
	let tq = tc + (ceil((ta - tc) / dt - jit) + jit) * dt;
	let steps = i32(clamp(ceil((tb - tq) / dt), 0.0, cap + 1.0));
	let pxs = pix_scale();
	let k = vu.k.z * dt / R;
	let tn3 = i.c.rgb / max(max(i.c.r, max(i.c.g, i.c.b)), 0.001);
	var tr = 1.0;
	var col = vec3<f32>(0.0);
	var smoke = 0.0;
	var sk = -1.0;
	for (var s = 0; s < steps; s++) {
		let tp = tq + f32(s) * dt;
		if (tp > ts) {
			if (sk < 0.0) {
				sk = sd * smoothstep(SOFT_FRONT.x, SOFT_FRONT.y, 1.0 - tr);
			}
			if (tp - ts >= 3.0 * sk) {
				break;
			}
		}
		let p = eye + rd * tp;
		var fd: vec2<f32>;
		if (jet) {
			fd = jet_at(p, i.a, i.b, i.c, i.d, i.e, i.f, i.g, dt, tp * pxs);
		} else if (ball) {
			fd = ball_at(p, i.a, i.b, dt, tp * pxs);
		} else {
			fd = flame_at(p, i.a, i.b, i.d, dt, tp * pxs);
		}
		if (fd.y <= 0.002) {
			continue;
		}
		let behind = select(1.0, exp(-(tp - ts) / max(sk, 1.0e-3)), tp > ts);
		let al = (1.0 - exp(-fd.y * k)) * behind * select(1.0, smoothstep(0.25 * NEAR, NEAR, tp), jet);
		let T = fd.x;
		let tint = mix(vec3<f32>(1.0), tn3, vu.k.w * (1.0 - T));
		let e = ramp(T) * tint * (0.3 + 2.2 * T * T * vu.q.w + 0.6 * T);
		col += tr * al * e;
		smoke += tr * al * (1.0 - smoothstep(0.05, 0.3, T));
		// How much a step hides what lies behind it: a jet's flame and soot are thick (an opaque, crisp surface); a flame's glow thinner.
		tr *= 1.0 - al * select(0.25 + 0.6 * T, JOCC + (1.0 - JOCC) * T, jet);
		if (tr < 0.02) {
			break;
		}
	}
	let fade = select(clamp(select(i.b.z, 1.0, ball), 0.0, 1.0) * i.c.w * inside, 1.0, jet);
	let lit = col * vu.k2.z * fade;
	let cover = clamp((1.0 - tr) * vu.k2.y + smoke * vu.k2.x * 0.35, 0.0, 1.0) * fade;
	if (cover <= 0.001 && max(lit.r, lit.g) <= 0.001) {
		discard;
	}
	var o: FireOut;
	o.c = vec4<f32>(lit + vec3<f32>(0.04, 0.03, 0.025) * smoke * vu.k2.x * 0.35 * fade, cover);
	o.glow = vec4<f32>(clamp(lit * soft.p.w * vu.q.z * 0.6, vec3<f32>(0.0), vec3<f32>(1.0)), cover);
	return o;
}
