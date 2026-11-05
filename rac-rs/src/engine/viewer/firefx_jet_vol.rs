//! [PC] Fire, the Pyrocitor's stream as a true 3D flame (Fire on, volumes): the planner's node chain (firefx_jet.rs) is cut into a
//! few slabs, each a fire volume of kind 2 (firefx_vol.wgsl `jet_at`). A slab runs from one node to another; its ends are the bisector
//! planes of the chain at those nodes, so the slabs tile the stream without gaps or overlaps, and the field is continuous across them
//! (distance from the nozzle, radius and the axis point all meet at the shared plane). The noise runs in the stream's own frame:
//! along = metres from the start of the chain, across = the offset square to the jet direction. The field: a cone that opens from
//! the nozzle (it starts `BACK` half widths before the first node), torn by a domain-warped noise that flows along the stream;
//! white-yellow along the axis near the nozzle, orange in the body, dark red and smoke where it breaks up toward the tip. The puffs of
//! firefx_jet_quads.rs stay for `fire_vol=0` and for a jet that no longer fits under the volume cap. Dev knobs (`RAC_TUNE`):
//! fire_jet_vw (width), fire_jet_slab (slab length); fire_jet_flow / core / turb / hot are the shader's row (firefx_vol.rs `rows`).

use super::firefx::tune;
use super::firefx_jet::{Jet, Node};
use super::firefx_vol::{Vol, KIND_JET};

