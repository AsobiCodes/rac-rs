//! Download of the game for a new install: the latest RAC-RS RELEASE (never the main branch) of AsobiCodes/rac-rs, asset `rac-rs-<VER>.zip`,
//! unpacked into the chosen folder. The zip holds one top folder `rac-rs-<VER>/`; it is stripped. Entries that would leave the folder are refused.
//! The disc setup (setup.ps1 through the engine) is NOT done here: the folder is added to the library and the normal flow takes over.
use crate::library;
use crate::releases::{self, Asset, Release, ReleaseError, ReleaseSource, Transfer};
use std::fs::File;
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

pub const STAGE_LOOKUP: u8 = 0;
pub const STAGE_DOWNLOAD: u8 = 1;
pub const STAGE_UNPACK: u8 = 2;

/// The oldest game version the launcher installs: every release from 0.2.1 on has a zip with a checksum and a setup.ps1 that takes -Console -Iso
/// -SkipHash -NoShortcut (0.2.0 and 0.1.x are tags only).
pub const MIN_INSTALLABLE: &str = "0.2.1";

/// One line of the version picker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Installable {
    pub version: String,
    pub date: String,
    pub prerelease: bool,
    pub notes: String,
}

/// The versions that can be installed, newest first: no drafts, prereleases only with `show_test`, a `rac-rs-<ver>.zip` with a sha256 digest, >= MIN_INSTALLABLE.
pub fn installable(list: &[Release], show_test: bool) -> Vec<Installable> {
    let mut v: Vec<Installable> = list
        .iter()
        .filter(|r| !r.draft && (show_test || !r.prerelease))
        .filter(|r| crate::update::compare_version(&r.version(), MIN_INSTALLABLE) != std::cmp::Ordering::Less)
        .filter(|r| r.asset(&format!("rac-rs-{}.zip", r.version())).map(|a| releases::digest_hex(a).is_some()).unwrap_or(false))
        .map(|r| Installable { version: r.version(), date: r.date(), prerelease: r.prerelease, notes: r.notes() })
        .collect();
    v.sort_by(|a, b| crate::update::compare_version(&b.version, &a.version));
    v.dedup_by(|a, b| a.version == b.version);
    v
}

/// What the picker shows by default: the newest entry (a test version only when it is the only kind there is).
pub fn default_pick(v: &[Installable]) -> Option<&Installable> {
    v.iter().find(|i| !i.prerelease).or_else(|| v.first())
}

/// Folder name suggested for a version: "Ratchet & Clank" for the newest, "Ratchet & Clank <ver>" for an older one.
pub fn folder_name(version: &str, newest: Option<&str>) -> String {
    match newest {
        Some(n) if n != version => format!("Ratchet & Clank {version}"),
        _ => "Ratchet & Clank".to_string(),
    }
}

/// A running (or finished) game download; the UI polls it every frame.
pub struct GameDl {
    pub transfer: Arc<Transfer>,
    pub stage: Arc<AtomicU8>,
    pub started: Instant,
    pub dest: PathBuf,
    pub version: Arc<Mutex<String>>,
    pub result: Arc<Mutex<Option<Result<String, ReleaseError>>>>,
}

impl GameDl {
    pub fn start(src: ReleaseSource, dest: PathBuf, want: Option<String>) -> GameDl {
        let dl = GameDl {
            transfer: Transfer::new(),
            stage: Arc::new(AtomicU8::new(STAGE_LOOKUP)),
            started: Instant::now(),
            dest: dest.clone(),
            version: Arc::new(Mutex::new(String::new())),
            result: Arc::new(Mutex::new(None)),
        };
        let (t, st, ver, res) = (dl.transfer.clone(), dl.stage.clone(), dl.version.clone(), dl.result.clone());
        std::thread::spawn(move || {
            let r = run(&src, &dest, want.as_deref(), &t, &st, &ver);
            if let Ok(mut g) = res.lock() {
                *g = Some(r);
            }
        });
        dl
    }
    pub fn take_result(&self) -> Option<Result<String, ReleaseError>> {
        self.result.lock().ok().and_then(|mut g| g.take())
    }
    pub fn cancel(&self) {
        self.transfer.cancel.store(true, Ordering::Relaxed);
    }
}

