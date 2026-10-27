//! The flows behind the library and the profiles: open / add / remove an install, create / copy / rename / delete / select a profile.
//! (The pages `page_library.rs` and `profile_picker.rs` only draw and call these.)
use super::page_mods::ModsUi;
use super::state::{App, DiscState, Sel, View};
use crate::games::InstallInfo;
use crate::library::{self, NEEDED_BYTES};
use crate::platform;
use crate::profiles::Profiles;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKind {
    New,
    Copy,
    Rename,
    Delete,
}

/// The small form of the profile picker (a name to type, or a delete to confirm).
#[derive(Clone, Debug)]
pub struct ProfileEdit {
    pub kind: EditKind,
    pub text: String,
}

#[derive(Default)]
pub struct ProfileUi {
    /// The picker's list is open.
    pub open: bool,
    pub edit: Option<ProfileEdit>,
    pub error: Option<String>,
    /// Where the button was drawn this frame (`Rect::ZERO` = not drawn); the list opens under it.
    pub anchor: Option<egui::Rect>,
}

/// The two ways to add an install, as the card on the Library page shows them.
#[derive(Clone, Debug)]
pub enum AddDialog {
    /// "Add existing install": the folder of a set-up game.
    Existing { text: String },
    /// "New install": the folder the game should live in.
    New { text: String },
}

#[derive(Default)]
pub struct LibUi {
    pub dialog: Option<AddDialog>,
    /// Library id whose "Remove from list" asks for a second click.
    pub confirm_remove: Option<String>,
    /// Library id of a backup whose "Delete backup" asks for a second click.
    pub confirm_delete: Option<String>,
    /// The message under the dialog (error or note) after Add / Install was clicked.
    pub message: Option<(String, bool)>,
}

/// Text for a folder that is too full for a new install (None = fine or unknown).
pub fn space_problem(free: Option<u64>) -> Option<String> {
    match free {
        Some(f) if f < NEEDED_BYTES => Some(format!("Not enough free space on that drive: {} free, about {} needed.", library::size_text(f), library::size_text(NEEDED_BYTES))),
        _ => None,
    }
}

fn clean_path(text: &str) -> PathBuf {
    PathBuf::from(text.trim().trim_matches('"'))
}

impl App {
    // ------------------------------------------------------------------ opening an install

    /// Makes the library entry `id` the open install (reads its version, profiles, settings, mods). No checks for a running game.
    pub fn enter_install(&mut self, id: &str) {
        let Some(e) = self.library.get(id).cloned() else { return };
        if let Some(c) = self.disc_check.take() {
            c.cancel();
        }
        self.cur_id = Some(e.id.clone());
        self.root = e.path();
        self.view = View::Home;
        self.failure = None;
        self.disc_text = String::new();
        self.disc_state = DiscState::Empty;
        self.disc_path = None;
        self.disc_edit_at = None;
        self.game_external = false;
        self.last_ext_check = Instant::now() - Duration::from_secs(10);
        self.play_error = None;
        self.mods_ui = ModsUi::default();
        self.profile_ui = Default::default();
        self.confirm_reset = false;
        self.refresh_info();
        self.reload_profiles();
        self.reload_game_data();
        if self.info.installed {
            self.check_updates();
            if let Some(iso) = self.info.iso.clone().filter(|_| self.info.iso_exists) {
                self.disc_text = iso.to_string_lossy().to_string();
            }
        }
    }

    /// Opens an install from the sidebar or the Library page. Refused while something runs.
    pub fn select_install(&mut self, id: &str) -> bool {
        if self.cur_id.as_deref() == Some(id) {
            self.sel = Sel::Game("rac1");
            return true;
        }
        if self.busy() || self.game_running() {
            self.toast("Wait until the setup or the game is finished, then open the other install.", true);
            return false;
        }
        if self.library.get(id).is_none() {
            return false;
        }
        self.enter_install(id);
        self.sel = Sel::Game("rac1");
        true
    }

    // ------------------------------------------------------------------ the library

    pub(crate) fn save_library(&mut self) {
        if let Err(e) = self.library.save() {
            self.toast(format!("The list of installs could not be saved: {e}"), true);
        }
    }

    /// "Add existing install" with the typed or picked folder. Opens the install on success.
    pub fn add_existing_install(&mut self, text: &str) -> Result<String, String> {
        let e = self.library.add_existing(&clean_path(text))?;
        self.save_library();
        let version = crate::games::local_version(&e.path());
        self.select_install(&e.id);
        Ok(format!("Added Ratchet & Clank {version} from {}", e.folder))
    }

