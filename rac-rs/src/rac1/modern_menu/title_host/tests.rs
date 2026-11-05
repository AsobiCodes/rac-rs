//! Tests of the title host: the focus moves in the same call as a key, a hover selects at once and a click confirms, the worker's
//! pad filter, and the memory card message box (front mode 4) that keeps the keys and the pad for the game. The statics (`UP`,
//! `SOUND`, `BOX`, `ST`), the command queues (`menu_model::cmd`), the Quit entry's state (`title_quit`) and the program's quit request
//! are shared: every test takes `cmd::TEST_LOCK` first (`world`).
use super::*;
use crate::modern_menu::pause_tests::{snap, ADDR};
use crate::modern_menu::title::{build, DialogFacts, Facts};
use crate::retail::menu_model::cmd::{self, TEST_LOCK};
use crate::retail::mouse_instant::{Cmd, Request};
use crate::retail::mouse_menu::{CIRCLE, CROSS, TRIANGLE};
use crate::retail::title_quit::{self, NATIVE_ITEM};
use rac_engine::ui::input::GlyphSet;
use std::sync::MutexGuard;

const WIN: (u32, u32) = (1280, 720);

/// The request waiting for the worker, taken.
fn request() -> Option<Request> {
	match cmd::take_request() {
		Some(Cmd::Submit(r)) => Some(r),
		_ => None,
	}
}

/// The pad pulses queued so far (each pulse is one non-zero value); the queue is empty afterwards. The worker's part: the settled
/// menu is the page the pulses were made for.
fn pulses() -> Vec<u16> {
	let ready = MenuSnap { id: cmd::stamped().unwrap_or(8), state: 2, ..Default::default() };
	(0..80).map(|_| cmd::pad_step(Some(&ready), 0)).filter(|&b| b != 0).collect()
}

fn world() -> MutexGuard<'static, ()> {
	let g = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	UP.store(false, Ordering::Relaxed);
	SOUND.store(false, Ordering::Relaxed);
	BOX.store(false, Ordering::Relaxed);
	*lock() = None;
	let _ = (request(), pulses());
	cmd::stamp(None);
	title_quit::clear();
	let _ = rac_engine::quit::take();
	g
}

/// A front page the title cannot map keeps the last picture for a page change, then the page gives up (it was a soft lock: the pad
/// stayed neutral and the keys went to a stale snapshot); a mapped page, or a page that is not up, never gives up.
#[test]
fn an_unmapped_front_page_is_given_up_after_a_while() {
	let _w = world();
	assert!(!gives_up_at(true, 10_000_000), "the page is not up: nothing to give up");
	set_up(true);
	assert!(!gives_up_at(true, 1_000), "a page change in flight keeps the picture");
	assert!(!gives_up_at(true, 1_000 + GIVE_UP_MS), "exactly the grace");
	assert!(gives_up_at(true, 1_001 + GIVE_UP_MS), "an unmapped page that stays is given up");
	assert!(!gives_up_at(false, 5_000 + GIVE_UP_MS), "a mapped page resets the clock");
	assert!(!gives_up_at(true, 6_000 + GIVE_UP_MS), "and the next unmapped page gets its own grace");
	assert!(gives_up_at(true, 6_001 + 2 * GIVE_UP_MS));
	set_up(false);
	assert!(!gives_up_at(true, 99_000_000), "the page went down: the state is gone");
}

fn rows() -> Vec<(String, bool)> {
	["Pick a", "Pick b", "Pick c", "Pick d"].iter().map(|s| (s.to_string(), true)).collect()
}

fn facts<'a>(page: Page, rows: &'a [(String, bool)], sel: usize, dialog: Option<DialogFacts>) -> Facts<'a> {
	Facts {
		page,
		rows,
		header: "Options".into(),
		root: vec!["New Game".into(), "Load Game".into(), "Options".into()],
		slots: &[],
		empty: "Empty".into(),
		no_save: "Continue without saving".into(),
		sel,
		targets: (0..rows.len() as u32).map(|i| (ADDR, i)).collect(),
		dialog,
		sound: None,
		lang: 0,
		device: Device::Keyboard,
		glyphs: GlyphSet::PlayStation,
	}
}

