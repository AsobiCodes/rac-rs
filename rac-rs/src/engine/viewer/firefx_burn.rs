//! [PC] Fire: a burning enemy as ONE flame (lane nfire, agent 6; the user: "the enemies on fire look like they have like a 100 small
//! fire sprites"). Retail draws a burning moby as many small burn puffs (pool sprites of the `Burn` kind, firefx.rs `add_sprite`); each
//! became its own little flame volume. Now the puffs of one frame are gathered (`Burn::add`) and joined by single linkage (puffs within
//! `max(LINK, 1.2 x size)`) into clusters; each cluster is one flame (`Burn::flush`): standing under the cluster's alpha-weighted middle,
//! as tall as its spread plus 1.6 x the mean puff size, as wide as its spread (at least 0.6 x the mean size), with the brightest puff's
//! fade and the mean tint. It keeps its seed and noise from the last frame's cluster within `FOLLOW` m, and its height, width and foot
//! follow the puffs with a short lag (`TAU` s of game time) so a puff born or dying does not jump the flame. A flame over the volume cap
//! is drawn as a tongue card (`flush`'s `over`), as a burn puff over the cap was before.

use super::firefx::{tune, Emit};
use super::firefx_grid::{dist2, Grid};
use super::firefx_vol::{self, Vol};

/// Puffs closer than this (m, or 1.2 x the bigger puff's size) burn as one flame.
const LINK: f32 = 0.8;
/// A cluster within this many metres of one of the last frame's is the same flame.
const FOLLOW: f32 = 1.5;
/// The lag (s of game time) of a flame's height, width and foot behind its puffs.
const TAU: f64 = 0.15;
/// A puff fainter than this does not set the flame's foot.
const FOOT_ALPHA: f32 = 0.15;

/// One burn puff of the frame: position, size, colour after the near / far fade, pool slot.
#[derive(Clone, Copy, Debug)]
pub(super) struct Puff {
	pub pos: [f32; 3],
	pub size: f32,
	pub colour: [f32; 4],
	pub slot: u16,
}

/// One flame of a cluster of puffs (and the last frame's, followed).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Flame {
	/// The alpha-weighted middle of the puffs (x, y, z).
	pub mid: [f32; 3],
	/// Foot depth below `mid`, height, foot radius (smoothed).
	pub drop: f32,
	pub h: f32,
	pub r: f32,
	pub fade: f32,
	pub tint: [f32; 3],
	pub seed: f32,
	pub noise: [f32; 2],
}

impl Flame {
	pub(super) fn base(&self) -> [f32; 3] {
		[self.mid[0], self.mid[1], self.mid[2] - self.drop]
	}
}

/// The burn puffs of the frame and the flames of the last one.
#[derive(Default)]
pub(super) struct Burn {
	puffs: Vec<Puff>,
	kept: Vec<Flame>,
	next: Vec<Flame>,
	at: Option<f64>,
}

/// The noise of every burn flame (firefx_vol.rs `noise_of`): one tile size, so a flame growing or shrinking keeps its pattern.
fn noise() -> [f32; 2] {
	firefx_vol::noise_of(2.0, tune("fire_burn_s", 0.6) / 1.3)
}

/// Single linkage: the root of each puff's cluster (its lowest puff index). Only the puffs of nearby grid cells are compared (review R25
/// m5: all pairs were up to 2M per frame); the roots do not depend on the order the pairs are joined in.
pub(super) fn clusters(p: &[Puff]) -> Vec<usize> {
	let mut of: Vec<usize> = (0..p.len()).collect();
	fn root(of: &mut [usize], mut i: usize) -> usize {
		while of[i] != i {
			of[i] = of[of[i]];
			i = of[i];
		}
		i
	}
	let reach = LINK.max(1.2 * p.iter().map(|q| q.size).fold(0.0f32, f32::max));
	let mut grid = Grid::new(reach);
	for (i, q) in p.iter().enumerate() {
		grid.insert(q.pos, i);
	}
	for i in 0..p.len() {
		for j in grid.near(p[i].pos).filter(|&j| j > i) {
			if dist2(p[i].pos, p[j].pos) < LINK.max(1.2 * p[i].size.max(p[j].size)).powi(2) {
				let (a, b) = (root(&mut of, i), root(&mut of, j));
				of[a.max(b)] = a.min(b);
			}
		}
	}
	(0..p.len()).map(|i| root(&mut of, i)).collect()
}

