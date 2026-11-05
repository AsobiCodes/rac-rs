//! HUD graphics: each level's hud_header + five hud banks (data range sections 4..9).
//!
//! Header (u32 words, offsets from the header start; layout read from the boot code's bank linker and
//! icon lookup, then checked against the files):
//!   +0x00 u16 icon count, u16 frame count
//!   +0x04 icon rows   (8 bytes: u16 icon id, u16 frame count, u16 first frame, u16 pad), end id 0xffff
//!   +0x08 frames      (4 bytes: u16 palette index, u16 texture index)
//!   +0x0c palettes    (8 bytes: u32 bank offset | 0x80000000, 4 unused)
//!   +0x10 textures    (8 bytes: u32 bank offset | 0x80000000, u16 unused, u8 log2 w, u8 log2 h)
//!   +0x14 5 words (+0x18..) cumulative palette counts per bank, +0x34 cumulative texture counts per bank
//!         (bank b owns entries [limit[b-1], limit[b]) with limit[-1] = 0; the palette array starts one
//!         word early, so bank 0 has no palettes)
//!   +0x54 bank sizes (words 21..25)
//! A texture is PSMT8 (one byte per pixel, linear), a palette is 256 RGBA32 colours in CSM1 order.

use crate::bytes::{span, u16_at, u32_at, R};
use crate::level::{Level, Texture};

pub const BANKS: usize = 5;
const FIRST_BANK_SECTION: usize = 5;
const HEADER_SECTION: usize = 4;

/// One HUD image: a texture drawn with one palette.
pub struct Sprite {
	pub tex: Texture,
	/// The PCSX2 name of this picture (texture + palette), only when a texture pack is active (`naming`; the engine's `texpack::front_hd` looks it up).
	pub name: Option<crate::texname::TexName>,
	/// Its name when the game keeps the indices as PSMT8H instead (some HUD banks: the classic vendor's item icons are drawn as both
	/// [C: RAC_TEXNAMES, vendor_0093]); a pack made in PCSX2 has whichever one the game drew. Only with `naming`, like `name`.
	pub name_h: Option<crate::texname::TexName>,
}

#[derive(Clone, Copy, Debug)]
pub struct Icon {
	pub id: u16,
	pub first: usize,
	pub count: usize,
}

pub struct Hud {
	pub icons: Vec<Icon>,
	/// One entry per frame; None where a bank is missing or the data does not fit.
	pub frames: Vec<Option<Sprite>>,
}


fn bank_of(limits: &[usize], i: usize) -> Option<usize> {
	(0..BANKS).find(|&b| i >= if b == 0 { 0 } else { limits[b - 1] } && i < limits[b])
}

pub fn palette_rgba(p: &[u8]) -> Vec<[u8; 4]> {
	(0..256)
		.map(|i| {
			let j = (i & !0x18) | ((i & 8) << 1) | ((i & 0x10) >> 1);
			let c = &p[j * 4..j * 4 + 4];
			[c[0], c[1], c[2], (c[3] as u16 * 2).min(255) as u8]
		})
		.collect()
}

/// The HUD set one level loads (bolt counter, nanotech, weapon and gadget icons, ...). `naming`: give each sprite its PCSX2 name
/// (a texture pack is active: the engine's `texpack::hudrepl::enabled()`).
pub fn hud_sprites(level: &Level, naming: bool) -> R<Hud> {
	let h = level.sections[HEADER_SECTION].as_deref().ok_or("no hud_header")?;
	let banks: Vec<Option<&[u8]>> = (0..BANKS).map(|b| level.sections[FIRST_BANK_SECTION + b].as_deref()).collect();
	build(h, &banks, naming)
}

