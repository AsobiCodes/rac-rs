//! Waterfall sheets (Pokitaru): sheet shaping, `SheetMem` (the sheets as seen over time), the fast hash it uses, the foot
//! finder, and `WaterFx::sheet_falls`.

use super::*;

/// [PC] The foot of a steep quad (its bottom edge's middle) as a waterfall foot, one per CHURN_CELL.
pub(super) fn push_foot(falls: &mut Vec<[f32; 3]>, pos: &[[f32; 3]; 4]) {
	let mut v = *pos;
	v.sort_by(|a, b| a[2].total_cmp(&b[2]));
	let f = [(v[0][0] + v[1][0]) * 0.5, (v[0][1] + v[1][1]) * 0.5, (v[0][2] + v[1][2]) * 0.5];
	if !falls.iter().any(|g| (g[0] - f[0]).abs() < CHURN_CELL && (g[1] - f[1]).abs() < CHURN_CELL && (g[2] - f[2]).abs() < 1.0) {
		falls.push(f);
	}
}

/// [PC] How far from a waterfall's brink `shape_faces` looks (world units); farther points count as this far.
const BRINK_REACH: f32 = 4.0;

/// [PC] A sheet face's normal and its length, from its corners p0, p1 and the last; None when degenerate.
fn face_normal(c: &[[f32; 3]]) -> Option<([f32; 3], f32)> {
	let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
	let (a, b) = (sub(c[1], c[0]), sub(c[c.len() - 1], c[0]));
	let n = [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]];
	let nl = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
	if nl < 1e-9 { None } else { Some((n, nl)) }
}

