//! The host of the modern planet map, MAIN THREAD (v0.5 build 12; the twin of `ship_host.rs`). While the planet map (page 0xb) or
//! its mission list (0xc) is up in the Modern style, `host/handler/frame_map.rs` builds the page every display frame (`map::build`)
//! and hands it to `end`; the mouse reaches this file instead of the retail menu (`up()`). The KEYBOARD AND THE PAD keep driving the
//! retail page exactly as in the Classic style (`key` never takes a key): the map's tracking, zoom, L1 / R1, Missions and Exit are
//! the game's own presses. The mouse clicks the key list (each row presses its pad button), right click is Triangle (Exit on the map,
//! back to the map on the list), the wheel walks the mission list, and a tab of the tab bar closes the map with Start and opens that
//! pause tab (`retail::map_open`). `place_crops` lays the game's own pictures (crops of the retail overlay) into the page's holes.
use super::map::{compose, Built, Stage};
use super::watchdog::Watch;
use crate::retail::map_open::{self, Goal, START};
use crate::retail::menu_model::{cmd, submit, Command};
use crate::retail::mouse_menu::{MenuSnap, DOWN, TRIANGLE, UP as PAD_UP};
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::input::{pad_mask, Device, MouseButton, MouseEvent, PadSample};
use rac_engine::ui::kit::page::PageLayout;
use rac_engine::ui::kit::{page_draw, Scale};
use rac_engine::ui::prim::Prim;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use winit::keyboard::KeyCode;

/// The modern map page is up (MAIN writes, MAIN reads).
static UP: AtomicBool = AtomicBool::new(false);
static CLOCK: OnceLock<Instant> = OnceLock::new();
/// The watchdog's view of this host (`watchdog.rs`).
static WATCH: Watch = Watch::new();
/// The shell's figures of the last in-level menu seen: (level << 32) | bolts. The page shows them while the Map tab's presses are on
/// their way and no menu names them (`frame_map`).
static HEADER: AtomicU64 = AtomicU64::new(0);

#[derive(Default)]
struct State {
	/// The last frame's page (its rects are what a click hits).
	built: Option<Built>,
	layout: Option<PageLayout>,
	/// The menu as the mouse sees it (None while the Map tab's presses are on their way).
	snap: Option<MenuSnap>,
	win: (u32, u32),
	last: Vec<Prim>,
	pad_prev: u16,
}

