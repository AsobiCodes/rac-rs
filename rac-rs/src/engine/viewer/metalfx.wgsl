// [PC] Graphics -> Effects -> Metal (metalfx.rs, metalenv.rs): metal shading for the textures the game pack marks. Only with
// Lighting Enhanced and Metal On (u.keep_tail.y > 0); otherwise every caller returns the retail colour untouched.
//
// The retail texel x its baked / vertex lighting is the base: it already holds the light the face gets, so the metal keeps that
// colour and brightness and shows what it reflects RELATIVE to it (`metal_shade`): the reflected light over the light around the
// face's normal. The reflected light is:
// - the level's sky, captured into six small cube faces with a mip chain (metalenv.rs; rougher kinds read blurrier levels), with
//   the ground below the horizon taken from the level's ambient and fog colours;
// - the scene itself where the reflected ray meets it in the previous frame's colour and depth (screen-space reflection, a
//   bounded march, only on metal pixels and only for the smoother kinds; falls back to the sky).
// A plain sky leaves the metal at its retail level, a darker object reflected in it darkens it, a brighter one lifts it (at most
// `MK_RMAX`); rough metal stays at the retail picture; toward grazing angles the smooth part reflects the surroundings themselves
// (Fresnel). On top: GGX highlights of the draw's key light pair and the retail dynamic lights (points and the posed moby's third light),
// coloured like the metal (gold glints golden).
// Kinds (bits 2..3 of `i.metal`): 0 polished, 1 mirror chrome, 2 brushed / worn, 3 painted metal; bit 1 = equipment (gentler).
// u.metal: [0] (reflection, dielectric F0, reflected-colour share, highlight), [1] roughness per kind, [2] (equipment, env ready,
// SSR ready, SSR cells), [3] (SSR distance, SSR thickness, highlight roughness floor, dynamic-light highlight), [4..8] the previous
// frame's view-projection (SSR), [8] metalness per kind, [9] (sky gain, SSR edge fade, SSR roughness cut, ground gain), [10] (strength
// looking down, strength at grazing angles, own-colour tint, texture break-up), [11] (sky blur in mips, sky occlusion, glossy cone per
// roughness, least reflection blur in mips), [12] the planar floor (metalplanar.rs), [13] contact reflections (near, far, blur in mips
// per unit of reflection distance: metalcontact.rs).
@group(0) @binding(4) var metal_env: texture_2d_array<f32>;
@group(0) @binding(5) var metal_samp: sampler;
@group(0) @binding(6) var metal_ssr_c: texture_2d<f32>;
@group(0) @binding(7) var metal_ssr_d: texture_2d<f32>;
// Non-filtering sampler for the sun map's 4-texel gather (`skyvis_sun_gap`; a depth texture takes no filtering sampler).
@group(0) @binding(8) var metal_nsamp: sampler;
// The planar floor reflection (metalplanar.rs): this frame's level mirrored about u.metal[12].y, with a short mip chain; its alpha is
// the contact weight by reflection distance, floor point to reflected point (metalcontact.rs: 1 up to u.metal[13].x, 0 from
// u.metal[13].y or where nothing was drawn); its colour is premultiplied by that weight.
@group(0) @binding(9) var metal_pl: texture_2d<f32>;

// Metal On / Off pipeline sets (metalpipe.rs): the Off set is built with this false, so Metal Off and Lighting Classic run the
// level shader without the metal code at all (the same picture as with it: `metal_look` returns at its first line either way).
override MK_ON: bool = true;

const MK_MIPS: f32 = 4.0;
// The most a reflection may lift a metal face over its own lit colour (a bright sky patch or lamp in a polished plate).
const MK_RMAX: f32 = 2.0;
// Under a roof a reflection lifts a face at most 1 / this over its surroundings (no glow of a whole dark floor).
const MK_INDOOR_LIFT: f32 = 0.7;
// Share of a met object's own light that a reflection under a roof adds on top of the relative look (`metal_look`).
const MK_POP: f32 = 1.2;
// The most of a screen hit's own light a metal pixel adds (its share in `env` plus the MK_POP gloss): under 1, so metal facing metal
// converges frame to frame instead of running away (the hit is read from the previous frame).
const MK_HIT_MAX: f32 = 0.7;
// Mip of the reflection source read as the light around a metal pixel (`metal_local`) and as the scene along a ray (`metal_vanish`).
const MK_LOCAL_LOD: f32 = 4.0;
const MK_VANISH_LOD: f32 = 3.0;
// The sky in the reflection depth (post.rs `fs_lin`: the clear depth reads as the far plane, retail's at 728 m or further).
const MK_SKY_Z: f32 = 700.0;
// Radius of the sky test's footprint (`skyvis_cone`, screen heights): x + y * roughness; the blocked share up to which a blocker does
// not count (a thin far post or rail), and from 1 - MK_SKY_THIN it blocks fully (a wall, a ceiling).
const MK_SKY_CONE: vec2<f32> = vec2<f32>(0.03, 0.25);
const MK_SKY_THIN: f32 = 0.25;

fn metal_lum(c: vec3<f32>) -> f32 {
	return dot(c, vec3<f32>(0.2126, 0.7152, 0.0722));
}

