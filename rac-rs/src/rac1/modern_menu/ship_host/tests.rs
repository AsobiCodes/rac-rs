//! Tests of the ship host (the modern Galactic Map's mouse): on the planet list (page 0xf) a hover is Select, a click is Select + Cross,
//! greyed planets are never a target and the wheel steps over them, a right click is the game's Triangle on both pages; on the details
//! page (0x10) the left click and the wheel do nothing; keys and the pad are never taken. The statics (`UP`, `ST`) and the command queues
//! (`menu_model::cmd`) are shared: every test takes `cmd::TEST_LOCK` first (`world`) and leaves the queues empty.
use super::*;
use crate::modern_menu::ship::{build, Facts, Row};
use crate::retail::menu_model::cmd::{self, TEST_LOCK};
use crate::retail::menu_model::extra::ShipFacts;
use crate::retail::mouse_instant::{Cmd, Request};
use crate::retail::mouse_menu::{Item, Kind, CROSS, TRIANGLE};
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::input::GlyphSet;
use std::sync::MutexGuard;

/// The list item's address in the fixtures (the target of every row request).
const ITEM: u32 = 0x1234;
/// Rows of the fixture that the game greys out (the 4th and 9th planet).
const GREY: [usize; 2] = [3, 8];

/// The pad pulses queued so far, in order (each pulse is one non-zero value); the queue is empty afterwards.
fn pulses() -> Vec<u16> {
	let ready = MenuSnap { id: cmd::stamped().unwrap_or(0), state: 2, ..Default::default() };
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
	*lock() = None;
	let _ = (request(), pulses());
	cmd::stamp(None);
	g
}

/// Eleven planets, `GREY` greyed, the cursor on `sel`.
fn listing(sel: usize) -> Listing {
	let row = |i: usize| {
		let on = !GREY.contains(&i);
		let (location, name) = if on { (format!("Place {i}"), format!("Planet {i}")) } else { (String::new(), "????".to_string()) };
		Row { planet: Some(i as u32 + 1), location, name, enabled: on }
	};
	Listing { item: ITEM, rows: (0..11).map(row).collect(), sel }
}

/// The menu as the mouse reads it: the list item with the same rows (greyed ones not selectable), page `id`, settled (`state` 2).
fn snap(id: u32, state: u32) -> MenuSnap {
	let rows = (0..11).map(|i| (100.0 + 20.0 * i as f32, !GREY.contains(&i))).collect();
	let kind = Kind::Label { rows, rowh: 20.0, cursor: 0 };
	let item = Item { addr: ITEM, slot: 0, rect: [20, 86, 144, 251], kind, texts: Vec::new(), planet_texts: Vec::new() };
	MenuSnap { page: 0x1b3d98, id, state, cur: ITEM, items: vec![item], front: false }
}

fn page(stage: Stage, sel: usize) -> Built {
	let (l, none, t) = (listing(sel), ShipFacts::default(), |_: u32| String::new());
	build(&Facts { list: &l, stage, focus: None, scroll: 0, here: 3, planets: &none, infobot: true, text: &t, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation }, &FontMeasure).expect("the page builds")
}

/// The page up on the 1280x720 window (design pixels = window pixels).
fn install(stage: Stage, sel: usize, snap: Option<MenuSnap>) -> Built {
	let b = page(stage, sel);
	*lock() = Some(State { built: Some(b.clone()), snap, win: (1280, 720), ..State::default() });
	UP.store(true, Ordering::Relaxed);
	b
}

fn centre(r: Rect) -> (f32, f32) {
	(r.cx(), r.cy())
}

fn row_at(b: &Built, i: usize) -> (f32, f32) {
	centre(b.rows.iter().find(|r| r.0 == i).expect("row is visible").1)
}

fn go(x: f32, y: f32) -> bool {
	mouse(MouseEvent::Move { x, y })
}

fn click_at((x, y): (f32, f32)) -> bool {
	mouse(MouseEvent::Down { button: MouseButton::Left, x, y })
}

fn right_click() -> bool {
	mouse(MouseEvent::Down { button: MouseButton::Right, x: 5.0, y: 5.0 })
}

fn wheel_down() -> bool {
	mouse(MouseEvent::Wheel { lines: -1.0 })
}

