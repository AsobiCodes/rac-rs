//! [PC] Watertight effect meshes on smoothed frames (lane yflk, 2026-10-24: Kalebo III sea "crack", a 1-2 px line across the sea for
//! about a field while the camera turned fast). The queue-A / queue-B quads blend corner by corner with their partner of last field
//! (`quad_pairs` by order, `still::still_pairs` by position), each quad on its own. Neighbours of a mesh share corners in this field,
//! but they need not share them in their partners (paired to different triangles of last field, one paired and its neighbour not):
//! on the in-between frames the shared edge then split and the scene under it showed through.
//! Once per field every corner is welded by its exact position in this field (the effects layer gives a shared GS vertex the same
//! world point bit for bit), across both queues and every kind (a sea's layers lie on the same vertices). A welded vertex gets ONE
//! displacement towards last field: the one its paired corners agree on, else their mean, or the smallest of them when they
//! disagree by more than `SNAP` (one corner paired with another triangle of last field: Kalebo III flick runs, up to 2.7 m;
//! its partner's move is the wrong one, a standing mesh barely moves); every quad with a corner on a moved
//! vertex moves that corner with it, also an unpaired quad. So the drawn mesh is the field's own mesh, each vertex moved once.
//! Each display frame only adds `d * (1 - t)` (`apply`); colours and texture coordinates still blend per quad (they do not open gaps).
use super::V3;
use super::still::StillPair;
use crate::particles::WorldQuad;

/// Paired corners of one vertex that disagree by more than this (world units) are a mispairing: the smallest move wins.
pub(super) const SNAP: f32 = 0.05;

/// Per queue (A, B): (quad index in this field's list, displacement of each corner towards last field at t = 0).
pub(super) type Weld = [Vec<(u32, [V3; 4])>; 2];

/// What `weld` found (RAC_SMOOTH_LOG): welded vertices (shared by more than one corner), vertices whose paired corners disagreed,
/// largest disagreement (world units), unpaired corners moved with a paired neighbour.
#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub(super) struct WeldStat {
	pub shared: u32,
	pub split: u32,
	pub spread: f32,
	pub carried: u32,
}

/// One welded vertex: its position bits, (paired corners seen, first displacement, sum of displacements, smallest displacement,
/// all equal so far, corners on it, largest disagreement).
#[derive(Clone, Copy)]
struct Vert {
	key: [u32; 3],
	n: u32,
	first: V3,
	sum: V3,
	small: V3,
	same: bool,
	corners: u32,
	spread: f32,
}

