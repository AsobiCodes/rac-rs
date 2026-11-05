//! Disc tests of formats/shrubs.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::shrubs::*;
	use crate::{iso, toc};
	use std::path::PathBuf;
	use crate::bytes::f32_at;
	use crate::bytes::i16_at;
	use crate::bytes::i32_at;
	use crate::bytes::span;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::level::texture_key;
	use crate::level::Level;
	use crate::level::TextureGroup;
	use crate::mesh::Lit;
	use crate::mesh::LitKind;
	use crate::mesh::Mesh;
	use crate::gameplay::baked_raw;
	use crate::gameplay::select_light;
	use crate::gameplay::DirLight;
	use crate::gameplay::TIE_CAP;
	use crate::ties::instance_sphere;
	use crate::ties::placement;
	use crate::ties::transformed;
	use crate::ties::unit;
	use std::collections::HashMap;

	/// The instance bounding sphere (class sphere x placement) encloses the instance's vertices on Veldin and Novalis.
	#[test]
	fn instance_spheres_enclose_the_mesh() {
		let mut disc = iso::Disc::open(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in &levels[..2] {
			let level = Level::load(&mut disc, info).unwrap();
			let lights = crate::gameplay::lighting(&level).unwrap().lights;
			let (mut bad, mut n) = (0, 0);
			for m in lit_instances(&level, &lights).unwrap().into_iter().map(|s| s.mesh) {
				let s = m.lit.as_ref().unwrap().sphere;
				let far = m.positions.iter().map(|p| ((p[0] - s[0]).powi(2) + (p[1] - s[1]).powi(2) + (p[2] - s[2]).powi(2)).sqrt()).fold(0.0f32, f32::max);
				n += 1;
				bad += (far > s[3] * 1.01 + 0.1) as usize;
			}
			println!("level {}: {bad} of {n} shrub spheres do not enclose their vertices", info.id);
			assert_eq!(bad, 0);
		}
	}

	#[test]
	fn veldin_shrub_colour() {
		let mut disc = iso::Disc::open(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let lights = crate::gameplay::lighting(&level).unwrap().lights;
		let shrubs = lit_instances(&level, &lights).unwrap();
		// vertex_colours.md "Veldin checks": shrub 0, first vertex (39.20, 36.13, 30.95, 128).
		let c = shrubs[0].mesh.colours[0];
		println!("shrub 0 vertex 0 {c:?}");
		assert!((c[0] - 39.20).abs() < 0.5 && (c[1] - 36.13).abs() < 0.5 && (c[2] - 30.95).abs() < 0.5 && c[3] == 128.0);
	}

	fn cull(dist: f32, bb: Option<u8>) -> ShrubCull {
		ShrubCull { centre: [0.0; 3], radius: 1.5, dist, bb }
	}

	/// Retail walks one run per class from the first record whose class changes, so records are grouped by class: the class
	/// load slots never decrease in record order (BRIDGE-27 C362).
	#[test]
	fn class_slots_are_grouped_on_both_levels() {
		let mut disc = iso::Disc::open(&PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in &levels[..2] {
			let level = Level::load(&mut disc, info).unwrap();
			let lights = crate::gameplay::lighting(&level).unwrap().lights;
			let shrubs = lit_instances(&level, &lights).unwrap();
			let slots: Vec<u32> = shrubs.iter().map(|s| s.class_slot).collect();
			let grouped = slots.windows(2).all(|w| w[0] <= w[1]);
			let with_bb = shrubs.iter().filter(|s| s.bb.is_some()).count();
			let zero_b = shrubs.iter().filter(|s| s.cull.bb == Some(0)).count();
			println!("level {}: {} shrubs, slots non-decreasing {grouped}, {with_bb} with a billboard, {zero_b} with B 0", info.id, shrubs.len());
			assert!(grouped);
			for s in shrubs.iter().filter_map(|s| s.bb) {
				assert!(s.half_w.is_finite() && s.h.is_finite() && s.z0.is_finite() && s.h >= 0.0);
			}
		}
	}
}