/// One display frame's page: `end` with the page built from `f` (the state of the host stays).
fn show(f: &Facts, sel: usize, boxed: bool) {
	end(build(f), Some(&snap(8, &[(0x4f00, true); 4], sel as u32)), sel, WIN, boxed);
}

/// The page comes up the way `front_modern` does: `UP`, then one `end` with the page built from `f`.
fn up_with(f: &Facts, sel: usize, boxed: bool) {
	*lock() = Some(State { primed: true, ..State::default() });
	UP.store(true, Ordering::Relaxed);
	show(f, sel, boxed);
}

/// One display frame's pad (`front_modern` calls `begin` once per frame): the new presses act on the page of the last frame.
fn pad(buttons: u16) {
	begin(Some(PadSample { buttons, lx: 0x80, ly: 0x80 }));
}

fn options(sel: usize) {
	let r = rows();
	up_with(&facts(Page::Options, &r, sel, None), sel, false);
}

fn center(i: usize) -> (f32, f32) {
	let g = lock();
	let r = g.as_ref().unwrap().layout.as_ref().unwrap().panel.as_ref().unwrap().items[i].rect;
	(r.cx(), r.cy())
}

fn asked(r: Option<Request>) -> Option<(u32, bool)> {
	r.map(|r| (r.sub, r.confirm))
}

/// Load Game as the game builds it: retail's slot list has no rows the snapshot can address, so the cards have no target (the RAM dump
/// of page 0x1f, 2026-11-02); the keys, the pad and the mouse move the game's own cursor with pulses instead (title_slots.rs).
#[test]
fn the_slot_cards_move_the_games_cursor_with_pulses() {
	use crate::retail::menu_model::view::SlotView;
	use crate::retail::mouse_menu::{DOWN, UP};
	let _w = world();
	let slots: Vec<Option<SlotView>> = (0..5).map(|i| Some(SlotView { level: i, name: "Kerwan", bolts: 10, hours: 0, minutes: 39, date: "01/01/26".into() })).collect();
	let r: Vec<(String, bool)> = Vec::new();
	let mut f = facts(Page::LoadGame, &r, 1, None);
	(f.slots, f.targets) = (&slots, Vec::new());
	up_with(&f, 1, false);
	assert!(key(KeyCode::ArrowDown, true, false), "the page takes the press");
	assert!(key(KeyCode::ArrowUp, true, false));
	assert!(key(KeyCode::Enter, true, false));
	assert!(request().is_none(), "no Select request: the list has no rows to write");
	assert_eq!(pulses(), vec![DOWN, UP, CROSS], "the game's cursor steps, Enter takes the card");
	pad(CROSS);
	pad(0);
	assert_eq!(pulses(), vec![CROSS], "the pad's Cross takes the card");
	let (c3, c1) = (center(3), center(1));
	mouse(MouseEvent::Down { button: MouseButton::Left, x: c3.0, y: c3.1 });
	assert_eq!(pulses(), vec![DOWN, DOWN], "a click on another card steps the cursor to it");
	mouse(MouseEvent::Down { button: MouseButton::Left, x: c3.0, y: c3.1 });
	assert_eq!(pulses(), Vec::<u16>::new(), "R4-2: a second click before the cursor arrived steps nothing again (no overshoot)");
	mouse(MouseEvent::Down { button: MouseButton::Left, x: c1.0, y: c1.1 });
	assert_eq!(pulses(), vec![UP, UP], "R4-2: card 1 while the steps go to card 3: back from 3, never a Cross that takes card 3");
	up_with(&f, 3, false);
	mouse(MouseEvent::Down { button: MouseButton::Left, x: c3.0, y: c3.1 });
	assert_eq!(pulses(), vec![CROSS], "a click on the selected card takes it");
	assert!(key(KeyCode::Escape, true, false));
	assert_eq!(pulses().len(), 1, "Esc still goes back (the page's own back button)");
	set_up(false);
}

