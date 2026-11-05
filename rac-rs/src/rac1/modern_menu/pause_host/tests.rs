//! Tests of the pause host: the message box answers only its own buttons, Circle / Triangle go back, the arrows skip refused rows,
//! the worker's pad filter, the mouse (hover, click, wheel, volume bar). The statics (`UP`, `SOUND`, `ST`) and the command queues
//! (`menu_model::cmd`) are shared: every test takes `cmd::TEST_LOCK` first (`world`) and leaves the queues empty.
use super::*;
use crate::modern_menu::pause::build;
use crate::modern_menu::pause_tests::{facts, snap, snap_values, text_of, values_of, view_of, ADDR};
use crate::retail::menu_keys;
use crate::retail::menu_model::cmd::{self, TEST_LOCK};
use crate::retail::menu_model::extra::{Extra, Sound};
use crate::retail::menu_model::view::PageView;
use crate::retail::mouse_instant::{Cmd, Request};
use crate::retail::mouse_menu::{Item, Kind, CIRCLE, CROSS, DOWN};
use std::sync::MutexGuard;

const T: u64 = 10_000;
const SQUARE: u16 = 0x80;
const L1: u16 = 0x04;
const R1: u16 = 0x08;
const PAD_UP: u16 = 0x1000;
const START_BTN: u16 = 0x800;

/// The pad pulses queued so far, in order (each pulse is one non-zero value); the queue is empty afterwards.
fn pulses() -> Vec<u16> {
	let ready = MenuSnap { id: cmd::stamped().unwrap_or(8), state: 2, ..Default::default() };
	(0..80).map(|_| cmd::pad_step(Some(&ready), 0)).filter(|&b| b != 0).collect()
}

/// The request waiting for the worker, taken.
fn request() -> Option<Request> {
	match cmd::take_request() {
		Some(Cmd::Submit(r)) => Some(r),
		_ => None,
	}
}

fn world() -> MutexGuard<'static, ()> {
	let g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	UP.store(false, Ordering::Relaxed);
	SOUND.store(false, Ordering::Relaxed);
	HIDDEN.store(0, Ordering::Relaxed);
	EAT.store(0, Ordering::Relaxed);
	*lock() = None;
	let _ = (request(), pulses());
	cmd::stamp(None);
	g
}

fn state(v: PageView, ex: &Extra, m: MenuSnap) -> State {
	let built = build(&facts(&v, ex, &text_of)).expect("the page builds");
	let layout = page_draw::frame(&built.screen, &FontMeasure).0;
	State { built: Some(built), layout: Some(layout), snap: Some(m), win: (1280, 720), primed: true, ..State::default() }
}

/// Four rows, the second refused by the game.
const OPT: [(u16, bool); 4] = [(0x4f00, true), (0x4f01, false), (0x4f02, true), (0x4f03, true)];
const YES_NO: [(u16, bool); 2] = [(0x524f, true), (0x524b, true)];
const ROOT: [(u16, bool); 7] = [(0x4ee2, true), (0x4ee3, true), (0x4ee4, true), (0x4ee5, true), (0x4ee6, true), (0x4ee7, true), (0x4fc2, true)];

fn options() -> State {
	state(view_of(Page::Options, &OPT, 0), &Extra::default(), snap(8, &OPT, 0))
}

fn quit() -> State {
	state(view_of(Page::QuitGame, &YES_NO, 0), &Extra::default(), snap(0xd, &YES_NO, 0))
}

fn root() -> State {
	state(view_of(Page::Root, &ROOT, 5), &Extra::default(), snap(2, &ROOT, 5))
}

fn sound() -> State {
	let ex = Extra { sound: Some(Sound { sfx: 0x400, music: 0x100, stereo: true }), ..Default::default() };
	state(values_of(Page::Sound, 3, 0), &ex, snap_values(0x20, 3, 0))
}

fn install(st: State) {
	*lock() = Some(st);
	UP.store(true, Ordering::Relaxed);
}

/// A press and release of pad `bit` on the page of `st`.
fn tap(st: &mut State, bit: u16) {
	pad_core(st, bit, T);
	pad_core(st, 0, T);
}

#[test]
fn a_pad_press_that_is_not_a_dialog_button_is_ignored() {
	let _w = world();
	for page in [quit(), state(view_of(Page::Overwrite, &YES_NO, 0), &Extra { slots: vec![None; 5], ..Default::default() }, snap(0x1a, &YES_NO, 0))] {
		let mut st = page;
		for bit in [CROSS, SQUARE, PAD_UP, DOWN, L1, R1, LEFT, RIGHT] {
			tap(&mut st, bit);
		}
		assert_eq!(pulses(), Vec::<u16>::new(), "Cross never confirms the question");
		assert!(request().is_none(), "no row is chosen under a question");
	}
}

#[test]
fn circle_and_triangle_answer_the_question() {
	let _w = world();
	let mut st = quit();
	tap(&mut st, CIRCLE);
	tap(&mut st, TRIANGLE);
	assert_eq!(pulses(), [CIRCLE, TRIANGLE], "Circle = Yes, Triangle = No");
}

