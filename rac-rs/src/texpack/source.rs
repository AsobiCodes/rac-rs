//! The raw PSMT8 texture tables of a level (terrain 0x30, moby 0x38, tie 0x40, shrub 0x48): index bytes of every mip level and the
//! CT32 CLUT, the inputs of the PCSX2 hashes. Same table walk as `Level::mip_chains` (mips 0 and 1 follow each other in the core
//! pixels, mips 2 and 3 sit in the GS RAM section at TBP2 / TBP3), but without expanding to RGBA.

use super::hash::Level as Lv;
use crate::bytes::{i16_at, span, u16_at, u32_at, R};
use crate::level::{Level, TextureGroup, TEXTURE_BASE};

pub const GROUPS: [TextureGroup; 4] = [TextureGroup::Terrain, TextureGroup::Moby, TextureGroup::Tie, TextureGroup::Shrub];

/// One table entry with its mip levels.
pub struct TexSource<'a> {
	pub group: TextureGroup,
	/// Index in the group's table (the same index `texture_key` uses).
	pub index: usize,
	/// Level 0 size.
	pub w: usize,
	pub h: usize,
	/// The CT32 CSM1 palette, 1024 bytes.
	pub pal: &'a [u8],
	/// Mip levels from the base down (1 to 4).
	pub levels: Vec<Lv<'a>>,
}

/// Every texture of every group of `l`. An entry whose data lies outside its section is skipped and counted in the second value.
pub fn level_sources(l: &Level) -> R<(Vec<TexSource<'_>>, usize)> {
	let gs = l.sections[3].as_deref().ok_or("no gs_ram section")?;
	let base = u32_at(&l.index, TEXTURE_BASE)? as usize;
	let (mut out, mut skipped) = (Vec::new(), 0);
	for group in GROUPS {
		for (index, e) in l.table(group as usize, 16)?.into_iter().enumerate() {
			let (pixels, w, h) = (u32_at(e, 0)? as usize, u16_at(e, 4)? as usize, u16_at(e, 6)? as usize);
			let (mips, cbp) = (i16_at(e, 8)? as usize, u16_at(e, 10)? as usize);
			if w == 0 && h == 0 {
				continue;
			}
			match one(l, gs, base + pixels, e, (w, h), (mips, cbp)) {
				Ok((pal, levels)) => out.push(TexSource { group, index, w, h, pal, levels }),
				Err(_) => skipped += 1,
			}
		}
	}
	Ok((out, skipped))
}

type Entry<'a> = (&'a [u8], Vec<Lv<'a>>);

fn one<'a>(l: &'a Level, gs: &'a [u8], start: usize, e: &[u8], (w, h): (usize, usize), (mips, cbp): (usize, usize)) -> R<Entry<'a>> {
	let pal = span(gs, cbp * 256, 1024)?;
	let mut levels = Vec::new();
	let mut at = start;
	for k in 0..mips.clamp(1, 4) {
		let (mw, mh) = ((w >> k).max(1), (h >> k).max(1));
		let px = if k < 2 {
			let p = span(&l.core, at, mw * mh)?;
			at += mw * mh;
			p
		} else {
			span(gs, u16_at(e, 8 + 2 * k)? as usize * 256, mw * mh)?
		};
		levels.push(Lv { px, w: mw, h: mh });
	}
	Ok((pal, levels))
}
