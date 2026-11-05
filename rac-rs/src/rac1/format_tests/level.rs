//! Disc tests of formats/level.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod particle_tests {
	use crate::level::*;
	use crate::bytes::i16_at;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::iso::Disc;
	use crate::wad;
	use crate::toc;

	/// Veldin: 40 particle textures, all 32x32; the bolt sparkle def (type 0x35) names one of them.
	#[test]
	fn particle_tables() {
		let mut disc = crate::iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = crate::toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		let t = l.particle_textures().unwrap();
		assert_eq!(t.len(), 40);
		assert!(t.iter().all(|t| t.as_ref().is_some_and(|t| (t.w, t.h) == (32, 32))));
		let d = l.particle_def_textures().unwrap();
		eprintln!("defs {:?}", d);
		assert!((d[0x35] as usize) < t.len());
	}
}

mod mip_tests {
	use crate::level::*;
	use crate::bytes::i16_at;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::iso::Disc;
	use crate::wad;
	use crate::toc;

	fn level(i: usize) -> Level {
		let mut disc = crate::iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = crate::toc::read_levels(&mut disc).unwrap();
		Level::load(&mut disc, &levels[i]).unwrap()
	}

	fn opaque(t: &Texture) -> usize {
		t.rgba.iter().filter(|p| p[3] == 255).count()
	}

	/// REN-CULL-SHRUB-BBTEX tests (1)-(5): Novalis chains of 0x7e / 0x8d / 0xe7 and the Veldin count.
	#[test]
	fn shrub_billboard_chains() {
		let t = level(1).shrub_billboard_textures().unwrap();
		let mut keys: Vec<u32> = t.keys().copied().collect();
		keys.sort();
		assert_eq!(keys, vec![0x7e, 0x8d, 0xe7]);
		let sizes = |c: &Vec<Texture>| c.iter().map(|t| t.w).collect::<Vec<_>>();
		assert_eq!(sizes(&t[&0x7e]), vec![32, 16, 8]);
		assert_eq!(sizes(&t[&0x8d]), vec![32, 16, 8]);
		assert_eq!(sizes(&t[&0xe7]), vec![64, 32, 16, 8]);
		let e7 = &t[&0xe7][0];
		assert_eq!(e7.rgba[0], [255, 56, 56, 0]);
		assert_eq!(e7.rgba[32 * 64 + 32], [255, 167, 167, 255]);
		assert_eq!(opaque(e7), 2364);
		assert_eq!(e7.rgba.iter().filter(|p| p[3] == 0).count(), 536);
		let c = &t[&0x7e][0];
		assert_eq!(c.rgba[0], [0, 0, 0, 0]);
		assert_eq!(c.rgba[16 * 32 + 16], [255, 141, 52, 255]);
		assert_eq!(c.rgba.iter().position(|p| p[3] > 0), Some(3 * 32 + 13));
		assert_eq!(c.rgba[3 * 32 + 13], [255, 217, 164, 255]);
		assert_eq!(opaque(c), 232);
		assert_eq!(t[&0x7e][1].rgba[8 * 16 + 8], [255, 127, 52, 255]);
		assert_eq!(opaque(&t[&0x7e][1]), 69);
		let d = &t[&0x8d][0];
		assert_eq!(d.rgba[16 * 32 + 16], [197, 162, 7, 255]);
		assert_eq!(d.rgba.iter().position(|p| p[3] == 255), Some(13));
		assert_eq!(d.rgba[13], [142, 125, 77, 255]);
		assert_eq!(opaque(d), 160);
		let v = level(0).shrub_billboard_textures().unwrap();
		assert_eq!(v.len(), 13);
		assert_eq!(v[&0x1d2].len(), 4);
		assert_eq!((v[&0x1d2][0].w, v[&0x1d2][0].h), (128, 128));
	}

	/// REN-TEX-RESIDENCY test (1): the Novalis terrain chains.
	#[test]
	fn terrain_chains() {
		let l = level(1);
		let t = l.terrain_mips().unwrap();
		assert_eq!(t.iter().filter(|t| t.is_some()).count(), 87);
		let e0 = t[0].as_ref().unwrap();
		assert_eq!(e0.iter().map(|t| t.w).collect::<Vec<_>>(), vec![128, 64, 32, 16]);
		// Level 0 is the base `textures()` returns, bit for bit.
		let base = l.textures(TextureGroup::Terrain).unwrap();
		for (a, b) in t.iter().zip(&base) {
			assert_eq!(a.as_ref().map(|c| &c[0].rgba), b.as_ref().map(|c| &c.rgba));
		}
	}

	/// NOV-35b test 3: Novalis metal textures: chrome 128 x 128 from gs 0x400 with its CLUT at gs 0, glass 64 x 64.
	#[test]
	fn metal_textures_novalis() {
		let l = level(1);
		assert_eq!((u32_at(&l.index, 0x90).unwrap(), u32_at(&l.index, 0x94).unwrap()), (0x400, 0));
		let gs = l.sections[3].as_deref().unwrap();
		let [chrome, glass] = l.metal_textures().unwrap();
		assert_eq!((chrome.w, chrome.h, glass.w, glass.h), (128, 128, 64, 64));
		assert_eq!(chrome.rgba[0], Texture::new(1, 1, &gs[0x400..0x401], &gs[..1024]).unwrap().rgba[0]);
	}
}

mod tests {
	use crate::level::*;
	use crate::{gameplay, iso, shrubs, terrain, ties};
	use crate::bytes::i16_at;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::iso::Disc;
	use crate::wad;
	use crate::toc;

	/// ARI-1 (decomp/notes/aridia_presentation.md): Aridia loads through the generic loader and keeps its overlay image.
	#[test]
	fn aridia_loads() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[2]).unwrap();
		assert_eq!(l.id, 2);
		let ov = l.sections[0].as_ref().unwrap();
		assert_eq!(ov.len(), 1_704_508);
		assert_eq!(u32_at(ov, 0).unwrap(), 0x0015ef00);
		let lt = gameplay::lighting(&l).unwrap();
		assert_eq!(lt.lights.len(), 1);
		assert_eq!(ties::lit_instances(&l, &lt.lights).unwrap().len(), 3751);
		assert_eq!(shrubs::lit_instances(&l, &lt.lights).unwrap().len(), 1199);
		assert_eq!(gameplay::moby_instances(&l).unwrap().len(), 775);
		let (_, lods) = terrain::terrain_lods(l.terrain_block().unwrap(), None, None).unwrap();
		assert!(!lods.is_empty());
	}
}
