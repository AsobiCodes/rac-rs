//! The host of the modern vendor shop, MAIN THREAD (v0.5 screens stage E). While the shop browses in the Modern style,
//! `host/handler/frame_vendor.rs` builds the page every display frame (`vendor::build`) and hands it to `end`; the mouse reaches this
//! file instead of the retail shop (`up()`), and what the player clicks goes to the game as `Select` requests and pad pulses
//! (`menu_model::cmd`). The twin of `pause_host.rs`, with two differences: the KEYBOARD AND THE PAD keep driving the retail shop
//! (`key` never takes a key), so the buy prompt is as strict as retail's, and there is no key repeat of our own; only while the list
//! browses do they pass `vendor_nav.rs` (worker thread): Up / Down walk the page's vertical list as retail's Left / Right, and Buy all
//! ammo is a row above the first one (Cross there = Square). The mouse is the page's own input: a hover chooses a row (Buy all ammo
//! too: `focus_buyall`), a click on a row twice buys it, Buy / the focused Buy all ammo row / the prompt's four buttons press the pad
//! button the game already reads, a right click goes back, the wheel walks the list (browse) or steps the quantity (prompt).
use super::title::{shown, GRACE_MS};
use super::vendor::{Built, Hit};
use super::vendor_nav;
use super::watchdog::Watch;
use crate::retail::menu_model::{cmd, resolve, submit, Command, Intent};
use crate::retail::mouse_menu::{Kind, MenuSnap, CROSS, LEFT, RIGHT, TRIANGLE};
use rac_engine::ui::input::{pad_mask, Device, MouseButton, MouseEvent, PadSample};
use rac_engine::ui::kit::Scale;
use rac_engine::ui::prim::{Prim, Rect, Symbol};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use winit::keyboard::KeyCode;

/// The modern shop page is up (MAIN writes, MAIN reads).
static UP: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();
/// The watchdog's view of this host (`watchdog.rs`).
static WATCH: Watch = Watch::new();
/// Square: the "Buy all ammo" hook reads it (`vendor.rs`).
const SQUARE: u16 = 0x80;
/// A click on a button of the page this soon (ms) after the page switched between the list and the buy prompt is dropped: the second
/// click of a double click on Buy must not confirm a prompt the player has not read, nor a double click on Cancel leave the shop or
/// buy again (review R4-1). Rows still take the hover part of such a click. ~21 fields at 60 fps (`now`: also under RAC_TEST_CLOCK). A second click made BEFORE the page switched (the game shows the new screen ~4 fields after the press) is
/// dropped by the pulse queue instead (`menu_model::cmd` `Spent`).
pub(super) const SETTLE_MS: u64 = 350;

#[derive(Default)]
struct State {
	/// The last frame's page (its rects are what a click hits; `last` repeats its draw list).
	built: Option<Built>,
	/// The shop as the mouse sees it (`mouse_vendor::read`, id 0xfe): the target of the row requests.
	snap: Option<MenuSnap>,
	win: (u32, u32),
	/// The player's focus: (row, ms).
	local: Option<(usize, u64)>,
	pad_prev: u16,
	/// The shown field the current empty hold (fly-in / closing) began at; None = a page was built since.
	hold_from: Option<u32>,
	/// Whether the last built page was the buy prompt (None before the first page), and when (ms) the page last switched between the
	/// list and the prompt (`SETTLE_MS`; None = never while this page is up).
	was_prompt: Option<bool>,
	flip_at: Option<u64>,
}

static ST: Mutex<Option<State>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<State>> {
	ST.lock().unwrap_or_else(|e| e.into_inner())
}

/// The clock of the focus grace and of `SETTLE_MS` (ms since the first call): wall time, or the game time of an RAC_TEST_CLOCK run (an
/// unpaced test runs its fields faster than real time; with the wall clock a deliberate click 66 fields after the prompt opened still
/// fell inside SETTLE_MS there).
pub fn now() -> u64 {
	rac_engine::viewer::test_clock::ms().unwrap_or_else(|| START.get_or_init(Instant::now).elapsed().as_millis() as u64)
}

/// Does the modern shop page own the mouse right now?
pub fn up() -> bool {
	UP.load(Ordering::Relaxed)
}

/// The window lost the focus: forget the page's local focus (the keys and the pad stay the game's).
pub fn lose_focus() {
	if let Some(st) = lock().as_mut() {
		st.local = None;
	}
}

