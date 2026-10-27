//! Finding the mods and the DLL, and starting the loader: the mods folder `<player dir>/mods/`, `omniloader.dll` next to the exe, the
//! binding (`omni/rac1.json` next to the exe, else the copy built into the exe), the plugins list, and the run-log lines.
//!
//! Nothing is loaded unless a mod folder (one with `omni-mod.json`) exists: no DLL, no hooks, no cost.

use super::binding::Binding;
use super::cabi::{Dll, Loader};
use super::host::{self, HostState};
use super::layout;
use super::json::{self, Json};
use super::Omni;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicU64;

/// The binding built into the exe (mod interface 1.3; THIRD_PARTY.md), used when `omni/rac1.json` is not next to the exe.
const BUILTIN_BINDING: &str = include_str!("rac1.json");

/// The ONE mods folder (texture packs, script and native mods, `active.txt`, `settings.json`): `RAC_MODS_DIR`, else `<install root>/mods`
/// (`texpack::mods::mods_root`), else, outside an install (dev lanes, tests), `<player dir>/mods`.
pub fn mods_dir() -> PathBuf {
	crate::texpack::mods::mods_root().unwrap_or_else(|| crate::retail::memcard::player_dir().join("mods"))
}

pub(super) fn exe_dir() -> Option<PathBuf> {
	std::env::current_exe().ok()?.parent().map(Path::to_path_buf)
}

/// RAC_OMNI_DLL=path, else the DLL next to the exe.
fn dll_path() -> Option<PathBuf> {
	if let Some(p) = std::env::var_os("RAC_OMNI_DLL") {
		return Some(PathBuf::from(p));
	}
	let name = if cfg!(windows) { "omniloader.dll" } else { "libomniloader.so" };
	Some(exe_dir()?.join(name))
}

/// Whether a mod loader can run: not switched off (`RAC_OMNI=0`) and its library next to the exe (or `RAC_OMNI_DLL`). The
/// Mods pages offer script / native mods only then; texture packs never need it.
pub fn loader_present() -> bool {
	if std::env::var("RAC_OMNI").is_ok_and(|v| v == "0") {
		return false;
	}
	dll_path().is_some_and(|p| p.is_file())
}

fn binding_text() -> String {
	exe_dir().map(|d| d.join("omni").join("rac1.json")).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_else(|| BUILTIN_BINDING.to_string())
}

/// One mod folder: its name, the manifest's id (the folder name when it has none) and whether it is a native mod (a `plugin` library table or
/// `"type": "native"`).
#[derive(Clone, Debug, PartialEq)]
pub struct ModDir {
	pub folder: String,
	pub id: String,
	pub native: bool,
}

/// Is this manifest text a native mod's?
pub fn is_native_manifest(j: &Json) -> bool {
	j.get("plugin").is_some() || j.get("type").and_then(Json::str) == Some("native")
}

/// The mod folders under `dir`, in folder-name order.
pub fn find_mods(dir: &Path) -> Vec<ModDir> {
	let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
	let mut out = Vec::new();
	for e in rd.flatten() {
		let Some(manifest) = super::modset::manifest_file(&e.path()) else { continue };
		let Ok(text) = std::fs::read_to_string(&manifest) else { continue };
		let folder = e.file_name().to_string_lossy().into_owned();
		let parsed = json::parse(&text).ok();
		let id = parsed.as_ref().and_then(|j| j.get("id").and_then(Json::str).map(str::to_string)).unwrap_or_else(|| folder.clone());
		out.push(ModDir { folder, id, native: parsed.as_ref().is_some_and(is_native_manifest) });
	}
	out.sort_by(|a, b| a.folder.cmp(&b.folder));
	out
}

