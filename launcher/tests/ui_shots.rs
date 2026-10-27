//! Offscreen snapshots of every launcher page and state. PNGs go to `shots\` next to the crate folder (override: RAC_LAUNCHER_SHOTS).
//! No window is ever opened: egui_kittest renders into an offscreen wgpu texture.
use egui::vec2;
use egui_kittest::Harness;
use rac_rs_launcher::games::RAC1_DISCS;
use rac_rs_launcher::setup_parse::Progress;
use rac_rs_launcher::ui::page_mods::AddState;
use rac_rs_launcher::ui::state::{App, DiscState, Failure, RunKind, Sel, Tab, View};
use rac_rs_launcher::ui::{theme, LauncherApp};
use rac_rs_launcher::update::UpdateState;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn shots_dir() -> PathBuf {
    let d = std::env::var_os("RAC_LAUNCHER_SHOTS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../shots")));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("racl_shots_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, text: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

const CHANGELOG: &str = "# Changelog\n\n## v0.4.0\n- Faster start on slow PCs.\n- New launcher: install, play and mods in one window.\n- Fixed the water flicker in Pokitaru.\n\n## v0.3.1\n- Fixed a freeze at start-up on some AMD cards.\n";

/// A finished install in a scratch folder; `remote` = the version the fake update server offers.
fn installed_root(name: &str, local: &str, remote: &str) -> (PathBuf, String) {
    let root = scratch(name);
    write(&root.join("build/release/rac-rs.exe"), "x");
    write(&root.join("Play.cmd"), "x");
    write(&root.join("rc.iso"), "disc");
    write(&root.join("generated/play.cfg"), &format!("iso={}\n", root.join("rc.iso").display()));
    write(&root.join("generated/built_version.txt"), &format!("{local}\n"));
    write(&root.join("VERSION"), &format!("{local}\n"));
    write(&root.join("player/pc_options.txt"), "version=1\n");
    let base = root.join("_remote");
    write(&base.join("VERSION"), &format!("{remote}\n"));
    write(&base.join("CHANGELOG.md"), CHANGELOG);
    let b = base.to_string_lossy().to_string();
    (root, b)
}

fn make_app(root: PathBuf, base: Option<String>) -> App {
    let mut app = App::new(root.clone(), base);
    app.covers.dir = Some(root.join("_no_covers"));
    // wait for the (local) update check to finish so the screen is the same every run
    let t = Instant::now();
    while matches!(app.update_state(), UpdateState::Checking) && app.info.installed && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(30));
    }
    app.info.disc = Some(&RAC1_DISCS[0]);
    app
}

fn empty_app(name: &str) -> App {
    make_app(scratch(name), Some(scratch(&format!("{name}_remote")).to_string_lossy().to_string()))
}

fn demo_progress(update: bool, step: usize, pct: f64) -> Progress {
    let mut p = Progress::new(update);
    p.step = step;
    p.progress = pct;
    p.status = "Compiling level 7 of 19 (rec_l6)".into();
    p.log = (1..=40).map(|i| format!("   Compiling rec_sh_{i:02} v0.1.0 (D:\\RAC-RS\\oracle\\rec\\sh_{i:02})")).collect();
    p
}

/// wgpu instances created at the same moment on several test threads crash the Vulkan loader: one render at a time.
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn render(name: &str, size: (f32, f32), ppp: f32, mut app: App) {
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    app.cfg.width = size.0;
    app.cfg.height = size.1;
    let mut h = Harness::builder()
        .with_size(vec2(size.0, size.1))
        .with_pixels_per_point(ppp)
        .with_context_setup(|ctx| theme::apply(ctx))
        .wgpu()
        .build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(app));
    for _ in 0..4 {
        h.step();
    }
    let img = h.render().expect("offscreen render");
    let path = shots_dir().join(format!("{name}.png"));
    img.save(&path).unwrap();
    assert!(img.width() > 100 && path.exists());
}

fn shot(name: &str, app: App) {
    render(name, (1100.0, 700.0), 1.0, app);
}

