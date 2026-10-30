//! [PC] TEST ONLY: the pairing and welding code of frame smoothing as it was before the per-field lookup tables (`index.rs`,
//! `weld::Verts`): hash maps keyed by (kind, cell) tuples built from scratch for each of `still_kinds`, `still_pairs` and
//! `weld`. Kept as the oracle: `tests` below compares it with the new code on synthetic fields (every output identical, in the
//! same order) and times both (`#[ignore]`, run it with `cargo test --release --lib interp::oracle -- --ignored --nocapture`).
//! The cell sums `cx + dx` use `wrapping_add`: the old code overflowed there on an infinite quad (cell = i32::MAX), which wraps in a
//! release build; wrapping keeps this oracle the same in a build with overflow checks.
use super::still::StillPair;
use super::weld::{SNAP, Weld, WeldStat};
use super::{FxMap, STILL_EPS, V3, kind_of, pair_ordered};
use crate::particles::WorldQuad;
use crate::viewer::smooth::near;
use std::collections::HashSet;

/// [PC] The quad kinds (texture, blend) that stand still: at least half their quads this field lie where one of last field's did
/// (`STILL_EPS`). Pokitaru's pool surface and waterfall sheets are such meshes; which of their triangles retail draws changes as
/// the camera turns (its VU1 clip), so the k-th of a kind is often another triangle than last field's k-th, and blending the two
/// pulled neighbours together: holes in the water for a frame (user clip 2026-10-19). Standing quads are drawn as they are.
/// The same triangle may come with its corners in another order (where its strip starts changes with the clip), so corners
/// match in any order, and the grid cell is the quad's centre.
/// A quad hits when a quad of its kind in last field's grid cell or one of the 26 around it is the `same`; a kind of fewer than 4
/// quads never counts, and a kind is decided (and its other quads skip the search) once half its quads hit or no longer can.
pub(super) fn still_kinds(prev: &[WorldQuad], cur: &[WorldQuad]) -> std::collections::HashSet<(usize, bool)> {
	let mid = |q: &WorldQuad| std::array::from_fn::<f32, 3, _>(|c| (q.pos[0][c] + q.pos[1][c] + q.pos[2][c] + q.pos[3][c]) * 0.25);
	let cell = |p: [f32; 3]| ((p[0] / 0.5).floor() as i32, (p[1] / 0.5).floor() as i32, (p[2] / 0.5).floor() as i32);
	// Per kind of this field: (hits, quads searched, quads).
	let mut count: FxMap<(usize, bool), (u32, u32, u32)> = FxMap::default();
	for q in cur {
		count.entry(kind_of(q)).or_default().2 += 1;
	}
	// Last field's quads of the kinds that can count, chained per (kind, cell): `head` holds a cell's last quad, `next` the one before.
	const END: u32 = u32::MAX;
	let mut head: FxMap<((usize, bool), (i32, i32, i32)), u32> = FxMap::default();
	let mut next = vec![END; prev.len()];
	for (j, q) in prev.iter().enumerate() {
		let kind = kind_of(q);
		if count.get(&kind).is_some_and(|c| c.2 >= 4) {
			let e = head.entry((kind, cell(mid(q)))).or_insert(END);
			next[j] = *e;
			*e = j as u32;
		}
	}
	let close = |a: V3, b: V3| (0..3).all(|c| (a[c] - b[c]).abs() < STILL_EPS);
	let within = |a: &WorldQuad, b: &WorldQuad| a.pos.iter().all(|&p| b.pos.iter().any(|&q| close(p, q)));
	let same = |a: &WorldQuad, b: &WorldQuad| within(a, b) && within(b, a);
	// The quad's own cell first (where a standing quad nearly always is), then the 26 around it.
	const AROUND: [(i32, i32, i32); 27] = {
		let mut o = [(0, 0, 0); 27];
		let mut n = 1;
		let mut i = 0;
		while i < 27 {
			let d = ((i / 9) as i32 - 1, (i / 3 % 3) as i32 - 1, (i % 3) as i32 - 1);
			if d.0 != 0 || d.1 != 0 || d.2 != 0 {
				o[n] = d;
				n += 1;
			}
			i += 1;
		}
		o
	};
	for q in cur {
		let kind = kind_of(q);
		let Some(e) = count.get_mut(&kind) else { continue };
		let (hit, searched, n) = *e;
		if n < 4 || 2 * hit >= n || 2 * (hit + n - searched) < n {
			continue;
		}
		let (cx, cy, cz) = cell(mid(q));
		let found = AROUND.iter().any(|&(dx, dy, dz)| {
			let mut j = head.get(&(kind, (cx.wrapping_add(dx), cy.wrapping_add(dy), cz.wrapping_add(dz)))).copied().unwrap_or(END);
			while j != END {
				if same(&prev[j as usize], q) {
					return true;
				}
				j = next[j as usize];
			}
			false
		});
		e.0 += found as u32;
		e.1 += 1;
	}
	count.into_iter().filter(|&(_, (hit, _, n))| n >= 4 && 2 * hit >= n).map(|(k, _)| k).collect()
}