#[test]
fn keyboard_enter_is_yes_and_esc_is_no() {
	let _w = world();
	let mut st = quit();
	act(&mut st, Action::Confirm, T);
	act(&mut st, Action::Back, T);
	assert_eq!(pulses(), [CIRCLE, TRIANGLE]);
	for a in [Action::Peek, Action::Up, Action::Down, Action::Left, Action::TabNext, Action::Reset] {
		act(&mut st, a, T);
	}
	assert_eq!(pulses(), Vec::<u16>::new(), "nothing else answers it");
	assert!(request().is_none());
}

#[test]
fn the_press_that_brought_the_page_up_does_not_act() {
	let _w = world();
	let mut st = State { built: options().built, snap: options().snap, ..State::default() };
	pad_core(&mut st, CROSS, T); // still held from the press that opened the menu
	pad_core(&mut st, CROSS, T);
	assert!(request().is_none() && st.primed);
	pad_core(&mut st, 0, T);
	pad_core(&mut st, CROSS, T);
	assert!(request().is_some_and(|r| r.confirm), "a new press acts");
}

#[test]
fn circle_and_triangle_go_back_and_cross_confirms() {
	let _w = world();
	let mut st = options();
	let back = menu_keys::screen(st.snap.as_ref().unwrap()).back;
	assert_ne!(back, 0);
	tap(&mut st, CIRCLE);
	tap(&mut st, TRIANGLE);
	assert_eq!(pulses(), [back, back]);
	tap(&mut st, CROSS);
	assert_eq!(request(), Some(Request { page_id: 8, item: ADDR, sub: 0, confirm: true, slide: None }));
}

#[test]
fn the_arrows_skip_rows_the_game_refuses_and_wrap() {
	let _w = world();
	let mut st = options();
	let mut seen = Vec::new();
	for (a, t) in [(Action::Down, T), (Action::Down, T + 10), (Action::Down, T + 20), (Action::Up, T + 30), (Action::Up, T + 40)] {
		act(&mut st, a, t);
		seen.push(request().map(|r| (r.sub, r.confirm)).unwrap());
	}
	// 0 -> 2 (row 1 refused) -> 3 -> 0 (wrap) -> 3 (wrap) -> 2.
	assert_eq!(seen, [(2, false), (3, false), (0, false), (3, false), (2, false)]);
}

#[test]
fn the_focus_moves_in_the_same_call_as_the_key() {
	let _w = world();
	let mut st = options();
	assert_eq!(current(&st, T), 0);
	act(&mut st, Action::Down, T);
	assert_eq!(current(&st, T), 2, "the very frame of the key, before retail's cursor follows");
	assert_eq!(current(&st, T + 400), 0, "after the grace the game's cursor shows again");
	let _ = request();
}

#[test]
fn the_root_steps_tabs_sideways_and_other_pages_do_not() {
	let _w = world();
	let mut st = root();
	act(&mut st, Action::Left, T);
	assert_eq!(request().map(|r| r.sub), Some(4));
	act(&mut st, Action::TabNext, T + 10);
	assert_eq!(request().map(|r| r.sub), Some(5));
	act(&mut st, Action::Right, T + 20);
	assert_eq!(request().map(|r| r.sub), Some(6));
	act(&mut st, Action::Right, T + 30);
	assert!(request().is_none() && pulses().is_empty(), "the Map tab (last): the focus only, Enter opens the map");
	assert_eq!(current(&st, T + 30), 7);
	act(&mut st, Action::Right, T + 40);
	assert_eq!(request().map(|r| r.sub), Some(0), "wraps");
	let mut st = options();
	for a in [Action::Left, Action::Right, Action::TabPrev, Action::TabNext, Action::Reset] {
		act(&mut st, a, T);
	}
	assert!(request().is_none() && pulses().is_empty(), "a list page does not step sideways (the pad keeps retail's rules there)");
}

#[test]
fn the_worker_filter_keeps_start_select_and_r3_only() {
	let _w = world();
	let mut b = CROSS | PAD_UP | 0x800 | 0x100 | 0x400 | LEFT;
	let mut s = [10, 20, 30, 40];
	filter(&mut b, &mut s);
	assert_eq!((b, s), (CROSS | PAD_UP | 0x800 | 0x100 | 0x400 | LEFT, [10, 20, 30, 40]), "page down: the game's own pad");
	UP.store(true, Ordering::Relaxed);
	(b, s) = (CROSS | PAD_UP | 0x800 | 0x100 | 0x400 | LEFT, [10, 20, 30, 40]);
	filter(&mut b, &mut s);
	assert_eq!((b, s), (START_BTN | 0x100 | 0x400, [0x80; 4]));
	SOUND.store(true, Ordering::Relaxed);
	(b, s) = (CROSS | RIGHT | 0x800, [10, 20, 30, 40]);
	filter(&mut b, &mut s);
	assert_eq!((b, s), (RIGHT | 0x800, [0x80, 0x80, 30, 0x80]), "the Sound page keeps Left / Right and the stick's x");
}

#[test]
fn a_repeated_picture_ends_the_sound_passthrough() {
	let _w = world();
	install(sound());
	UP.store(true, Ordering::Relaxed);
	SOUND.store(true, Ordering::Relaxed);
	let (mut b, mut s) = (RIGHT | CROSS, [10, 20, 30, 40]);
	filter(&mut b, &mut s);
	assert_eq!((b, s), (RIGHT, [0x80, 0x80, 30, 0x80]), "on the Sound page the game keeps Left / Right and the stick's x");
	let _ = repeat();
	(b, s) = (RIGHT | CROSS, [10, 20, 30, 40]);
	filter(&mut b, &mut s);
	assert_eq!((b, s), (0, [0x80; 4]), "the page flying in is not the Sound page: the pad is neutral again");
	assert!(!SOUND.load(Ordering::Relaxed));
}

