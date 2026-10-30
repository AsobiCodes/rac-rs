//! [PC] Frame smoothing: the pairing of one effect queue (A or B) between last field's quads and this field's, from per-field
//! lookup tables (`QIndex`) that are made ONCE per field and then serve twice: as this field's table now, and as the next
//! field's `prev` table (the old code hashed last field's quads again for each of `still_kinds` and `still_pairs`, with
//! std-style maps keyed by (kind, cell) tuples; Pokitaru's pool and waterfall sheets are thousands of quads a field, ~2 ms of the
//! laptop's render thread a tick). The answers are the old functions' answers bit for bit (tests: `interp/oracle.rs` keeps the
//! old code and compares on synthetic fields): the same kinds, in the same order, the same partner for each quad.
//! `pairs` = `still_kinds` (which kinds stand), `quad_pairs` (the k-th quad of a kind pairs with last field's k-th, `pair_ordered`)
//! and `still_pairs` (standing quads paired by position, the grid cell of the quad's centre and the 26 around it).
use super::still::StillPair;
use super::{FxMap, STILL_EPS, V3, kind_of};
use crate::particles::WorldQuad;
use crate::viewer::smooth::near;
use std::collections::HashSet;

/// End of a chain / empty slot.
pub(super) const END: u32 = u32::MAX;

/// The grid cell of a quad's centre (0.5 world units), as the old `still_kinds` / `still_pairs` computed it.
fn cell_of(q: &WorldQuad) -> [i32; 3] {
	let m: V3 = std::array::from_fn(|c| (q.pos[0][c] + q.pos[1][c] + q.pos[2][c] + q.pos[3][c]) * 0.25);
	[(m[0] / 0.5).floor() as i32, (m[1] / 0.5).floor() as i32, (m[2] / 0.5).floor() as i32]
}

const K0: u64 = 0x9E37_79B9_7F4A_7C15;
const K1: u64 = 0xC2B2_AE3D_27D4_EB4F;
const K2: u64 = 0x1656_67B1_9E37_79F9;
const K3: u64 = 0xD6E8_FEB8_6659_FD93;

/// A linear mix of (kind id, cell): the mix of a neighbouring cell is this plus a constant (`NB_OFF`), so the 27 lookups of a
/// quad need one mix and 27 additions. Equal keys have equal mixes; different keys that collide only share a probe chain (the
/// key itself is compared). The slot is `slot_of` the mix.
fn lin(kid: u32, c: [i32; 3]) -> u64 {
	(kid as u64).wrapping_mul(K0).wrapping_add((c[0] as i64 as u64).wrapping_mul(K1)).wrapping_add((c[1] as i64 as u64).wrapping_mul(K2)).wrapping_add((c[2] as i64 as u64).wrapping_mul(K3))
}

fn slot_of(h: u64, bits: u32) -> usize {
	(h.wrapping_mul(K0) >> (64 - bits)) as usize
}

/// A quad's kind id and cell.
#[derive(Clone, Copy, Default)]
struct Meta {
	cell: [i32; 3],
	kid: u32,
}

/// The lookup tables of one queue's quad list: each quad's kind id and grid cell, the quads of each kind in order, and a flat
/// hash table from (kind id, cell) to the chain of its quads in ascending index (so `still_pairs` still finds the lowest index
/// first). Buffers are reused from field to field (`build` clears them).
#[derive(Default)]
pub(super) struct QIndex {
	kinds: Vec<(usize, bool)>,
	kind_id: FxMap<(usize, bool), u32>,
	meta: Vec<Meta>,
	count: Vec<u32>,
	ofs: Vec<u32>,
	by_kind: Vec<u32>,
	next: Vec<u32>,
	/// Scratch for `build` (the next free place of each kind), kept for its allocation.
	fill: Vec<u32>,
	slots: Vec<u32>,
	bits: u32,
	/// The serial of the field the tables were made for (`build_for`); `pair_field` rebuilds a `prev` table of another field.
	pub(super) serial: u64,
}

impl QIndex {
	pub(super) fn len(&self) -> usize {
		self.meta.len()
	}

	/// `build` for field number `serial`.
	pub(super) fn build_for(&mut self, quads: &[WorldQuad], serial: u64) {
		self.serial = serial;
		self.build(quads);
	}

