//! Tests of the vendor host: the mouse (hover, click, wheel, right click) sends row requests and pad pulses, the buy prompt takes only
//! its four buttons, keys are never taken. The statics (`UP`, `ST`) and the command queues
//! (`menu_model::cmd`) are shared: every test takes `cmd::TEST_LOCK` first (`world`) and leaves the queues empty.
use super::*;
use crate::modern_menu::vendor::{build, Facts};
use crate::retail::menu_model::cmd::{self, TEST_LOCK};
use crate::retail::mouse_instant::{Cmd, Request};
use crate::retail::mouse_vendor::{self, ID};
use crate::retail::shop_view::tests::{Ram, LEVEL, STOCK};
use crate::retail::shop_view::ShopView;
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::input::GlyphSet;
use rac_engine::ui::prim::Rect;
use std::sync::MutexGuard;

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
	WATCH.allow(false); // no trip left over from another test
	vendor_nav::set_focus(false);
	let _ = (request(), pulses(), crate::retail::vendor::take_cue());
	cmd::stamp(None);
	g
}

/// The shop at `stage` with `sel` highlighted, 22 / 40 and 150 / 240 rounds held: the game's list and the mouse's view of it.
fn shop(stage: u32, sel: u32, bolts: u32) -> (ShopView, MenuSnap) {
	let ram = Ram::shop(&STOCK, &[(10, 22), (16, 150)], bolts, sel, stage);
	let rd = |a: u32| ram.0.get(&a).copied().unwrap_or(0);
	(ram.view(true).expect("a shop"), mouse_vendor::read(&rd, LEVEL).expect("a snapshot"))
}

fn page(v: &ShopView) -> Built {
	let names = ["Bomb Glove".to_string(), "Pyrocitor Ammo".to_string(), "Blaster Ammo".to_string()];
	build(&Facts { view: v, names: &names, focus: None, buyall: false, scroll: 0, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation }, &FontMeasure).expect("the page builds")
}