#[test]
fn keys_are_taken_by_the_page_and_the_sound_arrows_stay_the_games() {
	let _w = world();
	assert!(!key(KeyCode::Enter, true, false), "page down: not taken");
	install(quit());
	assert!(key(KeyCode::Enter, true, false), "taken");
	assert!(!key(KeyCode::Enter, false, false), "a release goes on to the game");
	assert!(key(KeyCode::Escape, true, false));
	assert!(key(KeyCode::Space, true, false), "taken, but it answers nothing");
	assert!(key(KeyCode::Enter, true, true), "an OS repeat is taken and dropped");
	assert!(!key(KeyCode::F11, true, false));
	assert_eq!(pulses(), [CIRCLE, TRIANGLE]);
	assert_eq!(device(), Device::Keyboard);
	install(sound());
	SOUND.store(true, Ordering::Relaxed);
	assert!(!key(KeyCode::ArrowLeft, true, false) && !key(KeyCode::KeyD, true, false), "the game steps the bars");
	assert!(key(KeyCode::ArrowDown, true, false));
	assert_eq!(request().map(|r| r.sub), Some(1));
}

fn center(st: &State, i: usize) -> (f32, f32) {
	let r = st.layout.as_ref().unwrap().panel.as_ref().unwrap().items[i].rect;
	(r.cx(), r.cy())
}

#[test]
fn hover_click_right_click_and_wheel() {
	let _w = world();
	let st = options();
	let (c0, c1, c2, c3) = (center(&st, 0), center(&st, 1), center(&st, 2), center(&st, 3));
	install(st);
	assert!(mouse(MouseEvent::Move { x: c2.0, y: c2.1 }));
	assert_eq!(request(), Some(Request { page_id: 8, item: ADDR, sub: 2, confirm: false, slide: None }), "a hover selects at once");
	assert!(mouse(MouseEvent::Move { x: c1.0, y: c1.1 }));
	assert!(request().is_none(), "a refused row is not chosen");
	assert!(mouse(MouseEvent::Down { button: MouseButton::Left, x: c3.0, y: c3.1 }));
	assert_eq!(request().map(|r| (r.sub, r.confirm)), Some((3, true)), "a click confirms");
	mouse(MouseEvent::Down { button: MouseButton::Left, x: c1.0, y: c1.1 });
	assert!(request().is_none());
	mouse(MouseEvent::Down { button: MouseButton::Right, x: c0.0, y: c0.1 });
	assert_eq!(pulses().len(), 1, "a right click goes back");
	mouse(MouseEvent::Wheel { lines: 1.0 });
	let up = request().map(|r| r.sub);
	mouse(MouseEvent::Wheel { lines: -1.0 });
	let down = request().map(|r| r.sub);
	assert_eq!((up, down), (Some(2), Some(3)), "the wheel walks the list (from row 3: up to 2, then the focus is 2: down to 3)");
	assert!(!mouse_with_page_down(), "page down: not taken");
}

fn mouse_with_page_down() -> bool {
	*lock() = None;
	UP.store(false, Ordering::Relaxed);
	mouse(MouseEvent::Wheel { lines: 1.0 })
}

#[test]
fn a_dialog_button_click_presses_its_bit_and_a_click_elsewhere_does_nothing() {
	let _w = world();
	let st = quit();
	let btn = st.layout.as_ref().unwrap().dialog.as_ref().unwrap().buttons[1].rect;
	install(st);
	mouse(MouseEvent::Down { button: MouseButton::Left, x: btn.cx(), y: btn.cy() });
	mouse(MouseEvent::Down { button: MouseButton::Left, x: 5.0, y: 5.0 });
	assert_eq!(pulses(), [TRIANGLE]);
	assert!(request().is_none());
}

#[test]
fn a_click_or_drag_on_a_volume_bar_sets_it() {
	let _w = world();
	let st = sound();
	let bar = st.layout.as_ref().unwrap().panel.as_ref().unwrap().items[0].bar.unwrap();
	let y = bar.y + bar.h * 0.5;
	install(st);
	mouse(MouseEvent::Down { button: MouseButton::Left, x: bar.x + bar.w * 0.5, y });
	let r = request().unwrap();
	assert_eq!((r.item, r.sub, r.confirm, r.slide), (ADDR, 0, false, Some(Slide::To(512))));
	mouse(MouseEvent::Move { x: bar.x + bar.w * 0.25, y: y + 200.0 });
	assert_eq!(request().and_then(|r| r.slide), Some(Slide::To(256)), "the drag follows the pointer even off the bar");
	mouse(MouseEvent::Move { x: bar.x - 90.0, y });
	assert_eq!(request().and_then(|r| r.slide), Some(Slide::To(0)), "clamped to the track");
	mouse(MouseEvent::Up { button: MouseButton::Left });
	mouse(MouseEvent::Move { x: bar.x + bar.w * 0.9, y });
	assert!(request().and_then(|r| r.slide).is_none(), "released: no more sliding");
	assert_eq!(lock().as_ref().unwrap().drag, None);
}

