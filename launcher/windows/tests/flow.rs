//! Engine flow test: the launcher's Engine runs the REAL setup.ps1 of a scratch copy of a finished install with `-CheckOnly`
//! (tools check + disc size check, no build) and parses its output into Progress; then the same through App::start_install.
//! The copy (`flow_base` next to the crate folder, ~1.4 GB incl. the toolchain, no game build) is made once from a finished
//! install given as RAC_FLOW_SRC (never modified); override the copy with RAC_FLOW_BASE and the disc image with RAC_FLOW_ISO.
//! No window and no game start. The tests skip (print why) when the install or the disc is not on this PC.
use rac_rs_launcher::engine::{Engine, EngineOpts};
use rac_rs_launcher::ui::state::{App, View};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// Both tests run setup.ps1 in the same folder: one at a time.
static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn env_path(name: &str, default: &str) -> PathBuf {
    std::env::var_os(name).map(PathBuf::from).unwrap_or_else(|| PathBuf::from(default))
}

/// The scratch copy to run setup.ps1 in, or None (skip) when the sources are missing.
fn flow_base() -> Option<PathBuf> {
    let base = env_path("RAC_FLOW_BASE", concat!(env!("CARGO_MANIFEST_DIR"), "/../flow_base"));
    if !base.join("toolchain").is_dir() {
        let src = env_path("RAC_FLOW_SRC", "");
        if src.as_os_str().is_empty() || !src.join("toolchain").is_dir() {
            eprintln!("flow test skipped: no finished install at {}", src.display());
            return None;
        }
        // robocopy exit codes below 8 mean success
        let st = Command::new("robocopy").arg(&src).arg(&base).args(["/E", "/XD", "build", "rac-rs", "/XF", "*.iso", "/NFL", "/NDL", "/NJH", "/NJS", "/NP"]).status().ok()?;
        assert!(st.code().unwrap_or(99) < 8, "robocopy failed: {st:?}");
    }
    // the setup.ps1 under test = the launcher's patched copy next to the crate
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("setup.ps1");
    std::fs::copy(&script, base.join("setup.ps1")).expect("copy setup.ps1");
    Some(base)
}

fn iso() -> Option<PathBuf> {
    let p = env_path("RAC_FLOW_ISO", concat!(env!("CARGO_MANIFEST_DIR"), "/../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
    if p.is_file() {
        Some(p)
    } else {
        eprintln!("flow test skipped: no disc image at {}", p.display());
        None
    }
}

#[test]
fn check_only_through_the_engine() {
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (Some(base), Some(iso)) = (flow_base(), iso()) else { return };
    let opts = EngineOpts { root: base.clone(), iso, update: None, regenerate: false, test_update_base: None, extra: vec!["-CheckOnly".to_string()] };
    let mut e = Engine::start(&opts).expect("start setup.ps1");
    let t = Instant::now();
    while e.done.is_none() {
        e.poll();
        std::thread::sleep(Duration::from_millis(40));
        assert!(t.elapsed() < Duration::from_secs(180), "setup.ps1 -CheckOnly did not end:\n{}", e.progress.log.join("\n"));
    }
    let log = e.progress.log.join("\n");
    assert_eq!(e.done, Some(0), "exit code; log:\n{log}");
    let p = &e.progress;
    assert!(p.last_error.is_empty(), "error: {} / {}", p.last_error, log);
    assert!(log.contains("== (a) tools") && log.contains("== (b) disc image"), "steps (a) and (b) reached:\n{log}");
    assert!(log.contains("Checks passed."), "{log}");
    assert!(!log.contains("== (c)") && !log.contains("== (d)"), "-CheckOnly must stop before (c):\n{log}");
    assert!(p.finished && p.step == 4 && p.compiled == 0 && p.exit_code == Some(0));
}

#[test]
fn check_only_through_the_app() {
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (Some(base), Some(iso)) = (flow_base(), iso()) else { return };
    // the App reads RAC_LAUNCHER_SETUP_ARGS for extra worker switches; this is the only test in this file that sets it
    std::env::set_var("RAC_LAUNCHER_SETUP_ARGS", "-CheckOnly");
    let shortcuts = std::env::temp_dir().join(format!("racl_flow_sc_{}", std::process::id()));
    let mut app = App::new(base.clone(), Some(base.join("_no_remote").to_string_lossy().to_string()));
    app.shortcut_wanted = false;
    app.shortcut_dir = Some(shortcuts.clone());
    app.disc_path = Some(iso);
    app.start_install();
    assert_eq!(app.view, View::Run, "the worker started");
    let t = Instant::now();
    while app.engine.is_some() {
        app.poll_engine();
        std::thread::sleep(Duration::from_millis(40));
        assert!(t.elapsed() < Duration::from_secs(180), "worker did not end");
    }
    std::env::remove_var("RAC_LAUNCHER_SETUP_ARGS");
    assert!(app.failure.is_none(), "failure: {:?}", app.failure.as_ref().map(|f| &f.message));
    assert_eq!(app.view, View::Done);
    let _ = std::fs::remove_dir_all(&shortcuts);
}
