//! Adding a mod from a .zip or a folder (and from PCSX2's own textures folder): list, detect, copy on a background thread with progress.
use super::detect::{analyze, make_info, Detected, Entry, Layout};
use super::{slug, valid_id, ModInfo};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub enum Source {
    Dir(PathBuf),
    Zip(PathBuf),
}

impl Source {
    pub fn path(&self) -> &Path {
        match self {
            Source::Dir(p) | Source::Zip(p) => p,
        }
    }

    pub fn name(&self) -> String {
        self.path().file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default()
    }
}

/// Picks the source kind from a path: Err = a message for the player (7z, rar ... "extract it first").
pub fn source_of(p: &Path) -> Result<Source, String> {
    if p.is_dir() {
        return Ok(Source::Dir(p.to_path_buf()));
    }
    if !p.is_file() {
        return Err("That file or folder could not be found.".to_string());
    }
    match p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref() {
        Some("zip") => Ok(Source::Zip(p.to_path_buf())),
        Some("7z") | Some("rar") | Some("tar") | Some("gz") | Some("xz") | Some("bz2") | Some("zst") => {
            Err("This launcher opens .zip files and folders only. Extract the archive first (for example with 7-Zip), then add the folder.".to_string())
        }
        _ => Err("Pick a .zip file or a folder.".to_string()),
    }
}

/// Progress of reading a source (listing its files), shared with the thread that does it.
#[derive(Default)]
pub struct ReadState {
    /// Files found so far.
    pub entries: AtomicU64,
    pub cancel: AtomicBool,
    /// Tests only: the worker waits while this is set (proves the caller does not wait for the worker).
    hold: AtomicBool,
    result: Mutex<Option<Result<Candidate, String>>>,
}

impl ReadState {
    pub fn is_done(&self) -> bool {
        self.result.lock().map(|r| r.is_some()).unwrap_or(false)
    }

    /// The finished result, once (None while the worker is still reading).
    pub fn take_result(&self) -> Option<Result<Candidate, String>> {
        self.result.lock().ok().and_then(|mut r| r.take())
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

const CANCELLED: &str = "Cancelled.";

/// Counts the files found (every 256th is enough for a progress text) and tells the walk to stop when the player cancelled.
fn tick(rs: Option<&ReadState>, n: usize) -> bool {
    let Some(rs) = rs else { return false };
    if n % 256 == 0 {
        rs.entries.store(n as u64, Ordering::Relaxed);
        return rs.cancelled();
    }
    false
}

fn walk(dir: &Path, base: &Path, out: &mut Vec<Entry>, rs: Option<&ReadState>) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else { return false };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if walk(&p, base, out, rs) {
                return true;
            }
        } else if ft.is_file() {
            let rel = p.strip_prefix(base).unwrap_or(&p).to_string_lossy().replace('\\', "/");
            out.push(Entry { path: rel.clone(), raw: rel, size: e.metadata().map(|m| m.len()).unwrap_or(0) });
            if tick(rs, out.len()) {
                return true;
            }
        }
    }
    false
}

/// A .zip opened for reading.
/// The zip crate's default (`ArchiveOffset::Detect`) scans the WHOLE file for the central directory's signature, to cope with junk in front
/// of the archive: on the 6.4 GB SCUS-97199 pack that is 15 to 90 seconds of reading before the first entry. Offset 0 (every normal .zip) is
/// checked with one read; only when that fails the slow detection is tried (a .zip with a self-extractor in front).
fn open_zip(z: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let open = || std::fs::File::open(z).map_err(|e| format!("The zip could not be opened: {e}"));
    let quick = zip::read::Config { archive_offset: zip::read::ArchiveOffset::Known(0) };
    match zip::ZipArchive::with_config(quick, open()?) {
        Ok(a) => Ok(a),
        Err(_) => zip::ZipArchive::new(open()?).map_err(|e| format!("This is not a readable .zip file: {e}")),
    }
}

/// All files of a source (folders are not listed).
pub fn list(src: &Source) -> Result<Vec<Entry>, String> {
    list_with(src, None)
}

