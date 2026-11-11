// [PC] Shadows from the fire lights (lane dlit stage 2, 2026-11-05; lane dlit2 stage 2, 2026-11-06: soft, 4 maps; sunshadow/omni.rs).
// Global illumination High (4 fires) / Medium (1): a fire (flitlist.rs `groups`) may hold a cube shadow map; every light of that fire
// reads it (flit.wgsl, its row's slot; once per pixel). Row FLS_MAPS + s = (the map's origin, far; 0 = no map), FLS_MAPN = the near
// planes of maps 0..3, FLS_MAPF = their fades (a map fades in and out, no popping), FLS_SIZE = the fire radius of maps 0..3. All
// zero: every light unshadowed.
// Fire light v2 stage 5 (lane dlit3, FIRE_LIGHT_V2.md 3.3): the lookup reads the maps' prefiltered MOMENT cubes (sunshadow/
// omni_mom.rs: per texel the mean (z, z^2, z^3, z^4) of the radial distance from the map's origin, z over [near, far], mips 128..8),
// not the depth atlas (the stage-2 PCSS grid search is gone). Per receiver: (1) a blocker search over the cone the emitter (radius
// FLS_SIZE) fills seen from the receiver (its radius x FLS_MAPQ .y), 4 taps: the blocked share and the mean blocker depth from the 4 moments
// (Hamburger 4MSM, Peters & Klein 2015, the canonical 3-point representation through the receiver's depth); nothing blocked: lit.
// (2) The penumbra cone seen from the map's origin, phi = R (1 / d_blocker - 1 / d_receiver) (similar triangles: a blocker close
// to a big emitter, the host hull, gives a cone of tens of degrees, so its shadow is soft and short, not a hard copy of the hull),
// at most FLS_MAPQ .z rad; (3) the lit share = 1 - the mean Hamburger share of 4 taps on that cone, each at the mip whose texel is
// the cone (x 2^FLS_MAPQ .x). Each tap compares the depth of the receiver's own PLANE in its direction (no self-shadow from a wide
// tap on the sand), less a slope bias for its texel's spread. FLS_MAPQ = (lod bias, emitter radius scale, widest cone (rad), light-
// bleeding reduction); FLS_BIAS = (normal offset in mip-0 texels, depth bias (m), slope bias scale, moment bias).
// FXC-safe: textureSampleLevel with an explicit lod, constant-bounded tap loops, no dynamic indexing of local arrays.
// Lane dlit2-9 (GPU cost): the pipelines come in two sets, this lookup compiled in (true) or out (false: a frame that reads no map,
// GI Low / Off, Classic, no fire with a map; metalpipe.rs `set_fls`), so the frames without fire shadows pay nothing for the code.
override FLS_ON: bool = true;
// Twins of sunshadow/omni.rs NEAR and flit.rs MAPS, MAPQ, MAPF, MAPSZ, MAPB, MAPZN.
const FLS_NEAR: f32 = 0.25;
const FLS_MAPN: u32 = 75u;
const FLS_TAIL: f32 = 0.1;
const FLS_MAPS: u32 = 65u;
const FLS_MAPQ: u32 = 69u;
const FLS_MAPF: u32 = 70u;
const FLS_SIZE: u32 = 71u;
const FLS_BIAS: u32 = 72u;
// A mip-0 moment texel's angle at a face's centre (rad: 2 / 128, the cube's 128 texels a face; omni_mom.rs MOM) and the last mip.
const FLS_TEX0: f32 = 0.015625;
const FLS_LODMAX: f32 = 4.0;
// The search's blocked share under which the filter fades out (a blocker only at the search cone's rim: no step where it starts).
const FLS_EDGE: f32 = 0.05;

// Fire light v2 stage 4 (lane dlit3, sunshadow/omni_mom.rs): the drawn maps' prefiltered moment cubes, cube k = map k, 128 texels a
// face, 5 mips; per texel the quantised mean 4 moments of z = the radial distance from the map's origin over [near, far] (rows
// FLS_MAPN / FLS_MAPS .w), read with the metal look's linear sampler (trilinear, seamless across the faces).
@group(0) @binding(10) var fls_mom: texture_cube_array<f32>;
const FLS_MOFS: f32 = 0.035955884801;
// The inverse of Peters & Klein 2015's quantisation transform (rows; WGSL columns, so FLS_MOMG * (o - offset)).
const FLS_MOMG: mat4x4<f32> = mat4x4<f32>(
	vec4<f32>(0.2227744146, 0.1549679261, 0.1451988946, 0.163127443),
	vec4<f32>(0.0771972861, 0.1394629426, 0.2120202157, 0.2591432266),
	vec4<f32>(0.7926986636, 0.7963415838, 0.7258694464, 0.6539092497),
	vec4<f32>(0.0319417555, -0.1722823173, -0.2758014811, -0.3376131734));

