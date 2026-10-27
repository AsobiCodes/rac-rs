//! Manual real check of the backup (ignored by default; session 11b): backs up a COPY of a real finished install (the source is only read)
//! into a scratch folder on the same volume and checks that the texture pack is hard-linked, not copied.
//!   RAC_BK_SRC   the finished install (default E:\rac_cleantest\v032_launcher_test)
//!   RAC_BK_DEST  the scratch backup folder (must not exist)
//!   RAC_BK_SNAP  where the "before" listing of the pack is written (default next to the crate)
//! `real_backup_make` makes the backup and prints the numbers; `real_backup_delete` removes it and compares the pack with the listing.
use rac_rs_launcher::backup;
use rac_rs_launcher::games::game_by_id;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn env_path(n: &str, d: &str) -> PathBuf {
    std::env::var_os(n).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(d))
}

fn src() -> PathBuf {
    env_path("RAC_BK_SRC", r"E:\rac_cleantest\v032_launcher_test")
}

fn snap_file() -> PathBuf {
    env_path("RAC_BK_SNAP", concat!(env!("CARGO_MANIFEST_DIR"), "/../real_backup_before.txt"))
}

/// Sorted "relative path|len|modified-nanos" lines of everything under `dir`.
fn listing(dir: &Path) -> String {
    let mut v: Vec<String> = backup::walk(dir)
        .into_iter()
        .map(|(rel, len)| {
            let t = std::fs::metadata(dir.join(&rel)).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_nanos()).unwrap_or(0);
            format!("{}|{len}|{t}", rel.display())
        })
        .collect();
    v.sort();
    v.join("\n")
}

#[test]
#[ignore]
fn real_backup_make() {
    let root = src();
    let dest = PathBuf::from(std::env::var("RAC_BK_DEST").expect("RAC_BK_DEST"));
    assert!(!dest.exists(), "the scratch backup folder exists already");
    let g = game_by_id("rac1").unwrap();
    let version = std::fs::read_to_string(root.join("VERSION")).unwrap().trim().to_string();
    let pack_dir = root.join(g.mods);
    let before = listing(&pack_dir);
    std::fs::write(snap_file(), &before).unwrap();
    println!("source mods listing: {} files", before.lines().count());
    let mut plan = backup::plan(&root, g, &version).expect("plan");
    println!("plan: copy {} B, shared {} B, left out {} B, link_ok {}, packs {:?}", plan.copy_bytes(), plan.shared_bytes(), plan.left_out_bytes(), plan.link_ok, plan.packs.iter().map(|p| (p.name.clone(), p.bytes, p.files)).collect::<Vec<_>>());
    plan.dest = dest.clone();
    let t = Instant::now();
    let st = backup::start(Arc::new(plan));
    while !st.finished() {
        std::thread::sleep(Duration::from_millis(100));
        assert!(t.elapsed() < Duration::from_secs(900), "the backup took too long");
    }
    let r = st.take_result().unwrap();
    println!("result {r:?} in {:?}; done_bytes {} of {}, files {} of {}; left out {:?}", t.elapsed(), st.done_bytes.load(std::sync::atomic::Ordering::Relaxed), st.total_bytes.load(std::sync::atomic::Ordering::Relaxed), st.done_files.load(std::sync::atomic::Ordering::Relaxed), st.total_files.load(std::sync::atomic::Ordering::Relaxed), st.left_out_names());
    r.expect("backup");
    assert!(dest.join("backup.txt").is_file());
    assert_eq!(listing(&pack_dir), before, "the source pack changed while backing up");
    println!("source pack listing unchanged after the backup");
}

#[test]
#[ignore]
fn real_backup_delete() {
    let dest = PathBuf::from(std::env::var("RAC_BK_DEST").expect("RAC_BK_DEST"));
    let g = game_by_id("rac1").unwrap();
    assert!(dest.join("backup.txt").is_file(), "not a backup folder: refusing to delete");
    backup::cleanup(&dest);
    assert!(!dest.exists());
    let before = std::fs::read_to_string(snap_file()).unwrap();
    assert_eq!(listing(&src().join(g.mods)), before, "the source pack changed (size or time)");
    println!("backup removed; source pack listing identical ({} files)", before.lines().count());
}