/// The page up on the 1280x720 window (design pixels = window pixels).
fn install(v: &ShopView, snap: Option<MenuSnap>) -> Built {
	let b = page(v);
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

fn req(m: &MenuSnap, sub: u32, confirm: bool) -> Option<Request> {
	Some(Request { page_id: ID, item: m.cur, sub, confirm, slide: None })
}

#[test]
fn a_hover_chooses_the_row_and_a_second_click_buys() {
	let _w = world();
	let (v, m) = shop(0, 0, 4200);
	let b = install(&v, Some(m.clone()));
	let (x, y) = row_at(&b, 1);
	assert!(go(x, y), "the page takes the mouse");
	assert_eq!(request(), req(&m, 1, false), "hover = choose, no Cross");
	assert_eq!(lock().as_ref().and_then(|s| s.local).map(|l| l.0), Some(1), "the focus moves at once");
	go(x + 1.0, y);
	assert_eq!(request(), None, "the same row again sends nothing");
	click_at((x, y));
	assert_eq!(request(), req(&m, 1, true), "a click on the chosen row confirms");
	click_at(row_at(&b, 2));
	assert_eq!(request(), req(&m, 2, false), "a click on another row only chooses it");
	assert_eq!(pulses(), Vec::<u16>::new());
}

#[test]
fn the_buttons_press_the_pad_buttons_the_game_reads() {
	let _w = world();
	let (v, m) = shop(0, 1, 4200);
	let b = install(&v, Some(m.clone()));
	click_at(centre(b.buy.expect("an affordable buy button")));
	click_at(centre(b.refill.expect("a refill button")));
	assert_eq!(request(), req(&m, 0, false), "the first click on Buy all ammo chooses it: retail's cursor goes to row 0, below it");
	assert!(vendor_nav::focused());
	click_at(centre(b.refill.unwrap()));
	assert_eq!(pulses(), [CROSS, SQUARE], "the second click buys all");
	mouse(MouseEvent::Down { button: MouseButton::Right, x: 5.0, y: 5.0 });
	assert_eq!(pulses(), [TRIANGLE], "a right click leaves, like Triangle");
	click_at((640.0, 400.0));
	assert_eq!((pulses(), request()), (vec![], None), "a click on nothing does nothing");
}

#[test]
fn the_buy_prompt_takes_only_its_four_buttons() {
	let _w = world();
	let (v, m) = shop(2, 1, 4200);
	let b = install(&v, Some(m));
	assert!(b.prompt);
	// The list, the detail card's buy and refill spots and the empty world behind the modal do nothing.
	for i in 0..3 {
		let p = row_at(&b, i);
		go(p.0, p.1);
		click_at(p);
	}
	for r in [b.buy, b.refill].into_iter().flatten() {
		click_at(centre(r));
	}
	click_at((640.0, 600.0));
	assert_eq!((pulses(), request()), (vec![], None), "nothing under the prompt but its buttons");
	for r in [b.more, b.less, b.confirm, b.cancel] {
		click_at(centre(r.expect("a live prompt button")));
	}
	assert_eq!(pulses(), [RIGHT, LEFT, CROSS, TRIANGLE]);
	mouse(MouseEvent::Wheel { lines: 1.0 });
	mouse(MouseEvent::Wheel { lines: -2.0 });
	assert_eq!(pulses(), [RIGHT, LEFT], "the wheel steps the quantity: up = more");
	mouse(MouseEvent::Down { button: MouseButton::Right, x: 5.0, y: 5.0 });
	assert_eq!(pulses(), [TRIANGLE], "a right click cancels the prompt");
}

#[test]
fn the_wheel_walks_the_list_and_stops_at_its_ends() {
	let _w = world();
	let (v, m) = shop(0, 0, 4200);
	install(&v, Some(m.clone()));
	mouse(MouseEvent::Wheel { lines: 1.0 });
	assert_eq!(request(), None, "up at the first row: Buy all ammo, retail's cursor stays");
	assert!(vendor_nav::focused() && crate::retail::vendor::take_cue(), "the focus moved, with the browse sound");
	mouse(MouseEvent::Wheel { lines: 1.0 });
	assert!(vendor_nav::focused() && !crate::retail::vendor::take_cue() && request().is_none(), "up at Buy all ammo");
	mouse(MouseEvent::Wheel { lines: -1.0 });
	assert!(!vendor_nav::focused() && crate::retail::vendor::take_cue() && request().is_none(), "down: row 0, where the cursor is");
	mouse(MouseEvent::Wheel { lines: -1.0 });
	assert_eq!(request(), req(&m, 1, false));
	mouse(MouseEvent::Wheel { lines: -1.0 });
	assert_eq!(request(), req(&m, 2, false), "the page's own focus counts, retail's cursor is still on row 0");
	mouse(MouseEvent::Wheel { lines: -1.0 });
	assert_eq!(request(), None, "down at the last row");
	mouse(MouseEvent::Wheel { lines: 0.0 });
	assert_eq!((request(), pulses()), (None, vec![]));
}

#[test]
fn a_shop_the_mouse_cannot_address_sends_nothing_but_is_still_taken() {
	let _w = world();
	let (v, _) = shop(0, 0, 4200);
	let b = install(&v, None); // eight rows or more: no snapshot
	let p = row_at(&b, 1);
	assert!(go(p.0, p.1) && click_at(p));
	mouse(MouseEvent::Wheel { lines: -1.0 });
	assert_eq!((request(), lock().as_ref().and_then(|s| s.local)), (None, None));
	let (_, busy) = shop(1, 0, 4200);
	install(&v, Some(busy));
	click_at(p);
	assert_eq!(request(), None, "a shop that animates is not for the mouse");
}

#[test]
fn keys_are_never_taken_and_the_hint_bar_follows_the_last_device() {
	let _w = world();
	let (v, m) = shop(0, 0, 4200);
	install(&v, Some(m));
	crate::modern_menu::look::note_pad();
	assert_eq!(device(), Device::Pad);
	for (code, pressed, repeat) in [(KeyCode::Enter, true, false), (KeyCode::Enter, true, true), (KeyCode::Enter, false, false), (KeyCode::Escape, true, false), (KeyCode::KeyF, true, false), (KeyCode::ArrowRight, true, false)] {
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
	assert_eq!((request(), pulses()), (None, vec![]));
}

#[test]
fn leaving_the_page_gives_the_mouse_back() {
	let _w = world();
	let (v, m) = shop(0, 0, 4200);
	install(&v, Some(m));
	assert!(up() && local().is_none());
	set_up(false);
	assert!(!up() && lock().is_none());
	assert!(!mouse(MouseEvent::Move { x: 1.0, y: 1.0 }) && !mouse(MouseEvent::Wheel { lines: 1.0 }), "the retail shop sees the mouse again");
	assert!(last().is_empty());
	set_up(true);
	assert!(up() && last().is_empty(), "a new page starts empty");
	set_up(false);
}

#[test]
fn the_focus_lasts_one_grace_then_retails_cursor_wins() {
	let _w = world();
	let (v, m) = shop(0, 0, 4200);
	let b = install(&v, Some(m));
	let (x, y) = row_at(&b, 2);
	go(x, y);
	assert_eq!(local(), Some(2));
	std::thread::sleep(std::time::Duration::from_millis(GRACE_MS + 30));
	assert_eq!(local(), None);
}

#[test]
fn a_key_or_a_pad_press_drops_the_mouse_focus() {
	let _w = world();
	let (v, m) = shop(0, 0, 4200);
	let b = install(&v, Some(m));
	let (x, y) = row_at(&b, 2);
	go(x, y);
	assert_eq!(local(), Some(2));
	assert!(!key(KeyCode::ArrowDown, true, false), "the key stays the game's");
	assert_eq!(local(), None, "the game's cursor moved: the mouse's row must not hide it");
	go(x, y);
	key(KeyCode::ArrowDown, true, true);
	assert_eq!(local(), None, "a repeating key too");
	go(x, y);
	begin(Some(PadSample { buttons: 0, lx: 0x80, ly: 0x80 }));
	assert_eq!(local(), Some(2), "no pad press, no change");
	begin(Some(PadSample { buttons: CROSS, lx: 0x80, ly: 0x80 }));
	assert_eq!(local(), None, "a new pad press");
	let _ = (request(), pulses());
}

#[test]
fn the_prompt_names_the_games_item_whatever_the_mouse_focused() {
	let _w = world();
	let (v, _) = shop(2, 1, 4200);
	let names = ["Bomb Glove".to_string(), "Pyrocitor Ammo".to_string(), "Blaster Ammo".to_string()];
	let facts = |focus| Facts { view: &v, names: &names, focus, buyall: false, scroll: 0, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation };
	assert_eq!(build(&facts(Some(2)), &FontMeasure).expect("the page builds").sel, 1, "the prompt asks about the game's row");
	let (browse, _) = shop(0, 1, 4200);
	let f = Facts { view: &browse, names: &names, focus: Some(2), buyall: false, scroll: 0, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation };
	assert_eq!(build(&f, &FontMeasure).expect("the page builds").sel, 2, "the list still follows the mouse's focus");
}

#[test]
fn the_wheel_sends_nothing_without_quantity_arrows() {
	let _w = world();
	for (stage, sel, why) in [(3, 1, "the closing step"), (4, 1, "the resolving step"), (2, 0, "the weapon prompt")] {
		let (v, m) = shop(stage, sel, 4200);
		let b = install(&v, Some(m));
		assert!(b.prompt && b.less.is_none() && b.more.is_none(), "{why}: a prompt without arrows");
		mouse(MouseEvent::Wheel { lines: 1.0 });
		mouse(MouseEvent::Wheel { lines: -1.0 });
		assert_eq!((pulses(), request()), (vec![], None), "{why}: the wheel queues no Left / Right for the list");
	}
	let (v, m) = shop(2, 1, 4200);
	install(&v, Some(m));
	mouse(MouseEvent::Wheel { lines: 1.0 });
	assert_eq!(pulses(), [RIGHT], "the asking ammo prompt still steps");
}

/// The watchdog: a page that is up with nothing built for 1.5 s is given up (the classic shop and mouse come back) and the shop's
/// "hold" frames, which ask it up again every frame, do not bring the blank page back; leaving the screen once ends the trip. A page
/// on screen (a finished frame, or the repeated last page) is never given up; an empty "hold" page is.
#[test]
fn the_watchdog_gives_up_a_blank_page_and_holds_do_not_revive_it() {
	use crate::modern_menu::watchdog::SILENT_MS;
	let _w = world();
	set_up(true);
	let t = now();
	assert!(!silent_at(t + 100), "a page that just came up has time");
	assert!(silent_at(t + SILENT_MS + 200), "up, nothing built for 1.5 s, nothing to draw");
	trip();
	assert!(!up() && !silent_at(t + 10_000), "given up: the mouse is the classic one again");
	set_up(true);
	assert!(!up(), "the hold frame asks the page up again: refused");
	set_up(false); // the shop was left
	set_up(true);
	assert!(up(), "a new visit starts normally");
	let (v, m) = shop(0, 0, 4200);
	end(page(&v), Some(&m), (1280, 720));
	assert!(!silent_at(now() + SILENT_MS * 4), "a picture is on screen: never stuck");
	hold();
	assert!(last().is_empty() && silent_at(now() + SILENT_MS * 4), "an empty hold page that stays is stuck");
	set_up(false);
}

#[test]
fn a_hold_counts_as_a_page_for_the_watchdog_only_while_young() {
	assert!(hold_young(100, 100) && hold_young(100, 100 + HOLD_FIELDS - 1));
	assert!(!hold_young(100, 100 + HOLD_FIELDS), "a hold of 3 s is a stuck shop");
	assert!(hold_young(u32::MAX - 5, 10), "the field counter wraps");
	assert!(HOLD_FIELDS > 37 * 3, "well past the measured fly-in");
}

/// Review R4-1 / R2b-1: within SETTLE_MS of the switch between the list and the prompt a click presses nothing (the second click of a
/// double click on Buy or Cancel); afterwards the prompt's buttons work as before.
#[test]
fn a_click_right_after_the_prompt_opens_or_closes_presses_nothing() {
	let _w = world();
	let (lv, lm) = shop(0, 1, 4200);
	let (pv, pm) = shop(2, 1, 4200);
	install(&lv, Some(lm.clone()));
	end(page(&lv), Some(&lm), (1280, 720));
	assert_eq!(lock().as_ref().and_then(|s| s.flip_at), None, "the first page is no switch");
	end(page(&pv), Some(&pm), (1280, 720));
	let at = lock().as_ref().and_then(|s| s.flip_at).expect("list -> prompt is a switch");
	let p = page(&pv);
	let tap = |r: Option<Rect>, t: u64| {
		let (x, y) = centre(r.expect("a live prompt button"));
		click(lock().as_mut().expect("up"), x, y, t);
	};
	tap(p.confirm, at + 10);
	tap(p.cancel, at + SETTLE_MS - 1);
	assert_eq!((pulses(), request()), (vec![], None), "a double click never confirms or cancels the prompt");
	tap(p.confirm, at + SETTLE_MS);
	let on_prompt: Vec<u16> = (0..20).map(|_| cmd::pad_step(Some(&pm), 0)).filter(|&b| b != 0).collect();
	assert_eq!(on_prompt, [CROSS], "a click once the prompt settled confirms");
	end(page(&lv), Some(&lm), (1280, 720));
	let at = lock().as_ref().and_then(|s| s.flip_at).expect("prompt -> list is a switch");
	let l = page(&lv);
	let (x, y) = row_at(&l, 1);
	click(lock().as_mut().expect("up"), x, y, at + 20);
	tap(l.buy, at + 30);
	assert_eq!(pulses(), Vec::<u16>::new(), "nor buys again from the list it went back to");
	assert_eq!(request().map(|r| r.confirm), Some(false), "a row still takes the hover part of the click");
	cmd::stamp(None);
}

/// Review R2b-2: Buy buys the row the page shows; while the hover is ahead of the game's cursor the row is selected with Cross.
#[test]
fn buy_takes_the_row_the_page_shows() {
	let _w = world();
	let (v, m) = shop(0, 1, 4200);
	let b = install(&v, Some(m.clone()));
	let buy = centre(b.buy.expect("an affordable buy button"));
	click_at(buy);
	assert_eq!((pulses(), request()), (vec![CROSS], None), "no hover: Cross on the game's row");
	let (x, y) = row_at(&b, 2);
	go(x, y);
	assert_eq!(request(), req(&m, 2, false), "the hover moves the game's cursor");
	click_at(buy);
	assert_eq!((pulses(), request()), (vec![], req(&m, 2, true)), "the game's cursor is still on row 1: select row 2 with Cross");
}

/// Buy all ammo is a row for the mouse too (user 2026-11-03): a hover chooses it, a click on it buys all, Buy and the hint's Cross
/// buy all while it is chosen, a hover or click on a stock row takes the focus back.
#[test]
fn buy_all_ammo_takes_the_hover_and_buys_on_click() {
	let _w = world();
	let (v, m) = shop(0, 0, 4200);
	let b = install(&v, Some(m.clone()));
	let all = centre(b.refill.expect("the Buy all row"));
	go(all.0, all.1);
	assert!(vendor_nav::focused() && crate::retail::vendor::take_cue(), "the hover chooses it; retail's cursor is on row 0 already: the sound alone");
	assert_eq!((request(), lock().as_ref().and_then(|s| s.local)), (None, None));
	go(all.0 + 1.0, all.1);
	assert!(!crate::retail::vendor::take_cue(), "the same row again: nothing");
	click_at(all);
	click_at(centre(b.hints.iter().find(|h| h.0 == Symbol::Cross).expect("the hint's Buy").1));
	assert_eq!(pulses(), [SQUARE, SQUARE], "the row's click and the hint's Buy both buy all, no purchase prompt");
	let (x, y) = row_at(&b, 0);
	go(x, y);
	assert!(!vendor_nav::focused() && crate::retail::vendor::take_cue() && request().is_none(), "back on row 0: the sound alone");
	go(all.0, all.1);
	assert!(vendor_nav::focused());
	let _ = crate::retail::vendor::take_cue();
	click_at(row_at(&b, 0));
	assert!(!vendor_nav::focused() && request().is_none() && pulses().is_empty(), "a click on row 0 from Buy all only chooses it");
	go(all.0, all.1);
	click_at(row_at(&b, 2));
	assert_eq!(request(), req(&m, 2, false), "a click on another row chooses it");
	assert!(!vendor_nav::focused());
	let _ = crate::retail::vendor::take_cue();
	// Not while the prompt is up, and the page coming up again starts on retail's own row.
	let (p, pm) = shop(2, 1, 4200);
	install(&p, Some(pm));
	wheel_up_and_check(false);
	vendor_nav::set_focus(true);
	set_up(false);
	assert!(!vendor_nav::focused(), "the page going down drops the focus");
}

fn wheel_up_and_check(expect: bool) {
	mouse(MouseEvent::Wheel { lines: 1.0 });
	assert_eq!(vendor_nav::focused(), expect);
	let _ = (pulses(), request());
}

/// The keys of the page (`vendor_nav::field`, WK): Up on row 0 chooses Buy all ammo with the sound, Enter there is Square, Down gives
/// row 0 back; the page down passes the pad word unchanged.
#[test]
fn the_keys_reach_buy_all_ammo_while_the_page_is_up() {
	let _w = world();
	let (v, m) = shop(0, 0, 4200);
	install(&v, Some(m.clone()));
	let field = |pad: u16| {
		let mut b = pad;
		vendor_nav::field_in(&mut b, &mut [0x80; 4], Some(&m), up(), true);
		b
	};
	use crate::retail::mouse_menu::{DOWN, UP as PAD_UP};
	assert_eq!(field(PAD_UP), 0);
	assert!(vendor_nav::focused() && crate::retail::vendor::take_cue());
	assert_eq!((field(0), field(CROSS), field(0)), (0, SQUARE, 0), "Enter / Cross on the row = Square");
	assert_eq!(field(DOWN), 0);
	assert!(!vendor_nav::focused() && crate::retail::vendor::take_cue());
	assert_eq!((field(0), field(DOWN), field(0)), (0, RIGHT, 0), "Down = retail's next row");
	UP.store(false, Ordering::Relaxed);
	vendor_nav::set_focus(true);
	assert_eq!((field(PAD_UP), field(0), field(CROSS), field(0)), (PAD_UP, 0, CROSS, 0), "the page down: unchanged");
	vendor_nav::set_focus(false);
}
