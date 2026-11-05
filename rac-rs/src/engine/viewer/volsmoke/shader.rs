//! The WGSL of the volumetric smoke. It reuses the fog's blocks (volfog/scatter.rs: the uniform struct F with bindings 0..8,
//! `HELPERS`: noise tap, sun shadow visibility, view ray, phase function; volfog/temporal.rs: the reprojection resolve) and adds
//! the smoke's own: the volume list (binding 9), the extra uniform (binding 10), the per-pixel ellipsoid hit list merged into
//! segments (the march walks only those), the density (`smoke_at`: the hit volumes joined into one coverage and carved by one
//! world-anchored billow noise whose lumps grow with the puffs; thin volumes clump into denser billows with gaps), the light (sun
//! with the shadow map and a short march toward the sun, multi-scatter, a sky ambient that lights the puffs' tops, the warm glow of
//! nearby fire; the sun and sky tint mostly removed: grey smoke) and the depth-aware apply. The resolve pass (`fs_resolve`) is the fog's.

use super::super::volfog::{scatter, temporal};

/// Types, bindings, the end of a ray and the density of one volume.
pub const DEFS: &str = r#"
struct Vol {
	c: vec4<f32>,    // centre, radius
	col: vec4<f32>,  // albedo, extinction per world unit at the core
	glow: vec4<f32>, // fire glow colour, share drawn (the trail band x the range fade: scales the density after the clumping)
	mo: vec4<f32>,   // rise speed (the CPU's: flow.rs), billow, age, tall (vertical half-extent over the radius, >= 1)
	lp: vec4<f32>,   // the fire light's centre, reach (0 = none): the glow fades with the distance from it
};
struct G {
	tune: vec4<f32>,  // density gain, big noise (tile metres per metre of the hit volumes' mean radius), detail noise (metres per tile), 0 (not read: the rise gain is flow.rs')
	tune2: vec4<f32>, // ambient gain, sun gain, fire glow gain, shadow march length (metres)
	tune3: vec4<f32>, // 0 (not read: the noise moves by g.flow), volume count, shadow taps, share of the albedo moved toward a light neutral grey
	amb: vec4<f32>,   // ambient colour, erosion
	cut: vec4<f32>,   // fire cores after the volumes (count), their shell the smoke thins in (times the core), detail fade, occluder density
	flow: vec4<f32>,  // the billow noise's offset (metres; flow.rs: wind drift + the frame's mean rise, integrated), wrapped by 1024 m
	flowd: vec4<f32>, // the same offset wrapped by the detail tile
};
@group(0) @binding(9) var<storage, read> vols: array<Vol>;
@group(0) @binding(10) var<uniform> g: G;

// The hit list of the pixel's ray: volumes it passes through, with the ray parameters (view depth) it enters and leaves at.
var<private> nh: i32;
var<private> hit_i: array<u32, 16>;
var<private> hit_a: array<f32, 16>;
var<private> hit_b: array<f32, 16>;
// The fire cores (fire_cut.rs) whose shell the pixel's ray passes through: record, chord middle and half (ray parameter), gain.
var<private> nc: i32;
var<private> cut_i: array<u32, 4>;
var<private> cut_m: array<f32, 4>;
var<private> cut_h: array<f32, 4>;
var<private> cut_k: array<f32, 4>;

// The view depth the pixel's ray ends at: the scene's depth at its place, at most the smoke range (f.march.y).
fn end_depth(c: vec2<i32>) -> f32 {
	let lc = min(c * i32(f.lin.z), vec2<i32>(f.lin.xy) - vec2<i32>(1));
	return min(textureLoad(lin, lc, 0).r, f.march.y);
}

// Weight of a noise octave whose cells are `cell` metres wide at a sample footprint of `fp` metres (the larger of the grid texel and
// the march step there): whole while a cell spans two footprints, gone below one (finer noise would alias into a fixed grain or
// hatch, not billows).
fn band(fp: f32, cell: f32) -> f32 {
	return clamp(2.0 * cell / max(fp, 1.0e-4) - 1.0, 0.0, 1.0);
}

