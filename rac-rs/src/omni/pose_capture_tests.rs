//! Tests of the snapshot / hold / debug slot (pose_capture.rs) and of holds in the store (pose.rs).

use super::*;
use crate::omni::cabi::OmniPoseOp;
use crate::omni::pose::{Store, OP_HOLD, TEST_LOCK};

/// Local rotations where joint j has x = j / 200 (distinct per joint).
fn local(shift: f32) -> Vec<[f32; 4]> {
	(0..111).map(|j| [j as f32 / 200.0 + shift, 0.0, 0.0, 1.0]).collect()
}

fn hold(joint: u32, source: u32) -> OmniPoseOp {
	OmniPoseOp { joint, op: OP_HOLD, source, q: [0.0, 0.0, 0.0, 1.0], weight: 1.0, ..OmniPoseOp::default() }
}

// A snapshot is registered at once (id > 0), waits for the renderer, and is read back per joint after `service`.
#[test]
fn a_snapshot_waits_for_the_renderer_and_is_read_back_per_joint() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	clear();
	assert!(!pending());
	let id = request(10, 14);
	assert!(id > 0 && pending());
	assert_eq!(rotation(id, 10), None);
	assert_eq!((covers(id, 10), covers(id, 13), covers(id, 14), covers(id, 9), covers(id + 77, 10)), (Ok(()), Ok(()), Err(ERR_BAD_JOINT), Err(ERR_BAD_JOINT), Err(ERR_RANGE)));
	assert_eq!(service(&local(0.0)[..12]), 0, "a pose that does not reach the joints fulfils nothing");
	assert!(pending());
	assert_eq!(service(&local(0.0)), 1);
	assert!(!pending());
	assert_eq!(rotation(id, 12), Some([12.0 / 200.0, 0.0, 0.0, 1.0]));
	assert_eq!(rotation(id, 14), None);
	assert_eq!(rotation(id, 9), None);
	// captured once: a later pose does not move it
	assert_eq!(service(&local(1.0)), 0);
	assert_eq!(rotation(id, 12), Some([12.0 / 200.0, 0.0, 0.0, 1.0]));
	let b = request(0, 111);
	assert!(b > id);
	clear();
	assert!(!pending() && rotation(id, 12).is_none() && rotation(b, 3).is_none());
}

#[test]
fn the_oldest_snapshots_are_forgotten_beyond_the_cap() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	clear();
	let first = request(0, 1);
	for _ in 0..MAX_SNAPSHOTS {
		request(0, 1);
	}
	assert_eq!(covers(first, 0), Err(ERR_RANGE));
	clear();
}

// A hold with no snapshot freezes the joint at its rotation of the first captured frame; no record before that.
#[test]
fn a_hold_freezes_the_joint_after_the_capture() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	clear();
	let mut s = Store::default();
	assert_eq!(s.set(7, 111, &[hold(20, 0)]), Ok(1));
	assert!(pending());
	assert!(s.records(7).is_empty(), "nothing to apply before the renderer captured");
	service(&local(0.0));
	let r = s.records(7);
	assert_eq!(r.len(), 1);
	assert_eq!((r[0].joint, r[0].rot, r[0].blend, r[0].mirror), (20, [20.0 / 200.0, 0.0, 0.0, 1.0], Some(1.0), Some(crate::anim_pose::SET_ROT)));
	// the animation moves on, the hold does not; renewing it with no snapshot keeps the capture
	service(&local(0.5));
	assert_eq!(s.set(7, 111, &[hold(20, 0)]), Ok(1));
	assert!(!pending(), "a renewed hold asks for no new capture");
	assert_eq!(s.records(7)[0].rot, [20.0 / 200.0, 0.0, 0.0, 1.0]);
	clear();
}

// A hold naming a snapshot uses its joint's rotation; a joint outside it refuses the whole batch and requests nothing.
#[test]
fn a_hold_from_a_snapshot_and_its_refusals() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	clear();
	let id = request(18, 24);
	service(&local(0.0));
	let mut s = Store::default();
	assert_eq!(s.set(7, 111, &[hold(19, id), hold(21, id)]), Ok(2));
	let r = s.records(7);
	assert_eq!((r[0].joint, r[0].rot[0], r[1].joint, r[1].rot[0]), (19, 19.0 / 200.0, 21, 21.0 / 200.0));
	let mut t = Store::default();
	assert_eq!(t.set(7, 111, &[hold(19, id), hold(30, id)]), Err(ERR_BAD_JOINT));
	assert!(t.is_empty() && !pending());
	assert_eq!(t.set(7, 111, &[hold(19, id + 9)]), Err(ERR_RANGE));
	// forgotten by a clear: the held joint goes back to the animation (no record)
	clear();
	assert!(s.records(7).is_empty());
}

#[test]
fn a_hold_fades_out_like_any_op() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	clear();
	let mut s = Store::default();
	let mut h = hold(20, 0);
	h.blend_ticks = 4;
	s.set(7, 111, &[h]).unwrap();
	service(&local(0.0));
	for _ in 0..4 {
		s.step();
	}
	assert_eq!(s.records(7)[0].blend, Some(1.0));
	let mut off = hold(20, 0);
	off.flags = crate::omni::pose::FLAG_REMOVE;
	off.blend_ticks = 2;
	s.set(7, 111, &[off]).unwrap();
	s.step();
	assert!(s.records(7)[0].blend.unwrap() < 1.0);
	s.step();
	s.step();
	assert!(s.is_empty());
	clear();
}

// The debug dots: a 4 x 4 rect and the name (or the joint number) for every joint that has a place on the screen.
#[test]
fn the_debug_list_has_a_dot_and_a_name_per_visible_joint() {
	let pts = [Some([100.0, 50.0]), None, Some([10.0, 20.0])];
	let (cmds, pool) = debug_cmds(&pts, &[("hips".to_string(), 0)]);
	assert_eq!(cmds.len(), 4);
	assert_eq!((cmds[0].kind, cmds[0].x, cmds[0].y, cmds[0].w, cmds[0].h), (2, 98.0, 48.0, 4.0, 4.0));
	let text = |c: &OmniDrawCmd| std::str::from_utf8(&pool[c.text_off as usize..(c.text_off + c.text_len) as usize]).unwrap().to_string();
	assert_eq!((cmds[1].kind, text(&cmds[1])), (1, "hips".to_string()));
	assert_eq!(text(&cmds[3]), "2", "no name: the joint number");
	assert!(debug_cmds(&[None, None], &[]).0.is_empty());
}

#[test]
fn debug_is_a_flag_and_clear_turns_it_off() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	clear();
	assert!(!debug_on());
	set_debug(true);
	publish_joints(vec![[1.0, 2.0, 3.0]]);
	assert!(debug_on());
	assert_eq!(joints(), vec![[1.0, 2.0, 3.0]]);
	clear();
	assert!(!debug_on() && joints().is_empty());
}

// The game-pose frames for `pose_read`: the worker asks (`want_reads`), the renderer publishes, the worker copies; `clear` takes all back.
#[test]
fn reads_ask_the_renderer_and_get_the_published_frames() {
	let _t = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	clear();
	assert!(!reads_on());
	assert!(rendered().0.is_empty(), "nothing before the first rendered frame");
	want_reads();
	assert!(reads_on());
	publish_world(vec![[1.0; 12], [2.0; 12]], [3.0, 4.0, 5.0]);
	assert_eq!(rendered(), (vec![[1.0; 12], [2.0; 12]], [3.0, 4.0, 5.0]));
	clear();
	assert!(!reads_on() && rendered().0.is_empty());
}