/// The quads of `cur` that blend with one of `prev` as (index in `cur`, index in `prev`): the k-th quad of a kind pairs with the
/// k-th of that kind (`pair_ordered`), not of a standing kind (`still_kinds`, left as they are), every corner `near`. Once per
/// field; each display frame only lerps them (`lerp_quads`).
pub(super) fn quad_pairs(prev: &[WorldQuad], cur: &[WorldQuad], still: &std::collections::HashSet<(usize, bool)>) -> Vec<(u32, u32)> {
	let key = |q: &WorldQuad| (kind_of(q), q.pos[0]);
	let pairs = pair_ordered(&prev.iter().map(key).collect::<Vec<_>>(), &cur.iter().map(key).collect::<Vec<_>>());
	cur.iter()
		.zip(pairs)
		.enumerate()
		.filter_map(|(i, (q, j))| {
			let j = j?;
			(!still.contains(&kind_of(q)) && (0..4).all(|k| near(prev[j].pos[k], q.pos[k]))).then_some((i as u32, j as u32))
		})
		.collect()
}

/// The moving quads' pairs (`quad_pairs`), the standing ones' (`still::still_pairs`) and the standing kinds of one queue.
pub(super) fn all_pairs(prev: &[WorldQuad], cur: &[WorldQuad]) -> (Vec<(u32, u32)>, Vec<StillPair>, std::collections::HashSet<(usize, bool)>) {
	let still = still_kinds(prev, cur);
	(quad_pairs(prev, cur, &still), still_pairs(prev, cur, &still), still)
}

/// The quads of `cur` of the standing kinds `still` with their partner in `prev` (`StillPair`). Once per field.
pub(super) fn still_pairs(prev: &[WorldQuad], cur: &[WorldQuad], still: &HashSet<(usize, bool)>) -> Vec<StillPair> {
	if still.is_empty() {
		return Vec::new();
	}
	let cell = |q: &WorldQuad| {
		let m: V3 = std::array::from_fn(|c| (q.pos[0][c] + q.pos[1][c] + q.pos[2][c] + q.pos[3][c]) * 0.25);
		((m[0] / 0.5).floor() as i32, (m[1] / 0.5).floor() as i32, (m[2] / 0.5).floor() as i32)
	};
	let mut grid: FxMap<((usize, bool), (i32, i32, i32)), Vec<u32>> = FxMap::default();
	for (j, q) in prev.iter().enumerate() {
		let k = kind_of(q);
		if still.contains(&k) {
			grid.entry((k, cell(q))).or_default().push(j as u32);
		}
	}
	let close = |a: V3, b: V3| (0..3).all(|c| (a[c] - b[c]).abs() < STILL_EPS);
	let corners = |a: &WorldQuad, b: &WorldQuad| -> Option<[u8; 4]> {
		let mut m = [0u8; 4];
		for (k, &p) in a.pos.iter().enumerate() {
			let d = |q: V3| (0..3).map(|c| (p[c] - q[c]).abs()).sum::<f32>();
			let best = (0..4).min_by(|&x, &y| d(b.pos[x]).total_cmp(&d(b.pos[y])))?;
			if !close(p, b.pos[best]) {
				return None;
			}
			m[k] = best as u8;
		}
		b.pos.iter().all(|&q| a.pos.iter().any(|&p| close(p, q))).then_some(m)
	};
	let mut out = Vec::new();
	for (i, q) in cur.iter().enumerate() {
		let k = kind_of(q);
		if !still.contains(&k) {
			continue;
		}
		let (cx, cy, cz) = cell(q);
		// The quad's own cell first, then the 26 around it.
		let found = (0..27).map(|n| if n == 0 { (0, 0, 0) } else { let n = if n <= 13 { n - 1 } else { n }; (n / 9 - 1, n / 3 % 3 - 1, n % 3 - 1) }).find_map(|(dx, dy, dz)| {
			grid.get(&(k, (cx.wrapping_add(dx), cy.wrapping_add(dy), cz.wrapping_add(dz))))?.iter().find_map(|&j| corners(q, &prev[j as usize]).map(|m| (j, m)))
		});
		if let Some((j, m)) = found {
			out.push((i as u32, j, m));
		}
	}
	out
}
/// One welded vertex: (paired corners seen, first displacement, sum of displacements, smallest displacement, all equal so far,
/// corners on it, largest disagreement).
#[derive(Clone, Copy)]
struct Vert {
	n: u32,
	first: V3,
	sum: V3,
	small: V3,
	same: bool,
	corners: u32,
	spread: f32,
}