// The albedo of a volume: the retail colour (brown or orange-tinted on some fires) moved by g.tune3.w toward a neutral grey of
// its own brightness, kept in a dark smoke range (burning buildings make dark grey smoke, not tan or glowing balls).
fn smoke_albedo(c: vec3<f32>) -> vec3<f32> {
	let lum = dot(c, vec3<f32>(0.3, 0.59, 0.11));
	let grey = vec3<f32>(clamp(lum, 0.16, 0.42)) * vec3<f32>(0.97, 0.98, 1.0);
	return mix(c, grey, clamp(g.tune3.w, 0.0, 1.0));
}

// The smoke at one point: extinction per world unit, albedo, fire glow.
struct Smp {
	d: f32,
	alb: vec3<f32>,
	gl: vec3<f32>,
	// Height in the puffs (-1 bottom .. 1 top, weighted): the sky lights a puff's top, its underside stays dark.
	hz: f32,
};

// The smoke at p, at sample footprint `fp` metres (1e3 = the big shape only: the shadow taps; `lit` = also the albedo and glow).
// The hit volumes are soft envelopes (1 at the centre, 0 at the surface of the ellipsoid, `mo.w` times taller than wide) joined
// into ONE coverage (1 - the product of their misses), and one world-anchored billow noise carves that coverage: big round lumps
// (the cellular channel, bright cell centres, warped by the fbm), a second octave of smaller lumps on them and fine wisps at the
// edges, each finer octave faded by its footprint (`band`). Because every puff of a plume is carved by the same noise, the plume
// keeps one cauliflower surface with sharp-ish edges and a dense body; puffs carved each by their own noise averaged into soft
// round blobs. The noise moves by one offset per frame (`g.flow`, flow.rs: the wind drift and the frame's eased mean rise,
// integrated on the CPU): a per-sample rise blend times the clock sheared it where volumes of different rise overlap.
// Each volume's share (`glow.w`: the trail band, the range fade) scales the density after the clumping, so a puff in a band
// cross-fades with its sprite: the shared coverage (each volume's coverage x its share) over the coverage. The shape (the joined
// coverage the billows carve) takes a volume by min(4 x share, 1) (review R29 M1): a puff entering the band at share ~0 leaves the
// volume it overlaps as it was (no step), a lone puff from share 0.25 fades exactly linearly, and below 0.25 its shape erodes away
// (less than linear) while its sprite (keep >= 0.75) carries the look. `shr` stays <= 1 (share <= min(4 x share, 1)).
fn smoke_at(p: vec3<f32>, fp: f32, lit: bool) -> Smp {
	var miss = 1.0;
	var smiss = 1.0;
	var wsum = 0.0;
	var ext = 0.0;
	var age = 0.0;
	var bil = 0.0;
	var hz = 0.0;
	var rad = 0.0;
	var alb = vec3<f32>(0.0);
	var gl = vec3<f32>(0.0);
	for (var h = 0; h < nh; h++) {
		let vv = vols[hit_i[h]];
		let rel = (p - vv.c.xyz) / vec3<f32>(vv.c.w, vv.c.w, vv.c.w * max(vv.mo.w, 1.0));
		let r2 = dot(rel, rel);
		if (r2 >= 1.0) { continue; }
		let cov = 1.0 - sqrt(r2);
		// Weighted by the volume's drawn thickness (extinction x radius x share), not its extinction: a big thin haze still counts.
		let w = cov * vv.col.w * vv.c.w * vv.glow.w + 1.0e-6;
		let ce = cov * min(vv.glow.w * 4.0, 1.0);
		miss *= 1.0 - ce;
		smiss *= 1.0 - cov * vv.glow.w;
		wsum += w;
		ext += w * vv.col.w;
		age += w * vv.mo.z;
		bil += w * vv.mo.y;
		hz += w * rel.z;
		rad += w * vv.c.w;
		if (lit) {
			alb += w * smoke_albedo(vv.col.rgb);
			// The fire's light falls off with the distance from the fire, not from the volume's centre: only the part near it glows.
			let kk = clamp(1.0 - length(p - vv.lp.xyz) / max(vv.lp.w, 0.01), 0.0, 1.0);
			gl += w * vv.glow.rgb * kk * kk;
		}
	}
	if (wsum <= 0.0) { return Smp(0.0, vec3<f32>(0.0), vec3<f32>(0.0), 0.0); }
	let inv = 1.0 / wsum;
	let cov = 1.0 - miss;
	let shr = (1.0 - smiss) / max(cov, 1.0e-6);
	let q = p - g.flow.xyz;
	let s = g.tune.y;
	// Clumping: a thin volume (below 0.6 per metre) keeps its mass in denser billows with gaps between them (up to 4x denser, more
	// eroded) instead of a uniform thin fog: thin fog through a big volume only shows its chord length, a smooth ball.
	let e0 = ext * inv;
	let clump = clamp(0.6 / max(e0, 1.0e-4), 1.0, 4.0);
	let er = g.amb.w * (0.8 + 0.4 * age * inv) * (0.75 + 0.25 * bil * inv) * (0.55 + 0.2 * clump);
	// Big lumps: 6 cells per tile, the tile 2^floor(log2(s x the mean radius)) and twice that, blended; medium: 6 cells per tile of a
	// third (and two thirds) of it. The billows grow with the puffs (a 30 m plume rolls in lumps of metres, a 1 m puff in ones of
	// decimetres; one fixed size made big volumes smooth balls with a fuzzy rim), and the blend of the two power-of-two tiles keeps
	// the noise anchored to the world: a changing radius cross-fades instead of zooming. Each tile size keeps its own offset whichever
	// slot it is in (by the parity of its level), so where the mean radius crosses a power of two the slot fading out and the one
	// taking over show the same noise: no seam in space, no pop in time (review R18 B1). `lv` stays below 10 (flow.rs WRAP).
	let lv = min(log2(s * max(rad * inv, 0.25)), 9.99);
	let l0 = floor(lv);
	let fk = lv - l0;
	let ta = exp2(l0);
	let odd = (i32(l0) & 1) != 0;
	let oe = vec3<f32>(0.37, 0.11, 0.73);
	let oo = vec3<f32>(0.61, 0.83, 0.19);
	let na = noise_at(q / ta + select(oe, oo, odd));
	let nb = noise_at(q / (ta * 2.0) + select(oo, oe, odd));
	// Both noises in standard units (the texture's 0.75 G + 0.25 R has mean 0.488, deviation 0.132; G 0.484, 0.171: sums and blends
	// of them sit near their mean, so unscaled they only shrank the volume evenly: smooth balls). The blend of the two tiles keeps
	// its deviation (divided by the blend's own).
	let vn = inverseSqrt(fk * fk + (1.0 - fk) * (1.0 - fk));
	let big = (mix(na.g * 0.75 + na.r * 0.25, nb.g * 0.75 + nb.r * 0.25, fk) - 0.488) * vn * (1.0 / 0.132);
	// Medium lumps on them: tiles a third of the big ones (6 cells each).
	let w2 = band(fp, ta * (1.0 + fk) / 18.0);
	var med = 0.0;
	if (w2 > 0.0) {
		let me = vec3<f32>(0.71, 0.29, 0.53);
		let mo = vec3<f32>(0.23, 0.47, 0.91);
		let ma = noise_at(q * 3.0 / ta + select(me, mo, odd)).g;
		let mb = noise_at(q * 1.5 / ta + select(mo, me, odd)).g;
		med = (mix(ma, mb, fk) - 0.484) * vn * (w2 / 0.171);
	}
	// The billows (0 crevice .. 1 lump top, about 0.07..0.93 for 90 % of the points): the coverage is cut where they are low, so the
	// plume's surface is theirs.
	let lump = clamp(0.5 + (big * 0.8 + med * 0.45) * 0.28, 0.0, 1.0);
	var d = cov - (1.0 - lump) * er;
	let w3 = band(fp, g.tune.z / 8.0);
	if (w3 > 0.0) {
		// Wisps: the fine fbm eats into the thin outer layer only (the body stays dense and lumpy).
		let det = noise_at((p - g.flowd.xyz) / g.tune.z + vec3<f32>(0.13, 0.59, 0.31)).b;
		d += (det - 0.5) * (0.05 / 0.121) * (0.4 + age * inv) * w3 * (1.0 - smoothstep(0.0, 0.25, d));
	}
	let dens = smoothstep(0.0, 0.1, d) * (0.6 + 0.4 * smoothstep(0.05, 0.4, d)) * e0 * clump * shr * g.tune.x;
	return Smp(dens, alb * inv, gl * inv, hz * inv);
}