// The moments (z, z^2, z^3, z^4) of a stored moment texel (twin of omni_mom.rs `unoptimized`).
fn fls_moments(o: vec4<f32>) -> vec4<f32> {
	return FLS_MOMG * (o - vec4<f32>(FLS_MOFS, 0.0, 0.0, 0.0));
}

// Hamburger 4MSM (Peters & Klein 2015, twin of sunshadow/omni_msm.rs `hamburger`): of the depths whose moments are `b0` (pulled
// towards (0.5, 0.5, 0.5, 0.5) by the moment bias `mb`), the share nearer than `zf` (x) and that share x their mean depth (y), from
// the canonical 3-point representation through zf: supports zf, z2, z3 = the roots of the polynomial that solves the Hankel system
// (Cholesky), weights from the Vandermonde system on (1, b1, b2).
fn fls_ham(b0: vec4<f32>, zf: f32, mb: f32) -> vec2<f32> {
	let b = mix(b0, vec4<f32>(0.5), mb);
	let d22 = max(b.y - b.x * b.x, 1e-12);
	let l32d22 = b.z - b.x * b.y;
	let l32 = l32d22 / d22;
	let d33 = max(b.w - b.y * b.y - l32d22 * l32, 1e-14);
	let c1a = zf - b.x;
	let c2 = (zf * zf - b.y - l32 * c1a) / d33;
	let c1 = c1a / d22 - l32 * c2;
	let c0 = 1.0 - b.x * c1 - b.y * c2;
	let ic2 = 1.0 / select(c2, 1e-12, abs(c2) < 1e-12);
	let p = c1 * ic2;
	let r = sqrt(max(0.25 * p * p - c0 * ic2, 0.0));
	let z2 = -0.5 * p - r;
	let z3 = -0.5 * p + r;
	// Weight of support xi among {xi, xj, xk}: (b2 - b1 (xj + xk) + xj xk) / ((xi - xj) (xi - xk)).
	let dn2 = (z2 - zf) * (z2 - z3);
	let w2 = (b.y - b.x * (zf + z3) + zf * z3) / select(dn2, 1e-12, abs(dn2) < 1e-12);
	let wf = (b.y - b.x * (z2 + z3) + z2 * z3) / max(1e-12, (zf - z2) * (zf - z3));
	if (z3 < zf) {
		let s = clamp(1.0 - wf, 0.0, 1.0);
		let w3 = s - w2;
		return vec2<f32>(s, clamp(w2 * z2 + w3 * z3, 0.0, s * zf));
	}
	if (z2 < zf) {
		let s = clamp(w2, 0.0, 1.0);
		return vec2<f32>(s, s * clamp(z2, 0.0, zf));
	}
	return vec2<f32>(0.0);
}

// One receiver against map `k`: v = its point from the map's origin (normal offset applied), dist = |v|, vh = v / dist, e1 / e2 =
// the tangent axes across vh, nn = its surface normal turned to the origin, near / span = the map's z range (m).
struct FlsRecv {
	v: vec3<f32>,
	vh: vec3<f32>,
	e1: vec3<f32>,
	e2: vec3<f32>,
	nn: vec3<f32>,
	dist: f32,
	near: f32,
	span: f32,
}

// The 4 taps on the cone of half-angle `phi` round the receiver's direction: (sum of blocked shares, sum of share x blocker z) / 4.
// Tap i at (+-1, +-1) x phi / 2 across vh (radius 0.71 phi: the 4-point rule of a disc), at the mip whose texel is phi; its compare
// depth is the receiver's plane in the tap's direction (the receiver's own distance where the direction misses the plane, at most
// 4 x it), less the depth bias and the slope bias of a texel's spread on that plane.
fn fls_cone(k: u32, r: FlsRecv, phi: f32) -> vec2<f32> {
	let q = u.flit[FLS_MAPQ];
	let b = u.flit[FLS_BIAS];
	let lod = clamp(log2(max(phi, 1e-6) / FLS_TEX0) + q.x, 0.0, FLS_LODMAX);
	let texa = FLS_TEX0 * exp2(lod);
	let h = tan(0.5 * phi);
	let pv = dot(r.nn, r.v);
	var acc = vec2<f32>(0.0);
	for (var i = 0u; i < 4u; i++) {
		let sx = select(-1.0, 1.0, (i & 1u) != 0u);
		let sy = select(-1.0, 1.0, (i & 2u) != 0u);
		let d = normalize(r.vh + (r.e1 * sx + r.e2 * sy) * h);
		let c = dot(r.nn, d);
		var t = r.dist;
		if (c < -1e-3) { t = min(pv / c, 4.0 * r.dist); }
		let ca = max(abs(c), 0.1);
		let slope = b.z * texa * t * sqrt(max(1.0 - ca * ca, 0.0)) / ca;
		let zf = (t - b.y - slope - r.near) / r.span;
		let m = fls_moments(textureSampleLevel(fls_mom, metal_samp, d, i32(k), lod));
		acc += fls_ham(m, zf, b.w);
	}
	return acc * 0.25;
}

