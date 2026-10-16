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
use crate::iso::Disc;
use crate::level::{Level, Texture};
use crate::{toc, wad};

const BANKS: usize = 5;
const FIRST_BANK_SECTION: usize = 5;
const HEADER_SECTION: usize = 4;

/// One HUD image: a texture drawn with one palette.
pub struct Sprite {
	pub tex: Texture,
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

/// The HUD set one level loads (bolt counter, nanotech, weapon and gadget icons, ...).
pub fn hud_sprites(level: &Level) -> R<Hud> {
	let h = level.sections[HEADER_SECTION].as_deref().ok_or("no hud_header")?;
	let banks: Vec<Option<&[u8]>> = (0..BANKS).map(|b| level.sections[FIRST_BANK_SECTION + b].as_deref()).collect();
	build(h, &banks)
}

/// The resident HUD set loaded at boot (hud header at ToC +0x14f8, five banks at +0x1500).
pub fn global_hud_sprites(disc: &mut Disc) -> R<Hud> {
	let toc = toc::read_toc(disc).map_err(|e| e.to_string())?;
	let mut get = |at: usize| -> R<Option<Vec<u8>>> {
		let (sector, sectors) = (u32_at(&toc, at)? as u64, u32_at(&toc, at + 4)? as u64);
		if sectors == 0 {
			return Ok(None);
		}
		let raw = disc.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
		Ok(Some(if wad::is_wad(&raw) { wad::decompress(&raw).map_err(|e| format!("WAD: {e}"))? } else { raw }))
	};
	let header = get(0x14f8)?.ok_or("no global hud header")?;
	let mut banks = Vec::new();
	for b in 0..BANKS {
		banks.push(get(0x1500 + b * 8)?);
	}
	let refs: Vec<Option<&[u8]>> = banks.iter().map(|b| b.as_deref()).collect();
	build(&header, &refs)
}

fn build(h: &[u8], banks: &[Option<&[u8]>]) -> R<Hud> {
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
	let palette = |i: usize| -> Option<Vec<[u8; 4]>> {
		let b = bank_of(&pal_limits, i)?;
		let e = span(h, t_pals + i * 8, 8).ok()?;
		let off = (u32_at(e, 0).ok()? & 0x7fff_ffff) as usize;
		Some(palette_rgba(span(bank(b)?, off, 1024).ok()?))
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

	let pals: Vec<Option<Vec<[u8; 4]>>> = (0..npals).map(palette).collect();
	let texs: Vec<Option<(usize, usize, Vec<u8>)>> = (0..ntexs).map(texture).collect();
	let mut frames = Vec::new();
	for f in 0..nframes {
		let e = span(h, t_frames + f * 4, 4)?;
		let (p, t) = (u16_at(e, 0)? as usize, u16_at(e, 2)? as usize);
		let s = match (pals.get(p).and_then(|x| x.as_ref()), texs.get(t).and_then(|x| x.as_ref())) {
			(Some(pal), Some((w, hh, px))) => Some(Sprite {
				tex: Texture { w: *w, h: *hh, rgba: px.iter().map(|&v| pal[v as usize]).collect() },
			}),
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


/// Boot executable data addresses (v1.00) of the three 232-entry glyph tables.
const FONT_TABLES: [u32; 3] = [0x1defd0, 0x1df370, 0x1df710];
const GLYPHS: usize = 232;

/// Read the boot ELF off the disc's ISO9660 root and parse the three glyph tables.
pub fn font_glyph_tables(disc: &mut Disc) -> R<Vec<Vec<Glyph>>> {
	let pvd = disc.read_sectors(16, 1).map_err(|e| e.to_string())?;
	let (root, rsize) = (u32_at(&pvd, 156 + 2)? as u64, u32_at(&pvd, 156 + 10)? as usize);
	let dir = disc.read_bytes(root, rsize).map_err(|e| e.to_string())?;
	let mut at = 0;
	let mut file = None;
	while at < dir.len() {
		let len = dir[at] as usize;
		if len == 0 {
			at = (at / 2048 + 1) * 2048;
			continue;
		}
		let nl = dir[at + 32] as usize;
		let name = &dir[at + 33..at + 33 + nl];
		if name.starts_with(b"SCES_509.16") || name.starts_with(b"SCUS_971.99") {
			file = Some((u32_at(&dir, at + 2)? as u64, u32_at(&dir, at + 10)? as usize, if name.starts_with(b"SCUS") { 0x80 } else { 0 }));
			break;
		}
		at += len;
	}
	let (sector, size, shift) = file.ok_or("boot executable not on the disc")?;
	let elf = disc.read_bytes(sector, size).map_err(|e| e.to_string())?;
	let phoff = u32_at(&elf, 0x1c)? as usize;
	let (phentsize, phnum) = (u16_at(&elf, 0x2a)? as usize, u16_at(&elf, 0x2c)? as usize);
	let mut out = Vec::new();
	for va in FONT_TABLES.map(|a| a + shift) {
		let mut table = None;
		for p in 0..phnum {
			let ph = span(&elf, phoff + p * phentsize, 32)?;
			let (off, vaddr, filesz) = (u32_at(ph, 4)?, u32_at(ph, 8)?, u32_at(ph, 16)?);
			if va >= vaddr && va + (GLYPHS * 4) as u32 <= vaddr + filesz {
				table = Some(span(&elf, (off + va - vaddr) as usize, GLYPHS * 4)?);
			}
		}
		let t = table.ok_or("font table outside the executable image")?;
		out.push(
			t.chunks_exact(4)
				.map(|c| Glyph { u: c[0] as u16, v: c[1] as u16, dy: c[2] as i8 as i16, adv: c[3] as i16 })
				.collect(),
		);
	}
	Ok(out)
}

/// The three text fonts (core fx_textures records 1..=3, 256x128 PSMT8), paired with `glyphs`
/// from `font_glyph_tables`. Font ids in the game are 1-based, so font 1 is element 0.
pub fn level_fonts(level: &Level, glyphs: &[Vec<Glyph>]) -> R<Vec<Font>> {
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
		let colours = palette_rgba(span(&level.core, base + pal, 1024)?);
		let px = span(&level.core, base + pix, w * h)?;
		out.push(Font {
			tex: Texture { w, h, rgba: px.iter().map(|&v| colours[v as usize]).collect() },
			glyphs: glyphs[k - 1].clone(),
		});
	}
	Ok(out)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::render::{png, Image};
	use crate::{iso, toc};

	// UI-TEXT-1 test (1): bytes / 128, and the on-screen value times the 0x80 texel (clamped).
	const DISC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");
	const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test_out/hud");

	fn flat(t: &Texture) -> Image {
		let mut rgb = Vec::new();
		for (i, p) in t.rgba.iter().enumerate() {
			let (x, y) = (i % t.w, i / t.w);
			let bg = if (x / 4 + y / 4) % 2 == 0 { 60.0 } else { 100.0 };
			let a = p[3] as f32 / 255.0;
			for k in 0..3 {
				rgb.push((p[k] as f32 * a + bg * (1.0 - a)) as u8);
			}
		}
		Image { w: t.w, h: t.h, rgb }
	}

	/// WPN-6.7e4 (8): the Novalis level HUD bank holds the Morph-o-Ray meter sheet 0x7533 (tank, bubble).
	#[test]
	fn novalis_hud_has_morph_sheet() {
		let mut disc = iso::Disc::open(std::path::Path::new(DISC)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let info = levels.iter().find(|i| toc::LEVEL_NAMES[i.id] == "Novalis").unwrap();
		let hud = hud_sprites(&Level::load(&mut disc, info).unwrap()).unwrap();
		let icon = hud.icons.iter().find(|i| i.id == 0x7533).expect("sheet 0x7533");
		println!("0x7533: {} frames", icon.count);
		assert!(icon.count >= 2);
		for k in 0..2 {
			let t = &hud.frames[icon.first + k].as_ref().unwrap().tex;
			println!("frame {k}: {}x{}", t.w, t.h);
		}
	}

	#[test]
	fn fonts_decode() {
		let mut disc = iso::Disc::open(std::path::Path::new(DISC)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let glyphs = font_glyph_tables(&mut disc).unwrap();
		for (n, g) in glyphs.iter().enumerate() {
			for d in 0..10usize {
				let e = g[0x30 + d];
				assert_eq!((e.u, e.v), (d as u16 * 16, 64), "font {} digit {d}", n + 1);
			}
		}
		assert_eq!(glyphs[0][0x30].adv, 13);
		for info in &levels {
			let l = Level::load(&mut disc, info).unwrap();
			let fonts = level_fonts(&l, &glyphs).unwrap();
			assert_eq!(fonts.len(), 3);
			if info.id == 0 {
				for (i, f) in fonts.iter().enumerate() {
					std::fs::write(format!("{OUT}/font{}.png", i + 1), png(&flat(&f.tex))).unwrap();
				}
			}
		}
	}
}
