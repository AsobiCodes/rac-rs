//! Offscreen snapshots of the library and profile UI (session 5): library with 2 installs, add-install dialog,
//! profile picker open (+ New form), settings with a greyed row, Play tab with the picker. PNGs go to `shots\` next to the crate folder.
use egui::vec2;
use egui_kittest::Harness;
use rac_rs_launcher::ui::state::{App, Sel, Tab};
use rac_rs_launcher::ui::state_lib::{AddDialog, EditKind};
use rac_rs_launcher::ui::{theme, LauncherApp};
use std::path::{Path, PathBuf};

fn shots_dir() -> PathBuf {
    let d = std::env::var_os("RAC_LAUNCHER_SHOTS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/target/shots")));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("racl_shots2_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn install(root: &Path, version: &str) {
    for (f, t) in [
        ("build/release/rac-rs", "x".to_string()),
        ("play.sh", "x".to_string()),
        ("setup.sh", "x".to_string()),
        ("rc.iso", "disc".to_string()),
        ("VERSION", format!("{version}\n")),
        ("generated/built_version.txt", format!("{version}\n")),
        ("player/pc_options.txt", "version=1\nlighting=enhanced\n".to_string()),
        ("generated/play.cfg", format!("iso={}\n", root.join("rc.iso").display())),
    ] {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }
}

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn app(name: &str, version: &str) -> App {
    let root = scratch(name);
    install(&root, version);
    let mut a = App::new(root.clone());
    a.covers.dir = Some(root.join("_no_covers"));
    a
}

fn shot(name: &str, a: App) {
    let _g = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = Harness::builder().with_size(vec2(1100.0, 700.0)).with_pixels_per_point(1.0).with_context_setup(|ctx| theme::apply(ctx)).wgpu().build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(a));
    for _ in 0..4 {
        h.step();
    }
    let img = h.render().expect("offscreen render");
    img.save(shots_dir().join(format!("{name}.png"))).unwrap();
}

#[test]
fn s24_library_two_installs() {
    let mut a = app("lib", "0.4.0");
    let second = scratch("lib2");
    install(&second, "0.3.1");
    a.library.add_existing(&second).unwrap();
    a.sel = Sel::Library;
    shot("24_library_two_installs", a);
}

#[test]
fn s25_add_install_dialog() {
    let mut a = app("add", "0.4.0");
    a.sel = Sel::Library;
    a.lib_ui.dialog = Some(AddDialog::New { text: r"C:\Users\Player\Documents\RAC-RS\Ratchet & Clank".into() });
    shot("25_new_install_dialog", a);
    let mut b = app("add2", "0.4.0");
    b.sel = Sel::Library;
    b.lib_ui.dialog = Some(AddDialog::Existing { text: r"D:\Games\no such folder".into() });
    shot("26_add_existing_dialog", b);
}

#[test]
fn s27_profile_picker_open() {
    let mut a = app("pick", "0.4.0");
    a.profiles.create("Steam Deck").unwrap();
    a.profile_list = a.profiles.list();
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Play;
    a.profile_ui.open = true;
    shot("27_profile_picker_open", a);
    let mut b = app("pick2", "0.4.0");
    b.sel = Sel::Game("rac1");
    b.tab = Tab::Settings;
    b.profile_ui.open = true;
    b.profile_begin(EditKind::New);
    shot("28_profile_new_form", b);
}

#[test]
fn s29_settings_greyed_row_and_play_tab() {
    let mut a = app("grey", "0.3.1");
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Settings;
    shot("29_settings_greyed_renderer", a);
    let mut b = app("running", "0.4.0");
    b.sel = Sel::Game("rac1");
    b.tab = Tab::Settings;
    b.game_external = true;
    shot("30_settings_game_running_readonly", b);
}

// ---------------------------------------------------------------------------------------------------------------------------------
// More states: saves buttons, greyed levels, manual backups.

use egui_kittest::kittest::Queryable;

/// Like `shot`, with a window size, a pixel scale and a step that may click things first.
fn shot_with(name: &str, a: App, size: egui::Vec2, ppp: f32, act: impl FnOnce(&mut Harness<'static, LauncherApp>)) {
    let _g = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(ppp).with_context_setup(|ctx| theme::apply(ctx)).wgpu().build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(a));
    for _ in 0..4 {
        h.step();
    }
    act(&mut h);
    for _ in 0..4 {
        h.step();
    }
    let img = h.render().expect("offscreen render");
    img.save(shots_dir().join(format!("{name}.png"))).unwrap();
}

