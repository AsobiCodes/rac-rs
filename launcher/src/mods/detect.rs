//! Recognising what a folder or zip holds: a launcher mod (mod.json) or a PCSX2 texture pack.
//!
//! PCSX2 texture replacements (GSTextureReplacements.cpp, `ParseReplacementName`): files in `<Documents>\PCSX2\textures\<SERIAL>\replacements\`
//! (read recursively; a `<SERIAL>.zip` in the textures folder works too), named
//!   `<TEX0 hash>[-<CLUT hash>][-r<W>x<H>]-<bits>.<ext>`   (hashes: hex, up to 16 digits; bits: hex, up to 8; W/H decimal)
//!   and the older `...-r<hex>-<bits>` region form; extensions png or dds (case-insensitive; the loader list is {png, dds}).
//! A `-mipN` suffix is not a loadable name (PCSX2 reads mips inside the dds).
use super::ModInfo;

/// One file of a source (a folder or a zip), path relative to the source root with `/`.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub path: String,
    /// The name as the zip stores it (to open it again); equals `path` for folders.
    pub raw: String,
    pub size: u64,
}

fn is_hex(s: &str, max: usize) -> bool {
    !s.is_empty() && s.len() <= max && s.bytes().all(|b| b.is_ascii_hexdigit())
}

/// `r<W>x<H>` (decimal) or the old `r<hex>` region field.
fn is_region(s: &str) -> bool {
    let Some(r) = s.strip_prefix('r') else { return false };
    if let Some((w, h)) = r.split_once('x') {
        return !w.is_empty() && !h.is_empty() && w.bytes().all(|b| b.is_ascii_digit()) && h.bytes().all(|b| b.is_ascii_digit());
    }
    is_hex(r, 16)
}

/// The file name (no folders) of a loadable PCSX2 replacement texture.
pub fn texture_name_ok(file_name: &str) -> bool {
    let Some((stem, ext)) = file_name.rsplit_once('.') else { return false };
    if !(ext.eq_ignore_ascii_case("png") || ext.eq_ignore_ascii_case("dds")) {
        return false;
    }
    let p: Vec<&str> = stem.split('-').collect();
    match p.as_slice() {
        [h, bits] => is_hex(h, 16) && is_hex(bits, 8),
        [h, mid, bits] => is_hex(h, 16) && (is_hex(mid, 16) || is_region(mid)) && is_hex(bits, 8),
        [h, clut, reg, bits] => is_hex(h, 16) && is_hex(clut, 16) && is_region(reg) && is_hex(bits, 8),
        _ => false,
    }
}

/// `SCES-50916` style.
pub fn is_serial(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() == 10 && b[..4].iter().all(|c| c.is_ascii_alphabetic()) && b[4] == b'-' && b[5..].iter().all(|c| c.is_ascii_digit())
}

#[derive(Clone, Debug, PartialEq)]
pub enum Layout {
    /// A mod with its own mod.json; `prefix` is its folder inside the source ("" = the root).
    ModJson { prefix: String },
    /// A PCSX2 texture pack; the files are those under `prefix` (all files for a replacements folder, only loadable names for loose files).
    Pcsx2 { prefix: String, serial: Option<String>, loose: bool },
    Unknown,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Detected {
    pub layout: Layout,
    /// The files that will be copied, relative to the layout's prefix.
    pub files: Vec<(Entry, String)>,
    pub bytes: u64,
    /// Name suggestion (the source's name without extension).
    pub name_hint: String,
}

fn under(path: &str, prefix: &str) -> Option<String> {
    if prefix.is_empty() {
        return Some(path.to_string());
    }
    path.strip_prefix(prefix).and_then(|r| r.strip_prefix('/')).map(|s| s.to_string())
}

fn file_name(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

fn collect(entries: &[Entry], prefix: &str, only_textures: bool) -> Vec<(Entry, String)> {
    entries
        .iter()
        .filter_map(|e| under(&e.path, prefix).map(|r| (e.clone(), r)))
        .filter(|(_, r)| !only_textures || texture_name_ok(file_name(r)))
        .collect()
}

fn best_prefix(entries: &[Entry], want: impl Fn(&[&str]) -> Option<(usize, Option<String>)>) -> Option<(String, Option<String>)> {
    use std::collections::HashMap;
    let mut count: HashMap<(String, Option<String>), usize> = HashMap::new();
    for e in entries {
        let comps: Vec<&str> = e.path.split('/').collect();
        if let Some((end, serial)) = want(&comps[..comps.len().saturating_sub(1)]) {
            if comps.len() > end + 1 {
                *count.entry((comps[..=end].join("/"), serial)).or_default() += 1;
            }
        }
    }
    count.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0 .0.cmp(&a.0 .0))).map(|(k, _)| k)
}

