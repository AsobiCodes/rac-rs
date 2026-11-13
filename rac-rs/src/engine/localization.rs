// SPDX-License-Identifier: GPL-3.0-only
//! Optional mod-driven sector overlays and modern UI strings. ISO bytes are never modified.
use crate::omni::json::{self, Json};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

mod pack;
pub use pack::{Pack, packs};
pub const API_VERSION: u32 = 1;

struct Segment {
    start: u64,
    end: u64,
    data: Vec<u8>,
}
#[derive(Default)]
struct Data {
    enabled: bool,
    segments: Vec<Segment>,
    ui: BTreeMap<String, String>,
    hd_cards: bool,
}
static DATA: OnceLock<Data> = OnceLock::new();

pub fn install() {
    rac_formats::iso::set_read_filter(filter);
    ee_oracle::hle::set_disc_read_filter(filter);
}
fn read_json(p: &Path) -> Option<Json> {
    if p.metadata().ok()?.len() > 16 << 20 {
        return None;
    }
    json::parse(
        std::fs::read_to_string(p)
            .ok()?
            .trim_start_matches('\u{feff}'),
    )
    .ok()
}
fn relative(root: &Path, name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains(['\\', ':']) {
        return None;
    }
    let path = Path::new(name);
    if path
        .components()
        .any(|p| !matches!(p, std::path::Component::Normal(_)))
    {
        return None;
    }
    let joined = root.join(path);
    if joined.exists()
        && !joined
            .canonicalize()
            .ok()?
            .starts_with(root.canonicalize().ok()?)
    {
        return None;
    }
    Some(joined)
}
fn load() -> Data {
    let run = || -> Result<Data, String> {
        let Some(pack) = pack::active_pack() else {
            return Ok(Data::default());
        };
        let id = &pack.mod_id;
        let dir = &pack.root;
        let cache = if pack.flag("prepared_cache") {
            crate::pcopts::player_dir()
                .join("localization-cache")
                .join(id)
        } else {
            dir.clone()
        };
        // Preparation is deliberately external to the game. A pack cannot execute arbitrary Python.
        // Installers generate private disc overlays once from the owner's ISO; enabling a pack is read-only.
        let diagnostic = crate::texpack::mods::mods_root()
            .map(|r| crate::omni::plugins::read_settings(&r))
            .is_some_and(|settings| {
                crate::omni::plugins::value_of(&settings, id, "charset_test")
                    .is_some_and(Json::bool)
            });
        let cache = if diagnostic && pack.flag("prepared_cache") {
            cache.with_file_name(format!("{id}-diagnostic"))
        } else {
            cache
        };
        let doc = match read_json(&cache.join("overlay.json")) {
            Some(doc) => doc,
            None if pack.flag("prepared_cache") => {
                return Err(format!(
                    "{id}: private overlay cache is missing; run the pack installer"
                ));
            }
            None => Json::Obj(vec![("format".into(), Json::Num(1.0))]),
        };
        if doc.get("format").and_then(Json::as_num) != Some(1.0) {
            return Err("unsupported sector overlay format".into());
        }
        let mut data = Data {
            enabled: true,
            hd_cards: dir.join("images-hd.json").is_file(),
            ..Data::default()
        };
        let mut total = 0usize;
        for item in doc.get("segments").map_or(&[][..], Json::arr) {
            let number = |key| {
                item.get(key)
                    .and_then(Json::as_num)
                    .filter(|n| n.is_finite() && n.fract() == 0. && *n >= 0.)
                    .map(|n| n as u64)
                    .ok_or("invalid overlay number")
            };
            let start = number("offset")?;
            let len = number("length")?;
            if len == 0 || len > 64 << 20 || start % 2048 != 0 || len % 2048 != 0 {
                return Err("invalid overlay extent".into());
            }
            let path = relative(
                &cache,
                item.get("file")
                    .and_then(Json::str)
                    .ok_or("missing overlay file")?,
            )
            .ok_or("unsafe overlay path")?;
            if path.metadata().map_err(|e| e.to_string())?.len() != len {
                return Err("overlay size mismatch".into());
            }
            let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
            if bytes.len() as u64 != len {
                return Err("overlay size mismatch".into());
            }
            total = total
                .checked_add(bytes.len())
                .ok_or("overlay memory overflow")?;
            if total > 256 << 20 {
                return Err("localization overlay exceeds 256 MiB".into());
            }
            data.segments.push(Segment {
                start,
                end: start.checked_add(len).ok_or("overlay extent overflow")?,
                data: bytes,
            });
        }
        data.segments.sort_by_key(|s| s.start);
        if data.segments.windows(2).any(|w| w[0].end > w[1].start) {
            return Err("overlapping localization extents".into());
        }
        if let Some(j) = read_json(&cache.join("ui.json")) {
            for (source, value) in j.entries() {
                if let Some(value) = value.str() {
                    data.ui.insert(source.clone(), value.into());
                }
            }
        }
        if let (Some(keys), Some(translations)) = (
            read_json(&dir.join("ui-extra.json")),
            read_json(&pack.strings),
        ) {
            if let Some(strings) = translations.get("strings") {
                for (source, key) in keys.entries() {
                    if let Some(value) = key
                        .str()
                        .and_then(|key| strings.get(key))
                        .and_then(Json::str)
                    {
                        data.ui.insert(source.clone(), value.into());
                    }
                }
            }
        }
        eprintln!(
            "localization: {id}: {} sector overlays, {} UI strings; charset test {diagnostic}; no recurring RAM/text scans",
            data.segments.len(),
            data.ui.len()
        );
        Ok(data)
    };
    run().unwrap_or_else(|e| {
        eprintln!("localization: {e}; original disc fallback");
        Data::default()
    })
}
fn data() -> &'static Data {
    DATA.get_or_init(load)
}
/// The successfully loaded provider for this process; malformed data disables all its hooks.
pub fn active_pack() -> Option<&'static Pack> {
    if data().enabled {
        pack::active_pack()
    } else {
        None
    }
}
pub fn hd_cards() -> bool {
    data().hd_cards
}
pub fn ui_text<'a>(text: &'a str) -> &'a str {
    data().ui.get(text).map_or(text, String::as_str)
}
pub fn filter(offset: u64, bytes: &mut [u8]) {
    let d = data();
    let Some(end) = offset.checked_add(bytes.len() as u64) else {
        return;
    };
    let first = d.segments.partition_point(|s| s.end <= offset);
    for s in &d.segments[first..] {
        if s.start >= end {
            break;
        }
        let a = offset.max(s.start);
        let b = end.min(s.end);
        bytes[(a - offset) as usize..(b - offset) as usize]
            .copy_from_slice(&s.data[(a - s.start) as usize..(b - s.start) as usize]);
    }
}

