//! One level's data, as the game loads it: the data range's eleven (offset, size) sections, the
//! decompressed core index and core data, and the bounds of each block inside core data.
//! Layout facts: rac1-decomp tools/extract/level.py (names from Wrench), verified on our v1.00 disc.

use crate::bytes::{i16_at, span, u16_at, u32_at, R};
use crate::iso::Disc;
use crate::toc;
use crate::wad;

pub const SECTION_NAMES: [&str; 11] = [
	"overlay", "sound_bank", "core_index", "gs_ram", "hud_header", "hud_bank_0", "hud_bank_1",
	"hud_bank_2", "hud_bank_3", "hud_bank_4", "core_data",
];

/// Core index: (count, offset) tables of fixed-size entries; each class entry starts with a core offset.
const CLASS_TABLES: [(usize, usize); 3] = [(0x18, 32), (0x20, 32), (0x28, 48)]; // moby, tie, shrub
const GADGET_TABLE: (usize, usize) = (0x80, 16);
pub const TERRAIN_OFFSET: usize = 0x08;
pub const SKY_OFFSET: usize = 0x10;
pub const TEXTURE_BASE: usize = 0x60;

pub struct Level {
	pub id: usize,
	pub sections: Vec<Option<Vec<u8>>>,
	pub index: Vec<u8>,
	pub core: Vec<u8>,
	/// PAL gameplay data (instances, triggers, ...), decoded.
	pub gameplay: Vec<u8>,
	boundaries: Vec<usize>,
	/// The front-end scene ("level -1", `Level::title`): its index puts the CLUT table at 0x08,
	/// terrain at core 0 and the sky at 0x14, and has no Ratchet sequence or gadget tables.
	pub title: bool,
}

/// ToC entry of the front-end WAD (decomp/notes/title_scene_data.md).
const TITLE_WAD: usize = 0x14e8;
/// Title blob layout: index + GS data, then core, then gameplay at core + index[0x7c].
const TITLE_GS: usize = 0x4800;
const TITLE_CORE: usize = 0x5c000;

fn decoded(raw: &[u8]) -> R<Vec<u8>> {
	if wad::is_wad(raw) {
		wad::decompress(raw).map_err(|e| format!("WAD: {e}"))
	} else {
		Ok(raw.to_vec())
	}
}

impl Level {
	pub fn load(disc: &mut Disc, info: &toc::Level) -> R<Level> {
		let data = disc.read_sectors(info.data.sector, info.data.sectors).map_err(|e| e.to_string())?;
		let mut sections = Vec::new();
		for i in 0..SECTION_NAMES.len() {
			let off = u32_at(&data, i * 8)? as usize;
			let size = u32_at(&data, i * 8 + 4)? as usize;
			sections.push(if size > 0 { Some(decoded(span(&data, off, size)?)?) } else { None });
		}
		let take = |s: &mut Vec<Option<Vec<u8>>>, i: usize| s[i].take().ok_or(format!("no {} section", SECTION_NAMES[i]));
		let index = take(&mut sections, 2)?;
		let core = take(&mut sections, 10)?;
		if core.len() != u32_at(&index, 0x8c)? as usize {
			return Err("core data size differs from its index".into());
		}
		let g = disc.read_sectors(info.gameplay_pal.sector, info.gameplay_pal.sectors).map_err(|e| e.to_string())?;
		let gameplay = decoded(&g)?;
		let mut level = Level { id: info.id, sections, index, core, gameplay, boundaries: Vec::new(), title: false };
		level.boundaries = level.core_boundaries()?;
		Ok(level)
	}

