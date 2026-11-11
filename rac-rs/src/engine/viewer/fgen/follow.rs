//! [PC] Frame Generation follows the Upscaler, live where the running device allows (lane fglive, user 2026-11-08: "choose fg
//! according to the upscaler ure using ... greyed out and not switchable without an upscaler"; "why do i need a restart" -> live
//! where possible). No retail counterpart, no RAM byte.
//!
//! Rule A (`fits`): the row offers only the frame generation of the running Upscaler's maker: Upscaler DLSS -> Off, DLSS 2x/3x/4x
//! (as the card allows); Upscaler FSR -> Off, FSR 2x; a card without DLSS frame generation (RTX 20/30: no DLSS value offered)
//! with Upscaler DLSS -> Off, FSR 2x; Upscaler Off -> Off only (the menu row dims: its `Needs` on the Upscaler row).
//! Rule B (`in_force`): the value in force is the chosen one while it fits the running Upscaler and its maker is the one this run
//! started (`set_running`: its swapchain / Streamline instance exists), else Off. An Upscaler change never rewrites the options
//! file's choice: DLSS 4x -> Upscaler FSR pauses it, Upscaler DLSS again resumes it.
//! Rule C (`needs_restart`, `note`): a value of the running maker applies at once (frame.rs `fg_paused`: Off = generation paused,
//! the frames are today's real ones; DLSS 2x/3x/4x: sl.rs `want`, the per-frame DLSS-G options); a value of another maker, incl.
//! any frame generation in a run that started without one, applies after a restart (the swapchain is made with the device).
//! `refresh` publishes rule A to the menus (ui/live.rs `offer`) with the row's note and logs one line per change (rule E).
//! A run forced by the env RAC_FG (tests) keeps the old behaviour: the forced generation runs whatever the Upscaler.

use super::choice;
use crate::viewer::upscale_choice as up;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Mutex;

/// Who makes a frame generation value.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Maker {
	Fsr,
	Dlss,
}

/// The maker of row value `v` (pure): FSR 2x -> Fsr, DLSS 2x..4x -> Dlss, Off -> None.
pub fn maker_of(v: i32) -> Option<Maker> {
	match v {
		choice::FSR_VALUE => Some(Maker::Fsr),
		2..=4 => Some(Maker::Dlss),
		_ => None,
	}
}

/// Rule A (pure): whether row value `v` fits Upscaler row value `upscaler` (0 Off, 1 FSR, 2 DLSS) on a card with DLSS frame
/// generation (`dlss_g`).
pub fn fits(v: i32, upscaler: i32, dlss_g: bool) -> bool {
	match (upscaler, maker_of(v)) {
		(_, None) => true,
		(up::FFX_VALUE, Some(Maker::Fsr)) => true,
		(up::DLSS_VALUE, Some(Maker::Dlss)) => dlss_g,
		(up::DLSS_VALUE, Some(Maker::Fsr)) => !dlss_g,
		_ => false,
	}
}

/// Whether this PC runs any DLSS frame generation value (`cap`: choice.rs `offered`, what the PC can run at all).
pub fn dlss_g(cap: &impl Fn(i32) -> bool) -> bool {
	choice::DLSS_VALUES.iter().any(|&v| cap(v))
}

/// Rule A with the PC (pure): value `v` can run here (`cap`) and fits the Upscaler.
pub fn offered_with(v: i32, upscaler: i32, cap: &impl Fn(i32) -> bool) -> bool {
	cap(v) && fits(v, upscaler, dlss_g(cap))
}

/// What the row shows for chosen value `chosen` (pure): choice.rs `shown` over the values rule A offers (a DLSS multiplier above
/// the card's maximum shows the one that runs; a value of the other maker, or any with the Upscaler Off, shows Off).
pub fn shown_with(chosen: u8, upscaler: i32, cap: &impl Fn(i32) -> bool) -> u8 {
	choice::shown(chosen, |w| offered_with(w, upscaler, cap))
}

/// Whether value `v` needs a restart to run (pure): a frame generation of a maker this run did not start (`running`).
pub fn needs_restart(v: i32, running: Option<Maker>) -> bool {
	maker_of(v).is_some_and(|m| Some(m) != running)
}