pub fn build(h: &[u8], banks: &[Option<&[u8]>], naming: bool) -> R<Hud> {
	let nicons = u16_at(h, 0)? as usize;
	let nframes = u16_at(h, 2)? as usize;
	let (t_icons, t_frames, t_pals, t_texs) = (
		u32_at(h, 4)? as usize,
		u32_at(h, 8)? as usize,
		u32_at(h, 12)? as usize,
		u32_at(h, 16)? as usize,
	);
	let word = |i: usize| u32_at(h, i * 4).map(|v| v as usize);
	let pal_limits = [word(5)?, word(6)?, word(7)?, word(8)?, word(9)?];
	let tex_limits = [word(13)?, word(14)?, word(15)?, word(16)?, word(17)?];
	let npals = *pal_limits.iter().max().unwrap();
	let ntexs = *tex_limits.iter().max().unwrap();

	let mut icons = Vec::new();
	for i in 0..nicons + 1 {
		let r = span(h, t_icons + i * 8, 8)?;
		let id = u16_at(r, 0)?;
		if id == 0xffff {
			break;
		}
		icons.push(Icon { id, count: u16_at(r, 2)? as usize, first: u16_at(r, 4)? as usize });
	}

	let bank = |b: usize| banks[b];
	// Names only with a texture pack active: without one the sprites are exactly what they were.
	let palette = |i: usize| -> Option<(Vec<[u8; 4]>, u64)> {
		let b = bank_of(&pal_limits, i)?;
		let e = span(h, t_pals + i * 8, 8).ok()?;
		let off = (u32_at(e, 0).ok()? & 0x7fff_ffff) as usize;
		let raw = span(bank(b)?, off, 1024).ok()?;
		Some((palette_rgba(raw), if naming { crate::texhash::clut_hash(raw) } else { 0 }))
	};
	let texture = |i: usize| -> Option<(usize, usize, Vec<u8>)> {
		let b = bank_of(&tex_limits, i)?;
		let e = span(h, t_texs + i * 8, 8).ok()?;
		let off = (u32_at(e, 0).ok()? & 0x7fff_ffff) as usize;
		let (lw, lh) = (e[6] as usize, e[7] as usize);
		let (w, hh) = (1usize << lw, 1usize << lh);
		let need = ((hh + 63) / 64 * 64) * ((w / 128).max(1) * 128).max(w);
		let raw = bank(b)?.get(off..)?;
		Ok::<_, ()>(()).ok()?;
		Some((w, hh, { let _ = need; raw[..(w*hh).min(raw.len())].to_vec() }))
	};

	let pals: Vec<Option<(Vec<[u8; 4]>, u64)>> = (0..npals).map(palette).collect();
	let texs: Vec<Option<(usize, usize, Vec<u8>)>> = (0..ntexs).map(texture).collect();
	let mut frames = Vec::new();
	let mut tex_hash: std::collections::HashMap<usize, u64> = std::collections::HashMap::new();
	for f in 0..nframes {
		let e = span(h, t_frames + f * 4, 4)?;
		let (p, t) = (u16_at(e, 0)? as usize, u16_at(e, 2)? as usize);
		let s = match (pals.get(p).and_then(|x| x.as_ref()), texs.get(t).and_then(|x| x.as_ref())) {
			(Some((pal, clut)), Some((w, hh, px))) => {
				let named = naming && px.len() == w * hh;
				let name = named.then(|| {
					let t0 = *tex_hash.entry(t).or_insert_with(|| crate::texhash::tex_hash_single(crate::texhash::Level { px, w: *w, h: *hh }));
					crate::texname::TexName::palette(t0, *clut, crate::texhash::bits_t8(*w, *hh))
				});
				let name_h = named.then(|| {
					let t0 = crate::texhash::tex_hash_t8h(crate::texhash::Level { px, w: *w, h: *hh });
					crate::texname::TexName::palette(t0, *clut, crate::texhash::bits_t8h(*w, *hh))
				});
				Some(Sprite { tex: Texture { w: *w, h: *hh, rgba: px.iter().map(|&v| pal[v as usize]).collect() }, name, name_h })
			}
			_ => None,
		};
		frames.push(s);
	}
	Ok(Hud { icons, frames })
}


/// One font glyph: a 16x16 cell at (u, v) in the 256x128 sheet, drawn `dy` lower, advancing `adv`.
#[derive(Clone, Copy, Debug)]
pub struct Glyph {
	pub u: u16,
	pub v: u16,
	pub dy: i16,
	pub adv: i16,
}

pub struct Font {
	pub tex: Texture,
	/// Indexed by character code (232 entries; codes below 0x20 are 24-wide button glyphs).
	pub glyphs: Vec<Glyph>,
}


/// The three text fonts (core fx_textures records 1..=3, 256x128 PSMT8), paired with `glyphs`
/// from `font_glyph_tables`. Font ids in the game are 1-based, so font 1 is element 0.
pub fn level_fonts(level: &Level, glyphs: &[Vec<Glyph>]) -> R<Vec<Font>> {
	Ok(level_fonts_raw(level)?
		.into_iter()
		.zip(glyphs)
		.map(|((px, colours), g)| Font { tex: Texture { w: 256, h: 128, rgba: px.iter().map(|&v| colours[v as usize]).collect() }, glyphs: g.clone() })
		.collect())
}

/// The three text fonts as the GS holds them: the 256x128 PSMT8 index bytes and the 256-colour palette in index order (`palette_rgba`),
/// for the Button glyphs option (retail/glyphs.rs), which needs the sheet's indices to recognise its upload and the palette to
/// quantise its pictures into.
pub fn level_fonts_raw(level: &Level) -> R<Vec<(Vec<u8>, Vec<[u8; 4]>)>> {
	let count = u32_at(&level.index, 0x58)? as usize;
	let table = u32_at(&level.index, 0x5c)? as usize;
	let base = u32_at(&level.index, 0x68)? as usize;
	let mut out = Vec::new();
	for k in 1..=3usize {
		if k >= count {
			return Err("fx texture table too short".into());
		}
		let r = span(&level.index, table + k * 16, 16)?;
		let (pal, pix, w, h) = (u32_at(r, 0)? as usize, u32_at(r, 4)? as usize, u32_at(r, 8)? as usize, u32_at(r, 12)? as usize);
		if (w, h) != (256, 128) {
			return Err(format!("font record {k} is {w}x{h}"));
		}
		out.push((span(&level.core, base + pix, w * h)?.to_vec(), palette_rgba(span(&level.core, base + pal, 1024)?)));
	}
	Ok(out)
}
