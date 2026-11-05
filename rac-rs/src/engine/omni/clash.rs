//! The clash switch: a mod that does the job of one of our own PC features replaces it, so the two never fight over the same game words.
//! The features are the game pack's (`Game::features`, `Game::hook_writes`; R&C1's table in rac1/omni_bind.rs, whose index constants
//! the gates pass to `off`).
//!
//! Our PC features (modern camera, RMB strafe aim, wide lock-on, keyboard table) register their hands in `Retail::load` BEFORE the mods load
//! (`omni::init`), so the switch is a run-time check: every gate has one more `&& !clash::off(ID)` term, an atomic bit that is 0 until a mod
//! replaces the feature. With no mods nothing sets a bit: every gate answers as before, game state and timing are those of a build without mods.
//!
//! Detection is automatic, from what each loaded mod really takes (no per-mod list). A mod replaces a feature when
//! (a) its manifest `conflicts` names the feature id, (b) a hook point it takes is in `Feature::hooks`, (c) a hook point it takes writes
//! game words that the feature writes too (`HOOK_WRITES`), (d) it takes the keyboard (`take_input("keyboard")`: the keyboard table), or
//! (e) the feature `needs` another that was replaced. The run log says it (`omni: lock-strafe replaces rac-rs modern camera (...)`) and
//! status.json lists `"replaces": [ids]` per mod. Pure matcher: `decide`; glue: `after_load`.

use super::json::{self, Json};
use super::load::ModDir;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

/// Bit i = feature i is replaced by a mod (decided once, when the mods loaded).
pub static OFF: AtomicU32 = AtomicU32::new(0);
/// A mod holds the keyboard right now (`take_input`): kept live by input.rs, because a mod that fails later gives it back.
static KEYBOARD_TAKEN: AtomicBool = AtomicBool::new(false);
/// The bit of the game's keyboard-table feature (`Game::keyboard_feature`), 0 until the game is set.
static KBM_BIT: AtomicU32 = AtomicU32::new(0);

/// Is this feature (an index of the game's `Game::features`) replaced by a mod? One relaxed load; false until `after_load` sets it.
/// The keyboard table asks `keyboard_off`.
#[inline]
pub fn off(feature: usize) -> bool {
	OFF.load(Ordering::Relaxed) >> feature & 1 != 0
}

/// Is the keyboard table off: a mod replaced the game's keyboard feature, or holds the keyboard right now (`set_keyboard_taken`).
#[inline]
pub fn keyboard_off() -> bool {
	OFF.load(Ordering::Relaxed) & KBM_BIT.load(Ordering::Relaxed) != 0 || KEYBOARD_TAKEN.load(Ordering::Relaxed)
}

/// game.rs `set`: which feature is the keyboard table.
pub(super) fn set_keyboard_feature(i: usize) {
	KBM_BIT.store(1 << i, Ordering::Relaxed);
}

pub fn set_off(feature: usize) {
	OFF.fetch_or(1 << feature, Ordering::Relaxed);
}

/// input.rs: whether a mod holds the keyboard now.
pub(super) fn set_keyboard_taken(on: bool) {
	KEYBOARD_TAKEN.store(on, Ordering::Relaxed);
}

/// A span of game words `[lo, hi)` (RAM addresses of the NTSC data layout, which PAL v1.00 shares below the overlay: hero base 0x13F350).
#[derive(Clone, Copy, Debug)]
pub struct Span {
	pub name: &'static str,
	pub lo: u32,
	pub hi: u32,
}

pub const fn span(name: &'static str, lo: u32, hi: u32) -> Span {
	Span { name, lo, hi }
}

/// One of our PC features: the hook points of the binding that do the same job, the game words it writes, the features it cannot run without.
pub struct Feature {
	pub id: &'static str,
	pub label: &'static str,
	pub hooks: &'static [&'static str],
	pub writes: &'static [Span],
	pub needs: &'static [&'static str],
}

/// The reason for the keyboard table: input.rs keeps that one live (`set_keyboard_taken`), so `after_load` does not latch it.
const KEYBOARD_WHY: &str = "it took the keyboard";

/// What the matcher needs to know about one loaded mod.
#[derive(Clone, Debug, Default)]
pub struct ModUse {
	pub id: String,
	/// Hook point ids it took (status.json `hooks`).
	pub hooks: Vec<String>,
	/// Manifest `conflicts`.
	pub conflicts: Vec<String>,
	/// It took the keyboard (`take_input("keyboard")`).
	pub keyboard: bool,
}

/// One replaced feature: which, by which mod, why.
#[derive(Clone, Debug, PartialEq)]
pub struct Replaced {
	pub feature: usize,
	pub by: String,
	pub why: String,
}

fn overlap(a: &Span, b: &Span) -> bool {
	a.lo < b.hi && b.lo < a.hi
}