/// The welded displacements of this field's queues `cur` against last field's `prev`, from the moving pairs `moving` (index in cur,
/// index in prev; corner k with corner k) and the standing pairs `still` (corner k with corner m[k]). Once per field.
pub(super) fn weld(prev: [&[WorldQuad]; 2], cur: [&[WorldQuad]; 2], moving: &[Vec<(u32, u32)>; 2], still: &[Vec<StillPair>; 2]) -> (Weld, WeldStat) {
	let mut stat = WeldStat::default();
	if moving.iter().all(Vec::is_empty) && still.iter().all(Vec::is_empty) {
		return (Default::default(), stat);
	}
	let key = |p: V3| (p[0].to_bits(), p[1].to_bits(), p[2].to_bits());
	let mut verts: FxMap<(u32, u32, u32), u32> = FxMap::default();
	let mut vs: Vec<Vert> = Vec::new();
	// Vertex of every corner of every quad, per queue.
	let mut at: [Vec<[u32; 4]>; 2] = Default::default();
	for qi in 0..2 {
		at[qi] = cur[qi]
			.iter()
			.map(|q| {
				std::array::from_fn(|k| {
					let n = vs.len() as u32;
					let v = *verts.entry(key(q.pos[k])).or_insert(n);
					if v == n {
						vs.push(Vert { n: 0, first: [0.0; 3], sum: [0.0; 3], small: [0.0; 3], same: true, corners: 0, spread: 0.0 });
					}
					vs[v as usize].corners += 1;
					v
				})
			})
			.collect();
	}
	let mut paired: [Vec<bool>; 2] = [vec![false; cur[0].len()], vec![false; cur[1].len()]];
	let mut add = |vs: &mut Vec<Vert>, v: u32, d: V3| {
		let e = &mut vs[v as usize];
		let len = |v: V3| v[0] * v[0] + v[1] * v[1] + v[2] * v[2];
		if e.n == 0 || len(d) < len(e.small) {
			e.small = d;
		}
		if e.n == 0 {
			e.first = d;
		} else if d != e.first {
			e.same = false;
			e.spread = e.spread.max((0..3).map(|c| (d[c] - e.first[c]).abs()).fold(0.0, f32::max));
		}
		e.n += 1;
		for c in 0..3 {
			e.sum[c] += d[c];
		}
	};
	for qi in 0..2 {
		for &(i, j) in &moving[qi] {
			let (Some(q), Some(a)) = (cur[qi].get(i as usize), prev[qi].get(j as usize)) else { continue };
			paired[qi][i as usize] = true;
			for k in 0..4 {
				add(&mut vs, at[qi][i as usize][k], std::array::from_fn(|c| a.pos[k][c] - q.pos[k][c]));
			}
		}
		for &(i, j, m) in &still[qi] {
			let (Some(q), Some(a)) = (cur[qi].get(i as usize), prev[qi].get(j as usize)) else { continue };
			paired[qi][i as usize] = true;
			for k in 0..4 {
				add(&mut vs, at[qi][i as usize][k], std::array::from_fn(|c| a.pos[m[k] as usize][c] - q.pos[k][c]));
			}
		}
	}
	// One displacement per vertex.
	let disp: Vec<Option<V3>> = vs
		.iter()
		.map(|e| {
			if e.corners > 1 {
				stat.shared += 1;
			}
			if !e.same {
				stat.split += 1;
				stat.spread = stat.spread.max(e.spread);
			}
			match e.n {
				0 => None,
				_ if e.same => Some(e.first),
				_ if e.spread > SNAP => Some(e.small),
				n => Some(e.sum.map(|s| s / n as f32)),
			}
		})
		.collect();
	let mut out: Weld = Default::default();
	for qi in 0..2 {
		for (i, corners) in at[qi].iter().enumerate() {
			if !corners.iter().any(|&v| disp[v as usize].is_some()) {
				continue;
			}
			if !paired[qi][i] {
				stat.carried += corners.iter().filter(|&&v| disp[v as usize].is_some()).count() as u32;
			}
			out[qi].push((i as u32, corners.map(|v| disp[v as usize].unwrap_or([0.0; 3]))));
		}
	}
	(out, stat)
}

