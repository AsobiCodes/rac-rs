//! Graphics -> More effects -> Metal reflections (classic menu; lane nslide, 2026-11-03, user: "make the metal 25% less reflective",
//! then "add a slider"). The row sits first in the "More effects" section (the Effects section is full at seven rows). The value
//! (0..=100 %, steps of 5, default 75) is `rac_engine::metal_option::reflect_pct`; the metal look's reflection gain is scaled by it
//! (engine `viewer/metalenv.rs` `knobs`) and the player's key is `metal_reflect` (pcopts.rs).
//!
//! Built like the Display steppers (pcdisp.rs): a multi-choice row whose two choices are the same dynamic text (`ID_VMREFL`,
//! "< 75 >"); retail's update toggles the row's byte `REFL_B` on Cross, which the host reads as "+5" (100 wraps to 0); Left / Right
//! (pad bits, also the mouse wheel) are read while the row is the page's cursor row. The row never writes retail's data.
//! Text ids 0x7fb9 (label) and 0x7fbe (value) are free slots of pcdisp's pool (`pcdisp::text` hands them here).
use super::pcctl::CBASE;
use super::pcopts::PcOptions;
use ee_oracle::mem::Mem;
use rac_engine::metal_option as mo;
use std::sync::atomic::{AtomicU8, Ordering};

/// The row's toggle byte (pcctl's block; retail flips it 0 <-> 1 on Cross).
pub(super) const REFL_B: u32 = CBASE + 0x20;
pub(super) const ID_MREFL: u32 = 0x7fb9;
pub(super) const ID_VMREFL: u32 = 0x7fbe;
/// The ids this file words.
pub(super) const IDS: [u32; 2] = [ID_MREFL, ID_VMREFL];
/// The row's index in the "More effects" table (0 is the section stepper).
const ROW: u32 = 1;

#[rustfmt::skip]
static LABEL: [&[u8]; 5] = [b"Metal reflections", b"Reflets du m\x8ctal", b"Metallspiegelung", b"Reflejos del metal", b"Riflessi del metallo"];

/// A step of the row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Step {
	Left,
	Right,
	Cross,
}

/// The percent after one step from `cur`: Left -5 / Right +5 stop at 0 / 100, Cross +5 wraps from 100 to 0.
pub(super) fn step(cur: u8, s: Step) -> u8 {
	let cur = super::pcopts::snap_metal_refl(cur);
	match s {
		Step::Left => cur.saturating_sub(5),
		Step::Right => (cur + 5).min(100),
		Step::Cross if cur >= 100 => 0,
		Step::Cross => cur + 5,
	}
}

/// The value changed since `sync` looked (the options file needs it).
static DIRTY: AtomicU8 = AtomicU8::new(0);
/// The row byte as last seen (0xff: not known yet). Per thread, like pcdisp's.
thread_local! { static SEEN: std::cell::Cell<u8> = const { std::cell::Cell::new(0xff) }; }

/// `pcext::init`: the file's value is the row's start value.
pub fn init(opts: &PcOptions) {
	mo::set_reflect(opts.metal_reflect_pct());
	DIRTY.store(0, Ordering::Relaxed);
	SEEN.with(|s| s.set(0xff));
}

/// `pcctl::build_gfx` on RAM that never held the page: the byte starts at 0.
pub(super) fn build(mem: &mut Mem) {
	mem.wr8(REFL_B, 0);
	SEEN.with(|s| s.set(0xff));
}

fn set(pct: u8) {
	mo::set_reflect(pct);
	DIRTY.store(1, Ordering::Relaxed);
	eprintln!("retail: PC options: metal reflections {pct}%");
}

/// The modern screen set the percent (`pcext/ui_bridge.rs`); the file follows in `sync`.
pub(super) fn set_from_ui(pct: u8) {
	let pct = super::pcopts::snap_metal_refl(pct);
	if mo::reflect_pct() != pct {
		set(pct);
	}
}

/// Worker, each field while the Graphics page shows the More effects section (`pcctl::field`): the row's steps. `dir` is Left / Right
/// this field (auto-repeated).
pub(super) fn field(mem: &mut Mem, level: u32, dir: [bool; 2]) {
	let sel = mem.rd8(REFL_B);
	let seen = SEEN.with(|s| s.replace(sel));
	let cross = seen != 0xff && seen != sel;
	let held = dir[0] != dir[1];
	if !cross && !(held && super::pcdisp::cursor_row(mem, level) == Some(ROW)) {
		return;
	}
	let s = if cross { Step::Cross } else if dir[0] { Step::Left } else { Step::Right };
	let (now, next) = (mo::reflect_pct(), step(mo::reflect_pct(), s));
	if next != now {
		set(next);
	}
}

/// Worker, every field: a change goes into the options file (true = the caller saves).
pub fn sync(opts: &mut PcOptions) -> bool {
	if DIRTY.swap(0, Ordering::Relaxed) == 0 {
		return false;
	}
	opts.metal_reflect = Some(mo::reflect_pct());
	true
}

/// The text of `id` in language `l` (0 En .. 4 It); empty for any other id.
pub fn text(id: u32, l: usize) -> Vec<u8> {
	match id {
		ID_MREFL => LABEL[l.min(4)].to_vec(),
		ID_VMREFL => format!("< {} >", mo::reflect_pct()).into_bytes(),
		_ => Vec::new(),
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn steps_stop_at_the_ends_and_cross_wraps() {
		assert_eq!((step(100, Step::Right), step(0, Step::Left)), (100, 0));
		assert_eq!((step(75, Step::Left), step(75, Step::Right)), (70, 80));
		assert_eq!((step(95, Step::Cross), step(100, Step::Cross), step(0, Step::Cross)), (100, 0, 5));
	}

	#[test]
	fn texts_and_options_round_trip() {
		let _g = crate::retail::pcopts::globals_guard();
		init(&PcOptions::new());
		assert_eq!(mo::reflect_pct(), 75);
		assert_eq!(text(ID_VMREFL, 0), b"< 75 >");
		assert_eq!(text(ID_MREFL, 0), b"Metal reflections");
		assert!(LABEL.iter().all(|l| l.len() <= 22));
		assert!(text(0x7fc0, 0).is_empty());
		let mut opts = PcOptions::new();
		assert!(!sync(&mut opts), "nothing changed: no save");
		set_from_ui(40);
		assert_eq!((mo::reflect_pct(), text(ID_VMREFL, 1)), (40, b"< 40 >".to_vec()));
		assert!(sync(&mut opts) && !sync(&mut opts));
		assert_eq!(opts.to_text(), "version=1\nmetal_reflect=40\n");
		init(&PcOptions::parse(&opts.to_text()));
		assert_eq!(mo::reflect_pct(), 40);
		init(&PcOptions::new());
		assert_eq!(mo::reflect_pct(), 75);
	}
}