/// `rac-rs-<VER>.zip` of the release (the exact name first, then the only `rac-rs-*.zip` that is not the launcher).
pub fn game_asset(r: &Release) -> Option<&Asset> {
    let exact = format!("rac-rs-{}.zip", r.version());
    r.asset(&exact).or_else(|| r.assets.iter().find(|a| a.name.starts_with("rac-rs-") && a.name.ends_with(".zip") && !a.name.contains("launcher")))
}

/// Looks up the release, downloads the zip into `<dest>/.racl_download/`, unpacks it into `dest`, removes the zip. Returns the version.
pub fn run(src: &ReleaseSource, dest: &Path, want: Option<&str>, t: &Transfer, stage: &AtomicU8, version: &Mutex<String>) -> Result<String, ReleaseError> {
    stage.store(STAGE_LOOKUP, Ordering::Relaxed);
    let rel = match want {
        None => src.latest()?,
        Some(v) => src.list()?.into_iter().find(|r| !r.draft && r.version() == v).ok_or(ReleaseError::NotFound)?,
    };
    let asset = game_asset(&rel).ok_or_else(|| ReleaseError::Bad("the release has no game zip".into()))?.clone();
    if let Ok(mut g) = version.lock() {
        *g = rel.version();
    }
    let tmp = dest.join(".racl_download");
    let zip = tmp.join(&asset.name);
    stage.store(STAGE_DOWNLOAD, Ordering::Relaxed);
    let r = releases::download(src, &asset, &zip, t).and_then(|_| {
        stage.store(STAGE_UNPACK, Ordering::Relaxed);
        unpack(&zip, dest, t)
    });
    let _ = std::fs::remove_dir_all(&tmp);
    r.map(|_| rel.version())
}

/// A zip entry name as a safe relative path: no `..`, no root, no drive letter. None = refuse.
pub fn safe_rel(name: &str) -> Option<PathBuf> {
    let n = name.replace('\\', "/");
    if n.starts_with('/') || n.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for part in n.split('/') {
        match part {
            "" | "." => {}
            ".." => return None,
            p => out.push(p),
        }
    }
    if out.components().any(|c| !matches!(c, Component::Normal(_))) {
        return None;
    }
    Some(out)
}

/// The single top folder every entry lives in, if there is one (`rac-rs-0.3.2/`).
pub fn common_top(names: &[String]) -> Option<String> {
    let mut top: Option<String> = None;
    for n in names {
        let n = n.replace('\\', "/");
        let first = n.trim_start_matches('/').split('/').next().unwrap_or("");
        if first.is_empty() || !n.trim_start_matches('/').contains('/') && !n.ends_with('/') {
            return None; // a file at the top level: nothing to strip
        }
        match &top {
            None => top = Some(first.to_string()),
            Some(t) if t == first => {}
            _ => return None,
        }
    }
    top
}

