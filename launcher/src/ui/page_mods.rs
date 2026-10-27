//! The Mods tab: list of installed mods (switch on/off, priority, details, remove), adding a mod from a .zip or folder or PCSX2's textures
//! folder (with a preview and progress).
use super::page_mods_script::{self, Changes, PanelState};
use super::state::{App, Purpose};
use super::theme::{self, Icon};
use crate::gating;
use crate::mods::detect::Layout;
use crate::mods::import::{self, Candidate, ImportState, ReadState, Source};
use crate::mods::settings::SettingsFile;
use crate::mods::{ModEntry, ModStore};
use crate::platform;
use egui::{RichText, ScrollArea, Ui};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;

#[derive(Default)]
pub enum AddState {
    #[default]
    None,
    /// PCSX2's own textures folders for our discs.
    Pcsx2(Vec<(String, Source)>),
    /// A dropped or picked pack is being looked at on a worker thread (a 6 GB .zip takes a moment to open).
    Reading { state: Arc<ReadState>, name: String },
    Preview { cand: Box<Candidate>, name: String },
    Importing { state: Arc<ImportState>, name: String },
    Error(String),
}

#[derive(Default)]
pub struct ModsUi {
    pub store: ModStore,
    pub selected: Option<String>,
    pub confirm_remove: Option<String>,
    pub add: AddState,
    /// `mods\settings.json`: the values of the script mods' settings.
    pub settings: SettingsFile,
    /// The script mod whose settings panel is open.
    pub settings_open: Option<String>,
    pub panel: PanelState,
}

impl ModsUi {
    /// `active` = the active.txt to use (a profile's); None = the one in the mods folder.
    pub fn reload(&mut self, dir: &Path, active: Option<&Path>) {
        self.store = match active {
            Some(a) => ModStore::load_with(dir, a),
            None => ModStore::load(dir),
        };
        self.settings = SettingsFile::load(&dir.join("settings.json"));
        if let Some(s) = &self.selected {
            if self.store.get(s).is_none() {
                self.selected = None;
            }
        }
        if let Some(s) = &self.settings_open {
            if self.store.get(s).and_then(|e| e.script.as_ref()).is_none() {
                self.settings_open = None;
            }
        }
    }

    /// A worker thread is reading or copying a pack (the window keeps repainting; a second pack waits).
    pub fn busy(&self) -> bool {
        matches!(self.add, AddState::Importing { .. } | AddState::Reading { .. })
    }
}

fn layout_text(l: &Layout) -> &'static str {
    match l {
        Layout::ModJson { .. } => "A mod with its own description",
        Layout::Pcsx2 { .. } => "A PCSX2 texture pack",
        Layout::Unknown => "Unknown",
    }
}

fn kind_text(kind: &str) -> &str {
    match kind {
        "textures" => "Texture pack",
        "script" => "Script mod",
        "native" => "Native mod",
        other => other,
    }
}

impl App {
    /// The selected profile's active.txt (None = the one in the mods folder).
    fn mods_ui_active_file(&self) -> Option<PathBuf> {
        (!self.profile_id.is_empty()).then(|| self.profiles.active_file(&self.profile_id))
    }

    fn mods_dir(&self) -> PathBuf {
        self.rac1().mods_dir(&self.root)
    }

