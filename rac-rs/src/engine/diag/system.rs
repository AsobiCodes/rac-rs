//! [PC] RAC_DIAG (diag.rs): the text of `system.txt`: the exe, this PC (the run log's system line), the env switches, the options
//! file, the mods, and which optional DLLs lie beside the exe. The GPU / display / upscaler block is appended once the window's
//! device exists (`gpu_block`), option changes later with a time stamp (diag.rs `options_saved`).

use std::fmt::Write;
use std::path::Path;

/// Optional DLLs beside the exe and what each one is for (a missing one switches only that feature off).
pub const DLLS: [(&str, &str); 16] = [
	("omniloader.dll", "script mods"),
	("rac_ngx.dll", "DLSS upscaler bridge"),
	("nvngx_dlss.dll", "DLSS upscaler"),
	("amd_fidelityfx_loader_vk.dll", "FSR upscaler / frame generation (Vulkan)"),
	("amd_fidelityfx_upscaler_vk.dll", "FSR upscaler (Vulkan)"),
	("amd_fidelityfx_framegeneration_vk.dll", "FSR frame generation (Vulkan)"),
	("amd_fidelityfx_loader_dx12.dll", "FSR upscaler / frame generation (DX12)"),
	("amd_fidelityfx_upscaler_dx12.dll", "FSR upscaler (DX12)"),
	("amd_fidelityfx_framegeneration_dx12.dll", "FSR frame generation (DX12)"),
	("sl.interposer.dll", "DLSS frame generation"),
	("sl.common.dll", "DLSS frame generation"),
	("sl.dlss_g.dll", "DLSS frame generation"),
	("sl.reflex.dll", "DLSS frame generation"),
	("sl.pcl.dll", "DLSS frame generation"),
	("nvngx_dlssg.dll", "DLSS frame generation"),
	("D3D12Core.dll", "DirectX 12 runtime (optional)"),
];

/// "present" / "MISSING" per optional DLL in `dir` (`there`: the file test, a parameter for the tests).
pub fn dll_lines(there: impl Fn(&str) -> bool) -> String {
	let mut s = String::new();
	for (n, what) in DLLS {
		let _ = writeln!(s, "  {:<8} {n:<42} {what}", if there(n) { "present" } else { "MISSING" });
	}
	s
}

/// The frame-generation files missing beside the exe, as one hint line, or None when one FSR pair or the whole DLSS set is there.
pub fn fg_missing(there: impl Fn(&str) -> bool) -> Option<String> {
	let fsr = [["amd_fidelityfx_loader_vk.dll", "amd_fidelityfx_framegeneration_vk.dll"], ["amd_fidelityfx_loader_dx12.dll", "amd_fidelityfx_framegeneration_dx12.dll"]];
	let fsr_ok = fsr.iter().any(|p| p.iter().all(|n| there(n)));
	let dlss: Vec<&str> = DLLS[9..15].iter().map(|d| d.0).filter(|n| !there(n)).collect();
	if fsr_ok && dlss.is_empty() {
		return None;
	}
	let mut s = String::from("frame generation files missing next to the game (copy the whole game folder, not only the exe):");
	if !fsr_ok {
		s += " FSR: amd_fidelityfx_loader_*.dll + amd_fidelityfx_framegeneration_*.dll;";
	}
	if !dlss.is_empty() {
		let _ = write!(s, " DLSS: {};", dlss.join(" "));
	}
	s.pop();
	Some(s)
}

/// The RAC_* / ORACLE_* switches of this run, sorted.
pub fn env_lines() -> String {
	let mut v: Vec<(String, String)> = std::env::vars().filter(|(k, _)| k.starts_with(concat!("RAC", "_")) || k.starts_with("ORACLE_")).collect();
	v.sort();
	v.iter().map(|(k, val)| format!("  {k}={val}\n")).collect()
}

/// The exe: path, size, modified time (seconds since 1970) and a 64-bit FNV hash of its bytes (the build id).
fn exe_lines() -> String {
	let Ok(p) = std::env::current_exe() else { return "  exe: unknown\n".into() };
	let meta = std::fs::metadata(&p).ok();
	let mtime = meta.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
	let size = meta.map_or(0, |m| m.len());
	// The exe itself is not read: a whole-file hash cost seconds at start-up on a cold disk or SD card.
	let id = build_id(size, mtime, crate::app_name());
	format!("  exe: {}\n  size: {size} bytes, built {} UTC, id {id:016x} (size + time + version)\n  version: {}\n", p.display(), super::session::utc_stamp(mtime / 1_000_000_000), crate::app_name())
}

