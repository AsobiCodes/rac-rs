//! The launcher's state and the flows behind the buttons (install, repair, update, play, disc check, shortcuts, restart after a launcher update).
//! The pages (`page_*.rs`) only draw and call these methods.
use crate::cfg::LauncherCfg;
use crate::discs::{self, DiscCheck, DiscResult};
use crate::engine::{self, Engine, EngineOpts};
use crate::games::{self, GameDef, InstallInfo};
use crate::library::Library;
use crate::profiles::{Profile, Profiles};
use crate::pcopts::Options;
use crate::platform;
use crate::play;
use crate::selfupdate;
use crate::setup_parse::Progress;
use crate::ui::cover::Covers;
use crate::ui::page_mods::ModsUi;
use crate::ui::state_lib::{AddDialog, LibUi, ProfileUi};
use crate::update::{self, UpdateState};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::mpsc::{channel, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const LAUNCHER_EXE: &str = "RAC-RS Launcher.exe";
pub const SHORTCUT_NAME: &str = "Ratchet & Clank (rac-rs)";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tab {
    Play,
    Settings,
    Mods,
}

impl Tab {
    pub fn id(self) -> &'static str {
        match self {
            Tab::Play => "play",
            Tab::Settings => "settings",
            Tab::Mods => "mods",
        }
    }
    pub fn from_id(s: &str) -> Tab {
        match s {
            "settings" => Tab::Settings,
            "mods" => Tab::Mods,
            _ => Tab::Play,
        }
    }
}

/// What the sidebar has selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sel {
    Game(&'static str),
    /// The list of installs (add, remove, open).
    Library,
    Launcher,
}

/// What the Play tab of the game shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum View {
    /// Not installed: pick the disc. Installed: the Play screen.
    Home,
    /// Repair / rebuild: pick the disc again.
    Repair,
    /// The worker runs.
    Run,
    /// The backup before an update is being copied.
    Backup,
    Failed,
    Done,
}

#[derive(Clone, Debug, PartialEq)]
pub enum RunKind {
    Install,
    Repair,
    Update { to: String, finish: bool },
}

