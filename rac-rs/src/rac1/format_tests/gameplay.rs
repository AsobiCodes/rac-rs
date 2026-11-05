//! Disc tests of formats/gameplay.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::gameplay::*;
	use crate::{iso, toc};
	use crate::bytes::f32_at;
	use crate::bytes::i32_at;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::level::Level;
	use crate::physics::V3;

	#[test]
	fn novalis_ship_flight_words() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let s = u32_at(&l.gameplay, 0).unwrap() as usize;
		let w = ship_flight(&l).unwrap();
		assert_eq!(w, (i32_at(&l.gameplay, s + 0x3c).unwrap(), i32_at(&l.gameplay, s + 0x40).unwrap(), i32_at(&l.gameplay, s + 0x44).unwrap()));
		eprintln!("ship flight words {w:?}");
		let d = flight_data(w, &polygons(&l).unwrap(), &cuboids(&l).unwrap());
		assert_eq!(d.0.is_some(), w.0 >= 0);
	}

	#[test]
	fn novalis_moby_slot_count_is_256() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		assert_eq!(moby_slot_count(&l).unwrap(), 256);
	}

	#[test]
	fn veldin_moby_slot_count_is_256() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		assert_eq!(moby_slot_count(&l).unwrap(), 256);
	}

	#[test]
	fn novalis_birdbots_and_peckbots_by_mission_slots() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let all = moby_instances(&l).unwrap();
		let count = |m: &[u8; 16]| {
			let n = |c: u32| all.iter().filter(|i| i.class_id == c && i.in_phase(m)).count();
			(n(459), n(577))
		};
		assert_eq!(count(&[0; 16]), (25, 14));
		let mut done = [0u8; 16];
		for s in [0, 1, 2, 6] {
			done[s] = 0xff;
		}
		assert_eq!(count(&done), (5, 58));
		// 528 and 913 share UID 376: identity is the placement index.
		assert_eq!(all[528].uid, all[913].uid);
		assert!(all.iter().enumerate().all(|(i, m)| m.index as usize == i));
	}

	#[test]
	fn novalis_point_lights() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let lt = lighting(&l).unwrap();
		assert_eq!(lt.points.len(), 17);
		let p = lt.points[0];
		assert!((p.pos[0] - 222.418).abs() < 0.01 && (p.r2 - 6.5025).abs() < 0.001, "{p:?}");
		assert_eq!(p.col, [49, 46, 36]);
		// At the light, the full colour is added (light 0 must be the first match in its cell).
		let a = lt.point_ambient(p.pos, [0.0; 3]);
		assert!((a[0] - (49 * 255 >> 8) as f32 / 128.0).abs() < 1e-6);
		// The grid reaches every light from its own position.
		assert!(lt.points.iter().all(|l| lt.point_at(l.pos).is_some()));
		eprintln!("wraps {:?}", lt.lights.iter().map(|d| d.wrap).collect::<Vec<_>>());
	}

	/// ARI-3 (decomp/notes/aridia_flow.md): hero spawn, death height and header ship on Aridia.
	#[test]
	fn aridia_spawn() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[2]).unwrap();
		assert_eq!(spawn(&l).unwrap(), ([210.41f32, 170.35, 25.35], 2.3099957f32));
		assert_eq!(death_height(&l).unwrap(), 0.0);
		let (pos, yaw) = ship(&l).unwrap().unwrap();
		let want = [205.35262f32, 153.97421, 26.614683];
		assert!((0..3).all(|k| (pos[k] - want[k]).abs() < 1e-5) && (yaw - 2.0943952).abs() < 1e-5, "{pos:?} {yaw}");
		assert_eq!(ship_flight(&l).unwrap(), (23, 70, 71));
	}
}

mod dump {
	use crate::gameplay::*;
	use crate::bytes::f32_at;
	use crate::bytes::i32_at;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::level::Level;
	use crate::physics::V3;
	use crate::toc;

	#[test]
	#[ignore]
	fn dump_instances() {
		use crate::{iso, level::Level, toc};
		let lv: usize = std::env::var("DUMP_LEVEL").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
		let classes: Vec<u32> = std::env::var("DUMP_CLASSES").unwrap_or_default().split(',').filter_map(|v| v.parse().ok()).collect();
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[lv]).unwrap();
		for (i, m) in crate::gameplay::moby_instances(&l).unwrap().iter().enumerate() {
			if !classes.is_empty() && !classes.contains(&m.class_id) {
				continue;
			}
			let words: Vec<String> = m.pvars.chunks_exact(4).map(|c| {
				let u = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
				let f = f32::from_bits(u);
				if u == 0 { "0".into() } else if u == 0xffff_ffff { "-1".into() } else if (u as i32).abs() < 100000 { format!("{}", u as i32) } else if f.abs() > 1e-4 && f.abs() < 1e6 { format!("{f:.3}") } else { format!("{u:#x}") }
			}).collect();
			println!("#{i} c{} pos {:.2?} rot {:.2?} param {} flags {:#x} uid {} pv[{}]: {}", m.class_id, m.pos, m.rot, m.param, m.flags, m.uid, m.pvars.len(), words.join(" "));
		}
	}
}
