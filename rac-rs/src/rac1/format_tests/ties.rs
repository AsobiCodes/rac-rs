//! Disc tests of formats/ties.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::ties::*;
	use crate::{iso, toc};
	use crate::bytes::f32_at;
	use crate::bytes::i16_at;
	use crate::bytes::i32_at;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::gameplay::baked_raw;
	use crate::gameplay::pext5;
	use crate::gameplay::select_light;
	use crate::gameplay::DirLight;
	use crate::gameplay::TIE_CAP;
	use crate::level::texture_key;
	use crate::level::Level;
	use crate::level::TextureGroup;
	use crate::mesh::Lit;
	use crate::mesh::LitKind;
	use crate::mesh::Mesh;
	use std::collections::BTreeMap;
	use std::collections::HashMap;

	/// vertex_colours.md "Tie normals": class +0xc holds 64 unit-length i15 normals per class.
	#[test]
	fn veldin_tie_normals() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let mut lens = Vec::new();
		for e in level.table(0x20, 32).unwrap() {
			let n = tie_normals(level.block(u32_at(e, 0).unwrap() as usize).unwrap()).unwrap();
			lens.extend(n.iter().map(|v| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()));
		}
		let near = lens.iter().filter(|&&l| (l - 1.0).abs() < 0.01).count();
		println!("{} normals, {near} unit, first {:?}", lens.len(), &lens[..4]);
		assert!(near * 10 >= lens.len() * 9);
		let lights = crate::gameplay::lighting(&level).unwrap().lights;
		assert!(!lit_instances(&level, &lights).unwrap().is_empty());
	}

	/// `tie_wrap`: the CLAMP source qword of every tie material on Veldin, Novalis and Pokitaru is register 8 with WMS / WMT 0
	/// or 1 (no region mode), and Veldin classes 2020 / 2027 give the vertex_colours.md sources (u32 lo = WMS, hi = WMT).
	#[test]
	fn tie_material_clamp_wraps() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in levels.iter().filter(|l| [0, 1, 11].contains(&l.id)) {
			let level = Level::load(&mut disc, info).unwrap();
			let mut count = [0usize; 4];
			for e in level.table(0x20, 32).unwrap() {
				let data = level.block(u32_at(e, 0).unwrap() as usize).unwrap();
				let n = data[0x23] as usize;
				let records = &data[u32_at(data, 0x2c).unwrap() as usize..][..n * 80];
				let wraps: Vec<u32> = records.chunks_exact(80).map(tie_wrap).collect();
				for r in records.chunks_exact(80) {
					assert_eq!(u32_at(r, 0x38).unwrap(), 8, "class {} CLAMP register", u32_at(e, 4).unwrap());
					assert!(u32_at(r, 0x30).unwrap() <= 1 && u32_at(r, 0x34).unwrap() <= 1, "class {} region mode", u32_at(e, 4).unwrap());
				}
				wraps.iter().for_each(|&w| count[w as usize] += 1);
				match (info.id, u32_at(e, 4).unwrap()) {
					(0, 2020) => assert_eq!(wraps, vec![2, 2, 0, 2]),
					(0, 2027) => assert_eq!(wraps, vec![3, 2, 2, 0, 3, 0]),
					_ => {}
				}
			}
			println!("level {}: tie materials repeat / u clamp / v clamp / both {count:?}", info.id);
			assert!(count[1] + count[2] + count[3] > 0);
		}
	}

	/// The instance bounding sphere (class sphere x placement) encloses the instance's vertices on Veldin and Novalis.
	#[test]
	fn instance_spheres_enclose_the_mesh() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in &levels[..2] {
			let level = Level::load(&mut disc, info).unwrap();
			let lights = crate::gameplay::lighting(&level).unwrap().lights;
			let (mut bad, mut n) = (0, 0);
			for t in lit_instances(&level, &lights).unwrap() {
				let m = &t.lods[0];
				let s = m.lit.as_ref().unwrap().sphere;
				let far = m.positions.iter().map(|p| ((p[0] - s[0]).powi(2) + (p[1] - s[1]).powi(2) + (p[2] - s[2]).powi(2)).sqrt()).fold(0.0f32, f32::max);
				n += 1;
				bad += (far > s[3] * 1.01 + 0.1) as usize;
			}
			println!("level {}: {bad} of {n} tie spheres do not enclose their vertices", info.id);
			assert_eq!(bad, 0);
		}
	}

	/// All three LODs of every class build on Veldin and Novalis; the fat vertices carry deltas, regular ones none, and
	/// each LOD mesh has one delta and one baked colour per vertex.
	#[test]
	fn three_lods_build_with_morph_deltas() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in &levels[..2] {
			let level = Level::load(&mut disc, info).unwrap();
			let lights = crate::gameplay::lighting(&level).unwrap().lights;
			let ties = lit_instances(&level, &lights).unwrap();
			let (mut fat, mut empty) = ([0usize; 3], [0usize; 3]);
			for t in &ties {
				for k in 0..3 {
					assert_eq!(t.morph[k].len(), t.lods[k].positions.len());
					assert_eq!(t.lods[k].colours.len(), t.lods[k].positions.len());
					fat[k] += t.morph[k].iter().any(|d| d.iter().any(|&c| c != 0.0)) as usize;
					empty[k] += t.lods[k].faces.is_empty() as usize;
				}
				for k in 0..3 {
					assert!(t.slots[k].iter().all(|(s, _)| s.iter().all(|&i| i < 64)) && t.slots[k].len() == t.lods[k].positions.len());
				}
				assert!(t.cull.dist > 0.0 && t.cull.lod[0] <= t.cull.lod[1] && t.cull.lod[1] <= t.cull.lod[2], "{:?}", t.cull);
			}
			println!("level {}: {} ties, with fat vertices per LOD {:?}, empty per LOD {:?}", info.id, ties.len(), fat, empty);
			assert!(!ties.is_empty() && empty[0] == 0);
		}
	}

	/// REN-CULL-TIE-MCOL tests 1-2: the index streams of Veldin class 2020 LOD 0, packets 0 and 1.
	#[test]
	fn index_stream_slots_of_class_2020() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let e = level.table(0x20, 32).unwrap().into_iter().find(|e| u32_at(e, 4).unwrap() == 2020).unwrap();
		let data = level.block(u32_at(e, 0).unwrap() as usize).unwrap();
		let table = u32_at(data, 0).unwrap() as usize;
		let slots = |packet: usize, regular: usize, fat: usize| {
			let entry = &data[table + packet * 16..table + packet * 16 + 16];
			assert_eq!((entry[10], entry[11]), if packet == 0 { (0x3e, 0x20) } else { (entry[10], 34) });
			let start = table + u32_at(entry, 0).unwrap() as usize;
			let stream = &data[start + entry[10] as usize * 16..][..entry[11] as usize * 4];
			packet_slots(stream, regular, fat).unwrap()
		};
		let p0 = slots(0, 16, 28);
		let reg: Vec<usize> = p0[..16].iter().map(|s| s.0[0]).collect();
		assert_eq!(reg, vec![4, 1, 5, 1, 8, 10, 9, 9, 4, 1, 5, 1, 33, 5, 61, 13]);
		assert_eq!((p0[16], p0[17]), (([6, 4, 1], true), ([7, 5, 1], true)));
		let p1 = slots(1, 15, 30);
		let reg: Vec<usize> = p1[..15].iter().map(|s| s.0[0]).collect();
		assert_eq!(reg, vec![0, 1, 2, 1, 22, 14, 23, 14, 22, 23, 25, 15, 34, 14, 14]);
		assert_eq!((p1[15], p1[16]), (([3, 0, 1], true), ([3, 2, 1], true)));
	}
}
