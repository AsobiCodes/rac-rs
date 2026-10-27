//! The library: the list of game installs the launcher knows (id, game, folder, added date).
//!
//! Where the library file lives (`data_dir`): next to the launcher exe when the exe sits inside a game folder (VERSION + setup.ps1 next to it)
//! or a `portable.txt` file is next to it; otherwise `%APPDATA%\RAC-RS Launcher\` (Linux: `$XDG_DATA_HOME/RAC-RS Launcher/`). `launcher.cfg` lives in the same folder.
//! The library only stores paths: removing an entry never deletes files.
use crate::games::{self, GameDef};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const LIBRARY_FILE: &str = "library.json";
pub const PORTABLE_MARKER: &str = "portable.txt";
/// The space a new install needs (game build, toolchain, generated code) in bytes: about 5 GB.
pub const NEEDED_BYTES: u64 = 5 * 1024 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LibEntry {
    pub id: String,
    pub game: String,
    pub folder: String,
    /// `YYYY-MM-DD`.
    pub added: String,
    /// The user put this install on an older game version: no update nag (a plain "Update to <newest>" button stays).
    #[serde(default)]
    pub pinned: bool,
    /// A backup made before an update (`backup.rs`): always pinned, never updated, can be deleted from the Library.
    #[serde(default)]
    pub backup: bool,
    /// The name shown for a backup ("Ratchet & Clank v0.3.1 (backup)"); empty = the game's title.
    #[serde(default)]
    pub name: String,
}

impl LibEntry {
    pub fn path(&self) -> PathBuf {
        PathBuf::from(&self.folder)
    }

    /// What the sidebar and the Library call it.
    pub fn title(&self, game: &GameDef) -> String {
        if self.name.is_empty() {
            game.title.to_string()
        } else {
            self.name.clone()
        }
    }
}

#[derive(Serialize, Deserialize, Default)]
struct FileFormat {
    #[serde(default)]
    installs: Vec<LibEntry>,
}

#[derive(Clone, Debug, Default)]
pub struct Library {
    /// The library.json path.
    pub file: PathBuf,
    pub entries: Vec<LibEntry>,
}

/// Why the launcher keeps its files where it does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataMode {
    /// The exe sits inside a game folder.
    InGameFolder,
    /// `portable.txt` next to the exe.
    Portable,
    /// `%APPDATA%\RAC-RS Launcher`.
    AppData,
}

/// What a valid game folder holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Found {
    pub game: &'static str,
    pub version: String,
    /// The game is built (exe + Play.cmd + generated\play.cfg); false = an unpacked release that is not set up yet.
    pub installed: bool,
}

/// VERSION and setup.ps1 are there: a rac-rs release tree (set up or not).
pub fn is_release_tree(dir: &Path) -> bool {
    dir.join("VERSION").is_file() && dir.join("setup.ps1").is_file()
}

/// The data folder for the launcher exe in `exe_dir` (`appdata` = the %APPDATA% folder).
pub fn data_dir(exe_dir: &Path, appdata: Option<&Path>) -> (PathBuf, DataMode) {
    if is_release_tree(exe_dir) {
        return (exe_dir.to_path_buf(), DataMode::InGameFolder);
    }
    if exe_dir.join(PORTABLE_MARKER).is_file() {
        return (exe_dir.to_path_buf(), DataMode::Portable);
    }
    match appdata {
        Some(a) => (a.join("RAC-RS Launcher"), DataMode::AppData),
        None => (exe_dir.to_path_buf(), DataMode::Portable),
    }
}

/// Looks at a folder for "Add existing install": a release tree whose game is built. The game is detected by its files.
pub fn check_folder(dir: &Path) -> Result<Found, String> {
    if !dir.is_dir() {
        return Err("That folder does not exist.".to_string());
    }
    if !is_release_tree(dir) {
        return Err("This is not a RAC-RS game folder. It must hold the files VERSION and setup.ps1.".to_string());
    }
    let version = games::local_version(dir);
    for g in games::GAMES.iter().filter(|g| g.supported) {
        if games::is_installed(dir, g) {
            return Ok(Found { game: g.id, version, installed: true });
        }
    }
    Err(format!("This folder holds RAC-RS {version}, but the game was not set up here yet (its game files are missing). Use \"New install\" and choose this folder to set it up."))
}

