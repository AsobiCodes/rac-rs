//! The host of the modern title screens, MAIN THREAD (v0.5 screens stage B). While the title shows a menu page in the Modern
//! style, `host/front_modern.rs` builds the page every display frame (`title::build`) and hands it to `end`; keys, mouse and the pad
//! reach this file instead of the retail menu (`up()`), and what the player chooses goes to the game as `Select` requests and pad
//! pulses (`menu_model::cmd`). The player's own focus (`local`) wins over retail's cursor for `title::GRACE_MS`, so the page
//! reacts in the very frame of the key. The worker side is `filter` (one call in `retail/input.rs tick_keys`): the game sees a
//! neutral pad while the page is up, except Left/Right on the Sound page (the bars step per held field in retail).
use super::host::map_key;
use super::title::{shown, Built};
use super::watchdog::Watch;
use crate::frontend::Page;
use crate::retail::menu_model::{cmd, resolve, submit, Command, Intent};
use crate::retail::mouse_menu::{LEFT, MenuSnap, CIRCLE, RIGHT, TRIANGLE};
use crate::retail::title_quit::{self, NATIVE_ITEM};
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::input::{key_action, pad_mask, Action, Device, MouseButton, MouseEvent, PadSample, Repeater, PAD_ACTIONS};
use rac_engine::ui::kit::page::{PageHit, PageLayout};
use rac_engine::ui::kit::HintKind;
use rac_engine::ui::kit::{page_draw, Scale};
use rac_engine::ui::prim::Prim;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;
use winit::keyboard::KeyCode;

/// The modern title page is up (MAIN writes, MAIN and the worker read).
static UP: AtomicBool = AtomicBool::new(false);
/// The page up is the Sound page: its Left / Right keep reaching the game.
static SOUND: AtomicBool = AtomicBool::new(false);
/// A memory card message box (front mode 4) is up: the game keeps the keys and the pad, the page only shows the box.
static BOX: AtomicBool = AtomicBool::new(false);
static START: OnceLock<Instant> = OnceLock::new();
/// The watchdog's view of this host (`watchdog.rs`).
static WATCH: Watch = Watch::new();

#[derive(Default)]
struct State {
	built: Option<Built>,
	layout: Option<PageLayout>,
	snap: Option<MenuSnap>,
	win: (u32, u32),
	/// Retail's selected row of the page now.
	sel: usize,
	/// The player's focus: (page, flat row, ms).
	local: Option<(Page, usize, u64)>,
	rep: Repeater,
	pad_prev: u16,
	primed: bool,
	/// The last draw list (a frame on an unknown page repeats it).
	last: Vec<Prim>,
	/// Since when (ms) the front end shows a page `page_of` cannot map while this page is up (`gives_up`).
	stale_since: Option<u64>,
	/// The slot card a click's queued steps take retail's cursor to: (page, card, ms of the click) (`title_slots::click`).
	aim: Option<(Page, usize, u64)>,
}

/// How long the last picture stays for a front page the title cannot map (a page change in flight) before the classic menu takes over.
pub const GIVE_UP_MS: u64 = 2000;

static ST: Mutex<Option<State>> = Mutex::new(None);

fn lock() -> std::sync::MutexGuard<'static, Option<State>> {
	ST.lock().unwrap_or_else(|e| e.into_inner())
}

fn now_ms() -> u64 {
	START.get_or_init(Instant::now).elapsed().as_millis() as u64
}

/// Does the modern title page own keys, mouse and pad right now?
pub fn up() -> bool {
	UP.load(Ordering::Relaxed)
}

/// The window size of the last drawn frame ((0, 0) before the first one): the scripted mouse (`host::scripted_mouse`) needs pixels.
pub fn win() -> (u32, u32) {
	lock().as_ref().map_or((0, 0), |s| s.win)
}

/// The page came up or went away (`front_modern`, once per frame).
pub fn set_up(on: bool) {
	let on = WATCH.allow(on); // a tripped watchdog keeps the page down while the frame step keeps asking for it
	super::hide::set(super::hide::Screen::Title, on);
	if UP.swap(on, Ordering::Relaxed) != on {
		*lock() = on.then(State::default);
		if on {
			WATCH.stamp(now_ms());
		}
		title_quit::clear(); // the Quit entry's state belongs to one style: a switch starts it fresh (a frame in the same style does not)
	}
	if !on {
		SOUND.store(false, Ordering::Relaxed);
		BOX.store(false, Ordering::Relaxed);
	}
}

