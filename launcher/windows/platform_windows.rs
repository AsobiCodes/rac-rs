//! The few operating-system calls (Windows-only ones behind cfg(windows), with a plain fallback elsewhere).
use std::path::{Path, PathBuf};
use std::process::Command;

/// Keeps the single-instance lock while it lives.
pub struct InstanceGuard {
    #[cfg(windows)]
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for InstanceGuard {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

/// Named mutex `Local\RacRsLauncher`; None = another launcher of this user is already open.
#[cfg(windows)]
pub fn single_instance(name: &str) -> Option<InstanceGuard> {
    use windows_sys::Win32::Foundation::{GetLastError, ERROR_ALREADY_EXISTS};
    use windows_sys::Win32::System::Threading::CreateMutexW;
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let h = CreateMutexW(std::ptr::null(), 0, wide.as_ptr());
        if h.is_null() {
            return None;
        }
        if GetLastError() == ERROR_ALREADY_EXISTS {
            windows_sys::Win32::Foundation::CloseHandle(h);
            return None;
        }
        Some(InstanceGuard { handle: h })
    }
}

#[cfg(not(windows))]
pub fn single_instance(name: &str) -> Option<InstanceGuard> {
    // Native Linux later: a lock file in the runtime dir. For now (Proton runs the Windows exe) there is no second instance check.
    let _ = name;
    Some(InstanceGuard {})
}

/// A hidden child: no console window.
pub fn hidden(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd
}

pub fn powershell_exe() -> PathBuf {
    match std::env::var_os("SystemRoot") {
        Some(r) => Path::new(&r).join("System32").join("WindowsPowerShell").join("v1.0").join("powershell.exe"),
        None => PathBuf::from("powershell.exe"),
    }
}

/// Kills the child and everything it started (exactly that PID's tree, as the old window did).
pub fn kill_tree(pid: u32) {
    #[cfg(windows)]
    {
        let tk = match std::env::var_os("SystemRoot") {
            Some(r) => Path::new(&r).join("System32").join("taskkill.exe"),
            None => PathBuf::from("taskkill.exe"),
        };
        let _ = hidden(Command::new(tk).args(["/PID", &pid.to_string(), "/T", "/F"])).status();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("kill").arg("-9").arg(pid.to_string()).status();
    }
}

/// Opens a folder or a file with the system's default program.
pub fn open_path(p: &Path) {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer.exe").arg(p).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("xdg-open").arg(p).spawn();
    }
}

/// Opens a web address in the default browser (only when the player clicked a link; never called by itself).
pub fn open_url(url: &str) {
    #[cfg(windows)]
    {
        let _ = Command::new("explorer.exe").arg(url).spawn();
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("xdg-open").arg(url).spawn();
    }
}

/// Opens the folder of a file and selects it (plain open on other systems).
pub fn reveal(p: &Path) {
    #[cfg(windows)]
    {
        let mut a = std::ffi::OsString::from("/select,");
        a.push(p.as_os_str());
        let _ = Command::new("explorer.exe").arg(a).spawn();
    }
    #[cfg(not(windows))]
    {
        open_path(p.parent().unwrap_or(p));
    }
}

#[derive(Clone, Copy)]
pub enum Known {
    Documents,
    Desktop,
}

/// Documents / Desktop of the user (follows OneDrive redirects through the shell), with an environment fallback.
pub fn known_folder(k: Known) -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Com::CoTaskMemFree;
        use windows_sys::Win32::UI::Shell::{FOLDERID_Desktop, FOLDERID_Documents, SHGetKnownFolderPath};
        let id = match k {
            Known::Documents => &FOLDERID_Documents,
            Known::Desktop => &FOLDERID_Desktop,
        };
        unsafe {
            let mut out: *mut u16 = std::ptr::null_mut();
            let hr = SHGetKnownFolderPath(id, 0, std::ptr::null_mut(), &mut out);
            if hr >= 0 && !out.is_null() {
                let mut n = 0;
                while *out.add(n) != 0 {
                    n += 1;
                }
                let s = String::from_utf16_lossy(std::slice::from_raw_parts(out, n));
                CoTaskMemFree(out as *const _);
                return Some(PathBuf::from(s));
            }
        }
    }
    let home = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME"))?;
    Some(Path::new(&home).join(match k {
        Known::Documents => "Documents",
        Known::Desktop => "Desktop",
    }))
}

