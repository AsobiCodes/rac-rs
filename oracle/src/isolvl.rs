//! Level data read straight from the player's disc image (ORACLE_ISO, else DEFAULT_ISO), so nothing has to be extracted first.
//! Layout (same as rac-rs/src/{toc,level}.rs): ToC at sector 1500 (0x2960 bytes), level table at +0x28c8 (19 x (header sector, size)),
//! level header (0x2434 bytes) words 2/3 = (first sector, sector count) of the level data block; the first 11 (offset, size) pairs of
//! that block are the sections. Section 0 = overlay records (WAD packed), section 1 = sound bank (raw).
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::{Mutex, OnceLock};

const SEC: u64 = 0x800;

fn rd(f: &mut std::fs::File, byte: u64, len: usize) -> Option<Vec<u8>> {
    f.seek(SeekFrom::Start(byte)).ok()?;
    let mut v = vec![0u8; len];
    f.read_exact(&mut v).ok()?;
    Some(v)
}

fn w(b: &[u8], o: usize) -> Option<u32> {
    b.get(o..o + 4).map(|x| u32::from_le_bytes(x.try_into().unwrap()))
}

/// Section `idx` (0..11) of level `lvl` (0..19) exactly as stored on the disc.
pub fn level_section_raw(lvl: u32, idx: usize) -> Option<Vec<u8>> {
    if lvl >= 19 || idx >= 11 {
        return None;
    }
    let iso = std::env::var("ORACLE_ISO").unwrap_or_else(|_| crate::DEFAULT_ISO.into());
    let mut f = std::fs::File::open(iso).ok()?;
    let toc = rd(&mut f, 1500 * SEC, 0x2960)?;
    if w(&toc, 0)? != 1 || w(&toc, 4)? != 0x2960 {
        return None;
    }
    let hdr = rd(&mut f, w(&toc, 0x28c8 + lvl as usize * 8)? as u64 * SEC, 0x2434)?;
    if w(&hdr, 0)? != lvl || w(&hdr, 4)? != 0x2434 {
        return None;
    }
    let base = w(&hdr, 8)? as u64 * SEC;
    let tab = rd(&mut f, base, 11 * 8)?;
    let (off, size) = (w(&tab, idx * 8)? as u64, w(&tab, idx * 8 + 4)? as usize);
    rd(&mut f, base + off, size)
}

/// The section with WAD packing removed.
pub fn level_section(lvl: u32, idx: usize) -> Option<Vec<u8>> {
    let raw = level_section_raw(lvl, idx)?;
    if raw.starts_with(b"WAD") {
        crate::wad::decompress(&raw).ok()
    } else {
        Some(raw)
    }
}

/// Level `lvl`'s overlay as (lowest load address, bytes): the records `[load, size, flags, entry][size bytes]` of section 0 placed at their
/// load addresses, gaps zero (equal to the PT_LOAD of the Wrench-extracted overlay.elf). Decoded once per level and kept.
pub fn overlay_image(lvl: u32) -> Option<&'static (u32, Vec<u8>)> {
    static CACHE: OnceLock<Mutex<HashMap<u32, Option<&'static (u32, Vec<u8>)>>>> = OnceLock::new();
    let mut m = CACHE.get_or_init(Default::default).lock().ok()?;
    if let Some(v) = m.get(&lvl) {
        return *v;
    }
    let v = build_overlay(lvl).map(|x| &*Box::leak(Box::new(x)));
    m.insert(lvl, v);
    v
}

fn build_overlay(lvl: u32) -> Option<(u32, Vec<u8>)> {
    let ov = level_section(lvl, 0)?;
    let mut recs = Vec::new();
    let mut p = 0usize;
    while p + 16 <= ov.len() {
        let (load, size) = (w(&ov, p)?, w(&ov, p + 4)? as usize);
        recs.push((load, p + 16, size));
        p += 16 + size;
    }
    if p != ov.len() || recs.is_empty() {
        return None;
    }
    let lo = recs.iter().map(|r| r.0).min()?;
    let hi = recs.iter().map(|r| r.0 as usize + r.2).max()?;
    let mut img = vec![0u8; hi - lo as usize];
    for (load, src, size) in recs {
        img[(load - lo) as usize..][..size].copy_from_slice(ov.get(src..src + size)?);
    }
    Some((lo, img))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The disc-derived overlay and sound bank equal the Wrench-extracted files (dev tree only: skipped without ../extracted or the disc).
    #[test]
    fn disc_sections_equal_extracted_files() {
        let root = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../extracted/levels"));
        let Ok(dirs) = std::fs::read_dir(&root) else { return };
        if level_section_raw(0, 0).is_none() {
            return;
        }
        let dirs: Vec<_> = dirs.flatten().map(|e| e.path()).collect();
        let mut checked = 0;
        for lvl in 0..19u32 {
            let pre = format!("{lvl}_");
            let Some(d) = dirs.iter().find(|p| p.file_name().and_then(|f| f.to_str()).is_some_and(|f| f.starts_with(&pre))) else { continue };
            if let Ok(elf) = std::fs::read(d.join("overlay.elf")) {
                let u = |o: usize| u32::from_le_bytes(elf[o..o + 4].try_into().unwrap());
                let ph = u(0x1c) as usize;
                let (off, va, size) = (u(ph + 4) as usize, u(ph + 8), u(ph + 16) as usize);
                let (lo, img) = overlay_image(lvl).expect("overlay from the disc");
                assert_eq!(*lo, va, "level {lvl} overlay load address");
                assert!(img.len() == size && img[..] == elf[off..off + size], "level {lvl} overlay bytes");
                checked += 1;
            }
            if let Ok(bank) = std::fs::read(d.join("sound_bank.bin")) {
                assert!(level_section(lvl, 1).expect("sound bank from the disc") == bank, "level {lvl} sound bank");
                checked += 1;
            }
        }
        eprintln!("isolvl: {checked} extracted files compared");
    }
}
