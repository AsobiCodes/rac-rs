use super::*;

const PIPE: [f32; 5] = [67.52, 223.5, 113.2, 237.1, 130.7];
const PIPE_MESH: [f32; 6] = [222.0, 112.0, 66.9, 238.0, 131.0, 69.7];
/// The strip that holds the pipe's plane in the user's replay (RAC_WATER_LOG `unflat`).
const PIPE_STREAM: [f32; 6] = [170.8, 111.4, 59.7, 237.1, 138.3, 67.6];

fn held(boxes: &[[f32; 5]], unfound: u32) -> Held {
	Held { height: boxes[0][0], boxes: boxes.to_vec(), age: 0, unfound }
}

#[test]
fn a_covering_mesh_holds_the_plane() {
	assert!(covers(&PIPE_MESH, &PIPE));
	assert!(covers(&PIPE_STREAM, &PIPE), "the stream strip of the replay");
	assert!(covers(&[170.8, 111.4, 52.0, 237.1, 138.3, 67.6], &PIPE), "the same strip twice as tall");
	assert!(covers(&[230.0, 112.0, 66.9, 238.0, 131.0, 69.7], &PIPE), "half of the plane box (within the slack)");
}

#[test]
fn a_mesh_elsewhere_or_at_another_height_does_not() {
	assert!(!covers(&[100.0, 60.0, 65.0, 115.0, 75.0, 66.0], &PIPE), "other place");
	assert!(!covers(&[222.0, 112.0, 60.0, 238.0, 131.0, 66.5], &PIPE), "below the plane");
	assert!(!covers(&[222.0, 112.0, 68.5, 238.0, 131.0, 70.0], &PIPE), "above the plane");
}

#[test]
fn a_tall_or_only_touching_mesh_does_not() {
	assert!(!covers(&[222.0, 112.0, 40.0, 238.0, 131.0, 90.0], &PIPE), "tall height range over the whole plane");
	assert!(!covers(&[222.0, 112.0, 60.0, 238.0, 131.0, 79.0], &PIPE), "taller than the plane is wide");
	assert!(covers(&[0.0, 0.0, 61.0, 10.0, 10.0, 66.0], &[65.0, 0.0, 0.0, 10.0, 10.0]), "rise half its side: still held");
	assert!(!covers(&[0.0, 0.0, 60.0, 10.0, 10.0, 69.0], &[65.0, 0.0, 0.0, 10.0, 10.0]), "waterfall-like: steeper than 1 in 2");
	assert!(!covers(&[236.0, 112.0, 66.9, 250.0, 131.0, 69.7], &PIPE), "a strip along one edge");
	assert!(!covers(&[222.0, 129.0, 66.9, 238.0, 140.0, 69.7], &PIPE), "a strip touching the far side");
	assert!(!covers(&[228.0, 100.0, 60.0, 232.0, 140.0, 75.0], &PIPE), "a narrow river strip running past");
	assert!(!covers(&PIPE_MESH, &[67.52, 237.1, 130.7, 223.5, 113.2]), "inverted plane box");
	assert!(!covers(&PIPE_MESH, &[f32::NAN, 223.5, 113.2, 237.1, 130.7]), "NaN height");
}

/// The pipe of the report: its plane drops out of the estimate, a mesh still covers it, so it stays; a found plane at about
/// its height wins (no duplicate); with no covering mesh (out of view) or past the cap it goes.
#[test]
fn held_plane_stays_while_covered() {
	let pool = [65.68, 100.2, 60.4, 115.0, 75.2];
	let h = [held(&[PIPE], 0)];
	assert_eq!(merge(vec![pool], &[PIPE_MESH], &h), vec![pool, PIPE]);
	assert_eq!(merge(vec![pool, [67.6, 223.5, 113.2, 237.1, 130.7]], &[PIPE_MESH], &h).len(), 2);
	assert_eq!(merge(vec![pool], &[], &h), vec![pool]);
	assert_eq!(merge(vec![pool], &[PIPE_MESH], &[held(&[PIPE], UNFOUND_CAP - 1)]), vec![pool, PIPE]);
	assert_eq!(merge(vec![pool], &[PIPE_MESH], &[held(&[PIPE], UNFOUND_CAP)]), vec![pool], "past the cap");
}

/// Every box of a held plane is tried: the mesh in view holds the second of two pieces at the same height.
#[test]
fn every_box_of_a_held_plane_is_tried() {
	let far = [67.52, 10.0, 10.0, 20.0, 20.0];
	assert_eq!(merge(Vec::new(), &[PIPE_MESH], &[held(&[far, PIPE], 3)]), vec![PIPE]);
}

