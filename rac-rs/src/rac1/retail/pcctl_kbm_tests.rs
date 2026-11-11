//! Tests of the keyboard + mouse half of the Controls page: capture of slot 1 / slot 2, clear, the Pause refusal, mouse and wheel
//! capture, Esc, the Square / Tab / Delete request, the value cells and the prompt names.

use super::super::pcctl::{self, text, CBASE, ID_VALUE};
use super::*;
use rac_engine::input::kbm::{Cap, Input, Mouse, Wheel};
use winit::keyboard::KeyCode as K;

fn idx(id: &str) -> usize {
	ka::install();
	kbm::table().index(id).unwrap()
}

fn slots(id: &str) -> [Option<Input>; 2] {
	kbm::binds().0[idx(id)]
}

/// A built page on section 0 with default bindings.
fn page() -> ee_oracle::mem::Mem {
	kbm::init(None, None, None, None);
	pcctl::test_page(0)
}

/// Presses the row `r` of the page (the retail item flips its byte) and lets `sync` start the capture.
fn press(mem: &mut ee_oracle::mem::Mem, r: u32) {
	mem.wr8(CBASE + r, 1);
	pcctl::sync(mem);
}

#[test]
fn rows_are_the_table_sections() {
	let _g = crate::retail::pcopts::globals_guard();
	ka::install();
	assert_eq!(rows(0).len(), 6);
	assert_eq!(rows(5), vec![idx("aim"), idx("zoom_in"), idx("zoom_out"), idx("ship_fire")]);
	assert_eq!((0..NSEC).map(|s| rows(s).len()).sum::<usize>(), kbm::table().actions.len());
}

#[test]
fn slot_one_and_slot_two_capture() {
	let _g = crate::retail::pcopts::globals_guard();
	let mut mem = page();
	// Row 1 = Jump: Cross / Enter / click = slot 1; a side button binds it.
	press(&mut mem, 1);
	assert!(kbm::capturing());
	kbm::capture_poll(&[Input::Mouse(Mouse::Back)], 0);
	pcctl::sync(&mut mem);
	assert_eq!(slots("jump"), [Some(Input::Mouse(Mouse::Back)), None]);
	assert_eq!(mem.rd8(CBASE + 1), 0);
	assert_eq!(text(ID_VALUE + 1, 0, 0, 0, false), b"Mouse 4");
	// Square / Tab asked for slot 2: a wheel notch binds it.
	want_slot2();
	press(&mut mem, 1);
	kbm::capture_poll(&[Input::Wheel(Wheel::Down)], 0);
	pcctl::sync(&mut mem);
	assert_eq!(slots("jump"), [Some(Input::Mouse(Mouse::Back)), Some(Input::Wheel(Wheel::Down))]);
	assert_eq!(text(ID_VALUE + 1, 0, 0, 0, false), b"Mouse 4 / Wheel down".to_vec());
	// The next plain press is slot 1 again.
	assert_eq!(take_slot(), 0);
	// A left click (mouse tap) is capturable, Esc cancels without a change.
	press(&mut mem, 2);
	kbm::capture_poll(&[Input::Mouse(Mouse::Left)], 0);
	pcctl::sync(&mut mem);
	assert_eq!(slots("wrench")[0], Some(Input::Mouse(Mouse::Left)));
	press(&mut mem, 3);
	let before = kbm::binds();
	kbm::capture_poll(&[Input::Key(K::Escape)], 0);
	pcctl::sync(&mut mem);
	assert!(!kbm::capturing() && kbm::binds() == before);
	kbm::init(None, None, None, None);
}

#[test]
fn a_capture_only_takes_inputs_that_went_down_after_it_began() {
	let _g = crate::retail::pcopts::globals_guard();
	let mut mem = page();
	press(&mut mem, 1);
	// Nothing fresh (the press that opened the capture is already past its first poll): it keeps waiting.
	for _ in 0..5 {
		kbm::capture_poll(&[], 0);
	}
	assert!(kbm::capturing() && slots("jump")[0] == Some(Input::Key(K::Space)));
	// 400 fields without an input end it.
	for _ in 0..kbm::CAP_FIELDS + 1 {
		kbm::capture_poll(&[], 0);
	}
	assert!(!kbm::capturing());
	pcctl::sync(&mut mem);
	assert!(matches!(kbm::table().actions.len(), 28) && slots("jump")[0] == Some(Input::Key(K::Space)));
	let _ = Cap::Idle;
}

