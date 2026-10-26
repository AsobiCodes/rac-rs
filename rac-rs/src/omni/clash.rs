//! The clash switch: a mod that does the job of one of our own PC features replaces it, so the two never fight over the same game words.
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

/// The feature indices, for `off`. Order of `FEATURES`.
pub const MODERN_CAMERA: usize = 0;
pub const STRAFE_AIM: usize = 1;
pub const LOCKON: usize = 2;
pub const KBM: usize = 3;

/// Bit i = feature i is replaced by a mod (decided once, when the mods loaded).
static OFF: AtomicU32 = AtomicU32::new(0);
/// A mod holds the keyboard right now (`take_input`): kept live by input.rs, because a mod that fails later gives it back.
static KEYBOARD_TAKEN: AtomicBool = AtomicBool::new(false);

/// Is this feature replaced by a mod? Two relaxed loads at most; false until `after_load` (or `set_keyboard_taken` for the keyboard) sets it.
#[inline]
pub fn off(feature: usize) -> bool {
	OFF.load(Ordering::Relaxed) >> feature & 1 != 0 || (feature == KBM && KEYBOARD_TAKEN.load(Ordering::Relaxed))
}

fn set_off(feature: usize) {
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

const fn span(name: &'static str, lo: u32, hi: u32) -> Span {
	Span { name, lo, hi }
}

/// hero + 0x98, the way he faces (retail/modcam.rs, strafe.rs, aim.rs HERO_YAW).
const HERO_FACING: Span = span("hero facing 0x13F3E8", 0x13F3E8, 0x13F3EC);
/// hero + 0x180, where the stick moves him (HERO_WISH).
const HERO_MOVE_YAW: Span = span("hero move yaw 0x13F4D0", 0x13F4D0, 0x13F4D4);

/// One of our PC features: the hook points of the binding that do the same job, the game words it writes, the features it cannot run without.
pub struct Feature {
	pub id: &'static str,
	pub label: &'static str,
	pub hooks: &'static [&'static str],
	pub writes: &'static [Span],
	pub needs: &'static [&'static str],
}

/// Order = the index constants above. [C from retail/modcam.rs, strafe.rs, aim.rs: what each patches.]
pub static FEATURES: [Feature; 4] = [
	// Follow-camera hand (camera record rows), the hero-attack hand, and in aim mode the hero facing, move yaw, manual-aim override and swing yaw.
	Feature {
		id: "rac-rs.modern-camera",
		label: "rac-rs modern camera",
		hooks: &["rac1.camera.follow.apply_overrides"],
		writes: &[HERO_FACING, HERO_MOVE_YAW, span("manual aim 0x140600", 0x140600, 0x140610), span("swing yaw 0x13FDA8", 0x13FDA8, 0x13FDB0)],
		needs: &[],
	},
	// The RMB strafe mode: jump-chooser flip hands (hero facing, flip state) and the draw-only leg twist; it reads the modern camera's yaw.
	Feature {
		id: "rac-rs.strafe-aim",
		label: "rac-rs strafe aim",
		hooks: &[],
		writes: &[HERO_FACING, span("flip state 0x13F760", 0x13F760, 0x13F7B0)],
		needs: &["rac-rs.modern-camera"],
	},
	// The wide Blaster lock-on: the lock target word (and the crosshair icon).
	Feature { id: "rac-rs.lockon", label: "rac-rs wide lock-on", hooks: &[], writes: &[span("lock target 0x13FDA0", 0x13FDA0, 0x13FDA4)], needs: &[] },
	// The keyboard table (retail/input.rs tick_keys): off while a mod has the keyboard.
	Feature { id: "rac-rs.kbm", label: "rac-rs keyboard table", hooks: &[], writes: &[], needs: &[] },
];

/// Hook point -> the game words its mod handler decides. Only the ones a feature also writes matter; a hook missing here decides no word of ours
/// (pad.process_report: the pad report; advance_animation: animation; set_planar_velocity: velocity; item/vendor/weapon/moby hooks: other state;
/// camera.follow.apply_overrides is listed in `Feature::hooks` of the modern camera instead).
pub static HOOK_WRITES: [(&str, &[Span]); 2] = [
	// turn_to_heading turns hero+0x98 toward hero+0x180.
	("rac1.player.turn_to_heading", &[HERO_FACING]),
	// update_motion_input writes hero+0x180 and the stick-derived motion.
	("rac1.player.update_motion_input", &[HERO_MOVE_YAW]),
];

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
	for h in &m.hooks {
		let writes = HOOK_WRITES.iter().find(|(id, _)| id == h).map_or(&[][..], |(_, w)| w);
		if let Some((fw, _)) = f.writes.iter().find_map(|fw| writes.iter().find(|w| overlap(fw, w)).map(|w| (fw, w))) {
			return Some(format!("hook {h} decides the same word, {}", fw.name));
		}
	}
	(f.id == FEATURES[KBM].id && m.keyboard).then(|| KEYBOARD_WHY.to_string())
}

/// The first feature `f` needs that is already in `out`.
fn needs_replaced(f: &Feature, out: &[Replaced]) -> Option<&'static str> {
	f.needs.iter().copied().find(|n| FEATURES.iter().position(|g| g.id == *n).is_some_and(|j| out.iter().any(|r| r.feature == j)))
}

/// The pure matcher: every feature each mod replaces, in feature order (a feature is listed once, for the first mod that replaces it).
pub fn decide(mods: &[ModUse]) -> Vec<Replaced> {
	let mut out: Vec<Replaced> = Vec::new();
	// `needs` may chain, so repeat until nothing new (the table is tiny).
	loop {
		let before = out.len();
		for (i, f) in FEATURES.iter().enumerate() {
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
	format!("omni: {} replaces {} ({})", r.by, FEATURES[r.feature].label, r.why)
}

/// The mods of a status.json text that loaded, with the hook points they took.
fn loaded_mods(status: &Json) -> Vec<ModUse> {
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
fn status_with_replaces(mut status: Json, replaced: &[Replaced]) -> String {
	if let Json::Obj(top) = &mut status
		&& let Some((_, Json::Arr(mods))) = top.iter_mut().find(|(k, _)| k == "mods")
	{
		for m in mods.iter_mut() {
			let id = m.get("id").and_then(Json::str).unwrap_or("").to_string();
			let mine: Vec<Json> = replaced.iter().filter(|r| r.by == id).map(|r| Json::Str(FEATURES[r.feature].id.to_string())).collect();
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

#[cfg(test)]
mod tests;
