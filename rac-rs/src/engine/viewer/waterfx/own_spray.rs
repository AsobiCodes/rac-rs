//! Where the hero and the movers have been in the water lately: the spray they threw up is theirs, never churn.

use std::collections::VecDeque;

/// How long (s) a point of a mover's path keeps its spray out of the churn: the longest a splash particle lives.
const OWN_LIFE: f32 = 4.0;
/// The spacing (s) of a path's points, and how many points are kept at most.
const OWN_GAP: f32 = 0.2;
const OWN_MAX: usize = 160;
/// Particles this close (world units) to a path point are its mover's spray: they spawn at the mover's feet and drift or
/// spread a little before they fade.
const OWN_RADIUS: f32 = 3.2;

/// The recent places (x, y, time) of every mover in the water, newest first.
#[derive(Default)]
pub(super) struct OwnPath {
	pts: VecDeque<([f32; 2], f32)>,
}

impl OwnPath {
	/// A mover is in the water at `p` at time `now` (s). A mover that stands still adds a point only once its last one grows old.
	pub(super) fn add(&mut self, p: [f32; 3], now: f32) {
		let near_new = self.pts.iter().take(8).any(|q| now - q.1 < OWN_GAP && now >= q.1 && (q.0[0] - p[0]).powi(2) + (q.0[1] - p[1]).powi(2) < 0.04);
		if !near_new {
			self.pts.push_front(([p[0], p[1]], now));
		}
	}

	/// Drops the points older than `OWN_LIFE` (and those from a clock that went back), and keeps at most `OWN_MAX`.
	pub(super) fn prune(&mut self, now: f32) {
		while self.pts.back().is_some_and(|q| now - q.1 > OWN_LIFE || now < q.1 - 1.0) {
			self.pts.pop_back();
		}
		self.pts.truncate(OWN_MAX);
	}

	/// Whether a particle at `p` is a mover's own spray.
	pub(super) fn owns(&self, p: [f32; 3]) -> bool {
		let r2 = OWN_RADIUS * OWN_RADIUS;
		self.pts.iter().any(|q| (q.0[0] - p[0]).powi(2) + (q.0[1] - p[1]).powi(2) < r2)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn spray_left_behind_a_moving_mover_stays_its_own() {
		let mut o = OwnPath::default();
		for k in 0..10 {
			o.add([k as f32, 0.0, 0.0], k as f32 * 0.25);
		}
		o.prune(2.5);
		assert!(o.owns([0.5, 2.0, 0.0]), "near an early point");
		assert!(o.owns([8.0, -3.0, 0.0]), "near a late point");
		assert!(!o.owns([4.0, 6.0, 0.0]), "a waterfall's far spray is not its");
	}

	#[test]
	fn old_points_are_forgotten_and_a_standing_mover_adds_few() {
		let mut o = OwnPath::default();
		for k in 0..100 {
			o.add([1.0, 1.0, 0.0], k as f32 * 0.02);
		}
		assert!(o.pts.len() <= 11, "{}", o.pts.len());
		o.prune(10.0);
		assert!(!o.owns([1.0, 1.0, 0.0]));
	}
}
