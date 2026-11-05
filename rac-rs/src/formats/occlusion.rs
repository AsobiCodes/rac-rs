//! Precomputed visibility (decomp/notes/novalis_enemies.md REN-CULL-OCC, BRIDGE-30): the level ships a grid of 4 m camera cells,
//! each holding a 1024-bit mask of the ties, terrain fragments and mobies a camera in that cell may see. Per frame the mask of
//! the camera's cell (or its neighbours) is built; every object carries a (mask byte, index byte) pair and is skipped when
//! `byte & mask[index] == 0`.

use crate::bytes::{u16_at, u32_at};
use crate::level::Level;

/// Unmapped objects test bit 1023, which every mask keeps set.
pub const UNMAPPED: (u8, u8) = (0x80, 0x7f);

/// The mapping block (gameplay +0x8c): per tie, terrain fragment and moby the (mask bit, object id) pairs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OccMap {
	pub tfrags: Vec<(u32, u32)>,
	pub ties: Vec<(u32, u32)>,
	pub mobys: Vec<(u32, u32)>,
}

impl OccMap {
	/// None when the gameplay block has no mapping (offset 0) or it is cut short.
	pub fn new(gameplay: &[u8]) -> Option<OccMap> {
		let off = u32_at(gameplay, 0x8c).ok()? as usize;
		if off == 0 {
			return None;
		}
		Self::from_block(gameplay.get(off..)?)
	}

	/// The mapping from its block: three counts (terrain fragments, ties, mobys) at 0, 4 and 8, then the (bit, id) pairs of the three lists from
	/// +0x10. R&C1 keeps the block inside the gameplay data (offset at +0x8c, `new`); R&C2 as the fourth block of the level WAD. None if cut short.
	pub fn from_block(b: &[u8]) -> Option<OccMap> {
		let counts = [u32_at(b, 0).ok()? as usize, u32_at(b, 4).ok()? as usize, u32_at(b, 8).ok()? as usize];
		let mut at = 0x10;
		let mut lists: [Vec<(u32, u32)>; 3] = Default::default();
		for (list, n) in lists.iter_mut().zip(counts) {
			for _ in 0..n {
				list.push((u32_at(b, at).ok()?, u32_at(b, at + 4).ok()?));
				at += 8;
			}
		}
		let [tfrags, ties, mobys] = lists;
		Some(OccMap { tfrags, ties, mobys })
	}
}

/// The (mask byte, index byte) of mapping bit `b`.
pub fn bit_bytes(b: u32) -> (u8, u8) {
	(1 << (b & 7), (b >> 3) as u8)
}

/// Tie bytes by record id: when the pair count equals the tie count and every id matches (as a 16-bit value), pair i belongs to
/// tie i; otherwise each tie takes the first pair with its id, or the unmapped bytes.
pub fn tie_bytes(map: &OccMap, ids: &[u32]) -> Vec<(u8, u8)> {
	let same = |a: u32, b: u32| a as u16 == b as u16;
	if map.ties.len() == ids.len() && ids.iter().zip(&map.ties).all(|(&id, p)| same(id, p.1)) {
		return map.ties.iter().map(|p| bit_bytes(p.0)).collect();
	}
	ids.iter().map(|&id| map.ties.iter().find(|p| same(id, p.1)).map_or(UNMAPPED, |p| bit_bytes(p.0))).collect()
}

/// Terrain fragment bytes by header byte +0x3d: all or nothing, the pairs must match the fragments one for one.
pub fn tfrag_bytes(map: &OccMap, ids: &[u8]) -> Vec<(u8, u8)> {
	if map.tfrags.len() == ids.len() && ids.iter().zip(&map.tfrags).all(|(&id, p)| id as u32 == p.1) {
		return map.tfrags.iter().map(|p| bit_bytes(p.0)).collect();
	}
	vec![UNMAPPED; ids.len()]
}

/// The moby form of `UNMAPPED` (byte +0x36 = 0x80, +0x37 = 0x7f).
pub const MOBY_UNMAPPED: u16 = 0x7f80;

/// A moby's occlusion u16 (mask byte low, index byte high): the first moby pair whose id equals the record UID as an `i16`,
/// else unmapped.
pub fn moby_occ(map: &OccMap, uid: u32) -> u16 {
	let uid = uid as u16 as i16 as i32;
	map.mobys.iter().find(|p| p.1 as i32 == uid).map_or(MOBY_UNMAPPED, |p| {
		let (m, i) = bit_bytes(p.0);
		(i as u16) << 8 | m as u16
	})
}

