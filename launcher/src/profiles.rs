//! Profiles of one game install. A profile = its own `pc_options.txt` (settings) + its own `active.txt` (mods on/off and their order).
//! The mods themselves stay in the install's `mods\` folder, the saves (`player\card.bin`) are shared by every profile.
//!
//! On disk: `<install>\profiles\<id>\profile.json` (name, created, last used), `pc_options.txt`, `active.txt`; `<install>\profiles\current.txt`.
//! The game is unchanged: before Play the launcher copies the chosen profile into `player\pc_options.txt` and `mods\active.txt`
//! (and writes a marker); after the game closes the launcher copies `player\pc_options.txt` back into the profile, so changes made
//! in the game's own Options menu stay in the profile. A marker left behind (the launcher was closed while the game ran) is handled at the next start.
use crate::games::GameDef;
use crate::library;
use crate::mods;
use crate::pcopts::Options;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DIR: &str = "profiles";
const CURRENT: &str = "current.txt";
const MARKER: &str = "running.txt";
pub const OPTIONS: &str = "pc_options.txt";
pub const ACTIVE: &str = "active.txt";
pub const DEFAULT_NAME: &str = "Default";
pub const MAX_NAME: usize = 40;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meta {
    pub name: String,
    /// `YYYY-MM-DD`.
    pub created: String,
    /// `YYYY-MM-DD`, empty = never played with it.
    #[serde(default)]
    pub last_used: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Profile {
    pub id: String,
    pub meta: Meta,
}

/// The profiles of the install in `root`.
#[derive(Clone, Debug)]
pub struct Profiles {
    pub root: PathBuf,
}

/// A name a player typed: trimmed, 1..=40 characters, no path characters.
pub fn clean_name(name: &str) -> Result<String, String> {
    let n = name.split_whitespace().collect::<Vec<_>>().join(" ");
    if n.is_empty() {
        return Err("Give the profile a name.".to_string());
    }
    if n.chars().count() > MAX_NAME {
        return Err(format!("The name is too long (at most {MAX_NAME} characters)."));
    }
    if n.chars().any(|c| matches!(c, '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()) {
        return Err("The name may not contain \\ / : * ? \" < > |".to_string());
    }
    Ok(n)
}

fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path)
}

fn copy_atomic(from: &Path, to: &Path) -> std::io::Result<()> {
    write_atomic(to, &std::fs::read(from)?)
}

impl Profiles {
    pub fn new(root: &Path) -> Profiles {
        Profiles { root: root.to_path_buf() }
    }

    pub fn dir(&self) -> PathBuf {
        self.root.join(DIR)
    }

    pub fn profile_dir(&self, id: &str) -> PathBuf {
        self.dir().join(id)
    }

    pub fn options_file(&self, id: &str) -> PathBuf {
        self.profile_dir(id).join(OPTIONS)
    }

    pub fn active_file(&self, id: &str) -> PathBuf {
        self.profile_dir(id).join(ACTIVE)
    }

    pub fn list(&self) -> Vec<Profile> {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(self.dir()) {
            for e in rd.flatten() {
                let id = e.file_name().to_string_lossy().to_string();
                if !e.path().is_dir() || !mods::valid_id(&id) {
                    continue;
                }
                if let Some(meta) = std::fs::read(e.path().join("profile.json")).ok().and_then(|b| serde_json::from_slice::<Meta>(&b).ok()) {
                    out.push(Profile { id, meta });
                }
            }
        }
        out.sort_by(|a, b| a.meta.created.cmp(&b.meta.created).then_with(|| a.meta.name.to_lowercase().cmp(&b.meta.name.to_lowercase())).then_with(|| a.id.cmp(&b.id)));
        out
    }

    pub fn get(&self, id: &str) -> Option<Profile> {
        self.list().into_iter().find(|p| p.id == id)
    }

    /// The selected profile (`current.txt`); falls back to the first one when the file is missing or names a profile that is gone.
    pub fn current(&self) -> Option<String> {
        let list = self.list();
        let want = std::fs::read_to_string(self.dir().join(CURRENT)).unwrap_or_default().trim().trim_start_matches('\u{feff}').to_string();
        list.iter().find(|p| p.id == want).or_else(|| list.first()).map(|p| p.id.clone())
    }

    pub fn set_current(&self, id: &str) -> Result<(), String> {
        if self.get(id).is_none() {
            return Err("That profile does not exist.".to_string());
        }
        write_atomic(&self.dir().join(CURRENT), format!("{id}\n").as_bytes()).map_err(|e| e.to_string())
    }