/// Rule B (pure): the value in force (0 = generation paused, today's frames).
pub fn in_force(chosen: u8, upscaler: i32, cap: &impl Fn(i32) -> bool, running: Option<Maker>) -> u8 {
	let s = shown_with(chosen, upscaler, cap);
	if needs_restart(s as i32, running) { 0 } else { s }
}

/// Why the generation of the running maker is paused (besides a picture box, frame.rs).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Why {
	/// The chosen value is Off.
	ChosenOff,
	/// The Upscaler is Off.
	UpscalerOff,
	/// The chosen value is not of the running Upscaler's maker (rule A).
	OtherUpscaler,
	/// The chosen value is of another maker than the running one: it runs after a restart.
	Restart,
}

impl Why {
	pub fn text(self) -> &'static str {
		match self {
			Why::ChosenOff => "Frame Generation Off",
			Why::UpscalerOff => "Upscaler Off",
			Why::OtherUpscaler => "the Upscaler's maker differs",
			Why::Restart => "another maker than this run's: after a restart",
		}
	}
}

/// Why the generation is paused for chosen value `chosen` (pure; None = it runs).
pub fn why(chosen: u8, upscaler: i32, cap: &impl Fn(i32) -> bool, running: Option<Maker>) -> Option<Why> {
	if in_force(chosen, upscaler, cap, running) != 0 {
		return None;
	}
	let s = shown_with(chosen, upscaler, cap);
	Some(if choice::shown(chosen, cap) == 0 {
		Why::ChosenOff
	} else if upscaler == 0 {
		Why::UpscalerOff
	} else if s == 0 {
		Why::OtherUpscaler
	} else {
		Why::Restart
	})
}

/// The row's note.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Note {
	NeedsUpscaler,
	NoCard,
	Missing,
	Restart,
	AtOnce,
}

/// The row's note (pure): no Upscaler first; a card without DLSS frame generation while nothing but Off fits Upscaler DLSS; files
/// missing next to the exe; else "Applies after a restart" when the chosen value needs one or every value on offer does, else
/// "Applies at once".
pub fn note(chosen: u8, upscaler: i32, cap: &impl Fn(i32) -> bool, running: Option<Maker>, no_card: bool, missing: bool) -> Note {
	let on: Vec<i32> = (1..choice::KINDS.len() as i32).filter(|&v| offered_with(v, upscaler, cap)).collect();
	let s = shown_with(chosen, upscaler, cap) as i32;
	if upscaler == 0 {
		Note::NeedsUpscaler
	} else if upscaler == up::DLSS_VALUE && no_card && on.is_empty() {
		Note::NoCard
	} else if missing {
		Note::Missing
	} else if needs_restart(s, running) || !on.is_empty() && on.iter().all(|&v| needs_restart(v, running)) {
		Note::Restart
	} else {
		Note::AtOnce
	}
}

/// The row's note while the Upscaler is Off.
pub const NEEDS_UPSCALER_NOTE: crate::ui::T5 = [
	"Needs DLSS or FSR as the Upscaler",
	"Exige DLSS ou FSR en mise \u{e0} l'\u{e9}chelle",
	"Braucht DLSS oder FSR als Hochskalierung",
	"Requiere escalado DLSS o FSR",
	"Richiede upscaling DLSS o FSR",
];
/// The row's note while a value on offer needs a restart (rule C).
pub const RESTART_NOTE: crate::ui::T5 = [
	"Applies after a restart",
	"S'applique apr\u{e8}s un red\u{e9}marrage",
	"Gilt nach einem Neustart",
	"Se aplica al reiniciar",
	"Si applica dopo il riavvio",
];
/// The row's note while every value on offer applies at once.
pub const AT_ONCE_NOTE: crate::ui::T5 = [
	"Applies at once",
	"S'applique imm\u{e9}diatement",
	"Gilt sofort",
	"Se aplica al instante",
	"Si applica subito",
];

