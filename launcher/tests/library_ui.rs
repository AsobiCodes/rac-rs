//! UI-flow tests of the library and the profiles, without a window and without a network:
//! select / remove / new install, marker recovery after a crash, the copy-back when the game closes, the profile picker clicks,
//! and the Mods switch writing the selected profile's own list.
use egui::vec2;
use egui_kittest::kittest::{NodeT, Queryable};
use egui_kittest::Harness;
use rac_rs_launcher::games::game_by_id;
use rac_rs_launcher::library::Library;
use rac_rs_launcher::releases::{self, ReleaseSource};
use rac_rs_launcher::ui::state::{App, Sel, Tab};
use rac_rs_launcher::ui::{theme, LauncherApp};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("racl_libui_{}_{}", name, std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn write(p: &Path, t: &str) {
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, t).unwrap();
}

/// A finished (built, disc known) install of `version` in `root`; the "game exe" is a dummy file.
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

struct Env {
    data: PathBuf,
    roots: Vec<PathBuf>,
    ids: Vec<String>,
}

/// An App with the normal start-up (library in `data`) that lists one install per version, the first one open.
fn lib_app(name: &str, versions: &[&str]) -> (App, Env) {
    let data = scratch(&format!("{name}_data"));
    let mut lib = Library::load(&data);
    let mut roots = Vec::new();
    let mut ids = Vec::new();
    for (i, v) in versions.iter().enumerate() {
        let root = scratch(&format!("{name}_i{i}"));
        install(&root, v);
        ids.push(lib.add_existing(&root).unwrap().id);
        roots.push(root);
    }
    lib.save().unwrap();
    let mut app = App::with_library(data.clone(), lib, ids.first().cloned(), Some(data.join("_no_remote").to_string_lossy().to_string()));
    app.covers.dir = Some(data.join("_no_covers"));
    (app, Env { data, roots, ids })
}

fn harness(app: App) -> Harness<'static, LauncherApp> {
    Harness::builder().with_size(vec2(1100.0, 1000.0)).with_context_setup(|ctx| theme::apply(ctx)).build_ui_state(|ui, la: &mut LauncherApp| la.draw(ui), LauncherApp::new(app))
}

fn cleanup(e: &Env) {
    let _ = std::fs::remove_dir_all(&e.data);
    for r in &e.roots {
        let _ = std::fs::remove_dir_all(r);
    }
}

fn saved_ids(data: &Path) -> Vec<String> {
    Library::load(data).entries.into_iter().map(|e| e.id).collect()
}

// ------------------------------------------------------------------ select

#[test]
fn select_install_opens_the_other_install() {
    let (mut a, e) = lib_app("select", &["0.4.0", "0.3.1"]);
    assert_eq!(a.cur_id.as_deref(), Some(e.ids[0].as_str()));
    assert_eq!(a.info.local_version, "0.4.0");
    assert!(a.select_install(&e.ids[1]));
    assert_eq!(a.cur_id.as_deref(), Some(e.ids[1].as_str()));
    assert_eq!(a.root, e.roots[1]);
    assert_eq!(a.info.local_version, "0.3.1");
    assert_eq!(a.sel, Sel::Game("rac1"));
    assert!(!a.profile_id.is_empty(), "the new install got its Default profile");
    assert!(e.roots[1].join("profiles").is_dir());
    // an id that is not in the list changes nothing
    assert!(!a.select_install("nope"));
    assert_eq!(a.cur_id.as_deref(), Some(e.ids[1].as_str()));
    cleanup(&e);
}

#[test]
fn select_is_refused_while_the_game_runs() {
    let (mut a, e) = lib_app("select_busy", &["0.4.0", "0.3.1"]);
    a.game_external = true;
    assert!(!a.select_install(&e.ids[1]));
    assert_eq!(a.cur_id.as_deref(), Some(e.ids[0].as_str()), "the open install did not change");
    assert!(a.toast.as_ref().map(|t| t.error).unwrap_or(false), "the player is told why");
    cleanup(&e);
}