/// The moby draw test: `occ` (see `moby_occ`) against the frame's mask.
pub fn occ_pass(mask: &[u8; 128], occ: u16) -> bool {
	occ as u8 & mask[(occ >> 8) as usize & 0x7f] != 0
}

/// The OR of the 128-byte masks of every cell the grid indexes (z rows, y columns, x cells; 0xffff = empty).
fn grid_union(d: &[u8]) -> [u8; 128] {
	let mut ever = [0u8; 128];
	let rd = |o: usize| u16_at(d, o).ok().map(|v| v as usize);
	let (Some(base), Some(nz)) = (u32_at(d, 0).ok().map(|v| v as usize), rd(6)) else { return [0xff; 128] };
	let mut seen = std::collections::HashSet::new();
	for z in 0..nz {
		let Some(row) = rd(8 + 2 * z).filter(|&r| r != 0).map(|r| r * 4) else { continue };
		let Some(ny) = rd(row + 2) else { continue };
		for y in 0..ny {
			let Some(col) = rd(row + 4 + 2 * y).filter(|&c| c != 0).map(|c| c * 4) else { continue };
			let Some(nx) = rd(col + 2) else { continue };
			for x in 0..nx {
				if let Some(idx) = rd(col + 4 + 2 * x).filter(|&i| i != 0xffff) {
					seen.insert(idx);
				}
			}
		}
	}
	for idx in seen {
		if let Some(m) = d.get(base + 128 * idx..base + 128 * idx + 128) {
			for (a, b) in ever.iter_mut().zip(m) {
				*a |= b;
			}
		}
	}
	ever[0x7f] |= 0x80;
	ever
}

#[derive(Clone)]
pub struct Occlusion {
	grid: Option<Vec<u8>>,
	/// Grid and mapping both present (retail enable variable 2); otherwise every object passes.
	pub enabled: bool,
	/// The fallback mode 0x15f5e8: 1 for the frame of a Visibomb launch (REN-CULL-OCC-M1), reset to 0 at the end of `update`.
	pub mode: u8,
	last: Option<[u8; 128]>,
	pub mask: [u8; 128],
	/// Retail mode: the mask retail itself built this field (RAM 0x174100 + level shift). Used as is instead of the grid lookup,
	/// which disagrees with retail where the camera leaves the grid (the Drek's Fleet ship fight).
	pub forced: Option<[u8; 128]>,
	/// [PC] The union of every grid cell's mask: a bit clear here is visible from no camera cell (hidden by design). Under
	/// Culling Off such an object stays hidden (`ever_visible`).
	ever: [u8; 128],
}

impl Occlusion {
	pub fn new(grid: Option<Vec<u8>>, map: Option<&OccMap>) -> Occlusion {
		let enabled = grid.as_ref().is_some_and(|g| !g.is_empty()) && map.is_some();
		let ever = if enabled { grid.as_deref().map_or([0xff; 128], grid_union) } else { [0xff; 128] };
		Occlusion { grid, enabled, mode: 0, last: None, mask: [0xff; 128], forced: None, ever }
	}

	/// The level's grid (core index +0x0c; the title scene has none) and mapping block.
	pub fn from_level(level: &Level) -> Occlusion {
		let grid = if level.title {
			None
		} else {
			u32_at(&level.index, 0x0c).ok().filter(|&s| s != 0).and_then(|s| level.block(s as usize).ok()).map(<[u8]>::to_vec)
		};
		Occlusion::new(grid, OccMap::new(&level.gameplay).as_ref())
	}

	/// The mask of cell (x, y, z), None outside the grid or in an empty cell.
	pub fn cell(&self, x: i32, y: i32, z: i32) -> Option<[u8; 128]> {
		let d = self.grid.as_deref()?;
		let rd = |o: usize| u16_at(d, o).ok().map(|v| v as usize);
		let z = z - rd(4)? as i32;
		if z < 0 || z >= rd(6)? as i32 {
			return None;
		}
		let row = rd(8 + 2 * z as usize)?;
		if row == 0 {
			return None;
		}
		let row = row * 4;
		let y = y - rd(row)? as i32;
		if y < 0 || y >= rd(row + 2)? as i32 {
			return None;
		}
		let col = rd(row + 4 + 2 * y as usize)?;
		if col == 0 {
			return None;
		}
		let col = col * 4;
		let x = x - rd(col)? as i32;
		if x < 0 || x >= rd(col + 2)? as i32 {
			return None;
		}
		let idx = rd(col + 4 + 2 * x as usize)?;
		if idx == 0xffff {
			return None;
		}
		let at = u32_at(d, 0).ok()? as usize + 128 * idx;
		d.get(at..at + 128)?.try_into().ok()
	}