fn note_text(n: Note) -> &'static crate::ui::T5 {
	match n {
		Note::NeedsUpscaler => &NEEDS_UPSCALER_NOTE,
		Note::NoCard => &choice::NO_CARD_NOTE,
		Note::Missing => &choice::MISSING_NOTE,
		Note::Restart => &RESTART_NOTE,
		Note::AtOnce => &AT_ONCE_NOTE,
	}
}

/// The maker this run started (0 none, 1 FSR, 2 DLSS): set when its swapchain presents (present.rs `set_own`).
static RUNNING: AtomicU8 = AtomicU8::new(0);
/// The last change line (`refresh`).
static LAST: Mutex<String> = Mutex::new(String::new());

/// The maker this run's swapchain started with (None: wgpu's surface, no generation possible without a restart).
pub fn running() -> Option<Maker> {
	match RUNNING.load(Ordering::Relaxed) {
		1 => Some(Maker::Fsr),
		2 => Some(Maker::Dlss),
		_ => None,
	}
}

/// The run's swapchain presents with `m`'s frame generation (None: it fell back to wgpu's surface). Public for the game packs'
/// menu tests.
pub fn set_running(m: Option<Maker>) {
	RUNNING.store(m.map_or(0, |m| 1 + (m == Maker::Dlss) as u8), Ordering::Relaxed);
	refresh();
}

/// Whether the env RAC_FG forces a generation (tests): it then runs whatever the Upscaler and the menu say.
pub(crate) fn forced() -> bool {
	static F: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*F.get_or_init(|| std::env::var("RAC_FG").is_ok_and(|v| !v.trim().is_empty()))
}

fn cap(v: i32) -> bool {
	choice::offered(v)
}

/// The Upscaler value the slot really runs (upscale_choice `kind_in_force`: the env RAC_UPSCALE and a refused slot included).
fn upscaler() -> i32 {
	up::kind_in_force() as i32
}

/// The Upscaler value whose maker runs the frame generation in question (pure; the Frame Generation row's fix button): the
/// chosen value's maker; nothing chosen: the maker this run started; else the first upscaler this PC runs (`up_offered`).
pub fn upscaler_for(chosen: u8, running: Option<Maker>, up_offered: &impl Fn(i32) -> bool) -> i32 {
	match maker_of(chosen as i32).or(running) {
		Some(Maker::Dlss) => up::DLSS_VALUE,
		Some(Maker::Fsr) => up::FFX_VALUE,
		None => [up::FFX_VALUE, up::DLSS_VALUE].into_iter().find(|&v| up_offered(v)).unwrap_or(up::FFX_VALUE),
	}
}

/// The Upscaler value the Frame Generation row's fix button sets now (`upscaler_for`; the menu's NeedsRule `fix`).
pub fn fix_upscaler() -> i32 {
	upscaler_for(choice::chosen_raw(), running(), &up::offered)
}

/// Render thread: whether frame generation can run this frame (forced by RAC_FG, or a value in force): the motion vector and
/// depth passes without an upscaler (upscale.rs `Mode::Still`) only pay off then.
pub(crate) fn generates() -> bool {
	forced() || in_force(choice::chosen_raw(), upscaler(), &cap, running()) != 0
}

/// Whether row value `v` is on offer now (rule A on this PC): the menus step over the others.
pub fn offered_now(v: i32) -> bool {
	offered_with(v, upscaler(), &cap)
}

/// What the row shows now (rule A; a value of the other maker or the Upscaler Off: Off).
pub fn shown_now() -> u8 {
	shown_with(choice::chosen_raw(), upscaler(), &cap)
}

/// Whether value `v` would need a restart to run (the classic row's "- restart").
pub fn restart_for(v: i32) -> bool {
	needs_restart(v, running())
}

/// Whether the Upscaler is Off (the row is dimmed and locked).
pub fn needs_upscaler() -> bool {
	upscaler() == 0
}

/// Whether any value but Off is on offer now.
pub fn available_now() -> bool {
	(1..choice::KINDS.len() as i32).any(offered_now)
}

/// Render thread: why the running maker `m`'s generation pauses this frame (None: it runs; a forced env run never pauses here).
pub(crate) fn pause_why(m: Maker) -> Option<Why> {
	if forced() {
		return None;
	}
	why(choice::chosen_raw(), upscaler(), &cap, Some(m))
}

