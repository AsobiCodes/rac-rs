//! [PC] Fire: a uniform grid of points for the fire's neighbour searches (lane nfire, review R25 m5: the burn puffs' clusters, the
//! ground tongues' twins and their follow from the last frame compared every pair, up to 2048 x 2048 per display frame). A point within
//! `cell` of another lies in one of the 27 cells around it, so `near` hands out only those candidates; the callers still test the
//! exact distance and pick by index on ties, so the result is the all-pairs one.

use std::collections::HashMap;

/// The points (by index) in cells `cell` m across.
pub(super) struct Grid {
	cell: f32,
	map: HashMap<[i32; 3], Vec<usize>>,
}

impl Grid {
	/// An empty grid whose cells are `cell` m across (at least 1 cm).
	pub(super) fn new(cell: f32) -> Grid {
		Grid { cell: cell.max(0.01), map: HashMap::new() }
	}

	fn key(&self, p: [f32; 3]) -> [i32; 3] {
		std::array::from_fn(|k| (p[k] / self.cell).floor() as i32)
	}

	/// A point with a non-finite coordinate is not stored (it can have no neighbour).
	pub(super) fn insert(&mut self, p: [f32; 3], i: usize) {
		if !p.iter().all(|c| c.is_finite()) {
			return;
		}
		let k = self.key(p);
		self.map.entry(k).or_default().push(i);
	}

	/// Every point that may lie within `cell` of `p` (the 27 cells around it; also some farther ones), in no set order.
	pub(super) fn near(&self, p: [f32; 3]) -> impl Iterator<Item = usize> + '_ {
		let finite = p.iter().all(|c| c.is_finite());
		let k = self.key(p);
		(0..if finite { 27 } else { 0 })
			// Checked: a huge position's key sits at the i32 end; the cell beyond it does not exist (no overflow, no repeated cell).
			.filter_map(move |n| Some([k[0].checked_add(n % 3 - 1)?, k[1].checked_add(n / 3 % 3 - 1)?, k[2].checked_add(n / 9 - 1)?]))
			.flat_map(move |at| self.map.get(&at).into_iter().flatten().copied())
	}
}

/// The squared distance of two points.
pub(super) fn dist2(a: [f32; 3], b: [f32; 3]) -> f32 {
	(a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_grid_finds_every_point_within_a_cell_and_skips_far_ones() {
		let pts: Vec<[f32; 3]> = (0..400).map(|i| [(i % 20) as f32 * 0.37 - 3.0, (i / 20) as f32 * 0.41 - 4.0, ((i * 7) % 5) as f32 * 0.3]).collect();
		let mut g = Grid::new(0.8);
		for (i, &p) in pts.iter().enumerate() {
			g.insert(p, i);
		}
		for &p in &pts {
			let mut got: Vec<usize> = g.near(p).filter(|&j| dist2(pts[j], p) < 0.64).collect();
			got.sort();
			let want: Vec<usize> = (0..pts.len()).filter(|&j| dist2(pts[j], p) < 0.64).collect();
			assert_eq!(got, want);
		}
		assert!(g.near([100.0, 0.0, 0.0]).next().is_none());
	}

	#[test]
	fn non_finite_and_huge_positions_are_safe() {
		let mut g = Grid::new(0.8);
		g.insert([f32::NAN, 0.0, 0.0], 0);
		g.insert([f32::INFINITY, 0.0, 0.0], 1);
		g.insert([1e30, -1e30, 3e38], 2);
		g.insert([0.0, 0.0, 0.0], 3);
		assert!(g.near([f32::NAN, 0.0, 0.0]).next().is_none());
		assert_eq!(g.near([1e30, -1e30, 3e38]).collect::<Vec<_>>(), vec![2]);
		assert_eq!(g.near([0.0, 0.0, 0.0]).collect::<Vec<_>>(), vec![3]);
	}
}