	pub(super) fn build(&mut self, quads: &[WorldQuad]) {
		let n = quads.len();
		self.kinds.clear();
		self.kind_id.clear();
		self.meta.clear();
		self.count.clear();
		self.ofs.clear();
		self.by_kind.clear();
		self.next.clear();
		self.slots.clear();
		self.meta.reserve(n);
		let mut last: Option<((usize, bool), u32)> = None;
		for q in quads {
			let k = kind_of(q);
			let id = match last {
				Some((lk, id)) if lk == k => id,
				_ => {
					let fresh = self.kinds.len() as u32;
					let id = *self.kind_id.entry(k).or_insert(fresh);
					if id == fresh {
						self.kinds.push(k);
						self.count.push(0);
					}
					last = Some((k, id));
					id
				}
			};
			self.count[id as usize] += 1;
			self.meta.push(Meta { cell: cell_of(q), kid: id });
		}
		let nk = self.kinds.len();
		self.ofs.resize(nk + 1, 0);
		for k in 0..nk {
			self.ofs[k + 1] = self.ofs[k] + self.count[k];
		}
		self.by_kind.resize(n, 0);
		let mut fill = std::mem::take(&mut self.fill);
		fill.clear();
		fill.extend_from_slice(&self.ofs[..nk]);
		for (i, m) in self.meta.iter().enumerate() {
			self.by_kind[fill[m.kid as usize] as usize] = i as u32;
			fill[m.kid as usize] += 1;
		}
		self.fill = fill;
		self.bits = (2 * n).next_power_of_two().max(16).trailing_zeros();
		let size = 1usize << self.bits;
		self.slots.resize(size, END);
		self.next.resize(n, END);
		for j in (0..n).rev() {
			let m = self.meta[j];
			let mut s = slot_of(lin(m.kid, m.cell), self.bits);
			loop {
				let h = self.slots[s];
				if h == END {
					self.slots[s] = j as u32;
					break;
				}
				let o = self.meta[h as usize];
				if o.kid == m.kid && o.cell == m.cell {
					self.next[j] = h;
					self.slots[s] = j as u32;
					break;
				}
				s = (s + 1) & (size - 1);
			}
		}
	}

	/// The lowest-index quad of kind id `k` in cell `c` whose mix is `h` (END: none); the chain goes on through `next`.
	fn head_at(&self, k: u32, c: [i32; 3], h: u64) -> u32 {
		if self.slots.is_empty() {
			return END;
		}
		let mask = self.slots.len() - 1;
		let mut s = slot_of(h, self.bits);
		loop {
			let q = self.slots[s];
			if q == END {
				return END;
			}
			let m = &self.meta[q as usize];
			if m.kid == k && m.cell == c {
				return q;
			}
			s = (s + 1) & mask;
		}
	}

	/// The heads of the 27 cells round cell `c` (its own first) for kind id `k`, in the order of `AROUND`, one at a time.
	fn heads(&self, k: u32, c: [i32; 3]) -> impl Iterator<Item = u32> + '_ {
		// Next to the ends of the i32 range the cell sums wrap: use the exact mix of the wrapped cell there.
		let edge = c.iter().any(|v| v.unsigned_abs() >= i32::MAX as u32 - 1);
		let base = lin(k, c);
		(0..27).map(move |n| {
			let cc = nbr(c, AROUND[n]);
			self.head_at(k, cc, if edge { lin(k, cc) } else { base.wrapping_add(NB_OFF[n]) })
		})
	}

	fn id_of(&self, kind: (usize, bool)) -> u32 {
		self.kind_id.get(&kind).copied().unwrap_or(END)
	}
}

/// One queue's pairing: the moving quads' pairs (index in cur, index in prev), the standing quads' pairs, the standing kinds.
#[derive(Default)]
pub(super) struct QPairs {
	pub moving: Vec<(u32, u32)>,
	pub still: Vec<StillPair>,
	pub standing: HashSet<(usize, bool)>,
}

/// The 27 cells of a quad's neighbourhood: its own first, then the 26 around it.
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

/// The mix offset of each cell of `AROUND`: `lin(k, c + d) == lin(k, c) + NB_OFF[n]` (wrapping) while the cell sums do not wrap.
const NB_OFF: [u64; 27] = {
	let mut o = [0u64; 27];
	let mut n = 0;
	while n < 27 {
		let d = AROUND[n];
		o[n] = (d.0 as i64 as u64).wrapping_mul(K1).wrapping_add((d.1 as i64 as u64).wrapping_mul(K2)).wrapping_add((d.2 as i64 as u64).wrapping_mul(K3));
		n += 1;
	}
	o
};

fn close(a: V3, b: V3) -> bool {
	(0..3).all(|c| (a[c] - b[c]).abs() < STILL_EPS)
}

fn within(a: &WorldQuad, b: &WorldQuad) -> bool {
	a.pos.iter().all(|&p| b.pos.iter().any(|&q| close(p, q)))
}

fn same(a: &WorldQuad, b: &WorldQuad) -> bool {
	within(a, b) && within(b, a)
}

/// Corner k of `a` with the corner of `b` it matches, when `a` is a standing quad's partner (`StillPair`).
fn corners(a: &WorldQuad, b: &WorldQuad) -> Option<[u8; 4]> {
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
}

fn nbr(c: [i32; 3], d: (i32, i32, i32)) -> [i32; 3] {
	[c[0].wrapping_add(d.0), c[1].wrapping_add(d.1), c[2].wrapping_add(d.2)]
}