	/// The nearer neighbour along one axis first (frac < 0.5: the lower cell), then the other.
	fn near(&self, frac: f32, lo: (i32, i32, i32), hi: (i32, i32, i32)) -> Option<[u8; 128]> {
		let (a, b) = if frac < 0.5 { (lo, hi) } else { (hi, lo) };
		self.cell(a.0, a.1, a.2).or_else(|| self.cell(b.0, b.1, b.2))
	}

	/// Once per frame before any tie / terrain fragment / moby test: builds `mask` for the camera at `campos`.
	/// REN-CULL-OCC-M1: `mode` (set to 1 by the Visibomb launch) is read here and reset to 0 at the end (FUN_001f8538).
	pub fn update(&mut self, campos: [f32; 3]) {
		self.update_mask(campos);
		if let (Some(f), true) = (self.forced, self.enabled) {
			self.mask = f;
			self.mask[0x7f] |= 0x80;
		}
		self.mode = 0;
	}

	fn update_mask(&mut self, campos: [f32; 3]) {
		if !self.enabled {
			self.mask = [0xff; 128];
			return;
		}
		let s = [campos[0] * 0.25, campos[1] * 0.25, campos[2] * 0.25];
		let c = [s[0] as i32, s[1] as i32, s[2] as i32];
		if let Some(m) = self.cell(c[0], c[1], c[2]) {
			self.mask = m;
			self.last = Some(m);
		} else {
			let mut merged = None;
			if self.mode == 0 {
				let f = |k: usize| s[k] - c[k] as f32;
				let step = |k: usize, d: i32| {
					let mut p = c;
					p[k] += d;
					(p[0], p[1], p[2])
				};
				let found = [self.near(f(0), step(0, -1), step(0, 1)), self.near(f(1), step(1, -1), step(1, 1)), self.near(f(2), step(2, -1), step(2, 1))];
				if found.iter().any(Option::is_some) {
					let mut m = [0u8; 128];
					for n in found.iter().flatten() {
						for (a, b) in m.iter_mut().zip(n) {
							*a |= b;
						}
					}
					merged = Some(m);
				}
			}
			match merged {
				Some(m) => {
					self.mask = m;
					self.last = Some(m);
				}
				None if self.mode == 1 => self.mask = [0xff; 128],
				None => self.mask = self.last.unwrap_or([0xff; 128]),
			}
		}
		self.mask[0x7f] |= 0x80;
	}

	/// [PC] Whether some camera cell of the level shows an object with `bytes`. Unmapped objects test bit 1023, which every mask
	/// keeps set, so they are always visible (retail draws them too); with no occlusion data everything is.
	pub fn ever_visible(&self, bytes: (u8, u8)) -> bool {
		!self.enabled || bytes.0 & self.ever[bytes.1 as usize & 0x7f] != 0
	}

	/// `ever_visible` for a moby's occlusion u16 (mask byte low, index byte high).
	pub fn ever_visible_moby(&self, occ: u16) -> bool {
		self.ever_visible((occ as u8, (occ >> 8) as u8))
	}