#[test]
fn a_click_on_a_sound_label_leaves_the_volume_alone() {
	let _w = world();
	let st = sound();
	let item = st.layout.as_ref().unwrap().panel.as_ref().unwrap().items[1].rect;
	install(st);
	mouse(MouseEvent::Down { button: MouseButton::Left, x: item.x + 10.0, y: item.cy() });
	let r = request().unwrap();
	assert_eq!((r.sub, r.confirm, r.slide), (1, true, None));
}

#[test]
fn losing_the_focus_ends_a_drag() {
	let _w = world();
	let st = sound();
	let bar = st.layout.as_ref().unwrap().panel.as_ref().unwrap().items[0].bar.unwrap();
	install(st);
	mouse(MouseEvent::Down { button: MouseButton::Left, x: bar.cx(), y: bar.cy() });
	assert!(lock().as_ref().unwrap().drag.is_some());
	lose_focus();
	assert_eq!(lock().as_ref().unwrap().drag, None);
	let _ = request();
}

#[test]
fn the_page_is_only_up_between_set_up_calls() {
	let _w = world();
	assert!(!up() && goodies() == false && local(Page::Root).is_none());
	install(root());
	assert!(up());
	assert!(lock().as_mut().is_some_and(|s| {
		s.goodies = true;
		s.local = Some((Page::Root, 3, 7));
		true
	}));
	assert!(goodies());
	assert_eq!(local(Page::Root), Some((3, 7)));
	assert_eq!(local(Page::Options), None, "a focus belongs to its page");
	assert_eq!(last(), Vec::<Prim>::new());
}

// ---- The icon-grid pages (Weapons, Gadgets, Items; `grid.rs`) ----

use crate::modern_menu::pause_grid::{preview_src, uv_of, Knobs};
use crate::modern_menu::pause_grid_tests::{extra as grid_extra, gadgets, items, view as grid_view, weapons, GRID};
use crate::retail::menu_model::extra::GridFacts;
use rac_engine::ui::kit::shell::SHELL_SCRIM;

/// The menu snapshot of a grid page: one Grid item per fact set (cursor `cursors[i]`), the first one current.
fn grid_snap(id: u32, grids: &[GridFacts], cursors: &[u32]) -> MenuSnap {
	let item = |i: usize, g: &GridFacts, c: u32| Item {
		addr: g.addr,
		slot: i + 2,
		rect: [0, 0, 100, 100],
		kind: Kind::Grid { rows: g.rows, cols: g.cols, cursor: c },
		texts: Vec::new(),
		planet_texts: Vec::new(),
	};
	let items = grids.iter().zip(cursors).enumerate().map(|(i, (g, &c))| item(i, g, c)).collect();
	MenuSnap { page: 0x1d0000, id, state: id, cur: grids[0].addr, items, front: false }
}

fn grid_page(page: Page, id: u32, grids: Vec<GridFacts>, cursors: &[u32], hint: Option<&str>) -> State {
	let v = grid_view(page, &grids, cursors, 0, hint);
	state(v, &grid_extra(grids.clone()), grid_snap(id, &grids, cursors))
}

/// Weapons: 3 x 5 cells, retail cursor on `cursor`, an Equip hint (so the accent button shows).
fn weapons_page(cursor: u32) -> State {
	grid_page(Page::Weapons, 3, vec![weapons()], &[cursor], Some("Equip"))
}

fn gadgets_page() -> State {
	grid_page(Page::Gadgets, 4, gadgets(), &[0; 4], None)
}

/// The middle of flat cell `k` (the design space is the window at 1280 x 720).
fn gcenter(st: &State, k: usize) -> (f32, f32) {
	let r = st.built.as_ref().unwrap().grid.as_ref().unwrap().cells[k];
	(r.cx(), r.cy())
}

fn drain() {
	let _ = (request(), pulses());
}

#[test]
fn an_arrow_inside_a_grid_moves_the_focus_locally_and_selects_the_cell() {
	let _w = world();
	let mut st = weapons_page(6);
	assert!(st.built.as_ref().is_some_and(|b| b.grid.is_some()), "the fixture is a grid page");
	act(&mut st, Action::Right, T);
	assert_eq!(st.local.map(|l| l.1), Some(7), "row 1, column 1 -> column 2");
	assert_eq!(request(), Some(Request { page_id: 3, item: GRID, sub: 7, confirm: false, slide: None }));
	act(&mut st, Action::Down, T);
	assert_eq!(request().map(|r| r.sub), Some(12), "the next row, same column");
	act(&mut st, Action::Left, T);
	act(&mut st, Action::Up, T);
	assert_eq!(request().map(|r| r.sub), Some(6), "the last request wins");
	assert_eq!(pulses(), Vec::<u16>::new(), "nothing reaches the D-pad while the focus stays inside the grid");
	drain();
}

