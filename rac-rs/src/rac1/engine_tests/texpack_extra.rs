//! Disc tests of engine/texpack/extra.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through the R&C1 pack, so
//! they live in it; moved unchanged from the engine file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::texpack::extra::*;
	use crate::texpack::index::{PackFile, PackIndex};
	use crate::texpack::name::Ext;
	use std::path::Path;
	use crate::texpack::hash;
	use crate::texpack::hash::Level as Lv;
	use crate::texpack::name::TexName;
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::level::Level;
	use crate::level::CHROME_KEY;
	use crate::level::GLASS_KEY;
	use crate::toc;

	const ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");

	/// Diagnostic: per level and kind, how many extra textures have the exact name in the local test pack, and how many more only
	/// share the texel hash (palette animation). Prints; asserts nothing about the numbers.
	#[test]
	fn extra_name_census() {
		// RAC_TEXW_CENSUS=1 turns it on, RAC_TEXW_PACK = the folder of a local PCSX2 texture pack
		let pack_dir = std::env::var_os("RAC_TEXW_PACK").unwrap_or_default();
		let (iso, pack) = (Path::new(ISO), Path::new(&pack_dir));
		if !iso.is_file() || !pack.is_dir() || std::env::var_os("RAC_TEXW_CENSUS").is_none() {
			return;
		}
		let mut d = crate::iso::Disc::open(iso).unwrap();
		let levels = crate::toc::read_levels(&mut d).unwrap();
		let mut ix = PackIndex::new();
		ix.scan_dir(pack, 0);
		let tally = |src: &[&ExtraSource]| format!("{}: exact {} texel-only {}", src.len(), src.iter().filter(|s| ix.get(&s.name()).is_some()).count(), src.iter().filter(|s| ix.get(&s.name()).is_none() && !ix.names_of_tex0(s.tex0()).is_empty()).count());
		for (i, info) in levels.iter().enumerate() {
			let l = Level::load(&mut d, info).unwrap();
			let all = extra_sources(&l);
			let mut line = format!("level {i:2}:");
			for kind in [Kind::Fx, Kind::Particle, Kind::Chrome, Kind::Glass, Kind::Sky] {
				line += &format!("  {kind:?} {};", tally(&all.iter().filter(|s| s.kind == kind).collect::<Vec<_>>()));
			}
			eprintln!("{line}");
			if std::env::var("RAC_TEXW_CENSUS_LIST").is_ok_and(|v| v.split(',').any(|x| x == i.to_string())) {
				for s in &all {
					let n = s.name();
					eprintln!("  {:?} key {:#x} {}x{} {} {} ({} under other palettes)", s.kind, s.key, s.w, s.h, n.stem(true), if ix.get(&n).is_some() { "EXACT" } else if !ix.names_of_tex0(s.tex0()).is_empty() { "texel-only" } else { "-" }, ix.names_of_tex0(s.tex0()).len());
				}
			}
		}
		for ship in 0..3 {
			if let Ok(b) = crate::scene::ship_bundle(&mut d, ship) {
				eprintln!("ship bundle {ship}: {}", tally(&ship_sources(&b).iter().collect::<Vec<_>>()));
			}
		}
		if let Ok(b) = crate::scene::warp_bundle(&mut d) {
			let all = warp_sources(&b);
			eprintln!("warp bundle: {}", tally(&all.iter().collect::<Vec<_>>()));
			for s in &all {
				eprintln!("  warp key {:#x} {}x{} {}", s.key, s.w, s.h, if ix.get(&s.name()).is_some() { "EXACT" } else { "-" });
			}
		}
	}
}