#[cfg(test)]
mod tests {
	use super::super::{Field, State, advance, pair_field, reset_off, still, weld};
	use super::*;
	use crate::retail::RetailFx;

	struct Rng(u32);
	impl Rng {
		fn f(&mut self) -> f32 {
			self.0 ^= self.0 << 13;
			self.0 ^= self.0 >> 17;
			self.0 ^= self.0 << 5;
			(self.0 % 100_000) as f32 / 100_000.0
		}
		fn u(&mut self, n: usize) -> usize {
			(self.f() * n as f32) as usize % n.max(1)
		}
	}

	fn quad(o: [f32; 3], tex: usize, tri: bool) -> WorldQuad {
		let p3 = if tri { [o[0], o[1] + 0.4, o[2]] } else { [o[0] + 0.4, o[1] + 0.4, o[2]] };
		WorldQuad { pos: [o, [o[0] + 0.4, o[1], o[2]], [o[0], o[1] + 0.4, o[2]], p3], uv: [[0.1, 0.2]; 4], colour: [[0.5; 4]; 4], tex, additive: tex % 2 == 1 }
	}

	/// A scene: three standing meshes (grid of triangles), eight moving kinds, one tiny kind, with fog bits in `tex`.
	fn scene(r: &mut Rng, nx: usize, ny: usize, moving: usize) -> Vec<WorldQuad> {
		use crate::retail::efx_draw::FOG_ON;
		let mut v = Vec::new();
		for m in 0..3 {
			for i in 0..nx {
				for j in 0..ny {
					v.push(quad([i as f32 * 0.45, j as f32 * 0.45, m as f32 * 3.0], (100 + m) | FOG_ON | (r.u(200) << 24), true));
				}
			}
		}
		for k in 0..8 {
			for _ in 0..moving {
				v.push(quad([r.f() * 40.0, r.f() * 40.0, r.f() * 5.0], 200 + k, false));
			}
		}
		for _ in 0..3 {
			v.push(quad([r.f() * 40.0, 0.0, 0.0], 300, false));
		}
		v
	}

	/// The next field of a list: standing kinds jitter, lose and rotate corners, moving kinds move (some teleport), quads spawn,
	/// duplicate, die and swap places.
	fn evolve(r: &mut Rng, prev: &[WorldQuad]) -> Vec<WorldQuad> {
		use crate::retail::efx_draw::FOG_ON;
		let mut v: Vec<WorldQuad> = Vec::new();
		for q in prev {
			let base = crate::retail::efx_draw::plain_tex(q.tex);
			let mut c = *q;
			if (100..103).contains(&base) {
				if r.f() < 0.08 {
					continue;
				}
				for p in &mut c.pos {
					for a in p.iter_mut() {
						*a += (r.f() - 0.5) * 0.16;
					}
				}
				if r.f() < 0.3 {
					c.pos.rotate_left(1 + r.u(3));
				}
				c.tex = base | FOG_ON | (r.u(200) << 24);
			} else {
				if r.f() < 0.05 {
					continue;
				}
				let d = if r.f() < 0.05 { 12.0 } else { r.f() * 1.5 };
				for p in &mut c.pos {
					p[0] += d;
					p[1] -= d * 0.5;
				}
			}
			v.push(c);
			if r.f() < 0.03 {
				v.push(c);
			}
			if r.f() < 0.05 {
				v.push(quad([r.f() * 40.0, r.f() * 40.0, 0.0], 200 + r.u(8), false));
			}
		}
		for _ in 0..v.len() / 20 {
			let (a, b) = (r.u(v.len()), r.u(v.len()));
			v.swap(a, b);
		}
		v
	}