/// The build id of an exe of `size` bytes modified at `mtime_ns` (since 1970) with version text `version`.
pub fn build_id(size: u64, mtime_ns: u64, version: &str) -> u64 {
	let mut b = Vec::with_capacity(16 + version.len());
	b.extend_from_slice(&size.to_le_bytes());
	b.extend_from_slice(&mtime_ns.to_le_bytes());
	b.extend_from_slice(version.as_bytes());
	crate::diag::fnv64(&b)
}

/// The CPU's brand string (x86_64: cpuid), else the OS's processor id.
pub fn cpu_name() -> String {
	#[cfg(target_arch = "x86_64")]
	{
		use std::arch::x86_64::__cpuid;
		// SAFETY: cpuid exists on every x86_64 CPU (safe fn on new toolchains, unsafe on older: hence the allow); leaf 0x80000000 says whether the brand leaves exist.
		#[allow(unused_unsafe)]
		let top = unsafe { __cpuid(0x8000_0000) }.eax;
		if top >= 0x8000_0004 {
			let mut b = Vec::with_capacity(48);
			for leaf in 0x8000_0002u32..=0x8000_0004 {
				// SAFETY: as above, the leaf exists.
				#[allow(unused_unsafe)]
				let r = unsafe { __cpuid(leaf) };
				for w in [r.eax, r.ebx, r.ecx, r.edx] {
					b.extend_from_slice(&w.to_le_bytes());
				}
			}
			return String::from_utf8_lossy(&b).trim_matches(char::from(0)).trim().to_string();
		}
	}
	std::env::var("PROCESSOR_IDENTIFIER").unwrap_or_else(|_| "unknown".into())
}

/// The mods of this run: the mods root and its `active.txt` ids.
fn mods_lines() -> String {
	let Some(root) = crate::texpack::mods::mods_root() else { return "  (no mods folder)\n".into() };
	let ids = std::fs::read_to_string(root.join("active.txt")).map(|t| crate::texpack::mods::parse_active(&t)).unwrap_or_default();
	let mut s = format!("  folder: {}\n", root.display());
	if ids.is_empty() {
		s += "  active: none\n";
	}
	for id in ids {
		let _ = writeln!(s, "  active: {id}{}", if root.join(&id).is_dir() { "" } else { " (folder missing)" });
	}
	s
}

/// The first block of system.txt.
pub fn start_text(stamp: &str, opts_text: &str, exe_dir: Option<&Path>) -> String {
	let mut s = format!("RAC_DIAG session, started {stamp} (local time)\n\n[game]\n{}\n[system]\n  {}\n  cpu: {}\n  wine: {}\n\n[env]\n{}\n[options] pc_options.txt at start\n", exe_lines(), crate::runlog::system_summary(), cpu_name(), crate::wine::version().unwrap_or("no"), env_lines());
	for l in opts_text.lines() {
		let _ = writeln!(s, "  {l}");
	}
	s += "\n[mods]\n";
	s += &mods_lines();
	s += "\n[optional files beside the exe]\n";
	let there = |n: &str| exe_dir.is_some_and(|d| d.join(n).is_file());
	s += &dll_lines(there);
	if let Some(m) = fg_missing(there) {
		let _ = writeln!(s, "  NOTE: {m}");
	}
	s
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fg_missing_names_what_is_absent() {
		assert_eq!(fg_missing(|_| true), None);
		let m = fg_missing(|_| false).unwrap();
		assert!(m.contains("copy the whole game folder") && m.contains("FSR:") && m.contains("nvngx_dlssg.dll"));
		// The Vulkan FSR pair alone and every DLSS file: nothing missing.
		assert_eq!(fg_missing(|n| !n.contains("dx12")), None);
		let only_fsr = fg_missing(|n| n.starts_with("amd_")).unwrap();
		assert!(!only_fsr.contains("FSR:") && only_fsr.contains("sl.interposer.dll"));
	}

	#[test]
	fn build_ids_change_with_size_time_and_version() {
		let a = build_id(10, 20, "v");
		assert_ne!(a, build_id(11, 20, "v"));
		assert_ne!(a, build_id(10, 21, "v"));
		assert_ne!(a, build_id(10, 20, "w"));
		assert_eq!(a, build_id(10, 20, "v"));
	}

	#[test]
	fn dll_lines_mark_each_file() {
		let t = dll_lines(|n| n == "omniloader.dll");
		assert_eq!(t.lines().count(), DLLS.len());
		assert!(t.lines().next().unwrap().contains("present"));
		assert!(t.lines().nth(1).unwrap().contains("MISSING"));
	}
}