/// The watchdog's question (`host::watchdog`): up, no page finished for 1.5 s, and an empty last draw list.
pub fn silent() -> bool {
	WATCH.silent(up(), now_ms(), lock().as_ref().is_none_or(|s| s.last.is_empty()))
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

/// Once per display frame: `stale` = the front end is on a page `page_of` cannot map while the page is up (the last picture is kept).
/// True after `GIVE_UP_MS` of that: the caller takes the modern page down, the classic menu draws and the game gets its pad and keys
/// back (an unmapped page that stays would be a soft lock).
pub fn gives_up(stale: bool) -> bool {
	gives_up_at(stale, now_ms())
}

fn gives_up_at(stale: bool, now: u64) -> bool {
	let mut g = lock();
	let Some(st) = g.as_mut() else { return false };
	if !stale {
		st.stale_since = None;
		return false;
	}
	let since = *st.stale_since.get_or_insert(now);
	now.saturating_sub(since) > GIVE_UP_MS
}

/// Worker, once per field in `tick_keys` (`front` = the front end is running): the game's pad while the page is up. A neutral pad,
/// except Left/Right (and the left stick's x) on the Sound page and everything during a card message box.
pub fn filter(b: &mut u16, s: &mut [u8; 4], front: bool) {
	if !front {
		return super::pause_host::filter(b, s); // in a level: the modern pause page's pad (pause_host.rs)
	}
	if !up() || BOX.load(Ordering::Relaxed) {
		return;
	}
	let sound = SOUND.load(Ordering::Relaxed);
	let (keep, lx) = (*b & (LEFT | RIGHT), s[2]);
	*b = if sound { keep } else { 0 };
	*s = [0x80; 4];
	if sound {
		s[2] = lx;
	}
}

fn local_focus(st: &State, page: Page) -> Option<(usize, u64)> {
	st.local.filter(|l| l.0 == page).map(|l| (l.1, l.2))
}

fn current(st: &State, now: u64) -> usize {
	let page = st.built.as_ref().map(|b| b.page);
	if page == Some(Page::Root) && title_quit::focus() {
		return title_quit::ROW; // the host's own Quit entry keeps the focus while retail's cursor rests on Options
	}
	shown(page.and_then(|p| local_focus(st, p)), retail_sel(st, page, st.sel, now), now)
}

/// Retail's selected row of `page`, except on a slot cards page where a click's steps are still walking its cursor: the clicked
/// card (`title_slots::shown`).
fn retail_sel(st: &State, page: Option<Page>, sel: usize, now: u64) -> usize {
	match (page, st.aim) {
		(Some(p), Some((ap, card, at))) if ap == p && super::title_slots::is_cards(p) => super::title_slots::shown(Some((card, at)), sel, now),
		_ => sel,
	}
}

/// The row to draw selected for `page`: the player's own focus while it is fresh, else retail's `sel`.
pub fn display_sel(page: Page, sel: usize) -> usize {
	let g = lock();
	match g.as_ref() {
		Some(_) if page == Page::Root && title_quit::focus() => title_quit::ROW,
		Some(st) => shown(local_focus(st, page), retail_sel(st, Some(page), sel, now_ms()), now_ms()),
		None => sel,
	}
}

/// Can flat row `i` be chosen (it has a target and the game lets that row be selected)?
fn live(st: &State, i: usize) -> bool {
	match (st.built.as_ref().and_then(|b| b.target(i)), &st.snap) {
		(Some((NATIVE_ITEM, _)), _) => true, // the host's Quit entry is always there to choose
		(Some((item, row)), Some(m)) => resolve(m, Intent::Select { item, row, confirm: false }).is_some(),
		_ => false,
	}
}

/// Chooses flat row `i` (a hover, a key or a click): the focus moves at once, the request goes to the game. A row the game
/// refuses (a disabled label, an inert entry) sends nothing and does not take the focus, so the highlight never rests on it.
/// The host's own Quit entry (`NATIVE_ITEM`) is no retail item: choosing it sends NOTHING to the game, it takes the focus and a
/// confirm asks the question (`title_quit`); choosing a retail row gives the focus back.
fn select(st: &mut State, i: usize, confirm: bool, now: u64) {
	let Some((page, target)) = st.built.as_ref().map(|b| (b.page, b.target(i))) else { return };
	if title_quit::ask() {
		return; // the question is up: nothing behind it can be chosen
	}
	if let Some((NATIVE_ITEM, _)) = target {
		st.local = Some((page, i, now));
		title_quit::set_focus(true);
		if confirm {
			title_quit::set_ask(true);
		}
		return;
	}
	let (Some((item, row)), Some(m)) = (target, &st.snap) else { return };
	if let Some(c) = resolve(m, Intent::Select { item, row, confirm }) {
		st.local = Some((page, i, now));
		title_quit::set_focus(false);
		submit(c);
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

/// The Quit question is up and a pad bit comes in: Circle = Yes, Triangle = No, everything else (Cross too) does nothing. True = the
/// question took the press (it is the host's own box, so the game gets no pulse; it is answered by the bit, not by the Action).
fn quit_answer(bit: u16) -> bool {
	if !title_quit::ask() {
		return false;
	}
	match bit {
		CIRCLE => title_quit::answer(true),
		TRIANGLE => title_quit::answer(false),
		_ => {}
	}
	true
}

fn act(st: &mut State, a: Action, now: u64) {
	if title_quit::ask() {
		// The Quit question: Enter = Yes, Esc (and a right click) = No; the pad's Circle / Triangle come by `pad_press`.
		match a {
			Action::Confirm => title_quit::answer(true),
			Action::Back => title_quit::answer(false),
			_ => {}
		}
		return;
	}
	let Some(b) = &st.built else { return };
	if !b.buttons.is_empty() {
		// The message box: Confirm answers its first button (Yes / Continue), Back its last (No).
		let bit = match a {
			Action::Confirm | Action::Peek => b.buttons.first(),
			Action::Back => b.buttons.last(),
			_ => None,
		};
		if let Some(&bit) = bit {
			submit(Command::Press(bit));
		}
		return;
	}
	// New / Load Game: the slot cards move the game's own cursor with pulses (title_slots.rs).
	if let Some(bit) = Some(b.page).filter(|&p| super::title_slots::is_cards(p)).and_then(|_| super::title_slots::action_bit(a)) {
		st.aim = None; // the keys take over from a click's walk
		submit(Command::Press(bit));
		return;
	}
	let n = b.rows();
	let cur = current(st, now);
	match a {
		Action::Up if n > 0 => step(st, -1, now),
		Action::Down if n > 0 => step(st, 1, now),
		Action::Confirm | Action::Peek => select(st, cur, true, now),
		Action::Back => back(st),
		_ => {}
	}
}

/// One winit key. True = the page took the press; a release always goes on to the game so no key sticks, and the Sound page's
/// Left / Right stay the game's (they move the bars).
pub fn key(code: KeyCode, pressed: bool, repeat: bool) -> bool {
	let mut g = lock();
	let Some(st) = g.as_mut().filter(|s| s.built.is_some()) else { return false }; // never take a key for a page that was not drawn
	let Some(k) = map_key(code) else { return false };
	if BOX.load(Ordering::Relaxed) {
		return false;
	}
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
	let now = now_ms();
	super::look::note_keyboard();
	st.rep.press(code as u32, a, now);
	act(st, a, now);
	true
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

fn flat(h: PageHit) -> Option<usize> {
	match h {
		PageHit::Menu(i) | PageHit::Item(i) => Some(i),
		_ => None,
	}
}

/// A mouse event in window pixels while the page is up: a hover selects (instantly), a left click confirms, a right click goes
/// back. Always taken (the retail menu behind must not also see it).
pub fn mouse(ev: MouseEvent) -> bool {
	if up() && matches!(ev, MouseEvent::Down { .. } | MouseEvent::Wheel { .. }) {
		super::look::note_keyboard(); // the mouse is the keyboard side of the hint bar (a plain move is judged by the window filter)
	}
	let mut g = lock();
	let Some(st) = g.as_mut().filter(|s| s.built.is_some()) else { return false }; // no page drawn yet = nothing to click
	let now = now_ms();
	match ev {
		MouseEvent::Move { x, y } => {
			if let Some(i) = flat(hit_at(st, x, y)).filter(|&i| i != current(st, now)) {
				select(st, i, false, now);
			}
		}
		MouseEvent::Down { button: MouseButton::Left, x, y } => match hit_at(st, x, y) {
			PageHit::Button(i) => {
				if let Some(&bit) = st.built.as_ref().and_then(|b| b.buttons.get(i)) {
					if !quit_answer(bit) {
						submit(Command::Press(bit));
					}
				}
			}
			PageHit::Item(i) if st.built.as_ref().is_some_and(|b| super::title_slots::is_cards(b.page)) => {
				// A slot card: steps the game's cursor to it, or takes it when it is the selected one; while the steps of an earlier
				// click are on their way it counts from where they go (title_slots.rs `click`, R4-2).
				let page = st.built.as_ref().map(|b| b.page);
				let aim = st.aim.filter(|&(p, _, t)| Some(p) == page && now.saturating_sub(t) < super::title_slots::AIM_MS).map(|a| a.1);
				let bits = super::title_slots::click(i, st.sel, aim);
				if bits.first().is_some_and(|&b| b != crate::retail::mouse_menu::CROSS) {
					st.aim = page.map(|p| (p, i, now));
				}
				let quiet = super::title_slots::quiet_steps(&bits);
				for (n, bit) in bits.into_iter().enumerate() {
					if n < quiet {
						crate::retail::menu_model::submit_quiet(bit); // the walk is silent, the last step plays the one move sound
					} else {
						submit(Command::Press(bit));
					}
				}
			}
			PageHit::Hint(HintKind::Change) => act(st, Action::Confirm, now),
			PageHit::Hint(HintKind::Back) => act(st, Action::Back, now),
			h => {
				if let Some(i) = flat(h) {
					select(st, i, true, now);
				}
			}
		},
		MouseEvent::Down { button: MouseButton::Right, .. } => act(st, Action::Back, now),
		_ => {}
	}
	true
}

/// The window lost the focus: forget the held keys and buttons.
pub fn lose_focus() {
	if let Some(st) = lock().as_mut() {
		st.rep.clear();
	}
}

/// One new pad press on the page of the last frame. A question takes only its own buttons (retail's Overwrite box answers Circle =
/// Yes and Triangle = No; Cross does nothing); on a page Triangle goes back (as Circle does) and Cross confirms.
fn pad_press(st: &mut State, bit: u16, a: Action, now: u64) {
	if quit_answer(bit) {
		return;
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

/// Display frame, first step: the pad's new presses and the repeat clock act on the page of the last frame.
pub fn begin(pad: Option<PadSample>) {
	let mut g = lock();
	let Some(st) = g.as_mut() else { return };
	let now = now_ms();
	let mask = pad.map_or(0, pad_mask);
	if BOX.load(Ordering::Relaxed) {
		// A memory card box (front mode 4) belongs to the game: `filter` hands it the real pad, so the page queues nothing of
		// its own (it would answer the box a second time). The presses stay noted, so none acts when the box closes.
		if mask & !st.pad_prev != 0 {
			super::look::note_pad();
		}
		st.rep.clear();
		(st.pad_prev, st.primed) = (mask, true);
		return;
	}
	for a in st.rep.poll(now) {
		act(st, a, now);
	}
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
		pad_press(st, bit, a, now);
	}
}

/// Display frame, last step: keeps this frame's page, its layout and the menu it was built from; returns the draw list.
pub fn end(built: Built, snap: Option<&MenuSnap>, sel: usize, win: (u32, u32), boxed: bool) -> Vec<Prim> {
	SOUND.store(built.page == Page::Sound, Ordering::Relaxed);
	WATCH.stamp(now_ms());
	BOX.store(boxed, Ordering::Relaxed);
	if built.page != Page::Root {
		title_quit::clear(); // the Quit entry lives on the root page only
	}
	let (layout, prims) = page_draw::frame(&built.screen, &FontMeasure);
	cmd::stamp(snap); // the pulses queued until the next frame are made for this page
	if let Some(st) = lock().as_mut() {
		(st.layout, st.snap, st.sel, st.win) = (Some(layout), snap.cloned(), sel, win);
		st.built = Some(built);
		st.last = prims.clone();
	}
	prims
}

/// The draw list of the last frame.
pub fn last() -> Vec<Prim> {
	lock().as_ref().map(|s| s.last.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests;
