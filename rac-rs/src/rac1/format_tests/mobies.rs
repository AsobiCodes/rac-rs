//! Disc tests of formats/mobies.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::mobies::*;
	use crate::{iso, toc};
	use std::path::PathBuf;
	use crate::bytes::f32_at;
	use crate::bytes::i16_at;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::level::texture_key;
	use crate::level::Level;
	use crate::level::TextureGroup;
	use crate::mesh::Mesh;
	use crate::wad;
	use std::collections::HashMap;

	fn open() -> (iso::Disc, Vec<toc::Level>) {
		let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = iso::Disc::open(&path).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		(disc, levels)
	}

	/// Every level's moby classes decode (no CLAMP region mode, which `packet_strips` refuses) and the faces' wrap bits are the
	/// source words WMS | WMT << 1 of their AD block; prints the census (`--nocapture`).
	#[test]
	fn moby_clamp_wrap_census() {
		let (mut disc, levels) = open();
		let (mut faces, mut clamped, mut classes_n) = (0, [0usize; 4], 0);
		for l in levels.iter().take(20) {
			let Ok(level) = Level::load(&mut disc, l) else { continue };
			let classes = moby_classes(&level).expect("moby classes decode");
			for c in classes.values() {
				classes_n += 1;
				for f in &c.mesh.faces {
					assert_eq!(f.0 & !(SLOT_MASK | CULL_BACK | CULL_FRONT), 0);
					clamped[(f.0 >> crate::shrubs::WRAP_SHIFT & 3) as usize] += 1;
					faces += 1;
				}
			}
		}
		eprintln!("moby wrap census: {classes_n} classes, {faces} faces, wrap 0/1/2/3 = {clamped:?}");
	}

	/// Every moby material on the disc has a TEX1 K (MMIN 4): each textured face key carries a K code, the distinct K values fit the
	/// table of 15, and a texture drawn with two K values just gets two draw slots. Prints the census (`--nocapture`).
	#[test]
	fn moby_k_census() {
		let (mut disc, levels) = open();
		let (mut faces, mut without, mut most) = (0usize, 0usize, 0usize);
		let (mut two_k, mut all_ks) = (0usize, std::collections::BTreeSet::new());
		for l in levels.iter().take(20) {
			let Ok(level) = Level::load(&mut disc, l) else { continue };
			reset_k_table();
			let classes = moby_classes(&level).expect("moby classes decode");
			let mut per_tex: HashMap<u32, std::collections::BTreeSet<u32>> = HashMap::new();
			for c in classes.values() {
				for f in &c.mesh.faces {
					if f.0 & KEY_MASK == texture_key(TextureGroup::Moby, FLAT_SLOT) || (f.0 & KEY_MASK) >> 16 != 1 {
						continue;
					}
					faces += 1;
					match key_k(f.0) {
						Some(k) => {
							per_tex.entry(f.0 & KEY_MASK).or_default().insert(k);
							all_ks.insert(k);
						}
						None => without += 1,
					}
				}
			}
			two_k += per_tex.values().filter(|v| v.len() > 1).count();
			most = most.max(with_ks(|t| t.len()));
		}
		let ks: Vec<f32> = all_ks.iter().map(|&k| (if k >= 2048 { k as i32 - 4096 } else { k as i32 }) as f32 / 16.0).collect();
		eprintln!("moby K census: {faces} textured faces, {without} without K, {two_k} textures with two K values, most K per level {most}, {} distinct K overall {ks:?}", ks.len());
		assert!(without == 0 && most <= 15);
	}

	/// Packets with cull mode 2 give class 705 (fan) faces the back-cull bit; jets 660/688 are mode 254.
	#[test]
	fn cull_bits_reach_the_faces() {
		let (mut disc, levels) = open();
		let (mut fan, mut jets) = (0, 0);
		for l in levels.iter().take(20) {
			let Ok(level) = Level::load(&mut disc, l) else { continue };
			let Ok(classes) = moby_classes(&level) else { continue };
			if let Some(c) = classes.get(&705) {
				assert!(c.mesh.faces.iter().all(|f| f.0 & CULL_BACK != 0 && f.0 & CULL_FRONT == 0));
				fan += 1;
			}
			for id in [660, 688] {
				if let Some(c) = classes.get(&id) {
					assert!(c.mesh.faces.iter().all(|f| f.0 & (CULL_BACK | CULL_FRONT) == 0));
					jets += 1;
				}
			}
		}
		println!("fan levels {fan}, jet classes {jets}");
	}

	// BRIDGE-64a: the shadow primitive block before the skeleton (class header byte 0xf), walked to the last-flagged entry.
	#[test]
	fn shadow_prims_from_disc() {
		let (mut disc, levels) = open();
		let aridia = Level::load(&mut disc, &levels[2]).unwrap();
		let classes = moby_classes(&aridia).unwrap();
		// Constructor bot 612: one sphere on joint 7 (12 ring sides, r 0.36 at class scale) ahead of 10 capsules.
		let bot = &classes[&612].shadow_prims;
		assert_eq!(bot.len(), 11);
		match bot[0] {
			ShadowPrim::Sphere { joint, n, at, r } => {
				assert_eq!((joint, n), (7, 12));
				assert_eq!(at, [0.0; 3]);
				assert!((r - 0.36).abs() < 0.01, "{r}");
			}
			ShadowPrim::Capsule => panic!("612 starts with a sphere"),
		}
		assert!(bot[1..].iter().all(|p| matches!(p, ShadowPrim::Capsule)));
		// Shark 580: capsules only (BRIDGE-64b).
		let shark = &classes[&580].shadow_prims;
		assert_eq!(shark.len(), 6);
		assert!(shark.iter().all(|p| matches!(p, ShadowPrim::Capsule)));
		// Every placed class: the allocator's 0x400 flag exactly when the class has a shadow block.
		for li in 0..3 {
			let level = Level::load(&mut disc, &levels[li]).unwrap();
			for (id, c) in &moby_classes(&level).unwrap() {
				let f = crate::game::sched::placed_flags(*id);
				if f != 0 {
					assert_eq!(f & 0x400 != 0, !c.shadow_prims.is_empty(), "level {li} class {id} flags {f:#x}");
				}
			}
		}
	}

	// NOV-33.1 test 1: untextured (slot -1) faces join the mesh under FLAT_SLOT, `flat` stays for the fx path.
	#[test]
	fn untextured_faces_draw_with_the_flat_slot() {
		let (mut disc, levels) = open();
		let novalis = Level::load(&mut disc, &levels[1]).unwrap();
		let classes = moby_classes(&novalis).unwrap();
		let rocket = &classes[&686];
		assert_eq!(rocket.mesh.positions.len(), 50);
		assert_eq!(rocket.mesh.faces.len(), 64);
		assert_eq!(rocket.flat.len(), 64);
		assert_eq!(rocket.mesh.strip_phase.len(), 64);
		assert!(rocket.mesh.faces.iter().all(|f| f.0 == 0x1ffff && f.0 == texture_key(TextureGroup::Moby, FLAT_SLOT)));
		let veldin = Level::load(&mut disc, &levels[0]).unwrap();
		let classes = moby_classes(&veldin).unwrap();
		assert_eq!(classes[&1137].mesh.faces.len(), 64);
		assert_eq!(classes[&10].mesh.faces.len(), 4502);
	}

	/// Collision blocks against collision_and_gameplay.md "Moby collision shapes": the crate cube, the robot's
	/// two upright capsules, the toad's capsule.
	#[test]
	fn collision_blocks() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let classes = moby_classes(&level).unwrap();
		let near = |a: f32, b: f32| (a - b).abs() < 1e-3;
		let c = classes[&500].collision.as_ref().unwrap();
		assert_eq!((c.verts.len(), c.faces.len()), (8, 12));
		assert!(matches!(c.prims[..], [(Prim::Sphere { centre, r }, 2)] if near(centre[2], 0.5) && near(r, 0.5)));
		let (lo, hi) = c.verts.iter().fold(([9.0f32; 3], [-9.0f32; 3]), |(l, h), v| (std::array::from_fn(|i| l[i].min(v[i])), std::array::from_fn(|i| h[i].max(v[i]))));
		assert!(near(lo[0], -0.5) && near(hi[0], 0.5) && near(lo[2], 0.0) && near(hi[2], 1.0), "{lo:?} {hi:?}");
		let r = classes[&1440].collision.as_ref().unwrap();
		assert!(matches!(r.prims[..], [(Prim::Upright { base: a, h: ha, r: ra }, 1), (Prim::Upright { base: b, h: hb, r: rb }, 2)]
			if near(a[2], 0.5) && near(ha, 0.5) && near(ra, 0.5) && near(b[2], 0.5) && near(hb, 0.75) && near(rb, 0.75)), "{:?}", r.prims);
		assert!(r.faces.is_empty());
		let t = classes[&749].collision.as_ref().unwrap();
		assert!(matches!(t.prims[..], [(Prim::Upright { base, h, r }, 3)] if near(base[2], 0.5) && near(h, 0.2) && near(r, 0.5)), "{:?}", t.prims);
	}

	#[test]
	fn all_levels() {
		let (mut disc, levels) = open();
		let mut failed = 0;
		for info in &levels {
			let result = Level::load(&mut disc, info).and_then(|l| moby_classes(&l));
			match result {
				Ok(c) => println!("level {}: {} classes, {} triangles", info.id, c.len(), c.values().map(|m| m.mesh.faces.len()).sum::<usize>()),
				Err(e) => {
					println!("level {}: error {e}", info.id);
					failed += 1;
				}
			}
		}
		println!("{} levels, {failed} failed", levels.len());
		assert_eq!(failed, 0);
	}

	/// Shared edges of adjacent faces should run in opposite directions when winding is consistent.
	#[test]
	fn edge_consistency() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let c = &moby_classes(&level).unwrap()[&0];
		let mut ids: HashMap<[i64; 3], u32> = HashMap::new();
		let key: Vec<u32> = c
			.mesh
			.positions
			.iter()
			.map(|p| {
				let n = ids.len() as u32;
				*ids.entry(p.map(|x| (x * 10000.0).round() as i64)).or_insert(n)
			})
			.collect();
		let mut edges: HashMap<(u32, u32), usize> = HashMap::new();
		for (_, f) in &c.mesh.faces {
			let k = f.map(|i| key[i as usize]);
			for e in 0..3 {
				*edges.entry((k[e], k[(e + 1) % 3])).or_insert(0) += 1;
			}
		}
		let opposite: usize = edges.iter().filter(|((a, b), _)| a < b).map(|(&(a, b), &n)| n.min(edges.get(&(b, a)).copied().unwrap_or(0))).sum();
		let same: usize = edges.iter().filter(|((a, b), _)| a != b).map(|(_, &n)| n - 1).sum();
		println!("ratchet shared edges: {opposite} opposite, {same} same direction");
		assert!(opposite > 5 * same);
	}

	/// NOV-35b tests 1, 2 and 4: Novalis class 13 metal packets, the metal normal and the sphere-map UV; every metal face on the disc
	/// draws with a metal slot (< -1).
	#[test]
	fn metal_packets_decode() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[1]).unwrap();
		let classes = moby_classes(&level).unwrap();
		let c = &classes[&13];
		assert_eq!(c.metal.len(), 2);
		assert_eq!((c.metal[0].positions.len(), c.metal[1].positions.len()), (96, 60));
		let s = c.scale / 1024.0;
		assert_eq!(c.metal[0].positions[0], [-5944.0 * s, 3789.0 * s, 2196.0 * s]);
		assert_eq!(c.metal[0].skin[0], Skin::single(0));
		let n = normal(0x7d, 2);
		for (x, e) in n.iter().zip([-0.9960892f32, 0.07347595, 0.04906768]) {
			assert!((x - e).abs() < 1e-6, "{n:?}");
		}
		assert_eq!(c.metal[0].normals[0], n);
		let id = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
		assert_eq!(metal_uv(id, n), [0.001953125, 0.5366211]);
		assert!(c.metal.iter().all(|p| !p.faces.is_empty() && p.faces.iter().all(|f| f.0 == -3)));
		let mut bad = Vec::new();
		for info in &levels {
			let Ok(l) = Level::load(&mut disc, info) else { continue };
			for (id, k) in moby_classes(&l).unwrap() {
				if k.metal.iter().any(|p| p.faces.iter().any(|f| f.0 >= -1)) {
					bad.push((info.id, id));
				}
			}
		}
		assert!(bad.is_empty(), "{bad:?}");
	}
}
