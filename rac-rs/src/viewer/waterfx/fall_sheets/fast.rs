//! [PC] perf (lane nfalls 2026-10-30): the cheaper, exactly equal path of `SheetMem::observe` / `refill` (RAC_FALL_FAST=0 runs the
//! old one). Waterfall view on the laptop (Pokitaru's landing pad, frame smoothing on): ~2800 sheet quads = 11,200 corners a frame, all
//! with new (blended) bits, and `observe` was 55% of `sheet_falls`: per corner two `hypot` calls for the hint test, and on a miss
//! (a quad retail leaves out or adds shifts every later slot) a memo lookup, a grid search with two `hypot` per candidate and a
//! memo insert that never hits again. Here:
//! - `certified`: the hint test with a radius per vertex (`SheetMem::lim`) instead of the fixed SHEET_SNAP / 2: on a squared
//!   distance, no `hypot`.
//! - A hint miss asks a cache keyed by the corner's 0.09-wide position cell (`QEntry`) before the grid search, so a shifted slot
//!   list finds its vertices again without a search; `search_fast` rejects candidates on squared distance first.
//! - A quad whose three vertices are those of the same slot last frame is already in `known` (nothing to sort or insert).
//! - `refill_fast` does nothing without candidates, else builds its drawn set in a reused table.
//! Exactness. The old answer for a corner at p is "the vertex nearest to p within SHEET_SNAP, else a new one" (`SheetMem::id`).
//! A vertex v at distance r from p is that nearest one when r < SHEET_SNAP and r < d(v, w) - r for every other vertex w (then
//! d(p, w) >= d(v, w) - r > r: strictly farther). Vertices lie at least SHEET_SNAP apart (a vertex is added only where none is
//! within SHEET_SNAP), which gives `id_near`'s radius SHEET_SNAP / 2; here each vertex also keeps `lim`, from its real nearest
//! neighbour (`new_vertex_limits`: every neighbour within LIM_REACH, each new vertex updates the ones near it, farther ones are at
//! least LIM_REACH away): r < min(SHEET_SNAP, nn / 2), less 1% for rounding. A candidate vertex found in whatever way, accepted
//! by that test, is the old answer, so the slot hint and the cell cache need no invalidation. A rejected candidate falls back to the
//! full search, which gives the old answer by construction (`search_fast` visits the same cells in the same order and runs the old
//! `hypot` test on every candidate that could pass it).
//! Premises of that argument, besides the two proven above, each fixed by a `const _` assert below or by a test (`stress_tests.rs`):
//! - `search` only looks at the cells `SheetMem::reach` selects, so a certified vertex (r <= CAP = 0.99 SHEET_SNAP = 0.1782) must lie
//!   in one of them: `reach` widens SHEET_SNAP by REACH_EPS cells (SHEET_SNAP + REACH_EPS * CELL = 0.1805), 0.0023 more than CAP. Raise
//!   CAP to SHEET_SNAP or drop REACH_EPS and the argument (and the const assert) fails.
//! - The old path's `id_near` (lane lows) answers what `search` answers (test `id_near_is_id`); the fast path is compared with
//!   `search`, so with that fact it is compared with the old `observe`, and the tests compare the two end to end.
//! - A competitor w within SHEET_SNAP of p is within CAP + SHEET_SNAP = 0.3582 of v, under LIM_REACH = 0.5, so `new_vertex_limits`
//!   saw the pair.
//! RAC_FALL_FAST=0 isolates all of it: `on()` is read once per process, and with it off `push_vertex` keeps no `lim` (old code as in
//! dev). With it on, `lim` has one entry per vertex at all times (every vertex goes through `push_vertex`).

use super::*;

#[cfg(test)]
mod stress_tests;

/// RAC_FALL_FAST=0 runs the sheet code the old way (same-exe A/B and fault isolation: no vertex limits are kept either); on by
/// default. Read once, so it is fixed for the process: `lim` is either kept for every vertex or for none.
pub(super) fn on() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_FALL_FAST").map_or(true, |v| v != "0"))
}