fn select_tab(app: &mut App, t: Tab) {
    app.sel = Sel::Game("rac1");
    app.tab = t;
}

#[test]
fn s01_not_installed() {
    shot("01_not_installed", empty_app("s01"));
}

#[test]
fn s02_disc_checking() {
    let mut a = empty_app("s02");
    a.disc_text = r"D:\Games\Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso".into();
    a.disc_state = DiscState::Checking(42);
    shot("02_disc_checking", a);
}

#[test]
fn s03_disc_ok() {
    let mut a = empty_app("s03");
    a.disc_text = r"D:\Games\Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso".into();
    a.disc_state = DiscState::Ok(&RAC1_DISCS[0]);
    a.disc_path = Some(PathBuf::from(&a.disc_text));
    shot("03_disc_ok", a);
}

#[test]
fn s04_disc_bad() {
    let mut a = empty_app("s04");
    a.disc_text = r"D:\Games\Some other game.iso".into();
    a.disc_state = DiscState::Bad("This is not a supported Ratchet & Clank disc image. Use the PAL (Europe) or USA v1.00 disc.".into());
    shot("04_disc_bad", a);
}

#[test]
fn s05_installing() {
    let mut a = empty_app("s05");
    a.view = View::Run;
    a.run_kind = RunKind::Install;
    a.demo_run = Some((demo_progress(false, 2, 47.0), Instant::now() - Duration::from_secs(412)));
    shot("05_installing", a);
}

#[test]
fn s06_installing_details() {
    let mut a = empty_app("s06");
    a.view = View::Run;
    a.run_kind = RunKind::Install;
    a.show_details = true;
    a.demo_run = Some((demo_progress(false, 2, 47.0), Instant::now() - Duration::from_secs(412)));
    shot("06_installing_details", a);
}

#[test]
fn s07_install_failed() {
    let mut a = empty_app("s07");
    a.view = View::Failed;
    a.failure = Some(Failure {
        message: "The build stopped: there is not enough free disk space on drive D:. About 6 GB are needed.".into(),
        details: "error: no space left on device".into(),
        log: "   Compiling rec_sh_12 v0.1.0\nerror: no space left on device (os error 112)\nerror: could not compile `rec_sh_12`".into(),
        update: false,
    });
    shot("07_install_failed", a);
}

#[test]
fn s08_install_done() {
    let (root, base) = installed_root("s08", "0.3.1", "0.3.1");
    let mut a = make_app(root, Some(base));
    a.view = View::Done;
    a.run_kind = RunKind::Install;
    a.done_note = "A desktop shortcut was created.".into();
    shot("08_install_done", a);
}

#[test]
fn s09_installed() {
    let (root, base) = installed_root("s09", "0.3.1", "0.3.1");
    add_mods(&root);
    shot("09_installed", make_app(root, Some(base)));
}

#[test]
fn s10_update_banner() {
    let (root, base) = installed_root("s10", "0.3.1", "0.4.0");
    let mut a = make_app(root, Some(base));
    a.update_changes_open = true;
    shot("10_update_banner", a);
}

#[test]
fn s11_running() {
    let (root, base) = installed_root("s11", "0.3.1", "0.3.1");
    let mut a = make_app(root, Some(base));
    a.game_external = true;
    shot("11_running", a);
}

#[test]
fn s12_settings_classic() {
    let (root, base) = installed_root("s12", "0.3.1", "0.3.1");
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Settings);
    shot("12_settings_classic", a);
}

#[test]
fn s13_settings_enhanced() {
    let (root, base) = installed_root("s13", "0.3.1", "0.3.1");
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Settings);
    a.options.set("lighting", Some("enhanced"));
    a.options.set("water", Some("high"));
    a.options.set("camera", Some("modern"));
    shot("13_settings_enhanced", a);
}

