//! UI-flow tests of "Back up your current version first": the tick on the update banner and the pinned row (default on, greyed with the
//! reason when the space is short), the whole update flow (backup entry, then the update, here a dummy setup.ps1 that fails fast),
//! the progress card and Cancel (held with `backup::TEST_HOLD`), a copy that fails, the failed-update screen with "Play the backup",
//! and the Library side ("Backup" chip, "Delete backup" with a second click; no banner and no Repair on a backup).
use egui::vec2;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use rac_rs_launcher::backup;
use rac_rs_launcher::library::Library;
use rac_rs_launcher::ui::state::{App, Sel, Tab, View};
use rac_rs_launcher::ui::{theme, LauncherApp};
use rac_rs_launcher::update::{UpdateOffer, UpdateState};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

/// TEST_HOLD is process-wide: one test at a time.
static ONE: std::sync::Mutex<()> = std::sync::Mutex::new(());

struct Hold;
impl Hold {
    fn on() -> Hold {
        backup::TEST_HOLD.store(true, Ordering::SeqCst);
        Hold
    }
    fn off(&self) {
        backup::TEST_HOLD.store(false, Ordering::SeqCst);
    }
}
impl Drop for Hold {
    fn drop(&mut self) {
        backup::TEST_HOLD.store(false, Ordering::SeqCst);
    }
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("racl_bk_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, t: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, t).unwrap();
}

fn install(root: &Path, version: &str) {
    write(&root.join("build/release/rac-rs.exe"), "x");
    write(&root.join("Play.cmd"), "x");
    write(&root.join("setup.ps1"), "x");
    write(&root.join("rc.iso"), "disc");
    write(&root.join("VERSION"), &format!("{version}\n"));
    write(&root.join("generated/built_version.txt"), &format!("{version}\n"));
    write(&root.join("player/pc_options.txt"), "version=1\nlighting=classic\n");
    write(&root.join("player/card.bin"), "memory card bytes");
    write(&root.join("generated/play.cfg"), &format!("iso={}\n", root.join("rc.iso").display()));
}

/// An App with one install (the open one) at `version`; `Env.data` is the launcher data folder.
fn backup_app(name: &str, version: &str) -> (App, PathBuf, PathBuf) {
    let data = scratch(&format!("{name}_data"));
    let root = scratch(&format!("{name}_i0"));
    install(&root, version);
    let mut lib = Library::load(&data);
    let id = lib.add_existing(&root).unwrap().id;
    lib.save().unwrap();
    let mut app = App::with_library(data.clone(), lib, Some(id), Some(data.join("_no_remote").to_string_lossy().to_string()));
    app.covers.dir = Some(data.join("_no_covers"));
    app.sel = Sel::Game("rac1");
    app.tab = Tab::Play;
    (app, data, root)
}

/// Sets the update state AFTER the background check of the scratch install has finished (it would overwrite it otherwise).
fn offer_update(a: &mut App, from: &str, to: &str) {
    for _ in 0..200 {
        if a.update_state() != UpdateState::Checking {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    *a.update.lock().unwrap() = UpdateState::Available(UpdateOffer { from: from.into(), to: to.into(), changes: String::new(), finish: false });
}

fn harness(app: App) -> Harness<'static, LauncherApp> {
    Harness::builder().with_size(vec2(1100.0, 800.0)).with_context_setup(|ctx| theme::apply(ctx)).build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(app))
}

/// Steps the UI (and lets worker threads work) until `cond` holds; panics after `secs`.
fn until(h: &mut Harness<'static, LauncherApp>, secs: u64, what: &str, cond: impl Fn(&App) -> bool) {
    let end = Instant::now() + Duration::from_secs(secs);
    let ctx = egui::Context::default();
    while Instant::now() < end {
        // the window loop calls tick() every frame; the offscreen harness only draws, so do it here
        h.state_mut().app.tick(&ctx);
        h.run_steps(2);
        if cond(&h.state().app) {
            h.run_steps(2);
            return;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    let a = &h.state().app;
    panic!("timed out waiting for: {what} (view {:?}, engine {}, job {}, entries {}, backup.error {:?})", a.view, a.engine.is_some(), a.backup.job.is_some(), a.library.entries.len(), a.backup.error);
}

fn tick_label(v: &str) -> String {
    format!("Back up your current version (v{v}) first")
}

fn is_on(h: &Harness<'static, LauncherApp>, label: &str) -> bool {
    format!("{:?}", h.get_by_label(label).accesskit_node().toggled()).contains("True")
}

/// Wait for the plan to be read: the tick is on screen.
fn wait_tick(h: &mut Harness<'static, LauncherApp>, v: &str) {
    let l = tick_label(v);
    for _ in 0..300 {
        h.run_steps(2);
        if h.query_by_label(&l).is_some() {
            return;
        }
        std::thread::sleep(Duration::from_millis(15));
    }
    panic!("the backup tick never appeared");
}

fn backup_dirs(root: &Path) -> Vec<PathBuf> {
    let parent = root.parent().unwrap();
    let stem = root.file_name().unwrap().to_string_lossy().to_string();
    std::fs::read_dir(parent).unwrap().flatten().map(|e| e.path()).filter(|p| p.file_name().unwrap().to_string_lossy().starts_with(&format!("{stem} (backup"))).collect()
}

fn cleanup(data: &Path, root: &Path) {
    for b in backup_dirs(root) {
        let _ = std::fs::remove_dir_all(b);
    }
    let _ = std::fs::remove_dir_all(data);
    let _ = std::fs::remove_dir_all(root);
}

// ------------------------------------------------------------------ the tick

#[test]
fn the_tick_is_on_by_default_and_unticking_is_remembered() {
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut a, data, root) = backup_app("tick", "0.3.1");
    offer_update(&mut a, "0.3.1", "0.4.0");
    let mut h = harness(a);
    wait_tick(&mut h, "0.3.1");
    let l = tick_label("0.3.1");
    assert!(is_on(&h, &l), "on by default");
    assert!(!h.get_by_label(&l).accesskit_node().is_disabled());
    assert!(h.query_by_label_contains("Needs about").is_some(), "the size line is shown");
    assert!(h.state().app.cfg.backup_before_update);
    h.get_by_label(&l).click();
    h.run_steps(3);
    assert!(!h.state().app.cfg.backup_before_update, "unticked");
    assert!(!is_on(&h, &l));
    let saved = std::fs::read_to_string(data.join("launcher.cfg")).unwrap_or_default();
    assert!(saved.contains("backup_before_update=off"), "remembered in launcher.cfg: {saved}");
    cleanup(&data, &root);
}

#[test]
fn the_tick_is_greyed_with_the_reason_when_the_space_is_short() {
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut a, data, root) = backup_app("short", "0.3.1");
    a.backup.free_override = Some(10);
    offer_update(&mut a, "0.3.1", "0.4.0");
    let mut h = harness(a);
    wait_tick(&mut h, "0.3.1");
    let l = tick_label("0.3.1");
    assert!(h.get_by_label(&l).accesskit_node().is_disabled(), "greyed");
    assert!(!is_on(&h, &l), "and not ticked");
    assert!(h.query_by_label_contains("The update runs without a backup").is_some(), "the reason is shown");
    // the update still starts, without a backup
    h.get_by_label("Update now").click();
    h.run_steps(3);
    assert!(h.state().app.backup.job.is_none(), "no backup was started");
    assert_ne!(h.state().app.view, View::Backup);
    assert!(h.state().app.cfg.backup_before_update, "the saved choice is untouched by a short disk");
    assert!(backup_dirs(&root).is_empty());
    cleanup(&data, &root);
}

#[test]
fn the_pinned_row_has_the_tick_too() {
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut a, data, root) = backup_app("pin", "0.3.1");
    let id = a.cur_id.clone().unwrap();
    a.library.set_pinned(&id, true);
    offer_update(&mut a, "0.3.1", "0.4.0");
    let mut h = harness(a);
    wait_tick(&mut h, "0.3.1");
    assert!(is_on(&h, &tick_label("0.3.1")));
    assert!(h.query_by_label("Update to 0.4.0").is_some());
    cleanup(&data, &root);
}