#[test]
fn the_focus_moves_in_the_same_call_as_the_key() {
	let _w = world();
	options(0);
	assert!(key(KeyCode::ArrowDown, true, false), "the page takes the press");
	assert_eq!(display_sel(Page::Options, 0), 1, "retail still says row 0: the page already shows row 1");
	assert_eq!(asked(request()), Some((1, false)));
	assert!(key(KeyCode::ArrowUp, true, false));
	assert!(key(KeyCode::ArrowUp, true, false));
	assert_eq!(display_sel(Page::Options, 0), 3, "up from the first row wraps to the last");
	assert_eq!(display_sel(Page::Sound, 0), 0, "another page keeps retail's row");
	assert!(key(KeyCode::Enter, true, false));
	assert_eq!(asked(request()).map(|r| r.1), Some(true), "Enter confirms the focused row");
}

#[test]
fn a_key_release_and_a_repeat_do_not_act_twice() {
	let _w = world();
	options(0);
	assert!(key(KeyCode::ArrowDown, true, false));
	let _ = request();
	assert!(key(KeyCode::ArrowDown, true, true), "a repeat is taken but does nothing");
	assert!(request().is_none());
	assert!(!key(KeyCode::ArrowDown, false, false), "a release goes on to the game");
	assert!(!key(KeyCode::KeyJ, true, false), "an unmapped key is not taken");
}

#[test]
fn hover_selects_at_once_and_a_click_confirms() {
	let _w = world();
	options(0);
	let (c2, c3) = (center(2), center(3));
	assert!(mouse(MouseEvent::Move { x: c2.0, y: c2.1 }), "always taken");
	assert_eq!(asked(request()), Some((2, false)), "a hover selects at once, without confirming");
	assert_eq!(display_sel(Page::Options, 0), 2);
	mouse(MouseEvent::Move { x: c2.0, y: c2.1 });
	assert!(request().is_none(), "hovering the focused row again asks nothing");
	assert!(mouse(MouseEvent::Down { button: MouseButton::Left, x: c3.0, y: c3.1 }));
	assert_eq!(asked(request()), Some((3, true)), "a click selects and confirms");
	mouse(MouseEvent::Down { button: MouseButton::Left, x: 2.0, y: 2.0 });
	assert!(request().is_none(), "a click on nothing does nothing");
	mouse(MouseEvent::Down { button: MouseButton::Right, x: 2.0, y: 2.0 });
	assert_eq!(pulses().len(), 1, "a right click goes back");
}

#[test]
fn the_mouse_and_keys_are_not_taken_while_the_page_is_down() {
	let _w = world();
	assert!(!mouse(MouseEvent::Move { x: 5.0, y: 5.0 }));
	assert!(!key(KeyCode::ArrowDown, true, false));
	assert!(!up());
}

#[test]
fn the_worker_gives_the_game_a_neutral_pad_while_the_page_is_up() {
	let _w = world();
	let (mut b, mut s) = (0xffffu16, [0x10u8, 0x20, 0x30, 0x40]);
	filter(&mut b, &mut s, true);
	assert_eq!((b, s), (0xffff, [0x10, 0x20, 0x30, 0x40]), "page down: the game's own pad");
	options(0);
	filter(&mut b, &mut s, true);
	assert_eq!((b, s), (0, [0x80; 4]), "page up: neutral, Left / Right too");
}

#[test]
fn the_sound_page_keeps_left_right_and_the_stick_x_for_the_game() {
	let _w = world();
	let r = rows();
	up_with(&facts(Page::Sound, &r, 0, None), 0, false);
	assert!(SOUND.load(Ordering::Relaxed));
	let (mut b, mut s) = (LEFT | RIGHT | CROSS, [0x10u8, 0x20, 0x30, 0x40]);
	filter(&mut b, &mut s, true);
	assert_eq!((b, s), (LEFT | RIGHT, [0x80, 0x80, 0x30, 0x80]));
	assert!(!key(KeyCode::ArrowLeft, true, false) && !key(KeyCode::ArrowRight, true, false), "the bars step per held field in retail");
	assert!(key(KeyCode::ArrowDown, true, false), "Up / Down are the page's");
}

