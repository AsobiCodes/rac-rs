//! The follow maths against anim_pose.rs `mirrored()` on a synthetic symmetric skeleton, the history blend and the capture.

use super::*;
use crate::anim_pose::{mirrored, Part};

fn near(a: &M, b: &M, eps: f32) -> bool {
	a.iter().zip(b).all(|(x, y)| (x - y).abs() < eps)
}

fn quat(axis: [f32; 3], angle: f32) -> [f32; 4] {
	let n = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
	let (s, c) = (angle / 2.0).sin_cos();
	[axis[0] / n * s, axis[1] / n * s, axis[2] / n * s, c]
}

/// `[R(q) S | t]` of a local pose, column-major.
fn part_matrix(p: &Part) -> M {
	let ([x, y, z, w], s, t) = *p;
	let r = [
		[1.0 - 2.0 * (y * y + z * z), 2.0 * (x * y - z * w), 2.0 * (x * z + y * w)],
		[2.0 * (x * y + z * w), 1.0 - 2.0 * (x * x + z * z), 2.0 * (y * z - x * w)],
		[2.0 * (x * z - y * w), 2.0 * (y * z + x * w), 1.0 - 2.0 * (x * x + y * y)],
	];
	let mut m = IDENTITY;
	for c in 0..3 {
		for row in 0..3 {
			m[c * 4 + row] = r[row][c] * s[c];
		}
		m[12 + c] = t[c];
	}
	m
}

fn chain(parent: &M, parts: &[Part]) -> M {
	parts.iter().fold(*parent, |w, p| mul(&w, &part_matrix(p)))
}

/// A twisted, leaning chest, an arm of three joints on the right, the left arm made with `mirrored` (the pose ops' mirror) on every joint.
fn skeleton() -> (M, M, M, Vec<Part>) {
	let chest = chain(&IDENTITY, &[(quat([0.3, -0.2, 1.0], 0.7), [1.0; 3], [2.0, -1.0, 0.5]), (quat([1.0, 0.4, 0.1], 0.35), [1.0; 3], [0.1, 0.0, 1.2])]);
	let arm: Vec<Part> = vec![
		(quat([0.2, 1.0, 0.3], 1.1), [1.0; 3], [0.3, -0.4, 0.9]),
		(quat([1.0, 0.1, -0.5], -0.8), [1.0; 3], [0.0, -0.6, 0.0]),
		(quat([0.0, 0.7, 1.0], 0.6), [1.0; 3], [0.05, -0.5, 0.1]),
	];
	let w_right = chain(&chest, &arm);
	let w_left = chain(&chest, &arm.iter().map(mirrored).collect::<Vec<_>>());
	(chest, w_right, w_left, arm)
}

/// R * m, R = the reflection across the plane through the chest origin with the chest's local Y as normal.
fn reflect(chest: &M, m: &M) -> M {
	mul(&mul(&mul(chest, &[1.0, 0.0, 0.0, 0.0, 0.0, -1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0]), &rigid_inverse(chest)), m)
}

#[test]
fn the_mirrored_follow_is_the_reflection_of_the_source_on_a_leaning_chest() {
	let (chest, w_right, w_left, _) = skeleton();
	// An arbitrary rigid place of the source (the gun) relative to the right hand.
	let rel = chain(&IDENTITY, &[(quat([0.6, -0.3, 0.2], 2.3), [1.0; 3], [0.02, -0.07, 0.11])]);
	let w_src = mul(&w_right, &rel);
	let x = follow_matrix(&w_left, &w_right, &w_src, true);
	let want = reflect(&chest, &w_src);
	assert!(near(&x, &want, 1e-4), "follow {x:?} != reflected source {want:?}");
	assert!(det3(&x) < -0.99 && det3(&w_src) > 0.99, "mirrored = left-handed");
}

#[test]
fn without_the_mirror_the_item_keeps_its_place_relative_to_the_joint() {
	let (_, w_right, w_left, _) = skeleton();
	let rel = chain(&IDENTITY, &[(quat([0.1, 0.3, 1.0], 0.9), [1.0; 3], [0.0, 0.1, 0.2])]);
	let w_src = mul(&w_right, &rel);
	assert!(near(&follow_matrix(&w_right, &w_right, &w_src, false), &w_src, 1e-5), "same joint = the source itself");
	let x = follow_matrix(&w_left, &w_right, &w_src, false);
	assert!(near(&x, &mul(&w_left, &rel), 1e-5) && det3(&x) > 0.99);
}

#[test]
fn a_source_exactly_at_the_hand_gives_the_item_at_the_other_hand_mirrored() {
	let (_, w_right, w_left, _) = skeleton();
	let x = follow_matrix(&w_left, &w_right, &w_right, true);
	assert!(near(&x, &mul(&w_left, &negate_row1(IDENTITY)), 1e-5), "rel = identity: the item is the left hand with its Y flipped");
	assert!((0..3).all(|k| (x[12 + k] - w_left[12 + k]).abs() < 1e-5), "origin on the left hand");
}

#[test]
fn rigid_inverse_and_row_conversion() {
	let (_, w, _, _) = skeleton();
	assert!(near(&mul(&w, &rigid_inverse(&w)), &IDENTITY, 1e-5));
	let rows = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0];
	let m = from_rows(&rows);
	assert_eq!((m[0], m[1], m[2], m[4], m[12], m[13], m[14], m[15]), (1.0, 5.0, 9.0, 2.0, 4.0, 8.0, 12.0, 1.0));
}

fn snap(j: u16, x: f32) -> Snap {
	let mut m = IDENTITY;
	m[12] = x;
	Snap { joints: vec![(j, m)], held: None }
}

#[test]
fn the_history_blends_between_two_ticks_and_drops_without_a_blended_hero() {
	let mut s = State::default();
	s.roll(Some(snap(7, 0.0)), 1, false);
	s.roll(Some(snap(7, 2.0)), 1, true);
	assert_eq!(s.joint(7, Some(0.0)).unwrap()[12], 0.0);
	assert!((s.joint(7, Some(0.25)).unwrap()[12] - 0.5).abs() < 1e-5);
	assert_eq!(s.joint(7, None).unwrap()[12], 2.0, "no blend = the newest tick");
	assert!(s.joint(8, Some(0.5)).is_none());
	s.roll(Some(snap(7, 5.0)), 1, false);
	assert_eq!(s.joint(7, Some(0.5)).unwrap()[12], 5.0, "several ticks in a frame: no history");
	s.roll(None, 1, true);
	assert_eq!(s.joint(7, None).unwrap()[12], 5.0, "no new snapshot keeps the last");
}

#[test]
fn capture_keeps_only_the_named_joints_inside_the_pose() {
	let recs = [FollowRec { item: 1, source: 2, from: 56, to: 76, mirror: true }, FollowRec { item: 3, source: 2, from: 56, to: 200, mirror: false }];
	capture(&recs, 111, |j| [j as f32; 12]);
	capture_held(&[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0], &[1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
	let snap = PENDING.with(|p| p.borrow_mut().take()).expect("a snapshot");
	assert_eq!(snap.joints.iter().map(|j| j.0).collect::<Vec<_>>(), vec![56, 76]);
	assert!(snap.held.is_some());
	capture(&[], 111, |_| [0.0; 12]);
	assert!(PENDING.with(|p| p.borrow().is_none()));
}