// Optical depth from p toward the sun over `len` metres, through the hit volumes (taps jittered by `j`).
fn shadow_tau(p: vec3<f32>, j: f32, len: f32) -> f32 {
	let taps = i32(g.tune3.z);
	if (taps <= 0) { return 0.0; }
	let seg = len / f32(taps);
	var tau = 0.0;
	for (var s = 0; s < taps; s++) {
		let ps = p - f.sun.xyz * (seg * (f32(s) + 0.25 + 0.5 * j));
		tau += smoke_at(ps, 1.0e3, false).d * seg;
	}
	return tau;
}
"#;

/// The march and the light.
pub const MARCH: &str = r#"
// Ray parameters (enter, leave) of eye + d * t through the ellipsoid of record v grown by k: horizontal radius c.w * k, `tall`
// times higher (a sphere in a space squashed by the height ratio: the ray parameters stay the same); enter > leave when missed.
fn ray_ell(eye: vec3<f32>, d: vec3<f32>, v: Vol, tall: f32, k: f32) -> vec2<f32> {
	let sq = vec3<f32>(1.0, 1.0, 1.0 / tall);
	let oc = (eye - v.c.xyz) * sq;
	let ds = d * sq;
	let a = dot(ds, ds);
	let b = dot(oc, ds);
	let r = v.c.w * k;
	let disc = b * b - a * (dot(oc, oc) - r * r);
	if (disc <= 0.0) { return vec2<f32>(1.0, 0.0); }
	let s = sqrt(disc);
	return vec2<f32>((-b - s) / a, (-b + s) / a);
}

