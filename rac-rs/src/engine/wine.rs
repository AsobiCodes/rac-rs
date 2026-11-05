//! [PC] Running under Wine (Wine, Proton, Playden on the PS5): the Windows exe on a Linux-like system. GitHub issue #49.
//!
//! A Windows system never has this: ntdll.dll exports `wine_get_version` only under Wine; a Wine that hides its exports from the program
//! (wine-staging has a setting for it) still has Wine's own programs (wineboot.exe, winecfg.exe, winedevice.exe) in the prefix's system
//! folder, which is the second test. A registry key (`HKCU\Software\Wine`) is no test: a real Windows can keep one after a removed tool or
//! a copied profile, and would then get the Wine-only behaviour below; no real Windows has those programs. The answer is asked once and kept.
//! What the game does differently there (nothing else in rac-rs asks):
//! - wgpu hides a Vulkan adapter whose driver reports no conformance version (wgpu-hal `vulkan/adapter.rs`), and a Wine Vulkan
//!   driver (RADV on the PS5, device 0x0000) can be exactly that. The window's instance (`viewer/gpu/renderer.rs`) and the capture's
//!   own one (`retail/gscap/draw/gpu_shared.rs`) then get `ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER` (`instance_flags`); without it the
//!   game found no Vulkan adapter and fell back to DirectX 12 over vkd3d-proton, which crashed in the console's driver.
//!   `WGPU_ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER=0/1` (wgpu's own switch, read after these flags) still has the last word.
//! - The run log is on by default there (`runlog_policy.rs`): Wine always has a stderr, but nobody reads it, and it is also written
//!   to `<exe folder>/<exe name>_Data/output_log.txt` (`data_dir`), the file Playden collects. Native Windows and Linux: unchanged.
#[cfg(any(windows, test))]
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Wine's version ("11.17") when this exe runs under Wine, None on a real Windows and on every other system. Empty if Wine gave none.
pub fn version() -> Option<&'static str> {
	static VERSION: OnceLock<Option<String>> = OnceLock::new();
	VERSION.get_or_init(detect).as_deref()
}

/// True under Wine (see the module doc).
pub fn under_wine() -> bool {
	version().is_some()
}

#[cfg(windows)]
fn detect() -> Option<String> {
	ntdll_version().or_else(|| wine_marker_in(&system_dir()).then(String::new))
}

/// `%SystemRoot%\system32` of this process (`C:\Windows` when the variable is missing).
#[cfg(windows)]
fn system_dir() -> PathBuf {
	let root = std::env::var_os("SystemRoot").or_else(|| std::env::var_os("windir")).map_or_else(|| PathBuf::from("C:\\Windows"), PathBuf::from);
	root.join("system32")
}

/// Programs only a Wine prefix has in its system folder (Wine's own tools; Windows has none of them).
#[cfg(any(windows, test))]
const WINE_PROGRAMS: [&str; 3] = ["wineboot.exe", "winecfg.exe", "winedevice.exe"];

/// True when the system folder `dir` holds one of Wine's programs: the second test of `detect` (one file lookup, at most three).
#[cfg(any(windows, test))]
fn wine_marker_in(dir: &Path) -> bool {
	WINE_PROGRAMS.iter().any(|f| dir.join(f).is_file())
}

/// Wine's version from the ntdll export, None on a real Windows (and on a Wine that hides the export).
#[cfg(windows)]
fn ntdll_version() -> Option<String> {
	use std::ffi::{CStr, c_char, c_void};
	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn GetModuleHandleW(name: *const u16) -> *mut c_void;
		fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
	}
	let ntdll: Vec<u16> = "ntdll.dll".encode_utf16().chain(Some(0)).collect();
	// SAFETY: ntdll.dll is loaded in every Windows process; the export is looked up by name and, when it exists (Wine only), called with
	// Wine's own signature `const char *wine_get_version(void)` (cdecl, no arguments); the string it returns is static.
	unsafe {
		let module = GetModuleHandleW(ntdll.as_ptr());
		if module.is_null() {
			return None;
		}
		let export = GetProcAddress(module, c"wine_get_version".as_ptr().cast());
		if export.is_null() {
			return None;
		}
		let get: extern "C" fn() -> *const c_char = std::mem::transmute(export);
		let text = get();
		Some(if text.is_null() { String::new() } else { CStr::from_ptr(text).to_string_lossy().into_owned() })
	}
}

#[cfg(not(windows))]
fn detect() -> Option<String> {
	None
}