	/// The title screen's 3D scene: one WAD holding index, GS data, core and gameplay together.
	pub fn title(disc: &mut Disc) -> R<Level> {
		let toc = toc::read_toc(disc).map_err(|e| e.to_string())?;
		let raw = disc.read_sectors(u32_at(&toc, TITLE_WAD)? as u64, u32_at(&toc, TITLE_WAD + 4)? as u64).map_err(|e| e.to_string())?;
		let blob = decoded(&raw)?;
		let index = span(&blob, 0, TITLE_CORE)?.to_vec();
		let gameplay_at = TITLE_CORE + u32_at(&index, 0x7c)? as usize;
		let gameplay_len = u32_at(&index, 0x78)? as usize;
		let core = span(&blob, TITLE_CORE, gameplay_at - TITLE_CORE)?.to_vec();
		let gameplay = span(&blob, gameplay_at, gameplay_len)?.to_vec();
		let mut sections: Vec<Option<Vec<u8>>> = vec![None; SECTION_NAMES.len()];
		sections[3] = Some(span(&blob, TITLE_GS, TITLE_CORE - TITLE_GS)?.to_vec());
		let mut level = Level { id: usize::MAX, sections, index, core, gameplay, boundaries: Vec::new(), title: true };
		level.boundaries = level.core_boundaries()?;
		Ok(level)
	}

	/// The entries of a (count, offset) table in the core index.
	pub fn table(&self, at: usize, size: usize) -> R<Vec<&[u8]>> {
		let count = u32_at(&self.index, at)? as usize;
		let off = u32_at(&self.index, at + 4)? as usize;
		let data = span(&self.index, off, count * size)?;
		Ok(data.chunks_exact(size).collect())
	}

	/// Every known block start in core data; a block ends where the next one starts.
	fn core_boundaries(&self) -> R<Vec<usize>> {
		let mut b = vec![self.core.len()];
		let starts: &[usize] = if self.title { &[0x14, TEXTURE_BASE, 0x80, 0x84] } else { &[0x08, 0x0c, 0x10, 0x14, TEXTURE_BASE] };
		for &at in starts {
			b.push(u32_at(&self.index, at)? as usize);
		}
		for (at, size) in CLASS_TABLES.into_iter().chain((!self.title).then_some(GADGET_TABLE)) {
			for e in self.table(at, size)? {
				b.push(u32_at(e, 0)? as usize);
			}
		}
		let seq = if self.title { 0 } else { u32_at(&self.index, 0x78)? as usize };
		if seq != 0 {
			// 256 ratchet animation sequence offsets, stored in the index.
			for i in 0..256 {
				let o = u32_at(&self.index, seq + i * 4)? as i32;
				if o > 0 {
					b.push(o as usize);
				}
			}
		}
		b.retain(|&x| x > 0 && x <= self.core.len());
		b.sort_unstable();
		b.dedup();
		Ok(b)
	}

	/// The core block starting at `start`, ending at the next known block.
	pub fn block(&self, start: usize) -> R<&[u8]> {
		let i = self.boundaries.partition_point(|&x| x <= start);
		let end = *self.boundaries.get(i).ok_or(format!("cannot bound core block at {start:#x}"))?;
		span(&self.core, start, end - start)
	}

	pub fn terrain_block(&self) -> R<&[u8]> {
		self.block(if self.title { 0 } else { u32_at(&self.index, TERRAIN_OFFSET)? as usize })
	}

	/// Base mip of every texture in one group's table, by index (None for empty slots).
	/// Entry: (pixel offset from the texture base, width, height, type, palette in 256-byte units,
	/// mips, pad). Palettes must be RGBA32 CLUTs that the level's GS RAM table uploads.
	pub fn textures(&self, group: TextureGroup) -> R<Vec<Option<Texture>>> {
		let gs = self.sections[3].as_deref().ok_or("no gs_ram section")?;
		let base = u32_at(&self.index, TEXTURE_BASE)? as usize;
		let mut clut32 = Vec::new();
		for e in self.table(if self.title { 0x08 } else { 0x00 }, 16)? {
			if u32_at(e, 0)? == 0 {
				clut32.push(u32_at(e, 8)? as usize);
			}
		}
		let mut out = Vec::new();
		for (i, e) in self.table(group as usize, 16)?.into_iter().enumerate() {
			let pixels = u32_at(e, 0)? as usize;
			let (w, h) = (u16_at(e, 4)? as usize, u16_at(e, 6)? as usize);
			let palette = u16_at(e, 10)? as usize * 256;
			if w == 0 && h == 0 {
				out.push(None);
				continue;
			}
			if !clut32.contains(&palette) {
				return Err(format!("texture {i} uses an unknown palette"));
			}
			out.push(Some(Texture::new(w, h, span(&self.core, base + pixels, w * h)?, span(gs, palette, 1024)?)?));
		}
		Ok(out)
	}
}