#[test]
fn a_memory_card_box_keeps_the_keys_and_the_pad_for_the_game() {
	let _w = world();
	let r = rows();
	let boxed = DialogFacts { lines: vec!["No memory card".into()], buttons: vec![(CROSS, "Continue".into())] };
	up_with(&facts(Page::Options, &r, 0, Some(boxed)), 0, true);
	assert!(BOX.load(Ordering::Relaxed));
	let (mut b, mut s) = (CROSS | CIRCLE, [0x10u8, 0x20, 0x30, 0x40]);
	filter(&mut b, &mut s, true);
	assert_eq!((b, s), (CROSS | CIRCLE, [0x10, 0x20, 0x30, 0x40]), "mode 4: the pad reaches the game untouched");
	assert!(!key(KeyCode::Enter, true, false) && !key(KeyCode::Escape, true, false), "mode 4: the keys reach the game");
	assert!(request().is_none() && pulses().is_empty());
	options(0);
	assert!(!BOX.load(Ordering::Relaxed), "the next page without a box takes the input again");
	assert!(key(KeyCode::Enter, true, false));
}

#[test]
fn a_dialog_answers_with_its_own_buttons_only() {
	let _w = world();
	let r = rows();
	let d = DialogFacts { lines: vec!["Overwrite?".into()], buttons: vec![(CIRCLE, "Yes".into()), (TRIANGLE, "No".into())] };
	up_with(&facts(Page::Options, &r, 0, Some(d)), 0, false);
	assert!(key(KeyCode::Enter, true, false));
	assert!(key(KeyCode::Escape, true, false));
	assert_eq!(pulses(), [CIRCLE, TRIANGLE], "Enter = the first button, Esc = the last");
	assert!(key(KeyCode::ArrowDown, true, false));
	assert!(request().is_none(), "no row is chosen under a question");
}

#[test]
fn page_down_forgets_the_state() {
	let _w = world();
	options(0);
	assert!(up());
	set_up(false);
	assert!(!up() && lock().is_none() && last().is_empty());
	assert_eq!(display_sel(Page::Options, 3), 3, "no state: retail's row");
}

#[test]
fn the_overwrite_box_answers_only_circle_and_triangle_on_the_pad() {
	let _w = world();
	let r = rows();
	let d = DialogFacts { lines: vec!["Overwrite?".into()], buttons: vec![(CIRCLE, "Yes".into()), (TRIANGLE, "No".into())] };
	up_with(&facts(Page::Options, &r, 0, Some(d)), 0, false);
	pad(CROSS);
	assert!(pulses().is_empty() && request().is_none(), "Cross does nothing on the box (retail); it must not answer Yes");
	for b in [0, CIRCLE, 0, TRIANGLE, 0, CROSS | 0x80] {
		pad(b);
	}
	assert_eq!(pulses(), [CIRCLE, TRIANGLE], "Circle = Yes, Triangle = No, nothing else answers");
}

#[test]
fn pad_triangle_goes_back_and_never_confirms_a_row() {
	let _w = world();
	options(1);
	pad(TRIANGLE);
	assert!(request().is_none(), "Triangle must not pick the focused row (it loaded a save on Load Game)");
	assert_eq!(pulses().len(), 1, "Triangle goes back");
	pad(0);
	pad(CIRCLE);
	assert!(request().is_none());
	assert_eq!(pulses().len(), 1, "Circle goes back too");
	pad(0);
	pad(CROSS);
	assert_eq!(asked(request()), Some((1, true)), "Cross confirms the focused row");
}