#[test]
fn at_the_edge_of_a_grid_the_game_gets_the_dpad_and_the_local_focus_is_dropped() {
	let _w = world();
	for (cursor, a, bit) in [(0, Action::Up, PAD_UP), (0, Action::Left, LEFT), (4, Action::Right, RIGHT), (10, Action::Down, DOWN), (14, Action::Right, RIGHT)] {
		let mut st = weapons_page(cursor);
		act(&mut st, a, T);
		assert!(st.local.is_none(), "cell {cursor} {a:?}: the game's cursor shows");
		assert!(request().is_none(), "cell {cursor} {a:?}: no local request at the edge");
		assert_eq!(pulses(), [bit], "cell {cursor} {a:?}: the real D-pad press");
	}
	let mut st = weapons_page(3);
	act(&mut st, Action::Right, T);
	assert_eq!(st.local.map(|l| l.1), Some(4));
	drain();
	act(&mut st, Action::Right, T);
	assert_eq!(st.local, None, "the local focus is dropped at the edge");
	assert_eq!(pulses(), [RIGHT]);
	drain();
}

#[test]
fn the_gadget_groups_are_walked_inside_and_left_through_the_game() {
	let _w = world();
	let mut st = gadgets_page();
	act(&mut st, Action::Down, T);
	assert_eq!(request().map(|r| (r.item, r.sub)), Some((GRID, 3)), "Hand has two rows of three");
	act(&mut st, Action::Down, T);
	assert!(st.local.is_none() && request().is_none(), "the bottom row of Hand: the game jumps to the Back group");
	assert_eq!(pulses(), [DOWN]);
	let head = gcenter(&st, 9);
	install(st);
	mouse(MouseEvent::Move { x: head.0, y: head.1 });
	assert_eq!(request().map(|r| (r.page_id, r.item, r.sub, r.confirm)), Some((4, GRID + 0x200, 0, false)), "a hover over the Head group selects its first cell");
	drain();
}

#[test]
fn hover_click_button_wheel_and_right_click_on_a_grid() {
	let _w = world();
	let st = weapons_page(0);
	let (c3, c7) = (gcenter(&st, 3), gcenter(&st, 7));
	let btn = st.built.as_ref().unwrap().grid.as_ref().unwrap().button.expect("the Equip hint gives the accent button");
	install(st);
	assert!(mouse(MouseEvent::Move { x: c7.0, y: c7.1 }));
	assert_eq!(request(), Some(Request { page_id: 3, item: GRID, sub: 7, confirm: false, slide: None }), "a hover selects at once");
	mouse(MouseEvent::Move { x: c7.0, y: c7.1 });
	assert!(request().is_none(), "the cell is already the focus");
	mouse(MouseEvent::Down { button: MouseButton::Left, x: c3.0, y: c3.1 });
	assert_eq!(request().map(|r| (r.sub, r.confirm)), Some((3, true)), "a click confirms the cell");
	mouse(MouseEvent::Down { button: MouseButton::Left, x: btn.cx(), y: btn.cy() });
	assert_eq!(request().map(|r| (r.sub, r.confirm)), Some((3, true)), "the action button confirms the focused cell");
	assert!(mouse(MouseEvent::Down { button: MouseButton::Left, x: 5.0, y: 5.0 }), "taken");
	assert!(request().is_none(), "a click outside every cell does nothing");
	mouse(MouseEvent::Wheel { lines: -1.0 });
	assert_eq!(request().map(|r| r.sub), Some(4), "the wheel down steps to the next cell");
	mouse(MouseEvent::Wheel { lines: 1.0 });
	assert_eq!(request().map(|r| r.sub), Some(3), "and back up");
	mouse(MouseEvent::Down { button: MouseButton::Right, x: c3.0, y: c3.1 });
	assert_eq!(pulses().len(), 1, "a right click goes back");
	drain();
}

#[test]
fn the_wheel_wraps_over_the_flat_cell_list_and_crosses_the_gadget_groups() {
	let _w = world();
	install(weapons_page(0));
	mouse(MouseEvent::Wheel { lines: 1.0 });
	assert_eq!(request().map(|r| r.sub), Some(14), "up from the first cell wraps to the last");
	install(gadgets_page());
	for _ in 0..6 {
		mouse(MouseEvent::Wheel { lines: -1.0 });
	}
	assert_eq!(request().map(|r| (r.item, r.sub)), Some((GRID + 0x100, 0)), "six steps down from Hand's first cell: the first Back cell");
	drain();
}

#[test]
fn keys_and_the_pad_drive_a_grid_page_and_the_pad_strictness_is_unchanged() {
	let _w = world();
	let mut st = weapons_page(0);
	tap(&mut st, DOWN);
	assert_eq!(request().map(|r| r.sub), Some(5), "a pad Down steps inside the grid");
	tap(&mut st, CROSS);
	assert_eq!(request().map(|r| (r.sub, r.confirm)), Some((5, true)), "Cross confirms the focused cell");
	tap(&mut st, L1);
	tap(&mut st, R1);
	tap(&mut st, SQUARE);
	assert!(request().is_none() && pulses().is_empty(), "L1, R1 and Square do nothing here, as on the other pages");
	let back = menu_keys::screen(st.snap.as_ref().unwrap()).back;
	tap(&mut st, TRIANGLE);
	tap(&mut st, CIRCLE);
	assert_eq!(pulses(), [back, back], "Triangle and Circle both go back");
	install(weapons_page(0));
	assert!(key(KeyCode::ArrowRight, true, false));
	assert_eq!(request().map(|r| r.sub), Some(1));
	assert!(key(KeyCode::Enter, true, false));
	assert_eq!(request().map(|r| (r.sub, r.confirm)), Some((1, true)));
	let (mut b, mut s) = (0xffff_u16, [0, 1, 2, 3]);
	filter(&mut b, &mut s);
	assert_eq!((b, s), (PASS, [0x80; 4]), "the worker keeps only Start, Select and R3");
	drain();
}

