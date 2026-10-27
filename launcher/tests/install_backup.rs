//! Install/repair through the launcher's setup engine (a fake setup.sh) and a manual backup.
use rac_rs_launcher::backup;
use rac_rs_launcher::ui::state::{App, View};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("racl_flow_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

/// A game source folder (VERSION + setup.sh) and a fake setup.sh that prints the progress markers.
fn source(name: &str) -> PathBuf {
    let d = scratch(name);
    write(&d.join("VERSION"), "0.4.0\n");
    write(&d.join("play.sh"), "#!/bin/sh\n");
    write(
        &d.join("setup.sh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > args.txt\nprintf '== (a) tools\\n== (b) disc\\n  level 0 done: ok\\n== (c) code\\n== (c2) vu\\n== (d) build\\n   Compiling foo\\nexpect-compiles: 10\\n== done\\n'\n",
    );
    d
}

/// A built game in `root`.
fn built(root: &Path) {
    write(&root.join("VERSION"), "0.4.0\n");
    write(&root.join("play.sh"), "#!/bin/sh\n");
    write(&root.join("setup.sh"), "#!/bin/sh\n");
    write(&root.join("build/release/rac-rs"), "binary");
    write(&root.join("generated/play.cfg"), "iso=/tmp/rc.iso\n");
    write(&root.join("generated/built_version.txt"), "0.4.0\n");
    write(&root.join("player/card.bin"), "saves");
    write(&root.join("mods/active.txt"), "# none\n");
}

fn wait_done(a: &mut App) -> bool {
    let end = Instant::now() + Duration::from_secs(20);
    while Instant::now() < end {
        a.poll_engine();
        if a.view == View::Done || a.view == View::Failed {
            return a.view == View::Done;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    false
}

#[test]
fn new_install_runs_setup_sh_with_the_disc() {
    let root = source("install");
    let mut a = App::new(root.clone());
    a.shortcut_wanted = false;
    a.disc_path = Some(PathBuf::from("/tmp/rc.iso"));
    a.start_install();
    assert!(wait_done(&mut a), "the fake setup finishes");
    let args = std::fs::read_to_string(root.join("args.txt")).unwrap();
    assert!(args.contains("--iso\n/tmp/rc.iso"), "{args}");
    assert!(args.contains("--no-pause") && args.contains("--skip-hash"), "{args}");
    assert!(!args.contains("--regenerate"), "an install does not regenerate: {args}");
}

#[test]
fn repair_regenerates_through_the_same_engine() {
    let root = scratch("repair");
    write(&root.join("VERSION"), "0.4.0\n");
    write(&root.join("play.sh"), "#!/bin/sh\n");
    write(
        &root.join("setup.sh"),
        "#!/bin/sh\nprintf '%s\\n' \"$@\" > args.txt\nprintf '== (d) build\\n== done\\n'\n",
    );
    let mut a = App::new(root.clone());
    a.shortcut_wanted = false;
    a.view = View::Repair;
    a.disc_path = Some(PathBuf::from("/tmp/rc.iso"));
    a.start_install();
    assert!(wait_done(&mut a), "the fake repair finishes");
    let args = std::fs::read_to_string(root.join("args.txt")).unwrap();
    assert!(args.contains("--regenerate"), "{args}");
}

#[test]
fn a_manual_backup_is_listed_and_playable() {
    let root = scratch("backup");
    built(&root);
    let mut a = App::new(root.clone());
    a.begin_backup();
    assert_eq!(a.view, View::Backup);
    let end = Instant::now() + Duration::from_secs(30);
    let mut made = None;
    while Instant::now() < end {
        a.poll_backup();
        made = a.library.entries.iter().find(|e| e.backup).cloned();
        if made.is_some() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    let made = made.expect("the backup is in the library");
    assert!(made.pinned);
    let dir = made.path();
    assert!(dir.is_dir());
    assert!(dir.join(backup::MARKER).is_file());
    assert!(dir.join("build/release/rac-rs").is_file());
    assert!(dir.join("player/card.bin").is_file());
    assert_eq!(std::fs::read_to_string(dir.join("player/card.bin")).unwrap(), "saves");
    assert!(root.join("build/release/rac-rs").is_file(), "the install is untouched");
    let _ = std::fs::remove_dir_all(&root);
}
