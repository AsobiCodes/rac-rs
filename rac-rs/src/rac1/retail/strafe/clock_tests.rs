//! `draw_clock` (strafe.rs): the drawn run phase across forward <-> reversed switches of a settled run clip.

use super::*;

/// `draw_clock` keeps process-wide state: the tests of this file run one at a time.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// A 23-frame run clip stepping 0.8 frames per field, reversed for fields 20..60: the drawn phase steps about 0.8 frames per field
/// in the drawn direction at every field (no jump at the switches), and 80 fields after the last switch it is retail's again.
#[test]
fn switch_carries_phase_and_eases_back() {
	let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
	const N: usize = 23;
	let mut prop = crate::anim::PropAnim::new(4);
	let mut seen = Vec::new();
	let mut retail = 0.0f32;
	for f in 0..140 {
		let rev = (20..60).contains(&f);
		let c = &mut prop.clock;
		(c.old_seq, c.new_seq, c.cur, c.p) = (4, 4, retail.floor() as i32, retail.fract());
		c.next = (c.cur + 1) % N as i32;
		let (_, jumped) = draw_clock(c, N, rev);
		assert!(!jumped, "field {f}");
		seen.push((c.cur as f32 + c.p, rev, retail));
		retail = (retail + 0.8) % N as f32;
	}
	for (f, w) in seen.windows(2).enumerate() {
		let d = (w[1].0 - w[0].0 + N as f32 / 2.0).rem_euclid(N as f32) - N as f32 / 2.0;
		let along = if w[1].1 { -d } else { d };
		assert!((0.6..=1.0).contains(&along), "field {}: drawn step {d:.3} (rev {})", f + 1, w[1].1);
	}
	let (drawn, _, r) = seen[139];
	assert!((drawn - r).abs() < 1e-4, "drawn {drawn} retail {r}");
}

/// A forward <-> backward flip while the clock fades into the run cycle swaps the fade's target frame: reported as a jump (the smoothing
/// cuts); no flip, or a flip of a clip that is not the run cycle, is not.
#[test]
fn a_flip_during_a_fade_is_a_jump() {
	let _one = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
	const N: usize = 23;
	let fade = |seq: u8, rev: bool| {
		let mut prop = crate::anim::PropAnim::new(seq as usize);
		let c = &mut prop.clock;
		(c.old_seq, c.new_seq, c.cur, c.next, c.p) = (0xff, seq, 2, 5, 0.0);
		draw_clock(c, N, rev).1
	};
	assert!(!fade(4, false) && !fade(4, false), "no flip");
	assert!(fade(4, true), "flip to backward during a fade into the run cycle");
	assert!(!fade(4, true), "still backward: no new jump");
	assert!(fade(4, false), "flip back");
	assert!(!fade(9, false) && !fade(9, true) && !fade(9, true), "not the run cycle: the pose is not affected");
	fade(9, false); // leave the shared state as a fresh one (forward) for the other test
}
