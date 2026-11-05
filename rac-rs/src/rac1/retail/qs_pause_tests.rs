//! Tests of qs_pause.rs: the decision, the pad decoding and a frozen field's write / put back on a RAM image (Kerwan, PAL).
use super::*;

/// Kerwan (level 3): the update mask, the camera word beside it, and the Quick Select fade byte (HUD element array 0x17dcd0 +
/// element 3 * 0x90 + 0x70, wheel_mouse.rs `ELEM_PAL`).
const LV: u32 = 3;
const MASK_K: u32 = 0x16c0e0;
const FADE_K: u32 = 0x17dcd0 + 3 * 0x90 + 0x70;

/// The 18 pad bytes `Machine::set_pad` stores for `buttons`.
fn pad(buttons: u16) -> Option<[u8; 18]> {
	let b = !buttons;
	let mut r = [0u8; 18];
	r[0] = (b >> 8) as u8;
	r[1] = b as u8;
	Some(r)
}

/// A Kerwan RAM image in play: mask 0xf, camera word 0, game mode 0, the ring's fade byte `fade`.
fn ram(fade: u8) -> Mem {
	let mut m = Mem::new();
	m.wr32(MASK_K, PLAY);
	m.wr32(MASK_K + 0xc, 0);
	m.wr32(GAME_MODE, 0);
	m.wr8(FADE_K, fade);
	m
}

#[test]
fn freezes_only_with_the_row_on_the_ring_up_and_triangle_held_in_play() {
	let all = Seen { on: true, mode: 0, mask: PLAY, cam: 0, ring: true, triangle: true };
	assert!(wants(&all));
	assert!(!wants(&Seen { on: false, ..all }), "row Off = retail");
	assert!(!wants(&Seen { ring: false, ..all }), "ring not up");
	assert!(!wants(&Seen { triangle: false, ..all }), "the release field runs as retail's");
	assert!(!wants(&Seen { mode: 3, ..all }), "pause menu / other modes");
	assert!(!wants(&Seen { mask: 0x10, ..all }), "retail's debug step mode");
	assert!(!wants(&Seen { mask: 0, ..all }), "already 0");
	assert!(!wants(&Seen { cam: 2, ..all }), "another camera choice");
}

#[test]
fn reads_triangle_from_the_inverted_pad_bytes() {
	assert!(triangle(pad(0x10)));
	assert!(triangle(pad(0x10 | 0x40 | 0x800)));
	assert!(!triangle(pad(0)));
	assert!(!triangle(pad(0x20 | 0x1000)));
	assert!(!triangle(None));
}

#[test]
fn a_frozen_field_writes_0_and_gets_0xf_back() {
	let mut m = ram(1);
	assert_eq!(start(&mut m, LV, true, pad(0x10)), Some(MASK_K));
	assert_eq!(m.rd32(MASK_K), 0, "the world steps are off for this field");
	end(&mut m, MASK_K);
	assert_eq!(m.rd32(MASK_K), PLAY, "back after the field");
}

#[test]
fn nothing_is_written_when_it_does_not_freeze() {
	for (on, fade, buttons, lv) in [(false, 8, 0x10, LV), (true, 0, 0x10, LV), (true, 8, 0, LV), (true, 8, 0x10, 19), (true, 8, 0x10, 0)] {
		let mut m = ram(fade);
		assert_eq!(start(&mut m, lv, on, pad(buttons)), None, "on {on} fade {fade} buttons {buttons:#x} level {lv}");
		assert_eq!(m.rd32(MASK_K), PLAY);
	}
}

#[test]
fn a_mask_the_game_wrote_during_the_field_is_kept() {
	assert_eq!(put_back(0), Some(PLAY));
	assert_eq!(put_back(0x10), None);
	assert_eq!(put_back(PLAY), None);
	let mut m = ram(1);
	start(&mut m, LV, true, pad(0x10));
	m.wr32(MASK_K, 0x10);
	end(&mut m, MASK_K);
	assert_eq!(m.rd32(MASK_K), 0x10);
}

#[test]
fn the_mask_table_is_novalis_0x16c460_with_the_camera_globals_shift() {
	// modcam.rs `LVS` shifts (the same on both discs).
	const SHIFT: [i32; 19] = [-0x400, 0, 0x180, -0x380, -0x300, 0, 0x300, -0x400, 0x380, -0x300, 0, 0x580, 0, -0x180, 0x300, 0x180, -0x80, 0x480, 0x580];
	for (l, (&a, &s)) in MASK.iter().zip(SHIFT.iter()).enumerate() {
		assert_eq!(a as i32, 0x16c460 + s, "level {l}");
	}
}
