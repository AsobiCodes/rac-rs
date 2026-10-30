//! Always-on tests of the fast path (lane nfalls, review fix 2026-10-30): the old and the fast `sheet_frame` side by side on synthetic
//! waterfall sheets, bit for bit (vertices, `falls` and everything `SheetMem` keeps), and the claims the exactness argument of
//! `fast.rs` stands on. They need no recorded log (`replay_tests.rs` replays a real one on demand) and take a few seconds in
//! `cargo test --release`.
//! - `stress_sheets`: brink / fall / skirt sheets at vertex spacings 0.37, 0.25, 0.19 and 0.181 (the nearest neighbours right at
//!   SHEET_SNAP), jitter from none to 0.3 per vertex and per corner (new vertices mid-frame), dropped, inserted and rotated slot
//!   lists, refill candidates (also at unseen places), and a level change (`WaterFx::sheet_falls` starts empty on another level).
//! - `stress_runaway_reset`: positions that never settle cross the 20,000-vertex reset (`observe`), then a settled frame repeats.
//! - `stress_nonfinite`: NaN corners, and huge coordinates (infinity and -3e9 only in a release build: `search` and `shape_faces`
//!   of the old code overflow a cell index there, as in dev; the lane's own code does not).
//! - `certified_implies_search`: `lim` against a brute force, and `certified` => the vertex is what `search` answers, `search_fast` ==
//!   `search`, `id_cached` == `search`, on a dense cloud and probes at the certified radius.
//! - `search_fast_at_snap_edge`: `search_fast` against `search` for probes within 2e-6 of SHEET_SNAP.
//! With RAC_FALL_FAST=0 (`on()`) no `lim` is kept: the tests of `lim` then say so and return.

use super::super::replay_tests::{fbits, mem_bits, vbits, Quad, Run};
use super::*;

struct Rng(u64);

impl Rng {
	fn u(&mut self) -> u64 {
		self.0 ^= self.0 << 13;
		self.0 ^= self.0 >> 7;
		self.0 ^= self.0 << 17;
		self.0
	}
	/// 0 .. 1.
	fn f(&mut self) -> f32 {
		(self.u() >> 40) as f32 / (1u64 << 24) as f32
	}
	/// -1 .. 1.
	fn s(&mut self) -> f32 {
		self.f() * 2.0 - 1.0
	}
	fn n(&mut self, k: usize) -> usize {
		(self.u() % k as u64) as usize
	}
}

/// A sheet quad's corners (p3 == p2) and whether it is offered as a refill candidate instead of drawn.
type Q = ([[f32; 3]; 4], bool);

fn step(run: &mut Run, quads: &[Q], fast: bool) {
	let quad = |&(p, _): &Q| -> Quad { (p, [[0.0; 2]; 4], [[1.0; 4]; 4]) };
	let drawn: Vec<Quad> = quads.iter().filter(|q| !q.1).map(quad).collect();
	let refill: Vec<Quad> = quads.iter().filter(|q| q.1).map(quad).collect();
	run.step(&drawn, &refill, fast);
}

fn same(a: &Run, b: &Run, ctx: &str) {
	assert!(vbits(&a.verts) == vbits(&b.verts), "{ctx}: vertices ({} vs {})", a.verts.len(), b.verts.len());
	assert!(fbits(&a.falls) == fbits(&b.falls), "{ctx}: falls");
	// (verts, light, steep, feet_seen, feet, last_ids, faces, known)
	let (ma, mb) = (mem_bits(&a.mem), mem_bits(&b.mem));
	assert!(ma.0 == mb.0, "{ctx}: mem verts ({} vs {})", a.mem.verts.len(), b.mem.verts.len());
	assert!(ma.5 == mb.5, "{ctx}: last_ids");
	assert!(ma.6 == mb.6, "{ctx}: faces");
	assert!(ma.7 == mb.7, "{ctx}: known");
	assert!(ma.2 == mb.2, "{ctx}: steep");
	assert!(ma.1 == mb.1, "{ctx}: light");
	assert!(ma.3 == mb.3 && ma.4 == mb.4, "{ctx}: feet");
}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
	(a[0] - b[0]).hypot(a[1] - b[1]).hypot(a[2] - b[2])
}

