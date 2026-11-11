//! The classic menu's copies of four rows the Modern menu style had alone (v0.5.1, lanes rows and fload): Aspect ratio and Upscaler
//! on the Graphics page's Display section (after Frame smoothing), Quick Select pauses the game and Fast travel on its Gameplay
//! section (the modern Game tab's Gameplay group). Each classic row changes exactly what the modern row changes: the same engine
//! state and the same options file key (`aspect_box` key `aspect`, `upscale_choice` key `upscaler`, `qs_pause` key `qspause`,
//! `fast_travel` key `fasttravel`; `pcext/ui_bridge_more.rs` is the modern side).
//! The Upscale quality row stays modern-only (the quality keeps the file's value, default Quality).
//!
//! Built like Metal reflections (pcmrefl.rs): Aspect and Upscaler are multi-choice rows whose two choices are the same dynamic text
//! ("< 16:9 >"); retail's update toggles the row byte on Cross, which the host reads as "next value" (the last wraps to the first);
//! Left / Right (pad bits, also the mouse wheel) are read while the row is the page's cursor row and stop at the ends. The Upscaler
//! skips a value this PC cannot run (`upscale_choice::offered`). Quick Select pause and Fast travel are plain Off / On rows (retail's
//! own texts) whose byte `sync` compares with the setting, like Supporter drawings (pcsupp.rs). Fast travel's byte shows what acts
//! in this run (`fast_travel::shown`: Off in a replay / test run without the key; tb5 review N1). No row ever writes retail's data.
//!
//! RAM: the row bytes `UPS_B`, `ASP_B`, `QSP_B`, `FT_B` (pcctl's block, CBASE + 0x2a ..= 0x2d, default 0 = today's RAM) and the text
//! ids 0x7f98 ..= 0x7f9d, six more slots at the end of pcdisp's pool (`pcdisp::serves` / `pcdisp::slot` route them; 0x7fc0.. is the
//! Mods page's).
use super::pcctl::{self, CBASE};
use super::pcopts::PcOptions;
use ee_oracle::mem::Mem;
use rac_engine::viewer::{aspect_box, upscale_choice};
use std::sync::atomic::{AtomicI8, Ordering};

/// The rows' toggle bytes: retail flips them 0 <-> 1 on Cross (the host reads a change as one step).
pub(super) const ASP_B: u32 = CBASE + 0x2a;
pub(super) const UPS_B: u32 = CBASE + 0x2b;
pub(super) const QSP_B: u32 = CBASE + 0x2c;
pub(super) const FT_B: u32 = CBASE + 0x2d;
/// Text ids: Aspect ratio label and value, Upscaler label and value, Quick Select pause label, Fast travel label.
pub(super) const ID_ASPECT: u32 = 0x7f98;
pub(super) const ID_VASPECT: u32 = 0x7f99;
pub(super) const ID_UPS: u32 = 0x7f9a;
pub(super) const ID_VUPS: u32 = 0x7f9b;
pub(super) const ID_QSP: u32 = 0x7f9c;
pub(super) const ID_FT: u32 = 0x7f9d;
/// The ids pcdisp's pool serves through this file (`text`): its six, then the Frame Generation row's two (pcfgen.rs).
pub(super) const IDS: [u32; 8] = [ID_ASPECT, ID_VASPECT, ID_UPS, ID_VUPS, ID_QSP, ID_FT, super::pcfgen::ID_FG, super::pcfgen::ID_VFG];