#[test]
fn clicking_open_on_a_library_card_switches_install() {
    let (mut a, e) = lib_app("select_click", &["0.4.0", "0.3.1"]);
    a.sel = Sel::Library;
    let mut h = harness(a);
    h.run();
    // the open install's card says "Go to game", the other one "Open"
    h.get_by_label("Open").click();
    h.run();
    assert_eq!(h.state().app.cur_id.as_deref(), Some(e.ids[1].as_str()));
    assert_eq!(h.state().app.sel, Sel::Game("rac1"));
    cleanup(&e);
}

// ------------------------------------------------------------------ remove

#[test]
fn remove_install_keeps_the_files_and_opens_the_next_one() {
    let (mut a, e) = lib_app("remove", &["0.4.0", "0.3.1"]);
    a.remove_install(&e.ids[0]);
    assert_eq!(a.library.entries.len(), 1);
    assert_eq!(a.cur_id.as_deref(), Some(e.ids[1].as_str()), "the next install is open now");
    assert_eq!(saved_ids(&e.data), vec![e.ids[1].clone()], "library.json was saved");
    assert!(e.roots[0].join("player/pc_options.txt").is_file() && e.roots[0].join("VERSION").is_file(), "the removed install's files stay");
    // removing the last one leaves the Library page with nothing open
    a.remove_install(&e.ids[1]);
    assert!(a.library.entries.is_empty() && !a.has_install());
    assert_eq!(a.sel, Sel::Library);
    assert!(saved_ids(&e.data).is_empty());
    assert!(e.roots[1].join("VERSION").is_file());
    // and the empty Library page still draws, with accessible names
    let mut h = harness(a);
    h.run();
    cleanup(&e);
}

#[test]
fn remove_is_refused_for_the_install_that_is_running() {
    let (mut a, e) = lib_app("remove_busy", &["0.4.0", "0.3.1"]);
    a.game_external = true;
    a.remove_install(&e.ids[0]);
    assert_eq!(a.library.entries.len(), 2, "the running install stays in the list");
    a.remove_install(&e.ids[1]);
    assert_eq!(a.library.entries.len(), 1, "another install can go while this one runs");
    cleanup(&e);
}

#[test]
fn remove_from_list_needs_a_second_click() {
    let (mut a, e) = lib_app("remove_click", &["0.4.0", "0.3.1"]);
    a.sel = Sel::Library;
    let mut h = harness(a);
    h.run();
    h.get_all_by_label("Remove from list").next().unwrap().click();
    h.run();
    assert_eq!(h.state().app.library.entries.len(), 2, "the first click only asks");
    h.get_by_label("Yes, remove from list").click();
    h.run();
    assert_eq!(h.state().app.library.entries.len(), 1);
    assert_eq!(saved_ids(&e.data).len(), 1);
    cleanup(&e);
}

// ------------------------------------------------------------------ new install

#[test]
fn new_install_input_checks() {
    let (mut a, e) = lib_app("new_bad", &["0.4.0"]);
    assert!(a.new_install("").unwrap_err().contains("Choose a folder"));
    assert!(a.new_install("   ").unwrap_err().contains("Choose a folder"));
    let file = e.data.join("a_file.txt");
    write(&file, "x");
    assert!(a.new_install(&file.to_string_lossy()).unwrap_err().contains("file, not a folder"));
    // a folder with something in it that is not a game: refused before anything is downloaded
    let busy = e.data.join("my_documents");
    write(&busy.join("notes.txt"), "mine");
    a.net.game_src = ReleaseSource::Folder(e.data.join("_no_releases"));
    let r = a.new_install(&busy.to_string_lossy());
    assert!(r.unwrap_err().contains("not empty"));
    assert!(!a.downloading(), "nothing was started");
    assert_eq!(std::fs::read_to_string(busy.join("notes.txt")).unwrap(), "mine");
    assert_eq!(a.library.entries.len(), 1);
    cleanup(&e);
}