/// The flame of the puffs `m` (unsmoothed, seed from its lowest slot).
fn flame_of(p: &[Puff], m: &[usize]) -> Flame {
	let wsum: f32 = m.iter().map(|&i| p[i].colour[3].max(1e-3)).sum();
	let wmean = |f: &dyn Fn(&Puff) -> f32| m.iter().map(|&i| f(&p[i]) * p[i].colour[3].max(1e-3)).sum::<f32>() / wsum;
	let mid = [wmean(&|q| q.pos[0]), wmean(&|q| q.pos[1]), wmean(&|q| q.pos[2])];
	let size = wmean(&|q| q.size);
	let lit: Vec<usize> = m.iter().copied().filter(|&i| p[i].colour[3] >= FOOT_ALPHA).collect();
	let foot = if lit.is_empty() { m } else { &lit[..] };
	let zlo = foot.iter().map(|&i| p[i].pos[2]).fold(f32::INFINITY, f32::min);
	let zhi = foot.iter().map(|&i| p[i].pos[2]).fold(f32::NEG_INFINITY, f32::max);
	let spread = foot.iter().map(|&i| ((p[i].pos[0] - mid[0]).powi(2) + (p[i].pos[1] - mid[1]).powi(2)).sqrt()).fold(0.0f32, f32::max);
	let slot = m.iter().map(|&i| p[i].slot).min().unwrap_or(0);
	Flame {
		mid,
		drop: (mid[2] - zlo).max(0.0) + 0.3 * size,
		h: (zhi - zlo) + 1.6 * size,
		r: (0.6 * size).max(spread),
		fade: m.iter().map(|&i| p[i].colour[3]).fold(0.0, f32::max).clamp(0.0, 1.0),
		tint: [wmean(&|q| q.colour[0]), wmean(&|q| q.colour[1]), wmean(&|q| q.colour[2])],
		seed: (slot as u32 % 4096) as f32 / 4096.0,
		noise: noise(),
	}
}

impl Burn {
	/// A burn puff of this frame (`FireFx::add_sprite`).
	pub(super) fn add(&mut self, p: Puff) {
		self.puffs.push(p);
	}

	/// Forgets the last frame's flames (a swapped world).
	pub(super) fn forget(&mut self) {
		self.kept.clear();
		self.at = None;
	}