    fn unique_id(&self, name: &str) -> String {
        let base = if name.chars().any(|c| c.is_ascii_alphanumeric()) { mods::slug(name) } else { "profile".to_string() };
        let base = if mods::valid_id(&base) { base } else { "profile".to_string() };
        let taken = |id: &str| self.profile_dir(id).exists();
        if !taken(&base) {
            return base;
        }
        (2..).map(|n| format!("{base}-{n}")).find(|id| !taken(id)).unwrap_or(base)
    }

    fn name_taken(&self, name: &str, except: Option<&str>) -> bool {
        self.list().iter().any(|p| Some(p.id.as_str()) != except && p.meta.name.to_lowercase() == name.to_lowercase())
    }

    fn write_meta(&self, id: &str, meta: &Meta) -> Result<(), String> {
        let text = serde_json::to_string_pretty(meta).map_err(|e| e.to_string())?;
        write_atomic(&self.profile_dir(id).join("profile.json"), text.as_bytes()).map_err(|e| e.to_string())
    }

    fn make(&self, name: &str, options: &[u8], active: &[u8]) -> Result<String, String> {
        let name = clean_name(name)?;
        if self.name_taken(&name, None) {
            return Err(format!("There is already a profile called \"{name}\"."));
        }
        let id = self.unique_id(&name);
        write_atomic(&self.options_file(&id), options).map_err(|e| e.to_string())?;
        write_atomic(&self.active_file(&id), active).map_err(|e| e.to_string())?;
        self.write_meta(&id, &Meta { name, created: library::today(), last_used: String::new() })?;
        Ok(id)
    }

    /// First start: when there is no profile yet, "Default" is made from the install's current `player\pc_options.txt` and `mods\active.txt`.
    /// Returns the selected profile.
    pub fn ensure(&self, game: &GameDef) -> Result<String, String> {
        if self.list().is_empty() {
            let options = std::fs::read(game.options_path(&self.root)).unwrap_or_else(|_| Options::new().to_text().into_bytes());
            let active = std::fs::read(game.mods_dir(&self.root).join(ACTIVE)).unwrap_or_else(|_| mods::active_text(&[]).into_bytes());
            let id = self.make(DEFAULT_NAME, &options, &active)?;
            self.set_current(&id)?;
            return Ok(id);
        }
        let cur = self.current().ok_or("No profile.")?;
        if std::fs::read_to_string(self.dir().join(CURRENT)).map(|t| t.trim() != cur).unwrap_or(true) {
            self.set_current(&cur)?;
        }
        Ok(cur)
    }

    /// New profile with the defaults: the game's own settings (empty options file), no mods on.
    pub fn create(&self, name: &str) -> Result<String, String> {
        self.make(name, Options::new().to_text().as_bytes(), mods::active_text(&[]).as_bytes())
    }

    /// New profile with a copy of the settings and mods list of `from`.
    pub fn copy(&self, from: &str, name: &str) -> Result<String, String> {
        if self.get(from).is_none() {
            return Err("The profile to copy does not exist.".to_string());
        }
        let options = std::fs::read(self.options_file(from)).unwrap_or_else(|_| Options::new().to_text().into_bytes());
        let active = std::fs::read(self.active_file(from)).unwrap_or_else(|_| mods::active_text(&[]).into_bytes());
        self.make(name, &options, &active)
    }

    pub fn rename(&self, id: &str, name: &str) -> Result<(), String> {
        let name = clean_name(name)?;
        let mut p = self.get(id).ok_or("That profile does not exist.")?;
        if self.name_taken(&name, Some(id)) {
            return Err(format!("There is already a profile called \"{name}\"."));
        }
        p.meta.name = name;
        self.write_meta(id, &p.meta)
    }

