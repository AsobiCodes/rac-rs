//! The operating-system calls of the Linux launcher.
//!
//! The Windows implementations (named mutex, PowerShell, known folders, .lnk
//! shortcuts, free space through `GetDiskFreeSpaceExW`) are archived in
//! `windows/platform_windows.rs`; nothing here uses them.
use std::path::{Path, PathBuf};
use std::process::Command;

/// Keeps the single-instance lock (an open, locked file) while it lives.
pub struct InstanceGuard {
    _lock: std::fs::File,
}

/// Locks `<XDG_RUNTIME_DIR>/rac-rs-launcher-<uid>.lock`; None = another launcher of this user is already open.
pub fn single_instance(name: &str) -> Option<InstanceGuard> {
    let _ = name;
    use std::os::fd::AsRawFd;
    let dir = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|p| p.is_dir()).unwrap_or_else(std::env::temp_dir);
    let uid = unsafe { libc::geteuid() };
    let path = dir.join(format!("rac-rs-launcher-{uid}.lock"));
    let file = std::fs::OpenOptions::new().create(true).write(true).truncate(false).open(&path).ok()?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        return None;
    }
    Some(InstanceGuard { _lock: file })
}

/// A child process: there is no console window to hide on Linux.
pub fn hidden(cmd: &mut Command) -> &mut Command {
    cmd
}

/// Kills the child and everything it started (the process group when the caller made one).
pub fn kill_tree(pid: u32) {
    let _ = Command::new("kill").args(["-9", &format!("-{pid}")]).status();
    let _ = Command::new("kill").args(["-9", &pid.to_string()]).status();
}

/// Opens a folder or a file with the system's default program.
pub fn open_path(p: &Path) {
    let _ = Command::new("xdg-open").arg(p).spawn();
}

/// Opens a web address in the default browser (only when the player clicked a link; never called by itself).
pub fn open_url(url: &str) {
    let _ = Command::new("xdg-open").arg(url).spawn();
}

/// Opens the folder of a file and selects it (plain open on Linux).
pub fn reveal(p: &Path) {
    open_path(p.parent().unwrap_or(p));
}

#[derive(Clone, Copy)]
pub enum Known {
    Documents,
    Desktop,
}

/// The user's Documents or Desktop folder, following `~/.config/user-dirs.dirs`.
pub fn known_folder(k: Known) -> Option<PathBuf> {
    let (key, fallback) = match k {
        Known::Documents => ("DOCUMENTS", "Documents"),
        Known::Desktop => ("DESKTOP", "Desktop"),
    };
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    let cfg = home.join(".config").join("user-dirs.dirs");
    if let Ok(text) = std::fs::read_to_string(&cfg) {
        let prefix = format!("XDG_{key}_DIR=");
        for line in text.lines() {
            let line = line.trim();
            let Some(v) = line.strip_prefix(&prefix) else { continue };
            let v = v.trim().trim_matches('"').replace("$HOME", &home.to_string_lossy());
            if !v.is_empty() {
                return Some(PathBuf::from(v));
            }
        }
    }
    Some(home.join(fallback))
}

/// The per-user data base folder: `$XDG_DATA_HOME` or `~/.local/share`.
pub fn data_base() -> Option<PathBuf> {
    let nonempty = |v: Option<std::ffi::OsString>| v.filter(|s| !s.is_empty()).map(PathBuf::from);
    nonempty(std::env::var_os("XDG_DATA_HOME"))
        .filter(|p| p.has_root())
        .or_else(|| nonempty(std::env::var_os("HOME")).map(|h| h.join(".local").join("share")))
}

/// Where PCSX2 keeps texture replacements on Linux: the native build and the Flatpak.
pub fn pcsx2_texture_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(cfg) = data_config_dir() {
        v.push(cfg.join("PCSX2").join("textures"));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        v.push(home.join(".var/app/net.pcsx2.PCSX2/config/PCSX2/textures"));
    }
    v
}

/// Where PCSX2 keeps the covers it downloaded on Linux.
pub fn pcsx2_covers_dir() -> Option<PathBuf> {
    data_config_dir().map(|d| d.join("PCSX2").join("covers"))
}

/// `$XDG_CONFIG_HOME` or `~/.config`.
fn data_config_dir() -> Option<PathBuf> {
    let nonempty = |v: Option<std::ffi::OsString>| v.filter(|s| !s.is_empty()).map(PathBuf::from);
    nonempty(std::env::var_os("XDG_CONFIG_HOME"))
        .filter(|p| p.has_root())
        .or_else(|| nonempty(std::env::var_os("HOME")).map(|h| h.join(".config")))
}

