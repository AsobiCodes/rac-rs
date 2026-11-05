//! The host of the modern Options page (v0.5 modern menu), MAIN THREAD only: it owns the engine's `Ui` between "the worker saw Cross
//! on the Graphics / Camera row" (`retail::modern_swallow`, whose open request the display-frame step takes) and "the player left
//! the page". The window handler feeds it keys, mouse and focus loss as they come; the frame step (`host/handler/frame_ui.rs`) feeds it
//! the pad, the repeat clock and the values the worker published, and takes the draw list. While it is up `ui::is_open()` is true:
//! the game gets a neutral pad (`pad_filter`, called from `retail/input.rs`), and the button that closed the page stays eaten until
//! it is released, so Circle / Esc never also backs out of the retail Options list behind it.
use crate::retail::modern_swallow::Open;
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::prim::Prim;
use rac_engine::ui::{self, MouseEvent, Outcome, PadSample, Ui, UiKey};
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use winit::keyboard::KeyCode;

static UI: Mutex<Option<Ui>> = Mutex::new(None);
static START: OnceLock<Instant> = OnceLock::new();
/// The retail pad's buttons of the last field (`pad_filter`'s input): what is held when the page closes stays eaten.
static LAST_BUTTONS: AtomicU16 = AtomicU16::new(0);
static EAT: AtomicU16 = AtomicU16::new(0);
/// The page was opened from the title (`Open::front`).
static FRONT: AtomicBool = AtomicBool::new(false);
/// A Shift key is held (window keys while a modern screen is up): Shift+Tab walks the tabs back on the pause and title pages (R3-5).
static SHIFT: AtomicBool = AtomicBool::new(false);

/// Is a Shift key held (as the window's key events last said)?
pub fn shift() -> bool {
	SHIFT.load(Ordering::Relaxed)
}

/// The UI's own millisecond clock (display rate, not the game's).
fn now_ms() -> u64 {
	START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

fn lock() -> std::sync::MutexGuard<'static, Option<Ui>> {
	UI.lock().unwrap_or_else(|e| e.into_inner())
}

/// A keyboard key as the UI's key (None = not a menu key).
pub fn map_key(code: KeyCode) -> Option<UiKey> {
	Some(match code {
		KeyCode::ArrowUp => UiKey::Up,
		KeyCode::ArrowDown => UiKey::Down,
		KeyCode::ArrowLeft => UiKey::Left,
		KeyCode::ArrowRight => UiKey::Right,
		KeyCode::KeyW => UiKey::W,
		KeyCode::KeyA => UiKey::A,
		KeyCode::KeyS => UiKey::S,
		KeyCode::KeyD => UiKey::D,
		KeyCode::Enter | KeyCode::NumpadEnter => UiKey::Enter,
		KeyCode::Space => UiKey::Space,
		KeyCode::Escape => UiKey::Esc,
		KeyCode::Backspace => UiKey::Backspace,
		KeyCode::KeyQ => UiKey::Q,
		KeyCode::KeyE => UiKey::E,
		KeyCode::Tab => UiKey::Tab,
		KeyCode::ShiftLeft | KeyCode::ShiftRight => UiKey::Shift,
		KeyCode::PageUp => UiKey::PageUp,
		KeyCode::PageDown => UiKey::PageDown,
		KeyCode::Home => UiKey::Home,
		KeyCode::End => UiKey::End,
		_ => return None,
	})
}

pub fn pad_sample(buttons: u16, sticks: [u8; 4]) -> PadSample {
	PadSample { buttons, lx: sticks[2], ly: sticks[3] }
}

/// Opens the page (main thread). `pad`: the pad now, so a button still down from the press that opened it does not act.
/// False = the pack has no modern menu (nothing opened).
pub fn open(o: Open, pad: Option<PadSample>) -> bool {
	let Some(mut ui) = Ui::from_pack(Box::new(FontMeasure)) else { return false };
	ui.begin(o.front, Some(o.tab.id()));
	if let Some(p) = pad {
		ui.hold_pad(p);
	}
	ui.fix_scroll();
	*lock() = Some(ui);
	FRONT.store(o.front, Ordering::Relaxed);
	ui::set_open(true);
	true
}

/// Leaves the page: pending changes go to the game first, the held buttons stay eaten until released.
fn finish(slot: &mut Option<Ui>) {
	if let Some(mut ui) = slot.take() {
		ui.flush_pack();
	}
	EAT.store(LAST_BUTTONS.load(Ordering::Relaxed), Ordering::Relaxed);
	ui::set_open(false);
}

