//! R&C1 files the table of contents points at (engine split stage E2, 2026-10-26: moved out of formats/sound.rs, anim.rs,
//! hud.rs, level.rs and music.rs, which keep the format parsers): the global sound bank, HelpDesk voices, pause-menu and
//! shop-portrait clips, the resident HUD set, the boot-executable font glyph tables, the title scene and a level's music streams. Re-exported by `toc`.

use crate::anim::{parse_sequence, Sequence};
use crate::bytes::{span, u16_at, u32_at, R};
use crate::hud::{build, Glyph, Hud, BANKS};
use crate::iso::Disc;
use crate::level::Level;
use crate::music::Music;
use crate::sound::{decode_adpcm, Bank, Sample};
use crate::{toc, wad};

/// The global bank (ToC + 0x14e0: sector, sector count).
pub fn global_bank(disc: &mut Disc) -> R<Bank> {
	let t = toc::read_toc(disc).map_err(|e| e.to_string())?;
	let (sector, sectors) = (u32_at(&t, 0x14e0)? as u64, u32_at(&t, 0x14e4)? as u64);
	let raw = disc.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
	Bank::parse(&raw)
}

/// HelpDesk voice `voice` (help message entry +8) for menu language `lang` (EN FR DE ES IT): ToC +
/// 0x1ab8 holds 6 x 150 sector numbers (language table 1 is empty on this disc), each a VAGp file
/// (big-endian data size +0x0c, rate +0x10, PS-ADPCM from +0x30). FUN_002151d0 reads the same
/// table from the boot ELF (decomp/notes/ingame_menu_leftovers.md P1-a). Returns (rate, pcm).
pub fn help_voice(disc: &mut Disc, lang: usize, voice: usize) -> R<Option<(u32, Vec<i16>)>> {
	const TABLE: usize = 0x1ab8;
	const LANG: [usize; 5] = [0, 2, 3, 4, 5];
	if voice >= 150 {
		return Ok(None);
	}
	let t = toc::read_toc(disc).map_err(|e| e.to_string())?;
	let lba = u32_at(&t, TABLE + LANG[lang.min(4)] * 600 + voice * 4)? as u64;
	if lba == 0 {
		return Ok(None);
	}
	let head = disc.read_bytes(lba, 0x30).map_err(|e| e.to_string())?;
	if &head[..4] != b"VAGp" {
		return Err(format!("help voice {voice}: no VAGp at sector {lba}"));
	}
	let be = |o: usize| u32::from_be_bytes([head[o], head[o + 1], head[o + 2], head[o + 3]]);
	let (size, rate) = (be(0x0c) as usize, be(0x10));
	let data = disc.read_bytes(lba, 0x30 + size).map_err(|e| e.to_string())?;
	Ok(Some((rate, decode_adpcm(&data[0x30..])?.0)))
}

/// Every sound of the global bank decoded.
pub fn global_sounds(disc: &mut Disc) -> R<Vec<Sample>> {
	global_bank(disc)?.samples()
}

/// Number of streamed hero menu clips (globals/ratchet_seqs/0..27).
pub const MENU_CLIPS: usize = 28;

/// The pause-menu hero clips (decomp/notes/ingame_pause_menu.md "Hero preview clips"): ToC (sector 1500) +0x18
/// holds 28 (sector, sectors) entries, each a WAD block holding one standard sequence (frame offsets relative to
/// its header). Retail streams clip N as Ratchet seq (class-0 seq count + N); the port preloads them.
pub fn menu_clips(disc: &mut crate::iso::Disc, joints: usize) -> R<Vec<Sequence>> {
	let toc = crate::toc::read_toc(disc).map_err(|e| e.to_string())?;
	(0..MENU_CLIPS)
		.map(|n| {
			let (sector, sectors) = (u32_at(&toc, 0x18 + n * 8)? as u64, u32_at(&toc, 0x1c + n * 8)? as u64);
			let raw = disc.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
			let file = crate::wad::decompress(&raw).map_err(|e| format!("menu clip {n}: {e}"))?;
			parse_sequence(&file, &file, joints).map_err(|e| format!("menu clip {n}: {e}"))
		})
		.collect()
}

/// Pause-menu auxiliary clip `k` (ToC +0xf8 + 8k, table 0x137c78; FUN_00275a08 reads them at pause open and
/// FUN_00284028 installs them in their class's sequence slot, decomp/notes/ingame_pause_menu.md "Hero preview
/// records").
pub fn menu_aux_clip(disc: &mut crate::iso::Disc, k: usize, joints: usize) -> R<Sequence> {
	let toc = crate::toc::read_toc(disc).map_err(|e| e.to_string())?;
	let (sector, sectors) = (u32_at(&toc, 0xf8 + k * 8)? as u64, u32_at(&toc, 0xfc + k * 8)? as u64);
	let raw = disc.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
	let file = crate::wad::decompress(&raw).map_err(|e| format!("menu aux clip {k}: {e}"))?;
	parse_sequence(&file, &file, joints).map_err(|e| format!("menu aux clip {k}: {e}"))
}

/// UI-5a / BRIDGE-37 (c): the shop portrait's class-12 sequence pack (TOC +0x198 LBA / +0x19c sectors, a `WAD`): 16 u32 offsets
/// from the start of the decompressed buffer, nonzero word i = sequence i (header-relative frame and trigger offsets,
/// FUN_002541e0). Empty slots stay empty sequences (decomp/notes/vendor.md "UI-5a port spec").
pub fn portrait_seqs(disc: &mut crate::iso::Disc, joints: usize) -> R<Vec<Sequence>> {
	let toc = crate::toc::read_toc(disc).map_err(|e| e.to_string())?;
	let (sector, sectors) = (u32_at(&toc, 0x198)? as u64, u32_at(&toc, 0x19c)? as u64);
	let raw = disc.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
	let buf = crate::wad::decompress(&raw).map_err(|e| format!("portrait pack: {e}"))?;
	(0..16)
		.map(|i| {
			let off = u32_at(&buf, 4 * i)? as usize;
			if off == 0 {
				return Ok(Sequence::from_timing(&[]));
			}
			let seq = buf.get(off..).ok_or_else(|| format!("portrait sequence {i}: offset {off:#x} out of range"))?;
			parse_sequence(seq, seq, joints).map_err(|e| format!("portrait sequence {i}: {e}"))
		})
		.collect()
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
	build(&header, &refs, rac_engine::texpack::hudrepl::enabled())
}

/// ToC entry of the front-end WAD (decomp/notes/title_scene_data.md).
const TITLE_WAD: usize = 0x14e8;

/// The title screen's 3D scene (`Level::title` of the WAD at ToC +0x14e8).
pub fn title_level(disc: &mut Disc) -> R<Level> {
	let toc = toc::read_toc(disc).map_err(|e| e.to_string())?;
	let raw = disc.read_sectors(u32_at(&toc, TITLE_WAD)? as u64, u32_at(&toc, TITLE_WAD + 4)? as u64).map_err(|e| e.to_string())?;
	Level::title(&raw)
}

/// Every music stream of a level (`music::level_music` at the level's header sector from the level table).
pub fn level_music(disc: &mut Disc, level: &Level) -> R<Vec<Music>> {
	let levels = toc::read_levels(disc).map_err(|e| e.to_string())?;
	let info = levels.get(level.id).ok_or("no such level")?;
	crate::music::level_music(disc, info.header_sector)
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