#[test]
fn new_install_in_a_folder_that_is_already_listed_opens_it() {
    let (mut a, e) = lib_app("new_dup", &["0.4.0", "0.3.1"]);
    // typed with quotes, as pasted from Explorer
    let r = a.new_install(&format!("\"{}\"", e.roots[1].display())).unwrap();
    assert!(r.contains("already in your list"), "{r}");
    assert_eq!(a.cur_id.as_deref(), Some(e.ids[1].as_str()));
    assert_eq!(a.library.entries.len(), 2, "no second entry");
    cleanup(&e);
}

#[test]
fn new_install_in_an_unpacked_release_tree_is_listed_and_waits_for_the_disc() {
    let (mut a, e) = lib_app("new_tree", &["0.4.0"]);
    let tree = e.data.join("unpacked");
    write(&tree.join("VERSION"), "0.3.1\n");
    write(&tree.join("setup.ps1"), "# s");
    let r = a.new_install(&tree.to_string_lossy()).unwrap();
    assert!(r.contains("Pick your disc"), "{r}");
    assert_eq!(a.library.entries.len(), 2);
    assert_eq!(a.root, tree, "the new install is open");
    assert!(!a.info.installed, "not set up yet: the disc comes next");
    assert_eq!(saved_ids(&e.data).len(), 2);
    assert!(!a.downloading(), "an unpacked tree is not downloaded again");
    // a tree that is built already says so
    let built = e.data.join("built");
    install(&built, "0.3.1");
    let r = a.new_install(&built.to_string_lossy()).unwrap();
    assert!(r.contains("already holds a set-up game"), "{r}");
    assert_eq!(a.library.entries.len(), 3);
    cleanup(&e);
}