	/// A chain of fields (`cuts`: indexes that are cuts) through the new `pair_field` (table reuse, parallel when `par_min` allows)
	/// against the old functions on the same lists: every output equal.
	fn run_chain(fields: Vec<(Vec<WorldQuad>, Vec<WorldQuad>)>, cuts: &[usize], par_min: usize) -> [usize; 4] {
		let mut seen = [0usize; 4];
		let mut s = State::default();
		let mut prev: Option<Field> = None;
		for (n, (qa, qb)) in fields.into_iter().enumerate() {
			let f = Field { fx: Some(RetailFx { qa, qb, ..Default::default() }), serial: n as u64 + 1, ..Default::default() };
			let cut = cuts.contains(&n) || prev.is_none();
			pair_field(&mut s, prev.as_ref(), &f, cut, n as u32, par_min);
			if cut {
				assert!(s.quads.iter().all(Vec::is_empty) && s.still.iter().all(Vec::is_empty) && s.weld.iter().all(Vec::is_empty), "field {n}");
			} else {
				seen[0] += s.quads.iter().map(Vec::len).sum::<usize>();
				seen[1] += s.still.iter().map(Vec::len).sum::<usize>();
				seen[2] += s.standing.iter().map(HashSet::len).sum::<usize>();
				seen[3] += s.weld.iter().map(Vec::len).sum::<usize>();
				let (a, b) = (prev.as_ref().unwrap().fx.as_ref().unwrap(), f.fx.as_ref().unwrap());
				let (oa, ob) = (all_pairs(&a.qa, &b.qa), all_pairs(&a.qb, &b.qb));
				assert_eq!(s.quads, [oa.0.clone(), ob.0.clone()], "moving pairs, field {n}");
				assert_eq!(s.still, [oa.1.clone(), ob.1.clone()], "standing pairs, field {n}");
				assert_eq!(s.standing, [oa.2, ob.2], "standing kinds, field {n}");
				let (ow, ost) = super::weld([&a.qa, &a.qb], [&b.qa, &b.qb], &[oa.0, ob.0], &[oa.1, ob.1]);
				assert_eq!(format!("{:?}", s.weld), format!("{ow:?}"), "weld, field {n}");
				let (nw, nst) = weld::weld([&a.qa, &a.qb], [&b.qa, &b.qb], &s.quads, &s.still);
				assert_eq!((format!("{nw:?}"), nst), (format!("{ow:?}"), ost), "weld stat, field {n}");
			}
			prev = Some(f);
		}
		seen
	}

	fn chain(r: &mut Rng, nx: usize, ny: usize, moving: usize, len: usize, both: bool) -> Vec<(Vec<WorldQuad>, Vec<WorldQuad>)> {
		let mut a = scene(r, nx, ny, moving);
		let mut b = if both { scene(r, nx, ny, moving) } else { Vec::new() };
		let mut out = vec![(a.clone(), b.clone())];
		for _ in 1..len {
			a = evolve(r, &a);
			b = evolve(r, &b);
			out.push((a.clone(), b.clone()));
		}
		out
	}

	#[test]
	fn small_fields_match_the_old_code() {
		for seed in 1..60u32 {
			let mut r = Rng(seed.wrapping_mul(2_654_435_761) | 1);
			let (nx, ny, mv) = (1 + r.u(6), 1 + r.u(6), r.u(12));
			run_chain(chain(&mut r, nx, ny, mv, 5, seed % 3 != 0), &[3], if seed % 2 == 0 { 0 } else { usize::MAX });
		}
	}

	#[test]
	fn thousands_of_quads_match_the_old_code() {
		for seed in [7u32, 91] {
			let mut r = Rng(seed);
			let seen = run_chain(chain(&mut r, 24, 18, 200, 4, true), &[2], 0);
			// The test must reach the standing path: moving pairs, standing pairs, standing kinds and welded quads all occur.
			assert!(seen.iter().all(|&n| n > 0), "{seen:?}");
			run_chain(chain(&mut r, 24, 18, 200, 3, true), &[], usize::MAX);
		}
	}

