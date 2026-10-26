//! The PCSX2 names of a level's textures, and their lookup in the pack index. Each mip level has a single-level name (what the real
//! USA pack uses); with `MipNames::Chain` / `Both` the chain names (level k up to the last level in one hash) are added.

use super::hash::{self, Level as Lv};
use super::index::{PackFile, PackIndex};
use super::mods::MipNames;
use super::name::TexName;
use super::source::TexSource;
use super::xmap::{self, XMap};
use crate::level::TextureGroup;

/// Which hash produced a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
	Single,
	Chain,
}

/// One candidate name of a texture: mip level `level` of table entry (`group`, `index`).
#[derive(Clone, Copy)]
pub struct LevelName {
	pub group: TextureGroup,
	pub index: usize,
	pub level: u8,
	pub kind: Kind,
	pub name: TexName,
}

/// The names of one texture, level 0 first; per level the single name, then (when asked for and the texture has more levels
/// below) the chain name. Level k's TW / TH come from that level's size.
pub fn texture_names(t: &TexSource, mode: MipNames) -> Vec<LevelName> {
	let clut = hash::clut_hash(t.pal);
	let mut out = Vec::new();
	for (k, l) in t.levels.iter().enumerate() {
		let bits = hash::bits_t8(l.w, l.h);
		let mut push = |kind, tex0| out.push(LevelName { group: t.group, index: t.index, level: k as u8, kind, name: TexName::palette(tex0, clut, bits) });
		if mode != MipNames::Chain || t.levels.len() == 1 {
			push(Kind::Single, hash::tex_hash_single(*l));
		}
		if mode != MipNames::Single && t.levels.len() > 1 && k + 1 < t.levels.len() {
			push(Kind::Chain, hash::tex_hash_chain(&t.levels, k));
		}
	}
	out
}

/// All names of a level's textures.
pub fn level_names(srcs: &[TexSource], mode: MipNames) -> Vec<LevelName> {
	srcs.iter().flat_map(|t| texture_names(t, mode)).collect()
}

/// What the active packs provide for one level.
#[derive(Default)]
pub struct LevelHits {
	pub textures: usize,
	/// Distinct names computed.
	pub names: usize,
	/// Distinct names that have a file.
	pub hit_names: usize,
	/// Of those, the names whose file is named for their PAL <-> USA twin (`xmap`).
	pub mapped_names: usize,
	/// Textures with a file for at least one of their levels.
	pub textures_hit: usize,
	/// Every (level name, file) hit, texture order then level order.
	pub hits: Vec<(LevelName, PackFile)>,
}

/// Looks the names of `srcs` up in `index` (the same name used by several textures counts once in `names` / `hit_names`). A name
/// the pack lacks is tried under its twin on the other disc (`xmap`: a USA pack on the PAL disc and the other way round).
pub fn resolve(srcs: &[TexSource], mode: MipNames, index: &PackIndex) -> LevelHits {
	resolve_with(srcs, mode, index, xmap::shipped())
}

