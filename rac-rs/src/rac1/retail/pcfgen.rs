//! The classic menu's Frame Generation row (lane fgen, stage S4): Graphics page, Screen section, after Renderer (both apply after a
//! restart). It changes exactly what the modern row changes (modern_menu/rows_upscale.rs, `pcext/ui_bridge_more.rs`): the engine's
//! choice `rac_engine::viewer::fgen_choice`, options file key `framegen`.
//!
//! Built like the Upscaler row (pcmore.rs): a multi-choice row whose two value texts are the same dynamic text ("< FSR 2x >");
//! retail's update toggles the row byte on Cross, which the host reads as "next value" (the last wraps to the first); Left / Right
//! are read while the row is the section's cursor row and stop at the ends; a value this PC cannot run is skipped. Lane fglive:
//! the values on offer follow the running Upscaler like the modern row's (engine `fgen_follow`): with the Upscaler Off the row
//! does not step and reads "Off" while nothing is chosen (the default: the fork point's RAM bytes), else "Needs an upscaler"; a value that needs a restart (a maker this run did not start) reads
//! "... - restart" (like Renderer, pcrender.rs), the others apply at once; with no value but Off offered the row reads
//! "Not available".
//!
//! RAM: the row byte `FG_B` (pcctl's block, CBASE + 0x2e, default 0 = today's RAM) and the text ids 0x7f9e / 0x7f9f, two more slots
//! of pcdisp's pool after pcmore's six (`pcmore::IDS`). No row ever writes retail's data.
use super::pcctl::{self, CBASE};
use super::pcmore::{step, Step};
use super::pcopts::PcOptions;
use ee_oracle::mem::Mem;
use rac_engine::viewer::fgen_choice as fg;
use rac_engine::viewer::fgen_follow as fl;
use std::sync::atomic::{AtomicI8, Ordering};

/// The row's toggle byte: retail flips it 0 <-> 1 on Cross (the host reads a change as one step).
pub(super) const FG_B: u32 = CBASE + 0x2e;
/// Text ids: the label and the value.
pub(super) const ID_FG: u32 = 0x7f9e;
pub(super) const ID_VFG: u32 = 0x7f9f;

