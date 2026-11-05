//! Disc tests of formats/collision.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::collision::*;
	use crate::{iso, toc};
	use crate::bytes::i16_at;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::level::Level;
	use crate::physics::V3;

	#[test]
	fn veldin_collision_bounds() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in &levels {
			let l = Level::load(&mut disc, info).unwrap();
			let faces = collision(&l).unwrap();
			let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
			for f in &faces {
				for p in f.v {
					for k in 0..3 {
						lo[k] = lo[k].min(p[k]);
						hi[k] = hi[k].max(p[k]);
					}
				}
			}
			println!("{:>2} {:<27} {:>6} triangles  {lo:?}..{hi:?}", info.id, toc::LEVEL_NAMES[info.id], faces.len());
			if info.id == 0 {
				// Wrench's Veldin collision.dae bounds.
				assert!((lo[0] - 67.94).abs() < 0.01 && (hi[1] - 311.19).abs() < 0.01 && (hi[2] - 82.06).abs() < 0.01);
			}
		}
	}

	/// Cell copies (collision_and_gameplay.md "Terrain cell copies"): a copy is stored relative to its cell centre with
	/// a 10/10/12-bit offset, so a vertex over 32 m away is clamped there and the copies of a large face disagree
	/// (Novalis: by up to 0.94 m). Querying through each copy's own cell loses no hit against the union of all copies.
	#[test]
	fn cell_copies_match_the_union() {
		use crate::physics::World;
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for id in [0usize, 1] {
			let l = Level::load(&mut disc, &levels[id]).unwrap();
			let faces = collision(&l).unwrap();
			let union = World::typed(faces.iter().map(|f| (f.v, f.kind)), 4.0);
			let homed = World::homed(faces.iter().map(|f| (f.v, f.kind, Some(f.cell))), 4.0);
			let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
			for f in &faces {
				for p in f.v {
					for k in 0..3 {
						lo[k] = lo[k].min(p[k]);
						hi[k] = hi[k].max(p[k]);
					}
				}
			}
			let (mut n, mut worst, mut lost) = (0, 0f32, 0);
			let mut s = 12345u32;
			let mut rnd = || {
				s = s.wrapping_mul(1664525).wrapping_add(1013904223);
				(s >> 8) as f32 / (1u32 << 24) as f32
			};
			for _ in 0..30000 {
				let (x, y) = (lo[0] + rnd() * (hi[0] - lo[0]), lo[1] + rnd() * (hi[1] - lo[1]));
				let (a, b) = ([x, y, hi[2] + 1.0], [x + (rnd() - 0.5) * 6.0, y + (rnd() - 0.5) * 6.0, lo[2] - 1.0]);
				match (union.line(a, b, 0), homed.line(a, b, 0)) {
					(Some(u), Some(h)) => {
						n += 1;
						worst = worst.max((u.p[2] - h.p[2]).abs());
					}
					(Some(_), None) | (None, Some(_)) => lost += 1,
					_ => {}
				}
			}
			println!("level {id}: {n} common hits, worst hit z difference {worst}, hit/miss disagreements {lost}");
			assert!(lost * 1000 <= n.max(1), "{lost} of {n}");
		}
	}
}