#[test]
fn s40_open_saves_folder_buttons() {
    let mut a = app("sv1", "0.3.1");
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Play;
    shot_with("40_play_tab_open_saves_folder", a, vec2(1100.0, 700.0), 1.0, |_| {});
    let mut b = app("sv2", "0.3.1");
    b.sel = Sel::Game("rac1");
    b.tab = Tab::Play;
    shot_with("41_play_tab_open_saves_folder_small_860x580", b, vec2(860.0, 580.0), 1.0, |_| {});
    let mut c = app("sv3", "0.3.1");
    c.sel = Sel::Library;
    shot_with("42_library_card_open_saves_folder", c, vec2(1100.0, 700.0), 1.0, |_| {});
    let mut d = app("sv4", "0.3.1");
    d.sel = Sel::Library;
    shot_with("43_library_card_open_saves_folder_small_860x580", d, vec2(860.0, 580.0), 1.0, |_| {});
}

#[test]
fn s44_settings_levels_greyed_on_old_version() {
    let mut a = app("lv", "0.2.2");
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Settings;
    a.options.set("lighting", Some("enhanced"));
    a.options.set("water", Some("classic"));
    shot_with("44_settings_levels_greyed_on_0.2.2", a, vec2(1100.0, 1250.0), 1.0, |_| {});
}

// ------------------------------------------------------------------ manual backups

use rac_rs_launcher::backup;
use rac_rs_launcher::library::Library;
use rac_rs_launcher::ui::state::View;

#[test]
fn s45_backup_progress_card() {
    let mut a = app("bk4", "0.3.1");
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Play;
    // a bigger player folder so the numbers mean something
    std::fs::write(a.root.join("player/card.bin"), vec![1u8; 8 * 1024 * 1024]).unwrap();
    backup::TEST_HOLD.store(true, std::sync::atomic::Ordering::SeqCst);
    shot_with("45_backup_progress_card", a, vec2(1100.0, 700.0), 1.0, |h| {
        h.get_by_label("Back up this install").click();
        for _ in 0..200 {
            h.step();
            if h.state().app.view == View::Backup && h.state().app.backup.job.as_ref().map(|j| j.state.total_bytes.load(std::sync::atomic::Ordering::Relaxed) > 0).unwrap_or(false) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        // let the worker do a few files, then hold again for the picture
        backup::TEST_HOLD.store(false, std::sync::atomic::Ordering::SeqCst);
        std::thread::sleep(std::time::Duration::from_millis(5));
        backup::TEST_HOLD.store(true, std::sync::atomic::Ordering::SeqCst);
        h.step();
    });
    // release the worker (the picture is taken): it finishes or fails fast
    backup::TEST_HOLD.store(false, std::sync::atomic::Ordering::SeqCst);
}

fn backup_library(name: &str) -> App {
    let data = scratch(&format!("{name}_data"));
    let root = scratch(&format!("{name}_main"));
    install(&root, "0.4.0");
    let bdir = scratch(&format!("{name}_main (backup v0.3.1)"));
    install(&bdir, "0.3.1");
    let mut lib = Library::load(&data);
    let main_id = lib.add_existing(&root).unwrap().id;
    let g = rac_rs_launcher::games::game_by_id("rac1").unwrap();
    lib.add_backup(g, &bdir, "Ratchet & Clank v0.3.1 (backup)");
    lib.save().unwrap();
    let mut a = App::with_library(data.clone(), lib, Some(main_id));
    a.covers.dir = Some(data.join("_no_covers"));
    a
}

#[test]
fn s49_library_backup_card_and_delete_confirm() {
    let mut a = backup_library("bk5");
    a.sel = Sel::Library;
    shot_with("49_library_backup_card", a, vec2(1100.0, 700.0), 1.0, |_| {});
    let mut b = backup_library("bk6");
    b.sel = Sel::Library;
    shot_with("50_library_backup_delete_confirm_860x900", b, vec2(860.0, 900.0), 1.0, |h| {
        h.get_by_label("Delete backup").click();
    });
    // the backup install itself: its own note
    let mut c = backup_library("bk7");
    let bid = c.library.entries.iter().find(|e| e.backup).unwrap().id.clone();
    assert!(c.select_install(&bid));
    c.sel = Sel::Game("rac1");
    c.tab = Tab::Play;
    shot_with("51_play_tab_of_a_backup", c, vec2(1100.0, 700.0), 1.0, |_| {});
}