static ST: Mutex<Option<State>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<State>> {
	ST.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn now() -> u64 {
	CLOCK.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Does the modern map page own the mouse right now?
pub fn up() -> bool {
	UP.load(Ordering::Relaxed)
}

/// The window size of the last drawn frame ((0, 0) before the first one), for the scripted mouse.
pub fn win() -> (u32, u32) {
	lock().as_ref().map_or((0, 0), |s| s.win)
}

/// The page came up or went away (`frame_map`, once per frame). Also the hide switch of the retail map pages.
pub fn set_up(on: bool) {
	let on = WATCH.allow(on); // a tripped watchdog keeps the page down while the frame step keeps asking for it
	super::hide::set(super::hide::Screen::Map, on);
	if UP.swap(on, Ordering::Relaxed) != on {
		*lock() = on.then(State::default);
		if on {
			WATCH.stamp(now());
		}
	}
}

/// The watchdog's question (`host::watchdog`): up, no page finished for 1.5 s, and an empty last draw list.
pub fn silent() -> bool {
	WATCH.silent(up(), now(), last().is_empty())
}

/// The watchdog gives the page up: the retail picture and the classic mouse are back until the frame step asks the page down once.
pub fn trip() {
	set_up(false);
	WATCH.trip();
}

/// The last device the player used (keycaps or pad glyphs): read live, the mouse counts as the keyboard side.
pub fn device() -> Device {
	super::look::device(super::look::start_keyboard())
}

/// An in-level menu with these shell figures was on screen (`frame_map`, any page).
pub fn note_header(level: u32, bolts: u32) {
	HEADER.store((level as u64) << 32 | bolts as u64, Ordering::Relaxed);
}

/// The last noted shell figures: (level, bolts).
pub fn header() -> (u32, u32) {
	let v = HEADER.load(Ordering::Relaxed);
	((v >> 32) as u32, v as u32)
}

/// The window lost the focus: nothing of the page's is held (the keys and the pad stay the game's).
pub fn lose_focus() {
	if let Some(st) = lock().as_mut() {
		st.pad_prev = 0;
	}
}

fn settled(st: &State) -> bool {
	st.snap.as_ref().is_some_and(|m| !m.busy())
}

/// A pad press for the game, only on a settled page (the game answers none while its page flies).
fn press(st: &State, bit: u16) {
	if bit != 0 && settled(st) {
		submit(Command::Press(bit));
	}
}

fn design(st: &State, x: f32, y: f32) -> (f32, f32) {
	Scale::fit(st.win.0, st.win.1).to_design(x, y)
}

fn tab_at(st: &State, x: f32, y: f32) -> Option<usize> {
	let (dx, dy) = design(st, x, y);
	st.layout.as_ref()?.menu.iter().position(|m| m.rect.contains(dx, dy))
}

/// A tab of the tab bar: the Map tab on the mission list goes back to the map (on the map it is the page itself); any other closes the
/// map with Start and opens that pause tab (`map_open::Goal::Pause`, `pause_host` selects it once the root is up).
fn tab(st: &State, t: usize) {
	let Some(b) = st.built.as_ref() else { return };
	if t == b.map_tab {
		if b.stage == Stage::Missions {
			press(st, TRIANGLE);
		}
		return;
	}
	if !settled(st) || map_open::pending().is_some() {
		return;
	}
	submit(Command::Press(START));
	map_open::arm(Goal::Pause(t));
}

fn click(st: &State, x: f32, y: f32) {
	if let Some(t) = tab_at(st, x, y) {
		return tab(st, t);
	}
	let (dx, dy) = design(st, x, y);
	if let Some(bit) = st.built.as_ref().and_then(|b| b.act_at(dx, dy)) {
		press(st, bit);
	}
}

/// A mouse event in window pixels while the page is up. Always taken once a page was drawn (the retail page behind must not also
/// see it).
pub fn mouse(ev: MouseEvent) -> bool {
	if up() && matches!(ev, MouseEvent::Down { .. } | MouseEvent::Wheel { .. }) {
		super::look::note_keyboard(); // the mouse is the keyboard side of the key list
	}
	let g = lock();
	let Some(st) = g.as_ref().filter(|s| s.built.is_some()) else { return false };
	let missions = st.built.as_ref().is_some_and(|b| b.stage == Stage::Missions);
	match ev {
		MouseEvent::Down { button: MouseButton::Left, x, y } => click(st, x, y),
		MouseEvent::Down { button: MouseButton::Right, .. } => press(st, TRIANGLE),
		MouseEvent::Wheel { lines } if lines != 0.0 && missions => press(st, if lines > 0.0 { PAD_UP } else { DOWN }),
		_ => {}
	}
	true
}

/// One winit key: notes the keyboard for the key list's names; never taken (the game's own map keys work as in Classic).
pub fn key(_code: KeyCode, pressed: bool, repeat: bool) -> bool {
	if pressed && !repeat && up() {
		super::look::note_keyboard();
	}
	false
}

/// Display frame, first step: a new pad press means the pad is in use (the key list shows pad buttons).
pub fn begin(pad: Option<PadSample>) {
	let mut g = lock();
	let Some(st) = g.as_mut() else { return };
	let mask = pad.map_or(0, pad_mask);
	if mask & !st.pad_prev != 0 {
		super::look::note_pad();
	}
	st.pad_prev = mask;
}

/// Display frame, last step: keeps this frame's page, its tab layout and the menu; returns the draw list (the shell, its scrim with the
/// crops' holes, the page's panels).
pub fn end(built: Built, snap: Option<&MenuSnap>, win: (u32, u32)) -> Vec<Prim> {
	WATCH.stamp(now());
	cmd::stamp(snap); // the pulses queued until the next frame are made for this page
	let (layout, shell) = page_draw::frame(&built.screen, &FontMeasure);
	let prims = compose(shell, &built);
	if let Some(st) = lock().as_mut() {
		(st.layout, st.snap, st.win, st.built) = (Some(layout), snap.cloned(), win, Some(built));
		st.last = prims.clone();
	}
	prims
}

/// The draw list of the last frame.
pub fn last() -> Vec<Prim> {
	lock().as_ref().map(|s| s.last.clone()).unwrap_or_default()
}

/// A frame that draws no page although the host is held (the map closes): the crops leave with the page, since the overlay they are
/// cut from is going away.
pub fn drop_crops() {
	if let Some(b) = lock().as_mut().and_then(|s| s.built.as_mut()) {
		b.holes.clear();
	}
}

/// The game's own pictures in the page's holes: one crop of the retail overlay per hole (`crop::place`), pushed at the end of the HUD.
/// Called by `frame_ui::hide_retail` every frame; nothing while the page is down or flies in.
pub fn place_crops(hud: &mut Vec<crate::overlay::Quad>, win: (u32, u32)) {
	if !up() {
		return;
	}
	let holes = lock().as_ref().and_then(|s| s.built.as_ref()).map(|b| b.holes.clone()).unwrap_or_default();
	for (hole, uv) in holes {
		super::crop::place(hud, win, hole, uv);
	}
}