// Fills the hit list; returns the span (enter, leave) of all hits, enter > leave when nothing is hit.
fn find_hits(eye: vec3<f32>, d: vec3<f32>, t_end: f32) -> vec2<f32> {
	var lo = 1.0e9;
	var hi = 0.0;
	nh = 0;
	let n = min(i32(g.tune3.y), 64);
	for (var i = 0; i < n; i++) {
		let vv = vols[i];
		let e = ray_ell(eye, d, vv, max(vv.mo.w, 1.0), 1.0);
		let t0 = max(e.x, 0.1);
		let t1 = min(e.y, t_end);
		if (t1 <= t0) { continue; }
		// A full list keeps the 16 nearest hits: a nearer hit replaces the farthest.
		var slot = -1;
		if (nh < 16) {
			slot = nh;
			nh += 1;
		} else {
			var worst = 0;
			for (var h = 1; h < 16; h++) {
				if (hit_a[h] > hit_a[worst]) { worst = h; }
			}
			if (t0 < hit_a[worst]) { slot = worst; }
		}
		if (slot >= 0) {
			hit_i[slot] = u32(i);
			hit_a[slot] = t0;
			hit_b[slot] = t1;
			lo = min(lo, t0);
			hi = max(hi, t1);
		}
	}
	return vec2<f32>(lo, hi);
}