	#[test]
	fn empty_lists_and_one_empty_queue() {
		let mut r = Rng(5);
		run_chain(vec![(Vec::new(), Vec::new()); 3], &[], 0);
		let c = chain(&mut r, 8, 8, 30, 4, false);
		run_chain(c.clone(), &[], 0);
		// Queue A empty, queue B full; and a list that empties and fills again.
		run_chain(c.iter().map(|(a, b)| (b.clone(), a.clone())).collect(), &[], 0);
		let mut c2 = chain(&mut r, 8, 8, 30, 5, true);
		c2[2] = (Vec::new(), Vec::new());
		run_chain(c2, &[], 0);
	}

	#[test]
	fn duplicates_identical_fields_and_odd_quads() {
		// 300 copies of one quad in one cell (chain order), then the same field again.
		let q = quad([1.0, 1.0, 1.0], 100, true);
		let dup = vec![q; 300];
		run_chain(vec![(dup.clone(), dup.clone()); 3], &[], 0);
		// Identical fields; degenerate quads (all corners equal); a quad with NaN corners; an infinite one.
		let mut r = Rng(11);
		let mut a = scene(&mut r, 6, 6, 20);
		let mut d = quad([2.0, 2.0, 2.0], 205, false);
		d.pos = [d.pos[0]; 4];
		a.push(d);
		let mut nan = quad([3.0, 3.0, 3.0], 206, false);
		nan.pos[1][0] = f32::NAN;
		a.push(nan);
		let mut inf = quad([3.0, 3.0, 3.0], 100, true);
		inf.pos[2][1] = f32::INFINITY;
		a.push(inf);
		run_chain(vec![(a.clone(), a.clone()), (a.clone(), a.clone()), (a.clone(), a)], &[], usize::MAX);
	}

	/// One step of a flow through `advance` (what `field()` does for each retail field) or `reset_off`.
	enum Step {
		/// Smoothing switched Off for this field.
		Off,
		/// A field with these lists (`None`: a field without effects), in `level` (another level than the field before = a cut).
		Field(Option<(Vec<WorldQuad>, Vec<WorldQuad>)>, u32),
	}

	/// `[holds, paired fields]` seen. The reference for each field: a cut when there is no field before or the level differs; else the
	/// standing mesh carry-over (`still::hold`, applied BEFORE the pairing, on the newest lists) and the old pairing / weld code
	/// against the field before as `hold` left the newest. Also checks the lists the state keeps and the tables' field serials.
	fn run_flow(steps: Vec<Step>) -> [usize; 2] {
		type Lists = (Vec<WorldQuad>, Vec<WorldQuad>);
		let (mut s, mut seen) = (State::default(), [0usize; 2]);
		let mut r_prev: Option<(Option<Lists>, u32)> = None;
		let mut r_standing: [HashSet<(usize, bool)>; 2] = Default::default();
		let mut r_held: [Vec<(usize, bool)>; 2] = Default::default();
		let empty = |s: &State, n: usize| assert!(s.quads.iter().all(Vec::is_empty) && s.still.iter().all(Vec::is_empty) && s.weld.iter().all(Vec::is_empty) && s.standing.iter().all(HashSet::is_empty), "step {n}");
		for (n, step) in steps.into_iter().enumerate() {
			let (lists, level) = match step {
				Step::Off => {
					reset_off(&mut s);
					assert!(s.prev.is_none() && s.cur.is_none() && s.cut, "Off, step {n}");
					empty(&s, n);
					(r_prev, r_standing, r_held) = (None, Default::default(), Default::default());
					continue;
				}
				Step::Field(l, level) => (l, level),
			};
			let cam = ([0.0f32; 3], 0.0f32, 0.0f32, 1.0f32);
			let f = Field { fx: lists.as_ref().map(|(qa, qb)| RetailFx { qa: qa.clone(), qb: qb.clone(), ..Default::default() }), cam: Some(cam), key: (level, 0, false, false), ..Default::default() };
			advance(&mut s, f, n as u32);
			let cut = r_prev.as_ref().is_none_or(|(_, l)| *l != level);
			assert_eq!(s.cut, cut, "cut, step {n}");
			let mut cur = lists;
			let prev_fx = r_prev.as_ref().and_then(|(p, _)| p.as_ref());
			let mut held: [Vec<(usize, bool)>; 2] = Default::default();
			match (prev_fx, cur.as_mut()) {
				(Some(a), Some(b)) if !cut => {
					held = [still::hold(&a.0, &mut b.0, &r_standing[0], &r_held[0]), still::hold(&a.1, &mut b.1, &r_standing[1], &r_held[1])];
					seen[0] += held.iter().map(Vec::len).sum::<usize>();
					seen[1] += 1;
					let (oa, ob) = (all_pairs(&a.0, &b.0), all_pairs(&a.1, &b.1));
					assert_eq!(s.quads, [oa.0.clone(), ob.0.clone()], "moving pairs, step {n}");
					assert_eq!(s.still, [oa.1.clone(), ob.1.clone()], "standing pairs, step {n}");
					assert_eq!(s.standing, [oa.2.clone(), ob.2.clone()], "standing kinds, step {n}");
					let (ow, _) = super::weld([&a.0, &a.1], [&b.0, &b.1], &[oa.0, ob.0], &[oa.1, ob.1]);
					assert_eq!(format!("{:?}", s.weld), format!("{ow:?}"), "weld, step {n}");
					r_standing = [oa.2, ob.2];
				}
				_ => {
					empty(&s, n);
					r_standing = Default::default();
				}
			}
			assert_eq!(s.held, held, "held kinds, step {n}");
			r_held = held;
			// The newest field as the state keeps it (hold applied) is what the reference paired.
			let kept = s.cur.as_ref().and_then(|c| c.fx.as_ref()).map(|x| (format!("{:?}", x.qa), format!("{:?}", x.qb)));
			assert_eq!(kept, cur.as_ref().map(|(a, b)| (format!("{a:?}"), format!("{b:?}"))), "kept lists, step {n}");
			// The tables built for the newest field carry its serial.
			let serial = s.cur.as_ref().unwrap().serial;
			assert!(s.idx.iter().all(|i| i.serial == serial), "table serial, step {n}");
			r_prev = Some((cur, level));
		}
		seen
	}