#[test]
fn a_card_box_gets_no_pulse_from_the_page_and_its_presses_do_not_act_afterwards() {
	let _w = world();
	let r = rows();
	let d = DialogFacts { lines: vec!["Card".into()], buttons: vec![(CIRCLE, "Yes".into()), (TRIANGLE, "No".into())] };
	up_with(&facts(Page::Options, &r, 0, Some(d)), 0, true);
	for b in [CROSS, 0, CIRCLE, 0, TRIANGLE] {
		pad(b);
	}
	assert!(pulses().is_empty() && request().is_none(), "the game answers the box from the real pad; the page queues nothing");
	// The press that closed the box is still down when the page behind it shows: it must not act on it.
	show(&facts(Page::Options, &r, 0, None), 0, false);
	pad(TRIANGLE);
	assert!(pulses().is_empty() && request().is_none());
	pad(0);
	pad(TRIANGLE);
	assert_eq!(pulses().len(), 1, "a fresh press acts again");
}

#[test]
fn the_arrows_skip_rows_the_game_refuses_and_a_hover_on_one_takes_no_focus() {
	let _w = world();
	let r = rows();
	let f = facts(Page::Options, &r, 0, None);
	up_with(&f, 0, false);
	let rows = [(0x4f00, true), (0x4f00, false), (0x4f00, true), (0x4f00, true)];
	end(build(&f), Some(&snap(8, &rows, 0)), 0, WIN, false);
	assert!(key(KeyCode::ArrowDown, true, false));
	assert_eq!(asked(request()), Some((2, false)), "Down passes the disabled row 1");
	assert_eq!(display_sel(Page::Options, 0), 2);
	assert!(key(KeyCode::ArrowUp, true, false));
	assert_eq!(asked(request()), Some((0, false)), "Up passes it too");
	let c1 = center(1);
	mouse(MouseEvent::Move { x: c1.0, y: c1.1 });
	assert!(request().is_none(), "a hover on a refused row asks nothing");
	assert_eq!(display_sel(Page::Options, 0), 0, "and does not move the highlight onto it");
	mouse(MouseEvent::Down { button: MouseButton::Left, x: c1.0, y: c1.1 });
	assert!(request().is_none() && display_sel(Page::Options, 0) == 0, "a click on it does nothing");
}

// ---- The host's own fourth root entry, "Quit to Desktop" (`title_quit`) ----

/// One display frame of the root page the way `front_modern` makes it: retail's cursor is on panel `rc`, the row shown is
/// `display_sel`'s, and the Quit question is the dialog while it is up.
fn root_frame(rc: usize) {
	let m = title_quit::test_root(rc);
	let mut f = facts(Page::Root, &[], 0, None);
	f.root.push("Quit to Desktop".into());
	f.targets = crate::modern_menu::title::flatten(&m);
	f.sel = display_sel(Page::Root, rc);
	if title_quit::ask() {
		f.dialog = Some(DialogFacts { lines: vec!["Quit?".into()], buttons: vec![(CIRCLE, "Yes".into()), (TRIANGLE, "No".into())] });
	}
	end(build(&f), Some(&m), rc, WIN, false);
}

/// The root page comes up with retail's cursor on panel `rc`.
fn root_up(rc: usize) {
	*lock() = Some(State { primed: true, ..State::default() });
	UP.store(true, Ordering::Relaxed);
	root_frame(rc);
}

/// The question is up, as a Down to the entry and Enter would leave it (the frame is shown).
fn question_up() {
	root_up(2);
	title_quit::set_focus(true);
	title_quit::set_ask(true);
	root_frame(2);
	assert!(lock().as_ref().unwrap().built.as_ref().unwrap().screen.dialog.is_some(), "set up: the question is on the page");
}

fn menu_center(i: usize) -> (f32, f32) {
	let g = lock();
	let r = g.as_ref().unwrap().layout.as_ref().unwrap().menu[i].rect;
	(r.cx(), r.cy())
}

fn button_center(i: usize) -> (f32, f32) {
	let g = lock();
	let r = g.as_ref().unwrap().layout.as_ref().unwrap().dialog.as_ref().unwrap().buttons[i].rect;
	(r.cx(), r.cy())
}

fn item_asked(r: Option<Request>) -> Option<(u32, bool)> {
	r.map(|r| (r.item, r.confirm))
}

/// Nothing reached the game: no request, no pulse.
fn game_untouched() -> bool {
	request().is_none() && pulses().is_empty()
}

