//! [PC] Fire: the Pyrocitor's jets drawn between the last two fields (lane nfire; the user: "the fire is kinda laggy and looks somewhat
//! like a slideshow"). The jets are built from the newest field's pool sprites (firefx_jet.rs `build_with`), while with frame smoothing
//! on the hero and the gun are drawn between the last two fields at the smoothing alpha (smooth.rs `frame`): the flame snapped at the
//! field rate and ran up to a field ahead of the gun. `JetLerp` keeps the jets of the last two fields and draws each node at
//! lerp(previous field, newest field, alpha), the same alpha as the hero. Smoothing off: alpha 1, the newest jets exactly as before.
//! A jet with no jet of the previous field (same seed and a nozzle within reach, else a nozzle within `MATCH` m) is drawn as built; a
//! node beyond the previous jet's last grows out of that last node. A display frame that ran k > 1 fields (below the field rate, a
//! hitch) has no jets of the field before the newest: the hero's placement blends from that field (smooth.rs `begin_tick`), so the jets
//! are drawn at the same time, (k - 1 + alpha) / k of the way from the last shown field's; after more than `GAP` fields, as built.

use super::firefx_jet::{Jet, Node};
use std::sync::atomic::{AtomicU32, Ordering};

/// The frame smoothing's blend factor of this display frame (smooth.rs `frame`; 1.0 = smoothing off: the newest field).
static ALPHA: AtomicU32 = AtomicU32::new(0x3f80_0000);

/// A previous field's jet whose nozzle stood within this many metres is the same stream (when the seed does not match).
const MATCH: f32 = 1.5;

/// More fields than this between two shown fields: the jets are drawn as built (the last shown ones are too old to blend from).
const GAP: u32 = 4;

/// Called by the frame smoothing once a display frame (smooth.rs `frame`): the alpha the hero is drawn at, 1.0 while smoothing is off.
pub(crate) fn set_alpha(alpha: f32) {
	ALPHA.store(alpha.clamp(0.0, 1.0).to_bits(), Ordering::Relaxed);
}

pub(super) fn alpha() -> f32 {
	f32::from_bits(ALPHA.load(Ordering::Relaxed))
}

/// The jets of the last two fields (the pool's field counter tells a new field).
#[derive(Default)]
pub(super) struct JetLerp {
	prev: Vec<Jet>,
	cur: Vec<Jet>,
	frame: Option<u32>,
	/// Fields from the last shown field's (`prev`) to the newest (1 = the field before it).
	gap: u32,
}

fn mix(a: f32, b: f32, t: f32) -> f32 {
	a + (b - a) * t
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
	std::array::from_fn(|k| mix(a[k], b[k], t))
}

