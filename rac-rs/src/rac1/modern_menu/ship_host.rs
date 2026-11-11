//! The host of the modern Galactic Map, MAIN THREAD (v0.5 screens stage F2; the twin of `vendor_host.rs`). While the ship's map pages
//! (0xf list, 0x10 details) are up in the Modern style, `host/handler/frame_ship.rs` builds the page every display frame
//! (`ship::build`) and hands it to `end`; the mouse reaches this file instead of the retail menu (`up()`), and what the player does
//! goes to the game as row requests and pad pulses (`menu_model::cmd`). The KEYBOARD AND THE PAD keep driving the retail page exactly
//! as in the Classic style (`key` never takes a key): Blast off, the exit and L1 / R1 are the game's own presses. The mouse chooses a
//! planet (hover, click on a row or a marker, wheel); a click opens the details (the game's Cross); right click goes back. On the
//! details page the mouse only goes back. `place_art` lays the galaxy painting under the page, `place_planet` the game's own planet
//! picture (a crop of the retail overlay, `ship_pic.rs`) into the card of the right panel.
use super::ship::{Built, Hit, Listing, Stage};
use super::ship_marks;
use super::title::{shown, GRACE_MS};
use super::watchdog::Watch;
use crate::retail::menu_model::{cmd, resolve, submit, Command, Intent};
use crate::retail::mouse_menu::{MenuSnap, CIRCLE, CROSS};
use rac_engine::ui::input::{pad_mask, Device, MouseButton, MouseEvent, PadSample};
use rac_engine::ui::kit::Scale;
use rac_engine::ui::prim::{Prim, Rect, Symbol};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use winit::keyboard::KeyCode;

/// The modern ship page is up (MAIN writes, MAIN reads).
static UP: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();
/// The watchdog's view of this host (`watchdog.rs`).
static WATCH: Watch = Watch::new();
/// The painting decoded once: (width, rows, RGBA8).
static ART: OnceLock<Option<(u32, u32, Vec<u8>)>> = OnceLock::new();

#[derive(Default)]
struct State {
	/// The last frame's page (its rects are what a click hits; `last` repeats its draw list).
	built: Option<Built>,
	/// The menu as the mouse sees it: the target of the requests.
	snap: Option<MenuSnap>,
	win: (u32, u32),
	/// The player's focus: (row, ms).
	local: Option<(usize, u64)>,
	pad_prev: u16,
	/// The planet list of the last List-stage frame (the details page has no list of its own to read).
	memo: Option<Listing>,
}

static ST: Mutex<Option<State>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<State>> {
	ST.lock().unwrap_or_else(|e| e.into_inner())
}