fn slot_hash(k: [u32; 3]) -> usize {
	let mut h = (k[0] as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
	h = (h ^ k[1] as u64).wrapping_mul(0xFF51_AFD7_ED55_8CCD);
	h = (h ^ k[2] as u64).wrapping_mul(0xC4CE_B9FE_1A85_EC53);
	(h >> 32) as usize
}

/// The vertices of this field's quads (every corner by its exact position bits, across both queues and every kind) and the
/// vertex of each corner. Needs only this field's quads, so it can be made while the pairing runs; its buffers are reused
/// from field to field.
#[derive(Default)]
pub(super) struct Verts {
	vs: Vec<Vert>,
	at: [Vec<[u32; 4]>; 2],
	slots: Vec<u32>,
}

impl Verts {
	/// Numbers the vertices in first-seen order (queue A quads, then queue B; corners in order), as the first version did.
	pub(super) fn index(&mut self, cur: [&[WorldQuad]; 2]) {
		let total: usize = cur[0].len() + cur[1].len();
		self.vs.clear();
		self.slots.clear();
		self.slots.resize((4 * total).next_power_of_two().max(16), u32::MAX);
		let mask = self.slots.len() - 1;
		for qi in 0..2 {
			self.at[qi].clear();
			self.at[qi].reserve(cur[qi].len());
			for q in cur[qi] {
				let mut a = [0u32; 4];
				for k in 0..4 {
					let key = [q.pos[k][0].to_bits(), q.pos[k][1].to_bits(), q.pos[k][2].to_bits()];
					let mut s = slot_hash(key) & mask;
					let v = loop {
						let h = self.slots[s];
						if h == u32::MAX {
							let n = self.vs.len() as u32;
							self.slots[s] = n;
							self.vs.push(Vert { key, n: 0, first: [0.0; 3], sum: [0.0; 3], small: [0.0; 3], same: true, corners: 0, spread: 0.0 });
							break n;
						}
						if self.vs[h as usize].key == key {
							break h;
						}
						s = (s + 1) & mask;
					};
					self.vs[v as usize].corners += 1;
					a[k] = v;
				}
				self.at[qi].push(a);
			}
		}
	}
}

/// The welded displacements of this field's queues `cur` against last field's `prev`, from the moving pairs `moving` (index in cur,
/// index in prev; corner k with corner k) and the standing pairs `still` (corner k with corner m[k]). Once per field.
/// `verts` = `Verts::index(cur)`; it is used up (the displacements are summed into it).
pub(super) fn weld_with(verts: &mut Verts, prev: [&[WorldQuad]; 2], cur: [&[WorldQuad]; 2], moving: &[Vec<(u32, u32)>; 2], still: &[Vec<StillPair>; 2]) -> (Weld, WeldStat) {
	let mut stat = WeldStat::default();
	if moving.iter().all(Vec::is_empty) && still.iter().all(Vec::is_empty) {
		return (Default::default(), stat);
	}
	let Verts { vs, at, .. } = verts;
	let mut paired: [Vec<bool>; 2] = [vec![false; cur[0].len()], vec![false; cur[1].len()]];
	let add = |vs: &mut Vec<Vert>, v: u32, d: V3| {
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
				add(vs, at[qi][i as usize][k], std::array::from_fn(|c| a.pos[k][c] - q.pos[k][c]));
			}
		}
		for &(i, j, m) in &still[qi] {
			let (Some(q), Some(a)) = (cur[qi].get(i as usize), prev[qi].get(j as usize)) else { continue };
			paired[qi][i as usize] = true;
			for k in 0..4 {
				add(vs, at[qi][i as usize][k], std::array::from_fn(|c| a.pos[m[k] as usize][c] - q.pos[k][c]));
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

/// `weld_with` on a fresh `Verts` (tests).
#[cfg(test)]
pub(super) fn weld(prev: [&[WorldQuad]; 2], cur: [&[WorldQuad]; 2], moving: &[Vec<(u32, u32)>; 2], still: &[Vec<StillPair>; 2]) -> (Weld, WeldStat) {
	let mut v = Verts::default();
	v.index(cur);
	weld_with(&mut v, prev, cur, moving, still)
}

/// This field's quads `qa` / `qb` (copies) moved to `t` (0 = last field's place, 1 = this field's): each listed corner by its
/// welded displacement `d * (1 - t)`.
pub(super) fn apply(qa: &mut [WorldQuad], qb: &mut [WorldQuad], t: f32, w: &Weld) {
	let s = 1.0 - t;
	for (list, moves) in [qa, qb].into_iter().zip(w) {
		for &(i, d) in moves {
			let Some(q) = list.get_mut(i as usize) else { continue };
			for k in 0..4 {
				for c in 0..3 {
					q.pos[k][c] += d[k][c] * s;
				}
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn quad(x: f32, y: f32) -> WorldQuad {
		WorldQuad { pos: [[x, y, 0.0], [x + 1.0, y, 0.0], [x + 1.0, y + 1.0, 0.0], [x, y + 1.0, 0.0]], uv: [[0.0; 2]; 4], colour: [[1.0; 4]; 4], tex: 3, additive: false }
	}

	fn moved(mut q: WorldQuad, d: V3) -> WorldQuad {
		for p in &mut q.pos {
			for c in 0..3 {
				p[c] += d[c];
			}
		}
		q
	}

	/// Two neighbours paired with partners that moved differently, and an unpaired third neighbour: on the in-between frame
	/// every shared corner of the three is drawn at one place (no gap), and t = 1 is this field's mesh exactly.
	#[test]
	fn shared_corners_stay_shared() {
		let cur = vec![quad(0.0, 0.0), quad(1.0, 0.0), quad(2.0, 0.0)];
		let prev = vec![moved(quad(0.0, 0.0), [0.0, 0.2, 0.0]), moved(quad(1.0, 0.0), [0.0, -0.1, 0.0])];
		let moving = [vec![(0, 0), (1, 1)], vec![]];
		let (w, st) = weld([&prev, &[]], [&cur, &[]], &moving, &Default::default());
		assert_eq!((st.split, st.carried), (2, 2), "{st:?}");
		let mut out = cur.clone();
		apply(&mut out, &mut [], 0.5, &w);
		// Quad 0's right edge = quad 1's left edge; quad 1's right edge = quad 2's left edge.
		assert_eq!((out[0].pos[1], out[0].pos[2]), (out[1].pos[0], out[1].pos[3]));
		assert_eq!((out[1].pos[1], out[1].pos[2]), (out[2].pos[0], out[2].pos[3]));
		// Quad 0's own corners move by its partner's displacement: 0.2 * 0.5.
		assert!((out[0].pos[0][1] - 0.1).abs() < 1e-6, "{:?}", out[0].pos);
		let mut end = cur.clone();
		apply(&mut end, &mut [], 1.0, &w);
		assert!(end.iter().zip(&cur).all(|(a, b)| a.pos == b.pos));
	}

	/// Neighbours whose partners agree are lerped as before (the weld changes nothing), and a standing pair's corner order is used.
	#[test]
	fn agreeing_pairs_are_plain_lerps() {
		let cur = vec![quad(0.0, 0.0), quad(1.0, 0.0)];
		let mut prev = vec![moved(quad(0.0, 0.0), [0.0, 0.0, 0.4]), moved(quad(1.0, 0.0), [0.0, 0.0, 0.4])];
		prev[1].pos.rotate_left(1);
		let still = [vec![(1, 1, [3, 0, 1, 2])], vec![]];
		let (w, st) = weld([&prev, &[]], [&cur, &[]], &[vec![(0, 0)], vec![]], &still);
		assert_eq!((st.split, st.carried), (0, 0), "{st:?}");
		let mut out = cur.clone();
		apply(&mut out, &mut [], 0.25, &w);
		assert!(out.iter().flat_map(|q| q.pos).all(|p| (p[2] - 0.3).abs() < 1e-6), "{out:?}");
	}
}