fn list_with(src: &Source, rs: Option<&ReadState>) -> Result<Vec<Entry>, String> {
    match src {
        Source::Dir(d) => {
            let mut v = Vec::new();
            if walk(d, d, &mut v, rs) {
                return Err(CANCELLED.to_string());
            }
            Ok(v)
        }
        Source::Zip(z) => {
            let a = open_zip(z)?;
            let mut v = Vec::new();
            for i in 0..a.len() {
                // by_index_data reads the central directory only. (by_index_raw also reads each file's local header = one random read
                // per entry in a 6 GB .zip: 16 s for the 20,000 files of the SCUS-97199 pack, minutes on a hard disk.)
                let Ok(f) = a.by_index_data(i) else { continue };
                if f.is_dir() {
                    continue;
                }
                let raw = f.name().map(|c| c.to_string()).unwrap_or_default();
                v.push(Entry { path: raw.replace('\\', "/").trim_start_matches('/').to_string(), raw, size: f.size() });
                if tick(rs, v.len()) {
                    return Err(CANCELLED.to_string());
                }
            }
            Ok(v)
        }
    }
}

/// Source + what it holds, ready to show ("Add mod" preview).
#[derive(Clone, Debug)]
pub struct Candidate {
    pub source: Source,
    pub detected: Detected,
    /// Read from mod.json when the pack brings one.
    pub info: Option<ModInfo>,
}

pub fn inspect(src: Source) -> Result<Candidate, String> {
    inspect_with(src, None)
}

fn inspect_with(src: Source, rs: Option<&ReadState>) -> Result<Candidate, String> {
    let entries = list_with(&src, rs)?;
    let detected = analyze(&entries, &src.name());
    let info = match &detected.layout {
        Layout::ModJson { prefix } => read_mod_json(&src, prefix, &entries)?,
        Layout::Unknown => {
            return Err("This does not look like a mod or a PCSX2 texture pack. A texture pack holds a folder called replacements (or files named like 0123456789abcdef-89abcdef.png).".to_string())
        }
        _ => None,
    };
    if detected.files.is_empty() {
        return Err("No files were found in it.".to_string());
    }
    Ok(Candidate { source: src, detected, info })
}

fn read_mod_json(src: &Source, prefix: &str, entries: &[Entry]) -> Result<Option<ModInfo>, String> {
    let want = if prefix.is_empty() { "mod.json".to_string() } else { format!("{prefix}/mod.json") };
    let Some(e) = entries.iter().find(|e| e.path.eq_ignore_ascii_case(&want)) else { return Ok(None) };
    let bytes = match src {
        Source::Dir(d) => std::fs::read(d.join(&e.raw)).map_err(|e| e.to_string())?,
        Source::Zip(z) => {
            let mut a = open_zip(z)?;
            let mut f = a.by_name(&e.raw).map_err(|e| e.to_string())?;
            let mut b = Vec::new();
            f.read_to_end(&mut b).map_err(|e| e.to_string())?;
            b
        }
    };
    let info: ModInfo = serde_json::from_slice(&bytes).map_err(|e| format!("mod.json could not be read: {e}"))?;
    if !valid_id(&info.id) {
        return Err(format!("mod.json has an id that cannot be used as a folder name ({:?}); it must be lower-case letters, digits, . _ -", info.id));
    }
    Ok(Some(info))
}

/// Looks at a dropped or picked file/folder on its own thread (a 6 GB .zip with 20,000 files takes a while to open): the caller polls `take_result`.
pub fn start_inspect(path: PathBuf) -> Arc<ReadState> {
    spawn_inspect(path, false)
}

/// Like `start_inspect`, but the worker waits until `hold` is cleared (tests).
pub fn start_inspect_held(path: PathBuf) -> Arc<ReadState> {
    spawn_inspect(path, true)
}