/// Squared distance above which the old search test (`d < SHEET_SNAP`) certainly fails (SHEET_SNAP + 0.1%).
const SNAP_OUT2: f32 = SHEET_SNAP * 1.001 * SHEET_SNAP * 1.001;
/// Neighbours of a new vertex are looked for up to this far (the cells next to its own, `SheetMem::CELL` wide); farther ones are
/// at least this far.
const LIM_REACH: f32 = SheetMem::CELL;
/// The largest radius of `lim`: SHEET_SNAP less 1% for rounding.
const CAP: f32 = SHEET_SNAP * 0.99;
// CAP lies inside the cells `reach` selects (SHEET_SNAP widened by REACH_EPS cells).
const _: () = assert!(CAP < SHEET_SNAP + SheetMem::REACH_EPS * SheetMem::CELL);
// A competitor of a certified vertex (within CAP + SHEET_SNAP of it) is within LIM_REACH, and half of LIM_REACH is above SHEET_SNAP.
const _: () = assert!(CAP + SHEET_SNAP < LIM_REACH && LIM_REACH * 0.5 > SHEET_SNAP);

/// The `lim` of a new vertex at `p` (cell `c`), and, for each vertex within LIM_REACH of it, lowered if need be. (Called before the
/// vertex is stored: no self match.)
pub(super) fn new_vertex_limits(m: &mut SheetMem, p: [f32; 3], c: (i32, i32, i32)) -> f32 {
	// Nothing nearer than LIM_REACH: the radius is the SHEET_SNAP cap (LIM_REACH / 2 > SHEET_SNAP).
	let mut lim = CAP * CAP;
	for dx in -1..=1 {
		for dy in -1..=1 {
			for dz in -1..=1 {
				// Saturating: a cell index at the i32 end (a corner at infinity or beyond 2^30) must not overflow (debug builds); the
				// end cell visited twice then changes nothing (`min`).
				let cell = (c.0.saturating_add(dx), c.1.saturating_add(dy), c.2.saturating_add(dz));
				for &i in m.grid.get(&cell).into_iter().flatten() {
					let q = m.verts[i as usize];
					let d = (q[0] - p[0]).hypot(q[1] - p[1]).hypot(q[2] - p[2]);
					if d < LIM_REACH {
						// Half the distance, less 1% for rounding and 0.1% for the distance itself.
						let r = d * 0.5 * 0.99 * 0.999;
						lim = lim.min(r * r);
						m.lim[i as usize] = m.lim[i as usize].min(r * r);
					}
				}
			}
		}
	}
	lim
}

/// One entry of the position-cell cache: the vertex found last for a corner in this cell (verified by `near` at every use).
#[derive(Clone, Copy)]
pub(super) struct QEntry {
	key: [i32; 3],
	id: u32,
}

const QC_BITS: u32 = 13;
/// Width of a cache cell (SHEET_SNAP / 2).
const QC_INV: f32 = 2.0 / SHEET_SNAP;

fn empty_cache() -> Vec<QEntry> {
	vec![QEntry { key: [0; 3], id: u32::MAX }; 1 << QC_BITS]
}

fn qkey(p: [f32; 3]) -> [i32; 3] {
	[(p[0] * QC_INV).floor() as i32, (p[1] * QC_INV).floor() as i32, (p[2] * QC_INV).floor() as i32]
}

fn qslot(k: [i32; 3]) -> usize {
	let h = (k[0] as u32).wrapping_mul(0x9e37_79b1);
	let h = (h.rotate_left(5) ^ k[1] as u32).wrapping_mul(0x85eb_ca77);
	let h = (h.rotate_left(5) ^ k[2] as u32).wrapping_mul(0xc2b2_ae3d);
	(h >> (32 - QC_BITS)) as usize
}

impl SheetMem {
	/// `observe` with the same result: adds the new triangles, sets each vertex's `light`, and leaves each corner's vertex id in
	/// `last_ids` (what `observe` returns and keeps).
	pub(super) fn observe_fast(&mut self, verts: &mut [Vertex]) {
		if self.verts.len() > 20_000 {
			*self = SheetMem::default();
		}
		if self.qc.is_empty() {
			self.qc = empty_cache();
		}
		let prev = std::mem::take(&mut self.last_ids);
		let mut cur = std::mem::take(&mut self.ids_spare);
		cur.clear();
		cur.reserve(verts.len());
		for (k, v) in verts.iter().enumerate() {
			let hint = prev.get(k).copied().filter(|&h| self.certified(h, v.pos));
			cur.push(match hint {
				Some(h) => h,
				None => self.id_cached(v.pos),
			});
		}
		let mut dirty = false;
		for (n, q) in cur.chunks_exact(4).enumerate() {
			// The same three vertices as this slot's quad last frame: that frame inserted the triangle.
			if prev.get(n * 4..n * 4 + 4).is_some_and(|p| p[..3] == q[..3]) {
				continue;
			}
			dirty |= self.add_triangle([q[0], q[1], q[2]]);
		}
		if dirty {
			self.reshape(true);
		}
		for (v, &i) in verts.iter_mut().zip(&cur) {
			v.light = self.light[i as usize];
		}
		self.last_ids = cur;
		self.ids_spare = prev;
	}