/// The extra wgpu instance flags this process needs: `ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER` under Wine, none anywhere else.
pub fn instance_flags() -> wgpu::InstanceFlags {
	instance_flags_for(under_wine())
}

/// `instance_flags` for a given answer of the Wine test.
pub fn instance_flags_for(wine: bool) -> wgpu::InstanceFlags {
	if wine { wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER } else { wgpu::InstanceFlags::empty() }
}

/// `<exe folder>/<exe name>_Data` for the exe at `exe`: where Playden looks for a game's own `output_log.txt` (the folder name
/// Unity games use). Only the Windows run log (and the tests) asks.
#[cfg(any(windows, test))]
pub fn data_dir_for(exe: &Path) -> Option<PathBuf> {
	Some(exe.parent()?.join(format!("{}_Data", exe.file_stem()?.to_string_lossy())))
}

/// `data_dir_for` the running exe, and only under Wine (None everywhere else).
#[cfg(any(windows, test))]
pub fn data_dir() -> Option<PathBuf> {
	if !under_wine() {
		return None;
	}
	data_dir_for(&std::env::current_exe().ok()?)
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The flag that unhides a non-conformant Vulkan adapter is set under Wine and in no other case.
	#[test]
	fn only_wine_gets_the_noncompliant_adapter_flag() {
		assert!(instance_flags_for(true).contains(wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER));
		assert_eq!(instance_flags_for(true), wgpu::InstanceFlags::ALLOW_UNDERLYING_NONCOMPLIANT_ADAPTER, "nothing else is added");
		assert_eq!(instance_flags_for(false), wgpu::InstanceFlags::empty(), "a real Windows and Linux: the flags stay wgpu's own");
	}

	/// The test machine is a real Windows or Linux PC: the detection says no, and says it the same on the second ask.
	#[test]
	fn a_real_pc_is_not_wine() {
		if std::env::var_os("WINELOADER").is_some() || std::env::var_os("WINEPREFIX").is_some() {
			return; // the tests run under Wine themselves
		}
		assert_eq!(version(), None);
		assert!(!under_wine());
		assert_eq!(instance_flags(), wgpu::InstanceFlags::empty());
		assert_eq!(data_dir(), None);
	}

	/// The second test needs a Wine program in the system folder: a folder without one (a real Windows with a leftover Wine registry key has
	/// the Windows files only) is not Wine, one with any of them is.
	#[test]
	fn only_a_folder_with_wines_programs_counts_as_wine() {
		let dir = std::env::temp_dir().join(format!("rac_wine_marker_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&dir);
		std::fs::create_dir_all(&dir).unwrap();
		assert!(!wine_marker_in(&dir), "empty");
		for f in ["kernel32.dll", "ntdll.dll", "notepad.exe", "wine.txt"] {
			std::fs::write(dir.join(f), b"x").unwrap();
		}
		assert!(!wine_marker_in(&dir), "Windows files only (and a leftover key says nothing here)");
		std::fs::create_dir(dir.join("wineboot.exe")).unwrap();
		assert!(!wine_marker_in(&dir), "a folder of that name is no program");
		for f in WINE_PROGRAMS {
			let one = dir.join(f.replace(".exe", ""));
			std::fs::create_dir_all(&one).unwrap();
			std::fs::write(one.join(f), b"MZ").unwrap();
			assert!(wine_marker_in(&one), "{f}");
		}
		assert!(!wine_marker_in(&dir.join("no_such_folder")));
		let _ = std::fs::remove_dir_all(&dir);
	}

	/// This PC's real system folder has none of them (the test machine is no Wine), so the second test says no.
	#[cfg(windows)]
	#[test]
	fn a_real_windows_system_folder_is_not_wine() {
		if std::env::var_os("WINELOADER").is_none() && std::env::var_os("WINEPREFIX").is_none() {
			assert!(system_dir().is_dir(), "the system folder is found: {:?}", system_dir());
			assert!(!wine_marker_in(&system_dir()));
		}
	}

	/// Playden's file: next to the exe, in `<exe name>_Data`.
	#[test]
	fn the_data_folder_is_next_to_the_exe_and_named_after_it() {
		assert_eq!(data_dir_for(Path::new("C:/games/rac/rac-rs.exe")), Some(PathBuf::from("C:/games/rac/rac-rs_Data")));
		assert_eq!(data_dir_for(Path::new("/x/rac-rs-test")), Some(PathBuf::from("/x/rac-rs-test_Data")));
		assert_eq!(data_dir_for(Path::new("")), None);
	}
}