/// [PC] Shapes Pokitaru's waterfall sheets (`sheet_falls`; retail's quads, in fours, are triangles of ribbons with p3 == p2)
/// for `fs_fall`: each vertex's `light` = [steepness (0 flat .. 1 straight down) averaged over the faces sharing the vertex,
/// signed distance d from the nearest brink (where the sheet tips from flat to steep going down; < 0 on the water running to
/// it, > 0 down the fall, +-BRINK_REACH beyond), 1, 1 on the flat skirt at a foot (water falls onto it) else 0]. Interpolated over each face, so the bend over the edge is shaded
/// smoothly instead of facet by facet.
/// `pos`: the sheet's vertices, each once; `faces`: their corner ids in fours (p3 == p2: a triangle); `steep`: each vertex's
/// steepness. Sets the `light` of every vertex of `faces`.
fn shape_faces(pos: &[[f32; 3]], faces: &[[u32; 4]], steep: &[f32], light: &mut [[f32; 4]]) {
	use std::collections::HashMap;
	let sub = |a: [f32; 3], b: [f32; 3]| [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
	let corners = |f: &[u32; 4]| -> std::ops::Range<usize> { if f[3] == f[2] { 0..3 } else { 0..4 } };
	let mut used = vec![false; pos.len()];
	for f in faces {
		for k in corners(f) {
			used[f[k] as usize] = true;
		}
	}
	// Brink points: on each edge from a flat corner down to a steep one, where the steepness crosses 0.5; not where falling
	// water comes down from above, straight or slanting (a step in a fall, the skirt at a foot: retail's sheets reach on below
	// the pond surface).
	let cell = |p: [f32; 3]| ((p[0] / BRINK_REACH).floor() as i32, (p[1] / BRINK_REACH).floor() as i32);
	let mut steeps: HashMap<(i32, i32), Vec<[f32; 3]>> = HashMap::new();
	for (i, &p) in pos.iter().enumerate() {
		if used[i] && steep[i] >= 0.5 {
			steeps.entry(cell(p)).or_default().push(p);
		}
	}
	let fed = |m: [f32; 3]| {
		let (cx, cy) = cell(m);
		(-1..=1).any(|dx| (-1..=1).any(|dy| steeps.get(&(cx + dx, cy + dy)).into_iter().flatten().any(|q| q[2] > m[2] + 0.5 && (q[0] - m[0]).hypot(q[1] - m[1]) < 1.5 + (q[2] - m[2]))))
	};
	let mut brinks: HashMap<(i32, i32), Vec<[f32; 3]>> = HashMap::new();
	for face in faces {
		let c = &face[corners(face)];
		for k in 0..c.len() {
			let (ia, ib) = (c[k] as usize, c[(k + 1) % c.len()] as usize);
			let (a, b, sa, sb) = (pos[ia], pos[ib], steep[ia], steep[ib]);
			if (sa < 0.5) == (sb < 0.5) {
				continue;
			}
			let (f, s, sf, ss) = if sa < 0.5 { (a, b, sa, sb) } else { (b, a, sb, sa) };
			if f[2] <= s[2] {
				continue;
			}
			let t = (0.5 - sf) / (ss - sf);
			let m = [f[0] + (s[0] - f[0]) * t, f[1] + (s[1] - f[1]) * t, f[2] + (s[2] - f[2]) * t];
			if !fed(m) {
				brinks.entry(cell(m)).or_default().push(m);
			}
		}
	}
	for (i, &p) in pos.iter().enumerate() {
		if !used[i] {
			continue;
		}
		let st = steep[i];
		let (cx, cy) = cell(p);
		let mut best: Option<(f32, [f32; 3])> = None;
		for dx in -1..=1 {
			for dy in -1..=1 {
				for b in brinks.get(&(cx + dx, cy + dy)).into_iter().flatten() {
					let d = sub(p, *b);
					let d = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
					if d < BRINK_REACH && best.map_or(true, |(e, _)| d < e) {
						best = Some((d, *b));
					}
				}
			}
		}
		// Far from any brink: steep is down the fall; flat is the skirt at a foot when water falls onto it, else upstream.
		let (d, foot) = match best {
			Some((d, b)) if p[2] < b[2] - 0.05 => (d, 0.0),
			Some((d, _)) => (-d, 0.0),
			None if st >= 0.5 => (BRINK_REACH, 0.0),
			None if fed(p) => (BRINK_REACH, 1.0),
			None => (-BRINK_REACH, 0.0),
		};
		light[i] = [st, d, 1.0, foot];
	}
}

/// [PC] How close (world units) two reconstructed sheet vertices are to count as one (`SheetMem`): retail's positions are rebuilt from
/// GS depth, so a vertex wobbles by up to about 0.1 from frame to frame; distinct vertices lie at least 0.37 apart.
const SHEET_SNAP: f32 = 0.18;

/// [PC] Pokitaru's waterfall sheets over time: unique vertices (the first position seen, within SHEET_SNAP), the triangles seen so far,
/// and the shape (`shape_faces`) and feet of that whole set, recomputed only when a new triangle shows up. Each vertex's steepness
/// and the feet are summed up as triangles come, so a recompute shapes each vertex once (not once per triangle corner: the
/// camera turning brings new triangles nearly every frame, 2026-10-19).
/// Per frame it looks up some 5000 corners (1300 quads at Pokitaru's bay): `memo` remembers each exact position's vertex, the
/// grid search checks only the cells SHEET_SNAP reaches into, and the maps hash with `FastHash` (with std's hasher and all 27
/// cells per lookup this took ~2 ms a frame on a fast PC, 2026-10-19).
#[derive(Default)]
pub(super) struct SheetMem {
	verts: Vec<[f32; 3]>,
	pub(super) light: Vec<[f32; 4]>,
	grid: FastMap<(i32, i32, i32), Vec<u32>>,
	known: std::collections::HashSet<[u32; 3], std::hash::BuildHasherDefault<FastHash>>,
	/// Exact position (bits) -> its vertex (`find`); cleared when a vertex is added (the nearest may change).
	memo: FastMap<[u32; 3], u32>,
	/// Corner ids in fours, p3 == p2.
	faces: Vec<[u32; 4]>,
	/// Per vertex: the steepness of its triangles summed, and their count.
	steep: Vec<(f32, f32)>,
	/// The feet of the steep triangles in the order seen (`push_foot`), and the lowest of them (`keep_lowest`).
	feet_seen: Vec<[f32; 3]>,
	pub(super) feet: Vec<[f32; 3]>,
}

impl SheetMem {
	const CELL: f32 = 0.5;

	pub(super) fn cell(p: [f32; 3]) -> (i32, i32, i32) {
		((p[0] / Self::CELL).floor() as i32, (p[1] / Self::CELL).floor() as i32, (p[2] / Self::CELL).floor() as i32)
	}

	/// The vertex within SHEET_SNAP of `p` (the nearest), a new one if none.
	pub(super) fn id(&mut self, p: [f32; 3]) -> u32 {
		if let Some(i) = self.find(p) {
			return i;
		}
		let (cx, cy, cz) = Self::cell(p);
		let i = self.verts.len() as u32;
		self.verts.push(p);
		self.light.push([0.0; 4]);
		self.steep.push((0.0, 0.0));
		self.grid.entry((cx, cy, cz)).or_default().push(i);
		self.memo.clear();
		i
	}

	/// The vertex within SHEET_SNAP of `p` (the nearest), remembered per exact position.
	pub(super) fn find(&mut self, p: [f32; 3]) -> Option<u32> {
		let key = [p[0].to_bits(), p[1].to_bits(), p[2].to_bits()];
		if let Some(&i) = self.memo.get(&key) {
			return Some(i);
		}
		let i = self.search(p)?;
		// Positions that never settle (`observe`'s runaway reset) must not grow it without end.
		if self.memo.len() > 100_000 {
			self.memo.clear();
		}
		self.memo.insert(key, i);
		Some(i)
	}

	/// The cells next to p's (-1, 0, 1 per axis) that SHEET_SNAP reaches into.
	fn reach(v: f32) -> std::ops::RangeInclusive<i32> {
		let f = v / Self::CELL - (v / Self::CELL).floor();
		// A little wider than SHEET_SNAP / CELL so rounding never skips a cell that holds the nearest vertex.
		let r = SHEET_SNAP / Self::CELL + 1e-3;
		(if f < r { -1 } else { 0 })..=(if f > 1.0 - r { 1 } else { 0 })
	}

	/// The vertex within SHEET_SNAP of `p` (the nearest), searched in the grid.
	fn search(&self, p: [f32; 3]) -> Option<u32> {
		let (cx, cy, cz) = Self::cell(p);
		let mut best: Option<(f32, u32)> = None;
		for dx in Self::reach(p[0]) {
			for dy in Self::reach(p[1]) {
				for dz in Self::reach(p[2]) {
					for &i in self.grid.get(&(cx + dx, cy + dy, cz + dz)).into_iter().flatten() {
						let q = self.verts[i as usize];
						let d = (q[0] - p[0]).hypot(q[1] - p[1]).hypot(q[2] - p[2]);
						if d < SHEET_SNAP && best.map_or(true, |(e, _)| d < e) {
							best = Some((d, i));
						}
					}
				}
			}
		}
		best.map(|(_, i)| i)
	}

	/// [PC] Widescreen refill (retail/efx_draw.rs `FX_REFILL`): of `cand` (triangles in fours, p3 == p2), the ones that are a sheet
	/// triangle seen kicked before and not among `drawn` (this frame's kicked vertices, after `observe`), with their `light`.
	/// `drawn`: this frame's kicked triangles as `observe` returned their vertices.
	fn refill(&mut self, drawn: &[u32], cand: &[Vertex]) -> Vec<Vertex> {
		let tri = |known: &std::collections::HashSet<[u32; 3], _>, mut f: [u32; 3]| -> Option<[u32; 3]> {
			f.sort_unstable();
			(f[0] != f[1] && f[1] != f[2] && known.contains(&f)).then_some(f)
		};
		let shown: std::collections::HashSet<[u32; 3], std::hash::BuildHasherDefault<FastHash>> = drawn.chunks_exact(4).filter_map(|q| tri(&self.known, [q[0], q[1], q[2]])).collect();
		let mut out = Vec::new();
		for q in cand.chunks_exact(4) {
			let (Some(a), Some(b), Some(c)) = (self.find(q[0].pos), self.find(q[1].pos), self.find(q[2].pos)) else { continue };
			if tri(&self.known, [a, b, c]).is_some_and(|f| !shown.contains(&f)) {
				let ids = [a, b, c, c];
				out.extend(q.iter().zip(ids).map(|(v, i)| Vertex { light: self.light[i as usize], ..*v }));
			}
		}
		out
	}

	/// This frame's sheet vertices (triangles in fours, p3 == p2): adds the new triangles, and sets each vertex's `light`.
	/// Returns each vertex's id.
	fn observe(&mut self, verts: &mut [Vertex]) -> Vec<u32> {
		// Pokitaru's sheets are the same in every visit, so nothing is forgotten (a runaway set is: positions that never settle).
		if self.verts.len() > 20_000 {
			*self = SheetMem::default();
		}
		let ids: Vec<u32> = verts.iter().map(|v| self.id(v.pos)).collect();
		let mut dirty = false;
		for q in ids.chunks_exact(4) {
			let mut f = [q[0], q[1], q[2]];
			f.sort_unstable();
			if self.known.insert(f) {
				let pos = [self.verts[q[0] as usize], self.verts[q[1] as usize], self.verts[q[2] as usize], self.verts[q[2] as usize]];
				if let Some((n, nl)) = face_normal(&pos[..3]) {
					let st = 1.0 - n[2].abs() / nl;
					for &i in &q[..3] {
						let s = &mut self.steep[i as usize];
						s.0 += st;
						s.1 += 1.0;
					}
					if n[2].abs() < 0.5 * nl {
						push_foot(&mut self.feet_seen, &pos);
					}
				}
				self.faces.push([q[0], q[1], q[2], q[2]]);
				dirty = true;
			}
		}
		if dirty {
			let steep: Vec<f32> = self.steep.iter().map(|&(s, n)| if n > 0.0 { s / n } else { 0.0 }).collect();
			shape_faces(&self.verts, &self.faces, &steep, &mut self.light);
			self.feet = self.feet_seen.clone();
			keep_lowest(&mut self.feet);
		}
		for (v, &i) in verts.iter_mut().zip(&ids) {
			v.light = self.light[i as usize];
		}
		ids
	}
}

/// [PC] A fast hasher for small integer keys (rustc's Fx: rotate, xor, multiply); `SheetMem`'s per-frame lookups.
#[derive(Default, Clone, Copy)]
struct FastHash(u64);

impl FastHash {
	pub(super) fn add(&mut self, x: u64) {
		self.0 = (self.0.rotate_left(5) ^ x).wrapping_mul(0x517c_c1b7_2722_0a95);
	}
}

impl std::hash::Hasher for FastHash {
	fn finish(&self) -> u64 {
		self.0
	}
	fn write(&mut self, b: &[u8]) {
		for c in b.chunks(8) {
			let mut w = [0u8; 8];
			w[..c.len()].copy_from_slice(c);
			self.add(u64::from_le_bytes(w));
		}
	}
	fn write_u32(&mut self, x: u32) {
		self.add(x as u64);
	}
	fn write_i32(&mut self, x: i32) {
		self.add(x as u32 as u64);
	}
	fn write_u64(&mut self, x: u64) {
		self.add(x);
	}
	fn write_usize(&mut self, x: usize) {
		self.add(x as u64);
	}
}

type FastMap<K, V> = std::collections::HashMap<K, V, std::hash::BuildHasherDefault<FastHash>>;

/// [PC] Only the lowest foot of a falling sheet meets the water below it.
pub(super) fn keep_lowest(falls: &mut Vec<[f32; 3]>) {
	let all = std::mem::take(falls);
	falls.extend(all.iter().copied().filter(|f| !all.iter().any(|g| g[2] < f[2] - 0.5 && (g[0] - f[0]).powi(2) + (g[1] - f[1]).powi(2) < 9.0)));
}

/// The world-from-uv map of a strip: the least-squares fit uv = A (x, y) + c over its vertices, inverted (rows of A^-1); none
/// when the fit is degenerate.
pub(super) fn uv_map(pos: &[[f32; 3]], uv: &[[f32; 2]]) -> Option<[[f32; 2]; 2]> {
	let n = pos.len().min(uv.len());
	if n < 3 {
		return None;
	}
	let k = n as f64;
	let (mx, my) = (pos[..n].iter().map(|p| p[0] as f64).sum::<f64>() / k, pos[..n].iter().map(|p| p[1] as f64).sum::<f64>() / k);
	let (mu, mv) = (uv[..n].iter().map(|u| u[0] as f64).sum::<f64>() / k, uv[..n].iter().map(|u| u[1] as f64).sum::<f64>() / k);
	let (mut sxx, mut sxy, mut syy, mut sux, mut suy, mut svx, mut svy) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
	for (p, u) in pos[..n].iter().zip(&uv[..n]) {
		let (x, y, a, b) = (p[0] as f64 - mx, p[1] as f64 - my, u[0] as f64 - mu, u[1] as f64 - mv);
		sxx += x * x;
		sxy += x * y;
		syy += y * y;
		sux += a * x;
		suy += a * y;
		svx += b * x;
		svy += b * y;
	}
	let det = sxx * syy - sxy * sxy;
	if det.abs() < 1e-9 {
		return None;
	}
	// A = S_uv,xy S_xy,xy^-1 (2x2).
	let ixx = [[syy / det, -sxy / det], [-sxy / det, sxx / det]];
	let a = [[sux * ixx[0][0] + suy * ixx[1][0], sux * ixx[0][1] + suy * ixx[1][1]], [svx * ixx[0][0] + svy * ixx[1][0], svx * ixx[0][1] + svy * ixx[1][1]]];
	let da = a[0][0] * a[1][1] - a[0][1] * a[1][0];
	if da.abs() < 1e-12 {
		return None;
	}
	Some([[(a[1][1] / da) as f32, (-a[0][1] / da) as f32], [(-a[1][0] / da) as f32, (a[0][0] / da) as f32]])
}

impl WaterFx {
	/// [PC] Waterfall sheets drawn as effect quads (`fall_fx_tex`, Pokitaru): the foot of each steep quad churns the water it
	/// falls into, as a river's waterfall does (`quad_flows`), and the sheets are drawn as falling water (`fs_fall`) instead of
	/// retail's own draw (gpu.rs leaves them out of the level pass).
	/// After `quad_flows`, whose feet it keeps. Each quad: its corners' position, uv and colour.
	/// `refill`: a widescreen refill candidate (retail/efx_draw.rs `FX_REFILL`), drawn only as a known sheet triangle (`SheetMem::refill`).
	pub(in crate::viewer) fn sheet_falls(&mut self, level: usize, quads: impl Iterator<Item = ([[f32; 3]; 4], [[f32; 2]; 4], [[f32; 4]; 4], bool)>) {
		// The sheets seen so far belong to one level: another level (or the first visit of this run's level) starts empty.
		if self.sheet_level != level {
			self.sheet_level = level;
			self.sheet_mem = SheetMem::default();
		}
		if !self.falls_fresh {
			self.falls.clear();
		}
		self.falls_fresh = true;
		let start = self.fall_verts.len();
		// RAC_FALL_LOG=path (dev): appends this frame's sheet quads (positions, uv, colour) for flicker analysis.
		let (refill, quads): (Vec<_>, Vec<_>) = quads.partition(|q| q.3);
		let quads: Vec<_> = quads.into_iter().map(|(p, u, c, _)| (p, u, c)).collect();
		static LOG: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
		let log = LOG.get_or_init(|| std::env::var("RAC_FALL_LOG").ok());
		if let Some(path) = log {
			use std::io::Write;
			static FRAME: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
			let n = FRAME.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
			if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
				let _ = writeln!(f, "FRAME {n} quads {}", quads.len());
				for (pos, uv, colour) in &quads {
					let _ = writeln!(f, "{:?} {:?} {:?}", pos, uv, colour);
				}
			}
		}
		for (pos, uv, colour) in quads {
			// Retail scrolls their v up 0.01 a tick [C: quad dump at fullgame1 0447, 2026-10-17].
			// n.y = 2: `fs_fall` builds the streaks from retail's texture.
			self.fall_verts.extend((0..4).map(|k| Vertex { pos: pos[k], uv: uv[k], normal: [0.0, 2.0, 0.0], colour: colour[k], light: [0.0; 4] }));
		}
		{
			// The shape and the feet come from every triangle seen so far, not from the ones retail happened to draw this frame
			// (it leaves some out with the camera: their neighbours' steepness and brinks, and so the look, popped from frame to frame).
			let drawn = self.sheet_mem.observe(&mut self.fall_verts[start..]);
			let cand: Vec<Vertex> = refill.iter().flat_map(|(pos, uv, colour, _)| (0..4).map(move |k| Vertex { pos: pos[k], uv: uv[k], normal: [0.0, 2.0, 0.0], colour: colour[k], light: [0.0; 4] })).collect();
			let more = self.sheet_mem.refill(&drawn, &cand);
			self.fall_verts.extend(more);
			for f in &self.sheet_mem.feet {
				if !self.falls.iter().any(|g| (g[0] - f[0]).abs() < CHURN_CELL && (g[1] - f[1]).abs() < CHURN_CELL && (g[2] - f[2]).abs() < 1.0) {
					self.falls.push(*f);
				}
			}
			keep_lowest(&mut self.falls);
		}
		if let Some(path) = log {
			use std::io::Write;
			if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
				for v in &self.fall_verts[start..] {
					let _ = writeln!(f, "SHAPE {:?} {:?}", v.pos, v.light);
				}
			}
		}
	}

	/// [PC] Before `strip_flows` / `quad_flows` / `sheet_falls`: drops waterfall sheets a frame without a water pass left.
	pub(in crate::viewer) fn clear_falls(&mut self) {
		self.fall_verts.clear();
	}

	/// [PC] This frame has waterfall sheets to draw (the water pass runs for them even with no water in view).
	pub(in crate::viewer) fn has_falls(&self) -> bool {
		!self.fall_verts.is_empty()
	}
}