/// Optional language-specific number forms; all wording lives in the external pack.
pub fn count_form(key: &str, n: u32) -> Option<&'static str> {
    let p = active_pack()?;
    let forms = p.manifest.get("count_forms")?.get(key)?;
    let exact = n.to_string();
    forms.get(&exact).or_else(|| forms.get("other"))?.str()
}
pub fn thousands_separator() -> &'static str {
    active_pack()
        .and_then(|p| p.text("thousands_separator"))
        .unwrap_or(",")
}
pub fn overlay_letter(ch: char) -> Option<(u8, [u8; 2])> {
    static GLYPHS: OnceLock<BTreeMap<char, (u8, [u8; 2])>> = OnceLock::new();
    GLYPHS
        .get_or_init(|| {
            let mut out = BTreeMap::new();
            if let Some(doc) = active_pack()
                .and_then(|p| p.file("overlay_glyphs", "overlay-glyphs.json"))
                .and_then(|p| read_json(&p))
            {
                for (key, v) in doc.entries() {
                    let mut chars = key.chars();
                    let Some(ch) = chars.next().filter(|_| chars.next().is_none()) else {
                        continue;
                    };
                    let Some(base) = v
                        .get("base")
                        .and_then(Json::str)
                        .filter(|s| s.len() == 1 && s.is_ascii())
                    else {
                        continue;
                    };
                    let mark = v.get("mark").map_or(&[][..], Json::arr);
                    if mark.len() != 2 {
                        continue;
                    }
                    let number = |i: usize| {
                        mark[i]
                            .as_num()
                            .filter(|n| {
                                n.is_finite() && n.fract() == 0. && (0.0..=31.0).contains(n)
                            })
                            .map(|n| n as u8)
                    };
                    if let (Some(a), Some(b)) = (number(0), number(1)) {
                        out.insert(ch, (base.as_bytes()[0], [a, b]));
                    }
                }
            }
            out
        })
        .get(&ch)
        .copied()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_absolute_parent_and_platform_paths() {
        for path in ["../secret", "/absolute", "C:/absolute", r"\\server\share"] {
            assert!(relative(Path::new("pack"), path).is_none(), "{path}");
        }
        assert!(relative(Path::new("missing-pack"), "font/layout.json").is_some());
    }
}