/// The window size of the last drawn frame ((0, 0) before the first one): the scripted mouse (`host::scripted_mouse`) needs pixels.
pub fn win() -> (u32, u32) {
	lock().as_ref().map_or((0, 0), |s| s.win)
}

/// The page came up or went away (`frame_vendor`, once per frame). Also the hide switch of the retail shop's panels.
pub fn set_up(on: bool) {
	let on = WATCH.allow(on); // a tripped watchdog keeps the page down while the frame step keeps asking for it
	super::hide::set(super::hide::Screen::Vendor, on);
	if UP.swap(on, Ordering::Relaxed) != on {
		*lock() = on.then(State::default);
		vendor_nav::set_focus(false); // the page opens on retail's own first item, and keeps no focus when it goes
		let _ = crate::retail::vendor::take_cue(); // a browse sound left over from the last visit is no sound of the next one (R23 m5)
		if on {
			WATCH.stamp(now());
		}
	}
}

/// The watchdog's question (`host::watchdog`): up, no page finished for 1.5 s, and an empty last draw list.
pub fn silent() -> bool {
	silent_at(now())
}

fn silent_at(now: u64) -> bool {
	WATCH.silent(up(), now, last().is_empty())
}

/// The watchdog gives the page up: the retail shop and the classic mouse are back until the frame step asks the page down once.
pub fn trip() {
	set_up(false);
	WATCH.trip();
}

/// The shop is up but no page can be drawn (flying in before the list reads, or closing): an empty page, so the last page's rects are
/// gone too (no click lands on a row that is not on screen) and the watchdog sees the empty list.
pub fn hold() {
	if let Some(st) = lock().as_mut() {
		st.built = None;
	}
}

/// A hold may keep the watchdog quiet this many shown fields (3 s at 60; the fly-in measured 37 fields): an older one is a stuck shop.
pub const HOLD_FIELDS: u32 = 180;

/// Is a hold that began at shown field `from` still young at `field`?
pub fn hold_young(from: u32, field: u32) -> bool {
	field.wrapping_sub(from) < HOLD_FIELDS
}

/// The empty hold goes on at shown field `field` (`frame_vendor`, after `hold`): while it is young it counts as a finished page for
/// the watchdog, so a slow fly-in does not give the modern shop up for the whole visit (R4-4).
pub fn holding(field: u32) {
	let young = lock().as_mut().is_some_and(|st| hold_young(*st.hold_from.get_or_insert(field), field));
	if young {
		WATCH.stamp(now());
	}
}

/// The last device the player used (decides keycaps or pad glyphs in the hint bar): read live, the mouse counts as the keyboard side.
pub fn device() -> Device {
	super::look::device(super::look::start_keyboard())
}

/// The row the player's mouse chose a moment ago (it shows at once, before retail's cursor follows); None once retail's cursor wins.
pub fn local() -> Option<usize> {
	let (i, at) = lock().as_ref()?.local?;
	(now().saturating_sub(at) < GRACE_MS).then_some(i)
}

/// The row shown now: the player's own focus within its grace, else the page's.
fn current(st: &State, now: u64) -> usize {
	st.built.as_ref().map_or(0, |b| shown(st.local, b.sel, now))
}

/// Chooses row `i` (a hover or a click): the focus moves at once, the request goes to the game. Nothing while the prompt is up or
/// the shop animates (`resolve` finds no item then).
fn select(st: &mut State, i: usize, confirm: bool, now: u64) {
	if st.built.as_ref().is_none_or(|b| b.prompt || i >= b.count) {
		return;
	}
	let Some(m) = st.snap.as_ref().filter(|m| !m.busy()) else { return };
	if let Some(c) = resolve(m, Intent::Select { item: m.cur, row: i as u32, confirm }) {
		submit(c);
		st.local = Some((i, now));
	}
}

fn press(bit: u16) {
	submit(Command::Press(bit));
}

/// Puts the focus on Buy all ammo (a hover or a click on it, the wheel up from the first row): it shows selected at once and retail's
/// cursor goes to row 0, below it (as with the keys, `vendor_nav`); only the browse sound when the cursor is there already.
fn focus_buyall(st: &mut State, now: u64) {
	if vendor_nav::focused() || st.built.as_ref().is_none_or(|b| b.prompt || b.refill.is_none()) {
		return;
	}
	let Some(g) = game_sel(st) else { return };
	vendor_nav::set_focus(true);
	st.local = None;
	if g == 0 {
		crate::retail::vendor::browse_cue();
	} else {
		select(st, 0, false, now);
	}
}