/// The `lim` invariant for about `samples` vertices, against a brute force over all of them: one entry per vertex, and at most the
/// bound from the real nearest neighbour (`min(CAP, nn / 2)`, less the margins of `new_vertex_limits`).
fn check_lim(m: &SheetMem, samples: usize) {
	let n = m.verts.len();
	assert_eq!(m.lim.len(), n, "one lim per vertex");
	for i in (0..n).step_by(n / samples + 1) {
		let v = m.verts[i];
		let nn = (0..n).filter(|&j| j != i).map(|j| dist(v, m.verts[j])).fold(f32::MAX, f32::min);
		let want = CAP.powi(2).min((nn * 0.5 * 0.99 * 0.999).powi(2));
		assert!(m.lim[i] <= want * 1.000001, "lim[{i}] = {} > {want} (nn {nn})", m.lim[i]);
	}
}

/// A sheet like the fixture of `tests::sheet`: a flat run, a brink, a steep fall and a flat skirt; vertex spacing `sp`, each vertex
/// moved by up to `noise`. Returns the vertex positions and the triangles (vertex indices).
fn sheet(r: &mut Rng, (w, h): (usize, usize), sp: f32, off: [f32; 3], noise: f32) -> (Vec<[f32; 3]>, Vec<[usize; 3]>) {
	let mut pos = Vec::new();
	for row in 0..h {
		for c in 0..w {
			let (x, y) = (c as f32 * sp, row as f32 * sp);
			let z = if y < 4.0 { 12.0 } else if y < 5.0 { 12.0 - (y - 4.0) * 10.0 } else { 2.0 - (y - 5.0) * 0.05 };
			pos.push([off[0] + x + noise * r.s(), off[1] + y.min(5.0) + (y - 5.0).max(0.0) + noise * r.s(), off[2] + z + noise * r.s()]);
		}
	}
	let mut tris = Vec::new();
	for row in 0..h - 1 {
		for c in 0..w - 1 {
			let (a, b, d, e) = (row * w + c, row * w + c + 1, (row + 1) * w + c, (row + 1) * w + c + 1);
			tris.push([a, b, d]);
			tris.push([b, e, d]);
		}
	}
	(pos, tris)
}

/// How a run of frames is made from a sheet: per-frame jitter of every vertex (`jit`), per-corner jitter on top for the share
/// `per_corner` of the corners, the shares of the triangles dropped, inserted again at a random place, and offered as refill
/// candidates, new candidates at unseen places, corners made NaN / infinite, and the chance that the slot list is rotated.
struct Scn {
	jit: f32,
	per_corner: f32,
	drop: f32,
	ins: f32,
	refill: f32,
	newcand: f32,
	nan: f32,
	inf: f32,
	shift: f32,
}

impl Scn {
	fn new(jit: f32, per_corner: f32, drop: f32, ins: f32, refill: f32, newcand: f32, shift: f32) -> Scn {
		Scn { jit, per_corner, drop, ins, refill, newcand, nan: 0.0, inf: 0.0, shift }
	}
}

fn frames(seed: u64, base: &[[f32; 3]], tris: &[[usize; 3]], sc: &Scn, nf: usize) -> Vec<Vec<Q>> {
	let mut r = Rng(seed | 1);
	let mut out = Vec::new();
	for _ in 0..nf {
		let vj: Vec<[f32; 3]> = base.iter().map(|p| [p[0] + sc.jit * r.s(), p[1] + sc.jit * r.s(), p[2] + sc.jit * r.s()]).collect();
		let mut order: Vec<usize> = (0..tris.len()).filter(|_| r.f() >= sc.drop).collect();
		let ni = (order.len() as f32 * sc.ins) as usize;
		for _ in 0..ni {
			let at = r.n(order.len() + 1);
			order.insert(at, r.n(tris.len()));
		}
		if r.f() < sc.shift && order.len() > 10 {
			let k = r.n(order.len());
			order.rotate_left(k);
		}
		let mut q = Vec::new();
		for &t in &order {
			let mut p = [[0.0; 3]; 4];
			for k in 0..3 {
				let mut v = vj[tris[t][k]];
				if r.f() < sc.per_corner {
					for a in &mut v {
						*a += sc.jit * r.s();
					}
				}
				if r.f() < sc.nan {
					v[r.n(3)] = f32::NAN;
				}
				if r.f() < sc.inf {
					v[r.n(3)] = if r.f() < 0.5 { f32::INFINITY } else { f32::NEG_INFINITY };
				}
				p[k] = v;
			}
			p[3] = p[2];
			q.push((p, r.f() < sc.refill));
		}
		// Refill candidates at places no vertex is at.
		for _ in 0..(q.len() as f32 * sc.newcand) as usize {
			let c = base[r.n(base.len())];
			let mut p = [[0.0; 3]; 4];
			for k in 0..3 {
				p[k] = [c[0] + 0.6 * r.s(), c[1] + 0.6 * r.s(), c[2] + 0.6 * r.s()];
			}
			p[3] = p[2];
			q.push((p, true));
		}
		out.push(q);
	}
	out
}