// The lit fraction (0..1) of world point `w` (unit normal `n` when `has_n`) by shadow map `k`.
fn flit_shadow(k: u32, w: vec3<f32>, n: vec3<f32>, has_n: bool) -> f32 {
	let m = u.flit[FLS_MAPS + k];
	let q = u.flit[FLS_MAPQ];
	let fade = u.flit[FLS_MAPF][k];
	if (m.w <= 0.0 || fade <= 0.0 || q.z <= 0.0) { return 1.0; }
	let size = u.flit[FLS_SIZE][k] * q.y;
	let b = u.flit[FLS_BIAS];
	var v = w - m.xyz;
	let dist0 = max(length(v), 1e-3);
	var nn = -v / dist0;
	if (has_n) { nn = select(-n, n, dot(n, v) < 0.0); }
	// The receiver moves b.x mip-0 texels off its surface towards the light's side.
	v += nn * (b.x * FLS_TEX0 * dist0);
	let dist = max(length(v), 1e-3);
	let near = max(u.flit[FLS_MAPN][k], FLS_NEAR);
	let far = max(m.w, 2.0 * near);
	// Past far: lit; over the last FLS_TAIL of far the shadow fades to lit (no edge where the map ends).
	let tail = clamp((far - dist) / (FLS_TAIL * far), 0.0, 1.0);
	if (tail <= 0.0) { return 1.0; }
	let vh = v / dist;
	// Tangent axes across vh (the reference axis switches where |vh.x| = 0.71; the 4 taps overlap, so the turn does not show).
	let refa = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 0.0, 1.0), abs(vh.x) > 0.7071);
	let e1 = normalize(cross(refa, vh));
	let r = FlsRecv(v, vh, e1, cross(vh, e1), nn, dist, near, far - near);
	// 1: the blocker search over the emitter's cone seen from the receiver.
	let cs = clamp(atan(size / dist), 0.5 * FLS_TEX0, q.z);
	let bs = fls_cone(k, r, cs);
	if (bs.x <= 0.0) { return 1.0; }
	// 2: the penumbra cone seen from the origin from the mean blocker distance, 3: the filter on it.
	let db = near + (bs.y / bs.x) * (far - near);
	let phi = clamp(size * (1.0 / max(db, near) - 1.0 / dist), 0.5 * FLS_TEX0, q.z);
	let lit = 1.0 - fls_cone(k, r, phi).x;
	// Light-bleeding reduction (q.w: the last q.w of the lit share counts as shadow).
	let lr = clamp((lit - q.w) / max(1.0 - q.w, 1e-3), 0.0, 1.0);
	let soft = mix(1.0, lr, clamp(bs.x / FLS_EDGE, 0.0, 1.0));
	return mix(1.0, soft, fade * tail);
}

// flit_debug=5 (stage 4): map 0's moment cube at mip 10 x (flit_debug - 5) (5.2 = mip 2): grey = the mean occluder depth z (0 near,
// 1 far / nothing) in this point's direction from the map's origin, red = the point lies behind it (its own z > mean + 0.02).
fn flit_debug_mom(w: vec3<f32>) -> vec3<f32> {
	let none = vec3<f32>(0.0, 0.0, 0.1);
	if (!FLS_ON) { return none; }
	let m = u.flit[FLS_MAPS];
	if (m.w <= 0.0) { return none; }
	let near = max(u.flit[FLS_MAPN].x, FLS_NEAR);
	let far = max(m.w, 2.0 * near);
	let v = w - m.xyz;
	let lod = clamp(round((u.flit[FLIT_SHAPE].y - 5.0) * 10.0), 0.0, 4.0);
	let b = fls_moments(textureSampleLevel(fls_mom, metal_samp, v, 0, lod));
	let zr = clamp((length(v) - near) / (far - near), 0.0, 1.0);
	let behind = select(0.0, 1.0, zr > b.x + 0.02);
	return vec3<f32>(b.x) * (1.0 - 0.6 * behind) + vec3<f32>(0.6 * behind, 0.0, 0.0);
}