    /// "New install" with the typed or picked folder. A folder that already holds an unpacked RAC-RS release is listed as it is;
    /// otherwise the latest release is downloaded into it (`state_net.rs`); when that is done the entry is added and the disc setup is next.
    pub fn new_install(&mut self, text: &str) -> Result<String, String> {
        let dir = clean_path(text);
        if dir.as_os_str().is_empty() {
            return Err("Choose a folder for the game.".to_string());
        }
        if dir.is_file() {
            return Err("That is a file, not a folder.".to_string());
        }
        if let Some(e) = self.library.find_folder(&dir).cloned() {
            self.select_install(&e.id);
            return Ok("This folder was already in your list. It is open now.".to_string());
        }
        if !library::is_release_tree(&dir) {
            if let Some(p) = space_problem(library::free_space(&dir)) {
                return Err(p);
            }
            let want = self.picked_version();
            return self.start_game_download(dir, want);
        }
        let installed = library::check_folder(&dir).is_ok();
        if !installed {
            if let Some(p) = space_problem(library::free_space(&dir)) {
                return Err(p);
            }
        }
        let game = self.rac1();
        let e = self.library.add(game, &dir);
        self.save_library();
        self.select_install(&e.id);
        Ok(if installed { "This folder already holds a set-up game. It is in your list now.".to_string() } else { "The folder is in your list. Pick your disc to set the game up.".to_string() })
    }

    /// "Remove from list": the entry goes, no file is touched.
    pub fn remove_install(&mut self, id: &str) {
        if self.cur_id.as_deref() == Some(id) && (self.busy() || self.game_running()) {
            self.toast("Wait until the setup or the game is finished, then remove the install.", true);
            return;
        }
        if !self.library.remove(id) {
            return;
        }
        self.save_library();
        self.lib_ui.confirm_remove = None;
        if self.cur_id.as_deref() == Some(id) {
            match self.library.entries.first().map(|e| e.id.clone()) {
                Some(next) => self.enter_install(&next),
                None => {
                    self.cur_id = None;
                    self.root = PathBuf::new();
                    self.info = InstallInfo::default();
                    self.profiles = Profiles::new(Path::new(""));
                    self.profile_id.clear();
                    self.profile_list.clear();
                    self.mods_ui = ModsUi::default();
                    self.sel = Sel::Library;
                }
            }
        }
        self.toast("Removed from the list. The files stay on your disk.", false);
    }

    // ------------------------------------------------------------------ profiles

    /// Profiles of the open install: recovers a run that was not copied back, makes "Default" on the first start, reads the list.
    pub fn reload_profiles(&mut self) {
        self.profiles = Profiles::new(&self.root);
        self.profile_id.clear();
        self.profile_list.clear();
        if !self.has_install() || !self.info.installed {
            return;
        }
        let g = self.rac1();
        if self.profiles.pending_marker().is_some() {
            if platform::exe_in_use(&g.exe_path(&self.root)) {
                self.game_external = true;
            } else {
                self.profiles.copy_back(g);
            }
        }
        match self.profiles.ensure(g) {
            Ok(id) => self.profile_id = id,
            Err(e) => self.toast(format!("The profiles could not be read: {e}"), true),
        }
        self.profile_list = self.profiles.list();
    }

    pub fn profile_name(&self) -> String {
        self.profile_list.iter().find(|p| p.id == self.profile_id).map(|p| p.meta.name.clone()).unwrap_or_else(|| "Default".to_string())
    }

    /// Picks another profile (not while the game runs).
    pub fn select_profile(&mut self, id: &str) {
        if self.read_only() || id == self.profile_id {
            return;
        }
        match self.profiles.set_current(id) {
            Ok(()) => {
                self.profile_id = id.to_string();
                self.confirm_reset = false;
                self.reload_game_data();
            }
            Err(e) => self.toast(e, true),
        }
    }

    /// Opens the small form of the picker.
    pub fn profile_begin(&mut self, kind: EditKind) {
        let text = match kind {
            EditKind::New => String::new(),
            EditKind::Copy => format!("{} copy", self.profile_name()),
            EditKind::Rename => self.profile_name(),
            EditKind::Delete => String::new(),
        };
        self.profile_ui.edit = Some(ProfileEdit { kind, text });
        self.profile_ui.error = None;
    }

    /// OK in the form of the picker: does it, or keeps the form open with the reason.
    pub fn profile_apply(&mut self) {
        let Some(edit) = self.profile_ui.edit.clone() else { return };
        let cur = self.profile_id.clone();
        let r: Result<Option<String>, String> = match edit.kind {
            EditKind::New => self.profiles.create(&edit.text).map(Some),
            EditKind::Copy => self.profiles.copy(&cur, &edit.text).map(Some),
            EditKind::Rename => self.profiles.rename(&cur, &edit.text).map(|_| None),
            EditKind::Delete => self.profiles.delete(&cur).map(|_| None),
        };
        match r {
            Ok(new_id) => {
                self.profile_ui.edit = None;
                self.profile_ui.error = None;
                if let Some(id) = new_id {
                    let _ = self.profiles.set_current(&id);
                }
                self.profile_id = self.profiles.current().unwrap_or_default();
                self.profile_list = self.profiles.list();
                self.reload_game_data();
            }
            Err(m) => self.profile_ui.error = Some(m),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn space_message() {
        assert!(space_problem(Some(1024 * 1024 * 1024)).unwrap().contains("Not enough free space"));
        assert!(space_problem(Some(NEEDED_BYTES)).is_none());
        assert!(space_problem(None).is_none());
    }

    #[test]
    fn typed_paths_lose_quotes() {
        assert_eq!(clean_path("  \"D:\\Games\\RAC\"  "), PathBuf::from("D:\\Games\\RAC"));
    }
}
