//! Shadow-volume geometry (engine split stage E2 batch 4b, 2026-10-26: moved out of rac1/shadows.rs, which re-exports it):
//! the stencil volumes of sphere and capsule casters as the retail VU0 shadow code builds them (BRIDGE-64a, decomp/notes/
//! shadows.md); the pass that draws them is viewer/shadowvol.rs.

use crate::physics::{add, cross, dot, normalize, scale, sub, V3};

/// BRIDGE-64a: the shadow-volume extrusion length (shadow_basis 0x29aaf0 argument 1000.0).
pub const VOLUME_EXTRUDE: f32 = 1000.0;

/// BRIDGE-64a: the ring step numerator (shadow_sphere_ring 0x29b8b8 rotates by 6.28 / n, not 2 pi).
pub const RING_TURN: f32 = 6.28;

/// BRIDGE-64a / SCH-6b: the collector's shadow fade (entry +4): `min(((dist << 10) - trunc(z - w)) >> 1, 0x1000)`, `diff`
/// = trunc(z - w) with z the view depth and w the radius, both x1024.
pub fn volume_fade(dist: u8, diff: i32) -> i32 {
	(((dist as i32) << 10).wrapping_sub(diff) >> 1).min(0x1000)
}

/// BRIDGE-64a: light slot 0 (hero_light_slot_update 0x264520) from the hero's blended light vector `v` (FUN_00264cc8):
/// a = fast_atan2(v.x, v.y), (0.14 cos a, 0.14 sin a, -0.99).
pub fn volume_light(v: V3) -> V3 {
	let a = crate::vumath::fast_atan2(v[0], v[1]);
	[crate::vumath::fast_cos(a) * 0.14, crate::vumath::fast_sin(a) * 0.14, f32::from_bits(0xbf7d70a4)]
}

/// BRIDGE-64a: one sphere primitive of a caster as the record build FUN_0029b278 leaves it: the joint-transformed local
/// point `c` (model space at class scale, before k) and the radius at class scale; `n` ring sides.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct VolumeSphere {
	pub c: V3,
	pub r: f32,
	pub n: u32,
}

/// BRIDGE-64a: one entry of the shadow list (0x1aca00) with its record inputs.
#[derive(Clone, Debug)]
pub struct VolumeCaster {
	pub spheres: Vec<VolumeSphere>,
	/// The moby placement (column-major, translation in 12..15) without the scale.
	pub m: [f32; 16],
	/// k = scale * fade / 4096.
	pub k: f32,
	/// [+0x84, +0x88].
	pub band: [f32; 2],
	/// Entry +6: the shadow sphere straddles the view volume (the ring is clip-tested).
	pub straddle: bool,
}

/// BRIDGE-64a: the triangles of one primitive's volume: `inc` raise the stencil counter, `dec` lower it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShadowVolume {
	pub inc: Vec<[V3; 3]>,
	pub dec: Vec<[V3; 3]>,
}

/// BRIDGE-64a: shadow_basis 0x29aaf0: the unit light direction, the extrusion and the ring start (unit(dir x e), e the
/// axis of dir's smallest component).
pub fn volume_basis(light: V3) -> (V3, V3, V3) {
	let d = normalize(light);
	let mut k = 0;
	for i in 1..3 {
		if d[i] < d[k] {
			k = i;
		}
	}
	let mut e = [0.0; 3];
	e[k] = 1.0;
	(d, scale(d, VOLUME_EXTRUDE), normalize(cross(d, e)))
}

/// BRIDGE-64a: shadow_sphere_ring 0x29b8b8: `n` ring offsets of radius `r`, the start rotated by 6.28 / n about -dir each
/// step (the closing pair 0 is the caller's).
pub fn sphere_ring(dir: V3, start: V3, r: f32, n: u32) -> Vec<V3> {
	let mut th = RING_TURN / n.max(1) as f32;
	if th >= std::f32::consts::PI {
		th -= std::f32::consts::TAU;
	}
	let (sn, cs) = (crate::vumath::fast_sin(th), crate::vumath::fast_cos(th));
	let k = scale(dir, -1.0);
	let mut v = scale(start, r);
	let mut out = Vec::with_capacity(n as usize);
	for _ in 0..n {
		out.push(v);
		v = add(scale(v, cs), scale(cross(k, v), sn));
	}
	out
}