// The fire cores (fire_cut.rs: `g.cut.x` records after the smoke's own, c = centre and horizontal radius, mo.w = height ratio)
// whose shell (`g.cut.y` times the core) the pixel's ray crosses before `t_end`, the first 4 (fire_cut.rs sends the biggest on
// screen first). The flames are drawn before the smoke, so the smoke must not cover them from behind: each core is a soft occluder
// (density `g.cut.w` per core radius at its middle, falling as 1 - q^2 to 0 at its surface; `fire_tau`), its chord kept here.
fn fire_cuts(eye: vec3<f32>, d: vec3<f32>, t_end: f32, len: f32) {
	nc = 0;
	let base = i32(g.tune3.y);
	for (var j = 0; j < i32(g.cut.x); j++) {
		if (nc >= 4) { break; }
		let vv = vols[base + j];
		let sh = ray_ell(eye, d, vv, vv.mo.w, g.cut.y);
		if (sh.y <= sh.x || sh.y <= 0.0 || sh.x >= t_end) { continue; }
		let core = ray_ell(eye, d, vv, vv.mo.w, 1.0);
		let ds = d * vec3<f32>(1.0, 1.0, 1.0 / vv.mo.w);
		cut_i[nc] = u32(base + j);
		cut_m[nc] = 0.5 * (core.x + core.y);
		cut_h[nc] = max(0.5 * (core.y - core.x), 0.0);
		// Along the chord 1 - q^2 = A / r^2 * (h^2 - u^2) (A = |d squashed|^2, u from the chord's middle); per metre: times |d|.
		cut_k[nc] = g.cut.w / vv.c.w * len * dot(ds, ds) / (vv.c.w * vv.c.w);
		nc += 1;
	}
}

// Optical depth of the listed fire cores between the eye and ray parameter t (the closed form of the 1 - q^2 profile along each
// chord: no samples, no hard edge).
fn fire_tau(t: f32) -> f32 {
	var tau = 0.0;
	for (var j = 0; j < nc; j++) {
		let h = cut_h[j];
		let u = clamp(t - cut_m[j], -h, h);
		tau += cut_k[j] * (h * h * (u + h) - (u * u * u + h * h * h) / 3.0);
	}
	return tau;
}

// The share of the smoke kept at p: none inside a fire core, all from the edge of its shell out (smoke rises from a flame's tips,
// it does not wrap the flame in a veil).
fn hollow(p: vec3<f32>) -> f32 {
	var k = 1.0;
	for (var j = 0; j < nc; j++) {
		let vv = vols[cut_i[j]];
		let rel = (p - vv.c.xyz) / vec3<f32>(vv.c.w, vv.c.w, vv.c.w * vv.mo.w);
		k *= smoothstep(1.0, g.cut.y, length(rel));
	}
	return k;
}

// The hit spans merged into disjoint segments, nearest first (`seg_a` / `seg_b`, ray parameters): the march spends its steps only
// inside smoke, not on the gaps between puffs far apart in depth (a column 30 m deep of 1 m puffs would get one sample per puff
// and turn into noise and blobs). Returns the segments' summed length.
var<private> ns: i32;
var<private> seg_a: array<f32, 16>;
var<private> seg_b: array<f32, 16>;
fn segments() -> f32 {
	var ord: array<i32, 16>;
	for (var i = 0; i < nh; i++) {
		var j = i;
		while (j > 0 && hit_a[ord[j - 1]] > hit_a[i]) {
			ord[j] = ord[j - 1];
			j -= 1;
		}
		ord[j] = i;
	}
	ns = 0;
	var total = 0.0;
	for (var k = 0; k < nh; k++) {
		let a = hit_a[ord[k]];
		let b = hit_b[ord[k]];
		if (ns > 0 && a <= seg_b[ns - 1]) {
			if (b > seg_b[ns - 1]) {
				total += b - seg_b[ns - 1];
				seg_b[ns - 1] = b;
			}
		} else {
			seg_a[ns] = a;
			seg_b[ns] = b;
			ns += 1;
			total += b - a;
		}
	}
	return total;
}