    /// A dropped or picked file/folder: look at it (on a worker thread; `mods_poll` shows the preview when it is done).
    pub fn mods_inspect(&mut self, p: &Path) {
        if self.mods_ui.busy() {
            self.toast("Please wait until the pack that is being read or added is finished.", true);
            return;
        }
        let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| p.display().to_string());
        self.mods_ui.add = AddState::Reading { state: import::start_inspect(p.to_path_buf()), name };
    }

    pub fn mods_poll(&mut self) {
        self.mods_poll_reading();
        self.mods_poll_importing();
    }

    fn mods_poll_reading(&mut self) {
        let AddState::Reading { state, .. } = &self.mods_ui.add else { return };
        let Some(r) = state.take_result() else { return };
        self.mods_ui.add = match r {
            Ok(c) => {
                let name = import::plan_info(&c, |id| self.mods_ui.store.get(id).is_some(), None).name;
                AddState::Preview { cand: Box::new(c), name }
            }
            Err(m) if m.to_lowercase().contains("cancel") => AddState::None,
            Err(m) => AddState::Error(m),
        };
    }

    fn mods_poll_importing(&mut self) {
        let AddState::Importing { state, name } = &self.mods_ui.add else { return };
        let Some(res) = state.result() else { return };
        let name = name.clone();
        let skipped = state.skipped.load(Ordering::Relaxed);
        self.mods_ui.add = AddState::None;
        let dir = self.mods_dir();
        let active = self.mods_ui_active_file();
        self.mods_ui.reload(&dir, active.as_deref());
        match res {
            Ok(id) => {
                let r = self.mods_ui.store.set_enabled(&id, true, "rac1");
                let _ = self.mods_ui.store.save();
                self.mods_ui.selected = Some(id);
                match r {
                    Ok(()) if skipped > 0 => self.toast(format!("Added \"{name}\" and switched it on ({skipped} files left out: unusable or repeated names). It is used the next time the game starts."), false),
                    Ok(()) => self.toast(format!("Added \"{name}\" and switched it on. It is used the next time the game starts."), false),
                    Err(why) => self.toast(format!("Added \"{name}\", but it cannot be switched on: {why}."), true),
                }
            }
            Err(m) if m.to_lowercase().contains("cancel") => {}
            Err(m) => self.mods_ui.add = AddState::Error(m),
        }
    }

    fn mods_start_import(&mut self) {
        let AddState::Preview { cand, name } = std::mem::take(&mut self.mods_ui.add) else { return };
        let info = import::plan_info(&cand, |id| self.mods_ui.store.get(id).is_some(), Some(&name));
        let shown = info.name.clone();
        let state = import::start(*cand, info, self.mods_dir());
        self.mods_ui.add = AddState::Importing { state, name: shown };
    }

    pub fn show_mods(&mut self, ui: &mut Ui) {
        ui.label(theme::bold("Mods", 22.0));
        ui.label(theme::muted("Script mods (like Arsenal Mastery) and texture packs, including ones made for PCSX2. Changes apply the next time the game starts."));
        ui.add_space(4.0);
        if !self.profile_id.is_empty() {
            self.profile_header(ui);
            ui.add_space(4.0);
        }
        if let Some(n) = gating::needs_text(&self.info.local_version, gating::TEXTURE_PACKS) {
            theme::card_colored(ui, theme::ORANGE, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Mods and texture packs take effect once the game is updated.").color(theme::ORANGE).size(14.0));
                ui.label(theme::muted(format!("{n} You can already add and sort them here; they are used as soon as the game knows them.")));
            });
            ui.add_space(4.0);
        }
        self.mods_master_card(ui);
        ui.add_space(4.0);
        let running = self.read_only();
        ui.add_enabled_ui(!running, |ui| {
            self.mods_toolbar(ui);
            self.mods_add_panel(ui);
        });
        ui.add_space(2.0);
        if self.mods_ui.store.entries.is_empty() && matches!(self.mods_ui.add, AddState::None) {
            self.mods_empty(ui);
            return;
        }
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.add_enabled_ui(!running, |ui| self.mod_lists(ui));
            ui.add_space(8.0);
        });
    }

    /// The mod rows: script mods first, then texture packs, each list under a heading when both kinds are installed.
    fn mod_lists(&mut self, ui: &mut Ui) {
        let (scripts, packs): (Vec<String>, Vec<String>) = {
            let order: Vec<&ModEntry> = self.mods_ui.store.display_order();
            let (a, b): (Vec<&ModEntry>, Vec<&ModEntry>) = order.into_iter().partition(|e| e.is_script());
            (a.iter().map(|e| e.id.clone()).collect(), b.iter().map(|e| e.id.clone()).collect())
        };
        let both = !scripts.is_empty() && !packs.is_empty();
        for (title, ids) in [("Script mods", scripts), ("Texture packs", packs)] {
            if both {
                ui.add_space(2.0);
                ui.label(theme::bold(title, 16.0));
            }
            for id in ids {
                self.mod_row(ui, &id);
            }
        }
    }

    fn mods_empty(&mut self, ui: &mut Ui) {
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::bold("No mods yet", 18.0));
            ui.label(theme::muted("Click \"Add .zip\" or \"Add folder\" to add a texture pack, or \"Import from PCSX2\" if you already use texture packs there."));
            ui.label(theme::muted("A texture pack is a folder called \"replacements\" with picture files in it, as PCSX2 uses."));
            ui.label(theme::muted("A script mod is a folder with an omni-mod.json in it: put the folder into the mods folder (\"Open mods folder\") and it shows up here."));
        });
    }

    fn mods_toolbar(&mut self, ui: &mut Ui) {
        let busy = self.mods_ui.busy();
        ui.horizontal_wrapped(|ui| {
            if theme::button_enabled(ui, "Add .zip", !busy) {
                self.browse(Purpose::ModZip);
            }
            if theme::button_enabled(ui, "Add folder", !busy) {
                self.browse(Purpose::ModFolder);
            }
            if theme::button_enabled(ui, "Import from PCSX2", !busy) {
                self.mods_pcsx2_list();
            }
            if theme::button(ui, "Open mods folder") {
                let d = self.mods_dir();
                let _ = std::fs::create_dir_all(&d);
                platform::open_path(&d);
            }
        });
    }

    fn mods_pcsx2_list(&mut self) {
        let serials: Vec<&str> = match self.disc_serial() {
            Some(s) => vec![s],
            None => self.rac1().discs.iter().map(|d| d.serial).collect(),
        };
        let docs = platform::known_folder(platform::Known::Documents).unwrap_or_default();
        let found = import::pcsx2_candidates(&docs, &serials);
        self.mods_ui.add = if found.is_empty() {
            AddState::Error(format!(
                "No PCSX2 texture folder was found for your disc. PCSX2 keeps them in:\n{}\\PCSX2\\textures\\{}\\replacements",
                docs.display(),
                serials.first().copied().unwrap_or("SERIAL")
            ))
        } else {
            AddState::Pcsx2(found)
        };
    }

    fn mods_add_panel(&mut self, ui: &mut Ui) {
        match &self.mods_ui.add {
            AddState::None => {}
            AddState::Error(m) => {
                let m = m.clone();
                theme::card_colored(ui, theme::RED, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(m).color(theme::TEXT));
                    if theme::button(ui, "OK") {
                        self.mods_ui.add = AddState::None;
                    }
                });
            }
            AddState::Pcsx2(list) => {
                let list = list.clone();
                theme::card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(theme::bold("PCSX2 texture folders found", 16.0));
                    for (serial, src) in &list {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(format!("{serial}  -  {}", src.path().display())).size(13.5));
                            if theme::button(ui, "Look at it") {
                                self.mods_inspect(src.path());
                            }
                        });
                    }
                    if theme::button(ui, "Cancel") {
                        self.mods_ui.add = AddState::None;
                    }
                });
            }
            AddState::Preview { .. } => self.mods_preview(ui),
            AddState::Reading { state, name } => {
                let (state, name) = (state.clone(), name.clone());
                self.mods_reading_card(ui, &state, &name);
            }
            AddState::Importing { state, name } => {
                let (state, name) = (state.clone(), name.clone());
                mods_importing_card(ui, &state, &name);
            }
        }
    }

    /// "Reading the pack..." with an endless bar and a live count of the files found; Cancel closes it at once (the worker stops by itself).
    fn mods_reading_card(&mut self, ui: &mut Ui, state: &ReadState, name: &str) {
        let n = state.entries.load(Ordering::Relaxed);
        let mut cancel = false;
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::bold("Reading the pack...", 16.0));
            theme::progress_busy(ui, 8.0, theme::BLUE);
            let found = if n > 0 { format!("{} files found so far", count_text(n)) } else { "Opening it".to_string() };
            ui.add(egui::Label::new(theme::muted(format!("{name}  -  {found}"))).truncate());
            ui.label(theme::muted("A big pack can take a moment. The window stays usable."));
            cancel = theme::button(ui, "Cancel");
        });
        if cancel {
            state.cancel.store(true, Ordering::Relaxed);
            self.mods_ui.add = AddState::None;
        }
    }

    fn mods_preview(&mut self, ui: &mut Ui) {
        let AddState::Preview { cand, name } = &mut self.mods_ui.add else { return };
        let mut go = false;
        let mut cancel = false;
        let serial_note = match &cand.detected.layout {
            Layout::Pcsx2 { serial: Some(s), .. } => Some(s.clone()),
            _ => None,
        };
        let disc = self.info.disc.map(|d| d.serial);
        let mismatch = serial_note.as_ref().zip(disc).map(|(s, d)| !s.eq_ignore_ascii_case(d)).unwrap_or(false);
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::bold("Add this mod?", 16.0));
            ui.label(theme::muted(format!(
                "{}  -  {} files, {}{}",
                layout_text(&cand.detected.layout),
                cand.detected.files.len(),
                crate::discs::format_size(cand.detected.bytes),
                serial_note.as_ref().map(|s| format!("  -  made for {s}")).unwrap_or_default()
            )));
            if mismatch {
                ui.label(RichText::new(format!("Your disc is {}. {}", disc.unwrap_or(""), crate::mods::OTHER_REGION_NOTE)).color(theme::ORANGE));
            }
            if cand.info.is_none() {
                ui.horizontal(|ui| {
                    ui.label("Name:");
                    ui.add(egui::TextEdit::singleline(name).desired_width(280.0).hint_text("Name of this mod"));
                });
            }
            ui.horizontal(|ui| {
                go = theme::button(ui, "Add mod");
                cancel = theme::button(ui, "Cancel");
            });
        });
        if go {
            self.mods_start_import();
        } else if cancel {
            self.mods_ui.add = AddState::None;
        }
    }

    fn mod_row(&mut self, ui: &mut Ui, id: &str) {
        let Some(e) = self.mods_ui.store.get(id).cloned() else { return };
        let on = self.mods_ui.store.is_enabled(id);
        // switched on per mod, but its master tick is off: the game does not load it (shown dimmed, the switch stays on)
        let held = on && !self.mods_ui.store.is_loaded(id);
        let block = e.cannot_enable("rac1");
        let warn = e.serial_warning(self.disc_serial());
        let group = self.mods_ui.store.group_of(id);
        let (pos, enabled_count) = (group.iter().position(|a| a.as_str() == id), group.len());
        let has_settings = e.script.as_ref().is_some_and(|m| m.has_settings());
        let settings_open = has_settings && self.mods_ui.settings_open.as_deref() == Some(id);
        let mut settings_toggle = false;
        let mut changes: Changes = Vec::new();
        let given = e.script.as_ref().map(|m| self.mods_ui.settings.mod_values(&m.id)).unwrap_or_default();
        let (newer, broken) = (self.mods_ui.settings.newer, self.mods_ui.settings.broken.clone());
        let panel = &mut self.mods_ui.panel;
        let selected = self.mods_ui.selected.as_deref() == Some(id);
        let edge = if on && !held { theme::ORANGE } else { theme::LINE };
        let mut toggle = false;
        let mut mv = 0;
        let mut details = false;
        let mut remove = false;
        egui::Frame::new()
            .fill(theme::CARD)
            .stroke(egui::Stroke::new(if on && !held { 1.5 } else { 1.0 }, edge))
            .corner_radius(egui::CornerRadius::same(12))
            .inner_margin(egui::Margin::same(14))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if held {
                    ui.set_opacity(0.6);
                }
                let btn = RowButtons { has_settings, settings_open, selected, can_down: on && pos.map(|p| p + 1 < enabled_count).unwrap_or(false), can_up: on && pos.map(|p| p > 0).unwrap_or(false), on, script: e.is_script() };
                // the buttons sit to the right of the text when there is room, else under it (a long subtitle must not run under them)
                let reserve = btn.width();
                let side_by_side = ui.available_width() - 50.0 - reserve >= 320.0;
                let mut clicks = RowClicks::default();
                ui.horizontal_top(|ui| {
                    toggle = theme::switch(ui, on, block.is_none() || on, &format!("Use {}", e.name()));
                    let text_w = if side_by_side { ui.available_width() - reserve } else { ui.available_width() };
                    ui.allocate_ui_with_layout(egui::vec2(text_w, 0.0), egui::Layout::top_down(egui::Align::Min), |ui| {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(theme::bold(e.name(), 16.0));
                            if let Ok(i) = &e.info {
                                theme::chip(ui, kind_text(&i.kind), theme::BLUE);
                            }
                        });
                        ui.label(theme::muted(mod_subtitle(&e)));
                        if let Some(w) = warn.as_ref().or(block.as_ref()) {
                            ui.label(RichText::new(w).color(theme::ORANGE).size(13.0));
                        }
                        if !side_by_side {
                            ui.add_space(4.0);
                            ui.horizontal_wrapped(|ui| clicks = btn.draw(ui, false));
                        }
                    });
                    if side_by_side {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| clicks = btn.draw(ui, true));
                    }
                });
                (mv, details, remove, settings_toggle) = (clicks.mv, clicks.details, clicks.remove, clicks.settings);
                if settings_open {
                    ui.separator();
                    if let Some(m) = &e.script {
                        settings_notes(ui, newer, broken.as_deref());
                        changes = page_mods_script::panel(ui, m, &given, panel, newer);
                    }
                }
                if selected {
                    ui.separator();
                    mod_details(ui, &e);
                }
            });
        self.script_settings_actions(&e, settings_toggle, changes);
        self.mod_row_actions(id, toggle, mv, details, remove, selected);
        if self.mods_ui.confirm_remove.as_deref() == Some(id) {
            self.mod_confirm_remove(ui, id, &e);
        }
    }

    /// The Settings button, and the values the panel changed (written to settings.json now; the game reads them at its next start).
    fn script_settings_actions(&mut self, e: &ModEntry, toggle: bool, changes: Changes) {
        if toggle {
            self.mods_ui.settings_open = if self.mods_ui.settings_open.as_deref() == Some(e.id.as_str()) { None } else { Some(e.id.clone()) };
            self.mods_ui.panel = PanelState::default();
        }
        let Some(m) = &e.script else { return };
        if changes.is_empty() {
            return;
        }
        if let Err(why) = self.mods_ui.settings.apply(&m.id, &changes) {
            self.toast(format!("The settings of \"{}\" could not be saved: {why}", m.name), true);
        }
    }

    fn mod_row_actions(&mut self, id: &str, toggle: bool, mv: i32, details: bool, remove: bool, selected: bool) {
        let mut changed = false;
        if toggle {
            let on = self.mods_ui.store.is_enabled(id);
            match self.mods_ui.store.set_enabled(id, !on, "rac1") {
                Ok(()) => changed = true,
                Err(why) => self.toast(format!("This mod cannot be switched on: {why}."), true),
            }
        }
        if mv != 0 {
            self.mods_ui.store.move_by(id, mv);
            changed = true;
        }
        if changed {
            if let Err(e) = self.mods_ui.store.save() {
                self.toast(format!("The mod list could not be saved: {e}"), true);
            }
        }
        if details {
            self.mods_ui.selected = if selected { None } else { Some(id.to_string()) };
        }
        if remove {
            self.mods_ui.confirm_remove = Some(id.to_string());
        }
    }

    fn mod_confirm_remove(&mut self, ui: &mut Ui, id: &str, e: &ModEntry) {
        theme::card_colored(ui, theme::RED, |ui| {
            ui.set_width(ui.available_width());
            ui.label(format!("Remove \"{}\"? It goes to the Recycle Bin.", e.name()));
            ui.horizontal(|ui| {
                if theme::button(ui, "Yes, remove it") {
                    match self.mods_ui.store.remove(id) {
                        Ok(()) => {
                            let _ = self.mods_ui.store.save();
                            self.toast("Mod removed.", false);
                        }
                        Err(m) => self.toast(format!("The mod could not be removed: {m}"), true),
                    }
                    self.mods_ui.confirm_remove = None;
                    self.mods_ui.selected = None;
                }
                if theme::button(ui, "Keep it") {
                    self.mods_ui.confirm_remove = None;
                }
            });
        });
    }
}

