//! Tests of the input glue: the wheel notch latch and the Tab / Delete notes of the Controls page.

use super::*;
use rac_engine::input::kbm::{Input, Wheel};
use winit::keyboard::KeyCode as K;

#[test]
fn a_wheel_notch_latches_where_the_readers_look() {
	let _g = crate::retail::pcopts::globals_guard();
	crate::kbm_actions::install();
	kbm::init(None, None, None, None);
	for (up, w, slot) in [(true, Wheel::Up, 3), (false, Wheel::Down, 4)] {
		let i = wheel_index(up);
		assert_eq!(MouseTaps::INPUTS[i], Input::Wheel(w));
		// `press_mouse` index i lands in extra_taps[i - 2], which `play_pad` and `capture_step` read.
		assert_eq!(i - 2, slot);
		let mut extra = [0u8; 5];
		extra[i - 2] = TAP_FIELDS;
		let mut hold = (Vec::new(), 0);
		kbm::capture_begin(kbm::Cap::Kbm(0, 0));
		Retail::capture_step(&HashMap::new(), &MouseTaps([0; 2]), &extra, &mut hold);
		assert!(!kbm::capturing(), "the notch answered the capture");
	}
}

#[test]
fn delete_or_tab_that_answers_a_capture_is_not_a_page_command() {
	let mut taps = HashMap::new();
	taps.insert(K::Delete, TAP_FIELDS);
	taps.insert(K::Tab, TAP_FIELDS - 1);
	assert_eq!(page_notes(&taps, false, &[]), vec![K::Delete]);
	assert!(page_notes(&taps, true, &[]).is_empty(), "a capture ran this field");
	assert!(page_notes(&taps, false, &[K::Delete]).is_empty(), "still held from the answer");
}
