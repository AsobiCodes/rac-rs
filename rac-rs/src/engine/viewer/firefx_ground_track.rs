//! [PC] Fire: the ground tongues followed from frame to frame (lane nfire, agent 6). A retail flame slot keeps its point and size for
//! its whole life (rac1/fire.rs), so its tongue (firefx_ground.rs) must keep its noise seed, scale and scroll rate for that life too: a
//! tongue standing within `FOLLOW` m of one of the last frame's with the same height (`SAME_H`) takes that one's seed and noise; a new
//! one gets the seed of its point and the noise of its size (firefx_vol.rs `noise_of`). The noise scroll rate is quantised per volume
//! (firefx_vol.wgsl `scroll`), so a rate taken anew from a size that changed a little shifted the pattern by up to a tile.

use super::firefx_grid::{dist2, Grid};
use super::firefx_ground::{seed_at, Tongue, SAME_H};
use super::firefx_vol;

/// A tongue within this many metres of one of the last frame's (and as tall) is the same flame.
const FOLLOW: f32 = 0.3;

#[derive(Clone, Copy, Debug, PartialEq)]
struct Kept {
	foot: [f32; 3],
	h: f32,
	seed: f32,
	noise: [f32; 2],
}

/// The last frame's tongues (foot, height, seed, noise).
#[derive(Default)]
pub(super) struct GroundTrack {
	kept: Vec<Kept>,
	next: Vec<Kept>,
}

impl GroundTrack {
	/// Gives each of this frame's `tongues` its seed and noise (see the module doc) and keeps them for the next frame.
	pub(super) fn follow(&mut self, tongues: &mut [Tongue]) {
		self.next.clear();
		let mut used = vec![false; self.kept.len()];
		// Only the last frame's tongues in nearby grid cells are candidates (review R25 m5); nearest first, the lower index on a tie.
		let mut grid = Grid::new(FOLLOW);
		for (k, o) in self.kept.iter().enumerate() {
			grid.insert(o.foot, k);
		}
		for t in tongues.iter_mut() {
			let (foot, h) = (t.emit.base, t.emit.h);
			let hit = grid
				.near(foot)
				.filter(|&k| !used[k] && dist2(self.kept[k].foot, foot) < FOLLOW * FOLLOW && (self.kept[k].h - h).abs() <= SAME_H * self.kept[k].h.max(h))
				.min_by(|&a, &b| dist2(self.kept[a].foot, foot).total_cmp(&dist2(self.kept[b].foot, foot)).then(a.cmp(&b)));
			match hit {
				Some(k) => {
					used[k] = true;
					(t.emit.seed, t.noise) = (self.kept[k].seed, self.kept[k].noise);
				}
				None => (t.emit.seed, t.noise) = (seed_at(foot), firefx_vol::noise_of(h, t.emit.w * 0.5)),
			}
			self.next.push(Kept { foot, h, seed: t.emit.seed, noise: t.noise });
		}
		std::mem::swap(&mut self.kept, &mut self.next);
	}

	/// Forgets the tongues (a swapped world).
	pub(super) fn forget(&mut self) {
		self.kept.clear();
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::viewer::firefx::Emit;

	fn tongue(x: f32, h: f32) -> Tongue {
		Tongue { emit: Emit { base: [x, 5.0, 10.0], h, w: 2.0, seed: 0.0 }, tint: [1.0; 3], fade: 1.0, members: vec![0], noise: [0.0; 2] }
	}

	#[test]
	fn a_tongue_keeps_its_seed_and_noise_while_it_lives() {
		let mut tr = GroundTrack::default();
		let mut a = [tongue(0.0, 4.0), tongue(3.0, 2.0)];
		tr.follow(&mut a);
		assert_eq!(a[0].emit.seed, seed_at([0.0, 5.0, 10.0]));
		assert_eq!(a[0].noise, firefx_vol::noise_of(4.0, 1.0));
		assert!(a[0].noise[0] > 0.0 && a[0].noise[1] > 0.0);
		// Next frame: the first moved a little across a seed cell's edge and grew 5 %: same seed and noise; the second re-rolled (a new
		// slot 2 m away): its own.
		let mut b = [tongue(0.24, 4.2), tongue(5.0, 2.0)];
		tr.follow(&mut b);
		assert_eq!((b[0].emit.seed, b[0].noise), (a[0].emit.seed, a[0].noise));
		assert_eq!(b[1].emit.seed, seed_at([5.0, 5.0, 10.0]));
		// A different height at the same point is a new slot.
		let mut c = [tongue(0.24, 6.0)];
		tr.follow(&mut c);
		assert_eq!(c[0].noise, firefx_vol::noise_of(6.0, 1.0));
		// Two tongues never take the same kept one.
		let mut d = [tongue(0.25, 6.0), tongue(0.2, 6.0)];
		tr.follow(&mut d);
		assert!(d[0].emit.seed == c[0].emit.seed || d[1].emit.seed == c[0].emit.seed);
		tr.forget();
		let mut e = [tongue(0.24, 6.0)];
		tr.follow(&mut e);
		assert_eq!(e[0].emit.seed, seed_at([0.24, 5.0, 10.0]));
	}
}
