//! Locally generated texture and glyph-metric overrides. No disc data is built in.
use std::sync::{Mutex, OnceLock};

pub(super) struct Table {
    pub old: Vec<u8>,
    pub new: Vec<u8>,
}
struct Edit {
    at: usize,
    index: u8,
    rgba: [u8; 4],
}
struct Texture {
    raw: u64,
    rgba: u64,
    w: usize,
    h: usize,
    edits: Vec<Edit>,
}
#[derive(Default)]
struct Assets {
    chars: Vec<(char, u8)>,
    tables: Vec<Table>,
    textures: Vec<Texture>,
}

fn take<'a>(b: &'a [u8], at: &mut usize, n: usize) -> Option<&'a [u8]> {
    let end = at.checked_add(n)?;
    let out = b.get(*at..end)?;
    *at = end;
    Some(out)
}
fn u32le(b: &[u8], at: &mut usize) -> Option<u32> {
    Some(u32::from_le_bytes(take(b, at, 4)?.try_into().ok()?))
}
fn parse(b: &[u8]) -> Option<Assets> {
    if b.get(..8)? != b"RCLA0001" {
        return None;
    }
    let mut at = 8;
    let n = u32le(b, &mut at)? as usize;
    if n > 40 {
        return None;
    }
    let mut a = Assets::default();
    for _ in 0..n {
        let ch = char::from_u32(u32le(b, &mut at)?)?;
        let code = take(b, &mut at, 1)?[0];
        if !(0x80..0xa8).contains(&code) {
            return None;
        }
        a.chars.push((ch, code));
    }
    let n = u32le(b, &mut at)?;
    if n > 3 {
        return None;
    }
    for _ in 0..n {
        let len = u32le(b, &mut at)? as usize;
        if len != 928 {
            return None;
        }
        let old = take(b, &mut at, len)?.to_vec();
        let new = take(b, &mut at, len)?.to_vec();
        a.tables.push(Table { old, new });
    }
    let n = u32le(b, &mut at)?;
    if n > 1024 {
        return None;
    }
    for _ in 0..n {
        let raw = u64::from_le_bytes(take(b, &mut at, 8)?.try_into().ok()?);
        let rgba = u64::from_le_bytes(take(b, &mut at, 8)?.try_into().ok()?);
        let w = u32le(b, &mut at)? as usize;
        let h = u32le(b, &mut at)? as usize;
        if !matches!((w, h), (256, 128) | (512, 64)) {
            return None;
        }
        let n = u32le(b, &mut at)?;
        if n > 65536 {
            return None;
        }
        let mut edits = Vec::new();
        for _ in 0..n {
            let pos = u32le(b, &mut at)? as usize;
            if pos >= w * h {
                return None;
            }
            let p = take(b, &mut at, 5)?;
            edits.push(Edit {
                at: pos,
                index: p[0],
                rgba: p[1..5].try_into().ok()?,
            });
        }
        a.textures.push(Texture {
            raw,
            rgba,
            w,
            h,
            edits,
        });
    }
    (at == b.len()).then_some(a)
}
fn assets() -> &'static Assets {
    static A: OnceLock<Assets> = OnceLock::new();
    A.get_or_init(|| {
        let Some(path) = rac_engine::localization::active_pack()
            .and_then(|p| p.file("font_assets", "translation_assets.bin"))
        else {
            return Assets::default();
        };
        let Some(a) = std::fs::read(&path).ok().and_then(|b| parse(&b)) else {
            return Assets::default();
        };
        eprintln!(
            "locale-assets: {} characters, {} glyph tables, {} texture patches",
            a.chars.len(),
            a.tables.len(),
            a.textures.len()
        );
        a
    })
}
pub(super) fn code(ch: char) -> Option<u8> {
    assets().chars.iter().find(|p| p.0 == ch).map(|p| p.1)
}
pub(crate) fn unicode(code: u8) -> Option<char> {
    assets().chars.iter().find(|p| p.1 == code).map(|p| p.0)
}
pub(crate) fn accent_index(code: u32) -> Option<usize> {
    (!assets().chars.is_empty() && (0x80..0xa8).contains(&code)).then_some(code as usize + 64)
}
pub(super) fn tables() -> &'static [Table] {
    &assets().tables
}
pub(crate) fn table_bytes(raw: &mut [u8]) {
    for t in tables() {
        if raw == t.old.as_slice() {
            raw.copy_from_slice(&t.new);
            return;
        }
    }
}
fn fnv(bytes: impl Iterator<Item = u8>) -> u64 {
    bytes.fold(0xcbf29ce484222325, |h, b| {
        (h ^ b as u64).wrapping_mul(0x100000001b3)
    })
}
pub(crate) fn words(data: &mut [u128], want_orig: bool) -> Option<Vec<u128>> {
    if assets().textures.is_empty() || data.len() < 2048 {
        return None;
    }
    let hash = fnv(data[..2048].iter().flat_map(|w| w.to_le_bytes()));
    let t = assets().textures.iter().find(|t| t.raw == hash)?;
    let old = want_orig.then(|| data.to_vec());
    for e in &t.edits {
        let shift = (e.at % 16) * 8;
        data[e.at / 16] = (data[e.at / 16] & !(255u128 << shift)) | ((e.index as u128) << shift);
    }
    static SEEN: Mutex<Vec<u64>> = Mutex::new(Vec::new());
    if let Ok(mut s) = SEEN.lock() {
        if !s.contains(&hash) {
            s.push(hash);
            eprintln!("locale-assets: patched {}x{} texture {hash:016x}", t.w, t.h);
        }
    }
    old
}
pub(crate) fn rgba(w: usize, h: usize, px: &mut [[u8; 4]]) {
    if assets().textures.is_empty() {
        return;
    }
    let hash = fnv(px.iter().flat_map(|p| p.iter().copied()));
    if let Some(t) = assets()
        .textures
        .iter()
        .find(|t| t.w == w && t.h == h && t.rgba == hash)
    {
        for e in &t.edits {
            px[e.at] = e.rgba;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_bad_assets() {
        assert!(parse(b"RCLA0001").is_none());
        assert!(parse(b"bad").is_none());
    }
}
