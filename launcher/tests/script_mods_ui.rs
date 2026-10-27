//! Script mods on the Mods page (stage E2): the list with the manifest's name / version / author, the on/off switch through active.txt and the
//! "Load mods" tick, the settings panel (presets, values, Reset to defaults) writing `mods\settings.json`, and the game-version gate.
//! The pictures at the end go to `shots\` like the other shot tests.
use egui::vec2;
use egui_kittest::kittest::Queryable;
use egui_kittest::Harness;
use rac_rs_launcher::library::Library;
use rac_rs_launcher::mods::parse_active;
use rac_rs_launcher::ui::state::{App, Sel, Tab};
use rac_rs_launcher::ui::{theme, LauncherApp};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const ARSENAL: &str = include_str!("data/arsenal-mastery.omni-mod.json");

/// A mod that has one setting of every type, for the pictures and the keybind test.
const KITCHEN: &str = r#"{ "format": 2, "id": "quick-strafe", "name": "Quick Strafe", "version": "1.1.0", "author": "Someone", "type": "script", "api": "1.3",
  "description": "A dodge on a key.", "games": { "rac1": ["pal", "ntsc"] }, "scripts": ["main.luau"],
  "settings": [
    { "key": "mode", "name": "Mode", "type": "choice", "default": "lock", "choices": [ { "value": "lock", "name": "Lock with weapons" }, { "value": "hold", "name": "Hold L2 or R2" } ] },
    { "key": "trace", "name": "Trace", "type": "bool", "default": false, "description": "Write what the mod does to the log." },
    { "key": "every", "name": "Log every", "type": "int", "default": 10, "min": 1, "max": 600 },
    { "key": "scale", "name": "Speed", "type": "float", "default": 1.0, "min": 0.5, "max": 3.0, "step": 0.1 },
    { "key": "greeting", "name": "Greeting", "type": "string", "default": "hi", "max_length": 40 },
    { "key": "dodge", "name": "Dodge", "type": "keybind", "default": "KeyF,MouseMiddle", "group": "play" },
    { "key": "jump", "name": "Jump", "type": "keybind", "default": "Space", "binds": 1, "group": "play" }
  ] }"#;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("racl_script_{}_{}", name, std::process::id()));
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
    write(&root.join("generated/play.cfg"), &format!("iso={}\n", root.join("rc.iso").display()));
}

fn texture_mod(root: &Path, id: &str, name: &str) {
    write(&root.join(format!("mods/{id}/mod.json")), &format!("{{\"format\":1,\"id\":\"{id}\",\"name\":\"{name}\",\"author\":\"A\",\"version\":\"1.0\",\"description\":\"d\",\"game\":\"rac1\",\"kind\":\"textures\",\"serials\":[\"SCES-50916\"]}}"));
    write(&root.join(format!("mods/{id}/replacements/0123456789abcdef-00000001.png")), "x");
}

struct Env {
    data: PathBuf,
    root: PathBuf,
}

fn app_with(name: &str, version: &str, mods: impl FnOnce(&Path)) -> (App, Env) {
    let data = scratch(&format!("{name}_data"));
    let root = scratch(&format!("{name}_root"));
    install(&root, version);
    mods(&root);
    let mut lib = Library::load(&data);
    let id = lib.add_existing(&root).unwrap().id;
    lib.save().unwrap();
    let mut app = App::with_library(data.clone(), lib, Some(id), Some(data.join("_no_remote").to_string_lossy().to_string()));
    app.covers.dir = Some(data.join("_no_covers"));
    app.reload_game_data();
    app.sel = Sel::Game("rac1");
    app.tab = Tab::Mods;
    (app, Env { data, root })
}

fn harness(app: App) -> Harness<'static, LauncherApp> {
    harness_high(app, 1300.0)
}

/// A tall window, so that a long settings panel is fully on screen (a click outside the window does not reach the button).
fn harness_high(app: App, height: f32) -> Harness<'static, LauncherApp> {
    Harness::builder().with_size(vec2(1100.0, height)).with_context_setup(|ctx| theme::apply(ctx)).build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(app))
}

fn cleanup(e: &Env) {
    let _ = std::fs::remove_dir_all(&e.data);
    let _ = std::fs::remove_dir_all(&e.root);
}

fn settings_json(root: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(root.join("mods/settings.json")).unwrap()).unwrap()
}

/// The page saves the on/off list in the profile's own file; it is copied to mods\active.txt at Play.
fn act_file(root: &Path) -> PathBuf {
    root.join("profiles/default/active.txt")
}