fn near(a: [f32; 3], b: [f32; 3]) -> f32 {
	((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

fn mix_node(o: &Node, n: &Node, t: f32) -> Node {
	Node { pos: mix3(o.pos, n.pos, t), half: mix(o.half, n.half, t), alpha: mix(o.alpha, n.alpha, t), tint: mix3(o.tint, n.tint, t), d: mix(o.d, n.d, t), s: mix(o.s, n.s, t) }
}

/// Jet `c` of the newest field drawn at `t` from its previous field's `p` (node by node; c's extra nodes start at p's last; p's extra
/// nodes shrink into c's last and fade out, so at `t` 0 the drawn jet is p whole: no tail cut off at the field's start).
fn lerp_jet(p: &Jet, c: &Jet, t: f32) -> Jet {
	let Some(last) = c.nodes.last() else {
		return c.clone();
	};
	let gone = Node { alpha: 0.0, ..*last };
	let nodes = (0..c.nodes.len().max(p.nodes.len()))
		.map(|i| match (p.nodes.get(i).or(p.nodes.last()), c.nodes.get(i)) {
			(Some(o), Some(n)) => mix_node(o, n, t),
			(Some(o), None) => mix_node(o, &gone, t),
			(None, n) => *n.unwrap_or(last),
		})
		.collect();
	Jet { nodes, len: mix(p.len, c.len, t), step: mix(p.step, c.step, t), d0: mix(p.d0, c.d0, t), ..c.clone() }
}

impl JetLerp {
	/// Forgets both fields (a swapped world, fire off).
	pub(super) fn forget(&mut self) {
		self.prev.clear();
		self.cur.clear();
		self.frame = None;
		self.gap = 1;
	}

	/// This display frame's jets: `jets` = built from the pool of field `frame`, drawn at `alpha` between the previous field's and them
	/// (the hero's time: from the field before the newest, see the module doc).
	pub(super) fn blend(&mut self, jets: Vec<Jet>, frame: u32, alpha: f32) -> Vec<Jet> {
		if self.frame != Some(frame) {
			let gap = self.frame.map_or(0, |f| frame.wrapping_sub(f));
			self.prev = std::mem::take(&mut self.cur);
			if gap == 0 || gap > GAP {
				self.prev.clear();
			}
			self.gap = gap.max(1);
			self.frame = Some(frame);
		}
		self.cur = jets;
		let t = (self.gap as f32 - 1.0 + alpha) / self.gap as f32;
		if t >= 1.0 || self.prev.is_empty() {
			return self.cur.clone();
		}
		self.cur
			.iter()
			.map(|c| {
				let reach = MATCH.max(0.6 * c.len);
				let same = self.prev.iter().find(|p| p.seed == c.seed && c.seed != 0.0 && near(p.root, c.root) < reach);
				let close = || self.prev.iter().filter(|p| near(p.root, c.root) < MATCH).min_by(|a, b| near(a.root, c.root).total_cmp(&near(b.root, c.root)));
				match same.or_else(close) {
					Some(p) => lerp_jet(p, c, t),
					None => c.clone(),
				}
			})
			.collect()
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn jet(x: f32, n: usize, seed: f32) -> Jet {
		let nodes = (0..n).map(|i| Node { pos: [x + i as f32, 0.0, 1.0], half: 0.2 + 0.1 * i as f32, alpha: 1.0, tint: [1.0, 0.5, 0.2], d: i as f32, s: i as f32 * 0.5 }).collect();
		Jet { nodes, len: n as f32, step: 0.4, members: vec![], root: [x, 0.0, 1.0], seed, d0: 0.0 }
	}

	#[test]
	fn a_jet_is_drawn_between_the_last_two_fields() {
		let mut l = JetLerp::default();
		assert_eq!(l.blend(vec![jet(0.0, 3, 0.3)], 1, 0.5)[0].nodes[0].pos, [0.0, 0.0, 1.0], "the first field has nothing to blend from");
		let out = l.blend(vec![jet(1.0, 3, 0.3)], 2, 0.25);
		assert!((out[0].nodes[2].pos[0] - 2.25).abs() < 1e-6 && (out[0].len - 3.0).abs() < 1e-6);
		// Another display frame of the same field blends from the same previous field; alpha 1 is the newest field exactly.
		let out = l.blend(vec![jet(1.0, 3, 0.3)], 2, 0.75);
		assert!((out[0].nodes[0].pos[0] - 0.75).abs() < 1e-6);
		assert_eq!(l.blend(vec![jet(1.0, 3, 0.3)], 2, 1.0)[0].nodes[0].pos, [1.0, 0.0, 1.0]);
		// The next field blends from the field before it.
		let out = l.blend(vec![jet(2.0, 3, 0.3)], 3, 0.5);
		assert!((out[0].nodes[0].pos[0] - 1.5).abs() < 1e-6);
	}

	#[test]
	fn a_shorter_jet_takes_the_old_tail_in_and_fades_it() {
		let mut l = JetLerp::default();
		l.blend(vec![jet(0.0, 5, 0.3)], 1, 1.0);
		let start = l.blend(vec![jet(0.0, 3, 0.3)], 2, 0.0);
		assert_eq!(start[0].nodes, jet(0.0, 5, 0.3).nodes, "at the field's start the previous jet whole");
		let late = l.blend(vec![jet(0.0, 3, 0.3)], 2, 0.999);
		assert!(late[0].nodes.len() == 5 && late[0].nodes[4].alpha < 0.01 && (late[0].nodes[4].pos[0] - 2.0).abs() < 0.01, "shrunk into the last node, faded");
		assert_eq!(l.blend(vec![jet(0.0, 3, 0.3)], 2, 1.0)[0].nodes.len(), 3, "alpha 1: the newest jet as built");
	}

	#[test]
	fn a_longer_jet_grows_out_of_the_last_node_and_a_new_stream_is_drawn_as_built() {
		let mut l = JetLerp::default();
		l.blend(vec![jet(0.0, 2, 0.3)], 1, 1.0);
		let out = l.blend(vec![jet(0.0, 4, 0.3), jet(50.0, 2, 0.9)], 2, 0.5);
		// Node 3 starts from the previous jet's last node (x 1) toward its own x 3.
		assert!((out[0].nodes[3].pos[0] - 2.0).abs() < 1e-6 && out[0].nodes.len() == 4);
		assert_eq!(out[1].nodes[0].pos, [50.0, 0.0, 1.0], "no previous stream near: as built");
		// A stream with a new seed still matches by its nozzle.
		let mut l = JetLerp::default();
		l.blend(vec![jet(0.0, 2, 0.3)], 1, 1.0);
		assert!((l.blend(vec![jet(1.0, 2, 0.7)], 2, 0.5)[0].nodes[0].pos[0] - 0.5).abs() < 1e-6);
		l.forget();
		assert_eq!(l.blend(vec![jet(4.0, 2, 0.7)], 3, 0.5)[0].nodes[0].pos[0], 4.0);
	}

	#[test]
	fn a_frame_that_ran_two_fields_draws_the_jets_at_the_heros_time() {
		let mut l = JetLerp::default();
		l.blend(vec![jet(0.0, 3, 0.3)], 1, 1.0);
		// Field 1 -> field 3: the hero blends from field 2, so alpha 0.5 is field 2.5 = 3/4 of the way from field 1's jet.
		let out = l.blend(vec![jet(1.0, 3, 0.3)], 3, 0.5);
		assert!((out[0].nodes[0].pos[0] - 0.75).abs() < 1e-6, "{:?}", out[0].nodes[0].pos);
		assert_eq!(l.blend(vec![jet(1.0, 3, 0.3)], 3, 1.0)[0].nodes[0].pos, [1.0, 0.0, 1.0]);
		// The next field blends from field 3 again.
		assert!((l.blend(vec![jet(2.0, 3, 0.3)], 4, 0.5)[0].nodes[0].pos[0] - 1.5).abs() < 1e-6);
		// A long gap (or a counter that went back): the newest jets as built.
		assert_eq!(l.blend(vec![jet(9.0, 3, 0.3)], 4 + GAP + 1, 0.5)[0].nodes[0].pos, [9.0, 0.0, 1.0]);
		assert_eq!(l.blend(vec![jet(9.0, 3, 0.3)], 2, 0.5)[0].nodes[0].pos, [9.0, 0.0, 1.0]);
	}

	#[test]
	fn the_same_seed_far_away_is_another_stream() {
		let mut l = JetLerp::default();
		l.blend(vec![jet(0.0, 3, 0.31)], 1, 1.0);
		// A warp while firing: the new stream got the old seed but stands 40 m off.
		assert_eq!(l.blend(vec![jet(40.0, 3, 0.31)], 2, 0.5)[0].nodes[0].pos, [40.0, 0.0, 1.0]);
	}

	#[test]
	fn the_alpha_is_one_until_the_smoothing_sets_it() {
		assert!(alpha() > 0.0 && alpha() <= 1.0);
	}
}