impl Level {
	/// Billboard texture chain of every shrub class that has one (REN-CULL-SHRUB-BBTEX): key = class id (row +4), value = mips
	/// from the base down. The row's descriptor (+0x20, eight i16: width, height, mips, CBP, TBP of mips 0..3) points into the
	/// GS RAM section, which is addressed in 256-byte blocks from the same offset the uploads use; a zero width means no billboard.
	pub fn shrub_billboard_textures(&self) -> R<std::collections::HashMap<u32, Vec<Texture>>> {
		let gs = self.sections[3].as_deref().ok_or("no gs_ram section")?;
		let mut out = std::collections::HashMap::new();
		for e in self.table(0x28, 48)? {
			let class_id = u32_at(e, 4)?;
			let d: Vec<i16> = (0..8).map(|k| i16_at(e, 0x20 + k * 2)).collect::<R<_>>()?;
			let (w, h, mips, cbp) = (d[0] as i32, d[1] as i32, d[2] as usize, d[3] as usize);
			if w == 0 {
				continue;
			}
			if w < 0 || h < 0 || !(1..=4).contains(&mips) {
				return Err(format!("shrub class {class_id}: invalid billboard descriptor"));
			}
			let palette = span(gs, cbp * 256, 1024).map_err(|x| format!("shrub class {class_id}: {x}"))?;
			let mut chain = Vec::new();
			for k in 0..mips {
				let (mw, mh) = ((w as usize >> k).max(1), (h as usize >> k).max(1));
				let px = span(gs, d[4 + k] as usize * 256, mw * mh).map_err(|x| format!("shrub class {class_id} mip {k}: {x}"))?;
				chain.push(Texture::new(mw, mh, px, palette).map_err(|x| format!("shrub class {class_id} mip {k}: {x}"))?);
			}
			out.insert(class_id, chain);
		}
		Ok(out)
	}

	/// The authored mip chain of every terrain texture, by table index (REN-TEX-RESIDENCY): mips 0 and 1 follow each other in the
	/// core pixels, mips 2 and 3 sit in the GS RAM section at the entry's TBP2 / TBP3 (entry +12 / +14, 256-byte blocks); the
	/// palette is the entry's CBP (+10). An empty slot (width 0) is None; `mips` (+8) levels are returned.
	pub fn terrain_mips(&self) -> R<Vec<Option<Vec<Texture>>>> {
		self.mip_chains(TextureGroup::Terrain)
	}

	/// The authored mip chain of every texture of a group with the terrain entry layout, by table index. Shrub textures use it
	/// too: the shrub class setup (Veldin OV:51666-51671, twin of moby_ad_patch_tex FUN_00253c78) reads the same 16-byte entry,
	/// MXL = +8 mips - 1, MIPTBP1 TBP2 / TBP3 = +0xc / +0xe + gs base, TBP0 / TBP1 0 (streamed) [C]; mip 1 following mip 0 in the
	/// core pixels as for terrain [I] (test `shrub_mips_are_box_like`).
	pub fn mip_chains(&self, group: TextureGroup) -> R<Vec<Option<Vec<Texture>>>> {
		let gs = self.sections[3].as_deref().ok_or("no gs_ram section")?;
		let base = u32_at(&self.index, TEXTURE_BASE)? as usize;
		let mut out = Vec::new();
		for (i, e) in self.table(group as usize, 16)?.into_iter().enumerate() {
			let (pixels, w, h) = (u32_at(e, 0)? as usize, u16_at(e, 4)? as usize, u16_at(e, 6)? as usize);
			let (mips, cbp) = (i16_at(e, 8)? as usize, u16_at(e, 10)? as usize);
			if w == 0 && h == 0 {
				out.push(None);
				continue;
			}
			let palette = span(gs, cbp * 256, 1024)?;
			let mut chain = Vec::new();
			let mut at = base + pixels;
			for k in 0..mips.clamp(1, 4) {
				let (mw, mh) = ((w >> k).max(1), (h >> k).max(1));
				let px = match k {
					0 | 1 => {
						let p = span(&self.core, at, mw * mh).map_err(|x| format!("texture {i} mip {k}: {x}"))?;
						at += mw * mh;
						p
					}
					_ => span(gs, u16_at(e, 8 + 2 * k)? as usize * 256, mw * mh).map_err(|x| format!("texture {i} mip {k}: {x}"))?,
				};
				chain.push(Texture::new(mw, mh, px, palette).map_err(|x| format!("texture {i} mip {k}: {x}"))?);
			}
			out.push(Some(chain));
		}
		Ok(out)
	}
}

