//! Disc tests of formats/occlusion.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::occlusion::*;
	use crate::{iso, level::Level, toc};
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;

	fn veldin() -> Level {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		Level::load(&mut disc, &levels[0]).unwrap()
	}

	#[test]
	fn veldin_grid_and_masks() {
		let l = veldin();
		let grid = l.block(u32_at(&l.index, 0x0c).unwrap() as usize).unwrap();
		assert_eq!(&grid[..10], &[0x50, 0x27, 0, 0, 5, 0, 7, 0, 6, 0]);
		let map = OccMap::new(&l.gameplay).unwrap();
		assert_eq!((map.tfrags.len(), map.ties.len(), map.mobys.len()), (460, 1114, 111));
		assert_eq!(&map.ties[..4], &[(321, 1), (322, 3), (323, 4), (324, 6)]);
		let mut o = Occlusion::from_level(&l);
		assert!(o.enabled);
		o.update([122.0, 66.0, 22.0]);
		// Cell (30, 16, 5) is index 0: mask byte 40 = 0x42 keeps bit 321 and drops bit 320.
		assert_eq!(o.mask[40], 0x42);
		assert!(o.passes(bit_bytes(321)));
		assert!(!o.passes(bit_bytes(320)));
		// Byte 0x7f gets the always-visible bit: an unmapped object draws.
		assert_eq!(o.mask[0x7f] & 0x80, 0x80);
		assert!(o.passes(UNMAPPED));
		// (31, 15, 5) holds no cell.
		assert!(o.cell(31, 15, 5).is_none());
		assert!(o.cell(30, 16, 5).is_some());
	}

	#[test]
	fn veldin_moby_pairs_and_cell_zero() {
		let l = veldin();
		let map = OccMap::new(&l.gameplay).unwrap();
		assert_eq!(&map.mobys[..5], &[(973, 25), (973, 26), (973, 27), (973, 28), (974, 29)]);
		let mut o = Occlusion::from_level(&l);
		o.update([122.0, 66.0, 22.0]);
		assert_eq!((o.mask[122], o.mask[121]), (0x20, 0x0a));
		assert!(occ_pass(&o.mask, moby_occ(&map, 112)));
		assert!(!occ_pass(&o.mask, moby_occ(&map, 25)));
		let pass = map.mobys.iter().filter(|p| occ_pass(&o.mask, bit_bytes(p.0).1 as u16 * 256 + bit_bytes(p.0).0 as u16)).count();
		assert_eq!((pass, map.mobys.len() - pass), (16, 95));
	}

	#[test]
	fn placed_mobys_take_their_pairs() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for (k, moby_pairs, ties) in [(0, 111, 1114), (1, 398, 1508)] {
			let l = Level::load(&mut disc, &levels[k]).unwrap();
			let map = OccMap::new(&l.gameplay).unwrap();
			assert_eq!((map.mobys.len(), map.ties.len()), (moby_pairs, ties));
			let placed = crate::gameplay::moby_instances(&l).unwrap();
			// Mapped records (+0x5c == 0) take a pair's bytes; the rest stay always-drawn.
			let mapped = placed.iter().filter(|m| m.occ != MOBY_UNMAPPED).count();
			assert!(mapped > 0 && mapped <= moby_pairs, "{mapped}");
		}
	}

	/// Veldin (disc): 25 ties and 16 terrain fragments are set in no grid cell (hidden by design), every moby pair is.
	#[test]
	fn veldin_objects_no_cell_sees() {
		let l = veldin();
		let o = Occlusion::from_level(&l);
		let map = OccMap::new(&l.gameplay).unwrap();
		let (_, tfrags) = crate::terrain::terrain_lods(l.terrain_block().unwrap(), None, Some(&map)).unwrap();
		let never: Vec<usize> = (0..tfrags.len()).filter(|&i| !o.ever_visible(tfrags[i].cull.occ)).collect();
		assert_eq!(never.len(), 16);
		assert!(never.contains(&407), "the big piece at (173, 282, 26)");
		let ids: Vec<u32> = map.ties.iter().map(|p| p.1).collect();
		assert_eq!(tie_bytes(&map, &ids).into_iter().filter(|&b| !o.ever_visible(b)).count(), 25);
		let seen = |m: &(u32, u32)| o.ever_visible(bit_bytes(m.0));
		assert!(map.mobys.iter().all(seen));
	}
}