/// Free bytes for the current user on the file system that holds `dir`. None = unknown.
pub fn free_bytes(dir: &Path) -> Option<u64> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    let ok = unsafe { libc::statvfs(c.as_ptr(), &mut st) };
    (ok == 0).then_some(st.f_bavail as u64 * st.f_frsize as u64)
}

/// Tests only: true makes `recycle` delete its (scratch) path for good, so UI tests never fill the user's trash.
pub static TEST_NO_RECYCLE_BIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// To the trash, never a hard delete (except under `TEST_NO_RECYCLE_BIN`).
pub fn recycle(p: &Path) -> Result<(), String> {
    if TEST_NO_RECYCLE_BIN.load(std::sync::atomic::Ordering::SeqCst) {
        let r = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
        return r.map_err(|e| e.to_string());
    }
    trash::delete(p).map_err(|e| e.to_string())
}

/// A running binary can be replaced on Linux, so the game can always be updated.
pub fn exe_in_use(_exe: &Path) -> bool {
    false
}

/// Quotes one argument for a `.desktop` `Exec=` line.
fn exec_quote(p: &Path) -> String {
    let s = p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"").replace('`', "\\`").replace('$', "\\$");
    format!("\"{s}\"")
}

/// Installs the launcher icon under `$XDG_DATA_HOME/icons` and returns its path.
fn install_icon() -> Result<PathBuf, String> {
    let dir = data_base().ok_or("The data folder was not found.")?.join("icons");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let p = dir.join("rac-rs-launcher.png");
    if !p.is_file() {
        std::fs::write(&p, include_bytes!("../assets/rac-rs-256.png")).map_err(|e| e.to_string())?;
    }
    Ok(p)
}

/// A desktop entry in `dir` that opens `target` (the launcher), working folder `workdir`.
pub fn create_shortcut(dir: &Path, name: &str, target: &Path, workdir: &Path, icon: Option<&Path>) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let icon = match icon {
        Some(p) if p.is_file() => p.to_path_buf(),
        _ => install_icon()?,
    };
    let file = dir.join(format!("{name}.desktop"));
    let text = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Version=1.0\n\
         Name={name}\n\
         Comment=Install, play and configure the Ratchet & Clank PC port\n\
         Exec={}\n\
         Path={}\n\
         Icon={}\n\
         Terminal=false\n\
         Categories=Game;\n",
        exec_quote(target),
        workdir.display(),
        icon.display(),
    );
    std::fs::write(&file, text).map_err(|e| e.to_string())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755));
    }
    Ok(file)
}

/// "Debian GNU/Linux 12 (bookworm)" style text for the problem report.
pub fn os_text() -> String {
    if let Ok(t) = std::fs::read_to_string("/etc/os-release") {
        for line in t.lines() {
            if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
                let v = v.trim().trim_matches('"');
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    std::env::consts::OS.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_base_follows_the_platform() {
        // The XDG path is what this launcher uses on Linux.
        assert_eq!(data_base_from(Some("/data"), Some("/home/a")), Some(PathBuf::from("/data")));
        assert_eq!(data_base_from(None, Some("/home/a")), Some(PathBuf::from("/home/a/.local/share")));
    }

    fn data_base_from(xdg: Option<&str>, home: Option<&str>) -> Option<PathBuf> {
        let nonempty = |v: Option<&str>| v.filter(|s| !s.is_empty()).map(PathBuf::from);
        nonempty(xdg).filter(|p| p.has_root()).or_else(|| nonempty(home).map(|h| h.join(".local").join("share")))
    }

    #[test]
    fn desktop_entry_is_written_and_executable() {
        use std::os::unix::fs::PermissionsExt;
        let d = std::env::temp_dir().join(format!("racl_desktop_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let target = Path::new("/opt/RAC-RS Launcher/rac-rs-launcher");
        let icon = d.join("icon.png");
        std::fs::write(&icon, b"png").unwrap();
        let p = create_shortcut(&d, "Ratchet & Clank (rac-rs)", target, Path::new("/games/rac"), Some(&icon)).unwrap();
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("Type=Application"));
        assert!(text.contains("Exec=\"/opt/RAC-RS Launcher/rac-rs-launcher\""));
        assert!(text.contains("Path=/games/rac"));
        assert!(text.contains(&format!("Icon={}", icon.display())));
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o755);
        let _ = std::fs::remove_dir_all(&d);
    }
}