// One captured sky face `k` (faces as volclouds/skymodel.rs `FACES` and sky.rs `cube`) along unit direction d at mip `lod`; a direction
// outside the face reads its edge.
fn metal_face(d: vec3<f32>, k: i32, lod: f32) -> vec3<f32> {
	var fw = vec3<f32>(1.0, 0.0, 0.0);
	var rt = vec3<f32>(0.0, -1.0, 0.0);
	var up = vec3<f32>(0.0, 0.0, 1.0);
	switch k {
		case 1: { fw = vec3<f32>(-1.0, 0.0, 0.0); rt = vec3<f32>(0.0, 1.0, 0.0); }
		case 2: { fw = vec3<f32>(0.0, 1.0, 0.0); rt = vec3<f32>(1.0, 0.0, 0.0); }
		case 3: { fw = vec3<f32>(0.0, -1.0, 0.0); rt = vec3<f32>(-1.0, 0.0, 0.0); }
		case 4: { fw = vec3<f32>(0.0, 0.0, 1.0); rt = vec3<f32>(-1.0, 0.0, 0.0); up = vec3<f32>(0.0, 1.0, 0.0); }
		case 5: { fw = vec3<f32>(0.0, 0.0, -1.0); rt = vec3<f32>(1.0, 0.0, 0.0); up = vec3<f32>(0.0, 1.0, 0.0); }
		default: {}
	}
	let z = max(dot(d, fw), 1e-4);
	let n = clamp(vec2<f32>(dot(d, rt) / z, dot(d, up) / z), vec2<f32>(-1.0), vec2<f32>(1.0));
	return textureSampleLevel(metal_env, metal_samp, vec2<f32>(n.x * 0.5 + 0.5, 0.5 - n.y * 0.5), k, lod).rgb;
}

// Face of axis `i` (0 x, 1 y, 2 z) on the side of d.
fn metal_face_of(d: vec3<f32>, i: i32) -> i32 {
	let c = select(select(d.z, d.y, i == 1), d.x, i == 0);
	return 2 * i + select(1, 0, c > 0.0);
}

// The captured sky along unit direction d at mip `lod`. Each face is filtered and mipped on its own, so near a face boundary
// (|d| of the two largest axes close) the two faces are blended (equal at the boundary): no hard seam line on the floor.
fn metal_sky(d: vec3<f32>, lod: f32) -> vec3<f32> {
	let a = abs(d);
	var i = 2;
	var j = select(1, 0, a.x >= a.y);
	if (a.x >= a.y && a.x >= a.z) { i = 0; j = select(2, 1, a.y >= a.z); } else if (a.y >= a.z) { i = 1; j = select(2, 0, a.x >= a.z); }
	let ai = max(max(a.x, a.y), a.z);
	let aj = select(select(a.z, a.y, j == 1), a.x, j == 0);
	let w = 0.5 - 0.5 * smoothstep(0.0, 0.15, (ai - aj) / max(ai, 1e-4));
	let ci = metal_face(d, metal_face_of(d, i), lod);
	if (w <= 0.0) { return ci; }
	return mix(ci, metal_face(d, metal_face_of(d, j), lod), w);
}

// A reflection of the sky is far away, so it is never sharp (agent 18: Blarg's hangar printed the window's edge as long sharp bands
// down the painted floor). The sky's colour (`metal_sky_lobe`) and whether it is open (`metal_sky_vis`) are both read over a cone around
// r of half-angle MK_SKY_LOBE.x + MK_SKY_LOBE.y * roughness (radians; painted 0.38 = ~20 degrees): four directions half that off r on
// two crossed axes (`metal_lobe_axes`), each sky read at a mip at least as wide as their spacing (MK_FACE_TEXEL = one mip-0 texel of a
// 64-texel face, metalenv.rs FACE). A face seam, a thin bright thing in the sky box or a window edge becomes a soft gradient.
const MK_SKY_LOBE: vec2<f32> = vec2<f32>(0.1, 0.6);
const MK_FACE_TEXEL: f32 = 0.0245;
// The two offsets (perpendicular to unit r and to each other, length half the lobe angle) of the lobe's four directions r +- a, r +- b.
fn metal_lobe_axes(r: vec3<f32>, rough: f32) -> mat2x3<f32> {
	let h = 0.5 * (MK_SKY_LOBE.x + MK_SKY_LOBE.y * rough);
	let ax = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(r.z) > 0.9);
	let a = normalize(cross(r, ax)) * h;
	return mat2x3<f32>(a, cross(r, a));
}
fn metal_sky_lobe(r: vec3<f32>, rough: f32, lod: f32) -> vec3<f32> {
	let t = metal_lobe_axes(r, rough);
	let l = clamp(max(lod, log2(length(t[0]) / MK_FACE_TEXEL)), 0.0, MK_MIPS);
	let s = metal_sky(normalize(r + t[0]), l) + metal_sky(normalize(r - t[0]), l) + metal_sky(normalize(r + t[1]), l) + metal_sky(normalize(r - t[1]), l);
	return s * 0.25;
}

// The level's ground light below the horizon (and the occluded light with no reflection source yet), gamma.
fn metal_ground() -> vec3<f32> {
	return (u.ambient[0].rgb * 0.6 + u.fog_colour.rgb * 0.4) * u.metal[9].w;
}

// What the reflected direction r sees with no screen hit (linear light): the sky above, the level's ground light below.
// Glossy: `blur` more mip levels (u.metal[11].x for a reflection, read over its cone `metal_sky_lobe`; 0 for the blurred
// surroundings, one direction).
fn metal_world(r: vec3<f32>, rough: f32, blur: f32) -> vec3<f32> {
	let ground = metal_ground();
	var sky = ground;
	if (u.metal[2].y > 0.5) {
		let lod = min(rough * MK_MIPS + blur, MK_MIPS);
		if (blur > 0.0) { sky = metal_sky_lobe(r, rough, lod); } else { sky = metal_sky(r, lod); }
		sky *= u.metal[9].x;
	}
	// 1 - smoothstep(lo, hi, z) = smoothstep(hi, lo, z), written with edge0 < edge1 (reversed edges are undefined in SPIR-V/HLSL).
	let below = 1.0 - smoothstep(-0.30 - rough * 0.3, 0.02, r.z);
	let c = mix(sky, ground, below);
	return c * c;
}