#[test]
fn delete_clears_slot_two_then_slot_one_and_pause_keeps_one() {
	let _g = crate::retail::pcopts::globals_guard();
	kbm::init(None, None, None, None);
	let fire = idx("fire");
	assert_eq!(slots("fire"), [Some(Input::Mouse(Mouse::Left)), Some(Input::Key(K::KeyG))]);
	assert!(clear(fire));
	assert_eq!(slots("fire"), [Some(Input::Mouse(Mouse::Left)), None]);
	assert!(clear(fire));
	assert_eq!(slots("fire"), [None, None]);
	assert!(!clear(fire), "nothing left");
	// Pause (Esc | Enter): slot 2 goes, the last binding stays.
	let pause = idx("pause");
	assert!(clear(pause) && !clear(pause));
	assert_eq!(slots("pause"), [Some(Input::Key(K::Escape)), None]);
	assert_eq!(value(fire), b"-".to_vec());
	kbm::init(None, None, None, None);
}

#[test]
fn square_tab_and_delete_make_requests() {
	let _g = crate::retail::pcopts::globals_guard();
	assert_eq!(request(0), Edit::Nothing);
	assert_eq!(request(0x80), Edit::Capture2, "the pad's Square, on its press");
	assert_eq!(request(0x80), Edit::Nothing, "held: no second request");
	assert_eq!(request(0), Edit::Nothing);
	note_key(K::Tab);
	assert_eq!(request(0), Edit::Capture2);
	note_key(K::Delete);
	note_key(K::Tab);
	assert_eq!(request(0), Edit::Clear, "Delete wins");
	note_key(K::KeyA);
	assert_eq!(request(0), Edit::Nothing);
}

#[test]
fn value_cells_show_both_slots_and_shared_inputs() {
	let _g = crate::retail::pcopts::globals_guard();
	kbm::init(None, None, None, None);
	assert_eq!(value(idx("fire")), b"LMB / G".to_vec());
	assert_eq!(value(idx("jump")), b"Space".to_vec());
	assert_eq!(value(idx("quick_select")), b"T / MMB".to_vec());
	kbm::assign(idx("wrench"), 0, Input::Key(K::Space));
	assert_eq!(value(idx("jump")), b"Space x2".to_vec());
	assert_eq!(value(idx("wrench")), b"Space x2".to_vec());
	kbm::init(None, None, None, None);
	kbm::assign(idx("map"), 1, Input::Wheel(Wheel::Down));
	kbm::clear(idx("map"), 0);
	assert_eq!(value(idx("map")), b"- / Wheel down x2".to_vec());
	// Every language's labels and texts are short font bytes.
	for l in 0..5 {
		for a in 0..kbm::table().actions.len() {
			assert!(value(a).len() <= VALUE_MAX && !label(a, l).is_empty());
		}
		for t in [PRESS[l], HINT[l]].into_iter().chain(SECTION_NAMES.iter().map(|s| s[l])) {
			assert!(t.len() < 0x3f && t.iter().all(|&b| (0x20..0x7f).contains(&b) || (0x80..=0xb0).contains(&b)) && !t.contains(&b'%'), "{:?}", String::from_utf8_lossy(t));
		}
	}
	kbm::init(None, None, None, None);
}

#[test]
fn prompts_name_the_first_bound_input() {
	use super::super::prompt_names as pn;
	let _g = crate::retail::pcopts::globals_guard();
	kbm::init(None, None, None, None);
	assert_eq!((pn::glyph_name(0x20).unwrap(), pn::glyph_name(0x40).unwrap(), pn::glyph_name(0x10).unwrap()), ("LMB".into(), "Space".into(), "T".into()));
	assert!(pn::jump_is_space());
	// Fire moved to a side button; Quick Select's slot 1 empty falls to slot 2 (the middle button).
	kbm::assign(idx("fire"), 0, Input::Mouse(Mouse::Back));
	kbm::clear(idx("quick_select"), 0);
	assert_eq!((pn::glyph_name(0x20).unwrap(), pn::glyph_name(0x10).unwrap()), ("Mouse 4".into(), "MMB".into()));
	kbm::assign(idx("jump"), 0, Input::Wheel(Wheel::Up));
	assert_eq!((pn::glyph_name(0x40).unwrap(), pn::jump_is_space()), ("Wheel up".into(), false));
	kbm::clear(idx("quick_select"), 1);
	assert_eq!(pn::glyph_name(0x10).unwrap(), "None");
	kbm::init(None, None, None, None);
	assert_eq!((pn::stick_name(true), pn::stick_name(false)), ("WASD".to_string(), "Q/E/Z/X".to_string()));
}