/// Unpacks `zip` into `dest` (top folder stripped). Progress in `t` (uncompressed bytes). On any error or cancel the files written so far are removed.
pub fn unpack(zip: &Path, dest: &Path, t: &Transfer) -> Result<(), ReleaseError> {
    let f = File::open(zip).map_err(|e| releases::io_error(&e))?;
    let mut ar = zip::ZipArchive::new(f).map_err(|e| ReleaseError::Bad(format!("the zip is damaged: {e}")))?;
    let mut names = Vec::new();
    let mut total = 0u64;
    for i in 0..ar.len() {
        let e = ar.by_index(i).map_err(|e| ReleaseError::Bad(e.to_string()))?;
        names.push(entry_name(&e)?);
        total += e.size();
    }
    // refuse before writing anything
    for n in &names {
        if safe_rel(n).is_none() {
            return Err(ReleaseError::Bad(format!("the zip holds an unsafe path: {n}")));
        }
    }
    let top = common_top(&names);
    t.done.store(0, Ordering::Relaxed);
    t.total.store(total.max(1), Ordering::Relaxed);
    let mut written: Vec<PathBuf> = Vec::new();
    let r = unpack_entries(&mut ar, dest, top.as_deref(), t, &mut written);
    if r.is_err() {
        for p in written.iter().rev() {
            let _ = std::fs::remove_file(p);
        }
        for p in written.iter().rev() {
            if let Some(d) = p.parent() {
                let _ = std::fs::remove_dir(d); // only when empty
            }
        }
    }
    r
}

fn unpack_entries(ar: &mut zip::ZipArchive<File>, dest: &Path, top: Option<&str>, t: &Transfer, written: &mut Vec<PathBuf>) -> Result<(), ReleaseError> {
    let mut buf = vec![0u8; 256 * 1024];
    let mut done = 0u64;
    for i in 0..ar.len() {
        let mut e = ar.by_index(i).map_err(|e| ReleaseError::Bad(e.to_string()))?;
        let name = entry_name(&e)?;
        let rel = safe_rel(&name).ok_or_else(|| ReleaseError::Bad(format!("unsafe path {name}")))?;
        let rel = match top {
            Some(tp) => rel.strip_prefix(tp).map(Path::to_path_buf).unwrap_or(rel),
            None => rel,
        };
        if rel.as_os_str().is_empty() {
            continue;
        }
        let target = dest.join(&rel);
        if e.is_dir() {
            std::fs::create_dir_all(&target).map_err(|e| releases::io_error(&e))?;
            continue;
        }
        if let Some(p) = target.parent() {
            std::fs::create_dir_all(p).map_err(|e| releases::io_error(&e))?;
        }
        let mut out = File::create(&target).map_err(|e| releases::io_error(&e))?;
        written.push(target.clone());
        loop {
            if t.cancel.load(Ordering::Relaxed) {
                return Err(ReleaseError::Cancelled);
            }
            let n = e.read(&mut buf).map_err(|e| ReleaseError::Bad(e.to_string()))?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n]).map_err(|e| releases::io_error(&e))?;
            done += n as u64;
            t.done.store(done, Ordering::Relaxed);
        }
    }
    Ok(())
}

fn entry_name<R: Read + ?Sized>(e: &zip::read::ZipFile<'_, R>) -> Result<String, ReleaseError> {
    e.name().map(|n| n.into_owned()).map_err(|_| ReleaseError::Bad("the zip holds a name that is not valid text".into()))
}

/// True when `dest` can take a fresh unpack: missing, empty, or already only holding our download folder.
pub fn folder_is_free(dest: &Path) -> bool {
    match std::fs::read_dir(dest) {
        Err(_) => !dest.exists(),
        Ok(rd) => rd.filter_map(|e| e.ok()).all(|e| e.file_name() == ".racl_download"),
    }
}

