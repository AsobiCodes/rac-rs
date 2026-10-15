//! Triangle-soup collision: a 2D grid over XY (Z up) for fast lookups, a downward ground ray and
//! sphere push-out. Independent of where the triangles come from (the level collision mesh, or the
//! visible geometry as a stand-in).

use std::collections::HashMap;

pub type V3 = [f32; 3];

pub fn sub(a: V3, b: V3) -> V3 {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub fn add(a: V3, b: V3) -> V3 {
	[a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub fn scale(a: V3, s: f32) -> V3 {
	[a[0] * s, a[1] * s, a[2] * s]
}
pub fn dot(a: V3, b: V3) -> f32 {
	a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
pub fn cross(a: V3, b: V3) -> V3 {
	[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
pub fn length(a: V3) -> f32 {
	dot(a, a).sqrt()
}
pub fn normalize(a: V3) -> V3 {
	scale(a, 1.0 / length(a).max(1e-9))
}


#[derive(Clone)]
pub struct Tri {
	pub v: [V3; 3],
	pub n: V3,
	/// Collision face type byte (bits 0-4 surface id, 31 = default; collision.rs).
	pub kind: u8,
	/// The collision cell (4 m, tree indices x, y, z) this copy was stored in; None = visible from every cell it overlaps.
	pub home: Option<[i32; 3]>,
}

impl Tri {
	pub fn surface(&self) -> u8 {
		self.kind & 0x1f
	}

	/// The vertices in the order the game stores them, (v0, v1, v2) with the front normal (v2 - v0) x (v1 - v0): the
	/// reverse of `v` (collision.rs reverses the stored winding; a quad's second half (v0, v2, v3) is stored reversed too).
	pub fn retail(&self) -> [V3; 3] {
		[self.v[2], self.v[1], self.v[0]]
	}

}

/// A segment hit: distance along it, point, face normal (turned to face the segment) and face type.
#[derive(Clone, Copy, Debug)]
pub struct Hit {
	pub t: f32,
	pub p: V3,
	pub kind: u8,
}

#[derive(Clone)]
pub struct World {
	pub tris: Vec<Tri>,
	cell: f32,
	grid: HashMap<(i32, i32), Vec<u32>>,
}

impl World {
	/// Degenerate triangles are dropped. Every face gets the default type (31).
	pub fn new(tris: impl IntoIterator<Item = [V3; 3]>, cell: f32) -> World {
		World::typed(tris.into_iter().map(|v| (v, 31)), cell)
	}

	/// Like `new`, with each face's collision type byte.
	pub fn typed(tris: impl IntoIterator<Item = ([V3; 3], u8)>, cell: f32) -> World {
		World::homed(tris.into_iter().map(|(v, kind)| (v, kind, None)), cell)
	}

	/// Like `typed`, with the cell each copy of a face was stored in (collision.rs `Face::cell`): queries see a copy only
	/// through the cells they overlap, as the game copies one cell at a time (collision_and_gameplay.md "Terrain cell copies").
	pub fn homed(tris: impl IntoIterator<Item = ([V3; 3], u8, Option<[i32; 3]>)>, cell: f32) -> World {
		let mut w = World { tris: Vec::new(), cell, grid: HashMap::new() };
		for (v, kind, home) in tris {
			let n = cross(sub(v[1], v[0]), sub(v[2], v[0]));
			if length(n) < 1e-8 {
				continue;
			}
			let id = w.tris.len() as u32;
			let (x0, x1) = (v.iter().map(|p| p[0]).fold(f32::MAX, f32::min), v.iter().map(|p| p[0]).fold(f32::MIN, f32::max));
			let (y0, y1) = (v.iter().map(|p| p[1]).fold(f32::MAX, f32::min), v.iter().map(|p| p[1]).fold(f32::MIN, f32::max));
			let (gx, gy) = home.map_or((w.c(x0)..=w.c(x1), w.c(y0)..=w.c(y1)), |h| (h[0]..=h[0], h[1]..=h[1]));
			for gx in gx {
				for gy in gy.clone() {
					w.grid.entry((gx, gy)).or_default().push(id);
				}
			}
			w.tris.push(Tri { v, n: normalize(n), kind, home });
		}
		w
	}

	fn c(&self, x: f32) -> i32 {
		(x / self.cell).floor() as i32
	}

	/// Triangle ids stored in the cells that the box `lo`..`hi` overlaps (a copy only through its own cell).
	fn within(&self, lo: V3, hi: V3) -> Vec<u32> {
		let mut out = Vec::new();
		let z = (self.c(lo[2]), self.c(hi[2]));
		for gx in self.c(lo[0])..=self.c(hi[0]) {
			for gy in self.c(lo[1])..=self.c(hi[1]) {
				for &id in self.grid.get(&(gx, gy)).into_iter().flatten() {
					if self.tris[id as usize].home.is_none_or(|h| h[2] >= z.0 && h[2] <= z.1) {
						out.push(id);
					}
				}
			}
		}
		out.sort_unstable();
		out.dedup();
		out
	}


	/// Highest surface hit by a ray going down from `from` within `max` distance: (z, normal).
	/// Only faces with normals pointing up count as ground.
	/// Water faces (surface id 0) are not solid for the hero or the ground ray (hero_swim.md).
	pub fn ground(&self, from: V3, max: f32) -> Option<(f32, V3)> {
		self.ground_where(from, max, |t| t.surface() != 0).map(|(z, n, _)| (z, n))
	}

	/// As `ground`, over the faces `keep` accepts, also returning the face type byte (the water ray keeps
	/// surface id 0 only; the slide probe reads id 4).
	pub fn ground_where(&self, from: V3, max: f32, keep: impl Fn(&Tri) -> bool) -> Option<(f32, V3, u8)> {
		let mut best: Option<(f32, V3, u8)> = None;
		let ids = self.grid.get(&(self.c(from[0]), self.c(from[1])));
		for &id in ids.into_iter().flatten() {
			let t = &self.tris[id as usize];
			if !keep(t) {
				continue;
			}
			let n = t.n; // one-sided: only front faces count (the line query's rule)
			if n[2] < 0.1 {
				continue;
			}
			// Barycentric test in XY, then the plane's Z at that point.
			let [a, b, c] = t.v;
			let d = (b[1] - c[1]) * (a[0] - c[0]) + (c[0] - b[0]) * (a[1] - c[1]);
			if d.abs() < 1e-12 {
				continue;
			}
			let l1 = ((b[1] - c[1]) * (from[0] - c[0]) + (c[0] - b[0]) * (from[1] - c[1])) / d;
			let l2 = ((c[1] - a[1]) * (from[0] - c[0]) + (a[0] - c[0]) * (from[1] - c[1])) / d;
			let l3 = 1.0 - l1 - l2;
			if l1 < 0.0 || l2 < 0.0 || l3 < 0.0 {
				continue;
			}
			let z = l1 * a[2] + l2 * b[2] + l3 * c[2];
			if z <= from[2] && from[2] - z <= max && best.is_none_or(|(bz, _, _)| z > bz) {
				best = Some((z, n, t.kind));
			}
		}
		best
	}


	/// `ray` under a query mask (0x10 two-sided, the 0x80 and 0x20 exclusions).
	pub fn ray_masked(&self, from: V3, to: V3, mask: u32) -> Option<Hit> {
		self.line(from, to, mask)
	}

	/// The terrain half of the line query FUN_001efbf0 / 0x211770: the nearest face the segment crosses from its front
	/// to its back (both ways under mask 0x10), with the unit normal turned to face the start. Winding is the stored
	/// one (`Tri::retail`).
	pub fn line(&self, from: V3, to: V3, mask: u32) -> Option<Hit> {
		let d = sub(to, from);
		let len = length(d);
		if len < 1e-6 {
			return None;
		}
		let mut best: Option<Hit> = None;
		let lo = [0, 1, 2].map(|k| from[k].min(to[k]) - 0.01);
		let hi = [0, 1, 2].map(|k| from[k].max(to[k]) + 0.01);
		for id in self.within(lo, hi) {
			let t = &self.tris[id as usize];
			if qmask::skips(mask, t.kind) {
				continue;
			}
			// The cell walk visits the cells the segment crosses; a copy elsewhere is not seen.
			if t.home.is_some_and(|h| !segment_in_cell(from, to, h, self.cell)) {
				continue;
			}
			let Some((u, n)) = ray_face(&t.retail(), from, to, mask) else { continue };
			if best.is_none_or(|h| u * len < h.t) {
				best = Some(Hit { t: u * len, p: add(from, scale(d, u)),  kind: t.kind,  });
			}
		}
		best
	}


	/// The terrain half of the capsule query FUN_001f1920 / 0x2134a0 (a sphere at `h` = 0, FUN_001f0ce0 / 0x212860):
	/// the nearest face within sqrt(`best`) of the axis `base`..`base + h`, one-sided unless mask 0x10, the mask's
	/// exclusions applied. `best` is the squared distance to beat (r^2 * 0.9995 for the first test of a query).
	pub fn nearest(&self, base: V3, h: f32, r: f32, mut best: f32, mask: u32) -> Option<Overlap> {
		let mut out: Option<Overlap> = None;
		let (lo, hi) = ([base[0] - r, base[1] - r, base[2] - r - 0.01], [base[0] + r, base[1] + r, base[2] + h + r + 0.01]);
		for id in self.within(lo, hi) {
			let t = &self.tris[id as usize];
			if let Some(o) = face_overlap(&t.retail(), t.kind, base, h, r, best, mask) {
				best = o.best;
				out = Some(o);
			}
		}
		out
	}



	/// Pushes a sphere out of the terrain with up to eight sphere queries (mask 0, as a bare FUN_001f0ce0 loop).
	/// Returns the new centre and whether a face with normal z >= `walkable` was touched.
	pub fn push_sphere(&self, centre: V3, radius: f32, walkable: f32) -> (V3, bool) {
		self.push_sphere_masked(centre, radius, walkable, 0)
	}


	/// `push_sphere` under a query mask (the camera passes 0x80).
	pub fn push_sphere_masked(&self, mut centre: V3, radius: f32, walkable: f32, mask: u32) -> (V3, bool) {
		let mut grounded = false;
		for _ in 0..8 {
			let Some(o) = self.nearest(centre, 0.0, radius, radius * radius * OVERLAP, mask) else { break };
			centre = o.to;
			grounded |= o.normal[2] >= walkable;
		}
		(centre, grounded)
	}
}


/// Whether the segment passes through the 4 m collision cell `h` (slab test, 1 mm margin): the cells the line query's
/// cell walk (FUN_001efbf0 / 0x211770) copies.
fn segment_in_cell(from: V3, to: V3, h: [i32; 3], cell: f32) -> bool {
	let (mut t0, mut t1) = (0.0f32, 1.0f32);
	for k in 0..3 {
		let (lo, hi) = (h[k] as f32 * cell - 1e-3, (h[k] + 1) as f32 * cell + 1e-3);
		let d = to[k] - from[k];
		if d.abs() < 1e-9 {
			if from[k] < lo || from[k] > hi {
				return false;
			}
			continue;
		}
		let (mut u, mut v) = ((lo - from[k]) / d, (hi - from[k]) / d);
		if u > v {
			std::mem::swap(&mut u, &mut v);
		}
		t0 = t0.max(u);
		t1 = t1.min(v);
		if t0 > t1 {
			return false;
		}
	}
	true
}



/// Query mask bits of the moby queries FUN_001efbf0 / FUN_001f0ce0 / FUN_001f1920 / FUN_001f27e8 (Novalis
/// 0x211770 / 0x212860 / 0x2134a0 / 0x214368; collision_and_gameplay.md "Collision queries").
pub mod qmask {
	/// Skip terrain.
	pub const NO_TERRAIN: u32 = 0x01;
	/// Mesh faces only, primitives skipped (the collector ignores this bit).
	pub const MESH_ONLY: u32 = 0x02;
	/// Primitive set A (clear: set B).
	pub const SET_A: u32 = 0x04;
	/// Faces count from both sides.
	pub const TWO_SIDED: u32 = 0x10;
	/// Skip the faces whose surface id is `(mask >> 8) & 0x1f`.
	pub const SKIP_SURFACE: u32 = 0x20;
	/// Skip faces whose attr has bit 7.
	pub const SKIP_BIT7: u32 = 0x80;


	/// Whether a face with this attr byte is excluded (bit 7 under 0x80, the material under 0x20).
	pub fn skips(mask: u32, attr: u8) -> bool {
		(mask & SKIP_BIT7 != 0 && attr & 0x80 != 0) || (mask & SKIP_SURFACE != 0 && (attr & 0x1f) as u32 == (mask >> 8) & 0x1f)
	}
}

/// One collision primitive of a moby in world space (moby_format.md "Collision block"): types 1-3 are a sphere or a
/// vertical capsule, type 4 a capsule between two joints, all as the axis segment `a`..`b` and a radius.
#[derive(Clone, Copy, Debug)]
pub struct SolidPrim {
	/// Position in the class's primitive list (the bit tested against `MobySolid::disabled`).
	pub index: u8,
	pub a: V3,
	pub b: V3,
	pub r: f32,
	/// Set bits: 1 = A, 2 = B.
	pub sets: u8,
	/// Line-query shape (record type byte): 1 sphere (types 1 and 2, centre `a`), 3 vertical cylinder (`a` up to
	/// `b.z`), 4 capsule between two joints.
	pub kind: u8,
}

/// One moby's collision as the queries see it: its collision-mesh triangles in world space with their attr byte
/// (bits 0-4 surface id, bit 7 excluded by query mask 0x80) and its primitives. `disabled` is the runtime
/// per-primitive disable mask (moby +0x98); a moby whose +0x94 collision pointer is zero is simply not listed.
#[derive(Clone, Debug, Default)]
pub struct MobySolid {
	pub tris: Vec<([V3; 3], u8)>,
	pub prims: Vec<SolidPrim>,
	pub disabled: u32,
}

/// The queries' contact threshold factor, the constant 0x3f7fdf3b the three query bodies load (0.9995; corrected
/// 2026-10-04: was 0.99951 = 0x3f7fe000). A query starts with best = r^2 * it and every accepted hit sets
/// best = d^2 * it, so a later face must be nearer than the last one by 0.05 %.
pub const OVERLAP: f32 = f32::from_bits(0x3f7fdf3b);

/// PS2 VU0 arithmetic for the query kernels (decomp/notes/collision_and_gameplay.md "VU lane arithmetic"): every add,
/// subtract, multiply, divide and square root rounds toward zero (the VU rounding mode), denormals read and write as
/// zero and an overflow saturates at the largest float. The lane order of the reductions is the VU's: a dot product is
/// ((x x' + y y') + z z') (VMUL, VADDAy, VMADDz), a cross product x = a.y b.z - a.z b.y per lane (VOPMULA/VOPMSUB),
/// a point on a segment A + (B - A) u is one product then one sum (VADDA, VMADDx).
fn vflush(x: f32) -> f32 {
	if x.abs() < f32::MIN_POSITIVE {
		0.0f32.copysign(x)
	} else {
		x
	}
}

/// An exactly (or double-) computed value as the VU stores it: truncated toward zero to f32, flushed, saturated.
fn vround(x: f64) -> f32 {
	let r = x as f32;
	let r = if r.is_infinite() {
		f32::MAX.copysign(r)
	} else if r != 0.0 && (r as f64).abs() > x.abs() {
		f32::from_bits(r.to_bits() - 1)
	} else {
		r
	};
	vflush(r)
}

pub fn vadd1(a: f32, b: f32) -> f32 {
	vround(vflush(a) as f64 + vflush(b) as f64)
}
pub fn vsub1(a: f32, b: f32) -> f32 {
	vround(vflush(a) as f64 - vflush(b) as f64)
}
pub fn vmul(a: f32, b: f32) -> f32 {
	vround(vflush(a) as f64 * vflush(b) as f64)
}
/// VDIV: a / b (a zero divisor saturates to the largest float of the quotient's sign).
pub fn vdiv(a: f32, b: f32) -> f32 {
	if vflush(b) == 0.0 {
		return f32::MAX.copysign(a * b.signum());
	}
	vround(vflush(a) as f64 / vflush(b) as f64)
}
/// VSQRT of |x|.
pub fn vsqrt(x: f32) -> f32 {
	vround((vflush(x).abs() as f64).sqrt())
}
/// VRSQRT: fs / sqrt(|ft|) as one VU op, truncated once (not `vdiv(fs, vsqrt(ft))`, which rounds twice); a zero
/// `ft` saturates to the largest float with the sign of `fs` (collision_and_gameplay.md COL-2).
pub fn vrsqrt(fs: f32, ft: f32) -> f32 {
	let (fs, ft) = (vflush(fs), vflush(ft).abs());
	if ft == 0.0 {
		return f32::MAX.copysign(fs);
	}
	if fs == 0.0 {
		return fs;
	}
	let n = fs.abs();
	// Exact test y^2 * ft <= n^2 on the integer mantissas: y does not exceed the true quotient.
	let parts = |x: f32| -> (u128, i32) {
		let b = x.to_bits();
		let (e, m) = ((b >> 23) & 0xff, b & 0x7f_ffff);
		if e == 0 {
			(m as u128, -149)
		} else {
			((m | 0x80_0000) as u128, e as i32 - 150)
		}
	};
	let le = |y: f32| -> bool {
		let ((my, ey), (mt, et), (mn, en)) = (parts(y), parts(ft), parts(n));
		let (mut l, el, mut r, er) = (my * my * mt, 2 * ey + et, mn * mn, 2 * en);
		if l == 0 {
			return true;
		}
		let (tl, tr) = (128 - l.leading_zeros() as i32 + el, 128 - r.leading_zeros() as i32 + er);
		if tl != tr {
			return tl < tr;
		}
		if el > er {
			l <<= el - er;
		} else {
			r <<= er - el;
		}
		l <= r
	};
	let mut y = (n as f64 / (ft as f64).sqrt()) as f32;
	if !y.is_finite() {
		y = f32::MAX;
	}
	while y > 0.0 && !le(y) {
		y = f32::from_bits(y.to_bits() - 1);
	}
	while y < f32::MAX && le(f32::from_bits(y.to_bits() + 1)) {
		y = f32::from_bits(y.to_bits() + 1);
	}
	vflush(y).copysign(fs)
}
pub fn vadd(a: V3, b: V3) -> V3 {
	[vadd1(a[0], b[0]), vadd1(a[1], b[1]), vadd1(a[2], b[2])]
}
pub fn vsub(a: V3, b: V3) -> V3 {
	[vsub1(a[0], b[0]), vsub1(a[1], b[1]), vsub1(a[2], b[2])]
}
pub fn vscale(a: V3, s: f32) -> V3 {
	[vmul(a[0], s), vmul(a[1], s), vmul(a[2], s)]
}
pub fn vdot(a: V3, b: V3) -> f32 {
	vadd1(vadd1(vmul(a[0], b[0]), vmul(a[1], b[1])), vmul(a[2], b[2]))
}
pub fn vcross(a: V3, b: V3) -> V3 {
	[vsub1(vmul(a[1], b[2]), vmul(a[2], b[1])), vsub1(vmul(a[2], b[0]), vmul(a[0], b[2])), vsub1(vmul(a[0], b[1]), vmul(a[1], b[0]))]
}
/// Unit vector: VSQRT of x.x, then VDIV 1 / that, then a lane multiply (VSQRT, VDIV, VMULq; not VRSQRT). The
/// `1e-18` floor is a port guard (retail has none).
pub fn vnormalize(a: V3) -> V3 {
	vscale(a, vdiv(1.0, vsqrt(vdot(a, a).max(1e-18))))
}


/// What a capsule/sphere query returns for its nearest overlap (the record at 0x174260..).
#[derive(Clone, Copy, Debug)]
pub struct Overlap {
	/// The new best (faces d^2 * OVERLAP, primitives d^2), the threshold the next face must beat.
	pub best: f32,
	/// The pushed-out base of the query (record +0x30).
	pub to: V3,
	/// Unit normal: record +0x40 (raw face cross, or the primitive's raw offset contact - k) normalised as its readers do.
	pub normal: V3,
}

/// Closest point of the edge `e0`..`e1` to `p`, the parameter clamped to 0..1 (the queries' edge fallback; VDIV by a
/// zero length saturates, 0x213ab8 / 0x213c30 / 0x212df0, then VMAX 0 / VMINI 1).
fn edge_point(p: V3, e0: V3, e1: V3) -> V3 {
	let d = vsub(e1, e0);
	let s = vdiv(vdot(vsub(p, e0), d), vdot(d, d)).clamp(0.0, 1.0);
	vadd(e0, vscale(d, s))
}

/// The edge of the triangle (v0-relative edges `a` = v1 - v0, `b` = v2 - v0, n = b x a) whose test fails first for
/// the v0-relative point `p`: v0-v1, v2-v0, v1-v2 (cross . n with its sign bit set, so -0.0 fails too).
fn failing_edge(p: V3, a: V3, b: V3, n: V3) -> Option<(V3, V3)> {
	let z = [0.0; 3];
	if vdot(vcross(p, a), n).is_sign_negative() {
		Some((z, a))
	} else if vdot(vcross(b, p), n).is_sign_negative() {
		Some((b, z))
	} else if vdot(vcross(vsub(p, a), vsub(b, a)), n).is_sign_negative() {
		Some((a, b))
	} else {
		None
	}
}

/// The point of the triangle the capsule kernel pairs with the v0-relative axis point `p` and their squared distance
/// (0x213af0-0x213c60): the foot p - n Q with Q = (p . n) / (n . n); the first failing edge sends it to that edge's
/// clamped projection (d2 = |q - p|^2), otherwise the foot is the contact (d2 = |n Q|^2). v0-relative.
fn tri_point(p: V3, a: V3, b: V3, n: V3) -> (V3, f32) {
	let nq = vscale(n, vdiv(vdot(p, n), vdot(n, n)));
	let foot = vsub(p, nq);
	match failing_edge(foot, a, b, n) {
		Some((e0, e1)) => {
			let q = edge_point(foot, e0, e1);
			let e = vsub(q, p);
			(q, vdot(e, e))
		}
		None => (foot, vdot(nq, nq)),
	}
}

/// The axis point the capsule query pairs with a face (0x2134a0, branch order from the raw words 0x213934-0x213c4c),
/// v0-relative: `rel` = base - v0, `top_rel` = rel + (0, 0, h). A face with |n.z| < 2^-20 m^2 (degenerate in the
/// 1/1024 m units) uses the axis point at v0's height; an axis whose base is on or above every vertex (sign bits of
/// rel.z, rel.z - a.z, rel.z - b.z clear) uses the base, one whose top is below every vertex (all three sign bits of
/// the top set) the top; otherwise the point p where the axis crosses the face plane (q = (n . rel) / (h n.z),
/// p.z = rel.z - h q), moved to the nearest edge point if outside, and the axis point at that height
/// (rel.z + clamp(z - rel.z, 0, h)).
fn axis_point(rel: V3, top_rel: V3, a: V3, b: V3, n: V3, h: f32) -> V3 {
	if h <= 0.0 {
		return rel;
	}
	let at = |z: f32| [rel[0], rel[1], vadd1(rel[2], vsub1(z, rel[2]).min(h).max(0.0))];
	if n[2].abs() * 1048576.0 < 1.0 {
		return at(0.0);
	}
	let signs = |z: f32| [z, vsub1(z, a[2]), vsub1(z, b[2])].map(|x| x.is_sign_negative());
	if signs(rel[2]).iter().all(|&s| !s) {
		return rel;
	}
	if signs(top_rel[2]).iter().all(|&s| s) {
		return top_rel;
	}
	let q = vdiv(vdot(n, rel), vmul(h, n[2]));
	let p = [rel[0], rel[1], vsub1(rel[2], vmul(h, q))];
	let q = failing_edge(p, a, b, n).map_or(p, |(e0, e1)| edge_point(p, e0, e1));
	at(q[2])
}

/// One face against the capsule kernel, v0-relative: (v0, axis point, contact, squared distance, n = b x a).
fn face_parts(t: &[V3; 3], attr: u8, base: V3, h: f32, mask: u32) -> Option<(V3, V3, V3, f32, V3)> {
	if qmask::skips(mask, attr) {
		return None;
	}
	let [v0, v1, v2] = *t;
	let (a, b) = (vsub(v1, v0), vsub(v2, v0));
	let n = vcross(b, a);
	if vdot(n, n) < 1e-30 {
		return None;
	}
	let h = h.max(0.0);
	let rel = vsub(base, v0);
	let top_rel = [rel[0], rel[1], vadd1(rel[2], h)];
	if mask & qmask::TWO_SIDED == 0 && vdot(n, rel).is_sign_negative() && vdot(n, top_rel).is_sign_negative() {
		return None;
	}
	let axis = axis_point(rel, top_rel, a, b, n, h);
	let (q, d2) = tri_point(axis, a, b, n);
	Some((v0, axis, q, d2, n))
}


/// The query record's pushed-out base (+0x30): contact + (base + c - contact) * r / sqrt(best) - c with
/// c = (0, 0, clamp(contact.z - base.z, 0, h)) (0x2142c8; r / sqrt(best) is one VRSQRT, 0x213418 / 0x2142d0).
fn pushed(base: V3, h: f32, r: f32, contact: V3, best: f32) -> V3 {
	let c = [0.0, 0.0, (contact[2] - base[2]).clamp(0.0, h.max(0.0))];
	vsub(vadd(contact, vscale(vsub(vadd(base, c), contact), vrsqrt(r, best))), c)
}

/// One face against a capsule query: the overlap if it is within sqrt(`best`) (d2 in the v0-relative frame, the face
/// best factor 0.9995).
fn face_overlap(t: &[V3; 3], attr: u8, base: V3, h: f32, r: f32, best: f32, mask: u32) -> Option<Overlap> {
	let (v0, _, q, d2, n) = face_parts(t, attr, base, h, mask)?;
	if d2 > best || d2 < 1e-12 {
		return None;
	}
	let next = vmul(d2, OVERLAP);
	let contact = vadd(v0, q);
	Some(Overlap { best: next, to: pushed(base, h, r, contact, next), normal: vnormalize(n),   })
}

/// The primitive half of the sphere / capsule kernels (0x2132a8-0x213470): `k` the closest primitive-axis point,
/// `d` = query axis point - k, `pr` the primitive radius, `r` the query radius. None when (r + pr)^2 - d.d < 0;
/// otherwise (contact, raw normal) with raw = (d / |d|) * min(|d|, pr) (vf21.x = 1.0: no 0.9995 here) and
/// contact = k + raw. The `|d| < 1e-6` guard is the port's (retail would divide by zero).
fn prim_contact(k: V3, d: V3, pr: f32, r: f32) -> Option<(V3, V3)> {
	if length(d) < 1e-6 {
		return None;
	}
	let rr = vadd1(r, pr);
	let d2 = vdot(d, d);
	if vsub1(vmul(rr, rr), d2) < 0.0 {
		return None;
	}
	let l = vsqrt(d2);
	let inv = vdiv(1.0, l);
	let raw = vscale(vscale(d, inv), l.min(pr));
	Some((vadd(k, raw), raw))
}

/// One face of the line query (retail winding): (segment parameter 0..1, unit normal facing the start). One-sided
/// needs (v0 - s).n < 0 and (v0 - e).n > 0 for n = (v2 - v0) x (v1 - v0); mask 0x10 takes both directions; the hit
/// lies inside the three edge crosses.
fn ray_face(t: &[V3; 3], from: V3, to: V3, mask: u32) -> Option<(f32, V3)> {
	let d = vsub(to, from);
	let n = vcross(vsub(t[2], t[0]), vsub(t[1], t[0]));
	let (ds, de) = (vdot(vsub(t[0], from), n), vdot(vsub(t[0], to), n));
	if mask & qmask::TWO_SIDED == 0 && !(ds < 0.0 && de > 0.0) {
		return None;
	}
	let denom = vdot(d, n);
	if denom.abs() < 1e-12 {
		return None;
	}
	let u = vdiv(ds, denom);
	if !(0.0..=1.0).contains(&u) {
		return None;
	}
	let p = vadd(from, vscale(d, u));
	if !(0..3).all(|i| vdot(vcross(vsub(t[(i + 1) % 3], t[i]), vsub(p, t[i])), n) <= 0.0) {
		return None;
	}
	let nn = vnormalize(n);
	Some((u, if vdot(nn, d) > 0.0 { vscale(nn, -1.0) } else { nn }))
}

impl MobySolid {
	fn prims_for(&self, mask: u32) -> impl Iterator<Item = &SolidPrim> {
		let set = if mask & qmask::SET_A != 0 { 1 } else { 2 };
		self.prims.iter().filter(move |p| p.sets & set != 0 && self.disabled >> p.index.min(31) & 1 == 0)
	}

	/// The nearest overlap of a vertical capsule (axis `base`..`base + h`, radius `r`; h = 0 is the sphere query
	/// FUN_001f0ce0) with this moby, as FUN_001f1920 finds it, beating `best` (the squared distance, r^2 * 0.9995 at the
	/// start of a query). Faces use `face_parts`; a primitive contributes K + (c - K)/|c - K| * min(|c - K|, pr) for K
	/// the closest primitive-axis point (`prim_contact`), best = d^2 with no 0.9995 (COL-2d).
	pub fn nearest(&self, base: V3, h: f32, r: f32, mut best: f32, mask: u32) -> Option<Overlap> {
		let mut out: Option<Overlap> = None;
		for &(t, attr) in &self.tris {
			if let Some(o) = face_overlap(&t, attr, base, h, r, best, mask) {
				best = o.best;
				out = Some(o);
			}
		}
		if mask & qmask::MESH_ONLY == 0 {
			for p in self.prims_for(mask) {
				let (axis, k) = closest_segments(base, [base[0], base[1], base[2] + h], p.a, p.b);
				let Some((contact, raw)) = prim_contact(k, vsub(axis, k), p.r, r) else { continue };
				let e = vsub(axis, contact);
				let d2 = vdot(e, e);
				if d2 > best || d2 < 1e-12 {
					continue;
				}
				best = d2;
				out = Some(Overlap { best, to: pushed(base, h, r, contact, best), normal: vnormalize(raw),   });
			}
		}
		out
	}



	/// First hit of the segment `from`..`to` with this moby (FUN_001efbf0's moby part): faces need the segment to
	/// cross from the front side to the back (both ways under 0x10), primitives (unless mask 2) use `ray_prim`; faces
	/// then primitives share the running best parameter. Returns the hit distance, point and unit normal.
	pub fn ray(&self, from: V3, to: V3, mask: u32) -> Option<Hit> {
		let d = sub(to, from);
		let len = length(d);
		if len < 1e-9 {
			return None;
		}
		let mut best: Option<Hit> = None;
		// Faces and primitives share the best parameter (initially 1.0, vf24.x) and need to beat it strictly.
		let mut best_u = 1.0f32;
		for &(t, attr) in &self.tris {
			if !qmask::skips(mask, attr) {
				if let Some((u, n)) = ray_face(&t, from, to, mask) {
					if u < best_u {
						best_u = u;
						best = Some(Hit { t: u * len, p: add(from, scale(d, u)),  kind: attr,  });
					}
				}
			}
		}
		if mask & qmask::MESH_ONLY == 0 {
			for pr in self.prims_for(mask) {
				if let Some((u, p, n)) = ray_prim(pr.kind, from, to, pr.a, pr.b, pr.r, best_u) {
					best_u = u;
					best = Some(Hit { t: u * len, p,  kind: 0,  });
				}
			}
		}
		best
	}
}

/// The line kernel's cylinder floor on |d.xy|^2 (retail 1/64 in 1/1024 m units = 2^-26 m^2, 0x2124ec).
const CYL_FLOOR: f32 = f32::from_bits(0x3280_0000);
/// The line kernel's parallel-capsule lane value (retail 1.0 in 1/1024 m units = 2^-40, 0x212360).
const PARALLEL_LANE: f32 = f32::from_bits(0x2b80_0000);

/// Segment-vs-primitive sweep of the line kernel (FUN_00211770 0x2122a0-0x212720): `kind` 1 sphere at `a`, 3 vertical
/// cylinder from `a` up to `b.z`, 4 capsule between the joints `a` and `b`; kinds 3 and 4 reduce the query to one
/// sphere at a derived axis point, then the common sphere-segment test. Returns (segment parameter s, hit point,
/// raw normal P - centre) when s is strictly below `best` (s clamped to 0..1: a segment starting inside hits at 0).
pub(crate) fn ray_prim(kind: u8, from: V3, to: V3, a: V3, b: V3, r: f32, best: f32) -> Option<(f32, V3, V3)> {
	let d = vsub(to, from);
	let centre = match kind {
		3 => {
			let h = vsub1(b[2], a[2]);
			let cf = vsub(a, from);
			let axy = vadd1(vmul(d[0], d[0]), vmul(d[1], d[1])).max(CYL_FLOOR);
			let bxy = vadd1(vmul(cf[0], d[0]), vmul(cf[1], d[1]));
			let cxy = vadd1(vmul(cf[0], cf[0]), vmul(cf[1], cf[1]));
			let r2 = vmul(r, r);
			let t = vdiv(bxy, axy).clamp(0.0, 1.0);
			let (x, y) = (vsub1(vadd1(from[0], vmul(d[0], t)), a[0]), vsub1(vadd1(from[1], vmul(d[1], t)), a[1]));
			if vadd1(vsub1(vmul(x, x), r2), vmul(y, y)) > 0.0 {
				return None;
			}
			let b2 = vadd1(bxy, bxy);
			let dd = vsub1(vmul(b2, b2), vmul(vmul(4.0, axy), vsub1(cxy, r2))).max(0.0);
			let s = vsub1(0.0, vdiv(vadd1(vsub1(0.0, b2), vsqrt(dd)), vadd1(axy, axy))).clamp(0.0, 1.0);
			let pz = vadd1(from[2], vmul(d[2], s));
			[a[0], a[1], pz.max(a[2]).min(vadd1(a[2], h))]
		}
		4 => {
			let e = vsub(b, a);
			let r2 = vmul(r, r);
			let n1 = vcross(e, d);
			let n2 = vcross(d, e);
			let n9 = [vmul(n1[0], n1[0]), vmul(n1[1], n1[1]), vmul(n1[2], n1[2])];
			let p = if n9.iter().all(|&x| x == PARALLEL_LANE) {
				from
			} else {
				let (v7, v8) = (vcross(d, n1), vcross(e, n2));
				let den = vadd1(vadd1(n9[0], n9[1]), n9[2]);
				let sa = vdiv(vdot(v7, vsub(from, a)), den);
				let pa = vadd(a, vscale(e, sa));
				let ta = vdiv(vdot(v8, vsub(a, from)), den);
				let pq = vadd(from, vscale(d, ta));
				let df = vsub(pq, pa);
				let d2 = vadd1(vadd1(vadd1(-r2, vmul(df[0], df[0])), vmul(df[1], df[1])), vmul(df[2], df[2]));
				if !d2.is_sign_negative() {
					return None;
				}
				let eh = vscale(e, vrsqrt(1.0, vdot(e, e)));
				let dh = vscale(d, vrsqrt(1.0, vdot(d, d)));
				let cs = vdot(eh, dh);
				let sn = vsub1(1.0, vmul(cs, cs));
				let hh = vsqrt(vdiv(vsub1(0.0, d2), sn).max(0.0));
				let ent = vsub(pq, vscale(dh, hh));
				if vdot(vsub(ent, from), d).is_sign_negative() {
					from
				} else {
					ent
				}
			};
			let u = vdiv(vdot(vsub(p, a), e), vdot(e, e)).clamp(0.0, 1.0);
			vadd(a, vscale(e, u))
		}
		_ => a,
	};
	// The common sphere-segment test (0x212620-0x212718).
	let aa = vdot(d, d);
	let cf = vsub(centre, from);
	let bb = vdot(cf, d);
	let c2 = vdot(cf, cf);
	let r2 = vmul(r, r);
	let t = vdiv(bb, aa).clamp(0.0, 1.0);
	let q = vsub(vadd(from, vscale(d, t)), centre);
	if vadd1(vadd1(vsub1(vmul(q[0], q[0]), r2), vmul(q[1], q[1])), vmul(q[2], q[2])) > 0.0 {
		return None;
	}
	let b2 = vadd1(bb, bb);
	let dd = vsub1(vmul(b2, b2), vmul(vmul(4.0, aa), vsub1(c2, r2))).max(0.0);
	let s = vsub1(0.0, vdiv(vadd1(vsub1(0.0, b2), vsqrt(dd)), vadd1(aa, aa))).clamp(0.0, 1.0);
	if vsub1(best, s) <= 0.0 {
		return None;
	}
	let p = vadd(from, vscale(d, s));
	Some((s, p, vsub(p, centre)))
}


/// Closest points of the segments p1-q1 and p2-q2 (Ericson 5.1.9): (point on the first, point on the second).
pub(crate) fn closest_segments(p1: V3, q1: V3, p2: V3, q2: V3) -> (V3, V3) {
	let (d1, d2, r) = (sub(q1, p1), sub(q2, p2), sub(p1, p2));
	let (a, e, f) = (dot(d1, d1), dot(d2, d2), dot(d2, r));
	let (s, t);
	if a <= 1e-12 && e <= 1e-12 {
		(s, t) = (0.0, 0.0);
	} else if a <= 1e-12 {
		(s, t) = (0.0, (f / e).clamp(0.0, 1.0));
	} else {
		let c = dot(d1, r);
		if e <= 1e-12 {
			(s, t) = ((-c / a).clamp(0.0, 1.0), 0.0);
		} else {
			let b = dot(d1, d2);
			let denom = a * e - b * b;
			let mut s0 = if denom > 1e-12 { ((b * f - c * e) / denom).clamp(0.0, 1.0) } else { 0.0 };
			let mut t0 = (b * s0 + f) / e;
			if t0 < 0.0 {
				t0 = 0.0;
				s0 = (-c / a).clamp(0.0, 1.0);
			} else if t0 > 1.0 {
				t0 = 1.0;
				s0 = ((b - c) / a).clamp(0.0, 1.0);
			}
			(s, t) = (s0, t0);
		}
	}
	(add(p1, scale(d1, s)), add(p2, scale(d2, t)))
}





/// Terrain plus moby solids as one query target (the moby grid walk is a plain loop here: each solid is visited
/// once per query, which is what the collector's query-ID dedupe achieves in the game).
pub struct Scene<'a> {
	pub world: &'a World,
	pub solids: &'a [MobySolid],
}

impl Scene<'_> {
	/// FUN_00211770 / FUN_001efbf0: the nearest hit of the segment, terrain unless mask 1 (faces one-sided unless
	/// mask 0x10), `skip` the querying moby's own solid.
	pub fn line(&self, from: V3, to: V3, mask: u32, skip: Option<usize>) -> Option<Hit> {
		self.line_solid(from,to,mask,skip).map(|(h,_)|h)
	}

	pub fn line_solid(&self, from: V3, to: V3, mask: u32, skip: Option<usize>) -> Option<(Hit,Option<usize>)> {
		let mut best = if mask & qmask::NO_TERRAIN == 0 { self.world.line(from, to, mask).map(|h|(h,None)) } else { None };
		for (i, s) in self.solids.iter().enumerate() {
			if Some(i) != skip {
				if let Some(h) = s.ray(from, to, mask) {
					if best.map_or(true, |b| h.t < b.0.t) {
						best = Some((h,Some(i)));
					}
				}
			}
		}
		best
	}

	/// FUN_001f1920 / FUN_002134a0 (a sphere at `h` = 0, FUN_001f0ce0 / FUN_00212860): the single nearest overlap of
	/// the vertical capsule `base`..`base + h` over terrain (unless mask 1) and every solid, with the solid's index
	/// (None = terrain). One `best` runs through the whole query, as the VU register does.
	pub fn capsule(&self, base: V3, h: f32, r: f32, mask: u32, skip: Option<usize>) -> Option<(Overlap, Option<usize>)> {
		let mut best = r * r * OVERLAP;
		let mut out = None;
		if mask & qmask::NO_TERRAIN == 0 {
			if let Some(o) = self.world.nearest(base, h, r, best, mask) {
				best = o.best;
				out = Some((o, None));
			}
		}
		for (i, s) in self.solids.iter().enumerate() {
			if Some(i) != skip {
				if let Some(o) = s.nearest(base, h, r, best, mask) {
					best = o.best;
					out = Some((o, Some(i)));
				}
			}
		}
		out
	}





}




#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn vu_arithmetic_truncates_and_flushes() {
		// 1 + 1.5 * 2^-24 rounds up in IEEE nearest; the VU truncates
		let e = 2f32.powi(-24) * 1.5;
		assert_eq!(vadd1(1.0, e), 1.0);
		assert_eq!(vmul(1e-30, 1e-30), 0.0);
		assert_eq!(vdot([1.0, 2.0, 3.0], [4.0, 5.0, 6.0]), 32.0);
		assert_eq!(vcross([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]), [0.0, 0.0, 1.0]);
	}

	fn floor() -> World {
		World::new([[[-10.0, -10.0, 0.0], [10.0, -10.0, 0.0], [10.0, 10.0, 0.0]], [[-10.0, -10.0, 0.0], [10.0, 10.0, 0.0], [-10.0, 10.0, 0.0]]], 4.0)
	}

	#[test]
	fn ground_ray_hits_floor() {
		let w = floor();
		assert_eq!(w.ground([1.0, 2.0, 5.0], 10.0).map(|g| g.0), Some(0.0));
		assert!(w.ground([1.0, 2.0, 5.0], 1.0).is_none());
		assert!(w.ground([1.0, 2.0, -1.0], 10.0).is_none());
	}

	#[test]
	fn sphere_rests_on_floor() {
		let (c, grounded) = floor().push_sphere([0.0, 0.0, 0.3], 0.5, 0.7);
		// The record stretches the push by 1/sqrt(0.9995): rests 0.00013 above r.
		assert!((c[2] - 0.5).abs() < 1e-3 && c[2] >= 0.5 && grounded);
	}

	/// Two faces wound so the front normal (v2 - v0) x (v1 - v0) points along `n`.
	fn quad(c: [V3; 4], n: V3, attr: u8) -> Vec<([V3; 3], u8)> {
		[[c[0], c[1], c[2]], [c[0], c[2], c[3]]]
			.into_iter()
			.map(|t| (if dot(cross(sub(t[2], t[0]), sub(t[1], t[0])), n) >= 0.0 { t } else { [t[0], t[2], t[1]] }, attr))
			.collect()
	}

	/// A box x, y in -1..1 and z in 0..1 (top, +x and -x faces only), outward front faces, all with `attr`.
	fn crate_solid(attr: u8, height: f32) -> MobySolid {
		let h = height;
		let mut s = MobySolid::default();
		s.tris.extend(quad([[-1.0, -1.0, h], [1.0, -1.0, h], [1.0, 1.0, h], [-1.0, 1.0, h]], [0.0, 0.0, 1.0], attr));
		s.tris.extend(quad([[1.0, -1.0, 0.0], [1.0, 1.0, 0.0], [1.0, 1.0, h], [1.0, -1.0, h]], [1.0, 0.0, 0.0], attr));
		s.tris.extend(quad([[-1.0, -1.0, 0.0], [-1.0, 1.0, 0.0], [-1.0, 1.0, h], [-1.0, -1.0, h]], [-1.0, 0.0, 0.0], attr));
		s
	}

	/// Faces are one-sided unless the mask has 0x10.
	#[test]
	fn faces_are_one_sided_unless_masked() {
		let s = crate_solid(0x1f, 1.0);
		let near = |mask| s.nearest([0.9, 0.0, 0.5], 0.0, 0.2, 0.04 * OVERLAP, mask).is_some();
		assert!(!near(qmask::SET_A) && near(qmask::SET_A | qmask::TWO_SIDED));
	}

	/// Primitives obey the set bit, the +0x98 disable mask and mask bit 2.
	#[test]
	fn primitives_follow_set_and_disable_mask() {
		let prim = |index, sets, a: V3, b: V3| SolidPrim { index, a, b, r: 0.5, sets, kind: 3 };
		let mut s = MobySolid { prims: vec![prim(0, 1, [0.0, 0.0, 0.5], [0.0, 0.0, 1.0]), prim(1, 2, [3.0, 0.0, 0.5], [3.0, 0.0, 0.5])], ..Default::default() };
		let hit = |s: &MobySolid, c: V3, mask| s.nearest(c, 0.0, 0.45, 0.45 * 0.45 * OVERLAP, mask).is_some();
		assert!(hit(&s, [0.8, 0.0, 0.7], qmask::SET_A) && !hit(&s, [0.8, 0.0, 0.7], 0));
		assert!(hit(&s, [3.8, 0.0, 0.5], 0) && !hit(&s, [3.8, 0.0, 0.5], qmask::SET_A));
		assert!(!hit(&s, [0.8, 0.0, 0.7], qmask::SET_A | qmask::MESH_ONLY));
		s.disabled = 1;
		assert!(!hit(&s, [0.8, 0.0, 0.7], qmask::SET_A) && hit(&s, [3.8, 0.0, 0.5], 0), "bit 0 disables primitive 0 only");
	}

	fn bits(v: V3) -> [u32; 3] {
		v.map(f32::to_bits)
	}
	fn fb(b: [u32; 3]) -> V3 {
		b.map(f32::from_bits)
	}

	/// COL-2: VRSQRT is one truncation (the two-op chain differs by one ulp); scaling by 1024 keeps the mantissa.
	#[test]
	fn vrsqrt_is_one_rounding() {
		assert_eq!(vrsqrt(1.0, 5.0).to_bits(), 0x3ee4f92e);
		assert_eq!(vdiv(1.0, vsqrt(5.0)).to_bits(), 0x3ee4f92f);
		assert_eq!(vrsqrt(1.0, 6.0).to_bits(), 0x3ed105eb);
		assert_eq!(vdiv(1.0, vsqrt(6.0)).to_bits(), 0x3ed105ec);
		assert_eq!(vmul(3.1, 1.7).to_bits(), 0x40a8a3d7);
		assert_eq!(vmul(3.1 * 1024.0, 1.7 * 1024.0).to_bits(), 0x4aa8a3d7);
		assert_eq!(vrsqrt(1.0, 0.0), f32::MAX);
		let best = f32::from_bits(0x3ca2711e);
		assert_eq!(vrsqrt(f32::from_bits(0x3ecccccd), best).to_bits(), 0x4035cbf3);
		assert_eq!((f32::from_bits(0x3ecccccd) / best.sqrt()).to_bits(), 0x4035cbf4);
	}

	/// COL-2b / COL-2d: primitive contact with m = min(l, pr), best = d2 (no 0.9995), raw normal normalised.
	#[test]
	fn prim_contact_bits() {
		let d = fb([0xbe20fe9a, 0x3cd9653f, 0xbd9fcc91]);
		let (c, raw) = prim_contact([0.0; 3], d, 0.5, 0.5).unwrap();
		assert_eq!(bits(c), [0xbe20fe99, 0x3cd9653e, 0xbd9fcc90]);
		assert_eq!(bits(raw), bits(c));
		let e = vsub(d, c);
		assert_eq!(bits(e), [0xb2800000, 0x31000000, 0xb2000000]);
		assert_eq!(vdot(e, e).to_bits(), 0x25a20000);
		assert_eq!(bits(vnormalize(raw)), [0xbf62bc68, 0x3e191581, 0xbee10d67]);
		let d = [0.6, 0.2, 0.1];
		let (c, raw) = prim_contact([0.0; 3], d, 0.3, 0.5).unwrap();
		assert_eq!(bits(c), [0x3e8fee03, 0x3dbfe805, 0x3d3fe805]);
		assert_eq!(bits(raw), bits(c));
		let e = vsub(d, c);
		assert_eq!(bits(e), [0x3ea34531, 0x3dd9b195, 0x3d59b195]);
		assert_eq!(vdot(e, e).to_bits(), 0x3ded2f23);
		assert_eq!(bits(vnormalize(raw)), [0x3f6fe206, 0x3e9fec05, 0x3e1fec05]);
		assert!(prim_contact([0.0; 3], [1.0, 0.0, 0.0], 0.5, 0.4).is_none(), "d2 1.0 > 0.9^2");
	}

	/// COL-2d (d): a zero-length edge saturates the divide and clamps to its start.
	#[test]
	fn edge_point_zero_edge() {
		let e0 = [1.0, 2.0, 3.0];
		assert_eq!(edge_point([5.0, 5.0, 5.0], e0, e0), e0);
	}

	/// COL-2d (h): the capsule kernel's axis point selection, v0-relative sign-bit tests.
	#[test]
	fn axis_point_branches() {
		let ap = |t: [V3; 3], base: V3, h: f32| {
			let (a, b) = (vsub(t[1], t[0]), vsub(t[2], t[0]));
			let n = vcross(b, a);
			let rel = vsub(base, t[0]);
			vadd(t[0], axis_point(rel, [rel[0], rel[1], vadd1(rel[2], h)], a, b, n, h))
		};
		let t = [[0.0, 0.0, 0.0], [0.0, 2.0, 0.0], [2.0, 0.0, 2.0]];
		assert_eq!(ap(t, [0.5, 0.5, 5.0], 1.0), [0.5, 0.5, 5.0]);
		assert_eq!(ap(t, [0.5, 0.5, -6.0], 1.0), [0.5, 0.5, -5.0]);
		assert_eq!(ap(t, [0.5, 0.5, -1.0], 2.0), [0.5, 0.5, 0.5]);
		assert_eq!(ap(t, [4.0, 0.5, -1.0], 2.0), [4.0, 0.5, 1.0]);
		let v = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 1.0]];
		assert_eq!(ap(v, [0.3, 0.4, -0.5], 2.0), [0.3, 0.4, 0.0]);
	}

	/// COL-2c: the line kernel's primitive sweep (sphere, vertical cylinder, joint capsule), f32 bits.
	#[test]
	fn ray_prim_bits() {
		let r = |kind, from: V3, to: V3, a: V3, b: V3, rad| ray_prim(kind, from, to, a, b, rad, 1.0).map(|(s, p, n)| (s.to_bits(), bits(p), bits(n)));
		let c = [0.0, 0.0, 1.0];
		assert_eq!(r(1, [0.3, -0.2, 3.1], [0.1, 0.15, -1.7], c, c, 0.5), Some((0x3eb16eee, [0x3e6c3a08, 0xbda13181, 0x3fb7e149], [0x3e6c3a08, 0xbda13181, 0x3edf8524])));
		let from = [0.1, 0.0, 1.1];
		assert_eq!(r(1, from, [0.0, 0.0, 3.0], c, c, 0.5), Some((0, bits(from), [0x3dcccccd, 0, 0x3dccccd0])));
		assert_eq!(r(1, [0.0, 0.0, 3.0], [0.0, 0.0, 1.5], c, c, 0.5), None, "s = 1.0 is not below best");
		let (base, top) = ([0.0, 0.0, 0.0], [0.0, 0.0, 1.0]);
		assert_eq!(r(3, [3.0, 0.2, 0.5], [-3.0, 0.1, 0.5], base, top, 0.5), Some((0x3ed7845a, [0x3ef2e5e8, 0x3e21b254, 0x3f000000], [0x3ef2e5e8, 0x3e21b254, 0])));
		let cap = r(3, [0.1, 0.0, 3.0], [0.1, 0.0, -1.0], base, top, 0.5).unwrap();
		assert_eq!((cap.0, cap.1[2]), (0x3ec14b04, 0x3fbeb4fc));
		let (a, b) = ([0.0, 0.0, 0.0], [1.0, 0.2, 0.1]);
		assert_eq!(r(4, [0.5, 3.0, 0.3], [0.4, -3.0, 0.35], a, b, 0.5), Some((0x3ed3486b, [0x3eeadf28, 0x3f0626c0, 0x3ea42a05], [0xbdde3928, 0x3ed2379c, 0x3e871f13])));
		let end = r(4, [-2.0, 0.1, 0.05], [2.0, 0.1, 0.05], a, b, 0.5).unwrap();
		assert_eq!((end.0, end.1), (0x3ec19ed4, [0xbef984b0, 0x3dcccccd, 0x3d4ccccd]));
	}
}