// The depth source's own error at view depth z: each texel holds the nearest of its 2x2 block (post.rs `fs_lin`) at a lower size, so a
// surface seen at a slant reads nearer than it is by a share of z. A ray must be further behind than that to count, or a floor seen
// at a slant "hits" itself just above the reflecting pixel and mirrors itself (streaks dragged down the screen).
fn metal_ssr_bias(z: f32) -> f32 {
	return 0.02 + 0.01 * z;
}

// Previous-frame screen uv (xy) and view depth (z) of world point q.
fn metal_ssr_at(vp: mat4x4<f32>, q: vec3<f32>) -> vec3<f32> {
	let c = vp * vec4<f32>(q, 1.0);
	let ndc = c.xy / max(abs(c.w), 1e-4);
	return vec3<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, c.w);
}

fn metal_ssr_depth(uv: vec2<f32>, dim: vec2<f32>) -> f32 {
	return textureLoad(metal_ssr_d, vec2<i32>(clamp(uv * dim, vec2<f32>(0.0), dim - 1.0)), 0).r;
}

// The depth step to the next source texel where the surface is slanted (per axis the smaller side, only where the depth runs one way
// across the texel; the larger axis). Each texel holds the nearest of its 2x2 block, so a slanted surface reads nearer than it is by
// up to one step: the far floor near the horizon, where `metal_ssr_bias` is too small. A thin object nearer than both neighbours
// (a pole) has no step on that axis.
fn metal_ssr_slope(uv: vec2<f32>, dim: vec2<f32>, z: f32) -> f32 {
	let dx = vec2<f32>(1.0 / dim.x, 0.0);
	let dy = vec2<f32>(0.0, 1.0 / dim.y);
	let ax = z - metal_ssr_depth(uv - dx, dim);
	let bx = metal_ssr_depth(uv + dx, dim) - z;
	let ay = z - metal_ssr_depth(uv - dy, dim);
	let by = metal_ssr_depth(uv + dy, dim) - z;
	return max(select(0.0, min(abs(ax), abs(bx)), ax * bx > 0.0), select(0.0, min(abs(ay), abs(by)), ay * by > 0.0));
}

// The top level of the reflection depth's nearest-depth chain (metalenv.rs `HIZ` - 1): level l holds the nearest of each 2^l x 2^l
// block of source texels (half the render size: the Deck's 640 x 400 down to 40 x 25).
const MK_HIZ_TOP: i32 = 4;

// Screen share at which the ray p0 + d s (source texels; `inv` = 1 / d with no zero, `side` 1 where d runs positive) leaves the
// level-`l` cell holding its point q.
fn metal_ssr_exit(p0: vec2<f32>, inv: vec2<f32>, side: vec2<f32>, q: vec2<f32>, l: i32) -> f32 {
	let cs = f32(1u << u32(l));
	let t = ((floor(q / cs) + side) * cs - p0) * inv;
	return min(t.x, t.y);
}