/// Closes the page from outside (the menu behind it went away, the UI pipes were refused).
pub fn close() {
	finish(&mut lock());
}

/// One winit key. True = the press is the page's (the game must not see it); a release always goes on to the game so no key sticks.
/// OS key repeats are dropped (the page has its own repeat, and a repeat of the Enter that opened it must not act).
pub fn key(code: KeyCode, pressed: bool, repeat: bool) -> bool {
	if matches!(code, KeyCode::ShiftLeft | KeyCode::ShiftRight) {
		SHIFT.store(pressed, Ordering::Relaxed);
	}
	if super::title_host::up() {
		return super::title_host::key(code, pressed, repeat); // [PC] the modern title page owns the keys
	}
	if super::map_host::up() {
		return super::map_host::key(code, pressed, repeat); // [PC] the modern planet map only notes the keyboard (keys stay the game's); before the pause page, which is still closing while the Map tab opens the map
	}
	if super::pause_host::up() {
		return super::pause_host::key(code, pressed, repeat); // [PC] so does the modern pause page
	}
	if super::vendor_host::up() {
		return super::vendor_host::key(code, pressed, repeat); // [PC] the modern shop only notes the keyboard (keys stay the game's)
	}
	if super::ship_host::up() {
		return super::ship_host::key(code, pressed, repeat); // [PC] so does the modern Galactic Map
	}
	let mut g = lock();
	let Some(ui) = g.as_mut() else { return false };
	let mapped = map_key(code);
	if repeat {
		return mapped.is_some();
	}
	let Some(k) = mapped else { return false };
	if ui.key(k, pressed, now_ms()) == Outcome::Close {
		finish(&mut g);
	}
	pressed
}

/// A mouse event in window pixels. True = taken (the retail menu behind must not move or click).
pub fn mouse(ev: MouseEvent) -> bool {
	if super::title_host::up() {
		return super::title_host::mouse(ev);
	}
	if super::map_host::up() {
		return super::map_host::mouse(ev); // before the pause page (see `key`)
	}
	if super::pause_host::up() {
		return super::pause_host::mouse(ev);
	}
	if super::vendor_host::up() {
		return super::vendor_host::mouse(ev);
	}
	if super::ship_host::up() {
		return super::ship_host::mouse(ev);
	}
	let mut g = lock();
	let Some(ui) = g.as_mut() else { return false };
	if ui.mouse(ev, now_ms()) == Outcome::Close {
		finish(&mut g);
	}
	true
}

/// The scripted mouse (RAC_RETAIL_MOUSE, `retail/mouse_script.rs`) for the five page hosts: the pointer at (fx, fy), fractions of the
/// window, then the left click, the right click and the release asked for. False = no page host is up (or none has drawn a frame
/// yet): the classic mouse model takes the entry.
pub fn scripted_mouse(fx: f32, fy: f32, click: bool, back: bool, up: bool) -> bool {
	let win = if super::title_host::up() {
		super::title_host::win()
	} else if super::map_host::up() {
		super::map_host::win()
	} else if super::pause_host::up() {
		super::pause_host::win()
	} else if super::vendor_host::up() {
		super::vendor_host::win()
	} else if super::ship_host::up() {
		super::ship_host::win()
	} else {
		return false;
	};
	if win.0 == 0 || win.1 == 0 {
		return false;
	}
	let (x, y) = (fx * win.0 as f32, fy * win.1 as f32);
	mouse(MouseEvent::Move { x, y });
	if click {
		mouse(MouseEvent::Down { button: rac_engine::ui::input::MouseButton::Left, x, y });
	}
	if back {
		mouse(MouseEvent::Down { button: rac_engine::ui::input::MouseButton::Right, x, y });
	}
	if up {
		mouse(MouseEvent::Up { button: rac_engine::ui::input::MouseButton::Left });
	}
	true
}

/// [PC] The watchdog (`watchdog.rs`), once per display frame (`frame_ui`, before the page steps): a page host that is up, finished no
/// page for 1.5 s and has an empty last draw list gives the screen back to the classic menu and mouse. Returns how many it gave up.
pub fn watchdog() -> usize {
	let mut n = 0;
	if super::title_host::silent() {
		super::title_host::trip();
		n += 1;
	}
	if super::pause_host::silent() {
		super::pause_host::trip();
		n += 1;
	}
	if super::vendor_host::silent() {
		super::vendor_host::trip();
		n += 1;
	}
	if super::ship_host::silent() {
		super::ship_host::trip();
		n += 1;
	}
	if super::map_host::silent() {
		super::map_host::trip();
		n += 1;
	}
	if n > 0 {
		eprintln!("modern menu: watchdog: {n} page host(s) up with nothing drawn for 1.5 s: given up (classic screen and mouse back)");
	}
	n
}

