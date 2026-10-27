//! REAL network check against the public repo AsobiCodes/rac-rs (ignored by default; run once with
//! `cargo test --test net_real -- --ignored --nocapture`). Unauthenticated GitHub calls only (2 API calls, 2 asset downloads of ~5 MB).
//! It never runs the disc setup: the downloaded game is only unpacked into `nettest\` next to the crate folder and looked at.
use rac_rs_launcher::game_dl::{self, installable};
use rac_rs_launcher::releases::{self, ReleaseSource, Transfer};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU8;
use std::sync::Mutex;

fn nettest_dir() -> PathBuf {
    let d = std::env::var_os("RAC_NETTEST_DIR").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../nettest")));
    assert!(d.ends_with("nettest"), "refusing to clean a folder that is not named nettest: {}", d.display());
    d
}

fn tree(root: &Path) -> Vec<String> {
    fn walk(base: &Path, dir: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            let rel = p.strip_prefix(base).unwrap().to_string_lossy().replace('\\', "/");
            if p.is_dir() {
                out.push(format!("{rel}/"));
                walk(base, &p, out);
            } else {
                out.push(format!("{rel} {}", p.metadata().unwrap().len()));
            }
        }
    }
    let mut v = Vec::new();
    walk(root, root, &mut v);
    v.sort();
    v
}

#[test]
#[ignore = "talks to the real GitHub; run by hand once"]
fn real_repo_lists_four_releases_and_downloads_0_2_2() {
    let src = ReleaseSource::github(releases::GAME_REPO);

    // 1. the list the version picker gets
    let list = src.list().expect("GitHub release list of AsobiCodes/rac-rs");
    for r in &list {
        println!("release {} draft={} pre={} assets={:?}", r.tag_name, r.draft, r.prerelease, r.assets.iter().map(|a| format!("{} {} {}", a.name, a.size, a.digest.as_deref().unwrap_or("-"))).collect::<Vec<_>>());
    }
    let picker: Vec<String> = installable(&list, false).into_iter().map(|i| i.version).collect();
    assert_eq!(picker, vec!["0.3.1", "0.3.0", "0.2.2", "0.2.1"], "the picker must get exactly these four, newest first");
    let with_test: Vec<String> = installable(&list, true).into_iter().map(|i| i.version).collect();
    assert_eq!(with_test, picker, "the repo has no test versions");
    for r in &list {
        let v = r.version();
        let a = r.asset(&format!("rac-rs-{v}.zip")).unwrap_or_else(|| panic!("{v}: no rac-rs-{v}.zip"));
        assert!(a.size > 1_000_000, "{v}: zip size {}", a.size);
        assert!(releases::digest_hex(a).is_some(), "{v}: sha256 digest");
    }

    // 2. download v0.2.2 by hand: digest compared here as well as inside download()
    let root = nettest_dir();
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    let rel = list.iter().find(|r| r.version() == "0.2.2").unwrap();
    let asset = game_dl::game_asset(rel).expect("game zip of 0.2.2").clone();
    let zip = root.join("zip").join(&asset.name);
    let t = Transfer::new();
    releases::download(&src, &asset, &zip, &t).expect("download + digest check");
    let got = releases::sha256_file(&zip).unwrap();
    assert_eq!(Some(got.clone()), releases::digest_hex(&asset), "sha256 of the downloaded file equals the GitHub digest");
    assert_eq!(zip.metadata().unwrap().len(), asset.size, "size equals the asset size");
    assert!(!PathBuf::from(format!("{}.part", zip.display())).exists(), "no .part left");
    println!("downloaded {} bytes, sha256 {got}", asset.size);
    let by_hand = root.join("by_hand");
    game_dl::unpack(&zip, &by_hand, &t).expect("unpack");
    assert!(by_hand.join("setup.ps1").is_file(), "setup.ps1 at the root of the install folder");
    assert_eq!(std::fs::read_to_string(by_hand.join("VERSION")).unwrap().trim(), "0.2.2");
    assert!(by_hand.join("Setup.cmd").is_file() || by_hand.join("Play.cmd").is_file() || by_hand.join("README.md").is_file());

    // 3. the app's own path (what "New install" runs): lookup + download + unpack + cleanup
    let app_dir = root.join("app_path");
    let (t2, stage, ver) = (Transfer::new(), AtomicU8::new(0), Mutex::new(String::new()));
    let v = game_dl::run(&src, &app_dir, Some("0.2.2"), &t2, &stage, &ver).expect("game_dl::run");
    assert_eq!(v, "0.2.2");
    assert!(app_dir.join("setup.ps1").is_file());
    assert!(!app_dir.join(".racl_download").exists(), "the temporary download folder is removed");
    assert_eq!(tree(&app_dir), tree(&by_hand), "both ways give the same files");
    println!("unpacked files: {}", tree(&app_dir).len());
}
