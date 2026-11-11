//! The controller as actions (v0.5.1 pad lane): the equivalence oracle against today's `pad_buttons(phys, map)` (kept in `binds.rs`),
//! the `gamepad=` file, the conversion of an old `pad=` line, Pause never without a button, sharing, the capture.

use super::generic::TABLE;
use super::*;
use crate::input::binds::{pad_buttons, pad_from_text, pad_shown, DEFAULT_PAD, NP, PHYS};
use winit::keyboard::KeyCode as K;

/// Every physical mask worth testing: nothing, every single button, every pair of the 16.
pub(crate) fn masks() -> Vec<u32> {
	let mut v = vec![0u32];
	for a in 0..NP {
		v.push(PHYS[a].1);
		for b in a + 1..NP {
			v.push(PHYS[a].1 | PHYS[b].1);
		}
	}
	v
}

/// Old `pad=` texts: the ones the existing tests use plus pseudo-random remaps (every function onto any button, with sharing).
pub(crate) fn old_texts() -> Vec<String> {
	let mut v: Vec<String> = ["cross:b", "cross:b,circle:a", "circle:a,cross:b", "l2:lb,l1:lt", "start:back,select:start", "r2:a", "r2:lb,cross:rt", "up:ddown,down:dup", "l3:rs,r3:ls"].iter().map(|s| s.to_string()).collect();
	let mut x = 0x2545_f491u32;
	let mut next = move || {
		x ^= x << 13;
		x ^= x >> 17;
		x ^= x << 5;
		x
	};
	for _ in 0..60 {
		let n = 1 + next() as usize % 6;
		let e: Vec<String> = (0..n).map(|_| format!("{}:{}", crate::input::binds::ID[next() as usize % NP], PHYS[next() as usize % NP].0)).collect();
		v.push(e.join(","));
	}
	v
}

fn slots(text: Option<&str>, gamepad: Option<&str>) -> Binds {
	let mut b = Binds::defaults(&TABLE);
	b.1 = match (gamepad, text) {
		(Some(g), _) => padfile::from_text(&TABLE, g),
		(None, Some(p)) => padfile::convert_old(&TABLE, p),
		_ => b.1,
	};
	b
}

#[test]
fn the_default_controller_equals_the_standard_layout_for_every_single_button_and_pair() {
	let b = Binds::defaults(&TABLE);
	for m in masks() {
		let o = eval_pad(&TABLE, &b, m, None);
		assert_eq!(o.buttons, pad_buttons(m, &DEFAULT_PAD), "mask {m:#x}");
		assert_eq!(o.free, o.buttons);
		assert_eq!(o.pc, 0);
	}
}

#[test]
fn converted_old_pad_files_equal_the_old_mapping_for_every_single_button_and_pair() {
	for text in old_texts() {
		let (map, b) = (pad_from_text(&text), slots(Some(&text), None));
		for m in masks() {
			assert_eq!(eval_pad(&TABLE, &b, m, None).buttons, pad_buttons(m, &map), "{text} mask {m:#x}");
		}
		assert_eq!(padfile::legacy_map(&TABLE, &b), map, "{text}: the old view");
		// The converted slots survive the new file.
		let g = padfile::to_text(&TABLE, &b);
		assert_eq!(slots(None, g.as_deref()), b, "{text}: round trip of {g:?}");
	}
}

#[test]
fn gamepad_text_round_trips_and_unreadable_entries_are_skipped() {
	assert_eq!(padfile::to_text(&TABLE, &Binds::defaults(&TABLE)), None, "defaults write nothing");
	let mut b = Binds::defaults(&TABLE);
	assert!(b.assign_pad(&TABLE, 0, 0, 1));
	assert!(b.assign_pad(&TABLE, 1, 1, 15));
	assert_eq!(padfile::to_text(&TABLE, &b).as_deref(), Some("cross:b|-,square:x|rt"));
	assert_eq!(padfile::from_text(&TABLE, "cross:b|-,square:x|rt"), b.1);
	let d = Binds::defaults(&TABLE).1;
	assert_eq!(padfile::from_text(&TABLE, "bogus:a,cross:zz|-,cross,forward:a|-,start:-|-,cross:a|b|x"), d, "nothing readable, nothing changes");
	assert_eq!(padfile::from_text(&TABLE, "cross:-|-")[0], [None, None], "a plain action may be empty");
	assert!(!b.assign_pad(&TABLE, 16, 0, 1), "stick directions are not controller actions");
	assert!(!b.assign_pad(&TABLE, 0, 0, 16) && !b.assign_pad(&TABLE, 0, 2, 1));
}

#[test]
fn pause_never_loses_its_last_controller_button() {
	let start = TABLE.index("start").unwrap();
	let mut b = Binds::defaults(&TABLE);
	assert!(!b.clear_pad(&TABLE, start, 0), "the only button");
	assert!(b.assign_pad(&TABLE, start, 1, 7));
	assert!(b.clear_pad(&TABLE, start, 0));
	assert!(!b.clear_pad(&TABLE, start, 1));
	assert_eq!(b.1[start], [None, Some(7)]);
	assert!(b.clear_pad(&TABLE, 0, 0), "an ordinary action may be emptied");
	assert!(!b.clear_pad(&TABLE, 0, 0), "already empty");
}