/// Folder compare without touching the disk: case-insensitive, `/` = `\`, no trailing separator.
pub fn norm(p: &Path) -> String {
    let mut s = p.to_string_lossy().replace('/', "\\").to_lowercase();
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s
}

pub fn same_folder(a: &Path, b: &Path) -> bool {
    norm(a) == norm(b)
}

/// `YYYY-MM-DD` of today (UTC; good enough for a list).
pub fn today() -> String {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    date_of(secs)
}

/// `YYYY-MM-DD` of a unix time (civil-from-days).
pub fn date_of(secs: u64) -> String {
    let z = (secs / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// `Documents\RAC-RS\Ratchet & Clank`, the folder a new install suggests.
pub fn default_install_folder() -> PathBuf {
    crate::platform::known_folder(crate::platform::Known::Documents).unwrap_or_else(|| PathBuf::from(".")).join("RAC-RS").join("Ratchet & Clank")
}

/// Free bytes on the drive that holds `dir` (or its nearest existing parent). None = unknown.
pub fn free_space(dir: &Path) -> Option<u64> {
    let mut p = dir.to_path_buf();
    while !p.exists() {
        if !p.pop() {
            return None;
        }
    }
    crate::platform::free_bytes(&p)
}

/// "4.2 GB" / "850 MB" / "5.3 MB" / "340 KB" (small downloads need the decimal: a game zip is only a few MB).
pub fn size_text(bytes: u64) -> String {
    let mb = bytes as f64 / (1024.0 * 1024.0);
    let gb = mb / 1024.0;
    if gb >= 10.0 {
        format!("{gb:.0} GB")
    } else if gb >= 1.0 {
        format!("{gb:.1} GB")
    } else if mb >= 100.0 {
        format!("{mb:.0} MB")
    } else if mb >= 1.0 {
        format!("{mb:.1} MB")
    } else {
        format!("{:.0} KB", bytes as f64 / 1024.0)
    }
}

impl Library {
    /// Reads `<data_dir>\library.json` (missing or broken = empty list).
    pub fn load(data_dir: &Path) -> Library {
        let file = data_dir.join(LIBRARY_FILE);
        let entries = std::fs::read(&file).ok().and_then(|b| serde_json::from_slice::<FileFormat>(&b).ok()).map(|f| f.installs).unwrap_or_default();
        Library { file, entries }
    }

    pub fn save(&self) -> std::io::Result<()> {
        if let Some(d) = self.file.parent() {
            std::fs::create_dir_all(d)?;
        }
        let text = serde_json::to_string_pretty(&FileFormat { installs: self.entries.clone() }).unwrap_or_default();
        let tmp = self.file.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &self.file)
    }

    pub fn get(&self, id: &str) -> Option<&LibEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub fn find_folder(&self, dir: &Path) -> Option<&LibEntry> {
        self.entries.iter().find(|e| same_folder(&e.path(), dir))
    }

    fn new_id(&self, game: &str) -> String {
        let mut n = 1;
        loop {
            let id = if n == 1 { game.to_string() } else { format!("{game}-{n}") };
            if self.get(&id).is_none() {
                return id;
            }
            n += 1;
        }
    }

    /// Adds a folder without looking at it (an entry for the same folder is returned instead of a second one).
    pub fn add(&mut self, game: &GameDef, folder: &Path) -> LibEntry {
        if let Some(e) = self.find_folder(folder) {
            return e.clone();
        }
        let e = LibEntry { id: self.new_id(game.id), game: game.id.to_string(), folder: folder.to_string_lossy().to_string(), added: today(), pinned: false, backup: false, name: String::new() };
        self.entries.push(e.clone());
        e
    }

    /// Lists a finished backup folder: pinned to its version, flagged as a backup, named. The folder is never listed twice.
    pub fn add_backup(&mut self, game: &GameDef, folder: &Path, name: &str) -> LibEntry {
        if let Some(e) = self.find_folder(folder) {
            return e.clone();
        }
        let e = LibEntry { id: self.new_id(game.id), game: game.id.to_string(), folder: folder.to_string_lossy().to_string(), added: today(), pinned: true, backup: true, name: name.to_string() };
        self.entries.push(e.clone());
        e
    }

    /// Marks an install as pinned to its (older) version, or clears it. True when the entry exists.
    pub fn set_pinned(&mut self, id: &str, pinned: bool) -> bool {
        match self.entries.iter_mut().find(|e| e.id == id) {
            Some(e) => {
                e.pinned = pinned;
                true
            }
            None => false,
        }
    }

    /// "Add existing install": the folder must be a built game install. Adding the same folder twice is refused.
    pub fn add_existing(&mut self, folder: &Path) -> Result<LibEntry, String> {
        let found = check_folder(folder)?;
        if let Some(e) = self.find_folder(folder) {
            return Err(format!("This install is already in your list (as \"{}\").", e.id));
        }
        let game = games::game_by_id(found.game).ok_or("Unknown game.")?;
        Ok(self.add(game, folder))
    }

    /// A launcher that sits inside a game folder adds that install by itself.
    pub fn auto_add(&mut self, exe_dir: &Path) -> Option<LibEntry> {
        if !is_release_tree(exe_dir) || self.find_folder(exe_dir).is_some() {
            return None;
        }
        let game = games::game_by_id("rac1")?;
        Some(self.add(game, exe_dir))
    }

    /// "Remove from list": the entry goes, the files stay. True when it was there.
    pub fn remove(&mut self, id: &str) -> bool {
        let n = self.entries.len();
        self.entries.retain(|e| e.id != id);
        self.entries.len() != n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_lib_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A built install in `dir`.
    pub fn make_install(dir: &Path, version: &str) {
        for (f, t) in [("VERSION", format!("{version}\n")), ("setup.ps1", "x".into()), ("Play.cmd", "x".into()), ("build/release/rac-rs.exe", "x".into()), ("generated/play.cfg", "iso=C:\\none.iso\n".into())] {
            let p = dir.join(f);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, t).unwrap();
        }
    }

    #[test]
    fn dates_are_right() {
        assert_eq!(date_of(0), "1970-01-01");
        assert_eq!(date_of(1_000_000_000), "2001-09-09");
        assert_eq!(date_of(1_792_000_000), "2026-10-14");
        assert_eq!(date_of(951_782_400), "2000-02-29");
    }

    #[test]
    fn data_dir_rules() {
        let exe = scratch("dd_exe");
        let app = scratch("dd_app");
        // plain folder: AppData
        assert_eq!(data_dir(&exe, Some(&app)), (app.join("RAC-RS Launcher"), DataMode::AppData));
        // portable.txt
        std::fs::write(exe.join("portable.txt"), "").unwrap();
        assert_eq!(data_dir(&exe, Some(&app)), (exe.clone(), DataMode::Portable));
        // inside a game folder
        let game = scratch("dd_game");
        make_install(&game, "0.3.1");
        assert_eq!(data_dir(&game, Some(&app)), (game.clone(), DataMode::InGameFolder));
    }

    #[test]
    fn library_file_round_trip_and_broken_file() {
        let d = scratch("rt");
        let mut lib = Library::load(&d);
        assert!(lib.entries.is_empty());
        let g = games::game_by_id("rac1").unwrap();
        let e = lib.add(g, Path::new("D:\\Games\\RAC1"));
        assert_eq!(e.id, "rac1");
        lib.save().unwrap();
        let again = Library::load(&d);
        assert_eq!(again.entries, lib.entries);
        std::fs::write(d.join(LIBRARY_FILE), "{ not json").unwrap();
        assert!(Library::load(&d).entries.is_empty());
    }

    #[test]
    fn add_existing_validates_and_refuses_duplicates() {
        let d = scratch("add");
        let mut lib = Library::load(&d);
        let inst = scratch("add_inst");
        make_install(&inst, "0.4.0");
        let e = lib.add_existing(&inst).unwrap();
        assert_eq!((e.game.as_str(), e.id.as_str()), ("rac1", "rac1"));
        // same folder, other spelling
        let other = PathBuf::from(inst.to_string_lossy().to_uppercase().replace('\\', "/") + "/");
        assert!(lib.add_existing(&other).unwrap_err().contains("already"));
        // a second install gets another id
        let inst2 = scratch("add_inst2");
        make_install(&inst2, "0.3.1");
        assert_eq!(lib.add_existing(&inst2).unwrap().id, "rac1-2");
        // errors
        assert!(lib.add_existing(&d.join("nothing")).unwrap_err().contains("does not exist"));
        assert!(lib.add_existing(&d).unwrap_err().contains("not a RAC-RS game folder"));
        let unbuilt = scratch("add_unbuilt");
        std::fs::write(unbuilt.join("VERSION"), "0.4.0\n").unwrap();
        std::fs::write(unbuilt.join("setup.ps1"), "x").unwrap();
        assert!(lib.add_existing(&unbuilt).unwrap_err().contains("not set up"));
        assert_eq!(lib.entries.len(), 2);
    }

    #[test]
    fn remove_keeps_the_files() {
        let d = scratch("rm");
        let mut lib = Library::load(&d);
        let inst = scratch("rm_inst");
        make_install(&inst, "0.4.0");
        let e = lib.add_existing(&inst).unwrap();
        assert!(lib.remove(&e.id));
        assert!(!lib.remove(&e.id));
        assert!(lib.entries.is_empty());
        assert!(inst.join("VERSION").is_file() && inst.join("build/release/rac-rs.exe").is_file());
    }

    #[test]
    fn launcher_in_a_game_folder_adds_it_once() {
        let d = scratch("auto");
        let mut lib = Library::load(&d);
        assert!(lib.auto_add(&d).is_none(), "not a game folder");
        let inst = scratch("auto_inst");
        make_install(&inst, "0.4.0");
        assert!(lib.auto_add(&inst).is_some());
        assert!(lib.auto_add(&inst).is_none());
        assert_eq!(lib.entries.len(), 1);
    }

    #[test]
    fn a_backup_entry_is_pinned_named_flagged_and_survives_the_file() {
        let d = scratch("bakentry");
        let mut lib = Library::load(&d);
        let g = games::game_by_id("rac1").unwrap();
        let main = lib.add(g, Path::new("D:\\Games\\RAC1"));
        assert!(!main.backup && !main.pinned && main.title(g) == "Ratchet & Clank");
        let b = lib.add_backup(g, Path::new("D:\\Games\\RAC1 (backup v0.3.1)"), "RAC1 v0.3.1 (backup)");
        assert!(b.backup && b.pinned);
        assert_eq!(b.title(g), "RAC1 v0.3.1 (backup)");
        assert_ne!(b.id, main.id);
        // the same folder again gives the same entry
        assert_eq!(lib.add_backup(g, Path::new("d:/games/rac1 (backup v0.3.1)"), "other").id, b.id);
        lib.save().unwrap();
        let again = Library::load(&d);
        assert_eq!(again.entries, lib.entries);
        assert!(again.get(&b.id).unwrap().backup);
        // an old library.json without the new fields still loads
        std::fs::write(d.join(LIBRARY_FILE), r#"{"installs":[{"id":"rac1","game":"rac1","folder":"X","added":"2026-10-01","pinned":true}]}"#).unwrap();
        let old = Library::load(&d);
        assert!(!old.entries[0].backup && old.entries[0].name.is_empty() && old.entries[0].pinned);
    }

    #[test]
    fn size_text_reads_well() {
        assert_eq!(size_text(5 * 1024 * 1024 * 1024), "5.0 GB");
        assert_eq!(size_text(300 * 1024 * 1024), "300 MB");
        assert_eq!(size_text(5 * 1024 * 1024 + 300_000), "5.3 MB");
        assert_eq!(size_text(340 * 1024), "340 KB");
        assert_eq!(size_text(120 * 1024 * 1024 * 1024), "120 GB");
    }
}