/// Looks at the file list of a source and decides what it is.
pub fn analyze(entries: &[Entry], source_name: &str) -> Detected {
    let hint = source_name.rsplit_once('.').filter(|(_, e)| e.eq_ignore_ascii_case("zip")).map(|(s, _)| s).unwrap_or(source_name).to_string();
    let none = |layout| Detected { layout, files: vec![], bytes: 0, name_hint: hint.clone() };
    let done = |layout: Layout, files: Vec<(Entry, String)>| {
        let bytes = files.iter().map(|(e, _)| e.size).sum();
        Detected { layout, files, bytes, name_hint: hint.clone() }
    };

    // 1. mod.json at the root or one folder down (the shallowest)
    let mj = entries.iter().filter(|e| file_name(&e.path).eq_ignore_ascii_case("mod.json") && e.path.matches('/').count() <= 1).min_by_key(|e| e.path.len());
    if let Some(m) = mj {
        let prefix = m.path.rsplit_once('/').map(|(p, _)| p.to_string()).unwrap_or_default();
        let files = collect(entries, &prefix, false);
        return done(Layout::ModJson { prefix }, files);
    }
    // the serial can also come from the source name (SCES-50916.zip, a folder called SCES-50916)
    let name_serial = is_serial(&hint).then(|| hint.to_ascii_uppercase());
    // 2. <SERIAL>/replacements/
    let by_serial = best_prefix(entries, |c| (0..c.len().saturating_sub(1)).find(|&i| is_serial(c[i]) && c[i + 1].eq_ignore_ascii_case("replacements")).map(|i| (i + 1, Some(c[i].to_ascii_uppercase()))));
    // 3. any folder called replacements
    let by_name = best_prefix(entries, |c| {
        c.iter().position(|x| x.eq_ignore_ascii_case("replacements")).map(|i| (i, i.checked_sub(1).and_then(|p| is_serial(c[p]).then(|| c[p].to_ascii_uppercase())).or_else(|| name_serial.clone())))
    });
    if let Some((prefix, serial)) = by_serial.or(by_name) {
        let files = collect(entries, &prefix, false);
        return done(Layout::Pcsx2 { prefix, serial, loose: false }, files);
    }
    // 4. loose texture files: the folder that holds the most of them
    let loose = best_prefix_dirs(entries);
    if let Some(prefix) = loose {
        let files = collect(entries, &prefix, true);
        let serial = name_serial.or_else(|| prefix.split('/').find(|c| is_serial(c)).map(|c| c.to_ascii_uppercase()));
        return done(Layout::Pcsx2 { prefix, serial, loose: true }, files);
    }
    none(Layout::Unknown)
}

fn best_prefix_dirs(entries: &[Entry]) -> Option<String> {
    use std::collections::HashMap;
    let mut count: HashMap<String, usize> = HashMap::new();
    for e in entries.iter().filter(|e| texture_name_ok(file_name(&e.path))) {
        let dir = e.path.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
        *count.entry(dir).or_default() += 1;
    }
    count.into_iter().max_by(|a, b| a.1.cmp(&b.1).then_with(|| b.0.cmp(&a.0))).map(|(k, _)| k)
}