/// The window lost the focus: forget held keys, buttons and a slider drag.
pub fn lose_focus() {
	super::title_host::lose_focus();
	super::pause_host::lose_focus();
	super::vendor_host::lose_focus();
	super::ship_host::lose_focus();
	super::map_host::lose_focus();
	crate::retail::menu_model::cmd::clear_pulses(); // no half-finished press survives into a page nobody is watching
	EAT.store(0, Ordering::Relaxed);
	SHIFT.store(false, Ordering::Relaxed);
	if let Some(ui) = lock().as_mut() {
		ui.lose_focus();
	}
}

/// Display-frame step: the pad, the repeat clock, the values the worker published; `valid` = the retail menu the page was opened
/// from is still up (false closes it). Returns the draw list, or None once the page is not up.
pub fn step(win: (u32, u32), pad: Option<PadSample>, valid: bool) -> Option<Vec<Prim>> {
	let mut g = lock();
	let ui = g.as_mut()?;
	let now = now_ms();
	ui.set_window(win.0, win.1);
	let mut out = ui.tick(now);
	if let Some(p) = pad {
		out = if ui.pad(p, now) == Outcome::Close { Outcome::Close } else { out };
	}
	ui.flush_pack();
	ui.sync_pack();
	if out == Outcome::Close || !valid {
		finish(&mut g);
		return None;
	}
	Some(ui.frame().1)
}

/// `retail/input.rs`, once per field with the field's merged pad: true = the game gets a neutral pad (the page is up, or the button
/// that closed it is still down).
pub fn pad_filter(buttons: u16) -> bool {
	LAST_BUTTONS.store(buttons, Ordering::Relaxed);
	let eat = EAT.load(Ordering::Relaxed) & buttons;
	EAT.store(eat, Ordering::Relaxed);
	ui::is_open() || eat != 0
}

/// A modern screen page (pause, title, Galactic Map) is up and steers the game's menu itself, through queued pulses and requests:
/// `retail/input.rs` keeps the pad's buttons away from the classic mouse state then (`mouse_menu::kbd_bits`). The shop keeps the
/// pad as in Classic.
pub fn pad_owned() -> bool {
	super::pause_host::up() || super::title_host::up() || super::ship_host::up()
}

/// Whether the retail menu overlay is hidden behind the page: over the pause menu yes (the world shows through the blur), over the
/// title no (the title's picture IS the overlay).
pub fn hides_retail() -> bool {
	ui::is_open() && !FRONT.load(Ordering::Relaxed)
}

/// [PC] The page is up over the title: the native title menu (front_mirror.rs) is not drawn, the title scene stays behind the blur.
pub fn hides_title_menu() -> bool {
	ui::is_open() && FRONT.load(Ordering::Relaxed)
}

/// The page was opened from the title's Options list (not the pause menu's).
pub fn from_title() -> bool {
	FRONT.load(Ordering::Relaxed)
}

#[cfg(test)]
pub(crate) fn eat_for_test() -> u16 {
	EAT.load(Ordering::Relaxed)
}

/// Test door: the page closes with `buttons` held in the last field (what `finish` does with the eaten mask).
#[cfg(test)]
pub(crate) fn close_for_test(buttons: u16) {
	LAST_BUTTONS.store(buttons, Ordering::Relaxed);
	finish(&mut None);
}

#[cfg(test)]
mod tests {
	use super::*;
	/// Any two distinct pad bits (Circle is 0x20 in the PS2 report).
	const CIRCLE: u16 = 0x20;
	const OTHER: u16 = 0x40;

	/// The button that closed the page stays away from the game until it is released; a neutral pad while the page is up.
	#[test]
	fn eat_keeps_closing_button_from_game() {
		ui::set_open(false);
		close_for_test(CIRCLE);
		assert_eq!(eat_for_test(), CIRCLE);
		assert!(pad_filter(CIRCLE), "still down: eaten");
		assert!(pad_filter(CIRCLE | OTHER), "other buttons are neutral too while one is eaten");
		assert!(!pad_filter(OTHER), "released: the game gets the pad again");
		assert_eq!(eat_for_test(), 0);
		ui::set_open(true);
		assert!(pad_filter(0), "page up: neutral");
		ui::set_open(false);
	}

	#[test]
	fn menu_keys_map() {
		assert_eq!(map_key(KeyCode::NumpadEnter), Some(UiKey::Enter));
		assert_eq!(map_key(KeyCode::F11), None);
		assert!(!hides_retail() || ui::is_open());
	}
}
