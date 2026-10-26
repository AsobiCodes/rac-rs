//! PCSX2 names of the PSMT8 textures that live outside the four world tables (lane texw): the fx table (index 0x58 / 0x5c / 0x68:
//! effects, shadows, glows, water sheets, the ocean's animation frames), the particle textures (index table 0x50, 32x32), the
//! two metal-packet textures (chrome 128x128, glass 64x64), the sky shells' textures, and the textures of the ship and warp
//! bundles. Same rules as `names.rs`: the TEX0 hash is XXH3 over the index bytes of the level as the GS holds them, the CLUT
//! hash is over the palette in logical order, `bits` is PSM / TW / TH. These textures have no mip levels of their own.
//! `extra_hd.rs` swaps the decoded pack images in.

use super::hash::{self, Level as Lv};
use super::name::TexName;
use crate::bytes::{span, u16_at, u32_at};
use crate::level::{Level, CHROME_KEY, GLASS_KEY};

/// Texture keys of the extra textures (the world groups use 0..3 << 16, the sky 4 << 16 | i): fx record `k` is `FX_KEY | k`,
/// particle texture `n` is `PART_KEY | n`; chrome and glass keep `CHROME_KEY` / `GLASS_KEY`. The warp bundle's textures
/// (moby table, fx table) are `WARP_MOBY_KEY | i` / `WARP_FX_KEY | i`; its sky keeps the sky keys.
pub const FX_KEY: u32 = 8 << 16;
pub const PART_KEY: u32 = 9 << 16;
pub const WARP_MOBY_KEY: u32 = 1 << 24;
pub const WARP_FX_KEY: u32 = 2 << 24;

/// Most fx records read (same bound as the fx upload in `viewer/app.rs`).
const FX_MAX: usize = 0x400;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	Fx,
	Particle,
	Chrome,
	Glass,
	Sky,
	Ship,
	Warp,
}

/// One PSMT8 texture with its CT32 CSM1 palette (1024 bytes, storage order) and row-major index bytes.
pub struct ExtraSource<'a> {
	pub kind: Kind,
	pub key: u32,
	pub w: usize,
	pub h: usize,
	pub pal: &'a [u8],
	pub px: &'a [u8],
}

impl ExtraSource<'_> {
	/// The name PCSX2 gives this texture (single level, no region).
	pub fn name(&self) -> TexName {
		TexName::palette(self.tex0(), hash::clut_hash(self.pal), hash::bits_t8(self.w, self.h))
	}

	/// The texel hash alone (the key of the palette-animation lookup).
	pub fn tex0(&self) -> u64 {
		hash::tex_hash_single(Lv { px: self.px, w: self.w, h: self.h })
	}
}

/// Every extra texture of the level `l` (fx table, particles, metal, sky); an entry whose data lies outside its section is skipped.
pub fn extra_sources(l: &Level) -> Vec<ExtraSource<'_>> {
	let mut out = Vec::new();
	fx_sources(l, &mut out);
	particle_sources(l, &mut out);
	metal_sources(l, &mut out);
	if let Ok(Some(block)) = crate::sky::block_of(l) {
		sky_sources(block, &mut out);
	}
	out
}

fn fx_sources<'a>(l: &'a Level, out: &mut Vec<ExtraSource<'a>>) {
	let (Ok(count), Ok(table), Ok(base)) = (u32_at(&l.index, 0x58), u32_at(&l.index, 0x5c), u32_at(&l.index, 0x68)) else { return };
	for k in 0..(count as usize).min(FX_MAX) {
		let Ok(r) = span(&l.index, table as usize + k * 16, 16) else { break };
		let (Ok(pal), Ok(pix), Ok(w), Ok(h)) = (u32_at(r, 0), u32_at(r, 4), u32_at(r, 8), u32_at(r, 12)) else { continue };
		let (base, w, h) = (base as usize, w as usize, h as usize);
		if let (Ok(pal), Ok(px)) = (span(&l.core, base + pal as usize, 1024), span(&l.core, base + pix as usize, w * h)) {
			if w > 0 && h > 0 {
				out.push(ExtraSource { kind: Kind::Fx, key: FX_KEY | k as u32, w, h, pal, px });
			}
		}
	}
}

fn particle_sources<'a>(l: &'a Level, out: &mut Vec<ExtraSource<'a>>) {
	let (Ok(base), Ok(rows)) = (u32_at(&l.index, 0x64), l.table(0x50, 16)) else { return };
	for (n, e) in rows.into_iter().enumerate() {
		let (Ok(pal), Ok(pix), Ok(side)) = (u32_at(e, 0), u32_at(e, 8), u32_at(e, 12)) else { continue };
		let (base, side) = (base as usize, side as usize);
		if let (Ok(pal), Ok(px)) = (span(&l.core, base + pal as usize, 1024), span(&l.core, base + pix as usize, side * side)) {
			if side > 0 {
				out.push(ExtraSource { kind: Kind::Particle, key: PART_KEY | n as u32, w: side, h: side, pal, px });
			}
		}
	}
}

pub(super) fn metal_sources<'a>(l: &'a Level, out: &mut Vec<ExtraSource<'a>>) {
	let Some(gs) = l.sections[3].as_deref() else { return };
	for (kind, key, at, side) in [(Kind::Chrome, CHROME_KEY, 0x90, 128usize), (Kind::Glass, GLASS_KEY, 0x98, 64)] {
		let (Ok(pixels), Ok(clut)) = (u32_at(&l.index, at), u32_at(&l.index, at + 4)) else { continue };
		if let (Ok(px), Ok(pal)) = (span(gs, pixels as usize, side * side), span(gs, clut as usize, 1024)) {
			out.push(ExtraSource { kind, key, w: side, h: side, pal, px });
		}
	}
}

