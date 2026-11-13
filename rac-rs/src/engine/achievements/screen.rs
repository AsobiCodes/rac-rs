//! The in-game list: opened from the pause menu with F6 (a hint line shows while paused) or by `open()` (the later pause-menu row),
//! scrolled with Up / Down / PageUp / PageDown / Home / End, closed with F6 or Escape (or by leaving the pause menu). Gamepad
//! (`ui_pad`): d-pad or stick Up / Down scroll, L1 / R1 a page, Circle or Triangle close. Drawn in the HUD pass over the paused picture.
use super::badge::badge;
use super::toast::{self, fit, scale};
use super::{store::unix_to_iso, Def};
use crate::localization::ui_text;
use crate::overlay::{self, Quad};
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU8, AtomicUsize, Ordering};
use winit::keyboard::KeyCode;

static OPEN: AtomicBool = AtomicBool::new(false);
static SCROLL: AtomicUsize = AtomicUsize::new(0);
/// Rows the last draw showed (the page size of PageUp / PageDown).
static PAGE: AtomicUsize = AtomicUsize::new(8);

/// Opens the list from the start (a game's pause-menu row, F6). Only meaningful while the game is paused: `draw` closes the list
/// again in any field that is not.
pub fn open() {
	SCROLL.store(0, Ordering::Relaxed);
	super::view::reset();
	OPEN.store(true, Ordering::Relaxed);
}

/// Closes the list.
pub fn close() {
	OPEN.store(false, Ordering::Relaxed);
}

pub fn is_open() -> bool {
	OPEN.load(Ordering::Relaxed)
}

/// Moves the first row by `d` rows (clamped); `d` = isize::MIN / MAX jump to the top / end.
fn scroll(d: isize) {
	let n = super::hub().as_ref().map_or(0, |h| h.pack.defs.len());
	let max = n.saturating_sub(PAGE.load(Ordering::Relaxed).max(1));
	let s = SCROLL.load(Ordering::Relaxed);
	let to = match d {
		isize::MIN => 0,
		isize::MAX => max,
		d => (s as isize).saturating_add(d).clamp(0, max as isize) as usize,
	};
	SCROLL.store(to, Ordering::Relaxed);
}

// PS2 pad bits (engine/input/pad.rs `PadState::buttons`).
const P_L1: u16 = 0x04;
const P_R1: u16 = 0x08;
const P_TRIANGLE: u16 = 0x10;
const P_CIRCLE: u16 = 0x20;
const P_UP: u16 = 0x1000;
const P_DOWN: u16 = 0x4000;
/// Buttons that closed the list and are still down: kept from the game until released (Circle is "back" in the pause menu).
static EAT: AtomicU16 = AtomicU16::new(0);
/// Buttons down in the last field (edges) and the fields the scroll direction has been held (auto-repeat).
static LAST: AtomicU16 = AtomicU16::new(0);
static HOLD: AtomicU8 = AtomicU8::new(0);

/// One field of the pad on the list.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct PadStep {
	/// Buttons incl. the stick as d-pad, to compare with next field.
	pub(super) now: u16,
	/// Fields the scroll direction has been held.
	pub(super) hold: u8,
	pub(super) close: bool,
	pub(super) rows: isize,
	pub(super) pages: isize,
}

/// `last`: `now` of the previous field; `hold`: its `hold`. A stick counts as the d-pad past the same dead zone as retail's pad
/// writer (|byte - 0x7f| > 0x2f). One row on the press, then after 20 fields every 4th; L1 / R1 are a page up / down.
pub(super) fn pad_step(last: u16, buttons: u16, sticks: [u8; 4], hold: u8) -> PadStep {
	let pushed = |v: u8, lo: u16, hi: u16| match i32::from(v) - 0x7f {
		d if d < -0x2f => lo,
		d if d > 0x2f => hi,
		_ => 0,
	};
	let now = buttons | pushed(sticks[3], P_UP, P_DOWN) | pushed(sticks[1], P_UP, P_DOWN);
	let fresh = now & !last;
	let dir = if now & P_UP != 0 { -1 } else if now & P_DOWN != 0 { 1 } else { 0 };
	let hold = if dir != 0 { hold.saturating_add(1) } else { 0 };
	let rows = if dir != 0 && (hold == 1 || (hold > 20 && hold % 4 == 0)) { dir } else { 0 };
	let pages = (fresh & P_R1 != 0) as isize - (fresh & P_L1 != 0) as isize;
	PadStep { now, hold, close: fresh & (P_CIRCLE | P_TRIANGLE) != 0, rows, pages }
}

