//! Disc image check (background thread): size picks the disc, then the SHA-1 proves the copy, with progress.
//! Messages are the ones of the old setup window.
use crate::games::{DiscDef, GameDef};
use sha1::{Digest, Sha1};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Debug, PartialEq)]
pub enum DiscResult {
    Running,
    Ok(&'static DiscDef),
    Bad(String),
    Cancelled,
}

/// Shared with the thread: progress counters and the result.
pub struct DiscCheck {
    pub path: PathBuf,
    pub pos: AtomicU64,
    pub total: AtomicU64,
    pub cancel: AtomicBool,
    pub result: Mutex<DiscResult>,
}

impl DiscCheck {
    pub fn percent(&self) -> u32 {
        let t = self.total.load(Ordering::Relaxed);
        if t == 0 {
            0
        } else {
            (100 * self.pos.load(Ordering::Relaxed) / t).min(100) as u32
        }
    }

    pub fn result(&self) -> DiscResult {
        self.result.lock().map(|r| r.clone()).unwrap_or(DiscResult::Running)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

pub fn format_size(len: u64) -> String {
    const GB: u64 = 1 << 30;
    const MB: u64 = 1 << 20;
    if len >= GB {
        format!("{:.1} GB", len as f64 / GB as f64)
    } else if len >= MB {
        format!("{:.0} MB", len as f64 / MB as f64)
    } else {
        format!("{} KB", len.div_ceil(1024))
    }
}

fn wrong_size_message(len: u64) -> String {
    format!(
        "This is not the right disc image. It must be Ratchet & Clank PAL / Europe v1.00 or USA v1.00, one .iso file of about 3.9 GB. This file is {}. Other regions, other versions and Platinum / Greatest Hits re-releases do not work.",
        format_size(len)
    )
}

/// Size only (fast): which disc this file looks like, or the message why not.
pub fn quick_check(game: &GameDef, path: &Path) -> Result<&'static DiscDef, String> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file()).ok_or_else(|| "That file could not be found.".to_string())?;
    game.disc_by_size(meta.len()).ok_or_else(|| wrong_size_message(meta.len()))
}

fn run(game: &GameDef, c: &DiscCheck, verify_sha1: bool) -> DiscResult {
    let disc = match quick_check(game, &c.path) {
        Ok(d) => d,
        Err(m) => return DiscResult::Bad(m),
    };
    if !verify_sha1 {
        return DiscResult::Ok(disc);
    }
    c.total.store(disc.size, Ordering::Relaxed);
    let mut f = match std::fs::File::open(&c.path) {
        Ok(f) => f,
        Err(e) => return DiscResult::Bad(format!("The file could not be read: {e}")),
    };
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 4 << 20];
    loop {
        if c.cancel.load(Ordering::Relaxed) {
            return DiscResult::Cancelled;
        }
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                h.update(&buf[..n]);
                c.pos.fetch_add(n as u64, Ordering::Relaxed);
            }
            Err(e) => return DiscResult::Bad(format!("The file could not be read: {e}")),
        }
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02X}")).collect();
    if hex != disc.sha1 {
        return DiscResult::Bad(format!("This file has the right size but it is not the {} disc (its checksum is different). Try making the .iso again from your disc, or use another copy.", disc.name));
    }
    DiscResult::Ok(disc)
}

/// Starts the check on a thread. `verify_sha1 = false` only checks the size (tests).
pub fn start(game: &'static GameDef, path: PathBuf, verify_sha1: bool) -> Arc<DiscCheck> {
    let c = Arc::new(DiscCheck { path, pos: AtomicU64::new(0), total: AtomicU64::new(0), cancel: AtomicBool::new(false), result: Mutex::new(DiscResult::Running) });
    let c2 = c.clone();
    std::thread::spawn(move || {
        let r = run(game, &c2, verify_sha1);
        if let Ok(mut g) = c2.result.lock() {
            *g = r;
        }
    });
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::game_by_id;

    #[test]
    fn sizes_are_formatted_like_the_old_window() {
        assert_eq!(format_size(4096), "4 KB");
        assert_eq!(format_size(5 << 20), "5 MB");
        assert_eq!(format_size(4_213_997_568), "3.9 GB");
    }

    #[test]
    fn missing_and_wrong_size_files_are_refused_with_a_message() {
        let g = game_by_id("rac1").unwrap();
        assert!(quick_check(g, Path::new("Z:\\no\\such.iso")).unwrap_err().contains("could not be found"));
        let p = std::env::temp_dir().join(format!("racl_dummy_{}.iso", std::process::id()));
        std::fs::write(&p, vec![0u8; 4096]).unwrap();
        let e = quick_check(g, &p).unwrap_err();
        assert!(e.contains("not the right disc image") && e.contains("4 KB"), "{e}");
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn the_real_disc_is_recognised_by_size_and_checksum() {
        // the disc image two folders above the crate (the project layout this was written in), or RAC_TEST_ISO
        let iso = std::env::var_os("RAC_TEST_ISO").map(std::path::PathBuf::from);
        let iso = iso.unwrap_or_else(|| concat!(env!("CARGO_MANIFEST_DIR"), "/../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso").into());
        let p = iso.as_path();
        if !p.is_file() {
            return;
        }
        let g = game_by_id("rac1").unwrap();
        assert_eq!(quick_check(g, p).unwrap().serial, "SCES-50916");
        let c = start(g, p.to_path_buf(), true);
        for _ in 0..600 {
            if c.result() != DiscResult::Running {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert_eq!(c.result(), DiscResult::Ok(&crate::games::RAC1_DISCS[0]));
    }
}