/// What a comparison saw, to assert that the interesting cases did happen.
#[derive(Default)]
struct Stats {
	/// Refill vertices the old path added.
	refilled: usize,
	/// Frames after which the old path's vertex count was lower than before (the 20,000 reset, a level change).
	shrinks: usize,
	max_verts: usize,
}

/// Runs the old and the fast path over `fr` and asserts that they agree after every frame. `level_changes`: frames that start with
/// an empty `SheetMem` (a level change in `sheet_falls`).
fn compare(name: &str, fr: &[Vec<Q>], level_changes: &[usize]) -> Stats {
	let (mut old, mut new) = (Run::default(), Run::default());
	let mut st = Stats::default();
	let mut prev = 0;
	for (n, q) in fr.iter().enumerate() {
		if level_changes.contains(&n) {
			old.mem = SheetMem::default();
			new.mem = SheetMem::default();
		}
		step(&mut old, q, false);
		step(&mut new, q, true);
		same(&old, &new, &format!("{name} frame {n}"));
		if on() && n % 5 == 4 {
			check_lim(&new.mem, 48);
		}
		st.refilled += old.verts.len() - q.iter().filter(|q| !q.1).count() * 4;
		let len = old.mem.verts.len();
		st.shrinks += usize::from(len < prev);
		prev = len;
		st.max_verts = st.max_verts.max(len);
	}
	st
}

#[test]
fn stress_sheets() {
	let (mut runs, mut refilled, mut shrinks) = (0, 0, 0);
	for (si, sp) in [0.37f32, 0.25, 0.19, 0.181].into_iter().enumerate() {
		let scenarios = [
			Scn::new(0.0, 0.0, 0.05, 0.02, 0.05, 0.01, 0.3),
			Scn::new(0.03, 0.2, 0.1, 0.05, 0.1, 0.02, 0.3),
			Scn::new(0.09, 0.5, 0.2, 0.1, 0.1, 0.05, 0.5),
			Scn::new(0.15, 1.0, 0.3, 0.2, 0.2, 0.05, 0.5),
			Scn::new(0.3, 1.0, 0.3, 0.2, 0.2, 0.05, 0.5),
		];
		for (ji, sc) in scenarios.iter().enumerate() {
			for seed in 0..2u64 {
				let mut r = Rng(0x9e37_79b9_7f4a_7c15 ^ (seed * 7919 + si as u64 * 104_729 + ji as u64 * 31));
				let off = [r.s() * 100.0, r.s() * 100.0, r.s() * 10.0];
				let (base, tris) = sheet(&mut r, (32, 22), sp, off, sp * 0.1);
				let fr = frames(seed * 13 + 5, &base, &tris, sc, 16);
				let level_changes: &[usize] = if seed == 1 { &[4, 9] } else { &[] };
				let st = compare(&format!("sp {sp} jit {} seed {seed}", sc.jit), &fr, level_changes);
				runs += 1;
				refilled += st.refilled;
				shrinks += st.shrinks;
			}
		}
	}
	assert_eq!(runs, 40);
	assert!(refilled > 1000, "refill vertices: {refilled}");
	assert!(shrinks >= 20, "level changes / resets seen: {shrinks}");
}

/// Unsettled positions: the 20,000-vertex reset of `observe`, then a settled frame repeated (the slot skip after a reset).
#[test]
fn stress_runaway_reset() {
	let mut r = Rng(12345);
	let mut fr = Vec::new();
	for _ in 0..10 {
		let mut q = Vec::new();
		for _ in 0..2500 {
			let c = [r.s() * 50.0, r.s() * 50.0, r.s() * 20.0];
			let mut p = [[0.0; 3]; 4];
			for k in 0..3 {
				p[k] = [c[0] + r.s(), c[1] + r.s(), c[2] + r.s()];
			}
			p[3] = p[2];
			q.push((p, r.f() < 0.1));
		}
		fr.push(q);
	}
	let last = fr.last().unwrap().clone();
	fr.push(last.clone());
	fr.push(last);
	let st = compare("runaway", &fr, &[]);
	assert!(st.shrinks >= 1, "the 20,000-vertex reset happened {} times", st.shrinks);
	assert!(st.max_verts > 20_000, "{} vertices at most", st.max_verts);
}