	/// The lists without the standing mesh `m` (base texture 100 + m): its quads are missing for one field.
	fn without_mesh(q: &[WorldQuad], m: usize) -> Vec<WorldQuad> {
		q.iter().filter(|q| crate::retail::efx_draw::plain_tex(q.tex) != 100 + m).cloned().collect()
	}

	/// A flow with a field without effects in the middle, a cut after it, and a standing mesh missing for one field (`still::hold`).
	#[test]
	fn flow_with_fxless_fields_cuts_and_holds_matches_the_old_code() {
		let mut r = Rng(77);
		let c = chain(&mut r, 8, 6, 20, 9, true);
		let f = |i: usize| Step::Field(Some(c[i].clone()), 1);
		let steps = vec![
			f(0),
			f(1),
			// Queue A loses a whole standing mesh for one field: carried over (hold), not again right after.
			Step::Field(Some((without_mesh(&c[2].0, 0), c[2].1.clone())), 1),
			f(3),
			// A field without effects: the next one has nothing to pair with, the table swap must not pair with stale tables.
			Step::Field(None, 1),
			f(5),
			f(6),
			// A cut in the middle (another level), then the chain goes on against the cut field.
			Step::Field(Some(c[7].clone()), 2),
			f(8),
		];
		let [holds, paired] = run_flow(steps);
		assert!(holds > 0, "the hold path was not reached");
		assert!(paired >= 4, "{paired}");
	}

	/// Smoothing switched Off in the middle resets everything: the first field after it is a cut, the next pairs again.
	#[test]
	fn off_resets_the_pairing() {
		let mut r = Rng(31);
		let c = chain(&mut r, 6, 6, 15, 7, true);
		let steps = vec![
			Step::Field(Some(c[0].clone()), 1),
			Step::Field(Some(c[1].clone()), 1),
			Step::Off,
			Step::Off,
			Step::Field(Some(c[4].clone()), 1),
			Step::Field(Some(c[5].clone()), 1),
			Step::Field(Some(c[6].clone()), 1),
		];
		let [_, paired] = run_flow(steps);
		assert_eq!(paired, 3);
	}