/// BRIDGE-64a: the volumes of every caster (one per sphere primitive; capsules are BRIDGE-64b): record build, ring, band clip
/// (shadow_ring_clip 0x29c058), the straddle clip test (`outside`: a world point outside the clip volume) and the faces.
/// [I] Each face and cap goes to `inc` when it faces the eye `eye` (z-pass counting, front faces up, back faces down):
/// retail tags the sides by their screen winding and the caps by sign(A * f), which is the same split.
pub fn volumes(casters: &[VolumeCaster], light: V3, eye: V3, outside: &dyn Fn(V3) -> bool) -> Vec<ShadowVolume> {
	let (dir, ext, start) = volume_basis(light);
	let mut out = Vec::new();
	for cs in casters.iter().filter(|c| c.band[1] > 0.0) {
		let m = &cs.m;
		for sp in &cs.spheres {
			if sp.n == 0 {
				continue;
			}
			let c = scale(sp.c, cs.k);
			let centre = [0, 1, 2].map(|i| m[i] * c[0] + m[4 + i] * c[1] + m[8 + i] * c[2] + m[12 + i]);
			let ring = sphere_ring(dir, start, sp.r * cs.k, sp.n);
			// Each pair (top, bottom) moved along its line to z = band[1] / band[0].
			let at_z = |p: V3, z: f32| {
				let b = add(p, ext);
				let t = (z - p[2]) / (b[2] - p[2]);
				add(p, scale(sub(b, p), t))
			};
			let pairs: Vec<(V3, V3)> = ring.iter().map(|&v| add(centre, v)).map(|p| (at_z(p, cs.band[1]), at_z(p, cs.band[0]))).collect();
			let clipped: Vec<bool> = pairs.iter().map(|&(t, b)| cs.straddle && (outside(t) || outside(b))).collect();
			out.push(ring_volume(&pairs, &clipped, dir, eye, ([0.0, 0.0, 1.0], cs.band), at_z(centre, cs.band[1])));
		}
	}
	out
}

/// M3a shadows: one primitive of a retail shadow record (Eudora 0x1ac880, built each drawn frame by shadow_records_build
/// 0x2787e0 = Novalis FUN_0029b278) in world space with k applied [C: decomp/notes/shadows.md "Record build"].
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum RecPrim {
	/// Type 0 (0x20 B): {0, n, 0, 0, centre xyz, radius}.
	Sphere { c: V3, r: f32, n: u32 },
	/// Type 1 (0x30 B): {1, n0, n1, 0, p0 xyz r0, p1 xyz r1}, the end trims already applied.
	Capsule { p0: V3, r0: f32, p1: V3, r1: f32, n0: u32, n1: u32 },
}

/// M3a shadows: one retail shadow record: header {prim count, straddle flag (list entry +6), band +0x84 / +0x88, light
/// slot 0x1aef80 + (+0xbd) * 16} and its primitives [C].
#[derive(Clone, Debug, PartialEq)]
pub struct RecCaster {
	pub straddle: bool,
	/// The clip slab {x : band[0] <= up . x <= band[1]} the volume is cut to (bottom, top).
	pub band: [f32; 2],
	/// The slab's axis: R&C1 world Z (the band = two world heights); R&C2 the caster's up axis (its rotation row +0xe0, the band
	/// d -+ h, rac2/shadows.rs), so a shadow lies on slopes and walls too.
	pub up: V3,
	pub light: V3,
	pub prims: Vec<RecPrim>,
}

impl Default for RecCaster {
	fn default() -> RecCaster {
		RecCaster { straddle: false, band: [0.0; 2], up: [0.0, 0.0, 1.0], light: [0.0; 3], prims: Vec::new() }
	}
}

/// BRIDGE-64b shadow_capsule_ring (Novalis FUN_0029ba08): the ring pairs (top, top + E) of a capsule primitive, the
/// closing copy of pair 0 last. `d` = unit light direction, `p` = the ring start of `volume_basis`, `e` = d * 1000.
/// [I] sin / cos are f32 (retail: VU0 microsubroutines 0xc90 / 0xc80); a zero L or q (VU0 divide by zero) gives no ring.
pub fn capsule_ring(p0: V3, r0: f32, p1: V3, r1: f32, n0: u32, n1: u32, d: V3, p: V3, e: V3) -> Vec<(V3, V3)> {
	let rot = |t: f32, v: V3| {
		let (s, c) = t.sin_cos();
		add(add(scale(v, c), scale(cross(d, v), s)), scale(d, (1.0 - c) * dot(d, v)))
	};
	let pair = |q: V3| (q, add(q, e));
	let cc = cross(sub(p1, p0), d);
	let l = dot(cc, cc).sqrt();
	let mut out = Vec::new();
	if l + r1 - r0 < 0.0 {
		// Containment: only the p0 sphere's ring, rotated before each push.
		if n0 > 0 {
			let s = 6.28 / n0 as f32;
			let mut w = scale(p, r0);
			for _ in 0..n0 {
				w = rot(s, w);
				out.push(pair(add(p0, w)));
			}
			out.push(out[0]);
		}
		return out;
	}
	if l == 0.0 {
		return out;
	}
	let b = if r0 - r1 == 0.0 {
		1.57
	} else {
		let q = (l * r1) / (r0 - r1);
		if q == 0.0 {
			return out;
		}
		1.57 - asin_poly(r1 / q)
	};
	let (a0, a1) = (2.0 * (3.14 - b), 2.0 * b);
	let h = b + a0;
	let u = scale(cc, 1.0 / l);
	let (v0, v1) = (scale(u, r0), scale(u, r1));
	if n0 == 0 {
		out.push(pair(sub(p0, v0)));
		out.push(pair(add(p0, v0)));
	} else {
		let mut w = rot(1.57 + b, v0);
		let s0 = a0 / n0 as f32;
		for _ in 0..n0 {
			out.push(pair(add(p0, w)));
			w = rot(s0, w);
		}
	}
	if n1 == 0 {
		out.push(pair(add(p1, v1)));
		out.push(pair(sub(p1, v1)));
	} else {
		let mut w = rot((1.57 + h) - 6.28, v1);
		let s1 = a1 / n1 as f32;
		for _ in 0..n1 {
			out.push(pair(add(p1, w)));
			w = rot(s1, w);
		}
	}
	out.push(out[0]);
	out
}