/// The gamepad, once per field with the merged pad (`buttons` = PS2 bits, `sticks` = [rx, ry, lx, ly], 0x80 = centre). True = the
/// list is open (or a button that closed it is still down): the caller hands the game a neutral pad this field, so nothing the
/// player presses on the list reaches the game or the pause menu behind it.
pub fn ui_pad(buttons: u16, sticks: [u8; 4]) -> bool {
	super::available() && pad_in(buttons, sticks)
}

/// `ui_pad` once the game has a list (the test seam: the tests have no pack).
pub(super) fn pad_in(buttons: u16, sticks: [u8; 4]) -> bool {
	let eat = EAT.load(Ordering::Relaxed) & buttons;
	EAT.store(eat, Ordering::Relaxed);
	if super::view::hosted() {
		// A game's page shows the list (`view.rs`): its host has the pad (and gives the game a neutral one), not this list.
		LAST.store(0, Ordering::Relaxed);
		return false;
	}
	if !is_open() {
		LAST.store(0, Ordering::Relaxed);
		return eat != 0;
	}
	let step = pad_step(LAST.load(Ordering::Relaxed), buttons, sticks, HOLD.load(Ordering::Relaxed));
	LAST.store(step.now, Ordering::Relaxed);
	HOLD.store(step.hold, Ordering::Relaxed);
	if step.close {
		close();
		EAT.store(buttons & (P_CIRCLE | P_TRIANGLE), Ordering::Relaxed);
		return true;
	}
	let page = PAGE.load(Ordering::Relaxed).max(1) as isize;
	if step.rows != 0 {
		scroll(step.rows);
	}
	if step.pages != 0 {
		scroll(step.pages * page);
	}
	true
}

/// F6 opens the closed list in the pause menu, unless the debug keys are on: then F6 is theirs (infinite ammo), as the hint says.
pub(super) fn f6_opens_list(paused: bool, debug_owns_f6: bool) -> bool {
	paused && !debug_owns_f6
}

/// Keys of the list. True = the key was taken (the game must not see it). `paused`: the game shows its pause menu.
pub fn ui_key(code: KeyCode, pressed: bool, repeat: bool, paused: bool) -> bool {
	super::available() && key_in(code, pressed, repeat, paused)
}

/// `ui_key` once the game has a list (the test seam).
pub(super) fn key_in(code: KeyCode, pressed: bool, repeat: bool, paused: bool) -> bool {
	let is_up = is_open();
	let mine = matches!(code, KeyCode::ArrowUp | KeyCode::ArrowDown | KeyCode::PageUp | KeyCode::PageDown | KeyCode::Home | KeyCode::End | KeyCode::Escape | KeyCode::F6);
	if !is_up {
		if code == KeyCode::F6 && f6_opens_list(paused, super::owns_f6()) {
			if pressed && !repeat {
				open();
			}
			return true;
		}
		return false;
	}
	if !mine || (super::view::hosted() && code != KeyCode::F6) {
		return false; // a game's page that shows the list takes its keys (`view.rs`); F6 still closes it
	}
	if !pressed {
		return true;
	}
	let page = PAGE.load(Ordering::Relaxed).max(1) as isize;
	match code {
		KeyCode::F6 | KeyCode::Escape if !repeat => close(),
		KeyCode::ArrowUp => scroll(-1),
		KeyCode::ArrowDown => scroll(1),
		KeyCode::PageUp => scroll(-page),
		KeyCode::PageDown => scroll(page),
		KeyCode::Home => scroll(isize::MIN),
		KeyCode::End => scroll(isize::MAX),
		_ => {}
	}
	true
}

/// The whole HUD part: the pop-up always, the hint and the list while paused (neither while a game's page shows the list, `view.rs`).
pub fn draw(out: &mut Vec<Quad>, w: f32, h: f32, paused: bool) {
	if super::available() {
		draw_in(out, w, h, paused);
	}
}

/// `draw` once the game has a list (the test seam).
pub(super) fn draw_in(out: &mut Vec<Quad>, w: f32, h: f32, paused: bool) {
	if !paused {
		close();
	}
	if paused && !super::view::hosted() {
		if OPEN.load(Ordering::Relaxed) {
			list(out, w, h);
		} else {
			hint(out, w, h);
		}
	}
	toast::draw(out, w, h);
}

/// The hint line under the paused picture. F6 is named only while it opens the list; once cheated it says so.
pub(super) fn hint_text(got: usize, all: usize, owns_f6: bool, cheated: bool) -> String {
	let key = if owns_f6 { "" } else { "F6  " };
	let tail = if cheated { format!("  {}", ui_text("OFF: DEBUG MODE")) } else { String::new() };
	format!("{key}{}  {got}/{all}{tail}", ui_text("ACHIEVEMENTS"))
}