fn esc(s: &str) -> String {
	s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// plugins.json for these ids, in this order, all enabled, no settings (the defaults of each mod's manifest).
pub fn plugins_json(ids: &[String]) -> String {
	let list: Vec<String> = ids.iter().map(|i| format!("{{\"id\":\"{}\",\"enabled\":true}}", esc(i))).collect();
	format!("{{\"format\":1,\"mods\":[{}]}}", list.join(","))
}

/// The run-log lines for a status.json text: `omni: N mods loaded (ids)`, then one line per mod that did not load.
pub fn status_lines(status: &str) -> Vec<String> {
	let Ok(j) = json::parse(status) else { return vec!["omni: status.json did not parse".to_string()] };
	let (mut loaded, mut other) = (Vec::new(), Vec::new());
	for m in j.get("mods").map_or(&[][..], Json::arr) {
		let id = m.get("id").and_then(Json::str).unwrap_or("?");
		match m.get("state").and_then(Json::str).unwrap_or("?") {
			"loaded" => loaded.push(id.to_string()),
			st => other.push(format!("omni: mod {id} {st}: {}", m.get("reason").and_then(Json::str).unwrap_or("no reason given"))),
		}
	}
	let mut out = vec![format!("omni: {} mods loaded ({})", loaded.len(), loaded.join(", "))];
	out.extend(other);
	out
}

/// How many mods status.json says are loaded.
pub fn loaded_count(status: &str) -> usize {
	let Ok(j) = json::parse(status) else { return 0 };
	j.get("mods").map_or(&[][..], Json::arr).iter().filter(|m| m.get("state").and_then(Json::str) == Some("loaded")).count()
}

/// Starts OmniLoader for this disc, or None (no mods, RAC_OMNI=0, no DLL, DLL or binding trouble: each said in the run log
/// except the first two, which are the normal case).
pub fn start(ntsc: bool, c: &mut ee_oracle::cpu::Cpu) -> Option<Omni> {
	if std::env::var("RAC_OMNI").is_ok_and(|v| v == "0") {
		return None;
	}
	let dir = mods_dir();
	let found = find_mods(&dir);
	if found.is_empty() {
		return None;
	}
	let Some(path) = dll_path().filter(|p| p.is_file()) else {
		eprintln!("omni: {} mod folder(s) in {} but omniloader.dll is not next to the exe: mods are off", found.len(), dir.display());
		return None;
	};
	// What to load (omni/plugins.rs): the mods `active.txt` switches on (or an explicit plugins.json), with the values of settings.json.
	for d in super::plugins::duplicates(&found) {
		eprintln!("omni: folder {} has the mod id {} of an earlier folder: ignored", d.folder, d.id);
	}
	let plan = super::plugins::plan(&dir, &found);
	if plan.enabled.is_empty() && !plan.explicit {
		eprintln!("omni: {} mod folder(s) in {}, none switched on (mods/active.txt): mods are off", found.len(), dir.display());
		return None;
	}
	let native = found.iter().any(|m| m.native && plan.enabled.contains(&m.id));
	let (disc, note) = layout::disc_string(ntsc, std::env::var("RAC_OMNI_DISC").ok().as_deref(), native);
	if let Some(n) = note {
		eprintln!("omni: {n}");
	}
	let _held = super::lock::enter();
	let result = (|| -> Result<Omni, String> {
		let dll = Dll::open(&path)?;
		let text = binding_text();
		let binding = Binding::parse(&text)?;
		let state: &'static HostState = Box::leak(Box::new(HostState { binding, ntsc, disc }));
		let h = host::make(state);
		let cfg = host::config(&dir.join("status.json"));
		// The loader picks absolute field addresses by the disc string; our data layout is the "ntsc" column on both discs (layout.rs).
		let for_loader = if disc == "pal" { layout::pal_as_ntsc(&text)? } else { text };
		super::native_host::set_ram(c);
		let loader = Loader::create(dll, &for_loader, &h, &cfg)?;
		// The mods' main scripts run now (the format: "at the game's start"): the machine exists, so `omni.read` / `write` in a main script see its RAM.
		// ABI 1.2: the folder of the per-save slot files, set BEFORE omni_load (a DLL without the export has no per-save store).
		loader.storage_dir(&crate::retail::memcard::player_dir().join("omni"));
		super::input::begin_load();
		let loaded = super::host::scope(c, || loader.load(&dir, &plan.text));
		super::input::end_load(loaded.is_ok());
		loaded?;
		let mods = loaded_count(&loader.status());
		Ok(Omni { loader, state, ticks: AtomicU64::new(0), native, mods, events: Default::default() })
	})();
	match result {
		Ok(o) => {
			eprintln!("omni: OmniLoader {} from {}, {} disc, mods are told \"{disc}\"", o.loader.version(), path.display(), if ntsc { "NTSC" } else { "PAL" });
			let status = o.loader.status();
			for l in status_lines(&status) {
				eprintln!("{l}");
			}
			super::hooks::installed(&o);
			super::newgame::install(&o);
			super::clash::after_load(&dir, &found, &status, &dir.join("status.json"));
			Some(o)
		}
		Err(e) => {
			eprintln!("omni: mods are off: {e}");
			None
		}
	}
}