#[test]
fn an_items_cell_click_selects_and_confirms_with_the_grid_item() {
	let _w = world();
	let st = grid_page(Page::Items, 6, vec![items()], &[0], None);
	let c4 = gcenter(&st, 4);
	assert!(st.built.as_ref().unwrap().grid.as_ref().unwrap().button.is_none(), "no Cross hint, no action button");
	install(st);
	mouse(MouseEvent::Down { button: MouseButton::Left, x: c4.0, y: c4.1 });
	assert_eq!(request(), Some(Request { page_id: 6, item: GRID, sub: 4, confirm: true, slide: None }));
	drain();
}

#[test]
fn end_cuts_the_hole_into_the_scrim_and_appends_the_grid() {
	let _w = world();
	let st = weapons_page(0);
	let (built, snap) = (st.built.clone().unwrap(), st.snap.clone().unwrap());
	install(st);
	let prims = end(built.clone(), Some(&snap), (1280, 720));
	let g = built.grid.as_ref().unwrap();
	assert!(prims.ends_with(&g.prims), "the grid panels come last");
	let scrims: Vec<_> = prims.iter().filter_map(|p| if let Prim::RRect { rect, fill, .. } = p { (*fill == SHELL_SCRIM).then_some(*rect) } else { None }).collect();
	assert!(scrims.len() >= 3 && scrims.iter().all(|r| r.h < 10_000.0), "the one giant scrim is gone, strips are left");
	let hole = g.hole;
	let overlaps = |r: &rac_engine::ui::prim::Rect| r.x < hole.right() && hole.x < r.right() && r.y < hole.bottom() && hole.y < r.bottom();
	assert!(!scrims.iter().any(overlaps), "no scrim strip covers the preview");
	assert_eq!(last(), prims, "`last` repeats the list");
	drain();
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
	a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 0.01)
}

#[test]
fn place_preview_does_nothing_while_the_page_is_down_or_is_a_list() {
	let _w = world();
	let mut hud = Vec::new();
	place_preview(&mut hud, (1280, 720));
	*lock() = Some(weapons_page(0));
	place_preview(&mut hud, (1280, 720));
	assert!(hud.is_empty(), "down: nothing, even with a state kept");
	install(options());
	place_preview(&mut hud, (1280, 720));
	assert!(hud.is_empty(), "a list page has no crop");
	drain();
}

#[test]
fn place_preview_pushes_one_crop_quad_with_an_aspect_kept_rect() {
	let _w = world();
	let st = weapons_page(0);
	let g = st.built.as_ref().unwrap().grid.clone().unwrap();
	install(st);
	let mut hud = Vec::new();
	place_preview(&mut hud, (1280, 720));
	assert_eq!(hud.len(), 1, "exactly one quad");
	let q = &hud[0];
	assert_eq!(q.tex, crate::viewer::RETAIL_OV_TEX);
	let src = preview_src(Page::Weapons, &Knobs::DEFAULT);
	assert_eq!(q.uv, uv_of(src), "the crop is Ratchet's panel of the overlay picture");
	assert!(close(q.rect, [g.hole.x, g.hole.y, g.hole.w, g.hole.h]), "1280 x 720 is the design space");
	let (w, h) = (src[2], src[3]);
	assert!((q.rect[2] / q.rect[3] - w / h * 7.0 / 6.0).abs() < 0.01, "the crop shows with its display aspect, not stretched");
	assert!(q.rect[2] <= 536.5 && q.rect[3] <= 220.5, "inside the detail panel's viewport");
	let mut big = Vec::new();
	place_preview(&mut big, (2560, 1440));
	assert!(close(big[0].rect, q.rect.map(|v| v * 2.0)), "twice the window: twice the rect");
	let mut wide = Vec::new();
	place_preview(&mut wide, (3440, 1440));
	assert!(close(wide[0].rect, [q.rect[0] * 2.0 + 440.0, q.rect[1] * 2.0, q.rect[2] * 2.0, q.rect[3] * 2.0]), "ultrawide: the design is centred, the crop keeps its shape");
	drain();
}

// ---- the tabs from a child page (child_tabs.rs) ----

/// The middle of tab `i` of the page's tab bar (design space = the 1280 x 720 window).
fn tab_center(st: &State, i: usize) -> (f32, f32) {
	let r = st.layout.as_ref().unwrap().menu[i].rect;
	(r.cx(), r.cy())
}

#[test]
fn a_click_on_another_tab_of_a_child_page_goes_back_then_selects_that_tab() {
	let _w = world();
	let st = weapons_page(0);
	let (gadgets, weapons) = (tab_center(&st, 1), tab_center(&st, 0));
	install(st);
	assert!(mouse(MouseEvent::Down { button: MouseButton::Left, x: weapons.0, y: weapons.1 }));
	assert_eq!((pulses(), request().is_none()), (vec![], true), "the tab of the page you are on does nothing");
	assert!(lock().as_ref().unwrap().pending.is_none());
	mouse(MouseEvent::Down { button: MouseButton::Left, x: gadgets.0, y: gadgets.1 });
	assert_eq!(pulses().len(), 1, "one Back leaves the Weapons page");
	let pending = lock().as_ref().unwrap().pending;
	assert!(pending.is_some() && request().is_none(), "the Select waits for the root page");
	begin(None);
	assert_eq!(pulses().len(), 0, "the same page is not left twice within the retry time");
	// The game is on the root page now: the next frame selects (and opens) the Gadgets row.
	let mut root = root();
	root.pending = pending;
	install(root);
	begin(None);
	assert_eq!(request().map(|r| (r.sub, r.confirm)), Some((1, true)), "Select Gadgets, confirmed");
	assert!(lock().as_ref().unwrap().pending.is_none(), "done");
}

