//! Tests of `fp_look.rs`: the first-person mouse turn maths (directions, limits, rows as retail rebuilds them, no frame-rate effect).

use super::*;

const Z: [f32; 3] = [0.0, 0.0, 1.0];
const X: [f32; 3] = [1.0, 0.0, 0.0];

/// The mouse's (yaw, pitch) at the default gains (Mouse speed 1.0, sensitivity 1.0, Normal on both axes).
fn mouse(dx: i16, dy: i16) -> (f32, f32) {
	let g = crate::retail::modcam_turn::Gains { mouse: MOUSE, sens: 1.0, mouse_k: 1.0, rot: 0.0, tilt: 0.0, stick_k: 0.0, lr: 1.0, ud: 1.0, mouse_lr: 1.0, mouse_ud: 1.0 };
	crate::retail::modcam_turn::turn([dx, dy], [0.0; 2], &g)
}

fn close(a: [f32; 3], b: [f32; 3], eps: f32) -> bool {
	(0..3).all(|i| (a[i] - b[i]).abs() <= eps)
}

#[test]
fn mouse_right_turns_right_and_up_looks_up() {
	let (y, p) = mouse(100, 0);
	let (r, w) = turn_view(X, Z, 0.0, y, p).unwrap();
	assert!(r[0][1] < -0.25 && r[0][2].abs() < 1e-6, "right: forward swings to -y (the view's right with z up): {:?}", r[0]);
	assert_eq!(w, 0.0, "no pitch counts: the pitch word stays");
	let (y, p) = mouse(0, -100);
	let (r, w) = turn_view(X, Z, 0.0, y, p).unwrap();
	assert!(r[0][2] > 0.25 && r[0][1].abs() < 1e-6, "mouse up looks up: {:?}", r[0]);
	assert!(w < 0.0 && (w + 0.3).abs() < 1e-5, "retail's pitch word: + = down, follows the turn: {w}");
}

#[test]
fn rows_are_rebuilt_as_retail_does() {
	let (y, p) = mouse(37, -55);
	let (r, _) = turn_view(X, Z, 0.1, y, p).unwrap();
	let left = unit(cross3(Z, r[0])).unwrap();
	assert!(close(r[1], left, 1e-6), "row 1 = unit(up x fwd)");
	assert!(close(r[2], cross3(r[0], r[1]), 1e-6), "row 2 = fwd x row 1");
	for i in 0..3 {
		assert!((len3(r[i]) - 1.0).abs() < 1e-5, "row {i} is unit");
	}
	assert!(dot3(r[0], r[1]).abs() < 1e-6 && r[1][2].abs() < 1e-6, "no roll on flat ground");
}

#[test]
fn pitch_stops_at_retails_hard_limit() {
	let (y, p) = mouse(0, -2000);
	let (r, w) = turn_view(X, Z, 0.0, y, p).unwrap();
	assert_eq!(w, -PITCH_MAX, "up: stops at -84 deg");
	assert!((r[0][2] - PITCH_MAX.fsin()).abs() < 1e-5, "forward at 84 deg up: {:?}", r[0]);
	let (_, w) = turn_view(r[0], Z, w, 0.0, mouse(0, -50).1).unwrap();
	assert_eq!(w, -PITCH_MAX, "further up does nothing");
	let (_, w2) = turn_view(r[0], Z, w, 0.0, mouse(0, 50).1).unwrap();
	assert!(w2 > w, "down again moves at once");
	let (_, w) = turn_view(X, Z, 0.0, 0.0, mouse(0, 2000).1).unwrap();
	assert_eq!(w, PITCH_MAX, "down: stops at +84 deg");
	// A word already past the limit (not written by retail; defensive) is not pulled back by a turn toward it.
	let (_, w) = turn_view(X, Z, 1.5, 0.0, 0.01).unwrap();
	assert_eq!(w, 1.5);
}

#[test]
fn yaw_turns_about_the_heros_up_on_a_magnetic_wall() {
	let up = [0.55, -0.10, 0.83]; // Batalia fullgame1 0479 (camera.md, magnetic ground)
	let u = unit(up).unwrap();
	let fwd = unit(cross3([0.0, 1.0, 0.0], u)).unwrap();
	let (y, _) = mouse(80, 0);
	let (r, w) = turn_view(fwd, up, 0.0, y, 0.0).unwrap();
	assert!((dot3(r[0], u) - dot3(fwd, u)).abs() < 1e-5, "the angle to the hero's up is kept");
	assert!(dot3(r[0], fwd) < 0.98, "and it turned");
	assert_eq!(w, 0.0);
}

#[test]
fn the_turn_does_not_depend_on_how_counts_are_split() {
	// 60 counts in one field or 6 fields of 10 (a fast or a slow display): the same view within float noise.
	let (y, p) = mouse(60, -30);
	let (a, wa) = turn_view(X, Z, 0.0, y, p).unwrap();
	let (mut f, mut w) = (X, 0.0);
	for _ in 0..6 {
		let (y, p) = mouse(10, -5);
		let (r, ww) = turn_view(f, Z, w, y, p).unwrap();
		(f, w) = (r[0], ww);
	}
	assert!(close(a[0], f, 2e-5), "{:?} vs {:?}", a[0], f);
	assert!((wa - w).abs() < 2e-5);
}

#[test]
fn degenerate_inputs_write_nothing() {
	assert!(turn_view(X, [0.0; 3], 0.0, 0.1, 0.0).is_none(), "no up vector");
	assert!(turn_view([0.0; 3], Z, 0.0, 0.1, 0.0).is_none(), "no forward row");
	assert!(turn_view(X, [f32::NAN, 0.0, 1.0], 0.0, 0.1, 0.0).is_none());
}

#[test]
fn a_zero_turn_keeps_the_forward_row() {
	let f = unit([0.6, -0.3, 0.2]).unwrap();
	let (r, w) = turn_view(f, Z, -0.2, 0.0, 0.0).unwrap();
	assert!(close(r[0], f, 1e-7) && w == -0.2);
}