// ------------------------------------------------------------------ the flow

#[test]
fn update_now_makes_a_backup_entry_then_starts_the_update_and_a_failed_update_offers_the_backup() {
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut a, data, root) = backup_app("flow", "0.3.1");
    write(&root.join("setup.ps1"), "exit 3
"); // the "setup" fails at once
    offer_update(&mut a, "0.3.1", "0.4.0");
    let main_id = a.cur_id.clone().unwrap();
    let mut h = harness(a);
    wait_tick(&mut h, "0.3.1");
    h.get_by_label("Update now").click();
    // the dummy setup.ps1 fails fast: the failed screen comes after the backup was made
    until(&mut h, 60, "the failed update screen", |a| a.view == View::Failed);
    let app = &h.state().app;
    let entries = app.library.entries.clone();
    assert_eq!(entries.len(), 2, "main install + backup");
    let b = entries.iter().find(|e| e.backup).expect("a backup entry");
    assert!(b.pinned && !b.name.is_empty(), "pinned and named: {b:?}");
    assert!(b.name.contains("0.3.1") && b.name.contains("backup"), "{}", b.name);
    let bpath = PathBuf::from(&b.folder);
    assert!(bpath.is_dir() && bpath.parent() == root.parent(), "the folder is next to the install: {}", bpath.display());
    assert!(bpath.join("backup.txt").is_file() && bpath.join("player/card.bin").is_file() && bpath.join("VERSION").is_file());
    assert!(!bpath.join("setup.ps1").exists(), "the backup cannot update itself");
    assert_eq!(std::fs::read_to_string(bpath.join("player/card.bin")).unwrap(), "memory card bytes");
    assert_eq!(app.cur_id.as_deref(), Some(main_id.as_str()), "the main install stays open");
    // the saved library has it
    assert!(Library::load(&data).entries.iter().any(|e| e.backup));
    // the failed screen offers the backup
    let bid = b.id.clone();
    h.get_by_label("Play the backup (v0.3.1)").click();
    h.run_steps(3);
    assert_eq!(h.state().app.cur_id.as_deref(), Some(bid.as_str()), "the backup is open now");
    cleanup(&data, &root);
}