#[derive(Clone, Debug, PartialEq)]
pub enum DiscState {
    Empty,
    Checking(u32),
    Ok(&'static games::DiscDef),
    Bad(String),
}

pub struct Failure {
    pub message: String,
    pub details: String,
    pub log: String,
    pub update: bool,
}

/// What a file dialog on its own thread was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Purpose {
    Iso,
    ModZip,
    ModFolder,
    /// "Add existing install": the install's folder.
    AddInstall,
    /// "New install": the folder to put the game in.
    NewInstall,
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

pub struct Args {
    pub root: Option<PathBuf>,
    pub update_base: Option<String>,
    pub restarted: bool,
}

fn set_env_arg(key: &str, v: Option<String>) {
    if let Some(v) = v {
        std::env::set_var(key, v);
    }
}

pub fn parse_args(args: impl IntoIterator<Item = String>) -> Args {
    let mut a = Args { root: None, update_base: std::env::var("RAC_LAUNCHER_UPDATE_BASE").ok().filter(|s| !s.is_empty()), restarted: false };
    let mut it = args.into_iter();
    while let Some(x) = it.next() {
        match x.as_str() {
            "--root" => a.root = it.next().map(PathBuf::from),
            "--update-base" => a.update_base = it.next(),
            "--restarted" => a.restarted = true,
            // the two release sources for tests (folders with latest.json); the environment is inherited by a restarted launcher
            "--launcher-releases" => set_env_arg("RAC_LAUNCHER_RELEASES", it.next()),
            "--game-releases" => set_env_arg("RAC_GAME_RELEASES", it.next()),
            _ => {}
        }
    }
    a
}

pub struct App {
    /// The folder of the selected install (empty = no install in the library).
    pub root: PathBuf,
    /// Where launcher.cfg and library.json live (see `library::data_dir`).
    pub data_dir: PathBuf,
    pub library: Library,
    /// Library id of the selected install.
    pub cur_id: Option<String>,
    pub profiles: Profiles,
    /// Id of the selected profile of this install ("" = not set up yet).
    pub profile_id: String,
    pub profile_list: Vec<Profile>,
    pub profile_ui: ProfileUi,
    pub lib_ui: LibUi,
    /// "Back up your current version first" (see state_backup.rs).
    pub backup: crate::ui::state_backup::BackupUi,
    pub cfg: LauncherCfg,
    pub sel: Sel,
    pub tab: Tab,
    pub view: View,
    pub info: InstallInfo,
    // pick the disc
    pub disc_text: String,
    pub disc_check: Option<Arc<DiscCheck>>,
    pub disc_state: DiscState,
    pub disc_path: Option<PathBuf>,
    pub disc_edit_at: Option<Instant>,
    pub shortcut_wanted: bool,
    pub picker: Option<(Purpose, Receiver<Option<PathBuf>>)>,
    // the worker
    pub engine: Option<Engine>,
    pub run_kind: RunKind,
    pub show_details: bool,
    pub confirm_cancel: bool,
    pub failure: Option<Failure>,
    pub done_note: String,
    pub last_update: Option<(String, bool)>,
    // updates
    pub update: Arc<Mutex<UpdateState>>,
    pub update_base: Option<String>,
    pub update_changes_open: bool,
    // the game
    pub game_child: Option<Child>,
    pub game_external: bool,
    pub last_ext_check: Instant,
    pub play_error: Option<String>,
    // settings and mods
    pub options: Options,
    pub confirm_reset: bool,
    /// The graphics preset the player clicked last (shown while the options still match it; see `presets`).
    pub preset_picked: Option<crate::ui::presets::Preset>,
    pub mods_ui: ModsUi,
    // shell
    pub covers: Covers,
    pub toast: Option<Toast>,
    pub confirm_quit: bool,
    pub quit_now: bool,
    pub shortcut_dir: Option<PathBuf>,
    pub restart_exe: Option<PathBuf>,
    pub restarted: bool,
    /// Downloads and the launcher's own update (see state_net.rs).
    pub net: crate::ui::state_net::Net,
    /// Started with `--root`: a restart passes it on.
    pub forced_root: bool,
    /// Snapshot tests only: a made-up worker progress to draw when no engine runs.
    pub demo_run: Option<(Progress, Instant)>,
}

impl App {
    /// "Use this install": `root` is the one install, and the launcher's own files (launcher.cfg, library.json) live there too.
    /// Used by `--root` and by the tests; the folder is put in the library whatever it holds.
    pub fn new(root: PathBuf, update_base: Option<String>) -> App {
        let mut library = Library::load(&root);
        let entry = library.add(games::game_by_id("rac1").unwrap_or(&games::GAMES[0]), &root);
        let _ = library.save();
        let mut app = Self::with_library(root, library, Some(entry.id), update_base);
        app.forced_root = true;
        app
    }

    /// The normal start: the library of `data_dir`, with `select` (or the last used, or the first install) opened.
    pub fn with_library(data_dir: PathBuf, library: Library, select: Option<String>, update_base: Option<String>) -> App {
        let cfg = LauncherCfg::load(&data_dir);
        let want = select.or_else(|| Some(cfg.install.clone()).filter(|s| !s.is_empty())).filter(|id| library.get(id).is_some()).or_else(|| library.entries.first().map(|e| e.id.clone()));
        let game = games::game_by_id(&cfg.game).filter(|g| g.visible).unwrap_or(&games::GAMES[0]);
        let mut app = App {
            sel: if want.is_some() { Sel::Game(game.id) } else { Sel::Library },
            tab: Tab::from_id(&cfg.tab),
            view: View::Home,
            disc_text: String::new(),
            disc_check: None,
            disc_state: DiscState::Empty,
            disc_path: None,
            disc_edit_at: None,
            shortcut_wanted: true,
            picker: None,
            engine: None,
            run_kind: RunKind::Install,
            show_details: false,
            confirm_cancel: false,
            failure: None,
            done_note: String::new(),
            last_update: None,
            update: Arc::new(Mutex::new(UpdateState::Checking)),
            update_base,
            update_changes_open: false,
            game_child: None,
            game_external: false,
            last_ext_check: Instant::now() - Duration::from_secs(10),
            play_error: None,
            options: Options::new(),
            confirm_reset: false,
            preset_picked: None,
            mods_ui: ModsUi::default(),
            covers: Covers::default(),
            toast: None,
            confirm_quit: false,
            quit_now: false,
            shortcut_dir: std::env::var_os("RAC_LAUNCHER_SHORTCUT_DIR").map(PathBuf::from),
            restart_exe: None,
            restarted: false,
            net: crate::ui::state_net::Net::from_env(),
            forced_root: false,
            demo_run: None,
            info: InstallInfo::default(),
            root: PathBuf::new(),
            profiles: Profiles::new(Path::new("")),
            profile_id: String::new(),
            profile_list: Vec::new(),
            profile_ui: ProfileUi::default(),
            lib_ui: LibUi::default(),
            backup: Default::default(),
            cur_id: None,
            data_dir,
            library,
            cfg,
        };
        app.cleanup_old_exe();
        if let Some(id) = want {
            app.enter_install(&id);
        }
        app
    }