type T5 = [&'static [u8]; 5];
// Game font bytes: e-acute 0x8c, a-grave 0x85, a-acute 0x84, a-umlaut 0x87, o-acute 0x9c. [I] wording like the modern rows, native review wanted.
#[rustfmt::skip]
static ASPECT: T5 = [b"Aspect ratio", b"Format d'image", b"Seitenverh\x87ltnis", b"Relaci\x9cn de aspecto", b"Proporzioni"];
#[rustfmt::skip]
static UPSCALER: T5 = [b"Upscaler", b"Mise \x85 l'\x8cchelle", b"Hochskalierung", b"Escalado", b"Upscaling"];
#[rustfmt::skip]
static QSPAUSE: T5 = [b"Quick Select pause", b"Pause s\x8clect. rapide", b"Schnellwahl-Pause", b"Pausa selecc. r\x84pida", b"Pausa selez. rapida"];
#[rustfmt::skip]
static FTRAVEL: T5 = [b"Fast travel", b"Voyage rapide", b"Schnellreise", b"Viaje r\x84pido", b"Viaggio rapido"];
/// "Off" like the modern menu's (rows_upscale.rs `OFF`).
static OFF: T5 = [b"Off", b"Non", b"Aus", b"No", b"No"];
/// The Aspect ratio values in the row's order (`aspect_box::ALL`); the first is "Auto" in every language.
const ASPECT_NAMES: [&str; 6] = ["Auto", "4:3", "16:9", "16:10", "21:9", "32:9"];
/// The Upscaler values (the product names are the same in every language).
const UPS_NAMES: [&str; 3] = ["Off", "FSR", "DLSS"];

/// The row values pending from the menu (-1 = none): `field` records a step, `sync` applies it with the options file in hand.
static PEND_ASP: AtomicI8 = AtomicI8::new(-1);
static PEND_UPS: AtomicI8 = AtomicI8::new(-1);

/// The Aspect ratio now (row value 0 Auto .. 5 32:9).
fn aspect_now() -> i32 {
	let p = PEND_ASP.load(Ordering::Relaxed);
	if p >= 0 { p as i32 } else { aspect_box::current() as i32 }
}

/// The Upscaler now (0 Off, 1 FSR, 2 DLSS; Off while the chosen one cannot run here).
fn ups_now() -> i32 {
	let p = PEND_UPS.load(Ordering::Relaxed);
	if p >= 0 { p as i32 } else { upscale_choice::kind() as i32 }
}

/// A step of a row.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Step {
	Left,
	Right,
	Cross,
}

/// The value after one step from `cur` among `n` values, of which `ok(v)` can be taken: Left / Right stop at the ends, Cross wraps.
/// The current value when nothing else can be taken.
pub(super) fn step(cur: i32, n: i32, s: Step, ok: impl Fn(i32) -> bool) -> i32 {
	let dir = if s == Step::Left { -1 } else { 1 };
	let mut v = cur;
	loop {
		v += dir;
		if v < 0 || v >= n {
			if s != Step::Cross {
				return cur;
			}
			v = if v < 0 { n - 1 } else { 0 };
		}
		if v == cur || ok(v) {
			return v;
		}
	}
}

/// The row's index in the Display section's table (row 0 is the section stepper), for `pcdisp::cursor_row`.
fn row_in_display(row: usize) -> Option<u32> {
	pcctl::gfx_section(pcctl::GFX_DISPLAY).iter().position(|&r| r == row).map(|p| p as u32 + 1)
}

/// An Off / On row whose byte is the setting: its byte, the setting now, the setter of both rows (true = the options file changed).
struct Toggle {
	byte: u32,
	on: fn() -> bool,
	set: fn(&mut PcOptions, bool) -> bool,
}

/// The Off / On rows: Quick Select pause, Fast travel (`TOG_SEEN` keeps their order).
const TOGGLES: [Toggle; 2] = [
	Toggle { byte: QSP_B, on: super::qs_pause::on, set: super::qs_pause::set_from_ui },
	Toggle { byte: FT_B, on: super::fast_travel::shown, set: super::fast_travel::set_from_ui },
];

thread_local! {
	/// The two bytes as last seen (0xff: not known yet). Per thread, like pcdisp's.
	static SEEN: std::cell::Cell<[u8; 2]> = const { std::cell::Cell::new([0xff; 2]) };
	/// The Off / On rows' bytes as `sync` last saw them (0xff: not known yet), so `sync` acts on a change of a byte only.
	static TOG_SEEN: std::cell::Cell<[u8; 2]> = const { std::cell::Cell::new([0xff; 2]) };
}

/// The Off / On rows' bytes = their settings, and seen as such (`build`, `follow_ui`).
fn toggles_to_settings(mem: &mut Mem) {
	let mut seen = [0xff; 2];
	for (k, t) in TOGGLES.iter().enumerate() {
		seen[k] = (t.on)() as u8;
		mem.wr8(t.byte, seen[k]);
	}
	TOG_SEEN.with(|s| s.set(seen));
}

/// `pcctl::build_gfx` on RAM that never held the page: the steppers start at 0, the Off / On rows at their settings.
pub(super) fn build(mem: &mut Mem) {
	mem.wr8(ASP_B, 0);
	mem.wr8(UPS_B, 0);
	super::pcfgen::build(mem);
	toggles_to_settings(mem);
	SEEN.with(|s| s.set([0xff; 2]));
}

