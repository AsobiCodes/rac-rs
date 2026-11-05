//! The Camera Options page's Lock-on strength row (row 8, right below Lock-on; lane nlock, 2026-11-03, user: a slider like Mouse
//! speed). The value scales the strafe lock-on's cone about the crosshair (aim/lock.rs `strafe_cone`): 100 = the full cone (the
//! lock picks of before the row), lower = a target must be nearer the crosshair to lock, 0 = no lock (as Lock-on Off). With
//! Lock-on Off the row is ignored (the search does not run).
//!
//! The row is built like Mouse speed (mousespeed.rs): a multi-choice row whose two choices are the same dynamic text (`ID_LPCT`,
//! "< 100 >"); retail's update toggles the row's byte `LOCKSTRSEL` on Cross, which the host reads as "+10" (100 wraps to 0); Left /
//! Right (pad bits, also the mouse wheel) are read by the host while the row is the page's cursor row: -10 / +10 with auto-repeat.
//! The value is the player's own key `lockon_strength` (pcopts.rs); the row never writes retail's data.
//!
//! Reserved RAM: the row byte `LOCKSTRSEL` (BASE + 0x3b), the label's text slot (id 0x7f87, the camopts pool's last slot before the
//! row table copy) and the value text's slot `LPCT_SLOT` (BASE + 0x40..0x80, unused since the row table moved to the camopts block).
use super::*;
use std::sync::atomic::AtomicU8;

/// The Lock-on strength row's byte: retail toggles it 0 <-> 1 on every Cross; the host reads each change as one +10 step.
pub(super) const LOCKSTRSEL: u32 = BASE + 0x3b;
/// The row's label and its value text "< 100 >" (built from the current percent on every lookup).
pub const ID_LOCKSTR: u32 = 0x7f87;
pub const ID_LPCT: u32 = 0x7f88;
/// The value text's own 0x40-byte slot (the camopts pool has room for ids up to `ID_LOCKSTR` before the row table copy).
pub const LPCT_SLOT: u32 = BASE + 0x40;
/// The row index on the Camera Options page (right below Lock-on, row 7).
pub(super) const LOCKSTR_ROW: u32 = 8;

/// The label per language (En, Fr, De, Es, It) in the font bytes (Es o-acute 0x9c). [I] wording, native review wanted.
#[rustfmt::skip]
static LABEL: [&[u8]; 5] = [b"Lock-on strength", b"Force verrouillage", b"Erfassungsgrad", b"Fuerza de fijaci\x9cn", b"Forza aggancio"];

/// The player's Lock-on strength in percent (0..=100; 100 without the key).
static LOCKSTR: AtomicU8 = AtomicU8::new(100);
/// `LOCKSTRSEL` as last seen (0xff: not known yet, the next look is the baseline).
static LOCKSTRSEL_SEEN: AtomicU8 = AtomicU8::new(0xff);

/// The strafe lock-on's strength in percent (aim/lock.rs): 100 = the full cone, 0 = no lock.
pub fn lockon_strength() -> u8 {
	LOCKSTR.load(Ordering::Relaxed)
}

/// The row's value after one `step` from `cur`: Left -10 and Right +10 stop at 0 / 100, Cross +10 wraps from 100 to 0.
pub(super) fn lockstr_step(cur: u8, step: Step) -> u8 {
	let cur = pcopts::snap_lockstr(cur);
	match step {
		Step::Left => cur.saturating_sub(10),
		Step::Right => (cur + 10).min(100),
		Step::Cross => {
			if cur >= 100 {
				0
			} else {
				cur + 10
			}
		}
	}
}

/// The row's text in `lang` (0 En .. 4 It); the value text is the current percent.
pub(super) fn lockstr_string(id: u32, lang: usize) -> Option<Vec<u8>> {
	match id {
		ID_LOCKSTR => Some(LABEL[lang.min(4)].to_vec()),
		ID_LPCT => Some(pct_text(lockon_strength())),
		_ => None,
	}
}

/// `init` / `setup`: the options file's value is known; the next look at the row byte is the baseline.
pub(super) fn lockstr_load(opts: &PcOptions) {
	LOCKSTR.store(opts.lockon_strength_pct(), Ordering::Relaxed);
	LOCKSTRSEL_SEEN.store(0xff, Ordering::Relaxed);
}

/// The player set the strength to `pct` (either menu): remembered and saved.
fn lockstr_set(s: &mut St, pct: u8) {
	LOCKSTR.store(pct, Ordering::Relaxed);
	s.opts.lockon_strength = Some(pct);
	s.save();
	eprintln!("retail: PC options: lock-on strength {pct}%");
}

/// Worker, each field of a level, after `camopts_field` read the buttons: the row's steps (Cross: retail toggled `LOCKSTRSEL`;
/// Left / Right: `dir`, the auto-repeated pad bits).
pub(super) fn lockstr_field(mem: &mut Mem, level: u32, dir: [bool; 2], s: &mut St) {
	let now = lockon_strength();
	let sel = mem.rd8(LOCKSTRSEL);
	let seen = LOCKSTRSEL_SEEN.swap(sel, Ordering::Relaxed);
	let cross = seen != 0xff && seen != sel;
	if (cross || dir[0] != dir[1]) && on_row(mem, level, LOCKSTR_ROW) {
		let next = lockstr_step(now, step_of(cross, dir));
		if next != now {
			lockstr_set(s, next);
		}
	}
}

/// The modern screen set the strength (`ui_bridge.rs`): the same state and file key as a step of the classic row.
pub(super) fn lockstr_set_from_ui(s: &mut St, pct: u8) {
	let pct = pcopts::snap_lockstr(pct);
	if lockon_strength() != pct {
		lockstr_set(s, pct);
	}
}

/// Test hook: the row's process-wide state back to "never set".
#[cfg(test)]
pub(super) fn lockstr_reset() {
	LOCKSTR.store(100, Ordering::Relaxed);
	LOCKSTRSEL_SEEN.store(0xff, Ordering::Relaxed);
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn steps_stop_at_the_ends_and_cross_wraps() {
		assert_eq!(lockstr_step(100, Step::Right), 100);
		assert_eq!(lockstr_step(0, Step::Left), 0);
		assert_eq!(lockstr_step(50, Step::Left), 40);
		assert_eq!(lockstr_step(50, Step::Right), 60);
		assert_eq!(lockstr_step(90, Step::Cross), 100);
		assert_eq!(lockstr_step(100, Step::Cross), 0, "Cross wraps to 0");
		assert_eq!(lockstr_step(0, Step::Cross), 10);
	}

	#[test]
	fn labels_fit_and_value_text() {
		for l in LABEL {
			assert!(l.len() < 0x40 && !l.is_empty());
		}
		assert_eq!(lockstr_string(ID_LOCKSTR, 0).unwrap(), b"Lock-on strength");
		assert_eq!(lockstr_string(ID_LOCKON, 0), None);
	}
}