#[test]
fn a_shared_button_drives_both_and_resets_are_per_device() {
	let mut b = Binds::defaults(&TABLE);
	assert!(b.assign_pad(&TABLE, 0, 0, 2)); // Jump onto X, which is Square's
	assert_eq!(eval_pad(&TABLE, &b, PHYS[2].1, None).buttons, 0x40 | 0x80);
	assert_eq!(b.shared_pad(&TABLE, 0), vec![1]);
	assert_eq!(b.shared_pad(&TABLE, 1), vec![0]);
	assert!(b.has_pad(0, 2) && !b.has_pad(0, 0));
	b.assign(0, 1, Input::Key(K::KeyK));
	b.reset_pad(&TABLE);
	assert_eq!(b.1, Binds::defaults(&TABLE).1);
	assert_eq!(b.0[0][1], Some(Input::Key(K::KeyK)), "the keyboard keeps its key");
	b.assign_pad(&TABLE, 0, 0, 3);
	b.reset_kbm(&TABLE);
	assert_eq!(b.1[0][0], Some(3), "the controller keeps its button");
}

#[test]
fn live_state_reads_gamepad_over_the_old_pad_line_and_writes_only_gamepad() {
	let _lock = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	init(None, None, Some("cross:b"), None);
	assert_eq!(binds().1[0], [Some(1), None], "old pad= converted");
	assert_eq!(crate::input::binds::pad_map()[0], 1);
	init(None, None, Some("cross:b"), Some("cross:y|-"));
	assert_eq!(binds().1[0], [Some(3), None], "gamepad= wins");
	assign_pad(1, 0, 5);
	assert_eq!(take_changes().unwrap().1.as_deref(), Some("cross:y|-,square:rb|-"));
	assert_eq!(take_changes(), None);
	assert!(clear_pad(1, 0) && binds().1[1] == [None, None]);
	reset_pad();
	assert_eq!(binds().1, Binds::defaults(table()).1);
	assert_eq!(take_changes().unwrap().1, None, "defaults: the line goes");
}

#[test]
fn a_controller_capture_ignores_the_opening_press_times_out_and_cancels() {
	let _lock = super::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let b = |i: usize| PHYS[i].1;
	init(None, None, None, None);
	capture_begin(Cap::Pad(0, 1));
	capture_poll(&[], b(3));
	assert!(capturing(), "the opening press (first poll) never binds");
	capture_poll(&[], b(3));
	assert!(!capturing() && capture_ended());
	assert_eq!(binds().1[0], [Some(0), Some(3)], "slot 2 took Y");
	capture_begin(Cap::Pad(0, 0));
	for _ in 0..CAP_FIELDS {
		capture_poll(&[], 0);
	}
	assert!(capturing());
	capture_poll(&[], 0);
	assert!(!capturing() && capture_ended(), "timeout ends it");
	assert_eq!(binds().1[0], [Some(0), Some(3)], "no change on timeout");
	capture_begin(Cap::Pad(0, 0));
	capture_poll(&[Input::Key(K::Escape)], 0);
	assert!(!capturing() && binds().1[0][0] == Some(0), "Esc cancels");
	capture_begin(Cap::Pad(16, 0));
	capture_poll(&[], 0);
	capture_poll(&[], b(1));
	assert_eq!(binds().1[16], [None, None], "a stick direction takes no button");
}

#[test]
fn button_names_follow_the_connected_controller() {
	assert_eq!((pad_shown(0, false), pad_shown(0, true)), ("A", "Cross"));
	assert_eq!((pad_shown(15, false), pad_shown(15, true)), ("RT", "R2"));
	assert_eq!((pad_shown(6, false), pad_shown(6, true)), ("Start", "Options"));
}

#[test]
fn the_consumed_aim_button_loses_every_output_it_alone_gives() {
	// A table with Aim (Pc 0) on RT, Crouch R2 (bit 2) on RT and LB, Jump on RT too: consuming takes every output of RT.
	static A: [ActionDef; 3] = [
		ActionDef { id: "jump", section: 0, layer: Layer::Game, output: Output::Pad(0x40), defaults: [None; 2], keep_one: false, pad_ok: true, pad_defaults: [Some(15), None], pad_follows: 0 },
		ActionDef { id: "crouch2", section: 0, layer: Layer::Game, output: Output::Pad(2), defaults: [None; 2], keep_one: false, pad_ok: true, pad_defaults: [Some(15), Some(4)], pad_follows: 0 },
		ActionDef { id: "aim", section: 0, layer: Layer::Game, output: Output::Pc(0), defaults: [None; 2], keep_one: false, pad_ok: true, pad_defaults: [Some(15), None], pad_follows: 2 },
	];
	static T: Table = Table { actions: &A, menu: &[] };
	let b = Binds::defaults(&T);
	let o = eval_pad(&T, &b, PHYS[15].1, Some(0));
	assert_eq!((o.buttons, o.free, o.pc), (0x42, 0, 1), "RT: Jump and R2 both go to the aim");
	let o = eval_pad(&T, &b, PHYS[15].1 | PHYS[4].1, Some(0));
	assert_eq!((o.buttons, o.free), (0x42, 0x02), "R2 also from LB: the aim does not take that");
	assert_eq!(eval_pad(&T, &b, PHYS[15].1, None).free, 0x42, "no consume rule: nothing taken");
}

#[test]
fn aim_on_lt_takes_l2_with_it_while_the_gate_is_on() {
	// R&C1's table: Aim moved to LT, Look (L2) still on LT: the consumed LT gives no L2.
	static A: [ActionDef; 2] = [
		ActionDef { id: "look2", section: 0, layer: Layer::Game, output: Output::Pad(1), defaults: [None; 2], keep_one: false, pad_ok: true, pad_defaults: [Some(14), None], pad_follows: 0 },
		ActionDef { id: "aim", section: 0, layer: Layer::Game, output: Output::Pc(0), defaults: [None; 2], keep_one: false, pad_ok: true, pad_defaults: [Some(14), None], pad_follows: 2 },
	];
	static T: Table = Table { actions: &A, menu: &[] };
	let o = eval_pad(&T, &Binds::defaults(&T), PHYS[14].1, Some(0));
	assert_eq!((o.buttons, o.free), (1, 0));
}