/// Worker, each field while the Graphics page shows the Display section (`pcctl::field`, the title's `sync`): the two steppers.
/// `dir` is Left / Right this field (auto-repeated).
pub(super) fn field(mem: &mut Mem, level: u32, dir: [bool; 2]) {
	let mut cursor = None;
	for (k, (addr, row)) in [(ASP_B, pcctl::ASP_ROW), (UPS_B, pcctl::UPS_ROW)].into_iter().enumerate() {
		let sel = mem.rd8(addr);
		let seen = SEEN.with(|s| {
			let mut v = s.get();
			let old = std::mem::replace(&mut v[k], sel);
			s.set(v);
			old
		});
		let cross = seen != 0xff && seen != sel;
		let held = dir[0] != dir[1];
		if !cross && !(held && *cursor.get_or_insert_with(|| super::pcdisp::cursor_row(mem, level)) == row_in_display(row)) {
			continue;
		}
		let s = if cross { Step::Cross } else if dir[0] { Step::Left } else { Step::Right };
		if k == 0 {
			let next = step(aspect_now(), aspect_box::ALL.len() as i32, s, |_| true);
			PEND_ASP.store(next as i8, Ordering::Relaxed);
		} else {
			let next = step(ups_now(), UPS_NAMES.len() as i32, s, upscale_choice::offered);
			PEND_UPS.store(next as i8, Ordering::Relaxed);
		}
	}
}

/// Worker, every field: a step goes into the engine and the options file (true = the caller saves). Quick Select pause, Fast travel:
/// the byte the classic row flipped is the setting.
pub fn sync(mem: &mut Mem, opts: &mut PcOptions) -> bool {
	let mut save = false;
	let a = PEND_ASP.swap(-1, Ordering::Relaxed);
	if a >= 0 {
		save |= aspect_box::set_from_ui(opts, a as i32);
	}
	let u = PEND_UPS.swap(-1, Ordering::Relaxed);
	if u >= 0 {
		save |= upscale_choice::set_kind(opts, u as i32);
	}
	save |= super::pcfgen::sync(opts);
	if pcctl::gfx_built(mem) {
		let now: [u8; 2] = std::array::from_fn(|k| mem.rd8(TOGGLES[k].byte));
		// A change of a byte since the last look is a press of the row; a byte that merely differs from the setting is not.
		let old = TOG_SEEN.with(|s| s.replace(now));
		for (k, t) in TOGGLES.iter().enumerate() {
			if old[k] != 0xff && old[k] != now[k] {
				save |= (t.set)(opts, now[k] != 0);
			}
		}
	} else {
		TOG_SEEN.with(|s| s.set([0xff; 2]));
	}
	save
}

/// The modern screen changed one of these settings (`pcext/ui_bridge_more.rs`): the classic page's Quick Select pause and Fast
/// travel bytes follow (else `sync` would read a differing byte as a press and undo the change).
pub(super) fn follow_ui(mem: &mut Mem) {
	if pcctl::gfx_built(mem) {
		toggles_to_settings(mem);
	}
}

/// The text of `id` in language `l` (0 En .. 4 It); empty for any other id.
pub fn text(id: u32, l: usize) -> Vec<u8> {
	let l = l.min(4);
	let stepper = |s: &[u8]| [b"< ".as_slice(), s, b" >"].concat();
	match id {
		ID_ASPECT => ASPECT[l].to_vec(),
		ID_UPS => UPSCALER[l].to_vec(),
		ID_QSP => QSPAUSE[l].to_vec(),
		ID_FT => FTRAVEL[l].to_vec(),
		super::pcfgen::ID_FG | super::pcfgen::ID_VFG => super::pcfgen::text(id, l),
		ID_VASPECT => stepper(ASPECT_NAMES[aspect_now().clamp(0, 5) as usize].as_bytes()),
		ID_VUPS => match ups_now().clamp(0, 2) as usize {
			0 => stepper(OFF[l]),
			n => stepper(UPS_NAMES[n].as_bytes()),
		},
		_ => Vec::new(),
	}
}

/// A number that changes with the two live values (`pcdisp::text_key`).
pub(super) fn text_key() -> u64 {
	(aspect_now() as u64) << 8 | ups_now() as u64
}

/// Test hook: nothing pending, bytes unseen.
#[cfg(test)]
pub(super) fn reset_statics() {
	PEND_ASP.store(-1, Ordering::Relaxed);
	PEND_UPS.store(-1, Ordering::Relaxed);
	SEEN.with(|s| s.set([0xff; 2]));
}

#[cfg(test)]
#[path = "pcmore_tests.rs"]
mod tests;
