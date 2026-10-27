//! The launcher updates only itself, from its own repo (AsobiCodes/rac-rs-launcher, latest release); game updates are setup.ps1's job.
//! Asset `rac-rs-launcher-windows-x64.exe` (Linux later: `rac-rs-launcher-linux-x64`), installed as `RAC-RS Launcher.exe` next to the running exe.
//! Steps of `apply`: download + checksum to `<exe>.new`; rename the RUNNING exe to `<exe>.old` (Windows allows that), rename `.new` to the exe name;
//! the caller starts the new exe with `--restarted` and quits; the new start deletes `.old` (`remove_old`).
use crate::releases::{self, Release, ReleaseError, ReleaseSource, Transfer};
use crate::update::compare_version;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

pub const ASSET_NAME: &str = if cfg!(windows) { "rac-rs-launcher-windows-x64.exe" } else { "rac-rs-launcher-linux-x64" };
/// The launcher's own version (Cargo.toml).
pub const OWN_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LauncherUpdate {
    pub version: String,
    pub notes: String,
    pub asset: releases::Asset,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum SelfState {
    #[default]
    Idle,
    Checking,
    UpToDate,
    Available(LauncherUpdate),
    /// Offline, rate limit, no repo yet, no matching asset: nothing is shown to the user.
    Unavailable(String),
}

/// A newer launcher in `rel`, if it has the asset for this platform. Notes keep only the text (no markdown cleanup: the UI shows them as plain lines).
pub fn offer_from(rel: &Release, own: &str) -> Result<Option<LauncherUpdate>, ReleaseError> {
    if compare_version(&rel.version(), own) != Ordering::Greater {
        return Ok(None);
    }
    let asset = rel.asset(ASSET_NAME).ok_or_else(|| ReleaseError::Bad(format!("the release has no {ASSET_NAME}")))?.clone();
    Ok(Some(LauncherUpdate { version: rel.version(), notes: rel.notes(), asset }))
}

pub fn check(src: &ReleaseSource, own: &str) -> Result<Option<LauncherUpdate>, ReleaseError> {
    offer_from(&src.latest()?, own)
}

/// Runs `check` on a thread; the answer lands in `slot`.
pub fn check_async(src: ReleaseSource, own: String, slot: Arc<Mutex<SelfState>>) {
    if let Ok(mut g) = slot.lock() {
        *g = SelfState::Checking;
    }
    std::thread::spawn(move || {
        let st = match check(&src, &own) {
            Ok(Some(u)) => SelfState::Available(u),
            Ok(None) => SelfState::UpToDate,
            Err(e) => SelfState::Unavailable(e.text()),
        };
        if let Ok(mut g) = slot.lock() {
            *g = st;
        }
    });
}

pub fn old_path(exe: &Path) -> PathBuf {
    PathBuf::from(format!("{}.old", exe.display()))
}
pub fn new_path(exe: &Path) -> PathBuf {
    PathBuf::from(format!("{}.new", exe.display()))
}

/// Downloads the update and swaps it in place of `exe` (the running launcher). Returns `exe` (now the new version) for the restart.
/// On any failure the old exe is back in place and the `.new` file is gone.
pub fn apply(src: &ReleaseSource, upd: &LauncherUpdate, exe: &Path, t: &Transfer) -> Result<PathBuf, ReleaseError> {
    let new = new_path(exe);
    let old = old_path(exe);
    releases::download(src, &upd.asset, &new, t)?;
    let _ = std::fs::remove_file(&old); // an older leftover
    if let Err(e) = std::fs::rename(exe, &old) {
        let _ = std::fs::remove_file(&new);
        return Err(releases::io_error(&e));
    }
    if let Err(e) = std::fs::rename(&new, exe) {
        let _ = std::fs::rename(&old, exe);
        let _ = std::fs::remove_file(&new);
        return Err(releases::io_error(&e));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(exe, std::fs::Permissions::from_mode(0o755));
    }
    Ok(exe.to_path_buf())
}

/// Deletes `<exe>.old` (the launcher before the update). One attempt; true when it is gone.
pub fn remove_old(exe: &Path) -> bool {
    let old = old_path(exe);
    !old.exists() || std::fs::remove_file(&old).is_ok()
}

/// At start: remove `.old`, retrying for ~15 s on a thread (the previous launcher may still be closing).
pub fn cleanup_old(exe: PathBuf) {
    if remove_old(&exe) {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..30 {
            std::thread::sleep(std::time::Duration::from_millis(500));
            if remove_old(&exe) {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering as AO;

    fn scratch(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_self_{n}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }
    fn good(_: &str, hex: &str) -> Option<String> {
        Some(format!("sha256:{hex}"))
    }

    #[test]
    fn newer_release_is_offered_older_or_equal_is_not() {
        let d = scratch("offer");
        releases::write_test_release(&d, "v0.2.0", "- better\n- faster", &[(ASSET_NAME, b"NEWEXE")], &good);
        let src = ReleaseSource::Folder(d.clone());
        let u = check(&src, "0.1.0").unwrap().unwrap();
        assert_eq!((u.version.as_str(), u.notes.as_str()), ("0.2.0", "- better\n- faster"));
        assert_eq!(check(&src, "0.2.0").unwrap(), None);
        assert_eq!(check(&src, "0.10.0").unwrap(), None);
        releases::write_test_release(&d, "v0.2.0", "", &[("other.txt", b"x")], &good);
        assert!(matches!(check(&src, "0.1.0"), Err(ReleaseError::Bad(_))));
        assert_eq!(check(&ReleaseSource::Folder(d.join("nope")), "0.1.0"), Err(ReleaseError::NotFound));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn update_swaps_the_exe_keeps_old_next_to_it_and_cleans_up() {
        let rel = scratch("rel");
        let app = scratch("app");
        let exe = app.join("RAC-RS Launcher.exe");
        std::fs::write(&exe, b"OLDEXE").unwrap();
        std::fs::write(old_path(&exe), b"ANCIENT").unwrap();
        releases::write_test_release(&rel, "v0.2.0", "notes", &[(ASSET_NAME, b"NEWEXE-bytes")], &good);
        let src = ReleaseSource::Folder(rel.clone());
        let upd = check(&src, "0.1.0").unwrap().unwrap();
        let t = Transfer::default();
        assert_eq!(apply(&src, &upd, &exe, &t).unwrap(), exe);
        assert_eq!(std::fs::read(&exe).unwrap(), b"NEWEXE-bytes");
        assert_eq!(std::fs::read(old_path(&exe)).unwrap(), b"OLDEXE");
        assert!(!new_path(&exe).exists());
        assert!(remove_old(&exe) && !old_path(&exe).exists());
        assert_eq!(t.done.load(AO::Relaxed), 12);
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(&app);
    }

    #[test]
    fn failed_update_leaves_the_old_exe_alone() {
        let rel = scratch("failrel");
        let app = scratch("failapp");
        let exe = app.join("RAC-RS Launcher.exe");
        std::fs::write(&exe, b"OLDEXE").unwrap();
        releases::write_test_release(&rel, "v0.2.0", "", &[(ASSET_NAME, b"NEW")], &|_, _| Some(format!("sha256:{}", "a".repeat(64))));
        let src = ReleaseSource::Folder(rel.clone());
        let upd = check(&src, "0.1.0").unwrap().unwrap();
        assert_eq!(apply(&src, &upd, &exe, &Transfer::default()), Err(ReleaseError::DigestMismatch));
        assert_eq!(std::fs::read(&exe).unwrap(), b"OLDEXE");
        assert!(!old_path(&exe).exists() && !new_path(&exe).exists());
        let _ = std::fs::remove_dir_all(&rel);
        let _ = std::fs::remove_dir_all(&app);
    }

    #[test]
    fn async_check_reports_into_the_slot() {
        let d = scratch("async");
        releases::write_test_release(&d, "v3.0.0", "n", &[(ASSET_NAME, b"x")], &good);
        let slot = Arc::new(Mutex::new(SelfState::Idle));
        check_async(ReleaseSource::Folder(d.clone()), "0.1.0".into(), slot.clone());
        for _ in 0..100 {
            if !matches!(*slot.lock().unwrap(), SelfState::Checking) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(matches!(&*slot.lock().unwrap(), SelfState::Available(u) if u.version == "3.0.0"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
