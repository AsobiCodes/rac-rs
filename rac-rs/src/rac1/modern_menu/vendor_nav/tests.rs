//! Tests of the keys of the modern shop's Buy all ammo row (`step` is pure; `field` with the page up is tested in
//! `vendor_host/tests.rs`, which owns the page's statics).
use super::*;
use crate::retail::shop_view::tests::{Ram, LEVEL, STOCK};

const SHORT: List = List { sel: 0, count: 3, cyclic: false, row: true };

fn at(sel: u32, count: u32, cyclic: bool) -> Option<List> {
	Some(List { sel, count, cyclic, row: true })
}

fn focused_nav() -> Nav {
	Nav { focus: true, ..Nav::default() }
}

#[test]
fn up_from_the_first_row_focuses_buy_all_and_down_gives_the_row_back() {
	let mut n = Nav::default();
	assert_eq!(step(&mut n, Some(SHORT), UP), Out { pad: 0, cue: true }, "Up on row 0: the focus moves, retail sees no Left");
	assert!(n.focus);
	assert_eq!(step(&mut n, Some(SHORT), UP), Out { pad: 0, cue: false }, "held: still eaten, no second move");
	assert_eq!(step(&mut n, Some(SHORT), 0), Out { pad: 0, cue: false });
	assert_eq!(step(&mut n, Some(SHORT), DOWN), Out { pad: 0, cue: true }, "Down: back to row 0, where retail's cursor is");
	assert!(!n.focus);
	assert_eq!(step(&mut n, Some(SHORT), DOWN).pad, 0, "the eaten Down stays away while held");
	assert_eq!(step(&mut n, Some(SHORT), 0).pad, 0);
	assert_eq!(step(&mut n, Some(SHORT), DOWN), Out { pad: RIGHT, cue: false }, "the next Down is retail's next row");
}

#[test]
fn up_and_down_walk_the_vertical_list_only_while_it_browses() {
	let mut n = Nav::default();
	let mid = at(1, 3, false);
	assert_eq!(step(&mut n, mid, UP).pad, LEFT, "Up = previous row");
	assert_eq!(step(&mut n, mid, 0).pad, 0);
	assert_eq!(step(&mut n, mid, DOWN).pad, RIGHT, "Down = next row");
	assert_eq!(step(&mut n, mid, DOWN | CROSS).pad, RIGHT | CROSS);
	assert!(!n.focus, "Up from a later row is retail's own move");
	let mut n = Nav::default();
	for pad in [UP, DOWN, LEFT, RIGHT, CROSS, UP | DOWN | CROSS] {
		assert_eq!(step(&mut n, None, pad).pad, pad, "the prompt, an animating shop, the page down: unchanged");
		step(&mut n, None, 0);
	}
}

#[test]
fn buy_all_is_the_top_of_a_short_list() {
	let mut n = focused_nav();
	assert_eq!(step(&mut n, Some(SHORT), UP), Out { pad: 0, cue: false }, "nothing above Buy all ammo");
	step(&mut n, Some(SHORT), 0);
	assert_eq!(step(&mut n, Some(SHORT), LEFT).pad, 0, "the arrow key Left too");
	assert!(n.focus);
	step(&mut n, Some(SHORT), 0);
	let mut last = Nav::default();
	assert_eq!(step(&mut last, at(2, 3, false), DOWN).pad, RIGHT, "Down on the last row: retail clamps");
	assert!(!last.focus, "a short list does not wrap onto Buy all ammo");
}

#[test]
fn a_wrapping_list_has_buy_all_between_its_last_and_first_rows() {
	let mut n = focused_nav();
	assert_eq!(step(&mut n, at(0, 9, true), UP), Out { pad: LEFT, cue: false }, "Up from Buy all: retail wraps to its last row, with its sound");
	assert!(!n.focus);
	let mut n = Nav::default();
	assert_eq!(step(&mut n, at(8, 9, true), DOWN), Out { pad: RIGHT, cue: false }, "Down on the last row: retail wraps to row 0, the focus is above it");
	assert!(n.focus);
	let mut n = Nav::default();
	assert_eq!(step(&mut n, at(0, 9, true), UP), Out { pad: 0, cue: true }, "Up on row 0 of a wrapping list: Buy all, not retail's wrap");
	assert!(n.focus);
}

#[test]
fn cross_on_buy_all_is_square_while_it_is_held() {
	let mut n = focused_nav();
	assert_eq!(step(&mut n, Some(SHORT), CROSS).pad, SQUARE, "Enter / Cross buys all, no purchase prompt");
	assert_eq!(step(&mut n, Some(SHORT), CROSS).pad, SQUARE, "held: Square held, no new press");
	assert_eq!(step(&mut n, Some(SHORT), CROSS | DOWN).pad, SQUARE, "the focus leaves while Cross is held: still Square until released");
	assert!(!n.focus);
	assert_eq!(step(&mut n, Some(SHORT), 0).pad, 0);
	assert_eq!(step(&mut n, Some(SHORT), CROSS).pad, CROSS, "on a row Cross is Cross");
	let mut held = Nav::default();
	step(&mut held, at(1, 3, false), CROSS);
	held.focus = true;
	assert_eq!(step(&mut held, Some(SHORT), CROSS).pad, CROSS, "a Cross held from before the focus came is no new press");
}