/// The clock of the focus grace (ms since the first call).
pub fn now() -> u64 {
	START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Does the modern ship page own the mouse right now?
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

/// The page came up or went away (`frame_ship`, once per frame). Also the hide switch of the retail ship pages.
pub fn set_up(on: bool) {
	let on = WATCH.allow(on); // a tripped watchdog keeps the page down while the frame step keeps asking for it
	super::hide::set(super::hide::Screen::Ship, on);
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

/// The last device the player used (keycaps or pad glyphs in the hint bar): read live, the mouse counts as the keyboard side.
pub fn device() -> Device {
	super::look::device(super::look::start_keyboard())
}

/// The row the player's mouse chose a moment ago; None once retail's cursor wins.
pub fn local() -> Option<usize> {
	let (i, at) = lock().as_ref()?.local?;
	(now().saturating_sub(at) < GRACE_MS).then_some(i)
}

/// The planet list remembered from the List stage (what the details page shows).
pub fn memo() -> Option<Listing> {
	lock().as_ref()?.memo.clone()
}

/// The first planet row the list showed last frame (`ship::Facts::scroll`); 0 with no page.
pub fn scroll() -> usize {
	lock().as_ref().and_then(|s| s.built.as_ref()).map_or(0, |b| b.first)
}

fn current(st: &State, now: u64) -> usize {
	st.built.as_ref().map_or(0, |b| shown(st.local, b.sel, now))
}

/// Chooses row `i` (a hover or a click): the focus moves at once, the request goes to the game. Only a row the game lets the player
/// choose, only on the list page.
fn select(st: &mut State, i: usize, confirm: bool, now: u64) {
	let Some(b) = st.built.as_ref().filter(|b| b.stage == Stage::List && b.enabled.get(i) == Some(&true)) else { return };
	let item = b.item;
	let Some(m) = st.snap.as_ref().filter(|m| !m.busy()) else { return };
	if let Some(c) = resolve(m, Intent::Select { item, row: i as u32, confirm }) {
		submit(c);
		st.local = Some((i, now));
	}
}

fn back(st: &State) {
	if let Some(c) = st.snap.as_ref().filter(|m| !m.busy()).and_then(|m| resolve(m, Intent::Back)) {
		submit(c);
	}
}

fn hit_at(st: &State, x: f32, y: f32) -> Hit {
	let Some(b) = &st.built else { return Hit::None };
	let (dx, dy) = Scale::fit(st.win.0, st.win.1).to_design(x, y);
	b.hit(dx, dy)
}

/// A click on a button: Cross opens the details of the chosen planet (List) or blasts off (Details), Triangle leaves, Circle is
/// the Infobot. The game answers a pad press only once its page has settled.
fn act(st: &mut State, sym: Symbol, now: u64) {
	let Some(b) = st.built.as_ref() else { return };
	let (stage, cur) = (b.stage, current(st, now));
	match (sym, stage) {
		(Symbol::Triangle, _) => back(st),
		(Symbol::Cross, Stage::List) => select(st, cur, true, now),
		(Symbol::Cross, _) | (Symbol::Circle, _) => {
			if st.snap.as_ref().is_some_and(|m| !m.busy()) {
				submit(Command::Press(if sym == Symbol::Cross { CROSS } else { CIRCLE }));
			}
		}
		_ => {}
	}
}

/// One wheel notch (`up` = toward the list's start): the next choosable planet in that direction.
fn wheel(st: &mut State, up: bool, now: u64) {
	let Some(b) = st.built.as_ref().filter(|b| b.stage == Stage::List) else { return };
	let (cur, n) = (current(st, now), b.enabled.len());
	let to = if up { (0..cur.min(n)).rev().find(|&i| b.enabled[i]) } else { (cur + 1..n).find(|&i| b.enabled[i]) };
	if let Some(to) = to {
		select(st, to, false, now);
	}
}

/// A mouse event in window pixels while the page is up. Always taken (the retail page behind must not also see it).
pub fn mouse(ev: MouseEvent) -> bool {
	if up() && matches!(ev, MouseEvent::Down { .. } | MouseEvent::Wheel { .. }) {
		super::look::note_keyboard(); // the mouse is the keyboard side of the hint bar (a plain move is judged by the window filter)
	}
	let mut g = lock();
	let Some(st) = g.as_mut().filter(|s| s.built.is_some()) else { return false }; // no page drawn yet = nothing to click: the mouse stays free
	let now = now();
	match ev {
		MouseEvent::Move { x, y } => {
			if let Hit::Row(i) = hit_at(st, x, y) {
				if i != current(st, now) {
					select(st, i, false, now);
				}
			}
		}
		MouseEvent::Down { button: MouseButton::Left, x, y } => match hit_at(st, x, y) {
			Hit::Row(i) => select(st, i, true, now),
			Hit::Act(sym) => act(st, sym, now),
			Hit::None => {}
		},
		MouseEvent::Down { button: MouseButton::Right, .. } => back(st),
		MouseEvent::Wheel { lines } if lines != 0.0 => wheel(st, lines > 0.0, now),
		_ => {}
	}
	true
}

/// One winit key. Records that the keyboard is in use and drops the mouse's focus (the key moves the game's cursor, which the page
/// must show at once, and the Details page must name that planet); the key itself is never taken (the game's own keys work as in
/// Classic).
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

/// Display frame, first step: a new pad press means the pad is in use, and the game's cursor (not the mouse's focus) is shown.
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

/// Display frame, last step: keeps this frame's page and menu (and the planet list on the List stage); returns the draw list. A page
/// that flies in has no planet picture yet (`ship_pic::settle`).
pub fn end(mut built: Built, listing: Option<&Listing>, snap: Option<&MenuSnap>, win: (u32, u32)) -> Vec<Prim> {
	if let Some(m) = snap {
		super::ship_pic::settle(&mut built, m.state);
	}
	let prims = built.prims.clone();
	cmd::stamp(snap); // the pulses queued until the next frame are made for this page
	WATCH.stamp(now());
	let (stage, sel) = (built.stage, built.sel);
	if let Some(st) = lock().as_mut() {
		(st.snap, st.win, st.built) = (snap.cloned(), win, Some(built));
		if let (Stage::List, Some(l)) = (stage, listing) {
			// `sel` is the game's cursor unless the mouse chose a row a moment ago (a key or a pad press drops that focus), so the
			// Details page names the planet the game flies to.
			st.memo = Some(Listing { sel, ..l.clone() });
		}
	}
	prims
}

/// The draw list of the last frame (a frame whose page cannot be built repeats it).
pub fn last() -> Vec<Prim> {
	lock().as_ref().and_then(|s| s.built.as_ref()).map(|b| b.prims.clone()).unwrap_or_default()
}

/// The painting's two HUD quads for a HUD of `win` pixels: a black quad over the whole picture (the bars of a wide or tall window)
/// and the painting at the 1280 x 720 design rectangle's place (`Scale::fit`, like the page).
fn art_quads(win: (u32, u32)) -> [crate::overlay::Quad; 2] {
	let r = Scale::fit(win.0, win.1).to_window(Rect::new(0.0, 0.0, 1280.0, 720.0));
	let quad = |rect: [f32; 4], c: f32| crate::overlay::Quad { tex: crate::viewer::MENU_ART_TEX, rect, uv: [0.0, 0.0, 1.0, 1.0], colour: [c, c, c, 1.0], angle: 0.0, corners: None };
	[quad([0.0, 0.0, win.0 as f32, win.1 as f32], 0.0), quad([r.x, r.y, r.w, r.h], 1.0)]
}

/// The painting under the page (`art_quads`), inserted FIRST in the HUD so the retail quads and the UI pass draw over it. `win` is
/// the HUD's pixel space, the render size (`config`, as for `place_planet`; not the window: at a Render scale below 100 % window
/// pixels drew the painting twice too big). Called by `frame_ui::hide_retail` every frame; frees the texture when the painting is
/// not wanted.
pub fn place_art(g: &mut crate::viewer::Gpu, win: (u32, u32)) {
	let want = up() && lock().as_ref().and_then(|s| s.built.as_ref()).is_some_and(|b| b.art);
	if !want {
		g.set_menu_art_anim(None);
		if g.has_menu_art() {
			g.clear_menu_art();
		}
		return;
	}
	if !g.has_menu_art() {
		let art = ART.get_or_init(|| {
			crate::texpack::png::load(ship_marks::ART_PNG).ok().map(|i| {
				let mut px = i.levels[0].clone();
				super::ship_anim::mark_stars(&mut px, i.w, i.h); // the star map the twinkle reads (alpha channel)
				(i.w, i.h, px)
			})
		});
		let Some((w, h, px)) = art else { return };
		g.set_menu_art(*w, *h, px);
	}
	let mut quads = art_quads(win);
	super::ship_anim::tick();
	if super::ship_anim::enabled() {
		quads[1].tex = crate::viewer::MENU_ART_ANIM_TEX; // the painting through the animation pipeline (ship_anim.rs)
		g.set_menu_art_anim(Some(super::ship_anim::anim_at(super::ship_anim::clock_s())));
	} else {
		g.set_menu_art_anim(None);
	}
	g.hud.splice(0..0, quads);
	g.hud_split = None;
}

/// A frame that draws no page although the host is held (the menu closes): the planet picture leaves with the page, since the
/// overlay it is cut from is flying away. Called by `host/handler/frame_ship.rs`.
pub fn drop_planet() {
	if let Some(b) = lock().as_mut().and_then(|s| s.built.as_mut()) {
		b.planet = None;
	}
}

/// The game's own planet picture in the page's card: a crop of the retail overlay (`ship_pic`, `crop::place`) pushed at the end of the
/// HUD, over the hole the page leaves. Called by `frame_ui::hide_retail` every frame, after `place_art`; nothing while the page is down,
/// on the details stage, or while the page flies in.
pub fn place_planet(hud: &mut Vec<crate::overlay::Quad>, win: (u32, u32)) {
	if !up() {
		return;
	}
	let Some(hole) = lock().as_ref().and_then(|s| s.built.as_ref()).and_then(|b| b.planet) else { return };
	super::crop::place(hud, win, hole, super::ship_pic::uv(super::ship_pic::inset_now()));
}

#[cfg(test)]
mod tests;
