//! The game table (one entry: Ratchet & Clank) and the install state of a game folder.
use std::path::{Path, PathBuf};

/// A supported disc image (v1.00 of each region), values as in `setup.sh`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DiscDef {
    pub serial: &'static str,
    pub region: &'static str,
    pub name: &'static str,
    pub size: u64,
    pub sha1: &'static str,
}

#[derive(Clone, Copy, Debug)]
pub struct GameDef {
    pub id: &'static str,
    pub title: &'static str,
    pub short: &'static str,
    pub discs: &'static [DiscDef],
    /// Relative to the install root.
    pub exe: &'static str,
    pub options: &'static str,
    pub mods: &'static str,
    pub visible: bool,
    /// false = "Coming soon" (nothing to install yet).
    pub supported: bool,
    pub tagline: &'static str,
}

pub const RAC1_DISCS: [DiscDef; 2] = [
    DiscDef { serial: "SCES-50916", region: "PAL / Europe", name: "PAL / Europe v1.00 (SCES-50916)", size: 4_213_997_568, sha1: "B0DFBD8EAC0D84F173B6FC25D8A02A5FF22C652F" },
    DiscDef { serial: "SCUS-97199", region: "USA", name: "USA v1.00 (SCUS-97199)", size: 4_214_095_872, sha1: "AEE9275384F739C9B24CD5DE6F7E200292DB7B01" },
];

pub const GAMES: &[GameDef] = &[
    GameDef {
        id: "rac1",
        title: "Ratchet & Clank",
        short: "R&C 1",
        discs: &RAC1_DISCS,
        exe: "build/release/rac-rs",
        options: "player/pc_options.txt",
        mods: "mods",
        visible: true,
        supported: true,
        tagline: "The 2002 original, running the retail game code from your own disc.",
    },
];

impl GameDef {
    pub fn disc_by_size(&self, len: u64) -> Option<&'static DiscDef> {
        self.discs.iter().find(|d| d.size == len)
    }

    pub fn disc_by_serial(&self, serial: &str) -> Option<&'static DiscDef> {
        self.discs.iter().find(|d| d.serial.eq_ignore_ascii_case(serial))
    }

    pub fn exe_path(&self, root: &Path) -> PathBuf {
        root.join(self.exe)
    }

    pub fn options_path(&self, root: &Path) -> PathBuf {
        root.join(self.options)
    }

    pub fn mods_dir(&self, root: &Path) -> PathBuf {
        root.join(self.mods)
    }
}

pub fn visible_games() -> impl Iterator<Item = &'static GameDef> {
    GAMES.iter().filter(|g| g.visible)
}

pub fn game_by_id(id: &str) -> Option<&'static GameDef> {
    GAMES.iter().find(|g| g.id == id)
}

/// What the install root holds for one game (a snapshot; `detect` reads the disk).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstallInfo {
    pub installed: bool,
    pub local_version: String,
    /// The version the game was built from (generated/built_version.txt; "0.1.0" when installed before versions existed; "" = not built).
    pub built_version: String,
    /// The disc image path setup wrote into generated/play.cfg.
    pub iso: Option<PathBuf>,
    pub iso_exists: bool,
    pub disc: Option<&'static DiscDef>,
}

impl InstallInfo {
    /// "Finish the update": the files are newer than the build.
    pub fn finish_pending(&self) -> bool {
        self.installed && !self.built_version.is_empty() && self.built_version != self.local_version
    }
}

fn read_trim(p: &Path) -> Option<String> {
    std::fs::read(p).ok().map(|b| String::from_utf8_lossy(&b).trim().trim_start_matches('\u{feff}').to_string())
}

pub fn is_version(s: &str) -> bool {
    let p: Vec<&str> = s.split('.').collect();
    p.len() == 3 && p.iter().all(|x| !x.is_empty() && x.bytes().all(|b| b.is_ascii_digit()))
}

/// VERSION of the install (missing = 0.1.0, the first public release).
pub fn local_version(root: &Path) -> String {
    read_trim(&root.join("VERSION")).filter(|v| is_version(v)).unwrap_or_else(|| "0.1.0".to_string())
}

/// Setup's own test (Test-Installed): the game binary, play.sh and generated/play.cfg exist.
pub fn is_installed(root: &Path, game: &GameDef) -> bool {
    game.exe_path(root).is_file() && root.join("play.sh").is_file() && root.join("generated").join("play.cfg").is_file()
}

/// `iso=` of generated/play.cfg, the disc image setup used.
pub fn installed_iso(root: &Path) -> Option<PathBuf> {
    let t = read_trim(&root.join("generated").join("play.cfg"))?;
    for l in t.lines() {
        if let Some(v) = l.strip_prefix("iso=") {
            return Some(PathBuf::from(v.trim()));
        }
    }
    None
}

pub fn detect(root: &Path, game: &GameDef) -> InstallInfo {
    let installed = is_installed(root, game);
    let built = read_trim(&root.join("generated").join("built_version.txt"))
        .filter(|v| is_version(v))
        .unwrap_or_else(|| if installed { "0.1.0".to_string() } else { String::new() });
    let iso = installed_iso(root);
    let meta = iso.as_ref().and_then(|p| std::fs::metadata(p).ok()).filter(|m| m.is_file());
    InstallInfo {
        installed,
        local_version: local_version(root),
        built_version: built,
        iso_exists: meta.is_some(),
        disc: meta.and_then(|m| game.disc_by_size(m.len())),
        iso,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("racl_games_{}_{}", name, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn table_has_the_one_game() {
        let v: Vec<_> = visible_games().map(|g| g.id).collect();
        assert_eq!(v, vec!["rac1"]);
        assert!(game_by_id("rac1").unwrap().supported);
        assert_eq!(RAC1_DISCS[0].sha1.len(), 40);
        assert_eq!(game_by_id("rac1").unwrap().disc_by_size(4_214_095_872).unwrap().serial, "SCUS-97199");
    }

    #[test]
    fn nothing_installed_in_an_empty_folder() {
        let d = scratch("empty");
        let i = detect(&d, game_by_id("rac1").unwrap());
        assert!(!i.installed && !i.finish_pending());
        assert_eq!(i.local_version, "0.1.0");
        assert_eq!(i.built_version, "");
    }

    #[test]
    fn installed_install_with_play_cfg_and_versions() {
        let d = scratch("inst");
        let g = game_by_id("rac1").unwrap();
        std::fs::create_dir_all(d.join("build/release")).unwrap();
        std::fs::create_dir_all(d.join("generated")).unwrap();
        std::fs::write(d.join("build/release/rac-rs"), b"x").unwrap();
        std::fs::write(d.join("play.sh"), b"x").unwrap();
        std::fs::write(d.join("generated/play.cfg"), "iso=/games/rc.iso\n").unwrap();
        std::fs::write(d.join("VERSION"), "0.4.0\n").unwrap();
        let i = detect(&d, g);
        assert!(i.installed);
        assert_eq!(i.built_version, "0.1.0", "no built_version.txt: built before versions existed");
        assert!(i.finish_pending());
        assert_eq!(i.iso.as_deref(), Some(Path::new("/games/rc.iso")));
        assert!(!i.iso_exists);
        std::fs::write(d.join("generated/built_version.txt"), "0.4.0\n").unwrap();
        assert!(!detect(&d, g).finish_pending());
    }

    #[test]
    fn version_shape() {
        assert!(is_version("0.3.1") && !is_version("0.3") && !is_version("v0.3.1") && !is_version("1..2"));
    }
}