/// The per-user data base folder: `%APPDATA%` on Windows; on Linux `$XDG_DATA_HOME` or `~/.local/share` (XDG base directories).
pub fn data_base() -> Option<PathBuf> {
    pick_data_base(cfg!(windows), std::env::var_os("APPDATA"), std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

fn pick_data_base(windows: bool, appdata: Option<std::ffi::OsString>, xdg: Option<std::ffi::OsString>, home: Option<std::ffi::OsString>) -> Option<PathBuf> {
    let nonempty = |v: Option<std::ffi::OsString>| v.filter(|s| !s.is_empty()).map(PathBuf::from);
    if windows {
        return nonempty(appdata);
    }
    nonempty(xdg).filter(|p| p.has_root()).or_else(|| nonempty(home).map(|h| h.join(".local").join("share")))
}

/// Free bytes for the current user on the drive that holds `dir` (an existing folder). None = unknown.
pub fn free_bytes(dir: &Path) -> Option<u64> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
        let wide: Vec<u16> = dir.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let (mut free, mut total, mut total_free) = (0u64, 0u64, 0u64);
        let ok = unsafe { GetDiskFreeSpaceExW(wide.as_ptr(), &mut free, &mut total, &mut total_free) };
        return (ok != 0).then_some(free);
    }
    #[cfg(not(windows))]
    {
        let _ = dir;
        None
    }
}

/// Tests only: true makes `recycle` delete its (scratch) path for good, so UI tests never fill the user's Recycle Bin.
pub static TEST_NO_RECYCLE_BIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// To the Recycle Bin, never a hard delete (except under `TEST_NO_RECYCLE_BIN`).
pub fn recycle(p: &Path) -> Result<(), String> {
    if TEST_NO_RECYCLE_BIN.load(std::sync::atomic::Ordering::SeqCst) {
        let r = if p.is_dir() { std::fs::remove_dir_all(p) } else { std::fs::remove_file(p) };
        return r.map_err(|e| e.to_string());
    }
    trash::delete(p).map_err(|e| e.to_string())
}

/// A running exe cannot be opened for writing on Windows: true = the game is running (also when another launcher started it).
pub fn exe_in_use(exe: &Path) -> bool {
    #[cfg(windows)]
    {
        if !exe.is_file() {
            return false;
        }
        match std::fs::OpenOptions::new().write(true).open(exe) {
            Ok(_) => false,
            Err(e) => matches!(e.raw_os_error(), Some(5) | Some(32)),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = exe;
        false
    }
}

/// A desktop shortcut `<dir>\<name>.lnk` to `target` (the launcher), working folder `workdir`, with the icon.
pub fn create_shortcut(dir: &Path, name: &str, target: &Path, workdir: &Path, icon: Option<&Path>) -> Result<PathBuf, String> {
    #[cfg(windows)]
    {
        let mut sl = mslnk::ShellLink::new(target).map_err(|e| e.to_string())?;
        sl.set_working_dir(Some(workdir.to_string_lossy().to_string()));
        sl.set_name(Some(name.to_string()));
        let ico = icon.map(|p| p.to_path_buf()).unwrap_or_else(|| target.to_path_buf());
        sl.set_icon_location(Some(ico.to_string_lossy().to_string()));
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let lnk = dir.join(format!("{name}.lnk"));
        sl.create_lnk(&lnk).map_err(|e| e.to_string())?;
        Ok(lnk)
    }
    #[cfg(not(windows))]
    {
        let _ = (dir, name, target, workdir, icon);
        Err("Desktop shortcuts are only made on Windows.".to_string())
    }
}

/// "Windows 10.0.26200" style text for the problem report.
pub fn os_text() -> String {
    #[cfg(windows)]
    {
        if let Ok(o) = hidden(Command::new("cmd").args(["/C", "ver"])).output() {
            let t = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !t.is_empty() {
                return t;
            }
        }
    }
    std::env::consts::OS.to_string()
}

#[cfg(test)]
mod xdg_tests {
    use super::*;
    use std::ffi::OsString;

    #[test]
    fn data_base_follows_the_platform() {
        let o = |s: &str| Some(OsString::from(s));
        assert_eq!(pick_data_base(true, o("C:/roaming"), o("/x"), o("/home/a")), Some(PathBuf::from("C:/roaming")));
        assert_eq!(pick_data_base(false, o("C:/x"), o("/data"), o("/home/a")), Some(PathBuf::from("/data")));
        assert_eq!(pick_data_base(false, None, None, o("/home/a")), Some(PathBuf::from("/home/a").join(".local").join("share")));
        assert_eq!(pick_data_base(false, None, o("relative"), o("/home/a")), Some(PathBuf::from("/home/a").join(".local").join("share")));
        assert_eq!(pick_data_base(false, None, None, None), None);
    }
}