fn hint(out: &mut Vec<Quad>, w: f32, h: f32) {
	let Some((n, all)) = super::hub().as_ref().map(|h| h.store.counts()) else { return };
	let k = scale(h);
	let s = hint_text(n, all, super::owns_f6(), super::cheated());
	let size = 14.0 * k;
	let tw = overlay::text_width(&s, size);
	overlay::rect(out, [w * 0.5 - tw * 0.5 - 8.0 * k, h - size - 26.0 * k, tw + 16.0 * k, size + 10.0 * k], [0.0, 0.0, 0.0, 0.55]);
	overlay::text(out, &s, w * 0.5 - tw * 0.5, h - size - 21.0 * k, size, [1.0, 1.0, 1.0, 0.9]);
}

/// The line shown when achievements are disabled by debug mode.
pub(super) const CHEAT_LINE: &str = "ACHIEVEMENTS ARE OFF FOR THIS SESSION: DEBUG MODE WAS ON";

fn status(d: &Def, e: &super::Entry) -> String {
	if e.unlocked {
		if e.at > 0 { unix_to_iso(e.at)[..10].to_string() } else { ui_text("UNLOCKED").to_string() }
	} else if d.max > 0 {
		format!("{} / {}", e.progress, d.max)
	} else {
		ui_text("LOCKED").to_string()
	}
}

fn list(out: &mut Vec<Quad>, w: f32, h: f32) {
	let hub = super::hub();
	let Some(hb) = hub.as_ref() else { return };
	let k = scale(h);
	let pw = (1100.0 * k).min(w * 0.96);
	let (px, py, ph) = ((w - pw) * 0.5, 40.0 * k, h - 80.0 * k);
	overlay::rect(out, [0.0, 0.0, w, h], [0.0, 0.0, 0.0, 0.6]);
	overlay::rect(out, [px, py, pw, ph], [0.04, 0.05, 0.08, 0.95]);
	let (got, all) = hb.store.counts();
	let pct = if all > 0 { got * 100 / all } else { 0 };
	overlay::text(out, &format!("{}   {got} / {all}   {pct}%", ui_text("ACHIEVEMENTS")), px + 24.0 * k, py + 18.0 * k, 28.0 * k, [1.0, 1.0, 1.0, 1.0]);
	let (top, row) = (py + 76.0 * k, 66.0 * k);
	let page = (((ph - 76.0 * k - 44.0 * k) / row) as usize).max(1);
	PAGE.store(page, Ordering::Relaxed);
	let first = SCROLL.load(Ordering::Relaxed).min(hb.pack.defs.len().saturating_sub(page));
	for (r, i) in (first..hb.pack.defs.len().min(first + page)).enumerate() {
		let (d, e) = (&hb.pack.defs[i], &hb.store.entries[i]);
		let y = top + r as f32 * row;
		let dim = if e.unlocked { 1.0 } else { 0.45 };
		let masked = d.hidden && !e.unlocked;
		overlay::rect(out, [px + 12.0 * k, y, pw - 24.0 * k, row - 6.0 * k], [0.12, 0.14, 0.2, 0.7]);
		badge(out, hb.pack, i, e.unlocked, [px + 18.0 * k, y + 3.0 * k, 54.0 * k], 1.0);
		let tx = px + 84.0 * k;
		let avail = pw - 84.0 * k - 230.0 * k;
		let (name, desc) = if masked { ("HIDDEN TROPHY", "KEEP PLAYING TO FIND OUT") } else { (d.name, d.desc) };
		let (name, desc) = (ui_text(name), ui_text(desc));
		overlay::text(out, name, tx, y + 8.0 * k, fit(name, avail, 22.0 * k), [1.0, 1.0, 1.0, dim]);
		overlay::text(out, desc, tx, y + 34.0 * k, fit(desc, avail, 15.0 * k), [0.8, 0.84, 0.92, dim]);
		let st = status(d, e);
		let sz = 17.0 * k;
		let c = d.grade.colour();
		overlay::text(out, &st, px + pw - 24.0 * k - overlay::text_width(&st, sz), y + 10.0 * k, sz, [c[0], c[1], c[2], dim.max(0.7)]);
		overlay::text(out, ui_text(d.grade.label()), px + pw - 24.0 * k - overlay::text_width(ui_text(d.grade.label()), 12.0 * k), y + 36.0 * k, 12.0 * k, [0.7, 0.72, 0.8, dim]);
	}
	overlay::text(out, ui_text("UP/DOWN SCROLL   L1/R1 PAGE   F6/ESC/CIRCLE/TRIANGLE CLOSE"), px + 24.0 * k, py + ph - 30.0 * k, 15.0 * k, [0.75, 0.78, 0.85, 1.0]);
	if super::cheated() {
		let line = ui_text(CHEAT_LINE);
		overlay::text(out, line, px + 24.0 * k, py + 52.0 * k, fit(line, pw - 48.0 * k, 16.0 * k), [1.0, 0.7, 0.3, 1.0]);
	}
}