    pub fn game(&self) -> &'static GameDef {
        match self.sel {
            Sel::Game(id) => games::game_by_id(id).unwrap_or(&games::GAMES[0]),
            Sel::Library | Sel::Launcher => games::game_by_id(&self.cfg.game).unwrap_or(&games::GAMES[0]),
        }
    }

    pub fn rac1(&self) -> &'static GameDef {
        games::game_by_id("rac1").unwrap_or(&games::GAMES[0])
    }

    pub fn has_install(&self) -> bool {
        self.cur_id.is_some() && !self.root.as_os_str().is_empty()
    }

    pub fn refresh_info(&mut self) {
        self.info = if self.has_install() { games::detect(&self.root, self.rac1()) } else { InstallInfo::default() };
    }

    /// The settings and mods of the SELECTED PROFILE (after a start, a finished run, a tab change, a profile change).
    pub fn reload_game_data(&mut self) {
        if !self.has_install() {
            return;
        }
        let g = self.rac1();
        if self.info.installed && self.profile_id.is_empty() {
            self.reload_profiles();
        }
        if self.profile_id.is_empty() {
            self.options = Options::load(&g.options_path(&self.root));
            self.mods_ui.reload(&g.mods_dir(&self.root), None);
        } else {
            self.options = Options::load(&self.profiles.options_file(&self.profile_id));
            let active = self.profiles.active_file(&self.profile_id);
            self.mods_ui.reload(&g.mods_dir(&self.root), Some(&active));
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast { text: text.into(), error, until: Instant::now() + Duration::from_secs(if error { 8 } else { 3 }) });
    }

    pub fn disc_serial(&self) -> Option<&'static str> {
        self.info.disc.map(|d| d.serial)
    }

    pub fn busy(&self) -> bool {
        self.engine.as_ref().map(|e| e.running()).unwrap_or(false) || self.backup.job.is_some()
    }

    // ------------------------------------------------------------------ the old exe left by a launcher update

    /// At start: remove `RAC-RS Launcher.exe.old` NEXT TO THE RUNNING EXE (not in the data folder), retrying for ~15 s on a thread.
    fn cleanup_old_exe(&self) {
        selfupdate::cleanup_old(self.net.self_exe.clone());
    }

    // ------------------------------------------------------------------ updates

    pub fn check_updates(&mut self) {
        if !self.info.installed {
            return;
        }
        self.backup_forget_plan();
        if self.is_backup() {
            // a backup stays on its version for good: no check, no banner
            if let Ok(mut g) = self.update.lock() {
                *g = UpdateState::UpToDate(self.info.local_version.clone());
            }
            return;
        }
        if let Ok(mut g) = self.update.lock() {
            *g = UpdateState::Checking;
        }
        update::check_async(self.info.local_version.clone(), self.info.built_version.clone(), self.update_base.clone(), self.root.clone(), self.update.clone());
    }

    pub fn update_state(&self) -> UpdateState {
        self.update.lock().map(|g| g.clone()).unwrap_or(UpdateState::Offline)
    }

    // ------------------------------------------------------------------ the disc

    pub fn set_disc_path(&mut self, path: &str) {
        self.disc_text = path.to_string();
        self.disc_edit_at = None;
        self.start_disc_check();
    }

    /// The text field changed: check 0.7 s after the last key.
    pub fn disc_text_changed(&mut self) {
        if let Some(c) = self.disc_check.take() {
            c.cancel();
        }
        self.disc_path = None;
        self.disc_state = DiscState::Empty;
        self.disc_edit_at = Some(Instant::now() + Duration::from_millis(700));
    }

    pub fn start_disc_check(&mut self) {
        if let Some(c) = self.disc_check.take() {
            c.cancel();
        }
        self.disc_path = None;
        let t = self.disc_text.trim().trim_matches('"').to_string();
        if t.is_empty() {
            self.disc_state = DiscState::Empty;
            return;
        }
        self.disc_state = DiscState::Checking(0);
        self.disc_check = Some(discs::start(self.rac1(), PathBuf::from(t), true));
    }

    fn poll_disc(&mut self) {
        if let Some(at) = self.disc_edit_at {
            if Instant::now() >= at {
                self.disc_edit_at = None;
                self.start_disc_check();
            }
        }
        let Some(c) = self.disc_check.clone() else { return };
        match c.result() {
            DiscResult::Running => self.disc_state = DiscState::Checking(c.percent()),
            DiscResult::Ok(d) => {
                self.disc_state = DiscState::Ok(d);
                self.disc_path = Some(c.path.clone());
                self.disc_check = None;
            }
            DiscResult::Bad(m) => {
                self.disc_state = DiscState::Bad(m);
                self.disc_check = None;
            }
            DiscResult::Cancelled => self.disc_check = None,
        }
    }

    pub fn browse(&mut self, purpose: Purpose) {
        if self.picker.is_some() {
            return;
        }
        let start_dir: Option<PathBuf> = match purpose {
            Purpose::Iso => Some(PathBuf::from(&self.cfg.last_iso_dir)).filter(|p| !self.cfg.last_iso_dir.is_empty() && p.is_dir()),
            _ => platform::known_folder(platform::Known::Documents),
        };
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut d = rfd::FileDialog::new();
            if let Some(s) = start_dir {
                d = d.set_directory(s);
            }
            let r = match purpose {
                Purpose::Iso => d.set_title("Pick your Ratchet & Clank (Europe or USA, v1.00) disc image").add_filter("Disc image (*.iso)", &["iso"]).add_filter("All files", &["*"]).pick_file(),
                Purpose::ModZip => d.set_title("Pick a mod or texture pack (.zip)").add_filter("Zip file (*.zip)", &["zip"]).pick_file(),
                Purpose::ModFolder => d.set_title("Pick the folder of a mod or texture pack").pick_folder(),
                Purpose::AddInstall => d.set_title("Pick the folder of your RAC-RS game (it holds VERSION and setup.ps1)").pick_folder(),
                Purpose::NewInstall => d.set_title("Pick the folder for the game").pick_folder(),
            };
            let _ = tx.send(r);
        });
        self.picker = Some((purpose, rx));
    }

    fn poll_picker(&mut self) {
        let Some((purpose, rx)) = &self.picker else { return };
        let purpose = *purpose;
        let Ok(res) = rx.try_recv() else { return };
        self.picker = None;
        let Some(p) = res else { return };
        match purpose {
            Purpose::Iso => {
                if let Some(d) = p.parent() {
                    self.cfg.last_iso_dir = d.to_string_lossy().to_string();
                }
                self.set_disc_path(&p.to_string_lossy());
            }
            Purpose::ModZip | Purpose::ModFolder => self.mods_inspect(&p),
            Purpose::AddInstall => self.lib_ui.dialog = Some(AddDialog::Existing { text: p.to_string_lossy().to_string() }),
            Purpose::NewInstall => self.lib_ui.dialog = Some(AddDialog::New { text: p.to_string_lossy().to_string() }),
        }
    }

    // ------------------------------------------------------------------ the worker (install / repair / update)

    pub fn goto_repair(&mut self) {
        self.view = View::Repair;
        match self.info.iso.clone().filter(|_| self.info.iso_exists) {
            Some(p) => self.set_disc_path(&p.to_string_lossy()),
            None => self.set_disc_path(""),
        }
    }

    /// Install (not installed yet) or Repair (view Repair), with the checked disc.
    pub fn start_install(&mut self) {
        let Some(iso) = self.disc_path.clone() else { return };
        let repair = self.view == View::Repair;
        self.run_kind = if repair { RunKind::Repair } else { RunKind::Install };
        let opts = EngineOpts { root: self.root.clone(), iso, update: None, regenerate: repair, test_update_base: None, extra: engine::env_extra_args() };
        self.start_engine(opts);
    }

    /// Why an update cannot start now (None = it can): the game runs, or the disc image is gone. Reads the install again.
    pub fn update_blocker(&mut self) -> Option<String> {
        self.refresh_info();
        if self.game_running() {
            return Some("The game is still running.\n\nClose it, then click Try again.".to_string());
        }
        match self.info.iso.clone().filter(|_| self.info.iso_exists) {
            Some(_) => None,
            None => {
                let p = self.info.iso.as_ref().map(|p| p.display().to_string()).unwrap_or_default();
                Some(format!("Your disc image was not found at:\n{p}\n\nPut it back where it was, or use Repair / rebuild (Settings, Advanced) to pick it again."))
            }
        }
    }

    pub fn start_update(&mut self, to: &str, finish: bool) {
        self.last_update = Some((to.to_string(), finish));
        self.run_kind = RunKind::Update { to: to.to_string(), finish };
        // a backup is a finished copy of an old version: nothing ever updates it
        let msg = if self.is_backup() { Some("A backup is never updated. Open your main install to update.".to_string()) } else { self.update_blocker() };
        if let Some(m) = msg {
            self.failure = Some(Failure { message: m, details: String::new(), log: String::new(), update: true });
            self.engine = None;
            self.view = View::Failed;
            return;
        }
        let iso = self.info.iso.clone().unwrap_or_default();
        let opts = EngineOpts {
            root: self.root.clone(),
            iso,
            update: Some((to.to_string(), finish)),
            regenerate: false,
            test_update_base: self.update_base.clone(),
            extra: engine::env_extra_args(),
        };
        self.start_engine(opts);
    }

    fn start_engine(&mut self, opts: EngineOpts) {
        self.show_details = false;
        self.confirm_cancel = false;
        self.failure = None;
        match crate::installer::backend().start(&opts) {
            Ok(e) => {
                self.engine = Some(e);
                self.view = View::Run;
            }
            Err(e) => {
                self.engine = None;
                self.failure = Some(Failure { message: format!("The setup could not be started.\n\n{e}"), details: String::new(), log: String::new(), update: opts.update.is_some() });
                self.view = View::Failed;
            }
        }
    }

    pub fn cancel_run(&mut self) {
        if self.backup.job.is_some() {
            self.stop_backup();
            return;
        }
        if let Some(e) = self.engine.as_mut() {
            e.cancel();
        }
        self.engine = None;
        self.confirm_cancel = false;
        self.view = View::Home;
        self.refresh_info();
    }

    pub fn poll_engine(&mut self) {
        let Some(e) = self.engine.as_mut() else { return };
        e.poll();
        let Some(code) = e.done else { return };
        let progress: Progress = e.progress.clone();
        self.engine = None;
        if code == 0 {
            self.finish_ok(&progress);
        } else {
            self.failure = Some(Failure { message: progress.error_text(), details: progress.details_text(&platform::os_text()), log: progress.log.join("\n"), update: progress.update });
            self.view = View::Failed;
        }
    }

    fn finish_ok(&mut self, progress: &Progress) {
        self.refresh_info();
        self.reload_game_data();
        self.done_note = String::new();
        if let RunKind::Update { to, .. } = self.run_kind.clone() {
            self.done_note = format!("Updated to v{to}. Your saves and settings were kept.");
            if let Some(e) = self.backup.made.as_deref().and_then(|id| self.library.get(id)) {
                self.done_note.push_str(&format!(" Your old version is kept as \"{}\" in the Library.", e.name));
            }
            if !self.backup.left_out.is_empty() {
                self.done_note.push_str(&format!(" The backup does not hold the texture pack(s) {} (this drive cannot share files).", self.backup.left_out.join(", ")));
            }
        } else if self.shortcut_wanted && self.run_kind == RunKind::Install {
            self.done_note = self.make_shortcut_note();
        } else if self.run_kind == RunKind::Repair {
            self.done_note = "The game was rebuilt from your disc. Your saves and settings were kept.".to_string();
        }
        let _ = progress;
        self.view = View::Done;
        self.check_updates();
    }

    fn make_shortcut_note(&mut self) -> String {
        match self.create_shortcut() {
            Ok(p) => {
                let desk = platform::known_folder(platform::Known::Desktop);
                if self.shortcut_dir.is_none() && p.parent().map(|d| Some(d.to_path_buf()) == desk).unwrap_or(false) {
                    format!("The shortcut \"{SHORTCUT_NAME}\" is on your desktop. It opens this launcher.")
                } else {
                    format!("A shortcut was created here: {}", p.display())
                }
            }
            Err(e) => format!("The desktop shortcut could not be made: {e}"),
        }
    }

    /// A desktop shortcut that opens the launcher (the launcher exe next to the game; the running exe when it is not there yet).
    pub fn create_shortcut(&mut self) -> Result<PathBuf, String> {
        let target = {
            let p = self.root.join(LAUNCHER_EXE);
            if p.is_file() {
                p
            } else {
                std::env::current_exe().map_err(|e| e.to_string())?
            }
        };
        let dir = self.shortcut_dir.clone().or_else(|| platform::known_folder(platform::Known::Desktop)).ok_or("The desktop folder was not found.")?;
        let ico = self.root.join("rac-rs.ico");
        let icon = ico.is_file().then_some(ico);
        platform::create_shortcut(&dir, SHORTCUT_NAME, &target, &self.root, icon.as_deref())
    }

    // ------------------------------------------------------------------ the game

    pub fn game_running(&self) -> bool {
        self.game_child.is_some() || self.game_external
    }

    /// Play: the selected profile's settings and mods list are written for the game, then the game starts.
    pub fn play(&mut self, ctx: &egui::Context) {
        self.play_error = None;
        let g = self.rac1();
        if let Err(m) = play::check_ready(&self.root, g) {
            self.play_error = Some(m);
            return;
        }
        if !self.profile_id.is_empty() {
            if let Err(m) = self.profiles.swap_in(g, &self.profile_id) {
                self.play_error = Some(m);
                return;
            }
            self.profile_list = self.profiles.list();
        }
        match play::start(&self.root, g) {
            Ok(c) => {
                self.game_child = Some(c);
                if self.cfg.minimize_while_playing {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }
            }
            Err(m) => {
                self.profiles.copy_back(g);
                self.play_error = Some(m);
            }
        }
    }

    /// The game closed (started here or found running when the launcher opened): its options go back into the profile it used.
    fn after_game_exit(&mut self) {
        if let Some(id) = self.profiles.copy_back(self.rac1()) {
            if id != self.profile_id && !self.profile_id.is_empty() {
                self.toast(format!("The game's settings were saved in the profile \"{id}\"."), false);
            }
        }
        self.reload_game_data();
    }

    fn poll_game(&mut self, ctx: &egui::Context) {
        if let Some(c) = self.game_child.as_mut() {
            if let Ok(Some(_)) = c.try_wait() {
                self.game_child = None;
                if self.cfg.minimize_while_playing {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                }
                self.after_game_exit();
            }
        } else if self.info.installed && self.last_ext_check.elapsed() > Duration::from_secs(2) {
            self.last_ext_check = Instant::now();
            let was = self.game_external;
            self.game_external = platform::exe_in_use(&self.rac1().exe_path(&self.root));
            if was && !self.game_external {
                self.after_game_exit();
            }
        }
    }

    // ------------------------------------------------------------------ the frame

    /// Everything that happens between frames; returns true while something is running (the caller keeps repainting).
    pub fn tick(&mut self, ctx: &egui::Context) -> bool {
        self.poll_picker();
        self.poll_disc();
        self.poll_backup();
        self.poll_engine();
        self.poll_game(ctx);
        self.mods_poll();
        let net_busy = self.poll_net();
        if let Some(t) = &self.toast {
            if Instant::now() > t.until {
                self.toast = None;
            }
        }
        let active = self.engine.is_some()
            || self.backup.job.is_some()
            || self.disc_check.is_some()
            || self.disc_edit_at.is_some()
            || self.picker.is_some()
            || self.game_child.is_some()
            || self.toast.is_some()
            || self.mods_ui.busy()
            || net_busy
            || matches!(self.update_state(), UpdateState::Checking)
            || self.info.installed;
        if let Some(exe) = self.restart_exe.take() {
            self.do_restart(&exe);
        }
        active
    }

    pub fn save_cfg(&mut self) {
        self.cfg.game = match self.sel {
            Sel::Game(id) => id.to_string(),
            Sel::Library | Sel::Launcher => self.cfg.game.clone(),
        };
        self.cfg.tab = self.tab.id().to_string();
        self.cfg.install = self.cur_id.clone().unwrap_or_default();
        self.cfg.store(&self.data_dir);
    }

    pub fn root_dir(&self) -> &Path {
        &self.root
    }

    /// Settings and mods cannot be edited while the game runs (it reads them at start and writes its options back at exit).
    pub fn read_only(&self) -> bool {
        self.game_running()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_are_read() {
        let a = parse_args(["--root".to_string(), "D:\\x".to_string(), "--restarted".to_string(), "--update-base".to_string(), "E:\\b".to_string()]);
        assert_eq!(a.root.as_deref(), Some(Path::new("D:\\x")));
        assert!(a.restarted);
        assert_eq!(a.update_base.as_deref(), Some("E:\\b"));
    }

    #[test]
    fn tab_ids_round_trip() {
        for t in [Tab::Play, Tab::Settings, Tab::Mods] {
            assert_eq!(Tab::from_id(t.id()), t);
        }
    }
}