fn two_mods(root: &Path) {
    write(&root.join("mods/arsenal-mastery/omni-mod.json"), ARSENAL);
    write(&root.join("mods/quick-strafe/omni-mod.json"), KITCHEN);
    texture_mod(root, "hd", "HD Pack");
}

#[test]
fn script_mods_are_listed_with_their_manifest_and_switch_through_active_txt() {
    let (a, e) = app_with("list", "0.4.0", |r| {
        two_mods(r);
        write(&r.join("mods/broken-one/omni-mod.json"), "{ nope");
    });
    let mut h = harness(a);
    h.run();
    h.get_by_label("Arsenal Mastery");
    h.get_by_label("Use Arsenal Mastery").click();
    h.run();
    let act = std::fs::read_to_string(act_file(&e.root)).unwrap();
    assert_eq!(parse_active(&act), vec!["arsenal-mastery"], "the manifest id, not the folder, goes into active.txt: {act}");
    // a mod the game would refuse: listed with its problem, cannot be switched on
    assert!(h.query_by_label_contains("omni-mod.json has a problem").is_some() || h.query_by_label_contains("it is not valid JSON").is_some());
    h.get_by_label("Use broken-one").click();
    h.run();
    assert_eq!(parse_active(&std::fs::read_to_string(act_file(&e.root)).unwrap()), vec!["arsenal-mastery"]);
    // the Load mods tick holds script mods back without touching texture packs
    h.get_by_label("Use HD Pack").click();
    h.run();
    h.get_by_label("Load mods, master switch").click();
    h.run();
    let act = std::fs::read_to_string(act_file(&e.root)).unwrap();
    assert_eq!(parse_active(&act), vec!["hd"], "{act}");
    assert!(act.lines().any(|l| l == "# Load mods: off") && act.lines().any(|l| l == "# arsenal-mastery"), "{act}");
    h.get_by_label("Load mods, master switch").click();
    h.run();
    let act = std::fs::read_to_string(act_file(&e.root)).unwrap();
    assert_eq!(parse_active(&act), vec!["hd", "arsenal-mastery"], "{act}");
    cleanup(&e);
}