fn row_addr(i: usize) -> u32 {
	title_quit::test_root(0).items[i].addr
}

#[test]
fn the_root_has_four_rows_and_the_arrows_walk_into_the_quit_entry_without_a_request() {
	let _w = world();
	root_up(0);
	assert_eq!(lock().as_ref().unwrap().built.as_ref().unwrap().rows(), 4);
	assert!(key(KeyCode::ArrowDown, true, false));
	assert_eq!(item_asked(request()), Some((row_addr(1), false)));
	assert!(key(KeyCode::ArrowDown, true, false));
	assert_eq!(item_asked(request()), Some((row_addr(2), false)));
	assert!(!title_quit::focus());
	assert!(key(KeyCode::ArrowDown, true, false));
	assert!(game_untouched(), "the step onto the host's own entry sends nothing to the game");
	assert!(title_quit::focus() && !title_quit::ask());
	assert_eq!(display_sel(Page::Root, 2), title_quit::ROW, "the entry is the row shown, while retail's cursor rests on Options");
	root_frame(2);
	let b = lock().as_ref().unwrap().built.clone().unwrap();
	assert!(b.screen.menu[3].selected && !b.screen.menu[2].selected);
	assert_eq!(b.target(3), Some((NATIVE_ITEM, 0)));
	assert!(key(KeyCode::ArrowDown, true, false));
	assert_eq!(item_asked(request()), Some((row_addr(0), false)), "Down from the entry wraps to New Game, with a request");
	assert!(!title_quit::focus(), "a retail row takes the focus back");
	assert!(key(KeyCode::ArrowUp, true, false));
	assert!(game_untouched(), "Up from New Game wraps onto the entry: no request");
	assert!(title_quit::focus());
	assert!(key(KeyCode::ArrowUp, true, false));
	assert_eq!(item_asked(request()), Some((row_addr(2), false)), "Up from the entry goes back to Options");
	assert!(!title_quit::focus());
}

#[test]
fn enter_on_the_entry_asks_and_sends_nothing() {
	let _w = world();
	root_up(2);
	assert!(key(KeyCode::ArrowDown, true, false));
	assert!(key(KeyCode::Enter, true, false));
	assert!(title_quit::ask() && game_untouched(), "Enter on the entry opens the question, the game hears nothing");
	assert!(!rac_engine::quit::requested(), "and it did not also answer it");
	root_frame(2);
	let b = lock().as_ref().unwrap().built.clone().unwrap();
	assert_eq!(b.buttons, [CIRCLE, TRIANGLE]);
	assert!(b.menu.iter().all(|m| m.is_none()), "no row can be chosen under the question");
}

#[test]
fn the_pad_cross_opens_the_question_on_the_entry() {
	let _w = world();
	root_up(2);
	title_quit::set_focus(true);
	root_frame(2);
	pad(CROSS);
	assert!(title_quit::ask() && game_untouched());
}

#[test]
fn the_question_is_answered_by_circle_triangle_enter_and_esc_only() {
	let _w = world();
	question_up();
	pad(CROSS);
	assert!(title_quit::ask() && !rac_engine::quit::requested() && game_untouched(), "Cross does nothing");
	pad(0);
	pad(CIRCLE);
	assert!(rac_engine::quit::take(), "Circle = Yes");
	assert!(title_quit::ask(), "the question stays while the window closes");
	assert!(game_untouched(), "and no pulse went to the game");
	question_up();
	pad(0);
	pad(TRIANGLE);
	assert!(!title_quit::ask() && !rac_engine::quit::requested(), "Triangle = No");
	assert!(title_quit::focus(), "the entry stays focused");
	assert!(game_untouched());
	question_up();
	assert!(key(KeyCode::Enter, true, false));
	assert!(rac_engine::quit::take(), "Enter = Yes");
	assert!(game_untouched());
	question_up();
	assert!(key(KeyCode::NumpadEnter, true, false));
	assert!(rac_engine::quit::take(), "the keypad's Enter too");
	question_up();
	assert!(key(KeyCode::Escape, true, false));
	assert!(!title_quit::ask() && !rac_engine::quit::requested(), "Esc = No");
	assert!(game_untouched());
	question_up();
	for code in [KeyCode::Space, KeyCode::Backspace, KeyCode::ArrowDown, KeyCode::ArrowUp, KeyCode::ArrowLeft] {
		key(code, true, false);
		key(code, false, false);
	}
	assert!(title_quit::ask() && !rac_engine::quit::requested() && game_untouched(), "Space, Backspace and the arrows do nothing");
	for b in [0, CROSS | 0x80, 0, 0x1000, 0, 0x4000] {
		pad(b);
	}
	assert!(title_quit::ask() && !rac_engine::quit::requested() && game_untouched(), "no other pad button does anything");
}