fn wheel_up() -> bool {
	mouse(MouseEvent::Wheel { lines: 1.0 })
}

/// The row request the host should send for planet row `sub` of the list on page 0xf.
fn req(sub: u32, confirm: bool) -> Option<Request> {
	Some(Request { page_id: 0xf, item: ITEM, sub, confirm, slide: None })
}

#[test]
fn a_hover_chooses_the_planet_and_a_click_chooses_and_confirms() {
	let _w = world();
	let b = install(Stage::List, 0, Some(snap(0xf, 2)));
	let (x, y) = row_at(&b, 1);
	assert!(go(x, y), "the page takes the mouse");
	assert_eq!(request(), req(1, false), "hover = Select, no Cross");
	assert_eq!(lock().as_ref().and_then(|s| s.local).map(|l| l.0), Some(1), "the focus moves at once");
	go(x + 1.0, y);
	assert_eq!(request(), None, "the same row again sends nothing");
	click_at((x, y));
	assert_eq!(request(), req(1, true), "a click on the chosen row = Select + Cross");
	click_at(row_at(&b, 2));
	assert_eq!(request(), req(2, true), "a click on another row chooses it and confirms in one go");
	// A marker on the painting answers like its row.
	let mark = centre(b.marks.iter().find(|m| m.0 == 5).expect("a marker for planet row 5").1);
	go(mark.0, mark.1);
	assert_eq!(request(), req(5, false), "hover on a marker");
	click_at(mark);
	assert_eq!(request(), req(5, true), "click on a marker");
	assert_eq!(pulses(), Vec::<u16>::new(), "no pad button for any of it");
}

#[test]
fn a_greyed_planet_is_never_a_target() {
	let _w = world();
	let b = install(Stage::List, 0, Some(snap(0xf, 2)));
	for g in GREY {
		assert!(!b.enabled[g] && b.rows.iter().chain(&b.marks).all(|r| r.0 != g), "row {g} has neither a rect nor a marker");
	}
	// Where the greyed row 3 is drawn (the slot between rows 2 and 4) nothing answers.
	let (a, c) = (row_at(&b, 2), row_at(&b, 4));
	let mid = (a.0, (a.1 + c.1) / 2.0);
	assert_eq!(b.hit(mid.0, mid.1), Hit::None);
	assert!(go(mid.0, mid.1) && click_at(mid), "still taken");
	assert_eq!((request(), pulses()), (None, vec![]));
	// The door refuses a greyed row by itself ...
	for g in GREY {
		select(lock().as_mut().expect("up"), g, true, now());
		assert_eq!(request(), None, "row {g}");
	}
	// ... and the game's own row table is the second lock if the page's flags were ever wrong.
	lock().as_mut().and_then(|s| s.built.as_mut()).expect("built").enabled[3] = true;
	select(lock().as_mut().expect("up"), 3, true, now());
	assert_eq!(request(), None, "resolve checks the game's row flags");
	select(lock().as_mut().expect("up"), 2, true, now());
	assert_eq!(request(), req(2, true), "an enabled row still goes through");
}

#[test]
fn the_wheel_steps_to_the_next_choosable_planet_and_skips_greyed_ones() {
	let _w = world();
	install(Stage::List, 2, Some(snap(0xf, 2)));
	assert!(wheel_down());
	assert_eq!(request(), req(4, false), "down from 2 steps over the greyed 3");
	wheel_down();
	assert_eq!(request(), req(5, false), "the page's own focus counts, retail's cursor is still on 2");
	wheel_up();
	assert_eq!(request(), req(4, false));
	wheel_up();
	assert_eq!(request(), req(2, false), "up from 4 steps over the greyed 3");
	install(Stage::List, 7, Some(snap(0xf, 2)));
	wheel_down();
	assert_eq!(request(), req(9, false), "down from 7 steps over the greyed 8");
	wheel_up();
	assert_eq!(request(), req(7, false), "and back");
	install(Stage::List, 10, Some(snap(0xf, 2)));
	wheel_down();
	assert_eq!(request(), None, "down at the last planet");
	wheel_up();
	assert_eq!(request(), req(9, false));
	install(Stage::List, 0, Some(snap(0xf, 2)));
	wheel_up();
	assert_eq!(request(), None, "up at the first planet");
	mouse(MouseEvent::Wheel { lines: 0.0 });
	assert_eq!((request(), pulses()), (None, vec![]));
}