fn add_mods(root: &Path) {
    let m = root.join("mods");
    let mk = |id: &str, name: &str, author: &str, desc: &str| {
        let json = format!(
            "{{\"format\":1,\"id\":\"{id}\",\"name\":\"{name}\",\"author\":\"{author}\",\"version\":\"1.2\",\"description\":\"{desc}\",\"game\":\"rac1\",\"kind\":\"textures\",\"serials\":[\"SCES-50916\"]}}"
        );
        write(&m.join(id).join("mod.json"), &json);
        write(&m.join(id).join("replacements/0123456789abcdef-00000001.png"), "x");
    };
    mk("hd-ratchet", "HD Ratchet and Clank", "Pixel Pete", "Sharper textures for Ratchet, Clank and the HUD.");
    mk("bright-veldin", "Bright Veldin", "Nina", "Warmer colours for the first planet.");
    mk("tiny-mod", "Tiny Mod", "", "");
    write(&m.join("active.txt"), "# enabled mods, top = highest priority\nhd-ratchet\nbright-veldin\n");
}

#[test]
fn s14_mods_list() {
    let (root, base) = installed_root("s14", "0.3.1", "0.3.1");
    add_mods(&root);
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Mods);
    a.reload_game_data();
    a.mods_ui.selected = Some("hd-ratchet".into());
    shot("14_mods_list", a);
}

#[test]
fn s15_mods_preview() {
    let (root, base) = installed_root("s15", "0.3.1", "0.3.1");
    add_mods(&root);
    let pack = scratch("s15_pack").join("Cool Texture Pack");
    for i in 0..12u64 {
        write(&pack.join(format!("SCES-50916/replacements/{:016x}-{:08x}.png", 0xABCDEF0000u64 + i, 0x1234)), "pixels");
    }
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Mods);
    a.reload_game_data();
    a.mods_inspect(&pack);
    // the pack is read on a worker thread: poll like the real frame does until the preview is there
    let t = Instant::now();
    while !matches!(a.mods_ui.add, AddState::Preview { .. }) && t.elapsed() < Duration::from_secs(10) {
        a.mods_poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(matches!(a.mods_ui.add, AddState::Preview { .. }), "preview did not come");
    shot("15_mods_preview", a);
}

#[test]
fn s16_mods_importing() {
    let (root, base) = installed_root("s16", "0.3.1", "0.3.1");
    add_mods(&root);
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Mods);
    a.reload_game_data();
    let st = Arc::new(rac_rs_launcher::mods::import::ImportState::default());
    st.files_total.store(340, Ordering::Relaxed);
    st.files_done.store(120, Ordering::Relaxed);
    st.bytes_total.store(900_000_000, Ordering::Relaxed);
    st.bytes_done.store(310_000_000, Ordering::Relaxed);
    *st.current.lock().unwrap() = "replacements/00aa11bb22cc33dd-0000ff01.png".into();
    a.mods_ui.add = AddState::Importing { state: st, name: "Cool Texture Pack".into() };
    shot("16_mods_importing", a);
}

#[test]
fn s45_mods_reading() {
    let (root, base) = installed_root("s45", "0.3.1", "0.3.1");
    add_mods(&root);
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Mods);
    a.reload_game_data();
    // a worker that is held back = the "Reading the pack..." state stays on screen
    let st = rac_rs_launcher::mods::import::start_inspect_held(scratch("s45_pack"));
    st.entries.store(12_800, Ordering::Relaxed);
    a.mods_ui.add = AddState::Reading { state: st, name: "SCUS-97199.zip".into() };
    shot("45_mods_reading", a);
}

#[test]
fn s46_mods_importing_big_pack() {
    let (root, base) = installed_root("s46", "0.3.1", "0.3.1");
    add_mods(&root);
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Mods);
    a.reload_game_data();
    let st = Arc::new(rac_rs_launcher::mods::import::ImportState::default());
    st.files_total.store(19_591, Ordering::Relaxed);
    st.files_done.store(8_214, Ordering::Relaxed);
    st.bytes_total.store(6_714_958_092, Ordering::Relaxed);
    st.bytes_done.store(2_870_000_000, Ordering::Relaxed);
    *st.current.lock().unwrap() = "LEVELS/NOVALIS/DISPLAY CASE ONLY GOLD WEAPONS /00aa11bb22cc33dd-0000ff01.png".into();
    a.mods_ui.add = AddState::Importing { state: st, name: "SCUS-97199".into() };
    shot("46_mods_importing_big_pack", a);
}