/// Property: when every held height has a plane of this frame within 0.5, `merge` returns this frame's planes unchanged.
#[test]
fn merge_is_identity_when_the_estimate_finds_every_held_plane() {
	let mut s = 0x2545_f491_u32;
	let mut r = move |lo: f32, hi: f32| {
		s ^= s << 13;
		s ^= s >> 17;
		s ^= s << 5;
		lo + (hi - lo) * (s as f32 / u32::MAX as f32)
	};
	for _ in 0..2000 {
		let found: Vec<[f32; 5]> = (0..1 + (r(0.0, 5.0) as usize)).map(|_| {
			let (x, y) = (r(-300.0, 300.0), r(-300.0, 300.0));
			[r(0.0, 100.0), x, y, x + r(0.0, 40.0), y + r(0.0, 40.0)]
		}).collect();
		let hs: Vec<Held> = (0..r(0.0, 4.0) as usize).map(|_| {
			let f = found[(r(0.0, found.len() as f32) as usize).min(found.len() - 1)];
			let mut b = f;
			b[0] += r(-0.49, 0.49);
			held(&[b], r(0.0, 600.0) as u32)
		}).collect();
		let unflat: Vec<[f32; 6]> = hs.iter().map(|h| { let b = h.boxes[0]; [b[1], b[2], b[0] - 1.0, b[3], b[4], b[0] + 1.0] }).collect();
		assert_eq!(merge(found.clone(), &unflat, &hs), found);
	}
}

/// The bookkeeping after a pick: boxes within 0.4 stored, found again resets the unfound count and refreshes the boxes, a
/// held-only pick counts up, a plane not picked stays `MIRROR_HOLD` frames while a slot is free.
#[test]
fn update_keeps_boxes_and_counts() {
	let mut m = Memory::new();
	m.start(1, 3, [0.0; 3]);
	let other = [67.3, 10.0, 10.0, 20.0, 20.0];
	let far = [60.0, 0.0, 0.0, 1.0, 1.0];
	m.update(&[PIPE, other, far], &[PIPE, other, far], &[67.52], 1);
	assert_eq!(m.held, vec![Held { height: 67.52, boxes: vec![PIPE, other], age: 0, unfound: 0 }]);
	// The estimate drops it; the hold re-adds its boxes and the pick takes it: same boxes, unfound counts.
	m.update(&[], &[PIPE, other], &[67.52], 1);
	m.update(&[], &[PIPE, other], &[67.52], 1);
	assert_eq!((m.held[0].unfound, m.held[0].age, m.held[0].boxes.len()), (2, 0, 2));
	// Found again (moved a little): the count resets and the boxes are this frame's.
	let moved = [67.6, 223.0, 113.0, 237.0, 131.0];
	m.update(&[moved], &[moved], &[67.6], 1);
	assert_eq!(m.held, vec![Held { height: 67.6, boxes: vec![moved], age: 0, unfound: 0 }]);
	// Not picked: held MIRROR_HOLD frames, then gone.
	for _ in 0..MIRROR_HOLD {
		m.update(&[], &[], &[], 1);
		assert_eq!(m.heights(), vec![67.6]);
	}
	m.update(&[], &[], &[], 1);
	assert!(m.held.is_empty());
}

/// The cap: held through the hold alone, the plane leaves the candidates after `UNFOUND_CAP` frames.
#[test]
fn cap_expires_a_plane_the_estimate_never_finds_again() {
	let mut m = Memory::new();
	m.start(1, 1, [0.0; 3]);
	m.update(&[PIPE], &[PIPE], &[67.52], 1);
	let mut frames = 0;
	loop {
		let boxes = merge(Vec::new(), &[PIPE_MESH], &m.held);
		let fresh: Vec<f32> = boxes.first().map(|b| b[0]).into_iter().collect();
		if fresh.is_empty() {
			break;
		}
		m.update(&[], &boxes, &fresh, 1);
		frames += 1;
		assert!(frames <= UNFOUND_CAP, "never expires");
	}
	assert_eq!(frames, UNFOUND_CAP);
}

#[test]
fn level_slots_and_camera_jumps_clear_the_memory() {
	let mut m = Memory::new();
	let fill = |m: &mut Memory| m.update(&[PIPE], &[PIPE], &[67.52], 3);
	m.start(1, 3, [0.0; 3]);
	fill(&mut m);
	m.start(1, 3, [1.0, 0.0, 0.0]);
	assert_eq!(m.heights().len(), 1, "small move keeps");
	m.start(2, 3, [1.0, 0.0, 0.0]);
	assert!(m.held.is_empty(), "level change");
	fill(&mut m);
	m.start(2, 1, [1.0, 0.0, 0.0]);
	assert!(m.held.is_empty(), "Water setting change");
	fill(&mut m);
	m.start(2, 1, [40.0, 0.0, 0.0]);
	assert!(m.held.is_empty(), "camera jump");
	fill(&mut m);
	m.forget();
	assert!(m.held.is_empty(), "frame without a pick");
}