/// The mod.json a texture pack gets when it has none.
pub fn make_info(id: &str, name: &str, d: &Detected) -> ModInfo {
    let serial = match &d.layout {
        Layout::Pcsx2 { serial, .. } => serial.clone(),
        _ => None,
    };
    ModInfo {
        format: 1,
        id: id.to_string(),
        name: name.to_string(),
        author: String::new(),
        version: "1.0".to_string(),
        description: format!("PCSX2 texture pack, {} files.", d.files.len()),
        game: "rac1".to_string(),
        kind: "textures".to_string(),
        serials: serial.into_iter().collect(),
        source: "pcsx2".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn e(p: &str) -> Entry {
        Entry { path: p.to_string(), raw: p.to_string(), size: 10 }
    }
    fn many(prefix: &str, n: usize) -> Vec<Entry> {
        (0..n).map(|i| e(&format!("{prefix}{:016x}-{:08x}.png", 0xabcdef0000u64 + i as u64, 7))).collect()
    }

    #[test]
    fn names_follow_pcsx2s_grammar() {
        for ok in [
            "0123456789abcdef-00000007.png",
            "0123456789abcdef-89abcdef01234567-00000007.dds",
            "0123456789abcdef-r64x32-00000007.PNG",
            "0123456789abcdef-89abcdef01234567-r64x32-00000007.png",
            "0123456789abcdef-r4000000000-00000007.png",
            "1-2.png",
        ] {
            assert!(texture_name_ok(ok), "{ok}");
        }
        for bad in ["0123456789abcdef-00000007-mip1.png", "readme.txt", "0123456789abcdef-00000007.jpg", "0123456789abcdef.png", "x-00000007.png", "0123456789abcdef-123456789.png", "a-b-c-d-e.png"] {
            assert!(!texture_name_ok(bad), "{bad}");
        }
    }

    #[test]
    fn pcsx2_folder_layout_with_serial() {
        let mut v = many("SCES-50916/replacements/", 3);
        v.push(e("SCES-50916/replacements/sub/0123456789abcdef-00000007.dds"));
        v.push(e("SCES-50916/readme.txt"));
        let d = analyze(&v, "MyPack.zip");
        assert_eq!(d.layout, Layout::Pcsx2 { prefix: "SCES-50916/replacements".into(), serial: Some("SCES-50916".into()), loose: false });
        assert_eq!(d.files.len(), 4);
        assert!(d.files.iter().any(|(_, r)| r == "sub/0123456789abcdef-00000007.dds"), "subfolders kept");
        assert_eq!(d.name_hint, "MyPack");
        assert_eq!(d.bytes, 40);
    }

    #[test]
    fn a_zip_wrapped_in_one_top_folder() {
        let v = many("Pack v2/SCUS-97199/replacements/", 2);
        let d = analyze(&v, "x.zip");
        assert_eq!(d.layout, Layout::Pcsx2 { prefix: "Pack v2/SCUS-97199/replacements".into(), serial: Some("SCUS-97199".into()), loose: false });
    }

    #[test]
    fn replacements_folder_without_serial_takes_it_from_the_name() {
        let v = many("replacements/", 2);
        assert_eq!(analyze(&v, "SCES-50916.zip").layout, Layout::Pcsx2 { prefix: "replacements".into(), serial: Some("SCES-50916".into()), loose: false });
        assert_eq!(analyze(&v, "pack").layout, Layout::Pcsx2 { prefix: "replacements".into(), serial: None, loose: false });
        let w = many("Textures/Replacements/", 2);
        assert!(matches!(analyze(&w, "p").layout, Layout::Pcsx2 { loose: false, .. }), "case-insensitive");
    }

    #[test]
    fn loose_files_and_unknown() {
        let mut v = many("", 3);
        v.push(e("notes.txt"));
        let d = analyze(&v, "loose");
        assert_eq!(d.layout, Layout::Pcsx2 { prefix: String::new(), serial: None, loose: true });
        assert_eq!(d.files.len(), 3, "only loadable names");
        let v2 = many("some/where/", 5);
        let d2 = analyze(&v2, "loose");
        assert_eq!(d2.layout, Layout::Pcsx2 { prefix: "some/where".into(), serial: None, loose: true });
        assert_eq!(analyze(&[e("a.txt"), e("b/c.doc")], "x").layout, Layout::Unknown);
        assert_eq!(analyze(&[], "x").layout, Layout::Unknown);
    }

    #[test]
    fn mod_json_wins_at_root_or_one_folder_down() {
        let v = vec![e("mod.json"), e("textures/0123456789abcdef-00000007.png")];
        assert_eq!(analyze(&v, "m").layout, Layout::ModJson { prefix: String::new() });
        let w = vec![e("cool-pack/mod.json"), e("cool-pack/textures/0123456789abcdef-00000007.png"), e("other.txt")];
        let d = analyze(&w, "m.zip");
        assert_eq!(d.layout, Layout::ModJson { prefix: "cool-pack".into() });
        assert_eq!(d.files.len(), 2);
    }

    #[test]
    fn generated_info() {
        let d = analyze(&many("SCES-50916/replacements/", 2), "Pack");
        let i = make_info("pack", "Pack", &d);
        assert_eq!((i.kind.as_str(), i.source.as_str(), i.game.as_str()), ("textures", "pcsx2", "rac1"));
        assert_eq!(i.serials, vec!["SCES-50916".to_string()]);
    }
}