/// "Importing <name>..." with the bar, files and size done of the total, the file being copied, and Cancel.
fn mods_importing_card(ui: &mut Ui, state: &ImportState, name: &str) {
    let (done, total) = (state.bytes_done.load(Ordering::Relaxed), state.bytes_total.load(Ordering::Relaxed));
    let (fdone, ftotal) = (state.files_done.load(Ordering::Relaxed), state.files_total.load(Ordering::Relaxed));
    let cancelling = state.cancel.load(Ordering::Relaxed);
    let cur = state.current.lock().map(|c| c.clone()).unwrap_or_default();
    theme::card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(theme::bold(format!("Importing \"{name}\"..."), 16.0));
        theme::progress(ui, state.percent(), 8.0, theme::BLUE);
        ui.label(theme::muted(format!(
            "{:.0}%  -  {} of {} files  -  {} of {}",
            state.percent() * 100.0,
            count_text(fdone),
            count_text(ftotal),
            crate::library::size_text(done),
            crate::library::size_text(total)
        )));
        ui.add(egui::Label::new(theme::muted(cur)).truncate());
        if cancelling {
            ui.label(theme::muted("Cancelling and cleaning up..."));
        } else if theme::button(ui, "Cancel") {
            state.cancel.store(true, Ordering::Relaxed);
        }
    });
    ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
}