/// After a successful unpack: the folder must now look like a release tree.
pub fn looks_unpacked(dest: &Path) -> bool {
    library::is_release_tree(dest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_gdl_{n}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn make_zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            let o = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
            for (n, b) in entries {
                if n.ends_with('/') {
                    w.add_directory(*n, o).unwrap();
                } else {
                    w.start_file(*n, o).unwrap();
                    w.write_all(b).unwrap();
                }
            }
            w.finish().unwrap();
        }
        buf.into_inner()
    }

    fn good(_: &str, hex: &str) -> Option<String> {
        Some(format!("sha256:{hex}"))
    }

    #[test]
    fn paths_are_guarded() {
        assert_eq!(safe_rel("rac-rs-0.3.2/a/b.txt"), Some(PathBuf::from("rac-rs-0.3.2").join("a").join("b.txt")));
        assert_eq!(safe_rel("a\\b.txt"), Some(PathBuf::from("a").join("b.txt")));
        for bad in ["../x", "a/../../x", "/etc/passwd", "C:/x", "C:\\x", "a/..\\..\\x"] {
            assert_eq!(safe_rel(bad), None, "{bad}");
        }
    }

    #[test]
    fn top_folder_is_found_only_when_shared() {
        let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(common_top(&v(&["rac-rs-0.3.2/", "rac-rs-0.3.2/VERSION", "rac-rs-0.3.2/x/y"])).as_deref(), Some("rac-rs-0.3.2"));
        assert_eq!(common_top(&v(&["rac-rs-0.3.2/VERSION", "other/y"])), None);
        assert_eq!(common_top(&v(&["VERSION", "a/b"])), None);
    }

    #[test]
    fn release_zip_is_downloaded_unpacked_and_cleaned() {
        let rel = scratch("rel");
        let dest = scratch("dest").join("game");
        let zip = make_zip(&[
            ("rac-rs-9.9.9/", b""),
            ("rac-rs-9.9.9/VERSION", b"9.9.9\n"),
            ("rac-rs-9.9.9/setup.ps1", b"# setup"),
            ("rac-rs-9.9.9/oracle/stubs.txt", b"stub"),
        ]);
        releases::write_test_release(&rel, "v9.9.9", "notes", &[("rac-rs-9.9.9.zip", &zip), ("rac-rs-launcher-windows-x64.exe", b"MZ")], &good);
        let src = ReleaseSource::Folder(rel.clone());
        let t = Transfer::default();
        let stage = AtomicU8::new(0);
        let ver = Mutex::new(String::new());
        assert_eq!(run(&src, &dest, None, &t, &stage, &ver).unwrap(), "9.9.9");
        assert_eq!(std::fs::read_to_string(dest.join("VERSION")).unwrap(), "9.9.9\n");
        assert_eq!(std::fs::read_to_string(dest.join("oracle").join("stubs.txt")).unwrap(), "stub");
        assert!(looks_unpacked(&dest));
        assert!(!dest.join(".racl_download").exists() && !dest.join("rac-rs-9.9.9").exists());
        assert!(folder_is_free(&dest) == false);
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn traversal_zip_is_refused_and_writes_nothing() {
        let d = scratch("evil");
        let zp = d.join("evil.zip");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(&zp, make_zip(&[("rac-rs-1/ok.txt", b"x"), ("rac-rs-1/../../escape.txt", b"bad")])).unwrap();
        let dest = d.join("out");
        let r = unpack(&zp, &dest, &Transfer::default());
        assert!(matches!(r, Err(ReleaseError::Bad(_))), "{r:?}");
        assert!(!dest.join("ok.txt").exists() && !d.join("escape.txt").exists());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn bad_digest_and_missing_asset_leave_the_folder_clean() {
        let rel = scratch("badrel");
        let dest = scratch("baddest").join("game");
        let zip = make_zip(&[("rac-rs-1.0.0/VERSION", b"1.0.0")]);
        releases::write_test_release(&rel, "v1.0.0", "", &[("rac-rs-1.0.0.zip", &zip)], &|_, _| Some(format!("sha256:{}", "1".repeat(64))));
        let src = ReleaseSource::Folder(rel.clone());
        let (t, st, v) = (Transfer::default(), AtomicU8::new(0), Mutex::new(String::new()));
        assert_eq!(run(&src, &dest, None, &t, &st, &v), Err(ReleaseError::DigestMismatch));
        assert!(folder_is_free(&dest));
        releases::write_test_release(&rel, "v1.0.0", "", &[("something.txt", b"x")], &good);
        assert!(matches!(run(&src, &dest, None, &t, &st, &v), Err(ReleaseError::Bad(_))));
        assert_eq!(run(&ReleaseSource::Folder(scratch("nothing")), &dest, None, &t, &st, &v), Err(ReleaseError::NotFound));
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

    #[test]
    fn version_list_is_filtered_sorted_and_defaulted() {
        let hex = "ab".repeat(32);
        let rel = |tag: &str, draft: bool, pre: bool, asset: Option<&str>, digest: bool| -> String {
            let assets = match asset {
                Some(a) => format!(r#"[{{"name":"{a}","size":5,"browser_download_url":"u","digest":{}}}]"#, if digest { format!("\"sha256:{hex}\"") } else { "null".into() }),
                None => "[]".into(),
            };
            format!(r#"{{"tag_name":"{tag}","draft":{draft},"prerelease":{pre},"published_at":"2026-10-0{}T10:00:00Z","assets":{assets}}}"#, tag.len() % 9)
        };
        let json = format!(
            "[{}]",
            [
                rel("v0.3.2-rc1", false, true, Some("rac-rs-0.3.2-rc1.zip"), true),
                rel("v0.3.1", false, false, Some("rac-rs-0.3.1.zip"), true),
                rel("v0.3.0", false, false, Some("rac-rs-0.3.0.zip"), true),
                rel("v0.2.2", false, false, Some("rac-rs-0.2.2.zip"), true),
                rel("v0.2.1", false, false, Some("rac-rs-0.2.1.zip"), true),
                rel("v0.2.0", false, false, None, false),
                rel("v0.1.3", false, false, Some("rac-rs-0.1.3.zip"), true),
                rel("v0.4.0", true, false, Some("rac-rs-0.4.0.zip"), true),
                rel("v0.2.3", false, false, Some("rac-rs-0.2.3.zip"), false),
            ]
            .join(",")
        );
        let list = releases::parse_release_list(&json).unwrap();
        let v: Vec<String> = installable(&list, false).iter().map(|i| i.version.clone()).collect();
        assert_eq!(v, ["0.3.1", "0.3.0", "0.2.2", "0.2.1"]);
        let t = installable(&list, true);
        assert_eq!(t[0].version, "0.3.2-rc1");
        assert!(t[0].prerelease && t.len() == 5);
        assert_eq!(default_pick(&t).unwrap().version, "0.3.1");
        assert_eq!(installable(&list, false)[3].date.len(), 10);
        assert_eq!(folder_name("0.2.2", Some("0.3.1")), "Ratchet & Clank 0.2.2");
        assert_eq!(folder_name("0.3.1", Some("0.3.1")), "Ratchet & Clank");
        assert_eq!(default_pick(&[]), None);
    }

    #[test]
    fn an_older_version_is_installed_by_its_number() {
        let rel = scratch("old");
        let dest = scratch("olddest").join("g");
        let z = make_zip(&[("rac-rs-0.2.2/VERSION", b"0.2.2\n"), ("rac-rs-0.2.2/setup.ps1", b"#")]);
        releases::write_test_release(&rel, "v0.2.2", "", &[("rac-rs-0.2.2.zip", &z)], &good);
        let latest = std::fs::read_to_string(rel.join("latest.json")).unwrap();
        std::fs::write(rel.join("releases.json"), format!("[{latest}]")).unwrap();
        let src = ReleaseSource::Folder(rel.clone());
        let (t, st, v) = (Transfer::default(), AtomicU8::new(0), Mutex::new(String::new()));
        assert_eq!(run(&src, &dest, Some("0.2.2"), &t, &st, &v).unwrap(), "0.2.2");
        assert_eq!(std::fs::read_to_string(dest.join("VERSION")).unwrap(), "0.2.2\n");
        assert_eq!(run(&src, &scratch("olddest2"), Some("0.1.0"), &t, &st, &v), Err(ReleaseError::NotFound));
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(dest.parent().unwrap());
    }

}