fn spawn_inspect(path: PathBuf, hold: bool) -> Arc<ReadState> {
    let st = Arc::new(ReadState::default());
    st.hold.store(hold, Ordering::Relaxed);
    let st2 = st.clone();
    std::thread::spawn(move || {
        while st2.hold.load(Ordering::Relaxed) && !st2.cancelled() {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let r = source_of(&path).and_then(|s| inspect_with(s, Some(&st2)));
        if let Ok(mut g) = st2.result.lock() {
            *g = Some(r);
        }
    });
    st
}

impl ReadState {
    /// Lets a held worker go (tests).
    pub fn release(&self) {
        self.hold.store(false, Ordering::Relaxed);
    }
}

/// Progress of a copy, shared with the thread.
#[derive(Default)]
pub struct ImportState {
    pub files_total: AtomicU64,
    pub files_done: AtomicU64,
    pub bytes_total: AtomicU64,
    pub bytes_done: AtomicU64,
    /// Files left out (unusable name, or a second file that maps to the same path as an earlier one); they still count in files_done.
    pub skipped: AtomicU64,
    pub cancel: AtomicBool,
    pub current: Mutex<String>,
    /// None while running; Ok(mod id) or Err(message).
    pub result: Mutex<Option<Result<String, String>>>,
}

impl ImportState {
    pub fn percent(&self) -> f32 {
        let t = self.bytes_total.load(Ordering::Relaxed).max(1) as f32;
        (self.bytes_done.load(Ordering::Relaxed) as f32 / t).min(1.0)
    }

    pub fn result(&self) -> Option<Result<String, String>> {
        self.result.lock().ok().and_then(|r| r.clone())
    }
}

/// Windows device names: a file or folder with one of these names (also `NUL.png`, any case) cannot be created.
fn is_reserved_name(c: &str) -> bool {
    let stem = c.split('.').next().unwrap_or(c).trim_end_matches(' ').to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        || (stem.len() == 4 && (stem.starts_with("COM") || stem.starts_with("LPT")) && matches!(stem.as_bytes()[3], b'1'..=b'9'))
}

/// One path part made safe for the file system, or None when it cannot be (`.`/`..`, a drive or stream mark `:`, nothing left).
/// Windows drops trailing spaces and dots when it creates a folder (so `A /f.png` would not be found again): they are trimmed here.
/// Characters Windows does not allow become `_`, device names get a `_` in front. A leading space is legal and stays.
fn safe_component(c: &str) -> Option<String> {
    if c == "." || c == ".." || c.contains(':') {
        return None;
    }
    let c = c.trim_end_matches([' ', '.']);
    if c.is_empty() {
        return None;
    }
    let mut s: String = c.chars().map(|ch| if ch.is_control() || matches!(ch, '<' | '>' | '"' | '|' | '?' | '*') { '_' } else { ch }).collect();
    if is_reserved_name(&s) {
        s.insert(0, '_');
    }
    Some(s)
}

/// Relative path that stays inside the folder (no `..`, no drive, no empty parts) and can be created on Windows (see `safe_component`).
fn safe_rel(rel: &str) -> Option<PathBuf> {
    let mut p = PathBuf::new();
    for c in rel.split(['/', '\\']) {
        p.push(safe_component(c)?);
    }
    (!p.as_os_str().is_empty()).then_some(p)
}

/// The id of the new mod and the info that will be written (name/id of a mod.json pack are kept; the id gets a suffix when taken).
pub fn plan_info(c: &Candidate, existing: impl Fn(&str) -> bool, name_override: Option<&str>) -> ModInfo {
    let free = |base: &str| -> String {
        if !existing(base) {
            return base.to_string();
        }
        (2..).map(|n| format!("{base}-{n}")).find(|x| !existing(x)).unwrap_or_else(|| base.to_string())
    };
    match &c.info {
        Some(i) => {
            let mut i = i.clone();
            i.id = free(&i.id);
            if let Some(n) = name_override.filter(|n| !n.trim().is_empty()) {
                i.name = n.trim().to_string();
            }
            i
        }
        None => {
            let name = name_override.filter(|n| !n.trim().is_empty()).map(|n| n.trim().to_string()).unwrap_or_else(|| c.detected.name_hint.clone());
            let name = if name.is_empty() { "Texture pack".to_string() } else { name };
            make_info(&free(&slug(&name)), &name, &c.detected)
        }
    }
}

/// Copies the pack into `<mods>\<id>.tmp`, writes mod.json, then renames to `<mods>\<id>`. Runs on its own thread.
pub fn start(c: Candidate, info: ModInfo, mods_dir: PathBuf) -> Arc<ImportState> {
    let st = Arc::new(ImportState::default());
    st.files_total.store(c.detected.files.len() as u64, Ordering::Relaxed);
    st.bytes_total.store(c.detected.bytes, Ordering::Relaxed);
    let st2 = st.clone();
    std::thread::spawn(move || {
        let r = run(&c, &info, &mods_dir, &st2);
        if r.is_err() {
            let _ = std::fs::remove_dir_all(mods_dir.join(format!("{}.tmp", info.id)));
        }
        if let Ok(mut g) = st2.result.lock() {
            *g = Some(r.map(|_| info.id.clone()));
        }
    });
    st
}

fn run(c: &Candidate, info: &ModInfo, mods_dir: &Path, st: &ImportState) -> Result<(), String> {
    let tmp = mods_dir.join(format!("{}.tmp", info.id));
    let fin = mods_dir.join(&info.id);
    if fin.exists() {
        return Err("A mod with that id is already installed.".to_string());
    }
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).map_err(|e| format!("Could not create the mod folder: {e}"))?;
    let modjson = matches!(c.detected.layout, Layout::ModJson { .. });
    let mut zip = match &c.source {
        Source::Zip(z) => Some(open_zip(z)?),
        Source::Dir(_) => None,
    };
    let prefix = match &c.detected.layout {
        Layout::ModJson { prefix } | Layout::Pcsx2 { prefix, .. } => prefix.clone(),
        Layout::Unknown => String::new(),
    };
    // Cleaned-up paths already written (Windows ignores case): trimming a name can make two files land on the same path; the first one wins.
    let mut taken = std::collections::HashSet::new();
    for (e, rel) in &c.detected.files {
        if st.cancel.load(Ordering::Relaxed) {
            return Err("Cancelled.".to_string());
        }
        if modjson && rel.eq_ignore_ascii_case("mod.json") {
            st.files_done.fetch_add(1, Ordering::Relaxed);
            continue; // rewritten below
        }
        let rel_path = safe_rel(rel).filter(|p| taken.insert(if cfg!(windows) { p.to_string_lossy().to_lowercase() } else { p.to_string_lossy().to_string() }));
        let Some(rel_path) = rel_path else {
            st.skipped.fetch_add(1, Ordering::Relaxed);
            st.files_done.fetch_add(1, Ordering::Relaxed);
            st.bytes_done.fetch_add(e.size, Ordering::Relaxed);
            continue;
        };
        let dest = if modjson { tmp.join(rel_path) } else { tmp.join("textures").join(rel_path) };
        if let Some(p) = dest.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        if let Ok(mut g) = st.current.lock() {
            *g = rel.clone();
        }
        let mut out = std::fs::File::create(&dest).map_err(|e| format!("Could not write {}: {e}", dest.display()))?;
        match (&c.source, zip.as_mut()) {
            (Source::Zip(_), Some(a)) => {
                let mut f = a.by_name(&e.raw).map_err(|e| e.to_string())?;
                copy_counted(&mut f, &mut out, st)?;
            }
            (Source::Dir(d), _) => {
                let mut f = std::fs::File::open(d.join(&e.raw)).map_err(|e| e.to_string())?;
                copy_counted(&mut f, &mut out, st)?;
            }
            _ => {}
        }
        st.files_done.fetch_add(1, Ordering::Relaxed);
    }
    let _ = prefix;
    let json = serde_json::to_string_pretty(info).map_err(|e| e.to_string())?;
    std::fs::write(tmp.join("mod.json"), json + "\n").map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, &fin).map_err(|e| format!("Could not finish the mod folder: {e}"))
}

