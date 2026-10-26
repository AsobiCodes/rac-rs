//! The Camera Options page's Mouse speed row (row 3, right below Camera speed; lane cam, 2026-10-23, player report: one number for
//! stick and mouse is far too fast for the mouse at the stick's good value).
//!
//! The row is built like Camera speed (camopts.rs): a multi-choice row whose two choices are the same dynamic text (`ID_MPCT`,
//! "< 50 >"); retail's update toggles the row's byte `MOUSESEL` on Cross, which the host reads as "+10" (200 wraps to 10); Left /
//! Right (pad bits, also the mouse wheel) are read by the host while the row is the page's cursor row: -10 / +10 with auto-repeat.
//! The value is the player's own key `mousespeed` (pcopts.rs): the Modern camera's mouse turn is scaled by percent / 50
//! (modcam.rs `step`, `mousespeed_scale`), exactly 1.0 without the key (= today's mouse feel, 50 shown). The row has no retail save
//! byte and never writes retail's data: only `MOUSESEL` (PC menu area) changes with the player's presses.
//!
//! Reserved RAM: the row byte `MOUSESEL` (BASE + 0x3a) and the two text slots of ids 0x7f85 / 0x7f86 in the camopts pool.
use super::*;
use std::sync::atomic::AtomicU8;

/// The Mouse speed row's byte: retail toggles it 0 <-> 1 on every Cross; the host reads each change as one +10 step.
pub(super) const MOUSESEL: u32 = BASE + 0x3a;
/// The row's label and its value text "< 50 >" (built from the current percent on every lookup).
pub const ID_MOUSE: u32 = 0x7f85;
pub const ID_MPCT: u32 = 0x7f86;
/// The row index on the Camera Options page (right below Camera speed, row 2).
pub(super) const MOUSE_ROW: u32 = SPEED_ROW + 1;

/// The label per language (En, Fr, De, Es, It) in the font bytes (Es o-acute 0x9c, It a-grave 0x85). [I] wording, native review
/// wanted; the longest (De 19) is no longer than the Graphics page's "Umgebungsverdeckung" / "Occlusione ambientale" (21).
#[rustfmt::skip]
static LABEL: [&[u8]; 5] = [b"Mouse speed", b"Vitesse souris", b"Mausgeschwindigkeit", b"Velocidad del rat\x9cn", b"Velocit\x85 mouse"];

/// The player's mouse speed in percent (`mousespeed` key); 0 = never set (50, factor exactly 1.0).
static MOUSESPEED: AtomicU8 = AtomicU8::new(0);
/// The percent the row shows now (the key, else 50).
static MOUSESHOW: AtomicU8 = AtomicU8::new(50);
/// `MOUSESEL` as last seen (0xff: not known yet, the next look is the baseline).
static MOUSESEL_SEEN: AtomicU8 = AtomicU8::new(0xff);

/// The Modern camera's mouse turn scale (modcam.rs `step`): the Mouse speed row's percent / 50, exactly 1.0 without the key.
pub fn mousespeed_scale() -> f32 {
	camspeed_factor(MOUSESPEED.load(Ordering::Relaxed))
}

/// The percent the row's value text shows.
pub(super) fn shown() -> u8 {
	MOUSESHOW.load(Ordering::Relaxed)
}

/// The row's text in `lang` (0 En .. 4 It); the value text uses `pct`.
pub(super) fn string(id: u32, lang: usize, pct: u8) -> Option<Vec<u8>> {
	match id {
		ID_MOUSE => Some(LABEL[lang.min(4)].to_vec()),
		ID_MPCT => Some(pct_text(pct)),
		_ => None,
	}
}

/// `init` / `setup`: the options file's value is known; the next look at the row byte is the baseline.
pub(super) fn load(opts: &PcOptions) {
	MOUSESPEED.store(opts.mousespeed.map_or(0, pcopts::snap_camspeed), Ordering::Relaxed);
	MOUSESHOW.store(opts.mousespeed.map_or(50, pcopts::snap_camspeed), Ordering::Relaxed);
	MOUSESEL_SEEN.store(0xff, Ordering::Relaxed);
}

/// Worker, each field of a level, after `camopts_field` read the buttons: the row's steps (Cross: retail toggled `MOUSESEL`;
/// Left / Right: `dir`, the auto-repeated pad bits).
pub(super) fn field(mem: &mut Mem, level: u32, dir: [bool; 2], s: &mut St) {
	let key = MOUSESPEED.load(Ordering::Relaxed);
	let mut now = if key != 0 { key } else { 50 };
	let sel = mem.rd8(MOUSESEL);
	let seen = MOUSESEL_SEEN.swap(sel, Ordering::Relaxed);
	let cross = seen != 0xff && seen != sel;
	if (cross || dir[0] != dir[1]) && on_row(mem, level, MOUSE_ROW) {
		let next = camspeed_step(now, step_of(cross, dir));
		if next != now {
			MOUSESPEED.store(next, Ordering::Relaxed);
			s.opts.mousespeed = Some(next);
			s.save();
			eprintln!("retail: PC options: mouse speed {next}%");
			now = next;
		}
	}
	MOUSESHOW.store(now, Ordering::Relaxed);
}

/// Test hook: the row's process-wide state back to "never set".
#[cfg(test)]
pub(super) fn reset_statics() {
	MOUSESPEED.store(0, Ordering::Relaxed);
	MOUSESHOW.store(50, Ordering::Relaxed);
	MOUSESEL_SEEN.store(0xff, Ordering::Relaxed);
}

/// Test hook: the player set the mouse speed (the value as `field` would after a step).
#[cfg(test)]
pub(super) fn set_for_test(pct: u8) {
	MOUSESPEED.store(pct, Ordering::Relaxed);
	MOUSESHOW.store(pct, Ordering::Relaxed);
}