#[test]
fn a_right_click_goes_back_on_both_pages() {
	for (stage, id) in [(Stage::List, 0xf), (Stage::Details, 0x10)] {
		let _w = world();
		install(stage, 0, Some(snap(id, 2)));
		assert!(right_click(), "page {id:#x} takes the mouse");
		assert_eq!(pulses(), [TRIANGLE], "a right click leaves, like Triangle (page {id:#x})");
		assert_eq!(request(), None);
	}
}

#[test]
fn on_the_details_page_the_left_click_and_the_wheel_do_nothing() {
	let _w = world();
	let list = page(Stage::List, 2);
	install(Stage::Details, 2, Some(snap(0x10, 2)));
	// Every spot that was a row or a marker on the list, plus the middle (the buttons are tested apart).
	let spots: Vec<(f32, f32)> = list.rows.iter().chain(&list.marks).map(|r| centre(r.1)).chain([(640.0, 360.0), (100.0, 150.0)]).collect();
	for p in spots {
		assert!(go(p.0, p.1) && click_at(p), "taken at {p:?}");
	}
	assert!(wheel_down() && wheel_up());
	assert_eq!((request(), pulses(), lock().as_ref().and_then(|s| s.local)), (None, vec![], None), "no row, no pad button, no focus");
}

#[test]
fn a_menu_the_game_is_animating_or_one_the_mouse_cannot_address_sends_nothing_but_is_still_taken() {
	let _w = world();
	let b = install(Stage::List, 0, Some(snap(0xf, 1))); // flying in
	let p = row_at(&b, 1);
	assert!(go(p.0, p.1) && click_at(p) && wheel_down());
	assert_eq!(request(), None, "a page that animates is not for the mouse");
	right_click();
	assert_eq!(pulses(), Vec::<u16>::new(), "nor is its Back");
	install(Stage::List, 0, None); // no snapshot
	assert!(go(p.0, p.1) && click_at(p) && wheel_down() && right_click());
	assert_eq!((request(), pulses(), lock().as_ref().and_then(|s| s.local)), (None, vec![], None));
}

#[test]
fn keys_and_the_pad_are_never_taken_and_the_hint_bar_follows_the_last_device() {
	let _w = world();
	install(Stage::List, 0, Some(snap(0xf, 2)));
	crate::modern_menu::look::note_pad();
	assert_eq!(device(), Device::Pad);
	for (code, pressed, repeat) in [(KeyCode::Enter, true, false), (KeyCode::Enter, true, true), (KeyCode::Enter, false, false), (KeyCode::Escape, true, false), (KeyCode::KeyF, true, false), (KeyCode::ArrowDown, true, false), (KeyCode::Space, true, false)] {
		assert!(!key(code, pressed, repeat), "{code:?} stays the game's");
	}
	assert_eq!(device(), Device::Keyboard);
	crate::modern_menu::look::note_pad();
	go(10.0, 10.0);
	assert_eq!(device(), Device::Pad, "a plain mouse move leaves the device to the window filter");
	mouse(MouseEvent::Down { button: MouseButton::Left, x: 10.0, y: 10.0 });
	assert_eq!(device(), Device::Keyboard, "the mouse is the keyboard side of the hint bar");
	begin(Some(PadSample { buttons: CROSS, lx: 0x80, ly: 0x80 }));
	assert_eq!(device(), Device::Pad, "a new pad press");
	assert_eq!((request(), pulses()), (None, vec![]), "the host sends nothing for a key or a pad press");
}

#[test]
fn leaving_the_page_gives_the_mouse_back() {
	let _w = world();
	install(Stage::List, 0, Some(snap(0xf, 2)));
	assert!(up() && local().is_none());
	set_up(false);
	assert!(!up() && lock().is_none());
	assert!(!mouse(MouseEvent::Move { x: 1.0, y: 1.0 }) && !wheel_up() && !right_click() && !click_at((5.0, 5.0)), "the retail map sees the mouse again");
	assert_eq!((request(), pulses()), (None, vec![]));
	assert!(last().is_empty());
	set_up(true);
	assert!(up() && last().is_empty(), "a new page starts empty");
	set_up(false);
}