/// The most slabs a jet is cut into (a jet is at most 28 nodes, `firefx_jet::quality`).
pub(super) const MAX_SLABS: usize = 8;
/// How far before the first node the flame starts, in that node's half widths (its segments' quads reached back to the nozzle; a
/// flame starting at the node's centre floated in front of the gun).
const BACK: f32 = 1.0;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
	[a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn unit(a: [f32; 3]) -> Option<[f32; 3]> {
	let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
	(l > 1e-5).then(|| [a[0] / l, a[1] / l, a[2] / l])
}

/// Where the slabs meet, as distances along the chain (the nodes' `d`; the first `.1` of `.0`) and the slab length: 0, seg, 2 seg, ...,
/// the chain end, seg = max(1 m, 2.5 x the alpha-weighted mean half width) stretched so at most `MAX_SLABS` fit the jet's length (not
/// the visible end: a tail node fading out moves no other cut); a last slab under a
/// tenth of seg joins the one before. The end is the jet's length plus a node step, cut one node after the last that shows (a faded
/// tail costs nothing). None for an invisible jet. Distances, not node indices (lane nfire): every input moves continuously while the
/// frame smoothing blends the jets (firefx_jet_lerp.rs), so the slabs slide instead of jumping to other nodes whenever the visible node
/// count changed (each new field and each tail node fading out moved every slab end and the whole flame's pattern at once).
fn cuts(jet: &Jet) -> Option<([f32; MAX_SLABS + 1], usize, f32)> {
	let n = jet.nodes.iter().rposition(|nd| nd.alpha > 0.004).map_or(0, |i| (i + 1).min(jet.nodes.len() - 1));
	if n == 0 {
		return None;
	}
	let nodes = &jet.nodes[..=n];
	let end = nodes[n].d.min(jet.len + jet.step);
	if end <= 1e-3 {
		return None;
	}
	let (w, h) = nodes.iter().fold((0.0f32, 0.0f32), |(w, h), nd| (w + nd.alpha, h + nd.alpha * nd.half));
	let mean = if w > 1e-6 { h / w } else { nodes.iter().map(|nd| nd.half).sum::<f32>() / (n + 1) as f32 };
	let seg = ((2.5 * mean).max(1.0) * tune("fire_jet_slab", 1.0).max(0.2)).max((jet.len + jet.step) / MAX_SLABS as f32);
	let mut k = ((end / seg - 1e-4).ceil() as usize).clamp(1, MAX_SLABS);
	if k > 1 && end - (k - 1) as f32 * seg < 0.1 * seg {
		k -= 1;
	}
	let mut out = [0.0f32; MAX_SLABS + 1];
	for (j, o) in out.iter_mut().enumerate().take(k) {
		*o = j as f32 * seg;
	}
	out[k] = end;
	Some((out, k + 1, seg))
}

/// The chain at `x` metres along it (the nodes' `d`): position, half width, alpha and tint between the two nodes around it.
fn point(nodes: &[Node], x: f32) -> Node {
	let i = nodes.windows(2).position(|w| w[1].d > x).unwrap_or(nodes.len() - 1);
	let (a, b) = (&nodes[i], &nodes[(i + 1).min(nodes.len() - 1)]);
	let f = if b.d - a.d > 1e-6 { ((x - a.d) / (b.d - a.d)).clamp(0.0, 1.0) } else { 0.0 };
	let m = |p: f32, q: f32| p + (q - p) * f;
	Node { pos: std::array::from_fn(|c| m(a.pos[c], b.pos[c])), half: m(a.half, b.half), alpha: m(a.alpha, b.alpha), tint: std::array::from_fn(|c| m(a.tint[c], b.tint[c])), d: x, s: m(a.s, b.s) }
}

/// Appends the jet's slabs (kind 2 volumes, nozzle first) to `out` when there are at most `room` of them and returns true; else
/// leaves `out` alone (no slabs, or over the cap). Rows: a = (P0, r0), b = (chain end s, seed, alpha at P1, kind), c = (tint, alpha
/// at P0), d = (P1, r1), e = (bisector normal at P0, s at P0), f = (bisector normal at P1, s at P1), g = (jet direction, width gain
/// fire_jet_vw; the noise tile is firefx_vol.wgsl JTILE x it: a fixed size, so the pattern does not change with the chain from frame
/// to frame). Distances s are metres from the flame's start (`BACK` before the first node); the normals point toward the tip.
pub(super) fn slabs_into(jet: &Jet, room: usize, out: &mut Vec<Vol>) -> bool {
	let Some((cut, len, seg)) = cuts(jet) else {
		return false;
	};
	if len - 1 > room {
		return false;
	}
	let mut pts = [point(&jet.nodes, 0.0); MAX_SLABS + 1];
	for (p, &x) in pts.iter_mut().zip(&cut[..len]).skip(1) {
		*p = point(&jet.nodes, x);
	}
	let nd = |i: usize| &pts[i];
	let mut dirs = [[1.0f32, 0.0, 0.0]; MAX_SLABS];
	for k in 0..len - 1 {
		dirs[k] = unit(sub(pts[k + 1].pos, pts[k].pos)).unwrap_or(if k > 0 { dirs[k - 1] } else { [1.0, 0.0, 0.0] });
	}
	let dirs = &dirs[..len - 1];
	let w = unit(sub(nd(len - 1).pos, nd(0).pos)).unwrap_or(dirs[0]);
	let gain = tune("fire_jet_vw", 1.0).max(0.1);
	// The flame starts `back` metres before the first node (the mean of the first segments, whose quads reached back to the nozzle):
	// distances are counted from there.
	let back = BACK * nd(0).half;
	// A piece cut off from the stream starts `d0` down it (`Jet::d0`): no nozzle core, no mouth; the shader fades its start in from
	// there (b.w = KIND_JET + d0).
	let d0 = jet.d0.max(0.0);
	let send = nd(len - 1).d + back + d0;
	// The plane at cut k: the bisector of the two slabs meeting there, each weighed by its length up to half a slab, so a slab growing
	// out of the chain's end turns the plane in from the old end plane (the first and the last: the slab's own direction).
	let weight = |k: usize| ((cut[k + 1] - cut[k]) / (0.5 * seg)).clamp(0.0, 1.0);
	let normal = |k: usize| -> [f32; 3] {
		let (ia, ib) = (k.saturating_sub(1).min(dirs.len() - 1), k.min(dirs.len() - 1));
		let (a, b, wa, wb) = (dirs[ia], dirs[ib], weight(ia), weight(ib));
		if ia == ib {
			return b;
		}
		unit([a[0] * wa + b[0] * wb, a[1] * wa + b[1] * wb, a[2] * wa + b[2] * wb]).unwrap_or(b)
	};
	out.extend((0..dirs.len()).map(|k| {
		let (p, q) = (nd(k), nd(k + 1));
		let (n0, n1) = (normal(k), normal(k + 1));
		let tint: [f32; 3] = std::array::from_fn(|c| 0.5 * (p.tint[c] + q.tint[c]));
		// The first slab reaches back to the flame's start along its own direction.
		let p0: [f32; 3] = if k == 0 { std::array::from_fn(|c| p.pos[c] - dirs[0][c] * back) } else { p.pos };
		Vol {
			a: [p0[0], p0[1], p0[2], (p.half * gain).max(0.02)],
			b: [send, jet.seed, q.alpha, KIND_JET + d0],
			c: [tint[0], tint[1], tint[2], p.alpha],
			d: [q.pos[0], q.pos[1], q.pos[2], (q.half * gain).max(0.02)],
			e: [n0[0], n0[1], n0[2], if k == 0 { d0 } else { p.d + back + d0 }],
			f: [n1[0], n1[1], n1[2], q.d + back + d0],
			g: [w[0], w[1], w[2], gain],
		}
	}));
	true
}

#[cfg(test)]
mod tests {
	use super::*;
	use super::super::firefx_jet::{build, Quality, Seg};

	const Q: Quality = Quality { step: 0.4, nodes: 28, emitters: 3 };

	/// A stream of `n` segments from the hero along a gentle curve (x out, y bending), 1 m wide.
	fn jet(n: usize) -> Jet {
		let segs: Vec<Seg> = (0..n).map(|i| i as f32 * 0.1).map(|x| Seg { pos: [x, 0.04 * x * x, 1.0], size: 1.0, colour: [1.0, 0.6, 0.2, 0.8] }).collect();
		build(&segs, &(0..n).collect::<Vec<_>>(), Some([-1.0, 0.0, 1.0]), Q, 1.0).expect("a jet")
	}

	fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
		a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
	}

	fn slabs(jet: &Jet) -> Vec<Vol> {
		let mut v = Vec::new();
		slabs_into(jet, MAX_SLABS, &mut v);
		v
	}

	#[test]
	fn slabs_go_in_only_when_they_all_fit() {
		let j = jet(80);
		let n = slabs(&j).len();
		let keep = Vol { a: [9.0; 4], ..bytemuck::Zeroable::zeroed() };
		let mut v = vec![keep];
		assert!(!slabs_into(&j, n - 1, &mut v) && v == [keep], "over the room: nothing added");
		assert!(slabs_into(&j, n, &mut v) && v.len() == n + 1 && v[0] == keep, "appended after what was there");
		assert_eq!(&v[1..], &slabs(&j)[..]);
	}

	fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
		[a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
	}

	fn len(a: [f32; 3]) -> f32 {
		dot(a, a).sqrt()
	}

	/// firefx_vol.wgsl `jet_at`'s noise frame (U1, U2) for the jet direction w, on the CPU (the test checks the shader has these lines).
	fn frame(w: [f32; 3]) -> ([f32; 3], [f32; 3]) {
		let fa = 1.0 / (1.0 + w[2]).max(1.0e-6);
		let u1 = [1.0 - w[0] * w[0] * fa, -w[0] * w[1] * fa, -w[0]];
		(u1, cross(w, u1))
	}

	/// The frame before: a reference axis switched from Z to X at |w.z| = 0.9.
	fn old_frame(w: [f32; 3]) -> ([f32; 3], [f32; 3]) {
		let c = cross(w, if w[2].abs() > 0.9 { [1.0, 0.0, 0.0] } else { [0.0, 0.0, 1.0] });
		let u1 = c.map(|x| x / len(c));
		(u1, cross(w, u1))
	}

	fn aim(az: f32, el: f32) -> [f32; 3] {
		let (a, e) = (az.to_radians(), el.to_radians());
		[e.cos() * a.cos(), e.cos() * a.sin(), e.sin()]
	}

	/// The most the frame moves per unit of aim change, between neighbouring aims along `path` (and the frames are orthonormal).
	fn worst_rate(path: &[[f32; 3]], frame: fn([f32; 3]) -> ([f32; 3], [f32; 3])) -> f32 {
		let mut worst = 0.0f32;
		for p in path.windows(2) {
			let ((a1, a2), (b1, b2)) = (frame(p[0]), frame(p[1]));
			for (u, v) in [(a1, a2), (b1, b2)] {
				assert!((len(u) - 1.0).abs() < 1e-3 && (len(v) - 1.0).abs() < 1e-3 && dot(u, v).abs() < 1e-3 && dot(u, p[0]).abs().min(dot(u, p[1]).abs()) < 1e-3);
			}
			worst = worst.max(len(sub(b1, a1)).max(len(sub(b2, a2))) / len(sub(p[1], p[0])));
		}
		worst
	}

	#[test]
	fn the_jet_noise_frame_turns_continuously_with_the_aim() {
		let src = include_str!("firefx_vol.wgsl");
		for line in ["let fa = 1.0 / max(1.0 + W.z, 1.0e-6);", "let U1 = vec3<f32>(1.0 - W.x * W.x * fa, -W.x * W.y * fa, -W.x);", "let U2 = cross(W, U1);"] {
			assert!(src.contains(line), "the shader's frame is the one replicated here: {line}");
		}
		// Up and down every 3 degrees of heading (0.05 degree steps, from 75 degrees down to straight up), and round the compass at the
		// old switch (about 64 degrees up and down), level and near straight up (0.02 degree steps).
		let mut paths: Vec<Vec<[f32; 3]>> = (0..120).map(|a| (0..=3300).map(|k| aim(a as f32 * 3.0, -75.0 + k as f32 * 0.05)).collect()).collect();
		let sw = 0.9f32.asin().to_degrees();
		for el in [-sw - 0.01, -sw + 0.01, 0.0, sw - 0.01, sw + 0.01, 89.0] {
			paths.push((0..=18000).map(|k| aim(k as f32 * 0.02, el)).collect());
		}
		let new = paths.iter().map(|p| worst_rate(p, frame)).fold(0.0f32, f32::max);
		let old = paths.iter().map(|p| worst_rate(p, old_frame)).fold(0.0f32, f32::max);
		assert!(new < 10.0, "the frame moves at most {new} x the aim: no jump");
		assert!(old > 200.0, "the old switch jumped ({old} x the aim): the sweep sees a pop");
	}

	#[test]
	fn the_slabs_tile_the_chain_and_meet_on_shared_planes() {
		let j = jet(80);
		let v = slabs(&j);
		assert!(v.len() >= 2 && v.len() <= MAX_SLABS, "{} slabs", v.len());
		assert!(v.iter().all(|s| s.b[3] == KIND_JET && s.b[0] == v[0].b[0] && s.g == v[0].g), "one kind, one chain end, one noise frame");
		assert!(v[0].e[3] == 0.0 && (v[v.len() - 1].f[3] - v[0].b[0]).abs() < 1e-6, "s runs from the nozzle to the chain end");
		for w in v.windows(2) {
			assert_eq!((&w[0].d, w[0].f), (&w[1].a, w[1].e), "slab k ends where slab k + 1 starts: same node, plane and s");
			assert!(w[1].e[3] > w[0].e[3], "s grows along the chain");
		}
		for s in &v {
			for n in [[s.e[0], s.e[1], s.e[2]], [s.f[0], s.f[1], s.f[2]], [s.g[0], s.g[1], s.g[2]]] {
				assert!((dot(n, n) - 1.0).abs() < 1e-4, "unit normals and direction");
			}
			let axis = sub([s.d[0], s.d[1], s.d[2]], [s.a[0], s.a[1], s.a[2]]);
			assert!(dot(axis, [s.e[0], s.e[1], s.e[2]]) > 0.0 && dot(axis, [s.f[0], s.f[1], s.f[2]]) > 0.0, "the planes face the tip");
			assert!(s.a[3] > 0.0 && s.d[3] > 0.0 && s.g[3] >= 0.2);
		}
	}

	#[test]
	fn a_short_jet_is_one_or_two_slabs_and_a_long_one_is_capped() {
		assert!(matches!(slabs(&jet(12)).len(), 1..=2));
		let long = jet(400);
		assert_eq!(slabs(&long).len(), MAX_SLABS);
	}

	#[test]
	fn the_slabs_slide_when_the_chain_grows_or_its_tail_fades() {
		// One more segment (a new field's stream a little longer) or the last visible node fading out moves only the last slab's end: the
		// cuts are distances along the chain, not node indices (lane nfire: they jumped and the whole flame's pattern with them).
		let (a, b) = (slabs(&jet(60)), slabs(&jet(64)));
		let (full, mut c) = (slabs(&jet(80)), jet(80));
		let last = c.nodes.iter().rposition(|nd| nd.alpha > 0.004).unwrap();
		c.nodes[last].alpha = 0.003;
		let c = slabs(&c);
		for (a, other) in [(&a, &b), (&full, &c)] {
			let n = a.len().min(other.len()) - 1;
			assert!(n >= 2, "{} / {} slabs", a.len(), other.len());
			for k in 0..n {
				let (p, q) = (&a[k], &other[k]);
				let moved = len(sub([p.d[0], p.d[1], p.d[2]], [q.d[0], q.d[1], q.d[2]]));
				assert!(moved < 0.08 && (p.f[3] - q.f[3]).abs() < 0.08, "slab {k} end moved {moved} m");
			}
			assert!((p_send(a) - p_send(other)).abs() <= 0.45, "the chain end moves at most a node step");
		}
	}

	fn p_send(v: &[Vol]) -> f32 {
		v[0].b[0]
	}

	#[test]
	fn a_faded_tail_gets_no_slabs_and_an_invisible_jet_none() {
		let mut j = jet(80);
		let full = slabs(&j);
		let n = j.nodes.len();
		for nd in &mut j.nodes[n / 2..] {
			nd.alpha = 0.0;
		}
		let half = slabs(&j);
		assert!(half.last().unwrap().f[3] < full.last().unwrap().f[3], "the chain ends one node after the last that shows");
		for nd in &mut j.nodes {
			nd.alpha = 0.0;
		}
		assert!(slabs(&j).is_empty());
	}

	#[test]
	fn a_piece_cut_off_from_the_stream_has_no_nozzle_of_its_own() {
		let at_gun = slabs(&jet(30));
		let piece = Jet { d0: 1.5, ..jet(30) };
		let v = slabs(&piece);
		assert_eq!(v.len(), at_gun.len());
		for (a, b) in at_gun.iter().zip(&v) {
			assert!(b.e[3] >= 1.5 && b.f[3] > b.e[3], "every slab starts at least d0 down the stream");
			assert!((b.e[3] - a.e[3] - 1.5).abs() < 1e-4 && (b.f[3] - a.f[3] - 1.5).abs() < 1e-4 && (b.b[0] - a.b[0] - 1.5).abs() < 1e-4);
			assert!((b.b[3] - (KIND_JET + 1.5)).abs() < 1e-6 && a.b[3] == KIND_JET, "the shader reads d0 from the kind row");
		}
		// The stream at the gun: d0 0 within 2.5 m of the hero, then how much farther; none without a hero.
		use super::super::firefx_jet_quads::start_down;
		assert_eq!(start_down([2.0, 0.0, 0.0], Some([0.0; 3])), 0.0);
		assert!((start_down([4.0, 0.0, 0.0], Some([0.0; 3])) - 1.5).abs() < 1e-6);
		assert_eq!(start_down([40.0, 0.0, 0.0], None), 0.0);
	}
}