// The march's start offset of a grid pixel in frame `fr`: a hash (no interleaved-gradient pattern: that stays screen fixed and
// turns into a diagonal hatch / checker in dense smoke), new each frame (the resolve averages it).
fn smoke_jit(px: vec2<f32>, fr: f32) -> f32 {
	var h = (u32(px.x) * 1973u) ^ (u32(px.y) * 9277u) ^ (u32(fr) * 26699u) ^ 0x9e3779b9u;
	h = (h ^ (h >> 16u)) * 0x7feb352du;
	h = (h ^ (h >> 15u)) * 0x846ca68bu;
	h = h ^ (h >> 16u);
	return f32(h >> 8u) * (1.0 / 16777216.0);
}

// rgb = light scattered toward the eye, a = transmittance, along the pixel's ray up to the scene.
@fragment fn fs_smoke(v: VO) -> @location(0) vec4<f32> {
	let c = vec2<i32>(v.pos.xy);
	let eye = f.cam[0].xyz;
	let d = view_dir(v.pos.xy);
	let len = length(d);
	let vdir = d / len;
	let hits = find_hits(eye, d, end_depth(c));
	if (nh == 0) { return vec4<f32>(0.0, 0.0, 0.0, 1.0); }
	fire_cuts(eye, d, hits.y, len);
	let total = segments();
	// One grid texel's width in metres per metre of depth: with the march step it is the sample footprint the noise octaves are
	// band-limited by (`band`).
	let texel = 2.0 * f.proj.y / f.size.y;
	let steps = max(i32(f.march.x), 1);
	let jit = smoke_jit(v.pos.xy, f.march.z);
	let dtp = total / f32(steps);
	let dt = dtp * len;
	var si = 0;
	var base = 0.0;
	// The self-shadow march reaches across the biggest hit volume (a 10 m plume is dark on its far side, not only 2.5 m in).
	var rmax = 0.0;
	for (var h = 0; h < nh; h++) {
		rmax = max(rmax, vols[hit_i[h]].c.w);
	}
	let slen = clamp(rmax * 1.2, g.tune2.w, g.tune2.w * 3.0);
	// The light that does not depend on the sample: the sun's phase toward the eye, the key light's colour.
	let cs = -dot(f.sun.xyz, vdir);
	let ph = min(0.65 * phase_rel(cs, f.sun_col.w) + 0.35 * phase_rel(cs, -0.25), 2.2);
	let sun_c = f.sun_col.rgb * g.tune2.y;
	var t_acc = 1.0;
	var s_acc = vec3<f32>(0.0);
	for (var k = 0; k < steps; k++) {
		let sk = (f32(k) + jit) * dtp;
		while (si < ns - 1 && sk > base + seg_b[si] - seg_a[si]) {
			base += seg_b[si] - seg_a[si];
			si += 1;
		}
		let t = min(seg_a[si] + sk - base, seg_b[si]);
		let p = eye + d * t;
		// The smoke behind a flame's core is hidden by the flame (drawn before the smoke): its share is the flame's transmittance.
		var occ = 1.0;
		if (nc > 0) {
			occ = exp(-fire_tau(t));
			if (occ < 0.004) { break; }
		}
		let fp = max(texel * t, dt) * (g.cut.z / 0.12);
		let sm = smoke_at(p, fp, true);
		var dens = sm.d;
		if (dens <= 1.0e-4) { continue; }
		let alb = sm.alb;
		let gl = sm.gl;
		dens *= hollow(p) * occ;
		if (dens <= 1.0e-4) { continue; }
		let tau_s = shadow_tau(p, jit, slen);
		let vis = sun_vis(p);
		let beam = exp(-tau_s * 1.1);
		let direct = sun_c * (vis * beam * ph);
		let multi = sun_c * (0.25 * exp(-0.3 * tau_s) * mix(0.35, 1.0, vis));
		let dn = 1.0 - exp(-dens * 2.0);
		// Ambient is shadowed inside the column too (the side away from the sun is darker: depth, not a flat smudge).
		// The sky lights the puffs' tops; their undersides stay dark (the billows read as round lumps, not a flat smudge).
		let sky = mix(0.45, 1.15, smoothstep(-0.9, 0.8, sm.hz));
		let amb = g.amb.rgb * g.tune2.x * sky * (1.0 - 0.4 * dn) * mix(0.55, 1.0, exp(-0.6 * tau_s));
		// The sun and sky light lose most of their tint (smoke_grey): the level's warm key light made grey smoke tan.
		let sky_sun = direct + multi + amb;
		let l = alb * (mix(sky_sun, vec3<f32>(dot(sky_sun, vec3<f32>(0.3, 0.59, 0.11))), 0.7 * g.tune3.w) + gl * g.tune2.z);
		let ext = exp(-dens * dt);
		s_acc += t_acc * (1.0 - ext) * min(l, vec3<f32>(3.0));
		t_acc *= ext;
		if (t_acc < 0.01) { break; }
	}
	return vec4<f32>(s_acc, t_acc);
}
"#;

