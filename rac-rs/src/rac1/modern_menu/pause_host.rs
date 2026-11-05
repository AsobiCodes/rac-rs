//! The host of the modern pause menu, MAIN THREAD (v0.5 screens stage C). While the pause menu shows one of the pages of
//! `pause::in_set` in the Modern style, `host/handler/frame_pause.rs` builds the page every display frame (`pause::build`) and hands
//! it to `end`; keys, mouse and the pad reach this file instead of the retail menu (`up()`), and what the player chooses goes to
//! the game as `Select` / `Slide` requests and pad pulses (`menu_model::cmd`). The twin of `title_host.rs`, with these differences:
//! a message box (Quit Game, Overwrite) answers ONLY its own pad buttons and Enter / Esc (any other pad press is ignored, so Cross
//! never confirms Quit), Circle and Triangle both go back, the wheel walks the list, a click or drag on a volume bar sets it, and
//! the worker's `filter` lets Start, Select and R3 through (Start closes the pause menu in retail). The icon-grid pages (Weapons,
//! Gadgets, Items: local focus in a grid, the game's D-pad at its edge, the preview crop) are the child `pause_host/grid.rs`.
use super::host::map_key;
use super::pause::Built;
use super::title::shown;
use super::watchdog::Watch;
use crate::retail::menu_model::extra::VOLUME_MAX;
use crate::retail::menu_model::pages::Page;
use crate::retail::menu_model::{cmd, resolve, submit, Command, Intent};
use crate::retail::mouse_menu::{MenuSnap, LEFT, RIGHT, TRIANGLE};
use crate::retail::mouse_slider::Slide;
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::input::{key_action, pad_mask, Action, Device, MouseButton, MouseEvent, PadSample, Repeater, PAD_ACTIONS};
use rac_engine::ui::kit::page::{PageHit, PageLayout};
use rac_engine::ui::kit::HintKind;
use rac_engine::ui::kit::{page_draw, Scale};
use rac_engine::ui::prim::Prim;
use std::sync::atomic::{AtomicBool, AtomicU16, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use winit::keyboard::KeyCode;

/// The modern pause page is up (MAIN writes, MAIN and the worker read).
static UP: AtomicBool = AtomicBool::new(false);
/// The page up is the Sound page: its Left / Right (and the stick's x) keep reaching the game, which steps the bars.
static SOUND: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();
/// The watchdog's view of this host (`watchdog.rs`).
static WATCH: Watch = Watch::new();
/// The pad bits the game keeps while the page is up: Select 0x100, R3 0x400, Start 0x800.
const PASS: u16 = 0x100 | 0x400 | 0x800;
/// The root listed the Goodies entry at some point this run (the modern map page's tab bar shows the tab too; MAIN).
static GOODIES_SEEN: AtomicBool = AtomicBool::new(false);
/// The held pad bits `filter` hid from the game in the last field the page was up (WK writes).
static HIDDEN: AtomicU16 = AtomicU16::new(0);
/// The bits still held when the page went down: kept from the game until each is released, so a Circle / Triangle that closed the
/// menu does not reach gameplay as a fresh press (R3-1; the Options page's `host::pad_filter` rule).
static EAT: AtomicU16 = AtomicU16::new(0);

#[derive(Default)]
struct State {
	built: Option<Built>,
	layout: Option<PageLayout>,
	snap: Option<MenuSnap>,
	win: (u32, u32),
	/// The player's focus: (page, flat row, ms).
	local: Option<(Page, usize, u64)>,
	rep: Repeater,
	pad_prev: u16,
	primed: bool,
	/// The root listed the Goodies entry in this pause.
	goodies: bool,
	/// The panel row whose volume bar the left button drags.
	drag: Option<usize>,
	/// The last draw list (a frame on a page the builder cannot read repeats it).
	last: Vec<Prim>,
	/// A tab clicked on a child page: Back to the root, then Select it (`child_tabs.rs`).
	pending: Option<child_tabs::Pending>,
	/// When the page on screen first could not be built (`hold_unbuilt`); None after a built frame.
	unbuilt: Option<u64>,
}

/// How long (ms) the last picture stays over a landed page the builder cannot read yet (`hold_unbuilt`).
const HOLD_MS: u64 = 250;

static ST: Mutex<Option<State>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<State>> {
	ST.lock().unwrap_or_else(|e| e.into_inner())
}

/// The clock of the focus grace (ms since the first call).
pub fn now() -> u64 {
	START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Does the modern pause page own keys, mouse and pad right now?
pub fn up() -> bool {
	UP.load(Ordering::Relaxed)
}

/// The window size of the last drawn frame ((0, 0) before the first one): the scripted mouse (`host::scripted_mouse`) needs pixels.
pub fn win() -> (u32, u32) {
	lock().as_ref().map_or((0, 0), |s| s.win)
}

/// The page came up or went away (`frame_pause`, once per frame). Also the hide switch of the retail pause picture.
pub fn set_up(on: bool) {
	let on = WATCH.allow(on); // a tripped watchdog keeps the page down while the frame step keeps asking for it
	super::hide::set(super::hide::Screen::Pause, on);
	if UP.swap(on, Ordering::Relaxed) != on {
		let mut g = lock();
		if !on && g.as_ref().is_some_and(ach::is_ach) {
			crate::achievements::close(); // the list the page showed goes with the page (`close_ach`)
		}
		*g = on.then(State::default);
		drop(g);
		if on {
			WATCH.stamp(now());
		} else {
			EAT.store(HIDDEN.swap(0, Ordering::Relaxed), Ordering::Relaxed);
		}
	}
	if !on {
		SOUND.store(false, Ordering::Relaxed);
	}
}

/// The pause menu closes under the page (`frame_pause`: the closing hold keeps an empty page up): an achievements list the last page
/// showed closes with it (review R22 M1: pad Start passes on every page and closes the retail menu, as in retail; the list left open
/// was drawn as the HUD list over the world for the ~12 closing fields, until the pause mode ended). `set_up(false)` does the same.
pub fn close_ach() {
	if lock().as_ref().is_some_and(ach::is_ach) {
		crate::achievements::close();
	}
}

/// The watchdog's question (`host::watchdog`): up, no page finished for 1.5 s, and an empty last draw list.
pub fn silent() -> bool {
	WATCH.silent(up(), now(), lock().as_ref().is_none_or(|s| s.last.is_empty()))
}

/// The watchdog gives the page up: the retail picture and the classic mouse are back until the frame step asks the page down once.
pub fn trip() {
	set_up(false);
	WATCH.trip();
}

/// The last device the player used (decides keycaps or pad glyphs in the hint bar): read live, the mouse counts as the keyboard side.
pub fn device() -> Device {
	super::look::device(super::look::start_keyboard())
}

/// The root listed the Goodies entry in this pause (the child pages keep the tab).
pub fn goodies() -> bool {
	lock().as_ref().is_some_and(|s| s.goodies)
}

/// The root listed the Goodies entry in some pause of this run (`map_host`'s tab bar).
pub fn goodies_seen() -> bool {
	GOODIES_SEEN.load(Ordering::Relaxed)
}

/// The player's own focus on `page`: (flat row, ms) while one is set.
pub fn local(page: Page) -> Option<(usize, u64)> {
	lock().as_ref().and_then(|s| local_focus(s, page))
}

/// `b` without the bits of `eat` still held; a released bit leaves `eat` for good.
fn eat_held(eat: &AtomicU16, b: u16) -> u16 {
	let left = eat.load(Ordering::Relaxed) & b;
	eat.store(left, Ordering::Relaxed);
	b & !left
}

/// Worker, once per field in `tick_keys` (via `title_host::filter` outside the front end): the game's pad while the page is up. Only
/// Start, Select and R3 stay the player's (and Left / Right and the stick's x on the Sound page); the rest is neutral, so the
/// menu is steered by the queued pulses and requests alone.
pub fn filter(b: &mut u16, s: &mut [u8; 4]) {
	if !up() {
		*b = eat_held(&EAT, *b);
		return;
	}
	let sound = SOUND.load(Ordering::Relaxed);
	let lx = s[2];
	let kept = PASS | if sound { LEFT | RIGHT } else { 0 };
	HIDDEN.store(*b & !kept, Ordering::Relaxed);
	*b &= kept;
	*s = [0x80; 4];
	if sound {
		s[2] = lx;
	}
}

fn local_focus(st: &State, page: Page) -> Option<(usize, u64)> {
	st.local.filter(|l| l.0 == page).map(|l| (l.1, l.2))
}

fn current(st: &State, now: u64) -> usize {
	st.built.as_ref().map_or(0, |b| shown(local_focus(st, b.page), b.sel, now))
}

/// Is flat row `i` the Map tab of the root page (`pause::MAP_TAB`: no root row, the host opens the map itself)?
fn is_map_tab(st: &State, i: usize) -> bool {
	st.built.as_ref().is_some_and(|b| b.root && b.ach.is_none() && i == b.map_tab)
}

/// The Map tab was chosen (Enter, a click, or a click on it from a child page): Select closes the pause menu from any page and
/// `map_open` presses Select once more in the first gameplay field, which opens the planet map [C: map_open.rs]. Only on a settled
/// page and once (a second click while the map is on its way does nothing).
pub(super) fn open_map(st: &State) {
	if crate::retail::map_open::pending().is_some() || !st.snap.as_ref().is_some_and(|m| !m.busy()) {
		return;
	}
	submit(Command::Press(crate::retail::map_open::SELECT));
	crate::retail::map_open::arm(crate::retail::map_open::Goal::Map);
}

/// Can flat row `i` be chosen (it has a target and the game lets that row be selected, or it is the Map tab)?
fn live(st: &State, i: usize) -> bool {
	if is_map_tab(st, i) {
		return true;
	}
	match (st.built.as_ref().and_then(|b| b.target(i)), &st.snap) {
		(Some((item, row)), Some(m)) => resolve(m, Intent::Select { item, row, confirm: false }).is_some(),
		_ => false,
	}
}

/// Chooses flat row `i` (a hover, a key or a click): the focus moves at once, the request goes to the game.
fn select(st: &mut State, i: usize, confirm: bool, now: u64) {
	let Some((page, target)) = st.built.as_ref().map(|b| (b.page, b.target(i))) else { return };
	st.local = Some((page, i, now));
	if is_map_tab(st, i) {
		if confirm {
			open_map(st);
		}
		return;
	}
	if let (Some((item, row)), Some(m)) = (target, &st.snap) {
		if let Some(c) = resolve(m, Intent::Select { item, row, confirm }) {
			submit(c);
		}
	}
}

/// `select` with a click for a tab switch's last step (`child_tabs::poll`): retail's root cursor moves to the tab without its move sound
/// (`mouse_instant::quiet`), so the click is heard once, as the new tab's open sound (user 2026-11-04, test build 10: "when u switch
/// tabs in the menu and i still hear the 'double' sound").
fn select_quiet(st: &mut State, i: usize, now: u64) {
	let Some((page, target)) = st.built.as_ref().map(|b| (b.page, b.target(i))) else { return };
	st.local = Some((page, i, now));
	if let (Some((item, row)), Some(m)) = (target, &st.snap) {
		if let Some(c) = resolve(m, Intent::Select { item, row, confirm: true }) {
			if let Command::Request(r) = c {
				crate::retail::mouse_instant::quiet(r);
			}
			submit(c);
		}
	}
}

/// Moves the focus to the next row in direction `dir` that can be chosen (wrapping); nothing chosable = no move.
fn step(st: &mut State, dir: isize, now: u64) {
	let n = st.built.as_ref().map_or(0, |b| b.rows());
	let cur = current(st, now);
	for k in 1..=n {
		let i = (cur as isize + dir * k as isize).rem_euclid(n as isize) as usize;
		if live(st, i) {
			select(st, i, false, now);
			return;
		}
	}
}

fn back(st: &State) {
	if let Some(c) = st.snap.as_ref().and_then(|m| resolve(m, Intent::Back)) {
		submit(c);
	}
}

fn act(st: &mut State, a: Action, now: u64) {
	if ach::is_ach(st) {
		return ach::act(st, a); // the achievements page (pause_host/ach.rs)
	}
	let Some(b) = &st.built else { return };
	if !b.buttons.is_empty() {
		// The message box answers Enter (its first button, Yes) and Esc (its last, No); nothing else.
		let bit = match a {
			Action::Confirm => b.buttons.first(),
			Action::Back => b.buttons.last(),
			_ => None,
		};
		if let Some(&bit) = bit {
			submit(Command::Press(bit));
		}
		return;
	}
	if b.grid.is_some() {
		return grid::act_grid(st, a, now);
	}
	let root = b.root;
	let dir = match a {
		Action::Up => -1,
		Action::Down => 1,
		Action::Left | Action::TabPrev if root => -1,
		Action::Right | Action::TabNext if root => 1,
		Action::Confirm | Action::Peek => return select(st, current(st, now), true, now),
		Action::Back => return back(st),
		_ => return,
	};
	step(st, dir, now);
}

/// One winit key. True = the page took the press; a release always goes on to the game so no key sticks, and the Sound page's
/// Left / Right stay the game's (they move the bars).
pub fn key(code: KeyCode, pressed: bool, repeat: bool) -> bool {
	// A Controls row waits for the next key (binds.rs): every key is the capture's, none is a menu key.
	if crate::retail::binds::capturing() {
		return false;
	}
	let mut g = lock();
	let Some(st) = g.as_mut().filter(|s| s.built.is_some()) else { return false }; // never take a key for a page that was not drawn
	let Some(k) = map_key(code) else { return false };
	let Some(a) = key_action(k, super::host::shift()) else { return false };
	if SOUND.load(Ordering::Relaxed) && matches!(a, Action::Left | Action::Right) {
		return false;
	}
	if repeat {
		return true;
	}
	if !pressed {
		st.rep.release(code as u32);
		return false;
	}
	let now = now();
	super::look::note_keyboard();
	st.rep.press(code as u32, a, now);
	act(st, a, now);
	true
}

fn design_x(st: &State, x: f32) -> f32 {
	Scale::fit(st.win.0, st.win.1).to_design(x, 0.0).0
}

fn hit_at(st: &State, x: f32, y: f32) -> PageHit {
	match (&st.built, &st.layout) {
		(Some(b), Some(l)) => {
			let (dx, dy) = Scale::fit(st.win.0, st.win.1).to_design(x, y);
			l.hit(&b.screen, dx, dy)
		}
		_ => PageHit::None,
	}
}

/// The flat row of a hit: the tabs on the root page, the panel's rows on every other.
fn flat(st: &State, h: PageHit) -> Option<usize> {
	let root = st.built.as_ref()?.root;
	match h {
		PageHit::Menu(i) if root => Some(i),
		PageHit::Item(i) if !root => Some(i),
		_ => None,
	}
}

/// The flat row under window point (x, y): a cell on the icon-grid pages, else the row of the page hit.
fn flat_at(st: &State, x: f32, y: f32) -> Option<usize> {
	if grid::is_grid(st) { grid::cell_at(st, x, y) } else { flat(st, hit_at(st, x, y)) }
}

/// The track of panel row `i`'s volume bar: (left edge, width) in design pixels.
fn bar_of(st: &State, i: usize) -> Option<(f32, f32)> {
	let item = st.layout.as_ref()?.panel.as_ref()?.items.get(i)?;
	item.bar.as_ref().map(|r| (r.x, r.w))
}

/// Sets the volume bar of panel row `i` to the pointer's x (a click, or a drag step).
fn slide_to(st: &mut State, i: usize, x: f32, now: u64) {
	let (Some((left, width)), Some(b)) = (bar_of(st, i), st.built.as_ref()) else { return };
	let (page, Some((item, row))) = (b.page, b.target(i)) else { return };
	let to = ((design_x(st, x) - left) / width.max(1.0)).clamp(0.0, 1.0) * VOLUME_MAX as f32;
	st.local = Some((page, i, now));
	if let Some(c) = st.snap.as_ref().and_then(|m| resolve(m, Intent::Slide { item, row, slide: Slide::To(to.round() as i32) })) {
		submit(c);
	}
}

/// Is window x over the volume bar of panel row `i` (a little beyond its ends, so the end stops are easy to hit)?
fn on_bar(st: &State, i: usize, x: f32) -> bool {
	bar_of(st, i).is_some_and(|(left, width)| (left - 8.0..=left + width + 8.0).contains(&design_x(st, x)))
}

fn click(st: &mut State, x: f32, y: f32, now: u64) {
	if let Some(tab) = child_tabs::tab_at(st, x, y) {
		return child_tabs::start(st, tab, now);
	}
	if grid::click_grid(st, x, y, now) {
		return;
	}
	match hit_at(st, x, y) {
		PageHit::Button(i) => {
			if let Some(&bit) = st.built.as_ref().and_then(|b| b.buttons.get(i)) {
				submit(Command::Press(bit));
			}
		}
		PageHit::Item(i) if on_bar(st, i, x) => {
			st.drag = Some(i);
			slide_to(st, i, x, now);
		}
		// The hint bar's buttons do what their keys do (Enter = the selected row, Esc = back).
		PageHit::Hint(HintKind::Change) => act(st, Action::Confirm, now),
		PageHit::Hint(HintKind::Back) => act(st, Action::Back, now),
		h => {
			if let Some(i) = flat(st, h).filter(|&i| live(st, i)) {
				select(st, i, true, now);
			}
		}
	}
}

/// A mouse event in window pixels while the page is up: a hover selects (instantly), a left click confirms (on a volume bar it
/// sets the volume, and a drag follows), a right click goes back, the wheel walks the list. Always taken (the retail menu behind
/// must not also see it).
pub fn mouse(ev: MouseEvent) -> bool {
	if up() && matches!(ev, MouseEvent::Down { .. } | MouseEvent::Wheel { .. }) {
		super::look::note_keyboard(); // the mouse is the keyboard side of the hint bar (a plain move is judged by the window filter)
	}
	let mut g = lock();
	let Some(st) = g.as_mut().filter(|s| s.built.is_some()) else { return false }; // no page drawn yet = nothing to click: the mouse stays free
	let now = now();
	if ach::is_ach(st) {
		ach::mouse(st, ev, now); // the achievements page: hover, wheel, tabs, Back (pause_host/ach.rs)
		return true;
	}
	match ev {
		MouseEvent::Move { x, y } => match st.drag {
			Some(i) => slide_to(st, i, x, now),
			None => {
				if let Some(i) = flat_at(st, x, y).filter(|&i| i != current(st, now) && live(st, i)) {
					select(st, i, false, now);
				}
			}
		},
		MouseEvent::Down { button: MouseButton::Left, x, y } => click(st, x, y, now),
		MouseEvent::Down { button: MouseButton::Right, .. } => act(st, Action::Back, now),
		MouseEvent::Up { button: MouseButton::Left } => st.drag = None,
		// The wheel walks the flat cell list of a grid page (across its groups), the rows of any other.
		MouseEvent::Wheel { lines } if lines != 0.0 && grid::is_grid(st) => step(st, if lines > 0.0 { -1 } else { 1 }, now),
		MouseEvent::Wheel { lines } if lines != 0.0 => act(st, if lines > 0.0 { Action::Up } else { Action::Down }, now),
		_ => {}
	}
	true
}

/// The window lost the focus: forget the held keys, buttons and a bar drag.
pub fn lose_focus() {
	EAT.store(0, Ordering::Relaxed);
	if let Some(st) = lock().as_mut() {
		st.rep.clear();
		st.drag = None;
		st.pending = None;
	}
}

/// The pad's new presses (`mask` against the last frame's) act on the page of the last frame. A message box takes only its own
/// buttons; elsewhere Circle and Triangle both go back and Cross confirms.
fn pad_core(st: &mut State, mask: u16, now: u64) {
	if !st.primed {
		// The press that brought the page up must not act.
		(st.pad_prev, st.primed) = (mask, true);
		return;
	}
	let (down, up) = (mask & !st.pad_prev, st.pad_prev & !mask);
	st.pad_prev = mask;
	if down != 0 {
		super::look::note_pad();
	}
	for &(bit, a) in PAD_ACTIONS.iter() {
		if up & bit != 0 {
			st.rep.release(0x10000 + bit as u32);
		}
		if down & bit == 0 {
			continue;
		}
		match st.built.as_ref().map(|b| &b.buttons) {
			Some(btns) if !btns.is_empty() => {
				if btns.contains(&bit) {
					submit(Command::Press(bit));
				}
			}
			_ => {
				let a = if bit == TRIANGLE { Action::Back } else { a };
				st.rep.press(0x10000 + bit as u32, a, now);
				act(st, a, now);
			}
		}
	}
}

/// The Map tab on the root page, every display frame: the player's focus on it stays (the game's cursor is still on a root row, so
/// the focus grace would give the tab back to that row); a tab asked for from the modern map page (`map_open::take_tab`) is chosen once
/// the root has settled.
fn map_tab_frame(st: &mut State, now: u64) {
	let Some(b) = st.built.as_ref().filter(|b| b.root && b.ach.is_none()) else { return };
	if let Some((page, i, _)) = st.local.filter(|l| l.0 == b.page && l.1 == b.map_tab) {
		st.local = Some((page, i, now));
	}
	if st.snap.as_ref().is_some_and(|m| !m.busy()) {
		if let Some(tab) = crate::retail::map_open::take_tab() {
			child_tabs::after_map(st, tab, now);
		}
	}
}

/// Display frame, first step: the key repeat clock and the pad's new presses act on the page of the last frame.
pub fn begin(pad: Option<PadSample>) {
	let mut g = lock();
	let Some(st) = g.as_mut() else { return };
	let now = now();
	for a in st.rep.poll(now) {
		act(st, a, now);
	}
	map_tab_frame(st, now);
	child_tabs::poll(st, now);
	pad_core(st, pad.map_or(0, pad_mask), now);
}

/// Display frame, last step: keeps this frame's page, its layout and the menu it was built from; returns the draw list.
pub fn end(built: Built, snap: Option<&MenuSnap>, win: (u32, u32)) -> Vec<Prim> {
	// Left / Right stay the game's on the Sound bars and on the Controls / Mods pages (the section stepper reads them).
	// The achievements page over one of them keeps them (it pages with Left / Right).
	SOUND.store(built.ach.is_none() && matches!(built.page, Page::Sound | Page::PcControls | Page::PcMods), Ordering::Relaxed);
	WATCH.stamp(now());
	cmd::stamp(snap); // the pulses queued until the next frame are made for this page
	let (layout, mut prims) = page_draw::frame(&built.screen, &FontMeasure);
	if let Some(g) = &built.grid {
		prims = grid::with_grid(prims, g);
	}
	if let Some(a) = &built.ach {
		prims = ach::with_ach(prims, &layout, a);
	}
	if built.goodies {
		GOODIES_SEEN.store(true, Ordering::Relaxed);
	}
	if let Some(st) = lock().as_mut() {
		st.goodies |= built.goodies;
		(st.layout, st.snap, st.win) = (Some(layout), snap.cloned(), win);
		st.built = Some(built);
		st.last = prims.clone();
		st.unbuilt = None;
	}
	prims
}

/// A landed page of the modern set that `pause::build` cannot read in this frame: its facts can come a field after its items (the
/// Gadgets grid right after a tab switch; user 2026-11-04, test build 10: the classic Gadgets page flashed mid-switch, with a hitch).
/// True = the last picture stays (`repeat`): while the modern page is up, for up to `HOLD_MS` from the first such frame; after that the
/// classic page is the picture (a page the builder really cannot draw). A built frame (`end`) starts the clock over.
pub fn hold_unbuilt(now: u64) -> bool {
	if !up() {
		return false;
	}
	let mut g = lock();
	let Some(st) = g.as_mut() else { return false };
	let since = *st.unbuilt.get_or_insert(now);
	now.saturating_sub(since) <= HOLD_MS
}

/// The draw list of the last frame.
pub fn last() -> Vec<Prim> {
	lock().as_ref().map(|s| s.last.clone()).unwrap_or_default()
}

/// The last picture again, for a frame whose page cannot be built yet (a page flying in): the game is already on the new page, so the
/// Sound page's Left / Right and stick pass-through (`filter`) ends here, not at the next `end`.
pub fn repeat() -> Vec<Prim> {
	SOUND.store(false, Ordering::Relaxed);
	last()
}

mod ach;
mod child_tabs;
mod grid;
pub use grid::place_preview;

#[cfg(test)]
mod tests;