#[test]
fn the_progress_card_and_cancel_remove_the_partial_backup() {
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let hold = Hold::on();
    let (mut a, data, root) = backup_app("cancel", "0.3.1");
    offer_update(&mut a, "0.3.1", "0.4.0");
    let mut h = harness(a);
    wait_tick(&mut h, "0.3.1");
    h.get_by_label("Update now").click();
    until(&mut h, 20, "the progress card", |a| a.view == View::Backup && a.backup.job.is_some());
    assert!(h.query_by_label_contains("Backing up version 0.3.1 first").is_some());
    assert!(h.query_by_label("Cancel").is_some());
    h.get_by_label("Cancel").click();
    h.run_steps(2);
    hold.off();
    until(&mut h, 20, "the cancel to finish", |a| a.backup.job.is_none());
    let app = &h.state().app;
    assert_eq!(app.library.entries.len(), 1, "no entry");
    assert!(backup_dirs(&root).is_empty(), "the partial folder is gone");
    assert!(app.engine.is_none() && app.view != View::Failed && app.view != View::Run, "the update was not started");
    assert!(app.toast.is_some());
    cleanup(&data, &root);
}

#[cfg(windows)]
#[test]
fn a_failed_copy_leaves_no_folder_no_entry_and_no_update() {
    use std::os::windows::fs::OpenOptionsExt;
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut a, data, root) = backup_app("fail", "0.3.1");
    offer_update(&mut a, "0.3.1", "0.4.0");
    let mut h = harness(a);
    wait_tick(&mut h, "0.3.1");
    // a file that cannot be read by anyone else
    let _lock = std::fs::OpenOptions::new().read(true).write(true).share_mode(0).open(root.join("player/card.bin")).unwrap();
    h.get_by_label("Update now").click();
    until(&mut h, 30, "the failed backup", |a| a.backup.error.is_some());
    let app = &h.state().app;
    assert!(app.backup.error.as_ref().unwrap().contains("update was not started"));
    assert_eq!(app.library.entries.len(), 1);
    assert!(backup_dirs(&root).is_empty(), "the folder was removed");
    assert!(app.engine.is_none() && app.view == View::Home);
    assert!(h.query_all_by_label_contains("The backup could not be made").count() >= 1, "the note is shown on the update card");
    drop(_lock);
    cleanup(&data, &root);
}

// ------------------------------------------------------------------ the Library side

/// An App with a main install (0.4.0) and a backup entry of 0.3.1 next to it.
fn with_backup_entry(name: &str) -> (App, PathBuf, PathBuf, PathBuf, String) {
    let data = scratch(&format!("{name}_data"));
    let root = scratch(&format!("{name}_main"));
    install(&root, "0.4.0");
    let bdir = scratch(&format!("{name}_main (backup v0.3.1)"));
    install(&bdir, "0.3.1");
    let mut lib = Library::load(&data);
    let main_id = lib.add_existing(&root).unwrap().id;
    let g = rac_rs_launcher::games::game_by_id("rac1").unwrap();
    let bid = lib.add_backup(g, &bdir, "Ratchet & Clank v0.3.1 (backup)").id;
    lib.save().unwrap();
    let mut app = App::with_library(data.clone(), lib, Some(main_id), Some(data.join("_no_remote").to_string_lossy().to_string()));
    app.covers.dir = Some(data.join("_no_covers"));
    (app, data, root, bdir, bid)
}