/// `still_kinds`: per kind id of `cur` whether the kind stands (at least 4 quads, at least half of them hit a `same` quad of
/// last field's in their cell or the 26 around); `map` = the kind id of each of `cur`'s kinds in `prev` (END: none).
fn standing_flags(prev_q: &[WorldQuad], cur_q: &[WorldQuad], prev: &QIndex, cur: &QIndex, map: &[u32]) -> Vec<bool> {
	// Per kind: (hits, quads searched).
	let mut st = vec![(0u32, 0u32); cur.kinds.len()];
	for (i, q) in cur_q.iter().enumerate() {
		let k = cur.meta[i].kid as usize;
		let n = cur.count[k];
		let (hit, searched) = st[k];
		if n < 4 || 2 * hit >= n || 2 * (hit + n - searched) < n {
			continue;
		}
		let c = cur.meta[i].cell;
		let found = map[k] != END
			&& prev.heads(map[k], c).any(|mut j| {
				while j != END {
					if same(&prev_q[j as usize], q) {
						return true;
					}
					j = prev.next[j as usize];
				}
				false
			});
		st[k].0 += found as u32;
		st[k].1 += 1;
	}
	(0..st.len()).map(|k| cur.count[k] >= 4 && 2 * st[k].0 >= cur.count[k]).collect()
}

/// `quad_pairs` (with `pair_ordered`): the k-th quad of a non-standing kind pairs with last field's k-th of that kind when every corner is `near`.
fn moving_pairs(prev_q: &[WorldQuad], cur_q: &[WorldQuad], prev: &QIndex, cur: &QIndex, map: &[u32], standing: &[bool]) -> Vec<(u32, u32)> {
	let mut seen = vec![0u32; cur.kinds.len()];
	let mut out = Vec::new();
	for (i, q) in cur_q.iter().enumerate() {
		let k = cur.meta[i].kid as usize;
		let n = seen[k];
		seen[k] += 1;
		let pk = map[k];
		if standing[k] || pk == END || n >= prev.count[pk as usize] {
			continue;
		}
		let j = prev.by_kind[(prev.ofs[pk as usize] + n) as usize] as usize;
		if (0..4).all(|c| near(prev_q[j].pos[c], q.pos[c])) {
			out.push((i as u32, j as u32));
		}
	}
	out
}

/// `still_pairs`: each quad of a standing kind with its partner of last field (the lowest-index `corners` match, own cell first).
fn still_partners(prev_q: &[WorldQuad], cur_q: &[WorldQuad], prev: &QIndex, cur: &QIndex, map: &[u32], standing: &[bool]) -> Vec<StillPair> {
	let mut out = Vec::new();
	if !standing.iter().any(|&s| s) {
		return out;
	}
	for (i, q) in cur_q.iter().enumerate() {
		let k = cur.meta[i].kid as usize;
		if !standing[k] || map[k] == END {
			continue;
		}
		let c = cur.meta[i].cell;
		let found = prev.heads(map[k], c).find_map(|mut j| {
			while j != END {
				if let Some(m) = corners(q, &prev_q[j as usize]) {
					return Some((j, m));
				}
				j = prev.next[j as usize];
			}
			None
		});
		if let Some((j, m)) = found {
			out.push((i as u32, j, m));
		}
	}
	out
}

/// One queue's pairing of `cur_q` (index `cur`) with last field's `prev_q` (index `prev`).
pub(super) fn pairs(prev_q: &[WorldQuad], cur_q: &[WorldQuad], prev: &QIndex, cur: &QIndex) -> QPairs {
	let map: Vec<u32> = cur.kinds.iter().map(|&k| prev.id_of(k)).collect();
	let flags = standing_flags(prev_q, cur_q, prev, cur, &map);
	let moving = moving_pairs(prev_q, cur_q, prev, cur, &map, &flags);
	let still = still_partners(prev_q, cur_q, prev, cur, &map, &flags);
	let standing = cur.kinds.iter().zip(&flags).filter(|&(_, &f)| f).map(|(&k, _)| k).collect();
	QPairs { moving, still, standing }
}

/// Tests: `pairs` on plain lists (builds both indexes).
#[cfg(test)]
pub(super) fn pairs_of(prev_q: &[WorldQuad], cur_q: &[WorldQuad]) -> QPairs {
	let (mut p, mut c) = (QIndex::default(), QIndex::default());
	p.build(prev_q);
	c.build(cur_q);
	pairs(prev_q, cur_q, &p, &c)
}

/// Tests: the standing quads' partners for the standing kinds `standing` (given, not decided by `pairs`).
#[cfg(test)]
pub(super) fn partners_of(prev_q: &[WorldQuad], cur_q: &[WorldQuad], standing: &HashSet<(usize, bool)>) -> Vec<StillPair> {
	let (mut p, mut c) = (QIndex::default(), QIndex::default());
	p.build(prev_q);
	c.build(cur_q);
	let map: Vec<u32> = c.kinds.iter().map(|&k| p.id_of(k)).collect();
	let flags: Vec<bool> = c.kinds.iter().map(|k| standing.contains(k)).collect();
	still_partners(prev_q, cur_q, &p, &c, &map, &flags)
}