#[test]
fn the_settings_panel_writes_settings_json_and_keeps_what_it_does_not_know() {
    let (a, e) = app_with("panel", "0.4.0", |r| {
        two_mods(r);
        write(&r.join("mods/settings.json"), r#"{"format":1,"keepme":true,"mods":{"someone-else":{"x":1},"arsenal-mastery":{"unknown_key":"stay","perks":false}}}"#);
    });
    let mut h = harness_high(a, 2600.0);
    h.run();
    h.get_all_by_label("Mod settings").next().unwrap().click();
    h.run();
    // the stored value shows (perks off); the others are the declared defaults
    h.get_by_label("Extra projectiles and perks, switch");
    h.get_by_label("Enemy rebalance: Standard");
    // a preset
    h.get_by_label("Arsenal Mastery preset: Survival").click();
    h.run();
    let j = settings_json(&e.root);
    assert_eq!(j["keepme"], json!(true));
    assert_eq!(j["mods"]["someone-else"], json!({"x": 1}));
    let m = &j["mods"]["arsenal-mastery"];
    assert_eq!(m["rebalance"], json!("hard"));
    assert_eq!(m["ammo_mode"], json!("survival"));
    assert_eq!(m["perks"], json!(true), "the preset's value, not the old stored one");
    assert_eq!(m["unknown_key"], json!("stay"));
    // one switch
    h.get_by_label("Weapon power from levels, switch").click();
    h.run();
    assert_eq!(settings_json(&e.root)["mods"]["arsenal-mastery"]["level_power"], json!(false));
    // a custom mix lights no preset
    assert!(h.state().app.mods_ui.settings.mod_values("arsenal-mastery").get("level_power") == Some(&json!(false)));
    // Reset to defaults removes the declared keys only
    h.get_by_label("Reset to defaults").click();
    h.run();
    let j = settings_json(&e.root);
    assert_eq!(j["mods"]["arsenal-mastery"], json!({"unknown_key": "stay"}));
    assert_eq!(j["mods"]["someone-else"], json!({"x": 1}));
    assert_eq!(j["keepme"], json!(true));
    let bytes = std::fs::read(e.root.join("mods/settings.json")).unwrap();
    assert_ne!(&bytes[..3], &[0xEF, 0xBB, 0xBF]);
    // settings are independent of the on/off switch: the mod was never switched on
    assert!(!std::fs::read_to_string(act_file(&e.root)).unwrap_or_default().contains("arsenal-mastery"));
    cleanup(&e);
}

#[test]
fn every_control_type_is_drawn_and_a_keybind_listens_for_the_next_key() {
    let (a, e) = app_with("kitchen", "0.4.0", two_mods);
    let mut h = harness(a);
    h.run();
    // open Quick Strafe's panel (the second "Mod settings" button in the list: mods sort by name)
    let btns: Vec<_> = h.get_all_by_label("Mod settings").collect();
    assert_eq!(btns.len(), 2);
    btns[1].click();
    h.run();
    for l in ["Mode: Lock with weapons", "Trace, switch", "Dodge, input 1: F", "Dodge, input 2: Middle mouse button", "Jump, input 1: Space"] {
        h.get_by_label(l);
    }
    for l in ["Log every", "Speed", "Greeting"] {
        h.get_all_by_label(l).next().unwrap();
    }
    assert!(h.query_by_label_contains("Also used by").is_none());
    h.get_by_label("Trace, switch").click();
    h.run();
    h.get_by_label("Mode: Hold L2 or R2").click();
    h.run();
    // the Space key for Dodge makes it clash with Jump
    h.get_by_label("Dodge, input 1: F").click();
    h.step();
    h.key_press(egui::Key::Space);
    h.run();
    assert_eq!(settings_json(&e.root)["mods"]["quick-strafe"]["dodge"], json!("Space,MouseMiddle"));
    assert!(h.query_all_by_label_contains("Also used by").count() >= 1, "Dodge and Jump share Space");
    let m = settings_json(&e.root)["mods"]["quick-strafe"].clone();
    assert_eq!(m["trace"], json!(true));
    assert_eq!(m["mode"], json!("hold"));
    cleanup(&e);
}

#[test]
fn a_game_older_than_0_3_5_greys_the_mods_tick_with_the_reason() {
    let (a, e) = app_with("old", "0.3.1", two_mods);
    let mut h = harness(a);
    h.run();
    assert!(h.query_all_by_label_contains("Needs game version 0.4.0").count() >= 2, "the banner and both ticks say why");
    h.get_by_label("Load mods, master switch").click();
    h.run();
    assert!(!std::fs::read_to_string(act_file(&e.root)).unwrap_or_default().contains("# Load mods"), "a greyed tick does nothing");
    cleanup(&e);
}

// ------------------------------------------------------------------ pictures

static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn shots_dir() -> PathBuf {
    let d = std::env::var_os("RAC_LAUNCHER_SHOTS").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../shots")));
    let _ = std::fs::create_dir_all(&d);
    d
}

fn render(name: &str, size: (f32, f32), mut app: App) {
    let _gpu = GPU.lock().unwrap_or_else(|e| e.into_inner());
    app.cfg.width = size.0;
    app.cfg.height = size.1;
    let mut h = Harness::builder().with_size(vec2(size.0, size.1)).with_context_setup(|ctx| theme::apply(ctx)).wgpu().build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(app));
    for _ in 0..4 {
        h.step();
    }
    let img = h.render().expect("offscreen render");
    img.save(shots_dir().join(format!("{name}.png"))).unwrap();
}

#[test]
fn s53_mods_page_with_a_script_mod() {
    let (mut a, e) = app_with("shot1", "0.4.0", two_mods);
    write(&act_file(&e.root), "arsenal-mastery\nhd\n");
    a.reload_game_data();
    render("53_mods_script_and_texture_list", (1100.0, 700.0), a);
    cleanup(&e);
}

#[test]
fn s54_arsenal_mastery_settings_open() {
    let (mut a, e) = app_with("shot2", "0.4.0", two_mods);
    write(&act_file(&e.root), "arsenal-mastery\nhd\n");
    write(&e.root.join("mods/settings.json"), r#"{"format":1,"mods":{"arsenal-mastery":{"rebalance":"hard","ammo_mode":"survival","xp_rate":1.5}}}"#);
    a.reload_game_data();
    a.mods_ui.settings_open = Some("arsenal-mastery".into());
    render("54_mods_arsenal_settings_open", (1100.0, 1500.0), a);
    cleanup(&e);
}

#[test]
fn s55_keybinds_text_and_numbers_narrow_window() {
    let (mut a, e) = app_with("shot3", "0.4.0", two_mods);
    write(&act_file(&e.root), "quick-strafe\n");
    write(&e.root.join("mods/settings.json"), r#"{"format":1,"mods":{"quick-strafe":{"dodge":"Space,MouseMiddle","scale":1.8,"greeting":"hello there"}}}"#);
    a.reload_game_data();
    a.mods_ui.settings_open = Some("quick-strafe".into());
    render("55_mods_script_all_control_types_860", (860.0, 1250.0), a);
    cleanup(&e);
}