#[test]
fn the_focus_lasts_one_grace_then_retails_cursor_wins() {
	let _w = world();
	let b = install(Stage::List, 0, Some(snap(0xf, 2)));
	let (x, y) = row_at(&b, 2);
	go(x, y);
	assert_eq!(local(), Some(2));
	assert_eq!(request(), req(2, false), "the hover that set the focus");
	std::thread::sleep(std::time::Duration::from_millis(GRACE_MS + 30));
	assert_eq!(local(), None);
}

#[test]
fn the_planet_list_is_remembered_from_the_list_stage_only() {
	let _w = world();
	let list = page(Stage::List, 2);
	install(Stage::List, 2, Some(snap(0xf, 2)));
	let prims = end(list.clone(), Some(&listing(7)), Some(&snap(0xf, 2)), (1280, 720));
	assert_eq!(prims, list.prims, "the draw list comes back");
	assert_eq!(memo().map(|l| (l.sel, l.rows.len(), l.item)), Some((list.sel, 11, ITEM)), "the cursor is the page's, the rows the game's");
	assert_eq!(last(), list.prims);
	end(page(Stage::Details, 5), Some(&listing(5)), Some(&snap(0x10, 2)), (1280, 720));
	assert_eq!(memo().map(|l| l.sel), Some(list.sel), "the details page keeps the List stage's list");
	set_up(false);
}

#[test]
fn a_key_or_a_pad_press_drops_the_mouse_focus_so_the_page_and_the_details_follow_the_game() {
	let _w = world();
	let b = install(Stage::List, 0, Some(snap(0xf, 2)));
	let (x, y) = row_at(&b, 2);
	go(x, y);
	assert_eq!(local(), Some(2));
	assert!(!key(KeyCode::ArrowDown, true, false), "the key stays the game's");
	assert_eq!(local(), None, "the game's cursor moved: the mouse's row must not hide it");
	go(x, y);
	assert_eq!(local(), Some(2));
	key(KeyCode::ArrowDown, true, true);
	assert_eq!(local(), None, "a repeating key too");
	go(x, y);
	begin(Some(PadSample { buttons: 0, lx: 0x80, ly: 0x80 }));
	assert_eq!(local(), Some(2), "no pad press, no change");
	begin(Some(PadSample { buttons: CROSS, lx: 0x80, ly: 0x80 }));
	assert_eq!(local(), None, "a new pad press");
	// The Details page names the game's planet: the last List frame built after the key had the cursor on row 3.
	go(x, y);
	key(KeyCode::ArrowDown, true, false);
	let (l, none, t) = (listing(3), ShipFacts::default(), |_: u32| String::new());
	let facts = Facts { list: &l, stage: Stage::List, focus: local(), scroll: 0, here: 3, planets: &none, infobot: true, text: &t, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation };
	end(build(&facts, &FontMeasure).expect("the page builds"), Some(&l), Some(&snap(0xf, 2)), (1280, 720));
	assert_eq!(memo().map(|m| m.sel), Some(3), "the Details page flies to the game's planet, not the mouse's stale row");
	set_up(false);
	let _ = (request(), pulses());
}