/// Chooses row `i` (a hover, a click, the wheel), taking the focus off Buy all ammo first: when retail's cursor is on that row already
/// only the sound of the move is due.
fn to_row(st: &mut State, i: usize, confirm: bool, now: u64) {
	if vendor_nav::focused() {
		vendor_nav::set_focus(false);
		if game_sel(st) == Some(i) && !confirm {
			crate::retail::vendor::browse_cue();
			return;
		}
	}
	select(st, i, confirm, now);
}

fn back(st: &State) {
	if let Some(c) = st.snap.as_ref().and_then(|m| resolve(m, Intent::Back)) {
		submit(c);
	}
}

fn hit_at(st: &State, x: f32, y: f32) -> Hit {
	let Some(b) = &st.built else { return Hit::None };
	let (dx, dy) = Scale::fit(st.win.0, st.win.1).to_design(x, y);
	b.hit(dx, dy)
}

/// The game's own highlighted row while the shop browses (its list's cursor); None in the prompt or while it animates.
fn game_sel(st: &State) -> Option<usize> {
	let m = st.snap.as_ref().filter(|m| crate::retail::mouse_vendor::stage(m) == 1)?;
	match m.items.first()?.kind {
		Kind::Grid { cursor, .. } => Some(cursor as usize),
		_ => None,
	}
}

/// Buy (the button or the hint's Cross on the list): the purchase of the row the page SHOWS. While the player's hover is still ahead
/// of the game's cursor the row is selected with Cross (the request moves the cursor first), else Cross alone (review R2b-2).
fn buy(st: &mut State, now: u64) {
	if vendor_nav::focused() {
		return press(SQUARE); // Buy all ammo
	}
	let shown = current(st, now);
	match game_sel(st) {
		Some(g) if g != shown => select(st, shown, true, now),
		_ => press(CROSS),
	}
}

/// A left click: a row (the second click on it buys; Buy all ammo the same way), or a button of the page, which presses the pad button
/// the game reads. Within `SETTLE_MS` of the switch between the list and the prompt a click only hovers a row and presses nothing.
fn click(st: &mut State, x: f32, y: f32, now: u64) {
	let settling = st.flip_at.is_some_and(|at| now.saturating_sub(at) < SETTLE_MS);
	let prompt = st.built.as_ref().is_some_and(|b| b.prompt);
	let focused = vendor_nav::focused();
	match hit_at(st, x, y) {
		Hit::Row(i) => {
			let on = i == current(st, now) && !settling && !focused;
			to_row(st, i, on, now);
		}
		Hit::Refill if focused && !settling => press(SQUARE),
		Hit::Refill => focus_buyall(st, now),
		_ if settling => {}
		Hit::Buy | Hit::Hint(Symbol::Cross) if !prompt => buy(st, now),
		Hit::Buy | Hit::Confirm | Hit::Hint(Symbol::Cross) => press(CROSS),
		Hit::Less => press(LEFT),
		Hit::More => press(RIGHT),
		Hit::Cancel | Hit::Hint(Symbol::Triangle) => press(TRIANGLE),
		Hit::Hint(_) | Hit::None => {}
	}
}

/// One wheel notch (`up` = toward the list's start): the list walks one row (clamped; Buy all ammo above row 0), the prompt steps the quantity. Only the
/// quantity prompt of an ammo row in its asking stage has the arrows (`less` / `more`); the closing steps and the weapon prompt take
/// nothing, so no queued Left / Right lands later on the list.
fn wheel(st: &mut State, up: bool, now: u64) {
	let Some((prompt, count, arrow)) = st.built.as_ref().map(|b| (b.prompt, b.count, if up { b.more } else { b.less })) else { return };
	if prompt {
		if arrow.is_some() {
			press(if up { RIGHT } else { LEFT });
		}
		return;
	}
	if vendor_nav::focused() {
		if !up {
			to_row(st, 0, false, now); // Buy all ammo is the top: up does nothing
		}
		return;
	}
	let cur = current(st, now);
	if up && cur == 0 {
		return focus_buyall(st, now);
	}
	let to = if up { cur.saturating_sub(1) } else { (cur + 1).min(count.saturating_sub(1)) };
	if to != cur {
		select(st, to, false, now);
	}
}