#[test]
fn a_click_on_the_questions_buttons_answers_it() {
	let _w = world();
	question_up();
	let (yes, no) = (button_center(0), button_center(1));
	assert!(mouse(MouseEvent::Down { button: MouseButton::Left, x: 2.0, y: 2.0 }));
	assert!(title_quit::ask() && !rac_engine::quit::requested(), "a click beside the box does nothing");
	let (c3, c0) = (menu_center(3), menu_center(0));
	mouse(MouseEvent::Down { button: MouseButton::Left, x: c3.0, y: c3.1 });
	mouse(MouseEvent::Down { button: MouseButton::Left, x: c0.0, y: c0.1 });
	mouse(MouseEvent::Move { x: c0.0, y: c0.1 });
	assert!(title_quit::ask() && game_untouched(), "the menu behind the question is dead");
	mouse(MouseEvent::Down { button: MouseButton::Left, x: no.0, y: no.1 });
	assert!(!title_quit::ask() && !rac_engine::quit::requested() && game_untouched(), "the second button (Triangle) is No");
	question_up();
	mouse(MouseEvent::Down { button: MouseButton::Left, x: yes.0, y: yes.1 });
	assert!(rac_engine::quit::take() && game_untouched(), "the first button (Circle) is Yes");
	question_up();
	mouse(MouseEvent::Down { button: MouseButton::Right, x: 2.0, y: 2.0 });
	assert!(!title_quit::ask() && !rac_engine::quit::requested() && game_untouched(), "a right click is No");
}

#[test]
fn hover_focuses_the_entry_and_a_click_asks() {
	let _w = world();
	root_up(2);
	let c3 = menu_center(3);
	assert!(mouse(MouseEvent::Move { x: c3.0, y: c3.1 }));
	assert!(title_quit::focus() && !title_quit::ask() && game_untouched(), "a hover focuses it and asks the game nothing");
	assert_eq!(display_sel(Page::Root, 2), title_quit::ROW);
	let c1 = menu_center(1);
	mouse(MouseEvent::Move { x: c1.0, y: c1.1 });
	assert_eq!(item_asked(request()), Some((row_addr(1), false)), "a hover on a retail button takes the focus back");
	assert!(!title_quit::focus());
	assert!(mouse(MouseEvent::Down { button: MouseButton::Left, x: c3.0, y: c3.1 }));
	assert!(title_quit::ask() && title_quit::focus() && game_untouched(), "a click on the fourth button asks");
}

#[test]
fn the_entry_belongs_to_the_root_page_and_to_one_style() {
	let _w = world();
	root_up(2);
	title_quit::set_focus(true);
	title_quit::set_ask(true);
	options(0);
	assert!(!title_quit::focus() && !title_quit::ask(), "another page: the entry and its question are gone");
	root_up(2);
	title_quit::set_focus(true);
	set_up(false);
	assert!(!title_quit::focus(), "the page went down");
	// Frames of the Classic style (the page stays down) must not wipe the Classic state.
	title_quit::set_focus(true);
	title_quit::set_ask(true);
	set_up(false);
	set_up(false);
	assert!(title_quit::focus() && title_quit::ask(), "a frame of the other style leaves the state alone");
	set_up(true);
	assert!(!title_quit::focus() && !title_quit::ask(), "a style switch starts fresh");
	set_up(false);
}