impl Level {
	/// Particle textures (index table 0x50: 16 B records of palette offset, 0, pixel offset, side;
	/// offsets are from core + index[0x64]; 32x32 PSMT8), by particle texture number [C: all 32
	/// extracted particle PNGs match pixel for pixel].
	pub fn particle_textures(&self) -> R<Vec<Option<Texture>>> {
		let base = u32_at(&self.index, 0x64)? as usize;
		let mut out = Vec::new();
		for e in self.table(0x50, 16)? {
			let (palette, pixels, side) = (u32_at(e, 0)? as usize, u32_at(e, 8)? as usize, u32_at(e, 12)? as usize);
			out.push(span(&self.core, base + pixels, side * side).ok().and_then(|px| Texture::new(side, side, px, span(&self.core, base + palette, 1024).ok()?).ok()));
		}
		Ok(out)
	}
}

impl Level {
	/// First byte of every particle-effect def (ParseParticleTexs / FUN_0023f218 table at index
	/// +0x6c: count, _, defs offset, defs size, then per-type def offsets): the particle texture
	/// number that effect type draws with. Indexed by effect type [I].
	pub fn particle_def_textures(&self) -> R<Vec<u8>> {
		let blob = u32_at(&self.index, 0x6c)? as usize;
		let head = span(&self.index, blob, 16)?;
		let count = u32_at(head, 0)? as usize;
		let defs = u32_at(head, 8)? as usize;
		let mut out = Vec::new();
		for k in 0..count {
			let off = u32_at(&self.index, blob + 16 + k * 4)? as usize;
			// An unset pointer falls back to the first def (FUN_0023f218).
			let at = if off == 0 { defs } else { off };
			out.push(*self.index.get(blob + at).ok_or("particle def out of range")?);
		}
		Ok(out)
	}
}

impl Level {
	/// The first `n` bytes of effect type `ty`'s particle def: the texture numbers of its frames (type 8, the
	/// Novalis flame, steps through 10 of them: FUN_0027ecb8 indexes the def by (life0 - life) * 10 / life0).
	pub fn particle_def_frames(&self, ty: usize, n: usize) -> R<Vec<u8>> {
		let blob = u32_at(&self.index, 0x6c)? as usize;
		let head = span(&self.index, blob, 16)?;
		let count = u32_at(head, 0)? as usize;
		let defs = u32_at(head, 8)? as usize;
		if ty >= count {
			return Err("particle def type out of range".into());
		}
		let off = u32_at(&self.index, blob + 16 + ty * 4)? as usize;
		let at = if off == 0 { defs } else { off };
		Ok(span(&self.index, blob + at, n)?.to_vec())
	}
}

/// NOV-35b: texture keys of the two metal-packet textures (`Level::metal_textures`), past the moby group's slots and FLAT_SLOT's neighbours.
pub const CHROME_KEY: u32 = 1 << 16 | 0xfffd;
pub const GLASS_KEY: u32 = 1 << 16 | 0xfffe;