#[test]
fn a_tab_switch_from_a_child_page_is_one_sound_the_back_is_hushed() {
	let _w = world();
	crate::retail::snd::hush(0);
	let st = weapons_page(0);
	let gadgets = tab_center(&st, 1);
	install(st);
	mouse(MouseEvent::Down { button: MouseButton::Left, x: gadgets.0, y: gadgets.1 });
	assert_eq!(pulses().len(), 1, "one Back");
	assert!(crate::retail::mouse_instant::skip_armed() > 0, "the tab click skips the page flights");
	assert_eq!(crate::retail::snd::hushed(), 2, "the Back's close sound is hushed: only the open sound of the new tab is heard");
	crate::retail::snd::hush(0);
	crate::retail::mouse_instant::skip_page_changes(0);
	// a plain Back (Esc on the Weapons page) keeps its sound
	let st = weapons_page(0);
	install(st);
	act(lock().as_mut().unwrap(), Action::Back, T);
	assert_eq!(pulses().len(), 1);
	assert_eq!(crate::retail::snd::hushed(), 0, "keyboard / pad Back is unchanged");
	assert_eq!(crate::retail::mouse_instant::skip_armed(), 0, "and keeps retail's page flight");
	// the Select that opens the new tab is a request, never hushed
	let mut root = root();
	root.pending = Some(child_tabs::Pending::test(1, T));
	install(root);
	begin(None);
	let r = request();
	assert_eq!(r.map(|r| (r.sub, r.confirm)), Some((1, true)));
	assert_eq!(crate::retail::snd::hushed(), 0);
	assert_eq!(crate::retail::mouse_instant::quieted(), r, "its cursor move is silent: only the new tab's open sound is heard");
}

/// A landed page the builder cannot read yet keeps the last picture for HOLD_MS while the page is up, then the classic page shows; a
/// built frame starts the clock over; nothing is held while the page is down.
#[test]
fn a_landed_page_without_its_facts_keeps_the_last_picture_briefly() {
	let _w = world();
	install(root());
	assert!(hold_unbuilt(1000) && hold_unbuilt(1000 + HOLD_MS));
	assert!(!hold_unbuilt(1001 + HOLD_MS), "then the classic page");
	lock().as_mut().unwrap().unbuilt = None; // what a built frame (`end`) does
	assert!(hold_unbuilt(5000));
	UP.store(false, Ordering::Relaxed);
	lock().as_mut().unwrap().unbuilt = None;
	assert!(!hold_unbuilt(6000), "the page is down: classic at once");
}

#[test]
fn a_tab_click_on_a_sub_page_of_that_tab_stops_at_the_tabs_main_page() {
	let _w = world();
	let mut sound = sound();
	let options_tab = tab_center(&sound, 5);
	sound.pending = None;
	install(sound);
	mouse(MouseEvent::Down { button: MouseButton::Left, x: options_tab.0, y: options_tab.1 });
	assert_eq!(pulses().len(), 1, "one Back leaves the Sound page");
	let pending = lock().as_ref().unwrap().pending;
	assert!(pending.is_some());
	let mut main = options();
	main.pending = pending;
	install(main);
	begin(None);
	assert!(lock().as_ref().unwrap().pending.is_none() && request().is_none() && pulses().is_empty(), "the Options page is the tab's own page: nothing more");
}

#[test]
fn a_message_box_blocks_the_tabs_of_a_child_page() {
	let _w = world();
	let st = quit();
	let tab = tab_center(&st, 0);
	install(st);
	mouse(MouseEvent::Down { button: MouseButton::Left, x: tab.0, y: tab.1 });
	assert!(lock().as_ref().unwrap().pending.is_none() && pulses().is_empty(), "a question holds the tabs");
}

#[test]
fn a_tab_switch_gives_up_after_three_seconds_and_losing_the_focus_ends_it() {
	let _w = world();
	let mut st = options();
	child_tabs::start(&mut st, 0, T);
	assert!(st.pending.is_some());
	child_tabs::poll(&mut st, T + 3_100);
	assert!(st.pending.is_none(), "a page that never settles ends the request");
	child_tabs::start(&mut st, 1, T);
	install(st);
	lose_focus();
	assert!(lock().as_ref().unwrap().pending.is_none());
	drain();
}

/// R3-1: a pad button held while the page closes (Circle / Triangle = Back on the root) does not reach gameplay as a fresh press.
#[test]
fn buttons_held_when_the_page_goes_down_stay_hidden_until_released() {
	let _w = world();
	set_up(false);
	set_up(true);
	assert!(up());
	let circle = CIRCLE;
	let (mut b, mut s) = (circle | START_BTN, [0x80u8; 4]);
	filter(&mut b, &mut s);
	assert_eq!(b, START_BTN, "the page hides Circle from the game");
	set_up(false);
	let mut b = circle | SQUARE;
	filter(&mut b, &mut s);
	assert_eq!(b, SQUARE, "Circle still held after the close is eaten; a button pressed later is not");
	let mut b = 0;
	filter(&mut b, &mut s);
	let mut b = circle;
	filter(&mut b, &mut s);
	assert_eq!(b, circle, "after the release a new press reaches the game");
	lose_focus();
}

