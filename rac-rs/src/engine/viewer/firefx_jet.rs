//! [PC] Fire, the Pyrocitor's stream as ONE flame jet (firefx.rs; Fire Low / Medium / High; stage 1 of the v0.5 fire extras).
//!
//! The retail stream is 3 to 4 new type-12 pool sprites a tick (decomp/notes/weapons_framework.md "Aim, obstruction and effect
//! generation"), each a fireball that flies out along the aim, so at any time a hundred or so overlapping fireballs lie along a line.
//! Drawn one by one they read as separate balls. Here the segments of one stream are joined into a chain of nodes, in stream order,
//! and drawn as a chain of overlapping flame puffs (firefx_jet_quads.rs) whose heat runs from a white-yellow core at the nozzle to
//! dark red tips (firefx.wgsl `jet_at`).
//!
//! Stability (frame smoothing is on, the pool changes every tick): the order is NOT the pool's slot order (a freed slot is reused by
//! the next particle) but the distance from the nozzle end, and a node is the tent-weighted mean of the segments around its distance
//! (weight 1 at the node, 0 one step away), so a segment crossing from one node to the next moves both nodes a little instead of
//! popping one; a node with little weight fades out instead of vanishing. The nozzle end is the end of the stream nearest the hero
//! when that is close, else the end whose segments are the freshest (brightest, the colour fades with age).
//! Streams are single-linkage groups of segments (`fire_jet_link` metres), at most `MAX_SEGS` segments and `MAX_JETS` streams.

use super::firefx::{dist, tune};

/// Segments looked at a frame (the rest of the type's sprites draw as lone fireballs) and streams built a frame.
pub(super) const MAX_SEGS: usize = 256;
pub(super) const MAX_JETS: usize = 4;
/// Fewer segments than this (or a stream shorter than `MIN_LEN`) is no jet: the segments draw as lone fireballs.
pub(super) const MIN_SEGS: usize = 4;
const MIN_LEN: f32 = 0.6;
/// A node with less weight than this has no segment of its own: its place is interpolated and it is clear.
const EMPTY: f32 = 0.05;

/// One stream segment as drawn: world centre, side of its quad after the pool's size scaling, colour after the near / far fade.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Seg {
	pub pos: [f32; 3],
	pub size: f32,
	pub colour: [f32; 4],
}

/// One node of a jet's chain: place, half width (m), opacity, tint, distance from the nozzle (m) and the same as 0 (nozzle) .. 1 (tip).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Node {
	pub pos: [f32; 3],
	pub half: f32,
	pub alpha: f32,
	pub tint: [f32; 3],
	pub d: f32,
	pub s: f32,
}

/// A joined stream: its nodes, the length the heat gradient runs over (m), the node spacing (m) and the segments it was built from.
#[derive(Clone, Debug)]
pub(super) struct Jet {
	pub nodes: Vec<Node>,
	pub len: f32,
	pub step: f32,
	pub members: Vec<usize>,
	/// The nozzle point the stream was built from (the next frame's `nozzle_from` keeps the end it stands at).
	pub root: [f32; 3],
	/// The noise seed of its pattern: 0 from `build`, chosen by `JetState::plan` (firefx_jet_quads.rs `seeds`: the stream keeps it from
	/// frame to frame with its nozzle, review R7-8).
	pub seed: f32,
	/// Metres of stream before its first node (`JetState::plan`): 0 for the stream at the gun; a piece cut off from it (a strafe
	/// reversal splits the stream) starts this far down the stream, so it has no white-hot nozzle core of its own (lane nfire, agent 6:
	/// "the source of flame jumps to different pieces").
	pub d0: f32,
}

/// What a quality level spends on a jet: node spacing, the most nodes, the ember emitters along it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Quality {
	pub step: f32,
	pub nodes: usize,
	pub emitters: usize,
}