	/// Vertex `i` is surely the one nearest to `p` within SHEET_SNAP (see the header; false for an id that is no vertex).
	#[inline(always)]
	fn certified(&self, i: u32, p: [f32; 3]) -> bool {
		let (Some(q), Some(&lim)) = (self.verts.get(i as usize), self.lim.get(i as usize)) else { return false };
		let (a, b, c) = (q[0] - p[0], q[1] - p[1], q[2] - p[2]);
		a * a + b * b + c * c < lim
	}

	/// The vertex within SHEET_SNAP of `p` (the nearest), a new one if none: `id`, through the cell cache.
	fn id_cached(&mut self, p: [f32; 3]) -> u32 {
		let key = qkey(p);
		let slot = qslot(key);
		let e = self.qc[slot];
		if e.key == key && self.certified(e.id, p) {
			return e.id;
		}
		let i = match self.search_fast(p) {
			Some(i) => i,
			None => self.push_vertex(p),
		};
		self.qc[slot] = QEntry { key, id: i };
		i
	}

	/// `search`, rejecting candidates on squared distance before the `hypot` test (which still decides every candidate that could pass).
	fn search_fast(&self, p: [f32; 3]) -> Option<u32> {
		let (cx, cy, cz) = Self::cell(p);
		let mut best: Option<(f32, u32)> = None;
		for dx in Self::reach(p[0]) {
			for dy in Self::reach(p[1]) {
				for dz in Self::reach(p[2]) {
					// Wrapping: what `search` does in a release build, without the overflow panic of a debug one (a corner at infinity).
					let cell = (cx.wrapping_add(dx), cy.wrapping_add(dy), cz.wrapping_add(dz));
					for &i in self.grid.get(&cell).into_iter().flatten() {
						let q = self.verts[i as usize];
						let (a, b, c) = (q[0] - p[0], q[1] - p[1], q[2] - p[2]);
						if !(a * a + b * b + c * c < SNAP_OUT2) {
							continue;
						}
						let d = a.hypot(b).hypot(c);
						if d < SHEET_SNAP && best.map_or(true, |(e, _)| d < e) {
							best = Some((d, i));
						}
					}
				}
			}
		}
		best.map(|(_, i)| i)
	}

	/// `find`, through `search_fast`.
	fn find_fast(&mut self, p: [f32; 3]) -> Option<u32> {
		let key = [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
		if let Some(&i) = self.memo.get(&key) {
			return Some(i);
		}
		let i = self.search_fast(p)?;
		if self.memo.len() > 100_000 {
			self.memo.clear();
		}
		self.memo.insert(key, i);
		Some(i)
	}

	/// `refill`, appending the refill vertices to `out`. `drawn`: this frame's corner ids (after `observe_fast`).
	pub(super) fn refill_fast(&mut self, drawn: &[u32], cand: &[Vertex], out: &mut Vec<Vertex>) {
		// Every drawn triangle was added to `known` by `observe_fast`, so the drawn set is just their sorted triples.
		let mut shown = std::mem::take(&mut self.shown);
		shown.clear();
		for q in drawn.chunks_exact(4) {
			let mut f = [q[0], q[1], q[2]];
			f.sort_unstable();
			if f[0] != f[1] && f[1] != f[2] {
				shown.insert(f);
			}
		}
		for q in cand.chunks_exact(4) {
			let (Some(a), Some(b), Some(c)) = (self.find_fast(q[0].pos), self.find_fast(q[1].pos), self.find_fast(q[2].pos)) else { continue };
			let mut f = [a, b, c];
			f.sort_unstable();
			if f[0] != f[1] && f[1] != f[2] && self.known.contains(&f) && !shown.contains(&f) {
				let ids = [a, b, c, c];
				out.extend(q.iter().zip(ids).map(|(v, i)| Vertex { light: self.light[i as usize], ..*v }));
			}
		}
		self.shown = shown;
	}
}