/// R3-4: the hint bar's Back works on a grid page too (the grid used to spend every click).
#[test]
fn the_hint_bars_back_goes_back_on_a_grid_page() {
	let _w = world();
	let st = weapons_page(0);
	let back = st.layout.as_ref().unwrap().hints.iter().find(|h| h.kind == HintKind::Back).expect("the grid page shows Back").rect;
	install(st);
	assert!(mouse(MouseEvent::Down { button: MouseButton::Left, x: back.cx(), y: back.cy() }));
	assert!(request().is_none(), "no cell is picked");
	assert_eq!(pulses().len(), 1, "Back goes back, as Esc does");
	drain();
}

/// R3-5: the window's Shift keys are tracked for Shift+Tab (the pause and title hosts pass it to `key_action`).
#[test]
fn shift_is_tracked_from_the_window_keys_and_dropped_with_the_focus() {
	let _w = world();
	crate::modern_menu::host::key(KeyCode::ShiftLeft, true, false);
	assert!(crate::modern_menu::host::shift());
	assert_eq!(map_key(KeyCode::Tab).and_then(|k| key_action(k, crate::modern_menu::host::shift())), Some(Action::TabPrev));
	crate::modern_menu::host::key(KeyCode::ShiftLeft, false, false);
	assert!(!crate::modern_menu::host::shift());
	crate::modern_menu::host::key(KeyCode::ShiftRight, true, false);
	crate::modern_menu::host::lose_focus();
	assert!(!crate::modern_menu::host::shift(), "a focus loss forgets a held Shift");
}

// ---- the achievements page (ach.rs; review R22) ----

/// The achievements page (an empty list) over page `v`.
fn ach_page(v: PageView, m: MenuSnap) -> State {
	use crate::achievements::ListView;
	let a = crate::modern_menu::pause_ach::AchFacts { view: ListView { rows: Vec::new(), got: 0, all: 0, cheated: false }, focus: 0, first: 0 };
	let ex = Extra::default();
	let mut f = facts(&v, &ex, &text_of);
	f.ach = Some(&a);
	let built = build(&f).expect("the achievements page builds");
	let layout = page_draw::frame(&built.screen, &FontMeasure).0;
	State { built: Some(built), layout: Some(layout), snap: Some(m), win: (1280, 720), primed: true, ..State::default() }
}

/// m6: a tab click on the achievements page closes the list, waits while the list is the last frame's page, then is a tab click on
/// the page under it: the root selects the tab; the Options page's own tab stays.
#[test]
fn a_tab_click_on_the_achievements_page_closes_the_list_then_acts_on_the_page_under_it() {
	use crate::achievements as a;
	let _w = world();
	let st = ach_page(view_of(Page::Root, &ROOT, 5), snap(2, &ROOT, 5));
	let gadgets = tab_center(&st, 1);
	install(st);
	a::open();
	assert!(mouse(MouseEvent::Down { button: MouseButton::Left, x: gadgets.0, y: gadgets.1 }));
	assert!(!a::is_open(), "the click closes the list");
	begin(None);
	assert!(request().is_none() && pulses().is_empty(), "nothing while the list is the last frame's page");
	let pending = lock().as_ref().unwrap().pending;
	assert!(pending.is_some());
	let mut root = root();
	root.pending = pending;
	install(root);
	begin(None);
	assert_eq!(request().map(|r| (r.sub, r.confirm)), Some((1, true)), "the root selects Gadgets, confirmed");
	assert!(lock().as_ref().unwrap().pending.is_none());
	// Over the Options page: its own tab closes the list and nothing more.
	let st = ach_page(view_of(Page::Options, &OPT, 0), snap(8, &OPT, 0));
	let options_tab = tab_center(&st, 5);
	install(st);
	a::open();
	mouse(MouseEvent::Down { button: MouseButton::Left, x: options_tab.0, y: options_tab.1 });
	let pending = lock().as_ref().unwrap().pending;
	let mut main = options();
	main.pending = pending;
	install(main);
	begin(None);
	assert!(!a::is_open() && lock().as_ref().unwrap().pending.is_none() && request().is_none() && pulses().is_empty(), "the page's own tab");
}

/// M1: the list the achievements page showed closes with the menu (the closing hold, `close_ach`) and with the page (`set_up(false)`);
/// another page leaves an open list alone.
#[test]
fn the_achievements_list_closes_with_the_menu_and_the_page() {
	use crate::achievements as a;
	let _w = world();
	install(options());
	a::open();
	close_ach();
	assert!(a::is_open(), "not the achievements page: left alone");
	install(ach_page(view_of(Page::Options, &OPT, 0), snap(8, &OPT, 0)));
	close_ach();
	assert!(!a::is_open(), "the menu closing (pad Start) closes the list");
	assert!(up(), "the closing hold keeps the page up");
	a::open();
	set_up(false);
	assert!(!a::is_open() && !up(), "the page given back closes it too");
}