/// The textures of a sky block (`sky::raw_textures`), keys `4 << 16 | i`.
pub fn sky_sources<'a>(block: &'a [u8], out: &mut Vec<ExtraSource<'a>>) {
	for (key, w, h, pal, px) in crate::sky::raw_textures(block).unwrap_or_default() {
		out.push(ExtraSource { kind: Kind::Sky, key, w, h, pal, px });
	}
}

/// The two textures of a ship bundle (`scene::ship_assets_of`): 256x256 then 128x128, keys 0 and 1. The pixels are at +0x430 and the
/// palette at +0x30 of the section the header words 1 / 3 point to.
pub fn ship_sources(bundle: &[u8]) -> Vec<ExtraSource<'_>> {
	let mut out = Vec::new();
	for (k, (word, side)) in [(1usize, 256usize), (3, 128)].into_iter().enumerate() {
		let Ok(base) = u32_at(bundle, 4 * word) else { continue };
		if let (Ok(px), Ok(pal)) = (span(bundle, base as usize + 0x430, side * side), span(bundle, base as usize + 0x30, 0x400)) {
			out.push(ExtraSource { kind: Kind::Ship, key: k as u32, w: side, h: side, pal, px });
		}
	}
	out
}

/// The textures of the warp bundle that `scene::warp_assets` decodes: the moby table, the fx table and the star-fan sky. The table
/// walk is the one of `warp_assets` (header words, 16-byte records); the pair pictures (`warp_pair_textures`) are not listed.
pub fn warp_sources(bundle: &[u8]) -> Vec<ExtraSource<'_>> {
	let mut out = Vec::new();
	let w = |i: usize| u32_at(bundle, 4 * i).ok().map(|v| v as usize);
	let (Some(gs_at), Some(base)) = (w(0), w(1)) else { return out };
	let Some(gs) = bundle.get(gs_at..) else { return out };
	if let (Some(n), Some(table), Some(px0)) = (w(6), w(7), w(0xc)) {
		for i in 0..n {
			let Ok(e) = span(bundle, table + 16 * i, 16) else { break };
			let (Ok(tw), Ok(th), Ok(pal), Ok(at)) = (u16_at(e, 4), u16_at(e, 6), u16_at(e, 10), u32_at(e, 0)) else { continue };
			let (tw, th) = (tw as usize, th as usize);
			if let (true, Ok(px), Ok(pal)) = (tw > 0 && th > 0, span(bundle, base + px0 + at as usize, tw * th), span(gs, pal as usize * 256, 1024)) {
				out.push(ExtraSource { kind: Kind::Warp, key: WARP_MOBY_KEY | i as u32, w: tw, h: th, pal, px });
			}
		}
	}
	if let (Some(n), Some(table), Some(fx0)) = (w(0xa), w(0xb), w(0xe)) {
		for i in 0..n {
			let Ok(e) = span(bundle, table + 16 * i, 16) else { break };
			let (Ok(pal), Ok(pix), Ok(tw), Ok(th)) = (u32_at(e, 0), u32_at(e, 4), u32_at(e, 8), u32_at(e, 12)) else { continue };
			let (tw, th) = (tw as usize, th as usize);
			if let (true, Ok(pal), Ok(px)) = (tw > 0 && th > 0, span(bundle, base + fx0 + pal as usize, 1024), span(bundle, base + fx0 + pix as usize, tw * th)) {
				out.push(ExtraSource { kind: Kind::Warp, key: WARP_FX_KEY | i as u32, w: tw, h: th, pal, px });
			}
		}
	}
	if let (Some(a), Some(b)) = (w(0x12), w(0x13)) {
		if let Some(block) = b.checked_sub(a).and_then(|n| span(bundle, base + a, n).ok()) {
			sky_sources(block, &mut out);
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::texpack::index::{PackFile, PackIndex};
	use crate::texpack::name::Ext;
	use std::path::Path;

	const ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");

	fn pat(n: usize, mul: usize, add: usize) -> Vec<u8> {
		(0..n).map(|i| ((i * mul + add) & 255) as u8).collect()
	}

	/// The synthetic 64x64 texture of the names golden test gets the same PCSX2 name as a world texture of that content, and the
	/// same texels under another palette are found by `names_of_tex0`.
	#[test]
	fn name_golden_and_palette_lookup() {
		let (px, pal, pal2) = (pat(64 * 64, 7, 3), pat(1024, 13, 5), pat(1024, 3, 1));
		let s = ExtraSource { kind: Kind::Fx, key: FX_KEY | 5, w: 64, h: 64, pal: &pal, px: &px };
		assert_eq!(s.name().stem(true), "fcebcc02d3f221f0-67092c9a05cb2660-00001993");
		let other = ExtraSource { pal: &pal2, ..s };
		let mut ix = PackIndex::new();
		assert!(ix.names_of_tex0(other.tex0()).is_empty());
		ix.insert(other.name(), PackFile { path: "x.dds".into(), ext: Ext::Dds, mod_idx: 0 });
		assert!(ix.get(&other.name()).is_some());
		assert_eq!(ix.names_of_tex0(other.tex0()), &[other.name()], "same texels, other palette");
		assert!(ix.get(&ExtraSource { pal: &pal, px: &px, kind: Kind::Fx, key: 0, w: 64, h: 64 }.name()).is_none(), "the exact name needs the palette too");
	}

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