/// `resolve` with an explicit name map (tests; an empty `XMap` gives the direct lookup).
pub fn resolve_with(srcs: &[TexSource], mode: MipNames, index: &PackIndex, map: &XMap) -> LevelHits {
	let mut r = LevelHits { textures: srcs.len(), ..Default::default() };
	let mut seen = std::collections::HashSet::new();
	let mut seen_hit = std::collections::HashSet::new();
	let mut seen_map = std::collections::HashSet::new();
	for t in srcs {
		let mut any = false;
		for ln in texture_names(t, mode) {
			seen.insert(ln.name);
			let direct = index.get(&ln.name);
			let Some(f) = direct.or_else(|| map.twins(&ln.name).iter().find_map(|w| index.get(w))) else { continue };
			seen_hit.insert(ln.name);
			if direct.is_none() {
				seen_map.insert(ln.name);
			}
			r.hits.push((ln, f.clone()));
			any = true;
		}
		r.textures_hit += any as usize;
	}
	(r.names, r.hit_names, r.mapped_names) = (seen.len(), seen_hit.len(), seen_map.len());
	r
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::texpack::index::PackFile;
	use crate::texpack::name::Ext;

	fn pat(n: usize, mul: usize, add: usize) -> Vec<u8> {
		(0..n).map(|i| ((i * mul + add) & 255) as u8).collect()
	}

	/// The synthetic 4-level texture of the hash golden test, with its CLUT.
	fn synth<'a>(a: &'a [u8], b: &'a [u8], c: &'a [u8], d: &'a [u8], pal: &'a [u8]) -> TexSource<'a> {
		TexSource { group: TextureGroup::Tie, index: 7, w: 64, h: 64, pal, levels: vec![Lv { px: a, w: 64, h: 64 }, Lv { px: b, w: 32, h: 32 }, Lv { px: c, w: 16, h: 16 }, Lv { px: d, w: 8, h: 8 }] }
	}

	#[test]
	fn names_per_mode() {
		let (a, b, c, d, pal) = (pat(64 * 64, 7, 3), pat(32 * 32, 5, 1), pat(16 * 16, 3, 9), pat(8 * 8, 11, 2), pat(1024, 13, 5));
		let t = synth(&a, &b, &c, &d, &pal);
		let single = texture_names(&t, MipNames::Single);
		assert_eq!(single.len(), 4);
		assert_eq!(single[0].name.stem(true), "fcebcc02d3f221f0-67092c9a05cb2660-00001993");
		assert_eq!(single[3].name.bits, 0x13 | 3 << 6 | 3 << 10, "level 3 is 8x8");
		let chain = texture_names(&t, MipNames::Chain);
		assert_eq!(chain.len(), 3, "the last level has no chain below it");
		assert_eq!(chain[0].name.stem(true), "be7f4b07be8ae043-67092c9a05cb2660-00001993");
		assert_eq!(texture_names(&t, MipNames::Both).len(), 7);
	}

	fn file(p: &str) -> PackFile {
		PackFile { path: p.into(), ext: Ext::Dds, mod_idx: 0 }
	}

	#[test]
	fn resolve_tries_the_twin_name_in_both_directions() {
		let (a, b, c, d, pal) = (pat(64 * 64, 7, 3), pat(32 * 32, 5, 1), pat(16 * 16, 3, 9), pat(8 * 8, 11, 2), pat(1024, 13, 5));
		let t = synth(&a, &b, &c, &d, &pal);
		let own = texture_names(&t, MipNames::Single);
		let other = TexName::palette(0x1234, 0x5678, own[0].name.bits);
		let mut map = XMap::default();
		// This disc's name is the PAL side of the pair (a USA pack on the PAL disc) ...
		map.add(own[0].name, other);
		let mut ix = PackIndex::new();
		ix.insert(other, file("twin.dds"));
		let r = resolve_with(std::slice::from_ref(&t), MipNames::Single, &ix, &map);
		assert_eq!((r.hit_names, r.mapped_names, r.textures_hit, r.hits.len()), (1, 1, 1, 1));
		assert_eq!((r.hits[0].0.level, r.hits[0].1.path.to_str()), (0, Some("twin.dds")));
		let r = resolve_with(std::slice::from_ref(&t), MipNames::Single, &ix, &XMap::default());
		assert_eq!((r.hit_names, r.textures_hit), (0, 0), "no row, no hit");
		// ... or the USA side (a PAL-named pack on the USA disc): the same symmetric table.
		let mut map = XMap::default();
		map.add(other, own[0].name);
		let mut ix = PackIndex::new();
		ix.insert(other, file("pal.dds"));
		let r = resolve_with(std::slice::from_ref(&t), MipNames::Single, &ix, &map);
		assert_eq!((r.hit_names, r.mapped_names, r.hits[0].1.path.to_str()), (1, 1, Some("pal.dds")));
	}

	#[test]
	fn a_direct_hit_beats_the_twin_and_needs_no_row() {
		let (a, b, c, d, pal) = (pat(64 * 64, 7, 3), pat(32 * 32, 5, 1), pat(16 * 16, 3, 9), pat(8 * 8, 11, 2), pat(1024, 13, 5));
		let t = synth(&a, &b, &c, &d, &pal);
		let own = texture_names(&t, MipNames::Single);
		let other = TexName::palette(0x1234, 0x5678, own[0].name.bits);
		let mut map = XMap::default();
		map.add(own[0].name, other);
		let mut ix = PackIndex::new();
		ix.insert(other, file("twin.dds"));
		ix.insert(own[0].name, file("direct.dds"));
		ix.insert(own[3].name, file("l3.dds"));
		let r = resolve_with(std::slice::from_ref(&t), MipNames::Single, &ix, &map);
		assert_eq!((r.hit_names, r.mapped_names, r.hits.len()), (2, 0, 2));
		assert_eq!((r.hits[0].1.path.to_str(), r.hits[1].0.level), (Some("direct.dds"), 3));
	}

	#[test]
	fn resolve_counts_hits() {
		let (a, b, c, d, pal) = (pat(64 * 64, 7, 3), pat(32 * 32, 5, 1), pat(16 * 16, 3, 9), pat(8 * 8, 11, 2), pat(1024, 13, 5));
		let t = synth(&a, &b, &c, &d, &pal);
		let names = texture_names(&t, MipNames::Single);
		let mut ix = PackIndex::new();
		for ln in [names[0], names[2]] {
			ix.insert(ln.name, PackFile { path: "x.dds".into(), ext: Ext::Dds, mod_idx: 0 });
		}
		let r = resolve(std::slice::from_ref(&t), MipNames::Single, &ix);
		assert_eq!((r.textures, r.names, r.hit_names, r.textures_hit, r.hits.len()), (1, 4, 2, 1, 2));
		assert_eq!(r.hits[1].0.level, 2);
	}
}
