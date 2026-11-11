use super::*;
use crate::retail::modcam_turn::{turn, Gains};
use crate::retail::pcctl;

fn g(lr: f32, ud: f32) -> Gains {
	let (mouse_lr, mouse_ud) = signs(lr, ud);
	Gains { mouse: 0.003, sens: 1.0, mouse_k: 1.0, rot: 0.03, tilt: 0.02, stick_k: 1.0, lr, ud, mouse_lr, mouse_ud }
}

fn set(ud: u8, lr: u8) {
	MODE[0].store(ud, Ordering::Relaxed);
	MODE[1].store(lr, Ordering::Relaxed);
}

#[test]
fn the_three_values_give_the_mouse_sign_with_retail_reversed_on_and_off() {
	let _g = crate::retail::pcopts::globals_guard();
	for retail in [1.0f32, -1.0] {
		assert_eq!(sign_for(LIKE, retail), retail, "like camera follows retail");
		assert_eq!(sign_for(NORMAL, retail), 1.0);
		assert_eq!(sign_for(INVERTED, retail), -1.0);
	}
	for (retail_inv, m, want) in [(false, LIKE, false), (true, LIKE, true), (false, NORMAL, false), (true, NORMAL, false), (false, INVERTED, true), (true, INVERTED, true)] {
		set(m, LIKE);
		assert_eq!(ud_inverted(retail_inv), want, "ship mouse {m} retail {retail_inv}");
	}
	set(LIKE, LIKE);
}

#[test]
fn like_camera_is_todays_turn_bit_for_bit_and_only_the_mouse_moves() {
	let _g = crate::retail::pcopts::globals_guard();
	let (look, s) = ([13, -9], [0.4, -0.7]);
	for (lr, ud) in [(1.0f32, 1.0f32), (-1.0, 1.0), (1.0, -1.0), (-1.0, -1.0)] {
		set(LIKE, LIKE);
		let like = g(lr, ud);
		assert_eq!((like.mouse_lr, like.mouse_ud), (lr, ud));
		// The formula before the change.
		let old = ((-(look[0] as f32) * like.mouse * like.sens * like.mouse_k + s[0] * like.rot * like.stick_k) * lr, (look[1] as f32 * like.mouse * like.sens * like.mouse_k - s[1] * like.tilt * like.stick_k) * ud);
		assert_eq!(turn(look, s, &like), old, "default = today");
		// Inverted on both axes: the mouse share is the opposite of Normal's, the stick share does not change.
		set(INVERTED, INVERTED);
		let (inv, st_inv) = (turn(look, [0.0; 2], &g(lr, ud)), turn([0, 0], s, &g(lr, ud)));
		set(NORMAL, NORMAL);
		let (nor, st_nor) = (turn(look, [0.0; 2], &g(lr, ud)), turn([0, 0], s, &g(lr, ud)));
		assert_eq!(inv, (-nor.0, -nor.1), "inverted = normal flipped (retail {lr} {ud})");
		assert_eq!(st_inv, st_nor, "the stick follows retail's options only");
		// Normal = retail Normal's mouse turn whatever retail says.
		let base = turn(look, [0.0; 2], &g(1.0, 1.0));
		assert_eq!(nor, base);
	}
	set(LIKE, LIKE);
}

#[test]
fn axes_are_independent() {
	let _g = crate::retail::pcopts::globals_guard();
	set(INVERTED, LIKE);
	assert_eq!(signs(1.0, 1.0), (1.0, -1.0));
	set(LIKE, NORMAL);
	assert_eq!(signs(-1.0, -1.0), (1.0, -1.0));
	set(LIKE, LIKE);
}

#[test]
fn the_file_keys_round_trip_and_like_camera_writes_nothing() {
	let _g = crate::retail::pcopts::globals_guard();
	init(&PcOptions::parse("mouse_ud=inverted\nmouse_lr=normal\n"));
	assert_eq!((mode(0), mode(1)), (INVERTED, NORMAL));
	init(&PcOptions::parse(""));
	assert_eq!((mode(0), mode(1)), (LIKE, LIKE));
	let mut o = PcOptions::parse("");
	let mut mem = Mem::new();
	assert!(set_from_ui(&mut mem, &mut o, 0, INVERTED));
	assert_eq!(o.extra_get("mouse_ud"), Some("inverted"));
	assert!(!set_from_ui(&mut mem, &mut o, 0, INVERTED), "no change, no save");
	assert!(set_from_ui(&mut mem, &mut o, 0, LIKE));
	assert_eq!(o.extra_get("mouse_ud"), None, "the default removes its key");
	assert_eq!(o.to_text(), PcOptions::parse("").to_text());
	init(&PcOptions::parse(""));
}

#[test]
fn the_classic_rows_cycle_through_the_byte_and_show_their_texts() {
	let _g = crate::retail::pcopts::globals_guard();
	init(&PcOptions::parse(""));
	let mut mem = pcctl::test_page(5);
	// Not built through build_page here: the page's magic is set by test_page.
	assert!(pcctl::controls_built(&mut mem));
	let mut o = PcOptions::parse("");
	assert!(!sync(&mut mem, &mut o), "zero bytes = Like camera = nothing to save");
	mem.wr8(UD_B, INVERTED);
	assert!(sync(&mut mem, &mut o));
	assert_eq!(mode(0), INVERTED);
	assert_eq!(o.extra_get("mouse_ud"), Some("inverted"));
	assert!(!sync(&mut mem, &mut o));
	// Rows 5 and 6 of the PC section (after its four actions) are the two mouse rows.
	for (r, a) in [(5u32, 0usize), (6, 1)] {
		for l in 0..5 {
			assert_eq!(pcctl::text(pcctl::ID_VALUE + r, l, 5, 0, false), value_text(ID_V0 + mode(a) as u32, l), "value row {r} lang {l}");
			assert_eq!(pcctl::text(0x7f40 + r, l, 5, 0, false), label(a, l), "label row {r} lang {l}");
		}
	}
	for l in 0..5 {
		let (a, b, c) = (value_text(ID_V0, l), value_text(ID_V0 + 1, l), value_text(ID_V0 + 2, l));
		assert!(a != b && b != c && [&a, &b, &c].iter().all(|v| !v.is_empty() && v.len() <= pcctl::VALUE_MAX));
	}
	init(&PcOptions::parse(""));
}