/// Low: coarse chain (12 puffs), no embers. Medium: 20 puffs, 3 ember emitters. High: 28 puffs, 3 ember emitters (and the warm light).
pub(super) fn quality(level: u8) -> Quality {
	let (step, nodes, emitters) = match level {
		0 | 1 => (0.9, 12, 0),
		2 => (0.55, 20, 3),
		_ => (0.4, 28, 3),
	};
	Quality { step: step * tune("fire_jet_step", 1.0).max(0.2), nodes, emitters }
}

fn d2(a: [f32; 3], b: [f32; 3]) -> f32 {
	(a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

fn find(p: &mut [usize], mut i: usize) -> usize {
	while p[i] != i {
		p[i] = p[p[i]];
		i = p[i];
	}
	i
}

/// The streams: groups of segment indices (single linkage, at most `MAX_SEGS` segments looked at), each in index order, the groups in
/// order of their lowest index. The link between two segments is `link` metres or 0.6 x the bigger quad, whichever is more.
pub(super) fn groups(segs: &[Seg], link: f32) -> Vec<Vec<usize>> {
	let n = segs.len().min(MAX_SEGS);
	let mut parent: Vec<usize> = (0..n).collect();
	for i in 0..n {
		for j in i + 1..n {
			let l = link.max(0.6 * segs[i].size.max(segs[j].size));
			if d2(segs[i].pos, segs[j].pos) <= l * l {
				let (a, b) = (find(&mut parent, i), find(&mut parent, j));
				if a != b {
					parent[a.max(b)] = a.min(b);
				}
			}
		}
	}
	let mut out: Vec<Vec<usize>> = Vec::new();
	let mut slot_of = vec![usize::MAX; n];
	for i in 0..n {
		let r = find(&mut parent, i);
		if slot_of[r] == usize::MAX {
			slot_of[r] = out.len();
			out.push(Vec::new());
		}
		out[slot_of[r]].push(i);
	}
	out
}

/// The nozzle end of the stream `idx`: of the two ends (the farthest segment from the middle, and the farthest from that one) the one
/// nearer to the hero when one is within 7 m of it, else the one whose three nearest segments are brighter (fresher). The point is the
/// mean of the segments within 0.7 m of that end, so it moves smoothly.
pub(super) fn nozzle(segs: &[Seg], idx: &[usize], hero: Option<[f32; 3]>) -> [f32; 3] {
	nozzle_from(segs, idx, hero, &[])
}

/// How much nearer to the hero the other end must be before the nozzle moves to it, and how much fresher (alpha) it must be when no hero
/// is near: two ends that are about equal must not swap from one frame to the next (the heat gradient would flip along the stream).
const HERO_HYSTERESIS: f32 = 0.75;
const FRESH_HYSTERESIS: f32 = 0.1;

/// `nozzle` with a memory: `prev` = the nozzle points of the last frame's jets. When one lies within max(2 m, 0.6 x the stream length) of an
/// end, that end stays the nozzle unless the other end is clearly better (nearer to the hero by `HERO_HYSTERESIS`, or fresher by
/// `FRESH_HYSTERESIS`); with no such memory the plain rule decides.
pub(super) fn nozzle_from(segs: &[Seg], idx: &[usize], hero: Option<[f32; 3]>, prev: &[[f32; 3]]) -> [f32; 3] {
	let p = |i: usize| segs[idx[i]].pos;
	let n = idx.len();
	let mid = (0..n).fold([0.0f32; 3], |m, i| std::array::from_fn(|k| m[k] + p(i)[k] / n as f32));
	let far = |from: [f32; 3]| (0..n).fold(0, |best, i| if d2(p(i), from) > d2(p(best), from) { i } else { best });
	let a = far(mid);
	let b = far(p(a));
	let fresh = |e: usize| {
		let mut near: Vec<(f32, f32)> = (0..n).map(|i| (d2(p(i), p(e)), segs[idx[i]].colour[3])).collect();
		near.sort_by(|x, y| x.0.total_cmp(&y.0));
		near.iter().take(3).map(|x| x.1).sum::<f32>() / near.len().clamp(1, 3) as f32
	};
	// The end last frame's nozzle stood at (the nearest remembered point to either end, if it is close enough to be this stream's).
	let reach = (2.0f32).max(0.6 * dist(p(a), p(b)));
	let kept = prev.iter().flat_map(|&r| [(dist(p(a), r), a, b), (dist(p(b), r), b, a)]).filter(|c| c.0 <= reach).min_by(|x, y| x.0.total_cmp(&y.0)).map(|c| (c.1, c.2));
	let root = match (hero, kept) {
		(Some(h), Some((mine, other))) if dist(p(a), h).min(dist(p(b), h)) < 7.0 => {
			if dist(p(mine), h) - dist(p(other), h) > HERO_HYSTERESIS {
				other
			} else {
				mine
			}
		}
		(Some(h), None) if dist(p(a), h).min(dist(p(b), h)) < 7.0 => {
			if dist(p(a), h) <= dist(p(b), h) {
				a
			} else {
				b
			}
		}
		(_, Some((mine, other))) => {
			if fresh(other) > fresh(mine) + FRESH_HYSTERESIS {
				other
			} else {
				mine
			}
		}
		_ if fresh(b) > fresh(a) => b,
		_ => a,
	};
	let near: Vec<usize> = (0..n).filter(|&i| d2(p(i), p(root)) <= 0.49).collect();
	std::array::from_fn(|k| near.iter().map(|&i| p(i)[k]).sum::<f32>() / near.len().max(1) as f32)
}

/// A weighted sum of nodes' values: position, size, alpha, rgb and the weight itself.
#[derive(Clone, Copy, Default)]
struct Acc {
	w: f32,
	pos: [f32; 3],
	size: f32,
	a: f32,
	rgb: [f32; 3],
}

/// Fills the nodes without weight of their own (`EMPTY`) by linear interpolation between the nearest ones that have some.
fn fill_empty(have: &[bool], v: &mut [[f32; 4]]) {
	let ok: Vec<usize> = (0..have.len()).filter(|&i| have[i]).collect();
	for i in 0..v.len() {
		if have[i] || ok.is_empty() {
			continue;
		}
		let lo = ok.iter().rev().find(|&&j| j < i);
		let hi = ok.iter().find(|&&j| j > i);
		v[i] = match (lo, hi) {
			(Some(&l), Some(&h)) => {
				let t = (i - l) as f32 / (h - l) as f32;
				std::array::from_fn(|k| v[l][k] + (v[h][k] - v[l][k]) * t)
			}
			(Some(&j), None) | (None, Some(&j)) => v[j],
			(None, None) => v[i],
		};
	}
}

/// One 1-2-1 smoothing pass (the ends keep 3/4 of themselves).
fn smooth(v: &mut [[f32; 4]]) {
	let src = v.to_vec();
	for i in 0..v.len() {
		let (a, b) = (src[i.saturating_sub(1)], src[(i + 1).min(src.len() - 1)]);
		v[i] = std::array::from_fn(|k| 0.5 * src[i][k] + 0.25 * (a[k] + b[k]));
	}
}

/// Joins the stream `idx` of `segs` into a jet. None = no jet (too few segments, too short): they draw as lone fireballs. `wid`
/// scales the half width (`fire_jet_wid`).
pub(super) fn build(segs: &[Seg], idx: &[usize], hero: Option<[f32; 3]>, q: Quality, wid: f32) -> Option<Jet> {
	build_with(segs, idx, hero, q, wid, &[])
}

/// `build` that remembers: `prev` = the nozzle points of the last frame's jets (`nozzle_from`).
pub(super) fn build_with(segs: &[Seg], idx: &[usize], hero: Option<[f32; 3]>, q: Quality, wid: f32, prev: &[[f32; 3]]) -> Option<Jet> {
	if idx.len() < MIN_SEGS {
		return None;
	}
	let root = nozzle_from(segs, idx, hero, prev);
	let ds: Vec<f32> = idx.iter().map(|&i| dist(segs[i].pos, root)).collect();
	let dmax = ds.iter().copied().fold(0.0, f32::max);
	if dmax < MIN_LEN {
		return None;
	}
	let step = q.step.max(dmax / (q.nodes.max(3) - 1) as f32);
	let k = ((dmax / step).ceil() as usize).min(q.nodes.max(3) - 1) + 1;
	let mut acc = vec![Acc::default(); k];
	for (m, &i) in idx.iter().enumerate() {
		let s = &segs[i];
		// The two nodes whose tent covers this segment.
		let lo = (ds[m] / step).floor() as usize;
		for j in [lo, lo + 1] {
			let Some(a) = acc.get_mut(j) else { continue };
			let w = 1.0 - (ds[m] - j as f32 * step).abs() / step;
			if w <= 0.0 {
				continue;
			}
			a.w += w;
			a.size += w * s.size;
			a.a += w * s.colour[3];
			for c in 0..3 {
				a.pos[c] += w * s.pos[c];
				a.rgb[c] += w * s.colour[c];
			}
		}
	}
	while acc.len() > 3 && acc.last().is_some_and(|a| a.w < EMPTY) {
		acc.pop();
	}
	if acc.len() < 3 {
		return None;
	}
	let have: Vec<bool> = acc.iter().map(|a| a.w >= EMPTY).collect();
	// Per node: x y z half | alpha r g b (two arrays so the fill and the smoothing work on four lanes each).
	let mut geo: Vec<[f32; 4]> = acc.iter().map(|a| if a.w >= EMPTY { [a.pos[0] / a.w, a.pos[1] / a.w, a.pos[2] / a.w, 0.5 * wid * 0.8 * a.size / a.w] } else { [0.0; 4] }).collect();
	let mut col: Vec<[f32; 4]> = acc.iter().map(|a| if a.w >= EMPTY { [a.a / a.w, a.rgb[0] / a.w, a.rgb[1] / a.w, a.rgb[2] / a.w] } else { [0.0; 4] }).collect();
	if !have[0] {
		geo[0] = [root[0], root[1], root[2], 0.0];
	}
	let mut have_geo = have.clone();
	have_geo[0] = true;
	fill_empty(&have_geo, &mut geo);
	fill_empty(&have, &mut col);
	smooth(&mut geo);
	smooth(&mut col);
	let len = dmax.max(2.0);
	let nodes = (0..acc.len())
		.map(|j| {
			// Opacity: the segments' own, thinned where few of them stand (a gap, the tip) so the ends fade instead of ending.
			let cover = ((acc[j].w - 0.15) / 0.85).clamp(0.0, 1.0);
			let cover = cover * cover * (3.0 - 2.0 * cover);
			let d = j as f32 * step;
			Node { pos: [geo[j][0], geo[j][1], geo[j][2]], half: geo[j][3].max(0.05), alpha: if have[j] { col[j][0] * cover } else { 0.0 }, tint: [col[j][1], col[j][2], col[j][3]], d, s: (d / len).min(1.0) }
		})
		.collect();
	Some(Jet { nodes, len, step, members: idx.to_vec(), root, seed: 0.0, d0: 0.0 })
}

#[cfg(test)]
mod tests {
	use super::*;

	const Q: Quality = Quality { step: 0.4, nodes: 28, emitters: 3 };

	/// `n` segments along +x from `x0`, `gap` metres apart, each 1 m wide, orange, alpha `a`.
	fn line(x0: f32, n: usize, gap: f32, a: f32) -> Vec<Seg> {
		(0..n).map(|i| Seg { pos: [x0 + i as f32 * gap, 0.0, 1.0], size: 1.0, colour: [1.0, 0.6, 0.2, a] }).collect()
	}

	fn all(segs: &[Seg]) -> Vec<usize> {
		(0..segs.len()).collect()
	}

	/// Hero standing at the start of a stream along +x.
	const HERO: Option<[f32; 3]> = Some([-1.0, 0.0, 1.0]);

	#[test]
	fn a_straight_stream_joins_in_order_from_the_nozzle() {
		let segs = line(0.0, 40, 0.1, 0.8);
		let jet = build(&segs, &all(&segs), HERO, Q, 1.0).expect("a 4 m stream is a jet");
		let xs: Vec<f32> = jet.nodes.iter().map(|n| n.pos[0]).collect();
		assert!(xs.windows(2).all(|w| w[1] > w[0]), "nodes run from the hero's end outwards: {xs:?}");
		assert!(xs[0] < 0.5 && *xs.last().unwrap() > 3.0, "the whole stream is covered: {xs:?}");
		let s: Vec<f32> = jet.nodes.iter().map(|n| n.s).collect();
		assert_eq!(s[0], 0.0);
		assert!(s.windows(2).all(|w| w[1] >= w[0]) && *s.last().unwrap() > 0.9 && *s.last().unwrap() <= 1.0, "s runs 0 .. 1: {s:?}");
		assert!(jet.nodes.iter().all(|n| n.half > 0.05 && n.alpha > 0.0), "every node of a full stream shows");
		assert_eq!(jet.members.len(), 40);
	}

	#[test]
	fn the_pool_order_of_the_segments_does_not_matter() {
		let segs = line(0.0, 40, 0.1, 0.8);
		let a = build(&segs, &all(&segs), HERO, Q, 1.0).unwrap();
		// A freed slot is reused by the next particle: the list order is anything.
		let mut rev = segs.clone();
		rev.reverse();
		let b = build(&rev, &all(&rev), HERO, Q, 1.0).unwrap();
		assert_eq!(a.nodes.len(), b.nodes.len());
		for (x, y) in a.nodes.iter().zip(&b.nodes) {
			assert!(dist(x.pos, y.pos) < 1e-3 && (x.alpha - y.alpha).abs() < 1e-3 && (x.half - y.half).abs() < 1e-3, "{x:?} vs {y:?}");
		}
	}

	/// No strobe: a segment that moves 5 cm (or crosses from one node's tent to the next) moves no node by more than 10 cm.
	#[test]
	fn a_small_shift_of_one_segment_moves_the_nodes_little() {
		let mut segs = line(0.0, 40, 0.1, 0.8);
		let a = build(&segs, &all(&segs), HERO, Q, 1.0).unwrap();
		for shift in [[0.0, 0.05, 0.0], [0.05, 0.0, 0.0], [0.0, 0.0, 0.05]] {
			let mut moved = segs.clone();
			for k in 0..3 {
				moved[20].pos[k] += shift[k];
			}
			let b = build(&moved, &all(&moved), HERO, Q, 1.0).unwrap();
			assert_eq!(a.nodes.len(), b.nodes.len());
			for (x, y) in a.nodes.iter().zip(&b.nodes) {
				assert!(dist(x.pos, y.pos) < 0.1, "{shift:?}: {x:?} vs {y:?}");
				assert!((x.alpha - y.alpha).abs() < 0.1 && (x.half - y.half).abs() < 0.1);
			}
		}
		// A segment that vanishes (the pool frees it) changes the nodes just as little.
		segs.remove(20);
		let c = build(&segs, &all(&segs), HERO, Q, 1.0).unwrap();
		for (x, y) in a.nodes.iter().zip(&c.nodes) {
			assert!(dist(x.pos, y.pos) < 0.1 && (x.alpha - y.alpha).abs() < 0.2, "{x:?} vs {y:?}");
		}
	}

	#[test]
	fn a_big_gap_makes_two_streams_and_a_small_one_fades_the_nodes_between() {
		let mut segs = line(0.0, 20, 0.1, 0.8);
		segs.extend(line(4.0, 20, 0.1, 0.8));
		let g = groups(&segs, 1.4);
		assert_eq!(g.len(), 2, "a 2 m gap is more than the link");
		assert_eq!((g[0].len(), g[1].len()), (20, 20));
		assert_eq!((g[0][0], g[1][0]), (0, 20), "in order of their lowest index");
		// 1 m gap: one stream; the nodes in the gap have no segment of their own and are clear.
		let mut segs = line(0.0, 20, 0.1, 0.8);
		segs.extend(line(2.9, 20, 0.1, 0.8));
		let g = groups(&segs, 1.4);
		assert_eq!(g.len(), 1);
		let jet = build(&segs, &g[0], HERO, Q, 1.0).unwrap();
		let mid = jet.nodes.iter().filter(|n| n.pos[0] > 2.1 && n.pos[0] < 2.8).map(|n| n.alpha).fold(f32::MAX, f32::min);
		let full = jet.nodes.iter().map(|n| n.alpha).fold(0.0, f32::max);
		assert!(mid < 0.3 * full, "the gap fades: {mid} vs {full}");
		// The nodes there still have a place on the chain (interpolated, not at the origin).
		assert!(jet.nodes.iter().all(|n| n.pos[2] > 0.5 && n.half > 0.04));
	}

	#[test]
	fn a_stream_that_loses_its_far_segments_shrinks() {
		let full = line(0.0, 40, 0.1, 0.8);
		let a = build(&full, &all(&full), HERO, Q, 1.0).unwrap();
		let short = &full[..20];
		let b = build(short, &all(short), HERO, Q, 1.0).unwrap();
		assert!(b.len < a.len && b.nodes.len() < a.nodes.len(), "{} / {} nodes", b.len, b.nodes.len());
		assert!(b.nodes.last().unwrap().pos[0] < 2.4, "the tip comes back to the last segment");
	}

	#[test]
	fn too_few_or_too_short_is_no_jet() {
		let few = line(0.0, 3, 0.5, 0.8);
		assert!(build(&few, &all(&few), HERO, Q, 1.0).is_none(), "fewer than MIN_SEGS");
		let short = line(0.0, 10, 0.03, 0.8);
		assert!(build(&short, &all(&short), HERO, Q, 1.0).is_none(), "shorter than 0.6 m");
	}

	#[test]
	fn the_nozzle_follows_the_hero_else_the_freshest_end() {
		let mut segs = line(0.0, 40, 0.1, 0.8);
		// Hero close to the far end: that end is the nozzle although the other is brighter.
		let far_hero = Some([5.0, 0.0, 1.0]);
		assert!(nozzle(&segs, &all(&segs), far_hero)[0] > 3.0);
		assert!(nozzle(&segs, &all(&segs), HERO)[0] < 1.0);
		// No hero near (over 7 m): the end whose segments are brighter (fresh) is the nozzle.
		for (i, s) in segs.iter_mut().enumerate() {
			s.colour[3] = 1.0 - i as f32 / 40.0;
		}
		let away = Some([0.0, 30.0, 1.0]);
		assert!(nozzle(&segs, &all(&segs), away)[0] < 1.0, "bright at x = 0");
		for (i, s) in segs.iter_mut().enumerate() {
			s.colour[3] = (i + 1) as f32 / 40.0;
		}
		assert!(nozzle(&segs, &all(&segs), None)[0] > 3.0, "bright at x = 4");
	}

	/// Review m8: two ends of equal rank do not swap; a clearly fresher or (hero) clearly nearer other end does, once.
	#[test]
	fn the_nozzle_keeps_its_end_unless_the_other_is_clearly_better() {
		let mut segs = line(0.0, 40, 0.1, 0.8);
		let all = all(&segs);
		let mut prev: Vec<[f32; 3]> = Vec::new();
		let (mut flips, mut side) = (0, None);
		// Ends alternate +-0.02 in freshness for 200 frames, no hero near: the root never flips.
		for f in 0..200 {
			let wob = if f % 2 == 0 { 0.02 } else { -0.02 };
			for (i, s) in segs.iter_mut().enumerate() {
				s.colour[3] = if i < 3 { 0.8 + wob } else if i >= 37 { 0.8 - wob } else { 0.8 };
			}
			let r = nozzle_from(&segs, &all, None, &prev);
			let now = r[0] > 2.0;
			flips += (side.is_some_and(|s| s != now)) as u32;
			side = Some(now);
			prev = vec![r];
		}
		assert_eq!(flips, 0, "equal ends never swap");
		// The end at x = 4 becomes clearly fresher (+0.3): one flip, then it stays.
		let start = side.unwrap();
		for s in segs.iter_mut().skip(37) {
			s.colour[3] = 1.1;
		}
		for f in 0..20 {
			let r = nozzle_from(&segs, &all, None, &prev);
			flips += ((r[0] > 2.0) != side.unwrap()) as u32;
			side = Some(r[0] > 2.0);
			prev = vec![r];
			assert!(f == 0 || (r[0] > 2.0) == side.unwrap());
		}
		assert!(side.unwrap() && (flips == 1 || start), "the clearly fresher end wins: flips {flips}");
		// Hero: crossing the middle by +-0.1 m moves nothing; a hero 1 m nearer to the other end does.
		let segs = line(0.0, 40, 0.1, 0.8);
		let mut prev = vec![nozzle(&segs, &all, Some([2.1, 0.0, 1.0]))];
		let first = prev[0][0] > 2.0;
		for f in 0..100 {
			let x = if f % 2 == 0 { 1.9 } else { 2.1 };
			let r = nozzle_from(&segs, &all, Some([x, 0.0, 1.0]), &prev);
			assert_eq!(r[0] > 2.0, first, "hero at x = {x}");
			prev = vec![r];
		}
		let r = nozzle_from(&segs, &all, Some([if first { 0.5 } else { 3.5 }, 0.0, 1.0]), &prev);
		assert_ne!(r[0] > 2.0, first, "a hero clearly nearer to the other end moves the nozzle");
	}

	#[test]
	fn quality_levels_spend_more_and_respect_their_caps() {
		let (lo, mid, hi) = (quality(1), quality(2), quality(3));
		assert!(lo.step > mid.step && mid.step > hi.step);
		assert_eq!((lo.emitters, mid.emitters, hi.emitters), (0, 3, 3));
		assert_eq!((quality(0), quality(9).nodes), (lo, hi.nodes));
		let long = line(0.0, 200, 0.15, 0.8); // 30 m of stream
		let n: Vec<usize> = [lo, mid, hi].iter().map(|q| build(&long, &all(&long), HERO, *q, 1.0).unwrap().nodes.len()).collect();
		assert!(n[0] <= lo.nodes && n[1] <= mid.nodes && n[2] <= hi.nodes, "{n:?}");
		assert!(n[0] < n[1] && n[1] < n[2], "{n:?}");
		let short = line(0.0, 30, 0.1, 0.8);
		let m: Vec<usize> = [lo, mid, hi].iter().map(|q| build(&short, &all(&short), HERO, *q, 1.0).unwrap().nodes.len()).collect();
		assert!(m[0] < m[2], "a 3 m jet has more nodes at High: {m:?}");
	}

	#[test]
	fn only_the_first_256_segments_are_grouped() {
		let segs = line(0.0, 300, 0.05, 0.8);
		let g = groups(&segs, 1.4);
		assert_eq!(g.iter().map(Vec::len).sum::<usize>(), MAX_SEGS);
		assert!(g.iter().flatten().all(|&i| i < MAX_SEGS));
	}

	#[test]
	fn two_streams_are_two_jets() {
		let mut segs = line(0.0, 30, 0.1, 0.8);
		let other: Vec<Seg> = line(0.0, 30, 0.1, 0.8).into_iter().map(|mut s| { s.pos[1] += 20.0; s }).collect();
		segs.extend(other);
		let g = groups(&segs, 1.4);
		assert_eq!(g.len(), 2);
		let jets: Vec<Jet> = g.iter().filter_map(|idx| build(&segs, idx, HERO, Q, 1.0)).collect();
		assert_eq!(jets.len(), 2);
		assert!(jets[1].nodes.iter().all(|n| n.pos[1] > 19.0), "the second jet lies on its own stream");
	}

	#[test]
	fn the_width_knob_scales_the_puffs() {
		let segs = line(0.0, 40, 0.1, 0.8);
		let a = build(&segs, &all(&segs), HERO, Q, 1.0).unwrap();
		let b = build(&segs, &all(&segs), HERO, Q, 2.0).unwrap();
		assert!((b.nodes[5].half / a.nodes[5].half - 2.0).abs() < 1e-3);
	}
}