    /// Deletes a profile (never the last one). The selection moves to the first remaining profile when it was the deleted one.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        let list = self.list();
        if !list.iter().any(|p| p.id == id) {
            return Err("That profile does not exist.".to_string());
        }
        if list.len() <= 1 {
            return Err("The last profile cannot be deleted.".to_string());
        }
        if self.pending_marker().as_deref() == Some(id) {
            return Err("The game is using this profile right now.".to_string());
        }
        let was_current = self.current().as_deref() == Some(id);
        std::fs::remove_dir_all(self.profile_dir(id)).map_err(|e| e.to_string())?;
        if was_current {
            if let Some(first) = self.list().first() {
                self.set_current(&first.id)?;
            }
        }
        Ok(())
    }

    fn touch_used(&self, id: &str) {
        if let Some(mut p) = self.get(id) {
            p.meta.last_used = library::today();
            let _ = self.write_meta(id, &p.meta);
        }
    }

    // ------------------------------------------------------------------ the game side

    fn marker_file(&self) -> PathBuf {
        self.dir().join(MARKER)
    }

    /// The profile the game was started with and that was not copied back yet.
    pub fn pending_marker(&self) -> Option<String> {
        let t = std::fs::read_to_string(self.marker_file()).ok()?;
        let id = t.trim().trim_start_matches('\u{feff}').to_string();
        (!id.is_empty()).then_some(id)
    }

    /// Before Play: marker, then the profile's files into `player\pc_options.txt` and `mods\active.txt`.
    pub fn swap_in(&self, game: &GameDef, id: &str) -> Result<(), String> {
        if self.get(id).is_none() {
            return Err("That profile does not exist.".to_string());
        }
        write_atomic(&self.marker_file(), format!("{id}\n").as_bytes()).map_err(|e| e.to_string())?;
        let opt = self.options_file(id);
        if opt.is_file() {
            copy_atomic(&opt, &game.options_path(&self.root)).map_err(|e| format!("The profile's settings could not be written for the game: {e}"))?;
        }
        let act = self.active_file(id);
        if act.is_file() {
            copy_atomic(&act, &game.mods_dir(&self.root).join(ACTIVE)).map_err(|e| format!("The profile's mods list could not be written for the game: {e}"))?;
        }
        self.touch_used(id);
        Ok(())
    }

    /// After the game closed: `player\pc_options.txt` goes back into the profile named by the marker, the marker is removed.
    /// Returns that profile (None = no marker).
    pub fn copy_back(&self, game: &GameDef) -> Option<String> {
        let id = self.pending_marker()?;
        let live = game.options_path(&self.root);
        if self.get(&id).is_some() && live.is_file() {
            let _ = copy_atomic(&live, &self.options_file(&id));
        }
        let _ = std::fs::remove_file(self.marker_file());
        Some(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::game_by_id;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_prof_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn rac1() -> &'static GameDef {
        game_by_id("rac1").unwrap()
    }

    fn put(p: PathBuf, t: &str) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, t).unwrap();
    }

    fn read(p: PathBuf) -> String {
        std::fs::read_to_string(p).unwrap_or_default()
    }

    #[test]
    fn first_start_makes_default_from_the_live_files() {
        let d = scratch("first");
        put(d.join("player/pc_options.txt"), "version=1\nlighting=enhanced\n");
        put(d.join("mods/active.txt"), "a\nb\n");
        let p = Profiles::new(&d);
        let id = p.ensure(rac1()).unwrap();
        assert_eq!(id, "default");
        assert_eq!(p.current().as_deref(), Some("default"));
        assert_eq!(p.get("default").unwrap().meta.name, "Default");
        assert_eq!(read(p.options_file("default")), "version=1\nlighting=enhanced\n");
        assert_eq!(read(p.active_file("default")), "a\nb\n");
        // second start changes nothing, even if the live files moved on
        put(d.join("player/pc_options.txt"), "version=1\nlighting=classic\n");
        assert_eq!(p.ensure(rac1()).unwrap(), "default");
        assert_eq!(read(p.options_file("default")), "version=1\nlighting=enhanced\n");
        assert_eq!(p.list().len(), 1);
    }

    #[test]
    fn first_start_without_any_files_works() {
        let d = scratch("nofiles");
        let p = Profiles::new(&d);
        assert_eq!(p.ensure(rac1()).unwrap(), "default");
        assert!(p.options_file("default").is_file() && p.active_file("default").is_file());
    }

    #[test]
    fn create_copy_rename_delete() {
        let d = scratch("crud");
        put(d.join("player/pc_options.txt"), "version=1\nwater=high\n");
        put(d.join("mods/active.txt"), "x\n");
        let p = Profiles::new(&d);
        p.ensure(rac1()).unwrap();
        assert_eq!(p.create("Fast & light").unwrap(), "fast-light");
        assert_eq!(p.create("\u{65e5}\u{672c}").unwrap(), "profile");
        // new from defaults: no water key, no mods
        let id = p.create("Low spec").unwrap();
        assert_eq!(id, "low-spec");
        assert!(!read(p.options_file(&id)).contains("water"));
        assert!(!read(p.active_file(&id)).lines().any(|l| l == "x"));
        // copy keeps settings and mods
        let c = p.copy("default", "Default copy").unwrap();
        assert!(read(p.options_file(&c)).contains("water=high"));
        assert!(read(p.active_file(&c)).lines().any(|l| l == "x"));
        // names: no duplicates, no empty, no path characters
        assert!(p.create("low SPEC").unwrap_err().contains("already"));
        assert!(p.create("   ").is_err());
        assert!(p.create("a/b").is_err());
        assert!(p.create(&"x".repeat(41)).is_err());
        // rename
        p.rename(&id, "Low Spec").unwrap();
        assert_eq!(p.get(&id).unwrap().meta.name, "Low Spec");
        assert!(p.rename(&id, "default").is_err());
        // a folder that already exists is never reused
        assert_eq!(p.create("low-spec").unwrap(), "low-spec-2");
        // delete moves the selection
        p.set_current(&id).unwrap();
        p.delete(&id).unwrap();
        assert!(p.get(&id).is_none());
        assert_eq!(p.current().as_deref(), Some("default"));
        assert!(p.delete("nothing").is_err());
    }

    #[test]
    fn the_last_profile_cannot_be_deleted() {
        let d = scratch("last");
        let p = Profiles::new(&d);
        p.ensure(rac1()).unwrap();
        assert!(p.delete("default").unwrap_err().contains("last"));
        assert_eq!(p.list().len(), 1);
    }

    #[test]
    fn swap_in_then_copy_back() {
        let d = scratch("swap");
        put(d.join("player/pc_options.txt"), "version=1\nlighting=classic\n");
        put(d.join("player/card.bin"), "SAVE");
        put(d.join("mods/active.txt"), "old\n");
        let p = Profiles::new(&d);
        p.ensure(rac1()).unwrap();
        let hi = p.create("High").unwrap();
        put(p.options_file(&hi), "version=1\nlighting=enhanced\nwater=high\n");
        put(p.active_file(&hi), "tex1\ntex2\n");
        p.swap_in(rac1(), &hi).unwrap();
        assert_eq!(read(d.join("player/pc_options.txt")), "version=1\nlighting=enhanced\nwater=high\n");
        assert_eq!(read(d.join("mods/active.txt")), "tex1\ntex2\n");
        assert_eq!(p.pending_marker().as_deref(), Some("high"));
        assert_eq!(p.get(&hi).unwrap().meta.last_used, library::today());
        // the player changes an option in the game's own menu
        put(d.join("player/pc_options.txt"), "version=1\nlighting=enhanced\nwater=low\n");
        assert_eq!(p.copy_back(rac1()).as_deref(), Some("high"));
        assert_eq!(read(p.options_file(&hi)), "version=1\nlighting=enhanced\nwater=low\n");
        assert_eq!(p.pending_marker(), None);
        // the other profile and the saves are untouched
        assert_eq!(read(p.options_file("default")), "version=1\nlighting=classic\n");
        assert_eq!(read(d.join("player/card.bin")), "SAVE");
        // nothing pending: nothing to copy
        assert_eq!(p.copy_back(rac1()), None);
    }

    #[test]
    fn marker_left_behind_is_recovered_at_the_next_start() {
        let d = scratch("marker");
        put(d.join("player/pc_options.txt"), "version=1\n");
        let p = Profiles::new(&d);
        p.ensure(rac1()).unwrap();
        let b = p.create("B").unwrap();
        p.swap_in(rac1(), &b).unwrap();
        // the launcher is closed while the game runs; the player changes an option; a new launcher starts
        put(d.join("player/pc_options.txt"), "version=1\nhud=off\n");
        let p2 = Profiles::new(&d);
        assert_eq!(p2.pending_marker().as_deref(), Some("b"));
        assert_eq!(p2.copy_back(rac1()).as_deref(), Some("b"));
        assert!(read(p2.options_file("b")).contains("hud=off"));
        assert!(!read(p2.options_file("default")).contains("hud=off"));
        assert_eq!(p2.pending_marker(), None);
    }

    #[test]
    fn marker_of_a_deleted_profile_is_dropped_and_a_running_profile_cannot_be_deleted() {
        let d = scratch("gone");
        let p = Profiles::new(&d);
        p.ensure(rac1()).unwrap();
        let b = p.create("B").unwrap();
        p.swap_in(rac1(), &b).unwrap();
        assert!(p.delete(&b).unwrap_err().contains("using"));
        std::fs::remove_dir_all(p.profile_dir(&b)).unwrap();
        put(d.join("player/pc_options.txt"), "version=1\nhud=off\n");
        assert_eq!(p.copy_back(rac1()).as_deref(), Some("b"));
        assert_eq!(p.pending_marker(), None);
        assert!(!read(p.options_file("default")).contains("hud=off"));
    }

    #[test]
    fn broken_current_falls_back() {
        let d = scratch("cur");
        let p = Profiles::new(&d);
        p.ensure(rac1()).unwrap();
        put(p.dir().join("current.txt"), "ghost\n");
        assert_eq!(p.current().as_deref(), Some("default"));
        assert_eq!(p.ensure(rac1()).unwrap(), "default");
        assert_eq!(read(p.dir().join("current.txt")).trim(), "default");
    }
}