/// NaN / infinite corners and huge coordinates (see the header for what a debug build skips).
#[test]
fn stress_nonfinite() {
	let release = !cfg!(debug_assertions);
	let mut r = Rng(777);
	let (base, tris) = sheet(&mut r, (22, 16), 0.37, [10.0, -20.0, 0.0], 0.03);
	let mut sc = Scn::new(0.05, 0.3, 0.1, 0.05, 0.1, 0.02, 0.3);
	sc.nan = 0.01;
	sc.inf = if release { 0.01 } else { 0.0 };
	compare("nonfinite", &frames(99, &base, &tris, &sc, 14), &[]);
	let mut far = vec![1e6f32, -1e6, 1e8, -1e8, 3e9];
	if release {
		far.push(-3e9);
	}
	for off in far {
		let (base, tris) = sheet(&mut r, (10, 10), 0.37, [off, off, 0.0], 0.0);
		let sc = Scn::new(0.05, 0.3, 0.1, 0.05, 0.1, 0.02, 0.3);
		compare(&format!("huge {off}"), &frames(5, &base, &tris, &sc, 5), &[]);
	}
}

/// The cell cache must exist for `id_cached` (`observe_fast` makes it on its first call).
fn cloud(r: &mut Rng, n: usize, size: [f32; 3]) -> SheetMem {
	let mut m = SheetMem::default();
	m.qc = empty_cache();
	for _ in 0..n {
		m.id([r.f() * size[0], r.f() * size[1], r.f() * size[2]]);
	}
	m
}

/// The claim itself: `certified(v, p)` => v is `search(p)`, `lim` is below the true nearest-neighbour bound, and the fast lookups
/// give what `search` gives.
#[test]
fn certified_implies_search() {
	if !on() {
		eprintln!("skipped: RAC_FALL_FAST=0 keeps no vertex limits");
		return;
	}
	let mut r = Rng(4242);
	// A dense cloud: many vertices at nearly SHEET_SNAP from each other.
	let mut m = cloud(&mut r, 30_000, [5.0, 5.0, 2.0]);
	let n = m.verts.len();
	assert!(n > 2000, "{n}");
	check_lim(&m, n);
	for i in 0..n {
		let nn = (0..n).filter(|&j| j != i).map(|j| dist(m.verts[i], m.verts[j])).fold(f32::MAX, f32::min);
		assert!(nn >= SHEET_SNAP, "vertices closer than SHEET_SNAP: {nn}");
	}
	let mut certs = 0;
	for k in 0..100_000 {
		let i = r.n(n);
		let v = m.verts[i];
		// Probes at 0.9 .. 1.1 times the certified radius, in every direction.
		let rad = m.lim[i].sqrt() * (0.9 + 0.2 * r.f());
		let d = [r.s(), r.s(), r.s()];
		let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
		let p = [v[0] + d[0] / l * rad, v[1] + d[1] / l * rad, v[2] + d[2] / l * rad];
		let want = m.search(p);
		assert_eq!(m.search_fast(p), want, "search_fast {k}");
		if m.certified(i as u32, p) {
			certs += 1;
			assert_eq!(want, Some(i as u32), "certified {k}: p {p:?} v {v:?}");
		}
		if let Some(w) = want {
			let before = m.verts.len();
			assert_eq!(m.id_cached(p), w, "id_cached {k}");
			assert_eq!(m.verts.len(), before, "id_cached added a vertex");
		}
	}
	assert!(certs > 25_000, "{certs} certified probes");
}

/// `search_fast` against `search` right at the SHEET_SNAP boundary (the squared prefilter must never reject a candidate the
/// `hypot` test accepts).
#[test]
fn search_fast_at_snap_edge() {
	let mut r = Rng(31337);
	let mut m = SheetMem::default();
	for _ in 0..3000 {
		m.id([r.f() * 8.0 - 4.0, r.f() * 8.0 - 4.0, r.f() * 2.0]);
	}
	let n = m.verts.len();
	let mut found = 0;
	for k in 0..60_000 {
		let v = m.verts[r.n(n)];
		let d = [r.s(), r.s(), r.s()];
		let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
		let rad = SHEET_SNAP * (1.0 + (r.f() - 0.5) * 4e-6);
		let p = [v[0] + d[0] / l * rad, v[1] + d[1] / l * rad, v[2] + d[2] / l * rad];
		let want = m.search(p);
		assert_eq!(m.search_fast(p), want, "{k}");
		found += usize::from(want.is_some());
	}
	assert!(found > 10_000, "{found} probes found a vertex");
}
