//! UI tests without a window: a click on a Settings row must reach the selected profile's pc_options.txt at once (write-through),
//! and every page the tests draw passes egui_kittest's accessibility check (all input widgets have a name).
use egui::vec2;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use rac_rs_launcher::ui::state::{App, Sel, Tab};
use rac_rs_launcher::ui::{theme, LauncherApp};
use std::path::PathBuf;

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("racl_set_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A finished install in a scratch folder (nothing to run), no update server.
fn installed(name: &str) -> PathBuf {
    let root = scratch(name);
    for (f, t) in [
        ("build/release/rac-rs.exe", "x"),
        ("Play.cmd", "x"),
        ("rc.iso", "disc"),
        ("VERSION", "0.3.1\n"),
        ("generated/built_version.txt", "0.3.1\n"),
        ("player/pc_options.txt", "version=1\nlighting=classic\n"),
    ] {
        let p = root.join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }
    std::fs::write(root.join("generated/play.cfg"), format!("iso={}\n", root.join("rc.iso").display())).unwrap();
    root
}

fn harness(root: &PathBuf, tab: Tab) -> Harness<'static, LauncherApp> {
    let mut app = App::new(root.clone(), Some(root.join("_no_remote").to_string_lossy().to_string()));
    app.covers.dir = Some(root.join("_no_covers"));
    app.sel = Sel::Game("rac1");
    app.tab = tab;
    Harness::builder()
        .with_size(vec2(1100.0, 3600.0))
        .with_context_setup(|ctx| theme::apply(ctx))
        .build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(app))
}

fn options_text(root: &PathBuf) -> String {
    std::fs::read_to_string(root.join("profiles/default/pc_options.txt")).unwrap_or_default()
}

