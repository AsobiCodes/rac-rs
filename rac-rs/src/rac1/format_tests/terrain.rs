//! Disc tests of formats/terrain.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::{gameplay, iso, level::Level, toc};
	use crate::terrain::*;
	use crate::bytes::f32_at;
	use crate::bytes::i32_at;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::gameplay::baked_colour;
	use crate::gameplay::pext5;
	use crate::gameplay::select_light;
	use crate::gameplay::DirLight;
	use crate::gameplay::TERRAIN_CAP;
	use crate::mesh::Lit;
	use crate::mesh::LitKind;
	use crate::mesh::Mesh;
	use std::collections::BTreeSet;

	/// Every fragment of both levels: the three meshes are prefixes of one info list, the lists index real vertices and the
	/// box corners enclose the sphere centre; the LOD scale is the block's f32.
	#[test]
	fn lods_of_veldin_and_novalis() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for (k, count, b) in [(0, 460, 12.0), (1, 1004, 15.0)] {
			let l = Level::load(&mut disc, &levels[k]).unwrap();
			let lights = gameplay::lighting(&l).unwrap().lights;
			let map = crate::occlusion::OccMap::new(&l.gameplay);
			let (got, lods) = terrain_lods(l.terrain_block().unwrap(), Some(&lights), map.as_ref()).unwrap();
			assert_eq!((lods.len(), got), (count, b));
			let near_meshes = terrain_lit(l.terrain_block().unwrap(), 0, Some(&lights)).unwrap();
			let mut off = 0;
			for (f, m) in lods.iter().zip(&near_meshes) {
				assert_eq!(f.near.positions, m.positions);
				assert!(f.far.positions.len() <= f.mid.positions.len() && f.mid.positions.len() <= f.near.positions.len());
				assert_eq!(f.far.positions[..], f.near.positions[..f.far.positions.len()]);
				assert_eq!(f.mid.positions[..], f.near.positions[..f.mid.positions.len()]);
				assert_eq!(f.vert_pos.len(), f.near.positions.len());
				assert_eq!(f.lit.positions.len(), f.pos_world.len());
				for st in 0..2 {
					assert!(f.morph[st].iter().all(|e| e.iter().all(|&x| (x as usize) < f.pos_world.len())));
					assert!(f.snap[st].iter().all(|e| (e[0] as usize) < f.near.positions.len() && (e[2] as usize) < f.near.positions.len()));
				}
				let c = f.cull.centre;
				let lo: Vec<f32> = (0..3).map(|j| f.cull.corners.iter().map(|p| p[j]).fold(f32::MAX, f32::min)).collect();
				let hi: Vec<f32> = (0..3).map(|j| f.cull.corners.iter().map(|p| p[j]).fold(f32::MIN, f32::max)).collect();
				assert!((0..3).all(|j| lo[j] <= c[j] + 1.0 && c[j] <= hi[j] + 1.0), "corners {lo:?} {hi:?} centre {c:?}");
				off += 1;
			}
			assert_eq!(off, count);
			println!("level {k}: far/mid/near {} / {} / {} vertices in total, snaps {}", lods.iter().map(|f| f.far.positions.len()).sum::<usize>(), lods.iter().map(|f| f.mid.positions.len()).sum::<usize>(), lods.iter().map(|f| f.near.positions.len()).sum::<usize>(), lods.iter().map(|f| f.snap[0].len() + f.snap[1].len()).sum::<usize>());
		}
	}

	#[test]
	fn veldin_baked_colours() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		let lights = gameplay::lighting(&l).unwrap().lights;
		let meshes = crate::terrain::terrain_lit(l.terrain_block().unwrap(), 0, Some(&lights)).unwrap();
		assert_eq!(meshes.len(), 460);
		// The dynamic-light sphere (header / 1024) encloses the vertices of its fragment (the units check), and normals are unit.
		for m in &meshes {
			let l = m.lit.as_ref().unwrap();
			assert_eq!(l.normals.len(), m.positions.len());
			for p in &m.positions {
				let d = ((p[0] - l.sphere[0]).powi(2) + (p[1] - l.sphere[1]).powi(2) + (p[2] - l.sphere[2]).powi(2)).sqrt();
				// A few LOD-0 refinement vertices poke out of the sphere by under 10%.
				assert!(d <= l.sphere[3] * 1.1 + 0.01, "{} outside sphere by {}", m.name, d - l.sphere[3]);
			}
		}
		for m in &meshes {
			assert_eq!(m.colours.len(), m.positions.len());
			assert!(m.colours.iter().all(|c| c[3] == 128.0 && c[..3].iter().all(|&x| (0.0..=255.0).contains(&x))));
		}
		let avg: Vec<f32> = (0..3).map(|j| meshes.iter().flat_map(|m| &m.colours).map(|c| c[j]).sum::<f32>() / meshes.iter().map(|m| m.colours.len()).sum::<usize>() as f32).collect();
		println!("average terrain colour {avg:?}");
	}

	/// Eudora (level 4) terrain CLAMP_1: the per-mode record counts equal the retail RAM records of state play/eudora1_0039
	/// (A+D TEX0 / TEX1 / CLAMP / MIPTBP1 / MIPTBP2 qwords: 649 repeat, 71 clamp u, 762 clamp v, 116 both; fx_census_late.md run 21).
	#[test]
	fn eudora_material_wrap() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[4]).unwrap();
		let (_, frags) = terrain_lods(l.terrain_block().unwrap(), None, None).unwrap();
		let all: Vec<TfragMaterial> = frags.iter().flat_map(|f| f.materials.iter().copied()).collect();
		let n = |w: u32| all.iter().filter(|m| m.wrap == w).count();
		assert_eq!((all.len(), n(0), n(1), n(2), n(3)), (1598, 649, 71, 762, 116));
	}

	/// REN-TEX-RESIDENCY tests (2) (3): the material records of Novalis (level 1).
	#[test]
	fn novalis_material_mip_state() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let (_, frags) = terrain_lods(l.terrain_block().unwrap(), None, None).unwrap();
		let all: Vec<TfragMaterial> = frags.iter().flat_map(|f| f.materials.iter().copied()).collect();
		assert_eq!(all.len(), 2167);
		assert!(all.iter().all(|m| m.mmin == 4));
		assert_eq!(frags[0].materials[0].k, -7.8125);
		let common = all.iter().filter(|m| m.k == -8.5625).count();
		assert_eq!(common, 736);
		let chains = l.terrain_mips().unwrap();
		let size = |m: &TfragMaterial| {
			let c = chains[m.texture as usize].as_ref().unwrap();
			(c[0].w, c.len())
		};
		assert_eq!(all.iter().filter(|m| size(m) == (128, 4)).count(), 2093);
		assert_eq!(all.iter().filter(|m| size(m) == (64, 4)).count(), 72);
		assert_eq!(all.iter().filter(|m| size(m) == (32, 3)).count(), 2);
		// Is K a function of the texture alone?
		let mut by_tex: std::collections::BTreeMap<u32, std::collections::BTreeSet<i32>> = Default::default();
		for m in &all {
			by_tex.entry(m.texture).or_default().insert((m.k * 16.0) as i32);
		}
		let multi = by_tex.values().filter(|s| s.len() > 1).count();
		println!("{} textures, {} with more than one K, {} distinct (texture, K)", by_tex.len(), multi, by_tex.values().map(|s| s.len()).sum::<usize>());
	}

	/// REN-TEX-RESIDENCY test (4): fragment 0's first texture sphere (the residency request list entry the GS mip state depends
	/// on) and its two distances. The port keeps no residency state, so the numbers are checked on the authored data only.
	#[test]
	fn novalis_first_texture_sphere() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let data = l.terrain_block().unwrap();
		let table_offset = u32_at(data, 0).unwrap() as usize;
		let header = &data[table_offset..table_offset + FRAGMENT_SIZE];
		let start = table_offset + u32_at(header, 0x10).unwrap() as usize;
		let at = start + u16_at(header, 0x2e).unwrap() as usize;
		let c: Vec<f32> = (0..3).map(|k| f32_at(data, at + k * 4).unwrap() / 1024.0).collect();
		let (r, b, tex) = (u16_at(data, at + 0xc).unwrap() as f32 / 1024.0, data[at + 0xe] as f32, data[at + 0xf]);
		assert_eq!(c, vec![292.62503, 142.31332, 82.875]);
		assert_eq!((r, b as u32, tex), (1.2548828, 156, 13));
		// Request below 128 b, full at 64 b (vf22.x = 0.5), both in 1/1024 m.
		assert_eq!(128.0 * b, 19968.0);
		assert_eq!(128.0 * b * 0.5, 9984.0);
	}
}
