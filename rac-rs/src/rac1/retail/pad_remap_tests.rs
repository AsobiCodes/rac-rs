//! The equivalence oracle of the controller remap (v0.5.1 pad lane): today's `pad_buttons(phys, map)` + `apply_r2` (v0.5.1 test build 1)
//! are kept here as a TEST-ONLY copy. With the R&C1 table, the default controller and converted old `pad=` files give the same PS2
//! buttons and the same aim for every single button and every pair of the 16, with the aim gate on and off, for every body form; the
//! Menu context reads the standard layout whatever the remap. ONE expected difference (decided 2026-11-05, review S2): while the aim
//! has its button, EVERY output of that button is taken (test build 1 took only R2): the oracle removes exactly the outputs the Aim's
//! old button alone gives (not what another held button gives, not the keyboard's) from test build 1's result, and asserts everything
//! else equal.

use super::prompts::{apply_r2, pad_actions, PadRoute};
use crate::kbm_actions;
use rac_engine::input::binds::{pad_buttons, pad_from_text, DEFAULT_PAD, ID, NP, PHYS};
use rac_engine::input::kbm;

fn masks() -> Vec<u32> {
	let mut v = vec![0u32];
	for a in 0..NP {
		v.push(PHYS[a].1);
		for b in a + 1..NP {
			v.push(PHYS[a].1 | PHYS[b].1);
		}
	}
	v
}

fn texts() -> Vec<Option<String>> {
	let mut v: Vec<Option<String>> = vec![None];
	v.extend(["cross:b", "circle:a,cross:b", "l2:lb,l1:lt", "start:back,select:start", "r2:a", "r2:lb,cross:rt", "r2:rb,cross:rb", "up:ddown,down:dup"].iter().map(|s| Some(s.to_string())));
	let mut x = 0x9e37_79b9u32;
	let mut next = move || {
		x ^= x << 13;
		x ^= x >> 17;
		x ^= x << 5;
		x
	};
	for _ in 0..40 {
		let n = 1 + next() as usize % 6;
		let e: Vec<String> = (0..n).map(|_| format!("{}:{}", ID[next() as usize % NP], PHYS[next() as usize % NP].0)).collect();
		v.push(Some(e.join(",")));
	}
	v
}

/// Test build 1's `apply_r2` + `r2_route`: (aim down, buttons after).
fn old_route(buttons: u16, held: u8, gameplay: bool, ratchet: bool, aim_button: bool) -> (bool, u16) {
	let pad_r2 = held & 1 != 0;
	let strip = gameplay && aim_button && ratchet;
	let (aim_down, to_game) = (strip && pad_r2, pad_r2 && !strip);
	let mut b = buttons;
	if held & 3 == 1 && !to_game {
		b &= !2;
	}
	(aim_down, b)
}

#[test]
fn defaults_and_converted_old_pad_files_give_todays_buttons_and_aim_except_the_aims_other_outputs() {
	let _g = super::pcopts::globals_guard();
	kbm_actions::install();
	for text in texts() {
		kbm::init(None, None, text.as_deref(), None);
		let map = text.as_deref().map_or(DEFAULT_PAD, pad_from_text);
		let p_aim = PHYS[map[7] as usize % NP].1; // the old R2 button = the converted Aim's slot 1
		for m in masks() {
			let ob = pad_buttons(m, &map);
			let (nb, route) = pad_actions(m, false);
			assert_eq!(nb, ob, "{text:?} mask {m:#x}: buttons; map {map:?}");
			assert_eq!(route.aim, m & p_aim != 0, "{text:?} mask {m:#x}: the aim is held exactly with the old R2 button");
			for kb in [false, true] {
				let old_held = (ob & 2 != 0) as u8 | (kb as u8) << 1;
				let (ob2, nb2) = (ob | if kb { 2 } else { 0 }, nb | if kb { 2 } else { 0 });
				let route = PadRoute { kb: if kb { 2 } else { 0 }, ..route };
				for (gameplay, ratchet, aim_button) in (0..8).map(|i| (i & 1 != 0, i & 2 != 0, i & 4 != 0)) {
					let (want_aim, want_b) = old_route(ob2, old_held, gameplay, ratchet, aim_button);
					// The expected difference: the other outputs of the aim's button, only while it is the aim's.
					let alone = pad_buttons(p_aim, &map) & !pad_buttons(m & !p_aim, &map) & !route.kb;
					let want_b = if want_aim { want_b & !alone } else { want_b };
					let mut b = nb2;
					let aim = apply_r2(&mut b, route, gameplay, ratchet, aim_button);
					assert_eq!((aim, b), (want_aim, want_b), "{text:?} mask {m:#x} kb {kb} gameplay {gameplay} ratchet {ratchet} aim_button {aim_button}");
				}
			}
		}
	}
	kbm::init(None, None, None, None);
}

#[test]
fn menus_read_the_standard_layout_whatever_the_remap() {
	let _g = super::pcopts::globals_guard();
	kbm_actions::install();
	for text in texts() {
		kbm::init(None, None, text.as_deref(), None);
		for m in masks() {
			let (b, route) = pad_actions(m, true);
			assert_eq!(b, pad_buttons(m, &DEFAULT_PAD), "{text:?} mask {m:#x}");
			assert_eq!(route.aim, m & PHYS[15].1 != 0, "the standard aim button is RT");
		}
	}
	kbm::init(None, None, None, None);
}

#[test]
fn a_remapped_gamepad_moves_gameplay_and_the_aim_follows_its_own_slot() {
	let _g = super::pcopts::globals_guard();
	kbm_actions::install();
	kbm::init(None, None, None, Some("jump:b|-,fire:a|-,aim:lt|-"));
	// B jumps now (Jump = Cross 0x40), A fires (Circle 0x20), LT aims; the fire button B is free of Fire.
	assert_eq!(pad_actions(PHYS[1].1, false).0, 0x40);
	assert_eq!(pad_actions(PHYS[0].1, false).0, 0x20);
	let (b, route) = pad_actions(PHYS[14].1, false);
	assert_eq!((b, route.aim, route.free), (1, true, 0), "LT: still Look (L2) and the aim; the route takes the L2 with it");
	let mut buttons = b;
	assert!(apply_r2(&mut buttons, route, true, true, true) && buttons == 0, "Aim on LT, L2 still on LT, gate on: no L2");
	let mut buttons = b;
	assert!(!apply_r2(&mut buttons, route, false, true, true) && buttons == 1, "gate off (pause): the L2 is retail's");
	let (_, route) = pad_actions(PHYS[15].1, false);
	assert!(!route.aim, "RT is no longer the aim");
	kbm::init(None, None, None, None);
}