	/// The `prev` tables carry the serial of their field: tables of another field with the SAME list length are rebuilt, not trusted.
	#[test]
	fn stale_prev_tables_of_equal_length_are_rebuilt() {
		let mut r = Rng(5);
		let x = scene(&mut r, 8, 8, 25);
		// `y` has the length of `x` but lies 50 m away: x's tables (serial 1) do not fit it.
		let mut y = x.clone();
		for q in &mut y {
			for p in &mut q.pos {
				p[0] += 50.0;
			}
		}
		let cur = evolve(&mut r, &y);
		let field = |qa: Vec<WorldQuad>, serial: u64| Field { fx: Some(RetailFx { qa, qb: Vec::new(), ..Default::default() }), serial, ..Default::default() };
		let mut s = State::default();
		pair_field(&mut s, None, &field(x, 1), true, 0, 0);
		let (other, f1) = (field(y.clone(), 77), field(cur.clone(), 78));
		assert_eq!(s.idx[0].len(), y.len());
		pair_field(&mut s, Some(&other), &f1, false, 1, 0);
		let o = all_pairs(&y, &cur);
		assert!(!o.0.is_empty() && !o.1.is_empty(), "the test pairs nothing");
		assert_eq!(s.quads[0], o.0);
		assert_eq!(s.still[0], o.1);
		assert!(s.pidx[0].serial == 77);
	}

	/// Times the old and the new pairing + weld on fields of about 4000 + 4000 quads:
	/// `cargo test --release --lib interp::oracle -- --ignored --nocapture`.
	#[test]
	#[ignore]
	fn timing_old_vs_new() {
		let mut r = Rng(1234);
		let fields = chain(&mut r, 30, 22, 260, 12, true);
		println!("quads per queue: {}", fields[0].0.len());
		let t = std::time::Instant::now();
		for w in fields.windows(2) {
			let (a, b) = (&w[0], &w[1]);
			let (oa, ob) = (all_pairs(&a.0, &b.0), all_pairs(&a.1, &b.1));
			let _ = super::weld([&a.0, &a.1], [&b.0, &b.1], &[oa.0, ob.0], &[oa.1, ob.1]);
		}
		let old = t.elapsed().as_secs_f64() * 1e3 / 11.0;
		// Where the single-threaded new code spends it (queue A + B together): index build, pairing, vertex table, weld.
		{
			use super::super::{index, weld};
			let (mut ia, mut ib) = (index::QIndex::default(), index::QIndex::default());
			let (mut pa, mut pb) = (index::QIndex::default(), index::QIndex::default());
			let mut wv = weld::Verts::default();
			let mut tt = [0.0f64; 4];
			let mut now = std::time::Instant::now();
			let mut lap = |k: usize, now: &mut std::time::Instant| {
				tt[k] += now.elapsed().as_secs_f64() * 1e3;
				*now = std::time::Instant::now();
			};
			for w in fields.windows(2) {
				let (a, b) = (&w[0], &w[1]);
				pa.build(&a.0);
				pb.build(&a.1);
				now = std::time::Instant::now();
				ia.build(&b.0);
				ib.build(&b.1);
				lap(0, &mut now);
				let (ra, rb) = (index::pairs(&a.0, &b.0, &pa, &ia), index::pairs(&a.1, &b.1, &pb, &ib));
				lap(1, &mut now);
				wv.index([&b.0, &b.1]);
				lap(2, &mut now);
				let _ = weld::weld_with(&mut wv, [&a.0, &a.1], [&b.0, &b.1], &[ra.moving, rb.moving], &[ra.still, rb.still]);
				lap(3, &mut now);
			}
			println!("new parts per field: build {:.3}, pairs {:.3}, vertex table {:.3}, weld {:.3} ms", tt[0] / 11.0, tt[1] / 11.0, tt[2] / 11.0, tt[3] / 11.0);
		}
		for (name, par_min) in [("new, one thread", usize::MAX), ("new, rayon join", 0)] {
			let mut s = State::default();
			let mut prev: Option<Field> = None;
			let mut total = 0.0;
			for (n, (qa, qb)) in fields.iter().enumerate() {
				let f = Field { fx: Some(RetailFx { qa: qa.clone(), qb: qb.clone(), ..Default::default() }), serial: n as u64 + 1, ..Default::default() };
				let t = std::time::Instant::now();
				pair_field(&mut s, prev.as_ref(), &f, prev.is_none(), n as u32, par_min);
				total += t.elapsed().as_secs_f64() * 1e3;
				prev = Some(f);
			}
			println!("{name}: {:.3} ms per field; old: {old:.3} ms per field", total / 11.0);
		}
	}
}