// Screen-space reflection in the previous frame (metalenv.rs `capture_ssr`): (linear colour, confidence 0..1). The ray (`reach` world
// units, cut at the near plane) is walked ON SCREEN through EVERY source texel it crosses, in order, without samples that could step
// over something: a cell of the nearest-depth chain that the ray stays in front of in its whole crossing (its deepest point there is
// not behind the cell's nearest depth plus the depth error) is skipped whole and the walk climbs a level; otherwise it descends, down
// to single texels. So the result is the FIRST surface the ray meets, the same for neighbouring pixels (no per-pixel jitter: no
// dithered edges or grain, nothing that changes between frames for frame smoothing to blend). A texel is met where the ray crosses
// the surface's depth (plus the depth error and the slanted-floor step `metal_ssr_slope`), or where it enters the texel at most an
// eighth of the thickness behind it (an object's side; more lets rays passing just behind a pole or a lamp head count); a ray
// passing UNDER an overhang or behind a thin object is no hit and walks on, so a reflection is the mirror image of what the ray
// meets, never a smear of what it passed behind. At most `u.metal[2].w` cells (a cost
// knob; a miss falls back to `metal_miss`). A hit where the ray crosses more depth inside the met texel than the thickness (a ray running
// almost along the view: one pole texel met over a long stretch of the ray, drawn as a long streak) fades out (`stretch`). Fades toward the screen edge, with the ray's length, near the side-entry limit (metalfx.rs
// `march_hiz` twin) and across the outer 2 texels of what it meets (`metal_ssr_cover`: round edges, not the source's stair steps).
// Glossy, not a mirror: the colour is read from the source's mip chain, blurrier the further the hit is from the surface (`rough` x
// the cone knob u.metal[11].z: the contact stays crisp), at least u.metal[11].w mips.
fn metal_ssr(p: vec3<f32>, r: vec3<f32>, rough: f32) -> vec4<f32> {
	if (u.metal[2].z < 0.5 || u.keep_tail.x > 0.0) { return vec4<f32>(0.0); }
	let iters = clamp(u.metal[2].w, 8.0, 128.0);
	let reach = max(u.metal[3].x, 0.5);
	let thick = max(u.metal[3].y, 0.02);
	let vp = mat4x4<f32>(u.metal[4], u.metal[5], u.metal[6], u.metal[7]);
	let dim = vec2<f32>(textureDimensions(metal_ssr_d));
	let a = metal_ssr_at(vp, p);
	if (a.z <= 0.1) { return vec4<f32>(0.0); }
	var tr = reach;
	let we = (vp * vec4<f32>(p + r * reach, 1.0)).w;
	if (we < 0.1) { tr = reach * (a.z - 0.1) / max(a.z - we, 1e-4); }
	let b = metal_ssr_at(vp, p + r * tr);
	let p0 = a.xy * dim;
	let d = (b.xy - a.xy) * dim;
	let len = max(abs(d.x), abs(d.y));
	if (len < 1.0) { return vec4<f32>(0.0); }
	let dd = select(d, vec2<f32>(1e-6), abs(d) < vec2<f32>(1e-6));
	let inv = vec2<f32>(1.0) / dd;
	let side = select(vec2<f32>(0.0), vec2<f32>(1.0), dd > vec2<f32>(0.0));
	let eps = 0.01 / len;
	let ia = 1.0 / a.z;
	let ib = 1.0 / b.z;
	var lvl = 0;
	// Past the reflecting pixel's own texel.
	var s = metal_ssr_exit(p0, inv, side, p0, 0) + eps;
	for (var k = 0; k < 128; k++) {
		if (f32(k) >= iters || s >= 1.0) { break; }
		let q = p0 + d * s;
		if (any(q < vec2<f32>(0.0)) || any(q >= dim)) { break; }
		let se = min(metal_ssr_exit(p0, inv, side, q, lvl), 1.0);
		let cell = vec2<i32>(floor(q / f32(1u << u32(lvl))));
		let w0 = 1.0 / mix(ia, ib, s);
		let w1 = 1.0 / mix(ia, ib, se);
		let z = textureLoad(metal_ssr_d, cell, lvl).r;
		if (max(w0, w1) <= z + metal_ssr_bias(z)) { s = se + eps; lvl = min(lvl + 1, MK_HIZ_TOP); continue; }
		if (lvl > 0) { lvl -= 1; continue; }
		let e = metal_ssr_bias(z) + metal_ssr_slope((vec2<f32>(cell) + 0.5) / dim, dim, z);
		let lim = 0.125 * thick + e;
		var sh = -1.0;
		if (w0 > z + e) { if (w0 - z <= lim) { sh = s; } } else if (w1 > z + e) { sh = clamp((1.0 / (z + e) - ia) / (ib - ia), s, se); }
		if (sh >= 0.0) {
			let h = vec3<f32>(mix(a.xy, b.xy, sh), 1.0 / mix(ia, ib, sh));
			let t = select(tr * sh, tr * (h.z - a.z) / (b.z - a.z), abs(b.z - a.z) > 1e-4) / reach;
			let edge = min(min(h.x, 1.0 - h.x), min(h.y, 1.0 - h.y));
			let stretch = 1.0 - smoothstep(1.0, 3.0, abs(w1 - w0) / thick);
			// Contact: painted / rough metal reflects only what is near it (u.metal[13]: full up to near, none from far along the ray;
			// a tall post's long stretch of hits beyond that is no reflection).
			let contact = 1.0 - smoothstep(u.metal[13].x, max(u.metal[13].y, u.metal[13].x + 1e-3), t * reach);
			let fade = smoothstep(0.0, max(u.metal[9].y, 1e-3), edge) * (1.0 - t * t) * (1.0 - smoothstep(0.3, 1.0, (h.z - z - e) / max(lim - e, 1e-4))) * stretch * contact;
			let cov = metal_ssr_cover(h.xy, dim, z, thick + e);
			let py = length(vec3<f32>(vp[0].y, vp[1].y, vp[2].y));
			let rad = t * reach * rough * u.metal[11].z * py / max(h.z, 0.1) * 0.5 * dim.y;
			let lod = clamp(log2(1.0 + 2.0 * rad) + u.metal[11].w, 0.0, f32(textureNumLevels(metal_ssr_c) - 1u));
			let c = textureSampleLevel(metal_ssr_c, metal_samp, h.xy, lod).rgb;
			return vec4<f32>(c * c, fade * smoothstep(0.5, 1.0, cov));
		}
		s = se + eps;
	}
	return vec4<f32>(0.0);
}

// Share of a 4 x 4 tent footprint (radius 2 texels) at source point uv (texels) that belongs to the met surface (the texels within
// `tol` of its depth z): 1 inside it, falling across the outer 2 texels of its silhouette (soft, no half-res stair steps).
fn metal_ssr_cover(uv: vec2<f32>, dim: vec2<f32>, z: f32, tol: f32) -> f32 {
	let f = uv * dim - 0.5;
	let b = floor(f) - 1.0;
	let fr = f - floor(f);
	var cov = 0.0;
	for (var k = 0; k < 16; k++) {
		let o = vec2<f32>(f32(k & 3), f32(k >> 2));
		let zt = textureLoad(metal_ssr_d, vec2<i32>(clamp(b + o, vec2<f32>(0.0), dim - 1.0)), 0).r;
		let w = (vec2<f32>(2.0) - abs(o - 1.0 - fr)) * 0.25;
		cov += select(0.0, w.x * w.y, abs(zt - z) <= tol);
	}
	return cov;
}

// How much the planar floor reflection (metalplanar.rs) covers a metal pixel at p with normal n (toward the eye): on the mirrored plane
// (height within MK_PL_TOL, fading out to three times that), facing up, times u.metal[12].x (the 0.2 s fade-in after a floor change,
// metalplanar.rs `fade`). 0 without a planar pass this frame or in the mirror / bed passes.
const MK_PL_TOL: f32 = 0.06;
fn metal_planar_w(p: vec3<f32>, n: vec3<f32>) -> f32 {
	if (u.metal[12].x <= 0.0 || u.keep_tail.x > 0.0) { return 0.0; }
	let dz = abs(p.z - u.metal[12].y);
	return min(u.metal[12].x, 1.0) * (1.0 - smoothstep(MK_PL_TOL, 3.0 * MK_PL_TOL, dz)) * smoothstep(0.9, 0.97, n.z);
}

