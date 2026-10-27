//! Starting the game (Start-Game / Get-GamePsi of setup.ps1): the game exe with no arguments, working folder = the install root,
//! RAC_NO_UPDATE_CHECK=1 (the launcher does the update check), and the save-card safety copies of Play.cmd.
use crate::games::{installed_iso, GameDef};
use crate::platform;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// card.bin -> card.start1.bin -> card.start2.bin (the newest two starts), as Play.cmd does. Errors are ignored.
pub fn safety_copies(player: &Path) {
    let (c0, c1, c2) = (player.join("card.bin"), player.join("card.start1.bin"), player.join("card.start2.bin"));
    if c1.is_file() {
        let _ = std::fs::copy(&c1, &c2);
    }
    if c0.is_file() {
        let _ = std::fs::copy(&c0, &c1);
    }
}

/// The messages of Get-GamePsi when the game cannot start.
pub fn check_ready(root: &Path, game: &GameDef) -> Result<(), String> {
    if !game.exe_path(root).is_file() {
        return Err("The game is not built yet. Install it first.".to_string());
    }
    let Some(iso) = installed_iso(root) else {
        return Err("The setup information was not found. Use Repair / rebuild in the game's Advanced settings.".to_string());
    };
    if !iso.is_file() {
        return Err(format!("Your disc image was not found at:\n{}\n\nIf you moved it, use Repair / rebuild (Settings, Advanced) to pick it again.", iso.display()));
    }
    Ok(())
}

pub fn start(root: &Path, game: &GameDef) -> Result<Child, String> {
    check_ready(root, game)?;
    let player = root.join("player");
    let _ = std::fs::create_dir_all(&player);
    safety_copies(&player);
    let mut cmd = Command::new(game.exe_path(root));
    cmd.current_dir(root).env("RAC_NO_UPDATE_CHECK", "1");
    platform::hidden(&mut cmd);
    cmd.spawn().map_err(|e| format!("The game could not be started.\n\n{e}"))
}

/// Where an install keeps its saves: the folder Play.cmd gives the game as RAC_PLAYER_DIR. Every release from 0.2.1 to 0.3.1 has
/// `set "RAC_PLAYER_DIR=%~dp0player"`, i.e. `<install>\player` (memory card `card.bin`, safety copies `card.start1.bin` / `card.start2.bin`).
/// Reads the line from the install's own Play.cmd when there is one, so a later version that moves it is followed; else `player`.
pub fn saves_dir(root: &Path) -> PathBuf {
    let text = std::fs::read_to_string(root.join("Play.cmd")).unwrap_or_default();
    root.join(saves_rel(&text))
}

pub fn saves_rel(play_cmd: &str) -> String {
    for line in play_cmd.lines() {
        let l = line.trim().trim_start_matches("set ").trim_matches('"');
        if let Some(v) = l.strip_prefix("RAC_PLAYER_DIR=%~dp0") {
            let v = v.trim_matches('"').trim_end_matches(|c| c == '\\' || c == '/');
            if !v.is_empty() && !v.contains("..") && !v.contains(':') {
                return v.to_string();
            }
        }
    }
    "player".to_string()
}

/// Opens the saves folder in the file manager (made first when it does not exist yet).
pub fn open_saves(root: &Path) -> std::io::Result<PathBuf> {
    let d = saves_dir(root);
    std::fs::create_dir_all(&d)?;
    crate::platform::open_path(&d);
    Ok(d)
}

pub const SAVES_HINT: &str = "Copy card.bin (your memory card) between installs to move your saves. Close the game first.";

/// The game's run log: `log.txt` in the saves folder (the run before it is `log.prev.txt` there).
pub fn log_file(root: &Path) -> PathBuf {
    saves_dir(root).join("log.txt")
}

/// Opens the log folder in the file manager with `log.txt` selected (just the folder before the first run).
pub fn open_logs(root: &Path) -> std::io::Result<PathBuf> {
    let log = log_file(root);
    let d = saves_dir(root);
    std::fs::create_dir_all(&d)?;
    if log.is_file() {
        crate::platform::reveal(&log);
    } else {
        crate::platform::open_path(&d);
    }
    Ok(d)
}

pub const LOG_HINT: &str = "Reporting a problem? Attach log.txt from the log folder (log.prev.txt if you already restarted the game).";
/// Where players can support the project (a link in the sidebar and on the Launcher page; opened only on a click).
pub const SUPPORT_URL: &str = "https://ko-fi.com/asobicodes";
pub const ISSUES_URL: &str = "https://github.com/AsobiCodes/rac-rs/issues";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::games::game_by_id;

    #[test]
    fn card_copies_rotate() {
        let d = std::env::temp_dir().join(format!("racl_card_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("card.bin"), b"A").unwrap();
        safety_copies(&d);
        assert_eq!(std::fs::read(d.join("card.start1.bin")).unwrap(), b"A");
        assert!(!d.join("card.start2.bin").exists());
        std::fs::write(d.join("card.bin"), b"B").unwrap();
        safety_copies(&d);
        assert_eq!(std::fs::read(d.join("card.start1.bin")).unwrap(), b"B");
        assert_eq!(std::fs::read(d.join("card.start2.bin")).unwrap(), b"A");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn not_ready_messages() {
        let d = std::env::temp_dir().join(format!("racl_ready_{}", std::process::id()));
        std::fs::create_dir_all(d.join("build/release")).unwrap();
        let g = game_by_id("rac1").unwrap();
        assert!(check_ready(&d, g).unwrap_err().contains("not built yet"));
        std::fs::write(d.join("build/release/rac-rs.exe"), b"x").unwrap();
        assert!(check_ready(&d, g).unwrap_err().contains("setup information"));
        std::fs::create_dir_all(d.join("generated")).unwrap();
        std::fs::write(d.join("generated/play.cfg"), "iso=Z:\\gone.iso\n").unwrap();
        assert!(check_ready(&d, g).unwrap_err().contains("was not found at"));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn saves_folder_is_player_in_every_version() {
        // the line as found in Play.cmd of 0.2.1, 0.2.2, 0.3.0 and 0.3.1 (checked in those four public releases)
        let line = "set \"RAC_PLAYER_DIR=%~dp0player\"
if not exist \"%RAC_PLAYER_DIR%\" mkdir \"%RAC_PLAYER_DIR%\"
";
        for v in ["0.2.1", "0.2.2", "0.3.0", "0.3.1"] {
            let d = std::env::temp_dir().join(format!("racl_saves_{v}_{}", std::process::id()));
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join("Play.cmd"), format!("@echo off
rem {v}
{line}")).unwrap();
            assert_eq!(saves_dir(&d), d.join("player"), "{v}");
            let _ = std::fs::remove_dir_all(&d);
        }
        assert_eq!(saves_dir(Path::new("X:/none")), Path::new("X:/none").join("player"));
        assert_eq!(saves_rel("set \"RAC_PLAYER_DIR=%~dp0../x\""), "player");
        assert_eq!(saves_rel("set \"RAC_PLAYER_DIR=%~dp0saves/\""), "saves");
    }

    #[test]
    fn log_lives_in_the_saves_folder() {
        let d = std::env::temp_dir().join(format!("racl_log_{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        assert_eq!(log_file(&d), d.join("player").join("log.txt"));
        std::fs::write(d.join("Play.cmd"), "set \"RAC_PLAYER_DIR=%~dp0saves\"\r\n").unwrap();
        assert_eq!(log_file(&d), d.join("saves").join("log.txt"));
        let _ = std::fs::remove_dir_all(&d);
    }
}