#[test]
fn a_backup_shows_its_chip_and_delete_needs_a_second_click() {
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    // the scratch backup is deleted for good, not put in the user's Recycle Bin
    rac_rs_launcher::platform::TEST_NO_RECYCLE_BIN.store(true, Ordering::SeqCst);
    let (mut a, data, root, bdir, bid) = with_backup_entry("lib");
    a.sel = Sel::Library;
    let mut h = harness(a);
    h.run_steps(3);
    // (the "Backup" chip is painted, not a widget: it is in the screenshot 49)
    assert!(h.query_all_by_label_contains("(backup)").count() >= 1, "the backup's own name is shown");
    assert!(h.state().app.library.entries.iter().any(|e| e.backup));
    assert!(h.query_by_label("Delete backup").is_some());
    h.get_by_label("Delete backup").click();
    h.run_steps(3);
    assert_eq!(h.state().app.lib_ui.confirm_delete.as_deref(), Some(bid.as_str()));
    assert!(bdir.is_dir(), "one click deletes nothing");
    assert!(h.query_by_label("Yes, delete the backup").is_some());
    // Cancel backs out
    h.get_by_label("Cancel").click();
    h.run_steps(3);
    assert!(h.state().app.lib_ui.confirm_delete.is_none() && bdir.is_dir());
    // the second click does it (Recycle Bin)
    h.get_by_label("Delete backup").click();
    h.run_steps(3);
    h.get_by_label("Yes, delete the backup").click();
    h.run_steps(3);
    let app = &h.state().app;
    assert!(!bdir.exists(), "the folder is gone (the Recycle Bin path minus the bin)");
    assert_eq!(app.library.entries.len(), 1);
    assert!(app.library.get(&bid).is_none());
    assert_eq!(Library::load(&data).entries.len(), 1, "saved");
    assert!(root.join("VERSION").is_file(), "the main install is untouched");
    cleanup(&data, &root);
}

#[test]
fn a_backup_has_no_update_banner_and_no_repair() {
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut a, data, root, bdir, bid) = with_backup_entry("nobanner");
    assert!(a.select_install(&bid));
    assert!(a.is_backup());
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Play;
    // even if the main install's offer were still around, a backup never shows it
    *a.update.lock().unwrap() = UpdateState::Available(UpdateOffer { from: "0.3.1".into(), to: "0.4.0".into(), changes: String::new(), finish: false });
    let mut h = harness(a);
    h.run_steps(4);
    assert!(h.query_by_label("Update now").is_none());
    assert!(h.query_by_label_contains("is available").is_none());
    assert!(h.query_by_label_contains("as it was before an update").is_some(), "the backup note");
    // begin_update only tells why
    h.state_mut().app.begin_update("0.4.0", false);
    assert!(h.state().app.toast.is_some() && h.state().app.view != View::Run && h.state().app.engine.is_none());
    // Settings: no Repair
    h.state_mut().app.tab = Tab::Settings;
    h.run_steps(4);
    assert!(h.query_by_label("Repair / rebuild...").is_none());
    let _ = std::fs::remove_dir_all(&bdir);
    cleanup(&data, &root);
}

#[test]
fn a_finished_update_tells_where_the_old_version_is_kept() {
    let _g = ONE.lock().unwrap_or_else(|e| e.into_inner());
    let (mut a, data, root) = backup_app("done", "0.3.1");
    write(&root.join("setup.ps1"), "exit 0
"); // the "setup" succeeds at once
    offer_update(&mut a, "0.3.1", "0.4.0");
    let mut h = harness(a);
    wait_tick(&mut h, "0.3.1");
    h.get_by_label("Update now").click();
    until(&mut h, 60, "the update to finish", |a| a.view == View::Done);
    let note = h.state().app.done_note.clone();
    assert!(note.contains("Your old version is kept as") && note.contains("(backup)"), "{note}");
    assert_eq!(h.state().app.library.entries.iter().filter(|e| e.backup).count(), 1);
    cleanup(&data, &root);
}