/// Why this mod replaces this feature, ignoring `needs`.
fn direct(f: &Feature, m: &ModUse) -> Option<String> {
	if m.conflicts.iter().any(|c| c == f.id) {
		return Some("its manifest conflicts names it".to_string());
	}
	if let Some(h) = m.hooks.iter().find(|h| f.hooks.contains(&h.as_str())) {
		return Some(format!("hook {h}"));
	}
	let g = super::game::get();
	for h in &m.hooks {
		let writes = g.hook_writes.iter().find(|(id, _)| id == h).map_or(&[][..], |(_, w)| w);
		if let Some((fw, _)) = f.writes.iter().find_map(|fw| writes.iter().find(|w| overlap(fw, w)).map(|w| (fw, w))) {
			return Some(format!("hook {h} decides the same word, {}", fw.name));
		}
	}
	(f.id == g.features[g.keyboard_feature].id && m.keyboard).then(|| KEYBOARD_WHY.to_string())
}

/// The first feature `f` needs that is already in `out`.
fn needs_replaced(f: &Feature, out: &[Replaced]) -> Option<&'static str> {
	f.needs.iter().copied().find(|n| super::game::get().features.iter().position(|g| g.id == *n).is_some_and(|j| out.iter().any(|r| r.feature == j)))
}

/// The pure matcher: every feature each mod replaces, in feature order (a feature is listed once, for the first mod that replaces it).
pub fn decide(mods: &[ModUse]) -> Vec<Replaced> {
	let mut out: Vec<Replaced> = Vec::new();
	// `needs` may chain, so repeat until nothing new (the table is tiny).
	loop {
		let before = out.len();
		for (i, f) in super::game::get().features.iter().enumerate() {
			if out.iter().any(|r| r.feature == i) {
				continue;
			}
			for m in mods {
				let why = direct(f, m).or_else(|| needs_replaced(f, &out).map(|n| format!("it needs {n}, which was replaced")));
				if let Some(why) = why {
					out.push(Replaced { feature: i, by: m.id.clone(), why });
					break;
				}
			}
		}
		if out.len() == before {
			break;
		}
	}
	out.sort_by_key(|r| r.feature);
	out
}

/// The run-log line for one replaced feature.
pub fn log_line(r: &Replaced) -> String {
	format!("omni: {} replaces {} ({})", r.by, super::game::get().features[r.feature].label, r.why)
}

/// The mods of a status.json text that loaded, with the hook points they took.
pub fn loaded_mods(status: &Json) -> Vec<ModUse> {
	let mut out = Vec::new();
	for m in status.get("mods").map_or(&[][..], Json::arr) {
		let (Some(id), Some("loaded")) = (m.get("id").and_then(Json::str), m.get("state").and_then(Json::str)) else { continue };
		let hooks = m.get("hooks").map_or(&[][..], Json::arr).iter().filter_map(|h| h.str().map(str::to_string)).collect();
		out.push(ModUse { id: id.to_string(), hooks, ..ModUse::default() });
	}
	out
}

/// Manifest `conflicts` of a mod folder.
fn conflicts_of(dir: &Path, folder: &str) -> Vec<String> {
	let text = super::modset::manifest_file(&dir.join(folder)).and_then(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
	let Ok(j) = json::parse(&text) else { return Vec::new() };
	j.get("conflicts").map_or(&[][..], Json::arr).iter().filter_map(|c| c.str().map(str::to_string)).collect()
}

/// status.json with `"replaces": [ids]` on every mod.
pub fn status_with_replaces(mut status: Json, replaced: &[Replaced]) -> String {
	if let Json::Obj(top) = &mut status
		&& let Some((_, Json::Arr(mods))) = top.iter_mut().find(|(k, _)| k == "mods")
	{
		for m in mods.iter_mut() {
			let id = m.get("id").and_then(Json::str).unwrap_or("").to_string();
			let mine: Vec<Json> = replaced.iter().filter(|r| r.by == id).map(|r| Json::Str(super::game::get().features[r.feature].id.to_string())).collect();
			if let Json::Obj(fields) = m {
				fields.retain(|(k, _)| k != "replaces");
				fields.push(("replaces".to_string(), Json::Arr(mine)));
			}
		}
	}
	status.to_text()
}

/// After the mods loaded (`load::start`): finds what they replace, switches those features off, logs it and rewrites status.json with the lists.
/// Returns the replaced features.
pub(super) fn after_load(dir: &Path, found: &[ModDir], status_text: &str, status_path: &Path) -> Vec<Replaced> {
	let Ok(status) = json::parse(status_text) else { return Vec::new() };
	let owners = super::input::keyboard_owners();
	let mut mods = loaded_mods(&status);
	for m in &mut mods {
		m.conflicts = found.iter().find(|f| f.id == m.id).map(|f| conflicts_of(dir, &f.folder)).unwrap_or_default();
		m.keyboard = owners.contains(&m.id);
	}
	let replaced = decide(&mods);
	for r in &replaced {
		if r.why != KEYBOARD_WHY {
			set_off(r.feature);
		}
		eprintln!("{}", log_line(r));
	}
	if !mods.is_empty() {
		if let Err(e) = std::fs::write(status_path, status_with_replaces(status, &replaced)) {
			eprintln!("omni: status.json could not be updated with the replaced features: {e}");
		}
	}
	replaced
}