/// Render thread: the DLSS value in force (2..4) while DLSS generation runs and the menu sets it (None: keep the started one).
pub(crate) fn dlss_value() -> Option<u8> {
	if forced() {
		return None;
	}
	let v = in_force(choice::chosen_raw(), upscaler(), &cap, running());
	(maker_of(v as i32) == Some(Maker::Dlss)).then_some(v)
}

/// Publishes rule A to the menus (offers and the note) and logs one line when the state changed: chosen, in force, why paused,
/// live or restart. Called by every change of the choice, the Upscaler, what the PC runs, and the running maker.
pub fn refresh() {
	// Held for the whole body: two threads never publish offers from two snapshots out of order.
	let mut last = LAST.lock().unwrap_or_else(|e| e.into_inner());
	let (u, chosen, run) = (upscaler(), choice::chosen_raw(), running());
	for v in 1..choice::KINDS.len() as i32 {
		crate::ui::live::offer(choice::KEY, v, offered_with(v, u, &cap));
	}
	let n = note(chosen, u, &cap, run, choice::no_card(), choice::missing());
	crate::ui::live::set_note(choice::KEY, Some(note_text(n)));
	let name = |v: u8| choice::KINDS.get(v as usize).copied().unwrap_or("?");
	let f = in_force(chosen, u, &cap, run);
	let why = if forced() { None } else { why(chosen, u, &cap, run) };
	let line = format!(
		"fgen: Frame Generation: chosen {}, in force {}{}, upscaler {}, {} ({:?})",
		name(chosen),
		if forced() { "env RAC_FG" } else { name(f) },
		why.map_or(String::new(), |w| format!(" (paused: {})", w.text())),
		up::KINDS.get(u as usize).copied().unwrap_or("?"),
		if needs_restart(shown_with(chosen, u, &cap) as i32, run) { "restart" } else { "live" },
		n,
	);
	if *last != line {
		eprintln!("{line}");
		*last = line;
	}
}

/// The steps of RAC_FG_TEST_STEPS (pure): `frame:fg=V` / `frame:up=V` separated by `;` (V = row value). Junk entries are skipped.
pub(crate) fn parse_steps(s: &str) -> Vec<(u64, bool, i32)> {
	s.split(';')
		.filter_map(|e| {
			let (f, kv) = e.trim().split_once(':')?;
			let (k, v) = kv.split_once('=')?;
			let up = match k.trim() {
				"fg" => false,
				"up" => true,
				_ => return None,
			};
			Some((f.trim().parse().ok()?, up, v.trim().parse().ok()?))
		})
		.collect()
}

/// [Test] RAC_FG_TEST_STEPS (lane fglive's live proofs, no menu needed): at the render thread's frame N the Frame Generation or
/// Upscaler row is set as the menu sets it (a throwaway options set: the player's file is never written). Unset: nothing.
pub(crate) fn test_tick() {
	use std::sync::atomic::AtomicU64;
	static STEPS: std::sync::OnceLock<Vec<(u64, bool, i32)>> = std::sync::OnceLock::new();
	static N: AtomicU64 = AtomicU64::new(0);
	let steps = STEPS.get_or_init(|| std::env::var("RAC_FG_TEST_STEPS").map(|s| parse_steps(&s)).unwrap_or_default());
	if steps.is_empty() {
		return;
	}
	let n = N.fetch_add(1, Ordering::Relaxed);
	for &(_, is_up, v) in steps.iter().filter(|s| s.0 == n) {
		let mut o = crate::pcopts::PcOptions::new();
		eprintln!("fgen: RAC_FG_TEST_STEPS frame {n}: {} {v}", if is_up { "upscaler" } else { "framegen" });
		if is_up {
			up::set_kind(&mut o, v);
		} else {
			choice::set(&mut o, v);
		}
	}
}

#[cfg(test)]
pub(crate) fn reset_for_test() {
	RUNNING.store(0, Ordering::Relaxed);
}

#[cfg(test)]
#[path = "follow_tests.rs"]
mod tests;