impl Level {
	/// NOV-35b: the metal-packet textures (moby_format.md 4.1, TEX0 OV:47060-47069): chrome 128 x 128 PSMT8 with pixels at the
	/// gs image offset in index word 0x90 and its CT32 CLUT at word 0x94; glass 64 x 64 PSMT8 at words 0x98 / 0x9c.
	pub fn metal_textures(&self) -> R<[Texture; 2]> {
		let gs = self.sections[3].as_deref().ok_or("no gs_ram section")?;
		let tex = |at: usize, side: usize| -> R<Texture> {
			let (pixels, clut) = (u32_at(&self.index, at)? as usize, u32_at(&self.index, at + 4)? as usize);
			Texture::new(side, side, span(gs, pixels, side * side)?, span(gs, clut, 1024)?)
		};
		Ok([tex(0x90, 128)?, tex(0x98, 64)?])
	}
}

/// Mesh faces carry `group key | texture index` so groups can share one texture map.
pub fn texture_key(group: TextureGroup, index: u32) -> u32 {
	let g = match group {
		TextureGroup::Terrain => 0,
		TextureGroup::Moby => 1,
		TextureGroup::Tie => 2,
		TextureGroup::Shrub => 3,
	};
	g << 16 | index
}

impl Level {
	/// Every texture of every group, by `texture_key`.
	pub fn all_textures(&self) -> R<std::collections::HashMap<u32, Texture>> {
		let mut out = std::collections::HashMap::new();
		for g in [TextureGroup::Terrain, TextureGroup::Moby, TextureGroup::Tie, TextureGroup::Shrub] {
			for (i, t) in self.textures(g)?.into_iter().enumerate() {
				if let Some(t) = t {
					out.insert(texture_key(g, i as u32), t);
				}
			}
		}
		Ok(out)
	}
}

#[derive(Clone, Copy)]
pub enum TextureGroup {
	Terrain = 0x30,
	Moby = 0x38,
	Tie = 0x40,
	Shrub = 0x48,
}

	/// An 8-bit indexed texture expanded to RGBA8 (PSMT8 with a CSM1 RGBA32 palette).
	#[derive(Clone)]
	pub struct Texture {
	pub w: usize,
	pub h: usize,
	pub rgba: Vec<[u8; 4]>,
}

impl Texture {
	pub(crate) fn new(w: usize, h: usize, pixels: &[u8], palette: &[u8]) -> R<Texture> {
		if !(1..=4096).contains(&w) || !(1..=4096).contains(&h) {
			return Err("unsupported texture dimensions".into());
		}
		// The GS reads CSM1 palettes with index bits 3 and 4 swapped; PS2 alpha 0x80 is opaque.
		let colours: Vec<[u8; 4]> = (0..256)
			.map(|i| {
				let j = (i & !0x18) | ((i & 8) << 1) | ((i & 0x10) >> 1);
				let c = &palette[j * 4..j * 4 + 4];
				[c[0], c[1], c[2], (c[3] as u16 * 2).min(255) as u8]
			})
			.collect();
		Ok(Texture { w, h, rgba: pixels.iter().map(|&p| colours[p as usize]).collect() })
	}

	pub fn sample(&self, u: f32, v: f32) -> [u8; 4] {
		let x = ((u - u.floor()) * self.w as f32) as usize % self.w;
		let y = ((v - v.floor()) * self.h as f32) as usize % self.h;
		self.rgba[y * self.w + x]
	}
}

#[cfg(test)]
mod particle_tests {
	use super::*;
	/// Veldin: 40 particle textures, all 32x32; the bolt sparkle def (type 0x35) names one of them.
	#[test]
	fn particle_tables() {
		let mut disc = crate::iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
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

#[cfg(test)]
mod mip_tests {
	use super::*;

	fn level(i: usize) -> Level {
		let mut disc = crate::iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{gameplay, iso, shrubs, terrain, ties};

	/// ARI-1 (decomp/notes/aridia_presentation.md): Aridia loads through the generic loader and keeps its overlay image.
	#[test]
	fn aridia_loads() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
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