#[test]
fn new_install_downloads_into_an_empty_folder_from_a_test_source() {
    let (mut a, e) = lib_app("new_dl", &["0.4.0"]);
    let rel = e.data.join("rel");
    let mut zip = std::io::Cursor::new(Vec::new());
    {
        use std::io::Write;
        let mut w = zip::ZipWriter::new(&mut zip);
        let o = zip::write::SimpleFileOptions::default();
        for (n, b) in [("rac-rs-9.9.9/VERSION", &b"9.9.9\n"[..]), ("rac-rs-9.9.9/setup.ps1", b"# s"), ("rac-rs-9.9.9/Play.cmd", b"x")] {
            w.start_file(n, o).unwrap();
            w.write_all(b).unwrap();
        }
        w.finish().unwrap();
    }
    releases::write_test_release(&rel, "v9.9.9", "", &[("rac-rs-9.9.9.zip", zip.get_ref())], &|_, hex| Some(format!("sha256:{hex}")));
    a.net.game_src = ReleaseSource::Folder(rel);
    let dest = e.data.join("games").join("rac");
    assert!(a.new_install(&dest.to_string_lossy()).is_ok());
    assert!(a.downloading());
    for _ in 0..300 {
        a.poll_net();
        if !a.downloading() {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!a.downloading() && a.net.dl_error.is_none(), "{:?}", a.net.dl_error);
    assert_eq!(a.info.local_version, "9.9.9");
    assert!(a.library.find_folder(&dest).is_some());
    assert!(!a.library.entries.iter().find(|x| x.path() == dest).unwrap().pinned, "the newest version is not pinned");
    cleanup(&e);
}

// ------------------------------------------------------------------ marker recovery and copy-back

fn profile_options(root: &Path, id: &str) -> String {
    std::fs::read_to_string(root.join("profiles").join(id).join("pc_options.txt")).unwrap_or_default()
}

#[test]
fn a_crash_marker_is_recovered_at_the_next_start() {
    let (a, e) = lib_app("marker", &["0.4.0"]);
    let g = game_by_id("rac1").unwrap();
    let id = a.profile_id.clone();
    assert_eq!(id, "default");
    // Play: the marker is written, the profile goes live; the game changes its options; the launcher dies before the game closes
    a.profiles.swap_in(g, &id).unwrap();
    assert_eq!(a.profiles.pending_marker().as_deref(), Some("default"));
    write(&e.roots[0].join("player/pc_options.txt"), "version=1\nlighting=enhanced\nwater=low\n");
    drop(a);
    // next start: the exe is not running, so the live file is copied back into the profile and the marker is gone
    let lib = Library::load(&e.data);
    let b = App::with_library(e.data.clone(), lib, None, Some(e.data.join("_no_remote").to_string_lossy().to_string()));
    assert!(!b.game_external);
    assert!(b.profiles.pending_marker().is_none(), "marker removed");
    let t = profile_options(&e.roots[0], "default");
    assert!(t.contains("water=low") && t.contains("lighting=enhanced"), "profile after recovery:\n{t}");
    assert!(b.options.to_text().contains("water=low"), "the page shows the recovered settings");
    cleanup(&e);
}

#[cfg(windows)]
#[test]
fn a_marker_stays_while_the_game_is_still_running() {
    use std::os::windows::fs::OpenOptionsExt;
    let (a, e) = lib_app("marker_busy", &["0.4.0"]);
    let g = game_by_id("rac1").unwrap();
    a.profiles.swap_in(g, "default").unwrap();
    write(&e.roots[0].join("player/pc_options.txt"), "version=1\nwater=medium\n");
    drop(a);
    // a handle that denies sharing = what a running exe looks like to platform::exe_in_use
    let _lock = std::fs::OpenOptions::new().read(true).share_mode(0).open(e.roots[0].join("build/release/rac-rs.exe")).unwrap();
    let lib = Library::load(&e.data);
    let b = App::with_library(e.data.clone(), lib, None, None);
    assert!(b.game_external, "a game started by another launcher is noticed");
    assert!(b.profiles.pending_marker().is_some(), "not copied back while it runs");
    assert!(!profile_options(&e.roots[0], "default").contains("water=medium"));
    drop(_lock);
    cleanup(&e);
}

#[test]
fn the_game_closing_copies_its_options_back_into_the_profile() {
    let (mut a, e) = lib_app("copyback", &["0.4.0"]);
    let g = game_by_id("rac1").unwrap();
    a.profiles.swap_in(g, "default").unwrap();
    // the game ran and wrote its options; it is gone now, but the launcher still believes it runs
    write(&e.roots[0].join("player/pc_options.txt"), "version=1\nlighting=enhanced\nparticles=medium\n");
    a.game_external = true;
    a.last_ext_check = Instant::now() - Duration::from_secs(10);
    let ctx = egui::Context::default();
    a.tick(&ctx);
    assert!(!a.game_external, "the exit was seen");
    assert!(a.profiles.pending_marker().is_none());
    assert!(profile_options(&e.roots[0], "default").contains("particles=medium"));
    assert!(a.options.to_text().contains("particles=medium"), "the Settings page shows what the game saved");
    cleanup(&e);
}

// ------------------------------------------------------------------ profile picker

fn play_harness(name: &str) -> (Harness<'static, LauncherApp>, Env) {
    let (mut a, e) = lib_app(name, &["0.4.0"]);
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Play;
    (harness(a), e)
}

/// Opens the picker's list when it is closed (the button toggles it).
fn open_picker(h: &mut Harness<'static, LauncherApp>) {
    h.run();
    if !h.state().app.profile_ui.open {
        let name = h.state().app.profile_name();
        h.get_by_label(&format!("Profile: {name}")).click();
        h.run();
    }
    assert!(h.state().app.profile_ui.open);
}

/// The one text field of the picker's form (its "Profile name" is a hint text, not a label).
fn name_field<'a>(h: &'a Harness<'static, LauncherApp>) -> egui_kittest::Node<'a> {
    h.get_by_role(egui::accesskit::Role::TextInput)
}

#[test]
fn profile_picker_new_select_rename_delete_by_clicks() {
    let (mut h, e) = play_harness("picker");
    h.run();
    // open the list, make a profile
    open_picker(&mut h);
    h.get_by_label("New").click();
    h.run();
    name_field(&h).type_text("Steam Deck");
    h.run();
    h.get_by_label("Create").click();
    h.run();
    let st = h.state();
    assert_eq!(st.app.profile_list.len(), 2, "{:?}", st.app.profile_ui.error);
    assert_eq!(st.app.profile_name(), "Steam Deck", "the new profile is selected");
    let new_id = st.app.profile_id.clone();
    assert!(e.roots[0].join("profiles").join(&new_id).join("profile.json").is_file());
    assert_eq!(st.app.profiles.current().as_deref(), Some(new_id.as_str()), "current.txt follows");
    // pick Default again from the open list
    open_picker(&mut h);
    h.get_by_label("Profile Default").click();
    h.run();
    assert_eq!(h.state().app.profile_id, "default");
    assert!(!h.state().app.profile_ui.open, "the list closes after a pick");
    // rename the open profile (Default)
    open_picker(&mut h);
    h.get_by_label("Rename").click();
    h.run();
    name_field(&h).focus();
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    name_field(&h).type_text("Main");
    h.run();
    h.get_by_label("Rename").click();
    h.run();
    assert_eq!(h.state().app.profile_name(), "Main", "{:?}", h.state().app.profile_ui.error);
    // delete the other profile: select it, then Delete + confirm
    open_picker(&mut h);
    h.get_by_label("Profile Steam Deck").click();
    h.run();
    open_picker(&mut h);
    h.get_by_label("Delete").click();
    h.run();
    h.get_by_label("Yes, delete it").click();
    h.run();
    let st = h.state();
    assert_eq!(st.app.profile_list.len(), 1, "{:?}", st.app.profile_ui.error);
    assert!(!e.roots[0].join("profiles").join(&new_id).exists(), "the deleted profile's folder is gone");
    assert_eq!(st.app.profile_name(), "Main");
    cleanup(&e);
}

#[test]
fn profile_picker_refuses_an_empty_name_and_keeps_the_form_open() {
    let (mut h, e) = play_harness("picker_bad");
    h.run();
    h.get_by_label("Profile: Default").click();
    h.run();
    h.get_by_label("New").click();
    h.run();
    h.get_by_label("Create").click();
    h.run();
    let st = h.state();
    assert!(st.app.profile_ui.error.is_some(), "the reason is shown");
    assert!(st.app.profile_ui.edit.is_some(), "the form stays open");
    assert_eq!(st.app.profile_list.len(), 1);
    cleanup(&e);
}

#[test]
fn profile_picker_is_locked_while_the_game_runs() {
    let (mut a, e) = lib_app("picker_busy", &["0.4.0"]);
    a.profiles.create("Second").unwrap();
    a.profile_list = a.profiles.list();
    let second = a.profile_list.iter().find(|p| p.meta.name == "Second").unwrap().id.clone();
    a.game_external = true;
    a.select_profile(&second);
    assert_eq!(a.profile_id, "default", "no profile change during a run");
    cleanup(&e);
}

// ------------------------------------------------------------------ mods + profile

#[test]
fn the_mods_switch_writes_the_selected_profiles_own_list() {
    let (mut a, e) = lib_app("mods", &["0.4.0"]);
    let root = e.roots[0].clone();
    write(&root.join("mods/hd/mod.json"), "{\"format\":1,\"id\":\"hd\",\"name\":\"HD Pack\",\"author\":\"A\",\"version\":\"1.0\",\"description\":\"d\",\"game\":\"rac1\",\"kind\":\"textures\",\"serials\":[\"SCES-50916\"]}");
    write(&root.join("mods/hd/replacements/0123456789abcdef-00000001.png"), "x");
    a.profiles.create("Other").unwrap();
    a.profile_list = a.profiles.list();
    a.reload_game_data();
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Mods;
    let mut h = harness(a);
    h.run();
    h.get_by_label("Use HD Pack").click();
    h.run();
    let own = std::fs::read_to_string(root.join("profiles/default/active.txt")).unwrap_or_default();
    assert!(own.lines().any(|l| l.trim() == "hd"), "the Default profile lists the mod:\n{own}");
    assert!(!root.join("mods/active.txt").exists(), "the game's own list is only written at Play (swap_in)");
    let other_id = h.state().app.profile_list.iter().find(|p| p.meta.name == "Other").unwrap().id.clone();
    let other = std::fs::read_to_string(root.join("profiles").join(&other_id).join("active.txt")).unwrap_or_default();
    assert!(!other.lines().any(|l| l.trim() == "hd"), "another profile is not touched:\n{other}");
    cleanup(&e);
}

fn texture_mod(root: &Path, id: &str, name: &str) {
    write(&root.join(format!("mods/{id}/mod.json")), &format!("{{\"format\":1,\"id\":\"{id}\",\"name\":\"{name}\",\"author\":\"A\",\"version\":\"1.0\",\"description\":\"d\",\"game\":\"rac1\",\"kind\":\"textures\",\"serials\":[\"SCES-50916\"]}}"));
    write(&root.join(format!("mods/{id}/replacements/0123456789abcdef-00000001.png")), "x");
}

#[test]
fn the_master_ticks_are_kept_per_profile_and_reach_active_txt_as_comment_lines() {
    use rac_rs_launcher::mods::{parse_active, Master};
    let (mut a, e) = lib_app("master", &["0.4.0"]);
    let root = e.roots[0].clone();
    texture_mod(&root, "hd", "HD Pack");
    texture_mod(&root, "cel", "Cel Pack");
    a.profiles.create("Other").unwrap();
    a.profile_list = a.profiles.list();
    a.reload_game_data();
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Mods;
    let mut h = harness(a);
    h.run();
    h.get_by_label("Use HD Pack").click();
    h.run();
    h.get_by_label("Use Cel Pack").click();
    h.run();
    let file = |id: &str| std::fs::read_to_string(root.join("profiles").join(id).join("active.txt")).unwrap_or_default();
    assert_eq!(parse_active(&file("default")), vec!["cel", "hd"]);
    assert!(!file("default").contains("Load texture packs"), "a tick that was never touched leaves no line");

    // Load texture packs OFF: both packs become `# id` lines, the per-pack switches stay on
    h.get_by_label("Load texture packs, master switch").click();
    h.run();
    let own = file("default");
    assert!(own.lines().any(|l| l == "# Load texture packs: off") && own.lines().any(|l| l == "# cel") && own.lines().any(|l| l == "# hd"), "{own}");
    assert!(parse_active(&own).is_empty(), "the game sees no pack: {own}");
    {
        let st = &h.state().app.mods_ui.store;
        assert!(st.is_enabled("hd") && st.is_enabled("cel") && !st.is_loaded("hd") && !st.master_on(Master::Textures));
    }
    // the game's file made at Play comes from the profile as it is
    let def = game_by_id("rac1").unwrap();
    h.state().app.profiles.swap_in(def, "default").unwrap();
    assert_eq!(std::fs::read_to_string(root.join("mods/active.txt")).unwrap(), own);

    // another profile has its own ticks
    let other_id = h.state().app.profile_list.iter().find(|p| p.meta.name == "Other").unwrap().id.clone();
    h.state_mut().app.select_profile(&other_id);
    h.run();
    assert!(h.state().app.mods_ui.store.master_on(Master::Textures), "the Other profile never switched it off");
    assert!(!file(&other_id).contains("# Load texture packs"));
    h.state_mut().app.select_profile("default");
    h.run();
    assert!(!h.state().app.mods_ui.store.master_on(Master::Textures), "the Default profile remembers its tick");

    // a per-pack switch still works while the tick is off, and is remembered
    h.get_by_label("Use HD Pack").click();
    h.run();
    assert_eq!(file("default").lines().filter(|l| *l == "# cel").count(), 1);
    assert!(!file("default").lines().any(|l| l.ends_with("hd") && l.starts_with('#')), "hd is off per pack: {}", file("default"));

    // ON: the pack that is still switched on is back as a plain id
    h.get_by_label("Load texture packs, master switch").click();
    h.run();
    let back = file("default");
    assert_eq!(parse_active(&back), vec!["cel"], "{back}");
    assert!(!back.contains("Load texture packs"), "{back}");
    // nothing of the other kind exists: its tick is greyed
    assert!(h.get_by_label("Load mods, master switch").accesskit_node().is_disabled());
    cleanup(&e);
}

#[test]
fn a_game_version_without_texture_packs_greys_the_texture_tick() {
    let (mut a, e) = lib_app("master_old", &["0.3.1"]);
    texture_mod(&e.roots[0], "hd", "HD Pack");
    a.reload_game_data();
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Mods;
    let mut h = harness(a);
    h.run();
    assert!(h.get_by_label("Load texture packs, master switch").accesskit_node().is_disabled());
    assert!(h.query_all_by_label_contains("Needs game version 0.4.0 or newer").count() >= 1);
    cleanup(&e);
}

// ------------------------------------------------------------------ adding a big pack never blocks the window

fn read_state_is(a: &App, want: &str) -> bool {
    use rac_rs_launcher::ui::page_mods::AddState;
    matches!((&a.mods_ui.add, want), (AddState::Reading { .. }, "reading") | (AddState::Preview { .. }, "preview") | (AddState::Importing { .. }, "importing") | (AddState::None, "none"))
}

/// Steps the window and the poll the way the real frame does, until the state is the wanted one.
fn pump_until(h: &mut Harness<'static, LauncherApp>, want: &str) {
    let t = Instant::now();
    while !read_state_is(&h.state().app, want) {
        assert!(t.elapsed() < Duration::from_secs(20), "never reached {want}");
        h.state_mut().app.mods_poll();
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn adding_a_pack_reads_and_copies_on_workers_and_the_window_keeps_answering() {
    use rac_rs_launcher::mods::import;
    use rac_rs_launcher::ui::page_mods::AddState;
    let (mut a, e) = lib_app("modasync", &["0.4.0"]);
    let root = e.roots[0].clone();
    let pack = e.data.join("Big Pack");
    for i in 0..600u64 {
        write(&pack.join(format!("SCUS-97199/replacements/LEVELS/L{}/{:016x}-00000007.png", i % 7, 0xaa00 + i)), "png");
    }
    a.sel = Sel::Game("rac1");
    a.tab = Tab::Mods;

    // 1. the reading worker is held back: the window shows "Reading the pack..." and still answers (Cancel works at once)
    let held = import::start_inspect_held(pack.clone());
    a.mods_ui.add = AddState::Reading { state: held.clone(), name: "Big Pack".into() };
    let mut h = harness(a);
    h.run_steps(2);
    h.get_by_label("Reading the pack...");
    for _ in 0..10 {
        h.state_mut().app.mods_poll();
        h.run_steps(1);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(read_state_is(&h.state().app, "reading"), "still reading while the worker is held: the frame did not wait for it");
    assert!(h.state().app.mods_ui.busy(), "a second pack waits");
    h.state_mut().app.mods_inspect(&pack);
    assert!(read_state_is(&h.state().app, "reading"), "a pack dropped while reading is refused, the first one stays");
    held.release();
    pump_until(&mut h, "preview");
    h.run_steps(2);
    h.get_by_label("Add mod");

    // 2. the real entry point: mods_inspect returns at once with the Reading state, the preview comes later
    h.state_mut().app.mods_ui.add = AddState::None;
    h.state_mut().app.mods_inspect(&pack);
    assert!(read_state_is(&h.state().app, "reading"), "mods_inspect only starts the worker");
    pump_until(&mut h, "preview");
    h.run_steps(2);
    h.get_by_label("Add mod").click();
    h.run_steps(2);
    // 3. Importing (or already finished on a fast disk): then the mod is listed and switched on, without the UI thread walking the files
    pump_until(&mut h, "none");
    h.run_steps(2);
    let a = &h.state().app;
    assert_eq!(a.mods_ui.store.entries.len(), 1);
    let id = a.mods_ui.store.entries[0].id.clone();
    assert!(a.mods_ui.store.is_enabled(&id), "switched on after the import");
    assert_eq!(std::fs::read_dir(root.join("mods").join(&id).join("textures/LEVELS")).unwrap().count(), 7);
    cleanup(&e);
}