/// M3a shadows: the volumes of retail's own shadow records (retail mode): per record shadow_basis(1000, its light), per
/// primitive the sphere ring (0x29b8b8) or the capsule ring (0x29ba08), the band clip (0x29c058), the straddle clip test
/// and the faces, as `volumes` does for the port's casters.
pub fn record_volumes(casters: &[RecCaster], eye: V3, outside: &dyn Fn(V3) -> bool) -> Vec<ShadowVolume> {
	let mut out = Vec::new();
	for cs in casters {
		// The point of the segment p..b on the plane up . x = z (up = Z: `(z - p[2]) / (b[2] - p[2])` exactly, 0 * x being 0).
		let up = cs.up;
		let at_z = |p: V3, b: V3, z: f32| add(p, scale(sub(b, p), (z - dot(up, p)) / (dot(up, b) - dot(up, p))));
		let (dir, ext, start) = volume_basis(cs.light);
		// A light along the slab (R&C2 casters on walls) cuts no volume (the VU0 clip divides by up . E).
		if dot(up, dir).abs() < 1e-4 {
			continue;
		}
		for pr in &cs.prims {
			let (mut ring, mid) = match *pr {
				RecPrim::Sphere { c, r, n } => (sphere_ring(dir, start, r, n).into_iter().map(|v| add(c, v)).map(|t| (t, add(t, ext))).collect::<Vec<_>>(), c),
				RecPrim::Capsule { p0, r0, p1, r1, n0, n1 } => {
					let mut ring = capsule_ring(p0, r0, p1, r1, n0, n1, dir, start, ext);
					ring.pop();
					(ring, scale(add(p0, p1), 0.5))
				}
			};
			if ring.len() < 2 {
				continue;
			}
			for q in ring.iter_mut() {
				*q = (at_z(q.0, q.1, cs.band[1]), at_z(q.0, q.1, cs.band[0]));
			}
			let clipped: Vec<bool> = ring.iter().map(|&(t, b)| cs.straddle && (outside(t) || outside(b))).collect();
			out.push(ring_volume(&ring, &clipped, dir, eye, (up, cs.band), at_z(mid, add(mid, ext), cs.band[1])));
		}
	}
	out
}

/// The side faces and caps of one band-clipped ring (pairs (top, bottom), the closing copy left out; `mid` a point inside
/// the top ring orients the sides). Faces touching a clipped pair are skipped and clipped pairs leave the caps.
fn ring_volume(pairs: &[(V3, V3)], clipped: &[bool], dir: V3, eye: V3, (up, band): (V3, [f32; 2]), mid: V3) -> ShadowVolume {
	let n = pairs.len();
	let mut vol = ShadowVolume::default();
	for i in 0..n {
		let j = (i + 1) % n;
		if clipped[i] || clipped[j] {
			continue;
		}
		let (t0, b0, t1, b1) = (pairs[i].0, pairs[i].1, pairs[j].0, pairs[j].1);
		let mut nrm = cross(sub(t1, t0), dir);
		if dot(nrm, sub(add(t0, t1), scale(mid, 2.0))) < 0.0 {
			nrm = scale(nrm, -1.0);
		}
		let list = if dot(nrm, sub(t0, eye)) < 0.0 { &mut vol.inc } else { &mut vol.dec };
		list.push([t0, b0, t1]);
		list.push([t1, b0, b1]);
	}
	// Caps: fans over the unclipped ring vertices; the top cap faces along `up` (+z), the bottom one against it.
	let keep: Vec<usize> = (0..n).filter(|&i| !clipped[i]).collect();
	if keep.len() >= 3 {
		let e = dot(up, eye);
		for (top, z) in [(true, band[1]), (false, band[0])] {
			let front = if top { e > z } else { e < z };
			let v = |i: usize| if top { pairs[i].0 } else { pairs[i].1 };
			let list = if front { &mut vol.inc } else { &mut vol.dec };
			for w in keep[1..].windows(2) {
				list.push([v(keep[0]), v(w[0]), v(w[1])]);
			}
		}
	}
	vol
}

/// FUN_00221258, the cubic arc-sine approximation (the same polynomial as game/tube.rs `asin_poly`).
fn asin_poly(x: f32) -> f32 {
	let a = x.abs();
	let poly = 1.57073 - 0.212114 * a + 0.074261 * a * a - 0.0187293 * a * a * a;
	let v = 1.5707964 - (1.0 - a).abs().sqrt() * poly;
	if x < 0.0 { -v } else { v }
}
