//! The Library page: every game install the launcher knows (open, show folder, remove from the list) and the two ways to add one.
use super::state::{App, Purpose, Sel};
use super::state_lib::{space_problem, AddDialog};
use super::theme;
use crate::games;
use crate::library::{self, LibEntry, NEEDED_BYTES};
use egui::{RichText, ScrollArea, Ui};
use std::path::Path;

/// What the Library shows about one entry (read from the disk).
struct EntryView {
    version: String,
    installed: bool,
    exists: bool,
}

fn look(e: &LibEntry) -> EntryView {
    let dir = e.path();
    let g = games::game_by_id(&e.game).unwrap_or(&games::GAMES[0]);
    EntryView { version: games::local_version(&dir), installed: games::is_installed(&dir, g), exists: dir.is_dir() }
}

impl App {
    pub fn show_library(&mut self, ui: &mut Ui) {
        ui.label(theme::bold("Library", 26.0));
        ui.label(theme::muted("Your Ratchet & Clank installs. You can keep several, for example the Europe and the USA version. Removing one from the list never deletes its files."));
        ui.add_space(10.0);
        ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            self.download_card(ui);
            let entries = self.library.entries.clone();
            if entries.is_empty() {
                theme::card_colored(ui, theme::BLUE, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(theme::bold("No game yet", 18.0));
                    ui.label(theme::muted("Add a game you already set up, or make a new install below."));
                });
            }
            for e in &entries {
                self.library_card(ui, e);
                ui.add_space(8.0);
            }
            ui.add_space(6.0);
            self.library_add_card(ui);
            ui.add_space(12.0);
        });
    }

    fn library_card(&mut self, ui: &mut Ui, e: &LibEntry) {
        let v = look(e);
        let g = games::game_by_id(&e.game).unwrap_or(&games::GAMES[0]);
        let current = self.cur_id.as_deref() == Some(e.id.as_str());
        let mut open = false;
        let mut reveal = false;
        let mut saves = false;
        let mut ask_remove = false;
        let mut remove = false;
        let mut cancel = false;
        let mut ask_delete = false;
        let mut delete = false;
        let edge = if current { theme::ORANGE } else { theme::LINE };
        theme::card_colored(ui, edge, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(theme::bold(e.title(g), 18.0));
                if e.backup {
                    theme::chip(ui, "Backup", theme::ORANGE);
                }
                theme::chip(ui, &format!("Version {}", v.version), theme::BLUE);
                if !v.exists {
                    theme::chip(ui, "Folder not found", theme::RED);
                } else if v.installed {
                    theme::chip(ui, "Ready", theme::GREEN);
                } else {
                    theme::chip(ui, "Not set up yet", theme::ORANGE);
                }
                if current {
                    theme::chip(ui, "Open now", theme::ORANGE);
                }
            });
            ui.label(theme::muted(e.folder.clone()));
            ui.label(theme::muted(format!("Added {}", e.added)));
            ui.label(theme::muted(crate::play::SAVES_HINT));
            ui.add_space(4.0);
            if self.lib_ui.confirm_delete.as_deref() == Some(e.id.as_str()) {
                ui.label(RichText::new("Delete this backup? Its folder goes to the Recycle Bin, with the saves that are in it. Your main install is not touched.").color(theme::TEXT));
                ui.horizontal(|ui| {
                    delete = theme::button(ui, "Yes, delete the backup");
                    cancel = theme::button(ui, "Cancel");
                });
            } else if self.lib_ui.confirm_remove.as_deref() == Some(e.id.as_str()) {
                ui.label(RichText::new("Remove this install from the list? Its files and saves stay on your disk.").color(theme::TEXT));
                ui.horizontal(|ui| {
                    remove = theme::button(ui, "Yes, remove from list");
                    cancel = theme::button(ui, "Cancel");
                });
            } else {
                ui.horizontal_wrapped(|ui| {
                    open = theme::button_enabled(ui, if current { "Go to game" } else { "Open" }, v.exists);
                    reveal = theme::button_enabled(ui, "Show folder", v.exists);
                    saves = theme::button_enabled(ui, "Open saves folder", v.exists);
                    if e.backup {
                        ask_delete = theme::button(ui, "Delete backup");
                    }
                    ask_remove = theme::button(ui, "Remove from list");
                });
            }
        });
        if open {
            self.select_install(&e.id);
        }
        if saves {
            if let Err(err) = crate::play::open_saves(&e.path()) {
                self.toast(&format!("The saves folder could not be opened: {err}"), true);
            }
        }
        if reveal {
            crate::platform::open_path(&e.path());
        }
        if ask_remove {
            self.lib_ui.confirm_remove = Some(e.id.clone());
        }
        if ask_delete {
            self.lib_ui.confirm_delete = Some(e.id.clone());
        }
        if delete {
            self.delete_backup(&e.id);
        }
        if cancel {
            self.lib_ui.confirm_remove = None;
            self.lib_ui.confirm_delete = None;
        }
        if remove {
            self.remove_install(&e.id);
        }
    }

    fn library_add_card(&mut self, ui: &mut Ui) {
        theme::card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(theme::bold("Add an install", 17.0).color(theme::ORANGE));
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                if theme::button(ui, "Add existing install...") {
                    self.lib_ui.dialog = Some(AddDialog::Existing { text: String::new() });
                    self.lib_ui.message = None;
                }
                if theme::button(ui, "New install...") {
                    self.lib_ui.dialog = Some(AddDialog::New { text: library::default_install_folder().to_string_lossy().to_string() });
                    self.lib_ui.message = None;
                }
            });
            match self.lib_ui.dialog.clone() {
                Some(AddDialog::Existing { text }) => self.add_existing_form(ui, text),
                Some(AddDialog::New { text }) => self.new_install_form(ui, text),
                None => {
                    ui.label(theme::muted("Existing: a folder with a game you set up before. New: choose where the game should live."));
                }
            }
            if let Some((m, err)) = self.lib_ui.message.clone() {
                ui.add_space(4.0);
                ui.label(RichText::new(m).color(if err { theme::RED } else { theme::GREEN }).size(14.0));
            }
        });
    }

    /// Text field + Browse; returns the new text when it changed.
    fn folder_field(&mut self, ui: &mut Ui, text: &str, hint: &str, purpose: Purpose) -> Option<String> {
        let mut t = text.to_string();
        let mut changed = false;
        ui.horizontal(|ui| {
            let w = (ui.available_width() - 110.0).max(120.0);
            let r = ui.add(egui::TextEdit::singleline(&mut t).hint_text(hint).desired_width(w).margin(egui::Margin::symmetric(10, 8)));
            r.widget_info(|| egui::WidgetInfo::text_edit(true, "", "", hint));
            changed = r.changed();
            if theme::button_enabled(ui, "Browse...", self.picker.is_none()) {
                self.browse(purpose);
            }
        });
        changed.then_some(t)
    }

    fn add_existing_form(&mut self, ui: &mut Ui, text: String) {
        ui.add_space(6.0);
        ui.separator();
        ui.label(theme::bold("Add existing install", 15.5));
        ui.label(theme::muted("Pick the folder of a RAC-RS game that was set up before. It must hold the files VERSION and setup.ps1 and the built game."));
        if let Some(t) = self.folder_field(ui, &text, "Folder of the game", Purpose::AddInstall) {
            self.lib_ui.dialog = Some(AddDialog::Existing { text: t });
            self.lib_ui.message = None;
        }
        let path = std::path::PathBuf::from(text.trim().trim_matches('"'));
        let mut ok = false;
        if text.trim().is_empty() {
            ui.label(theme::muted("Type the folder or use Browse."));
        } else {
            match library::check_folder(&path) {
                Ok(f) => {
                    ok = true;
                    ui.label(RichText::new(format!("Found Ratchet & Clank, version {}. Ready to add.", f.version)).color(theme::GREEN).size(14.5));
                }
                Err(m) => {
                    ui.label(RichText::new(m).color(theme::RED).size(14.0));
                }
            }
        }
        ui.horizontal(|ui| {
            if theme::button_enabled(ui, "Add", ok) {
                self.lib_ui.message = Some(match self.add_existing_install(&text) {
                    Ok(m) => {
                        self.lib_ui.dialog = None;
                        (m, false)
                    }
                    Err(m) => (m, true),
                });
            }
            if theme::button(ui, "Cancel") {
                self.lib_ui.dialog = None;
                self.lib_ui.message = None;
            }
        });
    }

    fn new_install_form(&mut self, ui: &mut Ui, text: String) {
        ui.add_space(6.0);
        ui.separator();
        ui.label(theme::bold("New install", 15.5));
        ui.label(theme::muted("Where should the game live? It needs about 5 GB. You can change the folder."));
        if let Some(t) = self.folder_field(ui, &text, "Folder for the game", Purpose::NewInstall) {
            self.lib_ui.dialog = Some(AddDialog::New { text: t });
            self.lib_ui.message = None;
        }
        self.load_versions(false);
        self.version_picker(ui);
        let path = std::path::PathBuf::from(text.trim().trim_matches('"'));
        if !text.trim().is_empty() {
            self.free_space_line(ui, &path);
        }
        if library::is_release_tree(&path) {
            ui.label(theme::muted("This folder already holds a RAC-RS release. It will be used."));
        } else {
            ui.label(theme::muted("The chosen version is downloaded from GitHub into this folder (a few MB), then you pick your disc to set the game up."));
        }
        ui.horizontal(|ui| {
            if theme::button_enabled(ui, if library::is_release_tree(&path) { "Use this folder" } else { "Download the game" }, !text.trim().is_empty() && !self.downloading()) {
                self.lib_ui.message = Some(match self.new_install(&text) {
                    Ok(m) => {
                        self.lib_ui.dialog = None;
                        (m, false)
                    }
                    Err(m) => (m, true),
                });
            }
            if theme::button(ui, "Cancel") {
                self.lib_ui.dialog = None;
                self.lib_ui.message = None;
            }
        });
    }

    fn free_space_line(&self, ui: &mut Ui, path: &Path) {
        match library::free_space(path) {
            Some(f) => {
                let low = space_problem(Some(f)).is_some();
                let t = format!("Free space on that drive: {} (about {} needed).", library::size_text(f), library::size_text(NEEDED_BYTES));
                ui.label(RichText::new(t).color(if low { theme::RED } else { theme::GREEN }).size(14.0));
            }
            None => {
                ui.label(theme::muted("The free space of that drive is unknown."));
            }
        }
    }

    /// The button the sidebar uses for "add / manage".
    pub fn goto_library(&mut self) {
        self.sel = Sel::Library;
    }
}
