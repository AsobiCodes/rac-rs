//! `launcher.cfg` in the install root: window size, last page, the "minimize while playing" setting. Plain `key=value` lines.
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct LauncherCfg {
    pub width: f32,
    pub height: f32,
    /// Id of the last game page ("rac1").
    pub game: String,
    /// Library id of the install that was open last.
    pub install: String,
    /// Last tab of the game page: play, settings, mods.
    pub tab: String,
    pub minimize_while_playing: bool,
    /// Where the disc picker started last time.
    pub last_iso_dir: String,
    /// The "Back up your current version first" tick of an update (default on).
    pub backup_before_update: bool,
}

impl Default for LauncherCfg {
    fn default() -> Self {
        LauncherCfg { width: 1000.0, height: 640.0, game: "rac1".into(), install: String::new(), tab: "play".into(), minimize_while_playing: false, last_iso_dir: String::new(), backup_before_update: true }
    }
}

pub fn path(root: &Path) -> PathBuf {
    root.join("launcher.cfg")
}

impl LauncherCfg {
    pub fn parse(text: &str) -> LauncherCfg {
        let mut c = LauncherCfg::default();
        for l in text.lines() {
            let Some((k, v)) = l.trim_start_matches('\u{feff}').split_once('=') else { continue };
            let v = v.trim();
            match k.trim() {
                "width" => c.width = v.parse().ok().filter(|w: &f32| (600.0..=8000.0).contains(w)).unwrap_or(c.width),
                "height" => c.height = v.parse().ok().filter(|h: &f32| (400.0..=6000.0).contains(h)).unwrap_or(c.height),
                "game" if !v.is_empty() => c.game = v.to_string(),
                "install" => c.install = v.to_string(),
                "tab" if matches!(v, "play" | "settings" | "mods") => c.tab = v.to_string(),
                "minimize_while_playing" => c.minimize_while_playing = matches!(v, "1" | "on" | "true" | "yes"),
                "last_iso_dir" => c.last_iso_dir = v.to_string(),
                "backup_before_update" => c.backup_before_update = !matches!(v, "0" | "off" | "false" | "no"),
                _ => {}
            }
        }
        c
    }

    pub fn to_text(&self) -> String {
        format!(
            "width={}\nheight={}\ngame={}\ninstall={}\ntab={}\nminimize_while_playing={}\nlast_iso_dir={}\nbackup_before_update={}\n",
            self.width.round(),
            self.height.round(),
            self.game,
            self.install,
            self.tab,
            if self.minimize_while_playing { "on" } else { "off" },
            self.last_iso_dir,
            if self.backup_before_update { "on" } else { "off" }
        )
    }

    pub fn load(root: &Path) -> LauncherCfg {
        std::fs::read(path(root)).map(|b| LauncherCfg::parse(&String::from_utf8_lossy(&b))).unwrap_or_default()
    }

    pub fn store(&self, root: &Path) {
        let _ = std::fs::write(path(root), self.to_text());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_defaults() {
        let c = LauncherCfg { width: 1111.0, height: 700.0, game: "rac2".into(), install: "rac1-2".into(), tab: "mods".into(), minimize_while_playing: true, last_iso_dir: "D:\\isos".into(), backup_before_update: false };
        assert_eq!(LauncherCfg::parse(&c.to_text()), c);
        assert_eq!(LauncherCfg::parse(""), LauncherCfg::default());
    }

    #[test]
    fn the_backup_tick_defaults_on_and_is_remembered() {
        assert!(LauncherCfg::default().backup_before_update);
        assert!(LauncherCfg::parse("width=900\n").backup_before_update, "an old launcher.cfg keeps the default");
        let off = LauncherCfg { backup_before_update: false, ..LauncherCfg::default() };
        assert!(off.to_text().contains("backup_before_update=off"));
        assert!(!LauncherCfg::parse(&off.to_text()).backup_before_update);
    }

    #[test]
    fn nonsense_is_ignored() {
        let c = LauncherCfg::parse("width=5\nheight=abc\ntab=zzz\nunknown=1\ngame=\n");
        assert_eq!(c, LauncherCfg::default());
    }
}