#[test]
fn s17_mods_error() {
    let (root, base) = installed_root("s17", "0.3.1", "0.3.1");
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Mods);
    a.mods_ui.add = AddState::Error("Extract the archive first: .rar files cannot be read directly. Drop the folder or a .zip instead.".into());
    shot("17_mods_error_empty", a);
}

#[test]
fn s18_coming_soon() {
    let (root, base) = installed_root("s18", "0.3.1", "0.3.1");
    let mut a = make_app(root, Some(base));
    a.sel = Sel::Game("rac2");
    shot("18_coming_soon", a);
}

#[test]
fn s19_launcher_page() {
    let (root, base) = installed_root("s19", "0.3.1", "0.3.1");
    let mut a = make_app(root, Some(base));
    a.sel = Sel::Launcher;
    shot("19_launcher_page", a);
}

#[test]
fn s20_scaled_and_small() {
    let (root, base) = installed_root("s20", "0.3.1", "0.4.0");
    let a = make_app(root, Some(base));
    render("20_installed_update_1p5x", (1100.0, 700.0), 1.5, a);
    let (root, base) = installed_root("s20b", "0.3.1", "0.4.0");
    let mut b = make_app(root, Some(base));
    select_tab(&mut b, Tab::Settings);
    render("21_settings_small_860x580", (860.0, 580.0), 1.0, b);
    let mut c = empty_app("s20c");
    c.view = View::Run;
    c.demo_run = Some((demo_progress(false, 2, 47.0), Instant::now() - Duration::from_secs(412)));
    c.show_details = true;
    render("22_installing_small_860x580_1p5x", (860.0, 580.0), 1.5, c);
}

#[test]
fn s21_settings_tall() {
    let (root, base) = installed_root("s21", "0.3.1", "0.3.1");
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Settings);
    a.options.set("lighting", Some("enhanced"));
    render("23_settings_full_height", (1100.0, 1900.0), 1.0, a);
}

#[test]
fn s47_mods_master_ticks() {
    let (root, base) = installed_root("s47", "0.4.0", "0.4.0");
    add_mods(&root);
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Mods);
    a.reload_game_data();
    a.mods_ui.store.set_master(rac_rs_launcher::mods::Master::Textures, false);
    shot("47_mods_master_ticks_textures_off", a);
}

#[test]
fn s48_settings_presets() {
    let (root, base) = installed_root("s48", "0.4.0", "0.4.0");
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Settings);
    a.choose_preset(rac_rs_launcher::ui::presets::Preset::Medium);
    shot("48_settings_preset_medium", a);
}

#[test]
fn s49_settings_presets_custom_on_0_2_2() {
    let (root, base) = installed_root("s49", "0.2.2", "0.2.2");
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Settings);
    a.choose_preset(rac_rs_launcher::ui::presets::Preset::Low);
    a.options.set("ao", Some("high"));
    shot("49_settings_preset_custom_old_game", a);
}

#[test]
fn s60_settings_skip_intro() {
    let (root, base) = installed_root("s60", "0.4.0", "0.4.0");
    write(&root.join("player/pc_options.txt"), "version=1\nintro=skip\nkeys=r2:Period\n");
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Settings);
    render("60_polish_settings_skip_intro", (1100.0, 3300.0), 1.0, a);
}

#[test]
fn s61_sidebar_support_link() {
    let (root, base) = installed_root("s61", "0.4.0", "0.4.0");
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Play);
    render("61_polish_play_support_link", (1100.0, 700.0), 1.0, a);
}

#[test]
fn s62_sidebar_support_link_small() {
    let (root, base) = installed_root("s62", "0.4.0", "0.4.0");
    let mut a = make_app(root, Some(base));
    select_tab(&mut a, Tab::Play);
    render("62_polish_play_support_link_small_860x580", (860.0, 580.0), 1.0, a);
}