fn copy_counted(r: &mut impl Read, w: &mut impl Write, st: &ImportState) -> Result<(), String> {
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if st.cancel.load(Ordering::Relaxed) {
            return Err("Cancelled.".to_string());
        }
        let n = r.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            return Ok(());
        }
        w.write_all(&buf[..n]).map_err(|e| e.to_string())?;
        st.bytes_done.fetch_add(n as u64, Ordering::Relaxed);
    }
}

/// PCSX2's own textures folders: `<Documents>\PCSX2\textures\<SERIAL>\` (or `<SERIAL>.zip`) for the serials we know.
pub fn pcsx2_candidates(documents: &Path, serials: &[&str]) -> Vec<(String, Source)> {
    let base = documents.join("PCSX2").join("textures");
    let mut v = Vec::new();
    for s in serials {
        let d = base.join(s);
        if d.is_dir() {
            v.push((s.to_string(), Source::Dir(d)));
        }
        let z = base.join(format!("{s}.zip"));
        if z.is_file() {
            v.push((s.to_string(), Source::Zip(z)));
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_imp_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn make_pack(root: &Path, n: usize) {
        let r = root.join("SCES-50916").join("replacements");
        std::fs::create_dir_all(r.join("sub")).unwrap();
        for i in 0..n {
            std::fs::write(r.join(format!("{:016x}-00000007.png", 0xaa00u64 + i as u64)), vec![i as u8; 100]).unwrap();
        }
        std::fs::write(r.join("sub").join("0123456789abcdef-00000009.dds"), b"dds").unwrap();
    }

    fn wait(st: &ImportState) -> Result<String, String> {
        for _ in 0..600 {
            if let Some(r) = st.result() {
                return r;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        panic!("import did not finish");
    }

    #[test]
    fn folder_import_makes_mod_json_and_copies_with_subfolders() {
        let d = scratch("dir");
        make_pack(&d.join("src"), 5);
        let src = source_of(&d.join("src").join("SCES-50916")).unwrap();
        let c = inspect(src).unwrap();
        assert_eq!(c.detected.files.len(), 6);
        let info = plan_info(&c, |_| false, None);
        assert_eq!(info.id, "sces-50916");
        assert_eq!(info.serials, vec!["SCES-50916"]);
        let mods = d.join("mods");
        let st = start(c, info, mods.clone());
        assert_eq!(wait(&st), Ok("sces-50916".to_string()));
        assert!(mods.join("sces-50916/textures/sub/0123456789abcdef-00000009.dds").is_file());
        assert_eq!(std::fs::read_dir(mods.join("sces-50916/textures")).unwrap().count(), 6);
        let j: serde_json::Value = serde_json::from_slice(&std::fs::read(mods.join("sces-50916/mod.json")).unwrap()).unwrap();
        assert_eq!((j["kind"].as_str(), j["game"].as_str(), j["source"].as_str(), j["format"].as_u64()), (Some("textures"), Some("rac1"), Some("pcsx2"), Some(1)));
        assert!(!mods.join("sces-50916.tmp").exists());
        assert_eq!(st.files_done.load(Ordering::Relaxed), 6);
        assert!((st.percent() - 1.0).abs() < 1e-6);
        // the store sees it
        assert_eq!(super::super::ModStore::load(&mods).entries.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn zip_import_and_traversal_is_refused() {
        let d = scratch("zip");
        let zp = d.join("Pack.zip");
        {
            let f = std::fs::File::create(&zp).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let o = zip::write::SimpleFileOptions::default();
            w.start_file("SCUS-97199/replacements/0123456789abcdef-00000007.png", o).unwrap();
            w.write_all(b"pngdata").unwrap();
            w.start_file("SCUS-97199/replacements/../../evil-0123456789abcdef-00000007.png", o).unwrap();
            w.write_all(b"x").unwrap();
            w.finish().unwrap();
        }
        let c = inspect(source_of(&zp).unwrap()).unwrap();
        assert!(matches!(&c.detected.layout, Layout::Pcsx2 { serial: Some(s), .. } if s == "SCUS-97199"));
        let info = plan_info(&c, |x| x == "pack", Some("My Pack"));
        assert_eq!(info.id, "my-pack");
        let mods = d.join("mods");
        let st = start(c, info, mods.clone());
        assert_eq!(wait(&st), Ok("my-pack".to_string()));
        assert_eq!(std::fs::read(mods.join("my-pack/textures/0123456789abcdef-00000007.png")).unwrap(), b"pngdata");
        assert!(!d.join("evil-0123456789abcdef-00000007.png").exists() && !mods.join("evil-0123456789abcdef-00000007.png").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn safe_rel_trims_trailing_spaces_and_dots_and_fixes_windows_names() {
        let s = |r: &str| safe_rel(r).map(|p| p.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join("/"));
        assert_eq!(s("LEVELS/A /f.png").as_deref(), Some("LEVELS/A/f.png"));
        assert_eq!(s("LEVELS/B../f.png").as_deref(), Some("LEVELS/B/f.png"));
        assert_eq!(s("LEVELS/ C/f.png").as_deref(), Some("LEVELS/ C/f.png"), "a leading space is legal");
        assert_eq!(s("a<b>c\"d|e?f*g\u{1}h/f.png").as_deref(), Some("a_b_c_d_e_f_g_h/f.png"));
        for (name, want) in [("NUL.png", "_NUL.png"), ("nul", "_nul"), ("Con.tar.gz", "_Con.tar.gz"), ("COM1.png", "_COM1.png"), ("lpt9", "_lpt9"), ("AUX .png", "_AUX .png")] {
            assert_eq!(s(&format!("x/{name}")).as_deref(), Some(format!("x/{want}").as_str()), "{name}");
        }
        assert_eq!(s("x/NULL.png").as_deref(), Some("x/NULL.png"));
        assert_eq!(s("x/COM0.png").as_deref(), Some("x/COM0.png"));
        assert_eq!(s("x/COM10.png").as_deref(), Some("x/COM10.png"));
        for bad in ["", "a//b", "/a", "a/../b", "../a", "a/./b", "a/ ../b", "a/.. /b", "a/. /b", "a/ /b", "a/.../b", "C:/a", "a/b:c", "a\\..\\b", "a/"] {
            assert_eq!(s(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn zip_import_trims_trailing_space_and_dot_folders_and_fixes_bad_names() {
        let d = scratch("zipnames");
        let zp = d.join("SCUS-97199.zip");
        {
            let f = std::fs::File::create(&zp).unwrap();
            let mut w = zip::ZipWriter::new(f);
            let o = zip::write::SimpleFileOptions::default();
            for (name, data) in [
                ("SCUS-97199/replacements/LEVELS/A /0123456789abcdef-00000007.png", "space"),
                ("SCUS-97199/replacements/LEVELS/B./0123456789abcdef-00000008.png", "dot"),
                ("SCUS-97199/replacements/LEVELS/ C/0123456789abcdef-00000009.png", "lead"),
                ("SCUS-97199/replacements/LEVELS/x<y>/NUL.png", "nul"),
                // both end up as LEVELS/D/...0a.png: the first one is kept, the second is counted as skipped
                ("SCUS-97199/replacements/LEVELS/D /0123456789abcdef-0000000a.png", "first"),
                ("SCUS-97199/replacements/LEVELS/D/0123456789abcdef-0000000a.png", "second"),
                ("SCUS-97199/replacements/LEVELS/ /0123456789abcdef-0000000b.png", "empty folder name"),
            ] {
                w.start_file(name, o).unwrap();
                w.write_all(data.as_bytes()).unwrap();
            }
            w.finish().unwrap();
        }
        let c = inspect(source_of(&zp).unwrap()).unwrap();
        assert_eq!(c.detected.files.len(), 7);
        let info = plan_info(&c, |_| false, None);
        let mods = d.join("mods");
        let st = start(c, info.clone(), mods.clone());
        assert_eq!(wait(&st), Ok(info.id.clone()), "no import error");
        let t = mods.join(&info.id).join("textures").join("LEVELS");
        assert_eq!(std::fs::read(t.join("A").join("0123456789abcdef-00000007.png")).unwrap(), b"space");
        assert_eq!(std::fs::read(t.join("B").join("0123456789abcdef-00000008.png")).unwrap(), b"dot");
        assert_eq!(std::fs::read(t.join(" C").join("0123456789abcdef-00000009.png")).unwrap(), b"lead");
        assert_eq!(std::fs::read(t.join("x_y_").join("_NUL.png")).unwrap(), b"nul");
        assert_eq!(std::fs::read(t.join("D").join("0123456789abcdef-0000000a.png")).unwrap(), b"first");
        assert_eq!(st.skipped.load(Ordering::Relaxed), 2, "the repeated path and the unusable folder name");
        assert_eq!(st.files_done.load(Ordering::Relaxed), 7);
        assert!(!mods.join(format!("{}.tmp", info.id)).exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Real texture pack check (not run by default): `RACL_REAL_ZIP=<zip> RACL_REAL_OUT=<scratch folder> cargo test --lib real_zip -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_zip_imports_without_errors() {
        let (Ok(zip), Ok(out)) = (std::env::var("RACL_REAL_ZIP"), std::env::var("RACL_REAL_OUT")) else { panic!("set RACL_REAL_ZIP and RACL_REAL_OUT") };
        let t = std::time::Instant::now();
        let src = source_of(Path::new(&zip)).unwrap();
        println!("TIME source_of: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let a = open_zip(Path::new(&zip)).unwrap();
        println!("TIME open_zip (central directory only): {:?}  entries: {}", t.elapsed(), a.len());
        drop(a);
        let t = std::time::Instant::now();
        let entries = list(&src).unwrap();
        println!("TIME list (zip central directory + entries): {:?}  entries: {}", t.elapsed(), entries.len());
        let t = std::time::Instant::now();
        let _ = analyze(&entries, &src.name());
        println!("TIME analyze: {:?}", t.elapsed());
        let t = std::time::Instant::now();
        let c = inspect(src).unwrap();
        println!("TIME inspect (all): {:?}", t.elapsed());
        println!("files to import: {}  bytes: {}  layout: {:?}", c.detected.files.len(), c.detected.bytes, c.detected.layout);
        let info = plan_info(&c, |_| false, None);
        let id = info.id.clone();
        let mods = PathBuf::from(&out).join("mods");
        let st = start(c, info, mods.clone());
        let t0 = std::time::Instant::now();
        let r = loop {
            if let Some(r) = st.result() {
                break r;
            }
            if t0.elapsed().as_secs() % 30 == 0 {
                println!("  {} / {} files, {:.0}%", st.files_done.load(Ordering::Relaxed), st.files_total.load(Ordering::Relaxed), st.percent() * 100.0);
            }
            std::thread::sleep(std::time::Duration::from_millis(1000));
        };
        println!("TIME import copy: {:?}", t0.elapsed());
        println!("result: {r:?}  skipped: {}  files_done: {}/{}  secs: {}", st.skipped.load(Ordering::Relaxed), st.files_done.load(Ordering::Relaxed), st.files_total.load(Ordering::Relaxed), t0.elapsed().as_secs());
        assert_eq!(r, Ok(id.clone()));
        let t = std::time::Instant::now();
        let store = super::super::ModStore::load(&mods);
        println!("TIME post-import ModStore::load (reload): {:?}  mods: {}", t.elapsed(), store.entries.len());
        let mut n = 0usize;
        let mut stack = vec![mods.join(&id).join("textures")];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else {
                    n += 1;
                }
            }
        }
        let gold = std::fs::read_dir(mods.join(&id).join("textures/LEVELS/NOVALIS/DISPLAY CASE ONLY GOLD WEAPONS")).unwrap().count();
        println!("files on disk: {n}  files under the trimmed folder: {gold}");
        assert_eq!(st.files_done.load(Ordering::Relaxed), st.files_total.load(Ordering::Relaxed));
    }

    /// Timing of the folder path on a real extracted pack (not run by default): `RACL_REAL_DIR=<pack folder> cargo test --release --lib real_dir -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn real_dir_inspect_timing() {
        let Ok(dir) = std::env::var("RACL_REAL_DIR") else { panic!("set RACL_REAL_DIR") };
        let t = std::time::Instant::now();
        let src = source_of(Path::new(&dir)).unwrap();
        let entries = list(&src).unwrap();
        println!("TIME list (folder walk): {:?}  entries: {}", t.elapsed(), entries.len());
        let t = std::time::Instant::now();
        let c = inspect(src).unwrap();
        println!("TIME inspect (all): {:?}  files: {}  layout: {:?}", t.elapsed(), c.detected.files.len(), c.detected.layout);
    }

    fn wait_read(st: &ReadState) -> Result<Candidate, String> {
        for _ in 0..1000 {
            if let Some(r) = st.take_result() {
                return r;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        panic!("reading did not finish");
    }

    #[test]
    fn start_inspect_returns_at_once_and_the_worker_reports_files_found() {
        let d = scratch("readasync");
        make_pack(&d.join("src"), 700);
        let pack = d.join("src").join("SCES-50916");
        // the worker is held back: the call has returned, nothing is finished, the player could still click Cancel
        let st = start_inspect_held(pack.clone());
        std::thread::sleep(std::time::Duration::from_millis(150));
        assert!(!st.is_done() && st.take_result().is_none(), "start_inspect must not wait for the worker");
        assert_eq!(st.entries.load(Ordering::Relaxed), 0);
        st.release();
        let c = wait_read(&st).unwrap();
        assert_eq!(c.detected.files.len(), 701);
        assert!(st.entries.load(Ordering::Relaxed) >= 512, "the live count is updated while reading");
        // a zip takes the same road
        let zp = d.join("Pack.zip");
        {
            let mut w = zip::ZipWriter::new(std::fs::File::create(&zp).unwrap());
            for i in 0..300u64 {
                w.start_file(format!("SCES-50916/replacements/{:016x}-00000007.png", 0xaa00 + i), zip::write::SimpleFileOptions::default()).unwrap();
                w.write_all(b"png").unwrap();
            }
            w.finish().unwrap();
        }
        let c = wait_read(&start_inspect(zp)).unwrap();
        assert_eq!(c.detected.files.len(), 300);
        // errors come back as messages, not panics
        assert!(wait_read(&start_inspect(d.join("nope.zip"))).unwrap_err().contains("could not be found"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cancelling_a_read_ends_it_with_a_cancel_message() {
        let d = scratch("readcancel");
        make_pack(&d.join("src"), 700);
        let st = start_inspect_held(d.join("src").join("SCES-50916"));
        st.cancel.store(true, Ordering::Relaxed);
        assert!(wait_read(&st).unwrap_err().to_lowercase().contains("cancel"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_zip_with_something_in_front_of_it_still_opens() {
        // the quick open assumes the archive starts at byte 0; a self-extractor stub in front needs the slow detection as fallback
        let d = scratch("zipjunk");
        let zp = d.join("Pack.zip");
        {
            let mut f = std::fs::File::create(&zp).unwrap();
            f.write_all(&[0x4d, 0x5a, 0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17]).unwrap();
            let mut w = zip::ZipWriter::new(f);
            w.start_file("SCUS-97199/replacements/0123456789abcdef-00000007.png", zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(b"pngdata").unwrap();
            w.finish().unwrap();
        }
        let c = inspect(source_of(&zp).unwrap()).unwrap();
        assert_eq!(c.detected.files.len(), 1);
        let info = plan_info(&c, |_| false, None);
        let st = start(c, info.clone(), d.join("mods"));
        assert_eq!(wait(&st), Ok(info.id.clone()));
        assert_eq!(std::fs::read(d.join("mods").join(&info.id).join("textures/0123456789abcdef-00000007.png")).unwrap(), b"pngdata");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn mod_json_pack_keeps_its_id_and_gets_a_suffix_when_taken() {
        let d = scratch("mj");
        let s = d.join("src");
        std::fs::create_dir_all(s.join("textures")).unwrap();
        std::fs::write(s.join("mod.json"), r#"{"format":1,"id":"cool","name":"Cool","game":"rac1","kind":"textures","serials":[]}"#).unwrap();
        std::fs::write(s.join("textures/0123456789abcdef-00000007.png"), b"p").unwrap();
        let c = inspect(source_of(&s).unwrap()).unwrap();
        assert_eq!(c.info.as_ref().unwrap().name, "Cool");
        let info = plan_info(&c, |x| x == "cool", None);
        assert_eq!(info.id, "cool-2");
        let st = start(c, info, d.join("mods"));
        assert_eq!(wait(&st), Ok("cool-2".to_string()));
        let m = super::super::ModStore::load(&d.join("mods"));
        assert_eq!(m.get("cool-2").unwrap().info.as_ref().unwrap().id, "cool-2");
        assert!(d.join("mods/cool-2/textures/0123456789abcdef-00000007.png").is_file());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn bad_sources_give_plain_messages() {
        let d = scratch("bad");
        std::fs::write(d.join("a.7z"), b"x").unwrap();
        assert!(source_of(&d.join("a.7z")).unwrap_err().contains("Extract the archive first"));
        assert!(source_of(&d.join("nope.zip")).unwrap_err().contains("could not be found"));
        std::fs::write(d.join("a.txt"), b"x").unwrap();
        assert!(source_of(&d.join("a.txt")).is_err());
        std::fs::create_dir_all(d.join("empty")).unwrap();
        std::fs::write(d.join("empty/readme.txt"), b"x").unwrap();
        assert!(inspect(Source::Dir(d.join("empty"))).unwrap_err().contains("does not look like"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn pcsx2_folder_discovery() {
        let d = scratch("pcsx2");
        make_pack(&d.join("PCSX2").join("textures"), 2);
        std::fs::write(d.join("PCSX2/textures/SCUS-97199.zip"), b"z").unwrap();
        let v = pcsx2_candidates(&d, &["SCES-50916", "SCUS-97199"]);
        assert_eq!(v.len(), 2);
        assert!(matches!(&v[0].1, Source::Dir(_)) && matches!(&v[1].1, Source::Zip(_)));
        let _ = std::fs::remove_dir_all(&d);
    }
}