	/// Whether an object with `bytes` is drawn under the current mask.
	pub fn passes(&self, bytes: (u8, u8)) -> bool {
		bytes.0 & self.mask[bytes.1 as usize & 0x7f] != 0
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn bits_map_to_bytes() {
		assert_eq!(bit_bytes(1021), (0x20, 127));
		assert_eq!(bit_bytes(321), (0x02, 40));
		assert_eq!(bit_bytes(320), (0x01, 40));
	}

	#[test]
	fn tie_matching_falls_back_per_tie() {
		let map = OccMap { tfrags: vec![], ties: vec![(321, 1), (322, 3)], mobys: vec![] };
		assert_eq!(tie_bytes(&map, &[1, 3]), vec![bit_bytes(321), bit_bytes(322)]);
		// Count mismatch: per-id lookup, unknown ids unmapped.
		assert_eq!(tie_bytes(&map, &[3, 9, 1]), vec![bit_bytes(322), UNMAPPED, bit_bytes(321)]);
		assert_eq!(tfrag_bytes(&map, &[1]), vec![UNMAPPED]);
	}

	#[test]
	fn moby_bytes_and_test() {
		let map = OccMap { tfrags: vec![], ties: vec![], mobys: vec![(981, 112), (973, 25), (1021, 7)] };
		assert_eq!(moby_occ(&map, 112), 0x7a20);
		assert_eq!(moby_occ(&map, 25), 0x7920);
		assert_eq!(moby_occ(&map, 7), 0x7f20);
		assert_eq!(moby_occ(&map, 99), MOBY_UNMAPPED);
		let mut mask = [0u8; 128];
		mask[0x7f] = 0x80;
		assert!(occ_pass(&mask, MOBY_UNMAPPED));
		assert!(!occ_pass(&mask, 0));
	}

	#[test]
	fn a_disabled_mask_passes_everything() {
		let mut o = Occlusion::new(None, None);
		o.update([1.0, 2.0, 3.0]);
		assert!(o.passes((1, 0)) && o.passes(UNMAPPED));
	}

	/// A two-cell grid (masks 0x01 and 0x02 in byte 0): the union of its cells is what some camera cell can see.
	fn two_cells() -> Vec<u8> {
		let mut g = vec![0u8; 32 + 256];
		g[0..4].copy_from_slice(&32u32.to_le_bytes());
		for (at, v) in [(6, 1u16), (8, 3), (14, 1), (16, 5), (22, 2), (24, 0), (26, 1)] {
			g[at..at + 2].copy_from_slice(&v.to_le_bytes());
		}
		g[32] = 0x01;
		g[32 + 128] = 0x02;
		g
	}

	#[test]
	fn ever_visible_is_the_union_of_every_cell() {
		let g = two_cells();
		assert_eq!(&grid_union(&g)[..2], &[0x03, 0]);
		let mut map = OccMap::default();
		map.ties.push((0, 1));
		let o = Occlusion::new(Some(g), Some(&map));
		assert!(o.enabled);
		// Bits 0 and 1 of byte 0 are set in some cell, bit 2 in none; byte 0x7f keeps the always-visible bit 7 (unmapped objects).
		assert!(o.ever_visible((0x01, 0)) && o.ever_visible((0x02, 0)));
		assert!(!o.ever_visible((0x04, 0)) && !o.ever_visible((0x01, 1)));
		assert!(o.ever_visible(UNMAPPED) && o.ever_visible_moby(MOBY_UNMAPPED));
		assert!(o.ever_visible_moby(0x0001) && !o.ever_visible_moby(0x0004));
		// No occlusion data: every object is visible.
		assert!(Occlusion::new(None, None).ever_visible((0x04, 0)));
	}

	/// REN-CULL-OCC-M1 tests (1)-(2) on a one-cell grid: outside it with no neighbour, mode 1 gives all 0xff instead of
	/// `last`, and the mode is 0 again after the update.
	#[test]
	fn mode_1_misses_show_everything_once() {
		let mut g = vec![0u8; 32 + 128];
		g[0..4].copy_from_slice(&32u32.to_le_bytes());
		let w = |g: &mut Vec<u8>, at: usize, v: u16| g[at..at + 2].copy_from_slice(&v.to_le_bytes());
		w(&mut g, 6, 1);
		w(&mut g, 8, 3);
		w(&mut g, 14, 1);
		w(&mut g, 16, 5);
		w(&mut g, 22, 1);
		w(&mut g, 24, 0);
		g[32..].fill(1);
		let ever = grid_union(&g);
		let mut o = Occlusion { grid: Some(g), enabled: true, mode: 0, last: None, mask: [0xff; 128], forced: None, ever };
		o.update([1.0, 1.0, 1.0]);
		assert_eq!(o.mask[0], 1);
		o.update([100.0, 100.0, 100.0]);
		assert_eq!(o.mask[0], 1, "mode 0 keeps last");
		o.mode = 1;
		o.update([100.0, 100.0, 100.0]);
		assert_eq!(o.mask[0], 0xff);
		assert_eq!(o.mode, 0);
	}
}