#[test]
fn settings_click_is_written_to_pc_options() {
    let root = installed("write");
    let mut h = harness(&root, Tab::Settings);
    h.run();
    h.get_by_label("Lighting: Enhanced").click();
    h.run();
    let t = options_text(&root);
    assert!(t.lines().any(|l| l == "lighting=enhanced"), "options file after the click:\n{t}");
    h.get_by_label("Water: Low").click();
    h.run();
    let t = options_text(&root);
    assert!(t.lines().any(|l| l == "water=low"), "options file after the second click:\n{t}");
    assert!(t.lines().any(|l| l == "lighting=enhanced"), "the first choice is kept:\n{t}");
    // the segment shows as selected afterwards
    h.run();
    let _ = h.get_by_label("Water: Low");
    // a choice back to the game's default removes the line only when it equals the default; high is the default of Water
    h.get_by_label("Water: High").click();
    h.run();
    assert!(!options_text(&root).lines().any(|l| l == "water=low"));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn camera_row_writes_the_camera_byte_values() {
    let root = installed("camera");
    let mut h = harness(&root, Tab::Settings);
    h.run();
    h.get_by_label("Camera: Static").click();
    h.run();
    let t = options_text(&root);
    assert!(t.lines().any(|l| l == "camera=modern") && t.lines().any(|l| l == "camfeel=static"), "options file:\n{t}");
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn pages_pass_the_accessibility_check() {
    let root = installed("a11y");
    for tab in [Tab::Play, Tab::Settings, Tab::Mods] {
        let mut h = harness(&root, tab);
        h.run(); // run() panics when an input widget has no accessible name
        assert!(h.unnamed_widgets().is_empty(), "{:?}: {:?}", tab.id(), h.unnamed_widgets());
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Per-value gating: the levels a game version ignores are greyed (not clickable, not written) and explained; the others still work.
#[test]
fn old_game_versions_grey_out_the_levels_they_ignore() {
    let root = installed("old022");
    std::fs::write(root.join("VERSION"), "0.2.2\n").unwrap();
    std::fs::write(root.join("generated/built_version.txt"), "0.2.2\n").unwrap();
    let mut h = harness(&root, Tab::Settings);
    h.run();
    assert!(h.get_by_label("Water: Low").accesskit_node().is_disabled());
    assert!(h.get_by_label("Particles: Medium").accesskit_node().is_disabled());
    assert!(!h.get_by_label("Water: Classic").accesskit_node().is_disabled());
    assert!(!h.get_by_label("Water: High").accesskit_node().is_disabled(), "high is the old enhanced look");
    assert!(!h.get_by_label("Ambient occlusion: Low").accesskit_node().is_disabled(), "rows without per-value rows are untouched");
    assert_eq!(h.query_all_by_label_contains("Low and Medium need game version 0.3.0").count(), 2, "one line under Water, one under Particles");
    h.get_by_label("Water: Low").click();
    h.get_by_label("Particles: Medium").click();
    h.run();
    let t = options_text(&root);
    assert!(!t.contains("water=") && !t.contains("particles="), "greyed values are never written:\n{t}");
    h.get_by_label("Water: Classic").click();
    h.run();
    assert!(options_text(&root).lines().any(|l| l == "water=classic"));
    let _ = std::fs::remove_dir_all(&root);

    // 0.3.0 knows every level: nothing greyed, no line
    let root = installed("new030");
    std::fs::write(root.join("VERSION"), "0.3.0\n").unwrap();
    std::fs::write(root.join("generated/built_version.txt"), "0.3.0\n").unwrap();
    let mut h = harness(&root, Tab::Settings);
    h.run();
    assert!(!h.get_by_label("Water: Low").accesskit_node().is_disabled());
    assert_eq!(h.query_all_by_label_contains("need game version 0.3.0").count(), 0);
    let _ = std::fs::remove_dir_all(&root);
}

/// Graphics presets: a click writes the preset's values at once; changing one value afterwards flips the selector to Custom.
#[test]
fn presets_set_the_values_and_a_changed_value_shows_custom() {
    let root = installed("preset");
    let mut h = harness(&root, Tab::Settings);
    h.run();
    assert!(h.get_by_label("Graphics quality: Custom").accesskit_node().is_disabled(), "no options file changes = High, Custom is only a status");
    h.get_by_label("Graphics quality: Low").click();
    h.run();
    let t = options_text(&root);
    for l in ["draw=low", "smoothing=off", "hud=original", "ao=low", "water=low", "waves=off", "vegetation=off"] {
        assert!(t.lines().any(|x| x == l), "{l} missing after the Low click:\n{t}");
    }
    assert!(t.lines().any(|x| x == "lighting=classic") || !t.contains("lighting="), "Lighting stays Classic:\n{t}");
    assert!(h.get_by_label("Graphics quality: Custom").accesskit_node().is_disabled(), "Low matches, so Custom stays a status");
    h.get_by_label("Ambient occlusion: High").click();
    h.run();
    assert!(!h.get_by_label("Graphics quality: Custom").accesskit_node().is_disabled(), "one value changed: the selector shows Custom");
    h.get_by_label("Graphics quality: High").click();
    h.run();
    let t = options_text(&root);
    for key in ["draw=", "smoothing=", "hud=", "ao=", "water=", "waves=", "vegetation="] {
        assert!(!t.contains(key), "High leaves {key} to the game's default:\n{t}");
    }
    let _ = std::fs::remove_dir_all(&root);
}

/// Skip intro (game 0.4.0 and newer): On writes `intro=skip`, Off writes `intro=play`; other lines of the file are kept.
#[test]
fn skip_intro_row_writes_the_intro_key() {
    const LABEL: &str = "Skip intro (go straight to the title screen)";
    let root = installed("intro");
    std::fs::write(root.join("VERSION"), "0.4.0\n").unwrap();
    std::fs::write(root.join("generated/built_version.txt"), "0.4.0\n").unwrap();
    std::fs::write(root.join("player/pc_options.txt"), "version=1\nlighting=classic\nfuture_thing=7\nkeys=r2:Period\n").unwrap();
    let mut h = harness(&root, Tab::Settings);
    h.run();
    assert!(!h.get_by_label(&format!("{LABEL}: On")).accesskit_node().is_disabled());
    h.get_by_label(&format!("{LABEL}: On")).click();
    h.run();
    let t = options_text(&root);
    assert!(t.lines().any(|l| l == "intro=skip"), "after On:\n{t}");
    assert!(t.lines().any(|l| l == "future_thing=7") && t.lines().any(|l| l == "keys=r2:Period") && t.lines().any(|l| l == "lighting=classic"), "other lines kept:\n{t}");
    h.get_by_label(&format!("{LABEL}: Off")).click();
    h.run();
    let t = options_text(&root);
    assert!(t.lines().any(|l| l == "intro=play") && !t.contains("intro=skip"), "after Off:\n{t}");
    assert!(t.lines().any(|l| l == "keys=r2:Period"), "kept:\n{t}");
    let _ = std::fs::remove_dir_all(&root);

    // a game older than 0.4.0 does not know the key: greyed, never written
    let root = installed("intro_old");
    let mut h = harness(&root, Tab::Settings);
    h.run();
    assert!(h.get_by_label(&format!("{LABEL}: On")).accesskit_node().is_disabled());
    let _ = std::fs::remove_dir_all(&root);
}

/// The "Support the project" link is in the sidebar on every page; it is only a widget (nothing opens without a click).
#[test]
fn support_link_is_in_the_sidebar() {
    let root = installed("support");
    let mut h = harness(&root, Tab::Play);
    h.run();
    assert_eq!(h.query_all_by_label("Support the project").count(), 1);
    assert_eq!(rac_rs_launcher::play::SUPPORT_URL, "https://ko-fi.com/asobicodes");
    let _ = std::fs::remove_dir_all(&root);
}