	/// The frame's flames: one per cluster of this frame's puffs, followed from the last frame's (`now` = the fire's game time, s),
	/// into `out` while it has `room`, the rest to `over` (a card); their ember emitters into `big` (under `emit_room`). Clears the puffs.
	pub(super) fn flush(&mut self, now: f64, room: usize, out: &mut Vec<Vol>, big: &mut Vec<Emit>, emit_room: usize, over: &mut dyn FnMut(&Flame)) {
		let k = match self.at {
			Some(t) => (1.0 - (-(now - t).max(0.0) / TAU).exp()) as f32,
			None => 1.0,
		};
		self.at = Some(now);
		let of = clusters(&self.puffs);
		// The groups in order of their first puff (a root is its cluster's lowest index, so it comes first).
		let mut groups: Vec<Vec<usize>> = Vec::new();
		let mut group_of = vec![usize::MAX; of.len()];
		for (i, &r) in of.iter().enumerate() {
			if group_of[r] == usize::MAX {
				group_of[r] = groups.len();
				groups.push(Vec::new());
			}
			groups[group_of[r]].push(i);
		}
		self.next.clear();
		let mut used = vec![false; self.kept.len()];
		for g in &groups {
			let mut f = flame_of(&self.puffs, g);
			let hit = (0..self.kept.len()).filter(|&j| !used[j] && dist2(self.kept[j].mid, f.mid) < FOLLOW * FOLLOW).min_by(|&a, &b| dist2(self.kept[a].mid, f.mid).total_cmp(&dist2(self.kept[b].mid, f.mid)));
			if let Some(j) = hit {
				used[j] = true;
				let o = self.kept[j];
				let lag = |a: f32, b: f32| a + (b - a) * k;
				(f.seed, f.noise, f.drop, f.h, f.r) = (o.seed, o.noise, lag(o.drop, f.drop), lag(o.h, f.h), lag(o.r, f.r));
			}
			self.next.push(f);
		}
		std::mem::swap(&mut self.kept, &mut self.next);
		self.puffs.clear();
		if self.kept.is_empty() {
			return;
		}
		for (n, f) in self.kept.iter().filter(|f| f.fade > 0.004).enumerate() {
			match n < room {
				true => out.push(firefx_vol::flame_own(f.base(), f.h, f.r, f.seed, f.fade, f.tint, f.noise)),
				false => over(f),
			}
		}
		let ember = tune("fire_ember_min", 1.8);
		let n = big.len();
		big.extend(self.kept.iter().filter(|f| f.h >= ember && f.fade > 0.3).map(|f| Emit { base: f.base(), h: f.h, w: 2.0 * f.r, seed: f.seed }).take(emit_room.saturating_sub(n)));
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn puff(x: f32, z: f32, slot: u16) -> Puff {
		Puff { pos: [x, 0.0, z], size: 0.5, colour: [1.0, 0.5, 0.2, 0.8], slot }
	}

	#[test]
	fn the_puffs_of_one_burning_enemy_are_one_flame() {
		let mut b = Burn::default();
		// An enemy: 30 puffs over a 1 x 1.5 m body; a second enemy 10 m away: 5 puffs.
		for i in 0..30 {
			b.add(puff((i % 5) as f32 * 0.25, 1.0 + (i / 5) as f32 * 0.3, i as u16 + 10));
		}
		for i in 0..5 {
			b.add(puff(10.0 + i as f32 * 0.2, 1.0, i as u16 + 100));
		}
		let (mut out, mut big) = (Vec::new(), Vec::new());
		b.flush(1.0, 64, &mut out, &mut big, 16, &mut |_| {});
		assert_eq!(out.len(), 2, "one flame per enemy, not one per puff");
		let f = b.kept[0];
		assert!((f.mid[0] - 0.5).abs() < 1e-4, "under the puffs' middle");
		assert!((f.base()[2] - (1.0 - 0.15)).abs() < 1e-4, "foot under the lowest puff");
		assert!((f.h - (1.5 + 0.8)).abs() < 1e-4 && f.r >= 0.5 - 1e-4);
		assert_eq!(f.seed, 10.0 / 4096.0, "the seed of its lowest slot");
		assert_eq!(out[0].d[0], noise()[0], "the burn noise");
		assert!(b.puffs.is_empty());
		assert_eq!(big.len(), 1, "embers only from the tall flame");
	}

	#[test]
	fn the_grid_clusters_are_the_all_pairs_ones() {
		// 600 puffs of mixed sizes scattered over 40 x 40 m: every pair within reach joined, nothing else (many clusters).
		let p: Vec<Puff> = (0..600u32)
			.map(|i| {
				let h = i.wrapping_mul(2654435761);
				let f = |s: u32| ((h >> s) & 0x3ff) as f32 / 1024.0;
				Puff { pos: [40.0 * f(0), 40.0 * f(10), 2.0 * f(20)], size: 0.2 + 0.8 * f(5), colour: [1.0; 4], slot: i as u16 }
			})
			.collect();
		let mut of: Vec<usize> = (0..p.len()).collect();
		fn root(of: &[usize], mut i: usize) -> usize {
			while of[i] != i {
				i = of[i];
			}
			i
		}
		for i in 0..p.len() {
			for j in i + 1..p.len() {
				if super::super::firefx::dist(p[i].pos, p[j].pos) < LINK.max(1.2 * p[i].size.max(p[j].size)) {
					let (a, b) = (root(&of, i), root(&of, j));
					of[a.max(b)] = a.min(b);
				}
			}
		}
		let want: Vec<usize> = (0..p.len()).map(|i| root(&of, i)).collect();
		assert_eq!(clusters(&p), want);
	}

	#[test]
	fn a_flame_follows_its_puffs_without_jumps() {
		let mut b = Burn::default();
		let (mut out, mut big) = (Vec::new(), Vec::new());
		for i in 0..4 {
			b.add(puff(i as f32 * 0.3, 1.0, i + 1));
		}
		b.flush(1.0, 64, &mut out, &mut big, 16, &mut |_| {});
		let f0 = b.kept[0];
		// Next frame (20 ms later): the lowest slot died and a new puff appeared 0.6 m higher: same seed, the height moves only part way.
		for i in 1..4 {
			b.add(puff(i as f32 * 0.3, 1.0, i + 1));
		}
		b.add(puff(0.45, 1.6, 50));
		out.clear();
		b.flush(1.02, 64, &mut out, &mut big, 16, &mut |_| {});
		let f1 = b.kept[0];
		assert_eq!((f1.seed, f1.noise), (f0.seed, f0.noise));
		let full = 0.6 + 1.6 * 0.5;
		assert!(f1.h > f0.h && f1.h < f0.h + 0.2 * (full - f0.h), "the height lags: {} -> {}", f0.h, f1.h);
		// Paused game time: nothing moves.
		for i in 1..4 {
			b.add(puff(i as f32 * 0.3, 1.0, i + 1));
		}
		b.add(puff(0.45, 1.6, 50));
		b.flush(1.02, 64, &mut out, &mut big, 16, &mut |_| {});
		assert_eq!(b.kept[0].h, f1.h);
		// Over the room no volume is pushed: the flame goes to `over` (a card); forget starts anew.
		out.clear();
		b.add(puff(0.0, 1.0, 7));
		let mut cards = 0;
		b.flush(2.0, 0, &mut out, &mut big, 16, &mut |_| cards += 1);
		assert!(out.is_empty() && cards == 1);
		b.forget();
		b.add(puff(0.0, 1.0, 7));
		b.flush(3.0, 4, &mut out, &mut big, 16, &mut |_| {});
		assert_eq!(b.kept[0].seed, 7.0 / 4096.0);
	}
}