/// The smoke applied to the picture (blend: picture * a + rgb): the four nearest smoke pixels, weighted by how close their depth
/// is to the pixel's own (no halo round near edges).
pub const APPLY: &str = r#"
fn smoke_taps(pos: vec2<f32>, zc: f32) -> vec4<f32> {
	let lin_dim = vec2<i32>(f.lin.xy);
	let div = i32(f.lin.z);
	let fdim = vec2<i32>(f.size.xy);
	let fp = pos * 0.5 / f.lin.z - vec2<f32>(0.5);
	let i0 = vec2<i32>(floor(fp));
	let fr = fp - floor(fp);
	var acc = vec4<f32>(0.0);
	var wsum = 0.0;
	for (var k = 0; k < 4; k++) {
		let o = vec2<i32>(k & 1, k >> 1);
		let ci = clamp(i0 + o, vec2<i32>(0), fdim - vec2<i32>(1));
		let zt = textureLoad(lin, min(ci * div, lin_dim - vec2<i32>(1)), 0).r;
		let wb = select(1.0 - fr.x, fr.x, o.x == 1) * select(1.0 - fr.y, fr.y, o.y == 1);
		let w = wb * exp(-abs(zt - zc) / (0.25 * zc + 0.6)) + 1.0e-4;
		acc += textureLoad(cur, ci, 0) * w;
		wsum += w;
	}
	return acc / wsum;
}

@fragment fn fs_apply(v: VO) -> @location(0) vec4<f32> {
	let lin_dim = vec2<i32>(f.lin.xy);
	let zc = textureLoad(lin, min(vec2<i32>(v.pos.xy * 0.5), lin_dim - vec2<i32>(1)), 0).r;
	return smoke_taps(v.pos.xy, zc);
}
"#;

/// The whole WGSL of the pass. `RAC_SMOKE_WGSL=<file>` replaces it with that file's text (look tuning without a rebuild); the
/// dev switch `RAC_WGSL_DIR` (devtune.rs) works on top, by the label "volsmoke".
pub fn source() -> String {
	if let Some(text) = std::env::var_os("RAC_SMOKE_WGSL").and_then(|p| std::fs::read_to_string(p).ok()) {
		return text;
	}
	built_in()
}

/// The built-in text.
pub fn built_in() -> String {
	format!("{}{}{}{}{}{}", scatter::BLOCK, scatter::HELPERS, DEFS, MARCH, temporal::RESOLVE, APPLY)
}