type T5 = [&'static [u8]; 5];
// Game font bytes: e-acute 0x8c, o-acute 0x9c (pcmore.rs). [I] wording like the modern row, native review wanted.
#[rustfmt::skip]
static LABEL: T5 = [b"Frame Generation", b"G\x8cn\x8cration d'images", b"Frame-Generierung", b"Gen. de fotogramas", b"Generazione frame"];
/// "Off" like the modern menu's (modern_menu/text.rs `OFF`).
static OFF: T5 = [b"Off", b"Non", b"Aus", b"No", b"No"];
/// The row while the Upscaler is Off (lane fglive). [I] wording, native review wanted.
#[rustfmt::skip]
pub(super) static NEEDS_UPSCALER: T5 = [b"Needs an upscaler", b"Mise \x85 l'\x8cch. requise", b"Hochskalierung n\x9ftig", b"Requiere escalado", b"Richiede upscaling"];
/// The row with no value but Off this PC can run.
#[rustfmt::skip]
pub(super) static NOT_AVAILABLE: T5 = [b"Not available", b"Indisponible", b"Nicht vorhanden", b"No disponible", b"Non disponibile"];
/// The values after Off in `fg_choice::KINDS` order (product names, the same in every language): FSR 2x, DLSS 2x / 3x / 4x (S3).
pub(super) const NAMES: [&str; 4] = ["FSR 2x", "DLSS 2x", "DLSS 3x", "DLSS 4x"];

/// The row value pending from the menu (-1 = none): `field` records a step, `sync` applies it with the options file in hand.
static PEND: AtomicI8 = AtomicI8::new(-1);

thread_local! {
	/// The byte as last seen (0xff: not known yet). Per thread, like pcmore's.
	static SEEN: std::cell::Cell<u8> = const { std::cell::Cell::new(0xff) };
}

/// The row value now (0 Off, 1 FSR 2x, ...).
fn now() -> i32 {
	let p = PEND.load(Ordering::Relaxed);
	if p >= 0 { p as i32 } else { fl::shown_now() as i32 }
}

/// `pcctl::build_gfx` on RAM that never held the page: the byte starts at 0.
pub(super) fn build(mem: &mut Mem) {
	mem.wr8(FG_B, 0);
	SEEN.with(|s| s.set(0xff));
}

/// Worker, each field while the Graphics page shows the Screen section (`pcctl::field`, the title's `sync`). `dir` is Left / Right
/// this field (auto-repeated).
pub(super) fn field(mem: &mut Mem, level: u32, dir: [bool; 2]) {
	let sel = mem.rd8(FG_B);
	let seen = SEEN.with(|s| s.replace(sel));
	let cross = seen != 0xff && seen != sel;
	let held = dir[0] != dir[1];
	if !cross && !(held && super::pcdisp::cursor_row(mem, level) == row_in_screen()) {
		return;
	}
	let s = if cross { Step::Cross } else if dir[0] { Step::Left } else { Step::Right };
	let (cur, next) = (now(), step(now(), fg::KINDS.len() as i32, s, fl::offered_now));
	if next != cur {
		PEND.store(next as i8, Ordering::Relaxed);
	}
}

/// The row's index in the Screen section's table (row 0 is the section stepper), for `pcdisp::cursor_row`.
fn row_in_screen() -> Option<u32> {
	pcctl::gfx_section(pcctl::SEC_SCREEN).iter().position(|&r| r == pcctl::FG_ROW).map(|p| p as u32 + 1)
}

/// Worker, every field (from `pcmore::sync`): a step goes into the engine and the options file (true = the caller saves).
pub(super) fn sync(opts: &mut PcOptions) -> bool {
	let v = PEND.swap(-1, Ordering::Relaxed);
	v >= 0 && fg::set(opts, v as i32)
}

/// The value text in language `l`: "< FSR 2x >", "< DLSS 2x - restart >" when it needs a restart, "< Needs an upscaler >",
/// "< Not available >".
pub(super) fn value_text(l: usize) -> Vec<u8> {
	let l = l.min(4);
	let mut t = match now() {
		// Review 6: the Upscaler Off with nothing chosen (the default options) reads as before lane fglive ("Off" / "Not
		// available": the same EE RAM bytes), the modern row carries the note; a chosen frame generation paused by it says why.
		_ if idle_default() => if fg::available() { OFF[l] } else { NOT_AVAILABLE[l] }.to_vec(),
		_ if fl::needs_upscaler() => NEEDS_UPSCALER[l].to_vec(),
		_ if !fl::available_now() => NOT_AVAILABLE[l].to_vec(),
		0 => OFF[l].to_vec(),
		n => NAMES.get(n as usize - 1).map_or(OFF[l], |s| s.as_bytes()).to_vec(),
	};
	if !fl::needs_upscaler() && fl::available_now() && fl::restart_for(now()) {
		t.extend_from_slice(super::pcrender::RESTART[l].as_bytes());
	}
	[b"< ".as_slice(), &t, b" >"].concat()
}

/// The Upscaler is Off and no frame generation is chosen or pending (the default options): the row reads as at the fork point.
fn idle_default() -> bool {
	fl::needs_upscaler() && fg::chosen_raw() == 0 && PEND.load(Ordering::Relaxed) < 0
}

/// The text of `id` in language `l` (0 En .. 4 It); empty for any other id.
pub fn text(id: u32, l: usize) -> Vec<u8> {
	match id {
		ID_FG => LABEL[l.min(4)].to_vec(),
		ID_VFG => value_text(l),
		_ => Vec::new(),
	}
}

/// A number (7 bits) that changes with the shown value (3 bits), the Upscaler being Off, its "- restart" note and whether any value
/// is offered (`pcdisp::text_key`).
pub(super) fn text_key() -> u64 {
	if idle_default() {
		return (fg::started() as u64 & 0x7) << 1 | fg::available() as u64; // the fork point's key in this state
	}
	(now() as u64 & 0x7) << 4 | (fl::needs_upscaler() as u64) << 3 | (fl::restart_for(now()) as u64) << 2 | fl::available_now() as u64
}

/// Test hook: nothing pending, byte unseen.
#[cfg(test)]
pub(super) fn reset_statics() {
	PEND.store(-1, Ordering::Relaxed);
	SEEN.with(|s| s.set(0xff));
}

#[cfg(test)]
#[path = "pcfgen_tests.rs"]
mod tests;