/// 19591 -> "19,591".
fn count_text(n: u64) -> String {
    let d = n.to_string();
    let mut out = String::new();
    for (i, c) in d.chars().enumerate() {
        if i > 0 && (d.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// `text` cut at a word near `max` characters, with "..." (the details show it whole).
fn short_text(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    let cut = cut.rsplit_once(' ').map(|(a, _)| a).unwrap_or(&cut);
    format!("{}...", cut.trim_end_matches(|c: char| c == ',' || c == '.' || c == ';'))
}

/// Notes above a settings panel: a file the launcher must not write, or one it will replace.
fn settings_notes(ui: &mut Ui, newer: bool, broken: Option<&str>) {
    if newer {
        ui.label(RichText::new("settings.json was written by a newer version of the game. It is shown here but cannot be changed from this launcher.").color(theme::ORANGE).size(13.0));
    } else if let Some(why) = broken {
        ui.label(RichText::new(format!("settings.json is damaged ({why}). The defaults are shown; the first change you make saves a new file and keeps the old one as settings.json.broken.")).color(theme::ORANGE).size(13.0));
    }
}

fn mod_subtitle(e: &ModEntry) -> String {
    match &e.info {
        Ok(i) => {
            let mut parts: Vec<String> = Vec::new();
            if !i.author.is_empty() {
                parts.push(format!("by {}", i.author));
            }
            if !i.version.is_empty() {
                parts.push(format!("v{}", i.version));
            }
            if !i.description.is_empty() {
                parts.push(short_text(i.description.lines().next().unwrap_or(""), 110));
            }
            parts.join("  -  ")
        }
        Err(m) => format!("Unreadable: {m}"),
    }
}

fn mod_details(ui: &mut Ui, e: &ModEntry) {
    if let (Ok(i), Some(m)) = (&e.info, &e.script) {
        ui.label(&i.description);
        let games: Vec<String> = m.games.iter().map(|(g, v)| format!("{} ({})", g, v.join(", "))).collect();
        let folder = e.dir.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        ui.label(theme::muted(format!("Made for: {}   Id: {}   Folder name: {}", games.join("; "), m.id, folder)));
    } else if let Ok(i) = &e.info {
        // the first line is already under the name; the details show a longer text only
        if i.description.lines().count() > 1 {
            ui.label(&i.description);
        }
        let serials = if i.serials.is_empty() { "any disc".to_string() } else { i.serials.join(", ") };
        ui.label(theme::muted(format!("Made for: {serials}   Source: {}   Folder name: {}", if i.source.is_empty() { "unknown" } else { &i.source }, e.id)));
    }
    ui.label(theme::muted(format!("Location: {}", e.dir.display())));
    if theme::button(ui, "Open its folder") {
        platform::open_path(&e.dir);
    }
}

/// What the buttons of a mod row show (the row draws them to the right of its text, or under it when the window is narrow).
struct RowButtons {
    has_settings: bool,
    settings_open: bool,
    selected: bool,
    on: bool,
    can_down: bool,
    can_up: bool,
    script: bool,
}

#[derive(Default)]
struct RowClicks {
    mv: i32,
    details: bool,
    remove: bool,
    settings: bool,
}

impl RowButtons {
    /// A little more than the buttons need, in points.
    fn width(&self) -> f32 {
        let mut w = 8.0 + 86.0 + 105.0; // Remove, Details / Hide details
        if self.has_settings {
            w += 8.0 + 120.0;
        }
        if self.on {
            w += 2.0 * (8.0 + 30.0);
        }
        w + 12.0
    }

    /// Draws up, down, settings, details, remove (in this order on screen: `right_to_left` = the layout is right to left, so reversed).
    fn draw(&self, ui: &mut Ui, right_to_left: bool) -> RowClicks {
        #[derive(Clone, Copy)]
        enum B {
            Up,
            Down,
            Settings,
            Details,
            Remove,
        }
        let mut order = vec![B::Up, B::Down, B::Settings, B::Details, B::Remove];
        if right_to_left {
            order.reverse();
        }
        let mut c = RowClicks::default();
        for b in order {
            match b {
                B::Up | B::Down if !self.on => {}
                B::Up => {
                    let tip = if self.script { "Earlier in the list: the game loads it first" } else { "Higher priority: wins when two packs change the same picture" };
                    if theme::icon_button(ui, Icon::Up, self.can_up, tip) {
                        c.mv = -1;
                    }
                }
                B::Down => {
                    if theme::icon_button(ui, Icon::Down, self.can_down, "Lower priority") {
                        c.mv = 1;
                    }
                }
                B::Settings if self.has_settings => c.settings = theme::button(ui, if self.settings_open { "Hide settings" } else { "Mod settings" }),
                B::Settings => {}
                B::Details => c.details = theme::button(ui, if self.selected { "Hide details" } else { "Details" }),
                B::Remove => c.remove = theme::button(ui, "Remove"),
            }
        }
        c
    }
}