/// A mouse event in window pixels while the page is up: a hover chooses a row (instantly), a left click acts, a right click goes
/// back, the wheel walks the list. Always taken (the retail shop behind must not also see it).
pub fn mouse(ev: MouseEvent) -> bool {
	if up() && matches!(ev, MouseEvent::Down { .. } | MouseEvent::Wheel { .. }) {
		super::look::note_keyboard(); // the mouse is the keyboard side of the hint bar (a plain move is judged by the window filter)
	}
	let mut g = lock();
	let Some(st) = g.as_mut().filter(|s| s.built.is_some()) else { return false }; // no page drawn yet = nothing to click: the mouse stays free
	let now = now();
	match ev {
		MouseEvent::Move { x, y } => match hit_at(st, x, y) {
			Hit::Refill => focus_buyall(st, now),
			Hit::Row(i) if i != current(st, now) || vendor_nav::focused() => to_row(st, i, false, now),
			_ => {}
		},
		MouseEvent::Down { button: MouseButton::Left, x, y } => click(st, x, y, now),
		MouseEvent::Down { button: MouseButton::Right, .. } => back(st),
		MouseEvent::Wheel { lines } if lines != 0.0 => wheel(st, lines > 0.0, now),
		_ => {}
	}
	true
}

/// One winit key. Records that the keyboard is in use (the hint bar shows keycaps) and drops the mouse's focus (the key moves the
/// game's cursor, which the page must show at once); the key itself is never taken, so the retail shop's own keys (arrows, Enter,
/// Esc, F) work as in the Classic style.
pub fn key(_code: KeyCode, pressed: bool, repeat: bool) -> bool {
	if pressed {
		if let Some(st) = lock().as_mut() {
			if !repeat {
				super::look::note_keyboard();
			}
			st.local = None;
		}
	}
	false
}

/// Display frame, first step: a new pad press means the pad is in use (pad glyphs in the hint bar), and the game's cursor is shown.
pub fn begin(pad: Option<PadSample>) {
	let mut g = lock();
	let Some(st) = g.as_mut() else { return };
	let mask = pad.map_or(0, pad_mask);
	if mask & !st.pad_prev != 0 {
		super::look::note_pad();
		st.local = None;
	}
	st.pad_prev = mask;
}

/// Display frame, last step: keeps this frame's page and the shop menu it was built from; returns the draw list.
pub fn end(built: Built, snap: Option<&MenuSnap>, win: (u32, u32)) -> Vec<Prim> {
	let prims = built.prims.clone();
	cmd::stamp(snap); // the pulses queued until the next frame are made for this page
	WATCH.stamp(now());
	if let Some(st) = lock().as_mut() {
		if st.was_prompt.is_some_and(|was| was != built.prompt) {
			st.flip_at = Some(now());
		}
		st.was_prompt = Some(built.prompt);
		(st.snap, st.win, st.built, st.hold_from) = (snap.cloned(), win, Some(built), None);
	}
	prims
}

/// The first stock row the list showed last frame (`vendor::Facts::scroll`); 0 with no page.
pub fn scroll() -> usize {
	lock().as_ref().and_then(|s| s.built.as_ref()).map_or(0, |b| b.first)
}

/// The draw list of the last frame (a frame whose page cannot be built repeats it).
pub fn last() -> Vec<Prim> {
	lock().as_ref().and_then(|s| s.built.as_ref()).map(|b| b.prims.clone()).unwrap_or_default()
}

/// The hole of the detail card the vendor's live portrait shows in (`vendor_portrait.rs`): None while the page is down or not built yet.
pub fn portrait() -> Option<Rect> {
	if !up() {
		return None;
	}
	lock().as_ref()?.built.as_ref()?.portrait
}

/// The vendor's portrait in the page's card: `retail` = the portrait quad `frame_ui::hide_retail` took out of the HUD list (None when the
/// game drew none this frame); the black backdrop is pushed whenever the page has its hole. Called by `hide_retail` every frame.
pub fn place_portrait(hud: &mut Vec<crate::overlay::Quad>, win: (u32, u32), retail: Option<crate::overlay::Quad>) {
	if let Some(hole) = portrait() {
		super::vendor_portrait::place(hud, win, hole, retail);
	}
}

#[cfg(test)]
mod tests;