// The planar floor reflection at floor point p: (linear light, contact weight). The mirrored level drawn at the same screen point (a
// point on the plane is its own mirror image), blurred by roughness (u.metal[12].w mips per unit; four taps a texel of that level
// apart: no blocks) and where it fades by reflection distance (up to u.metal[13].z mips per unit of far - near, by the level-0 weight:
// a near object keeps the roughness blur alone). Contact: the stored weight 1 - smoothstep(near, far, d) (u.metal[13]), the colour
// premultiplied by it (metalcontact.rs; divided back out here): only what is near the floor point is mirrored; the far window bars /
// rail posts of Blarg's hangar stand on the floor yet are metres away (user v13: radial bands, streaks under every post shimmering in
// motion), while Ratchet's own reflection, blurred with the faded background, keeps his colour (agent 20).
fn metal_planar(p: vec3<f32>, rough: f32) -> vec4<f32> {
	let c = u.view_proj * vec4<f32>(p, 1.0);
	let uv = vec2<f32>(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
	let top = f32(textureNumLevels(metal_pl) - 1u);
	let l0 = clamp(rough * u.metal[12].w, 0.0, top);
	let cw = textureSampleLevel(metal_pl, metal_samp, uv, 0.0).a;
	let lod = clamp(l0 + (1.0 - cw) * max(u.metal[13].y - u.metal[13].x, 0.0) * u.metal[13].z, 0.0, top);
	let o = exp2(lod) * 0.5 / vec2<f32>(textureDimensions(metal_pl));
	var s = textureSampleLevel(metal_pl, metal_samp, uv + vec2<f32>(o.x, o.y), lod);
	s += textureSampleLevel(metal_pl, metal_samp, uv + vec2<f32>(-o.x, o.y), lod);
	s += textureSampleLevel(metal_pl, metal_samp, uv + vec2<f32>(o.x, -o.y), lod);
	s += textureSampleLevel(metal_pl, metal_samp, uv - o, lod);
	let g = s * 0.25;
	let col = min(g.rgb / max(g.a, 1e-3), vec3<f32>(1.0));
	return vec4<f32>(col * col, g.a);
}

// The reflection source (last frame's scene) at uv, blurred to mip `lod`, linear light.
fn metal_scene(uv: vec2<f32>, lod: f32) -> vec3<f32> {
	let c = textureSampleLevel(metal_ssr_c, metal_samp, uv, clamp(lod, 0.0, f32(textureNumLevels(metal_ssr_c) - 1u))).rgb;
	return c * c;
}

// The lit scene around world point p (the source's coarse mip at p, linear), w = 1 when there is a source.
fn metal_local(p: vec3<f32>) -> vec4<f32> {
	if (u.metal[2].z < 0.5 || u.keep_tail.x > 0.0) { return vec4<f32>(0.0); }
	let a = metal_ssr_at(mat4x4<f32>(u.metal[4], u.metal[5], u.metal[6], u.metal[7]), p);
	if (a.z <= 0.1 || any(a.xy < vec2<f32>(0.0)) || any(a.xy > vec2<f32>(1.0))) { return vec4<f32>(0.0); }
	return vec4<f32>(metal_scene(a.xy, MK_LOCAL_LOD), 1.0);
}

// What the camera saw last frame straight along direction r (the scene "at infinity" that way: a far wall, the sky through a
// window), blurred; w = how much to trust it: on screen, away from the edge, and only far things (a near object is not where a
// parallel ray from the metal meets it).
fn metal_vanish(r: vec3<f32>) -> vec4<f32> {
	if (u.metal[2].z < 0.5 || u.keep_tail.x > 0.0) { return vec4<f32>(0.0); }
	let c = mat4x4<f32>(u.metal[4], u.metal[5], u.metal[6], u.metal[7]) * vec4<f32>(r, 0.0);
	if (c.w <= 1e-3) { return vec4<f32>(0.0); }
	let uv = vec2<f32>(c.x / c.w * 0.5 + 0.5, 0.5 - c.y / c.w * 0.5);
	let edge = min(min(uv.x, 1.0 - uv.x), min(uv.y, 1.0 - uv.y));
	if (edge <= 0.0) { return vec4<f32>(0.0); }
	let dim = vec2<f32>(textureDimensions(metal_ssr_d));
	let w = smoothstep(0.0, 0.08, edge) * smoothstep(3.0, 12.0, metal_ssr_depth(uv, dim));
	return vec4<f32>(metal_scene(uv, MK_VANISH_LOD), w);
}

// skyvis.wgsl hook: world point q against the sun's shadow map (sunshadow.rs: the level's casters seen from its key light): (how far
// q lies behind the first surface the light meets there, world units; 1 = inside a cascade, 0 = outside every map or no map).
fn skyvis_sun_gap(q: vec3<f32>) -> vec2<f32> {
	if (u.sun.w <= 0.0) { return vec2<f32>(0.0); }
	let nc = i32(u.sun_q.z);
	for (var k = 0; k < nc; k++) {
		let m = u.sun_vp[k];
		let c = m * vec4<f32>(q, 1.0);
		if (max(abs(c.x), abs(c.y)) < 0.98 && c.z > 0.0 && c.z < 1.0) {
			let uv = vec2<f32>(c.x * 0.5 + 0.5, 0.5 - c.y * 0.5) * u.sun_q.y;
			// Bilinear depth of the 4 nearest texels: the sky / roof boundary follows the map smoothly (no texel stairs on the floor).
			let res = vec2<f32>(textureDimensions(sun_map));
			let f = uv * res - 0.5;
			let b = floor(f);
			let fr = f - b;
			// The texels b .. b + 1 in one gather at their shared corner (the footprint exactly, robust to rounding; the sampler's
			// clamp to edge = the clamped loads): x = (lo, hi), y = (hi, hi), z = (hi, lo), w = (lo, lo).
			let g = textureGather(sun_map, metal_nsamp, (b + 1.0) / res, k);
			let d = mix(mix(g.w, g.z, fr.x), mix(g.x, g.y, fr.x), fr.y);
			let scale = max(length(vec3<f32>(m[0].z, m[1].z, m[2].z)), 1e-6);
			return vec2<f32>((c.z - u.sun_bias[k] - d) / scale, 1.0);
		}
	}
	return vec2<f32>(0.0, 0.0);
}

// skyvis.wgsl hook: the reflection source's alpha is 1 where the camera saw the sky (metalenv.rs `fs_ssr`: depth at or past
// MK_SKY_Z) and its mip chain averages it, so a level read is the sky share over a block that size (the previous frame, as
// `u.metal[4..8]`). Four taps rad / 2 apart at the level whose texel is rad wide cover the footprint; past the screen edge the
// sampler clamps (what the edge saw counts). Only big blockers count: the share runs through smoothstep(MK_SKY_THIN, 1 -
// MK_SKY_THIN), so a thin far post (agent 16: one long dark band down Blarg's hangar floor) leaves the sky open, a wall or a
// ceiling closes it. CPU twin: metalfx.rs `tests::cone`.
fn skyvis_cone(uv: vec2<f32>, rad: f32) -> f32 {
	let dim = vec2<f32>(textureDimensions(metal_ssr_c));
	let lod = clamp(log2(max(rad * dim.y, 1.0)), 0.0, f32(textureNumLevels(metal_ssr_c) - 1u));
	let o = vec2<f32>(rad * dim.y / dim.x, rad) * 0.5;
	var s = textureSampleLevel(metal_ssr_c, metal_samp, uv + o, lod).a;
	s += textureSampleLevel(metal_ssr_c, metal_samp, uv + vec2<f32>(-o.x, o.y), lod).a;
	s += textureSampleLevel(metal_ssr_c, metal_samp, uv + vec2<f32>(o.x, -o.y), lod).a;
	s += textureSampleLevel(metal_ssr_c, metal_samp, uv - o, lod).a;
	return smoothstep(MK_SKY_THIN, 1.0 - MK_SKY_THIN, s * 0.25);
}

// Whether the sky is open along the reflected ray r from p (1) or a roof, walls or a cave block it (0): skyvis.wgsl's shadow-map
// test and what the camera saw that way last frame (levels without dynamic shadows have no shadow map: there the screen decides).
// The occlusion knob u.metal[11].y (0 = the sky everywhere, as v3). The mirror and bed passes (u.keep_tail.x > 0) have no screen
// of their own to test against: the sky everywhere there, as v3. The screen test reads a footprint around the ray's vanishing point
// that widens with the roughness (MK_SKY_CONE: a glossy lobe, not a thin ray), so only big blockers close the sky, averaged over
// the four lobe directions (MK_SKY_LOBE).
fn metal_sky_vis(p: vec3<f32>, r: vec3<f32>, rough: f32) -> f32 {
	let occl = clamp(u.metal[11].y, 0.0, 1.0);
	if (occl <= 0.0 || u.keep_tail.x > 0.0) { return 1.0; }
	var scr = vec2<f32>(1.0, 0.0);
	if (u.metal[2].z >= 0.5) {
		// Over the glossy lobe (`metal_lobe_axes`): the screen point where each direction runs out moves with it, so the edge of a far
		// wall or window is a gradient across the floor, not a line.
		let vp = mat4x4<f32>(u.metal[4], u.metal[5], u.metal[6], u.metal[7]);
		let rad = MK_SKY_CONE.x + MK_SKY_CONE.y * rough;
		let t = metal_lobe_axes(r, rough);
		scr = skyvis_screen(vp, p, normalize(r + t[0]), rad) + skyvis_screen(vp, p, normalize(r - t[0]), rad);
		scr = (scr + skyvis_screen(vp, p, normalize(r + t[1]), rad) + skyvis_screen(vp, p, normalize(r - t[1]), rad)) * 0.25;
	}
	return 1.0 - occl * (1.0 - skyvis_open(skyvis_sun(p, r), scr));
}

// What the reflected ray r from p sees where the screen walk met nothing: the sky (blurred by roughness and the sky-blur knob) where
// it is open, the lit scene around p where a roof or walls block it (`occ`), and what the camera saw that way when it saw something
// far (`metal_vanish`). Indoor metal never mirrors the sky or a planet through the walls.
fn metal_miss(r: vec3<f32>, rough: f32, vis: f32, occ: vec3<f32>) -> vec3<f32> {
	let sky = skyvis_mix(occ, metal_world(r, rough, u.metal[11].x), vis);
	let fv = metal_vanish(r);
	return mix(sky, fv.rgb, fv.a);
}

// The metal response (linear light) of a face whose lit retail colour is `base`, reflecting `env` along its mirror ray, where
// `around` is the blurred surroundings of its normal (the light the retail lighting in `base` stands for). Metal part: `base` times
// the reflection relative to `around` (a share `k.z` of the reflection's own colour on smooth metal), as far as the smoothness and
// `k.x` let it, then Fresnel toward the surroundings themselves at grazing angles. Glossy, not a mirror (`g` = u.metal[10]): the
// reflection's strength runs from `g.x` looking down to `g.y` at grazing angles (Schlick), and it takes the metal's own colour (share
// `g.z`; the reflected colour's own hue keeps the rest of `k.z`). Painted part: `base` with a thin dielectric reflection (F0 `k.y`).
// Rough metal (roughness 1) is `base` exactly. CPU twin: metalfx.rs `tests::shade`.
fn metal_shade(base: vec3<f32>, env: vec3<f32>, around: vec3<f32>, ndv: f32, rough: f32, metalness: f32, k: vec4<f32>, g: vec4<f32>) -> vec3<f32> {
	let s = 1.0 - clamp(rough, 0.0, 1.0);
	let le = metal_lum(env);
	let rl = clamp(le / max(metal_lum(around), 0.01), 0.0, MK_RMAX);
	let tint = clamp(g.z, 0.0, 1.0);
	let hue = mix(vec3<f32>(1.0), env / max(le, 1e-4), clamp(k.z * s * s * (1.0 - tint), 0.0, 1.0));
	let graze = pow(1.0 - clamp(ndv, 0.0, 1.0), 5.0);
	let fw = clamp(g.x + (g.y - g.x) * graze, 0.0, 1.0);
	let rel = base * mix(vec3<f32>(1.0), rl * hue, clamp(k.x * s * fw, 0.0, 1.0));
	// At most 1 per channel: a reflection is never brighter than what it reflects (no feedback through the next frame's source).
	let own = min(mix(vec3<f32>(1.0), clamp(base / max(metal_lum(base), 1e-3), vec3<f32>(0.0), vec3<f32>(2.0)), tint), vec3<f32>(1.0));
	let ew = metal_env_ws(ndv, rough, k, g);
	let metal = mix(rel, env * own, ew.x);
	let plain = mix(base, env, ew.y);
	return mix(plain, metal, clamp(metalness, 0.0, 1.0));
}

// The weights of `env` itself in `metal_shade`: (the metal part toward grazing angles, the painted part's thin coat).
fn metal_env_ws(ndv: f32, rough: f32, k: vec4<f32>, g: vec4<f32>) -> vec2<f32> {
	let s = 1.0 - clamp(rough, 0.0, 1.0);
	let graze = pow(1.0 - clamp(ndv, 0.0, 1.0), 5.0);
	return vec2<f32>(clamp(graze * s * s * k.x * g.y, 0.0, 1.0), k.y + (max(s, k.y) - k.y) * graze);
}

// The share of `env` in `metal_shade`'s result for this metalness (the screen hit counted at that share already).
fn metal_env_w(ndv: f32, rough: f32, metalness: f32, k: vec4<f32>, g: vec4<f32>) -> f32 {
	let w = metal_env_ws(ndv, rough, k, g);
	return mix(w.y, w.x, clamp(metalness, 0.0, 1.0));
}

// GGX highlight (D * V, times n.l) of one light along unit `l` at roughness a = rough^2.
fn metal_ggx(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, rough: f32) -> f32 {
	let ndl = dot(n, l);
	if (ndl <= 0.0) { return 0.0; }
	let h = normalize(l + v);
	let ndh = max(dot(n, h), 0.0);
	let ndv = max(dot(n, v), 1e-3);
	let a = max(rough * rough, 1e-3);
	let a2 = a * a;
	let q = ndh * ndh * (a2 - 1.0) + 1.0;
	let d = a2 / (3.14159265 * q * q);
	let kk = (rough + 1.0) * (rough + 1.0) / 8.0;
	let vis = 0.25 / ((ndl * (1.0 - kk) + kk) * (ndv * (1.0 - kk) + kk));
	return min(d * vis * ndl, 16.0);
}

// The highlights of the draw's key light pair and of the dynamic lights, linear light.
fn metal_lights(i: VOut, n: vec3<f32>, v: vec3<f32>, rough: f32) -> vec3<f32> {
	var s = 0u;
	if (i.c.a >= 0.0 && i.l.w >= 0.0) { s = min(u32(floor(i.l.w)) / 16u, 15u); }
	let lt = u.lights[s];
	var out = vec3<f32>(0.0);
	for (var k = 0u; k < 2u; k++) {
		let dir = lt[2u * k + 1u].xyz;
		let dd = dot(dir, dir);
		if (dd < 1e-8) { continue; }
		let col = lt[2u * k].rgb;
		out += col * col * metal_ggx(n, v, dir * inverseSqrt(dd), rough);
	}
	out *= u.metal[0].w;
	var dynl = vec3<f32>(0.0);
	// The posed moby's merged third light (`dyn_moby`): travel direction, so toward the light is its negative.
	let id = u32(max(i.c.a, 0.0) + 0.5);
	if (i.c.a >= 0.0 && id > 0u && id <= 128u) {
		let col = u.dynl[2u * (id - 1u)].rgb;
		dynl += col * metal_ggx(n, v, -u.dynl[2u * (id - 1u) + 1u].xyz, rough);
	}
	// The 8 point-light slots (`points_moby`), per pixel.
	for (var k = 0u; k < 8u; k++) {
		let a = u.points[2u * k];
		let b = u.points[2u * k + 1u];
		let d = distance(i.w, a.xyz);
		if (a.w <= 0.0 || d >= a.w || d < 0.0001) { continue; }
		dynl += b.rgb * (1.0 - b.w * d) * metal_ggx(n, v, (a.xyz - i.w) / d, rough);
	}
	return out + dynl * u.metal[3].w;
}

fn metal_look(i: VOut, rgb: vec3<f32>) -> vec3<f32> {
	if (!MK_ON) { return rgb; }
	if (u.keep_tail.y <= 0.0 || i.metal == 0u) { return rgb; }
	let kind = (i.metal >> 2u) & 3u;
	let rough = clamp(u.metal[1][kind], 0.02, 1.0);
	let metalness = clamp(u.metal[8][kind], 0.0, 1.0);
	let nn = dot(i.n, i.n);
	let eye = u.eye.xyz - i.w;
	let vv = dot(eye, eye);
	let v = select(vec3<f32>(0.0, 0.0, 1.0), eye * inverseSqrt(max(vv, 1e-8)), vv >= 1e-8);
	var n = select(v, i.n * inverseSqrt(max(nn, 1e-8)), nn >= 1e-8);
	// Two-sided: a face seen from behind shades with its normal toward the eye.
	if (dot(n, v) < 0.0) { n = -n; }
	let ndv = clamp(dot(n, v), 1e-3, 1.0);
	let base = rgb * rgb;
	let r = reflect(-v, n);
	// Where the sky is blocked (indoors, under a roof), the reflection and the surroundings it is measured against are the lit scene
	// around the pixel (no sky through the walls); the level's ground light before the first reflection source.
	let loc = metal_local(i.w);
	let gr = metal_ground();
	let occ = select(gr * gr, loc.rgb, loc.a > 0.5);
	let vis = metal_sky_vis(i.w, r, rough);
	let miss = metal_miss(r, rough, vis, occ);
	var env = miss;
	var ha = 0.0;
	var pop = vec3<f32>(0.0);
	if (rough < u.metal[9].z) {
		// On the planar floor (metalplanar.rs) the mirrored level replaces the screen walk: whole, stable reflections, no streaks; at its
		// edge the two blend (premultiplied). Its gloss under a roof counts at the share u.metal[12].z (the ceiling is always "met").
		let wp = metal_planar_w(i.w, n);
		var hit = vec4<f32>(0.0);
		// The walk fades out toward half planar weight and is off above it: in the band at the plane's edge, or while a new floor fades in,
		// its streaks (window posts in Blarg's hangar) never show over the mirror (agent 15).
		if (wp < 0.5) { hit = metal_ssr(i.w, r, rough); hit.a *= 1.0 - 2.0 * wp; }
		var popw = 1.0;
		if (wp > 0.0) {
			// Contact reflections (metalcontact.rs): the mirror counts by its contact weight (reflection distance), so the ceiling and far
			// posts are not reflected (the sky / surroundings `miss` shows there).
			let pl = metal_planar(i.w, rough);
			let wq = wp * pl.a;
			let a = hit.a * (1.0 - wp) + wq;
			hit = vec4<f32>((hit.rgb * hit.a * (1.0 - wp) + pl.rgb * wq) / max(a, 1e-4), a);
			popw = mix(1.0, clamp(u.metal[12].z, 0.0, 1.0), wp);
		}
		ha = hit.a * (1.0 - rough / max(u.metal[9].z, 1e-3));
		env = mix(env, hit.rgb, ha);
		pop = hit.rgb * (ha * (1.0 - vis) * popw);
	}
	// Under a roof the surroundings are what the camera saw along the normal (a floor: the ceiling) where it saw that, not the face's own
	// darker colour; the far scene a missed ray reads lifts them to MK_INDOOR_LIFT of its own light (else a dark floor under a bright
	// ceiling reads every miss as brighter than its light: a uniform wash). An object the screen walk met (the hero, a crate) is measured
	// against the unlifted surroundings, so it stands out of the floor as glossy metal shows it.
	let nv = metal_vanish(n);
	let oa = mix(occ, nv.rgb, nv.a);
	let lift = mix(max(1.0, MK_INDOOR_LIFT * metal_lum(miss) / max(metal_lum(oa), 1e-4)), 1.0, ha);
	let around = mix(oa * lift, metal_world(n, 1.0, 0.0), vis);
	// Texture break-up: a texel darker than the lit scene around it reflects less (share u.metal[10].w), so the pattern reads through.
	var k = u.metal[0];
	if (loc.a > 0.5) { k.x *= mix(1.0, clamp(metal_lum(base) / max(metal_lum(loc.rgb), 1e-3), 0.0, 1.0), clamp(u.metal[10].w, 0.0, 1.0)); }
	var lin = metal_shade(base, env, around, ndv, rough, metalness, k, u.metal[10]);
	// Gloss under a roof: what the screen walk met (the hero, a crate, a lamp) adds its own light at the reflection strength (share
	// MK_POP), so it reads on dark indoor metal too (against an open sky the relative look above already shows it). Together with its
	// share in `env` the hit's own light counts at most MK_HIT_MAX: the hit is the next frame's source, so a gain over 1 would run away
	// on facing metal (each frame brighter than the last).
	let fw = clamp(u.metal[10].x + (u.metal[10].y - u.metal[10].x) * pow(1.0 - ndv, 5.0), 0.0, 1.0);
	let pw = min(MK_POP * metalness * clamp(k.x * (1.0 - rough) * fw, 0.0, 1.0), max(MK_HIT_MAX - metal_env_w(ndv, rough, metalness, k, u.metal[10]), 0.0) / max(ha * (1.0 - vis), 1e-4));
	lin += pop * pw;
	// Highlights coloured like the metal (the base colour's hue, weaker on a darker texel); the retail light colours are scaled for
	// diffuse light, hence the 1 / pi.
	let lum = metal_lum(base);
	let tint = clamp(base / max(lum, 0.02), vec3<f32>(0.0), vec3<f32>(2.0));
	let f0 = mix(vec3<f32>(u.metal[0].y), tint * clamp(sqrt(lum) * 1.3 + 0.2, 0.3, 1.0), metalness);
	lin += f0 * metal_lights(i, n, v, max(rough, u.metal[3].z)) * 0.31830989;
	// Equipment (Clank, boots, held weapons) and the original chrome packets: a gentler step from the retail colour.
	let subdued = (i.metal & 2u) != 0u || (i.c.a >= 0.0 && i.l.w < -4.125 && i.l.w > -4.5);
	if (subdued) { lin = mix(base, lin, clamp(u.metal[2].x, 0.0, 1.0)); }
	return sqrt(max(lin, vec3<f32>(0.0)));
}