#[test]
fn an_eaten_key_never_reaches_the_game_late() {
	let mut n = Nav::default();
	assert_eq!(step(&mut n, Some(SHORT), LEFT).pad, 0);
	assert!(n.focus);
	// The page goes down while Left is still held (the shop closes): the key stays away until released, then all is unchanged.
	n.focus = false;
	assert_eq!(step(&mut n, None, LEFT).pad, 0);
	assert_eq!(step(&mut n, None, LEFT | CROSS).pad, CROSS);
	assert_eq!(step(&mut n, None, 0).pad, 0);
	for pad in [LEFT, 0, 0xffff, 0] {
		assert_eq!(step(&mut n, None, pad).pad, pad);
	}
}

#[test]
fn the_list_is_the_browsing_shop_only() {
	let snap = |sel, stage| {
		let ram = Ram::shop(&STOCK, &[], 4200, sel, stage);
		mouse_vendor::read(&|a| ram.0.get(&a).copied().unwrap_or(0), LEVEL)
	};
	assert_eq!(list(snap(1, 0).as_ref(), true), at(1, 3, false));
	assert_eq!(list(snap(1, 0).as_ref(), false).map(|l| l.row), Some(false), "the page lists no Buy all row");
	assert_eq!(list(snap(1, 2).as_ref(), true), None, "the quantity prompt");
	assert_eq!(list(None, true), None);
}

#[test]
fn without_the_row_up_and_down_only_walk_the_list() {
	let bare = Some(List { row: false, ..SHORT });
	let mut n = Nav::default();
	assert_eq!(step(&mut n, bare, UP), Out { pad: LEFT, cue: false }, "Up on row 0: retail's Left (it clamps), no focus");
	assert!(!n.focus);
	step(&mut n, bare, 0);
	let mut n = focused_nav();
	assert_eq!(step(&mut n, bare, DOWN), Out { pad: RIGHT, cue: false }, "a stale focus goes: Down is retail's next row");
	assert!(!n.focus);
}

#[test]
fn the_focus_goes_when_retails_cursor_leaves_row_0_under_it() {
	let mut n = focused_nav();
	step(&mut n, at(0, 3, false), 0);
	assert!(n.focus);
	step(&mut n, at(2, 3, false), 0);
	assert!(!n.focus, "retail went to row 2 under the focus (a mouse request, a late snapshot): the page shows retail's row");
	let mut n = focused_nav();
	for sel in [3, 2, 1, 0, 0] {
		step(&mut n, at(sel, 4, false), 0);
		assert!(n.focus, "the mouse chose Buy all from row 3: retail's cursor walks to row 0 under the focus (sel {sel})");
	}
}

#[test]
fn a_held_key_moves_one_place_as_in_retail() {
	// Retail's shop reads the pad's edge word 0x13cb04, not the auto-repeat word: a held key moves one row there.
	let mut n = Nav::default();
	assert_eq!(step(&mut n, at(8, 9, true), DOWN).pad, RIGHT, "the last row of a wrapping list: retail wraps to row 0");
	assert!(n.focus);
	for _ in 0..30 {
		assert_eq!(step(&mut n, at(0, 9, true), DOWN).pad, RIGHT, "held: retail sees no new press");
		assert!(n.focus, "held Down stays on Buy all ammo, no wrap past it");
	}
	step(&mut n, at(0, 9, true), 0);
	assert_eq!(step(&mut n, at(0, 9, true), DOWN), Out { pad: 0, cue: true }, "a new press: row 0");
	assert!(!n.focus);
}

#[test]
fn the_left_stick_is_the_d_pad_on_the_page() {
	let _g = crate::retail::menu_model::cmd::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let ram = Ram::shop(&STOCK, &[], 4200, 0, 0);
	let snap = mouse_vendor::read(&|a| ram.0.get(&a).copied().unwrap_or(0), LEVEL);
	let go = |up: bool, s: [u8; 4]| {
		let (mut b, mut s) = (0u16, s);
		field_in(&mut b, &mut s, snap.as_ref(), up, true);
		(b, s)
	};
	set_focus(false);
	go(false, [0x80; 4]); // NAV's memory of other tests
	let _ = crate::retail::vendor::take_cue();
	assert_eq!(go(true, [0x80, 0x80, 0x80, 0x00]), (0, [0x80; 4]), "stick up on row 0: Buy all ammo, the push stays away from the game");
	assert!(focused() && crate::retail::vendor::take_cue());
	go(true, [0x80; 4]);
	assert_eq!(go(true, [0x80, 0x80, 0xff, 0x80]), (0, [0x80; 4]), "stick right on Buy all ammo: back to row 0, eaten");
	assert!(!focused() && crate::retail::vendor::take_cue());
	go(true, [0x80; 4]);
	assert_eq!(go(true, [0x80, 0x80, 0xff, 0x80]), (RIGHT, [0x80; 4]), "stick right on row 0: retail's next row, as the D-pad");
	go(true, [0x80; 4]);
	let s = [0x10, 0xf0, 0xff, 0x00];
	assert_eq!(go(false, s), (0, s), "the page down: the sticks as they are");
	set_focus(false);
}