#[test]
fn the_planet_picture_is_one_overlay_crop_in_the_hole_of_a_settled_list_page() {
	use crate::modern_menu::ship_pic;
	let _w = world();
	let mut hud = Vec::new();
	place_planet(&mut hud, (1280, 720));
	assert!(hud.is_empty(), "the page is down: no quad");
	let b = install(Stage::List, 2, Some(snap(0xf, 2)));
	let hole = b.planet.expect("the list page has a hole");
	place_planet(&mut hud, (1280, 720));
	assert_eq!(hud.len(), 1);
	assert_eq!((hud[0].tex, hud[0].rect, hud[0].uv), (crate::viewer::RETAIL_OV_TEX, [hole.x, hole.y, hole.w, hole.h], ship_pic::uv(ship_pic::INSET)));
	// A page that flies in has no picture yet (the retail panel is not in place) and its hole is filled; the next settled frame has it.
	let on_screen = |hud: &mut Vec<crate::overlay::Quad>| {
		hud.clear();
		place_planet(hud, (1280, 720));
		hud.len()
	};
	let flying = end(page(Stage::List, 2), Some(&listing(2)), Some(&snap(0xf, ship_pic::FLYING)), (1280, 720));
	assert_eq!(on_screen(&mut hud), 0, "flying in");
	assert_eq!(flying.len(), page(Stage::List, 2).prims.len() + 1, "the hole is filled with the card colour");
	end(page(Stage::List, 2), Some(&listing(2)), Some(&snap(0xf, 2)), (1280, 720));
	assert_eq!(on_screen(&mut hud), 1, "settled");
	// The details page shows the game's gold bolt, not a crop; a closing menu takes the picture away.
	end(page(Stage::Details, 2), Some(&listing(2)), Some(&snap(0x10, 2)), (1280, 720));
	assert_eq!(on_screen(&mut hud), 0, "details page");
	end(page(Stage::List, 2), Some(&listing(2)), Some(&snap(0xf, 2)), (1280, 720));
	drop_planet();
	assert_eq!(on_screen(&mut hud), 0, "closing");
	set_up(false);
	assert_eq!(on_screen(&mut hud), 0, "page down again");
}

#[test]
fn the_accent_button_and_the_hint_bar_are_clickable() {
	let _w = world();
	let b = install(Stage::List, 2, Some(snap(0xf, 2)));
	let acts = |s: Symbol| b.acts.iter().filter(|a| a.0 == s).map(|a| centre(a.1)).collect::<Vec<_>>();
	assert_eq!((acts(Symbol::Cross).len(), acts(Symbol::Triangle).len(), acts(Symbol::Circle).len()), (2, 1, 0), "button + hint, Exit; the fixture lists no Infobot");
	click_at(acts(Symbol::Cross)[0]);
	assert_eq!(request().map(|r| (r.sub, r.confirm)), Some((2, true)), "the accent button opens the details of the chosen planet");
	click_at(acts(Symbol::Cross)[1]);
	assert_eq!(request().map(|r| (r.sub, r.confirm)), Some((2, true)), "so does the Select hint");
	click_at(acts(Symbol::Triangle)[0]);
	assert_eq!(pulses(), [TRIANGLE], "Exit leaves");
	let b = install(Stage::Details, 2, Some(snap(0x10, 2)));
	let cross = centre(b.acts.iter().find(|a| a.0 == Symbol::Cross).unwrap().1);
	click_at(cross);
	assert_eq!(pulses(), [CROSS], "on the details page the button blasts off (the game's Cross)");
}

/// The galaxy painting's quads are in the HUD's pixels, the render size: at Render scale 50 % of a 1280 x 720 window (a 640 x 360 HUD)
/// they cover exactly that HUD, never the window's 1280 x 720 (which drew the painting twice too big, its top-left quarter on screen).
/// Every overlay quad the hidden-retail step lays (`frame_ui::hide_retail`: portrait, preview, painting, planet, the planet map's crops)
/// gets the same size.
#[test]
fn the_galaxy_painting_is_laid_out_in_render_size_pixels() {
	let [black, art] = art_quads((640, 360));
	assert_eq!((black.rect, art.rect), ([0.0, 0.0, 640.0, 360.0], [0.0, 0.0, 640.0, 360.0]));
	assert_eq!((black.colour, art.colour), ([0.0, 0.0, 0.0, 1.0], [1.0; 4]));
	let [black, art] = art_quads((1280, 540));
	assert_eq!((black.rect, art.rect), ([0.0, 0.0, 1280.0, 540.0], [160.0, 0.0, 960.0, 540.0]), "a 21:9 box: the design canvas fits");
	let src = include_str!("../../host/handler/frame_ui.rs");
	let body = src.split("fn hide_retail").nth(1).expect("hide_retail").split("
impl App").next().unwrap();
	assert!(body.contains("let win = (g.config.width, g.config.height);"), "the HUD size is the render size");
	let calls: Vec<&str> = body.lines().filter(|l| l.contains("::place_")).collect();
	assert_eq!(calls.len(), 5, "{calls:#?}");
	assert!(calls.iter().all(|l| l.contains("win") && !l.contains("win_size")), "{calls:#?}");
}
