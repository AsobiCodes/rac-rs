use super::*;

fn drive(k: f32) -> Drive {
	Drive { on: true, len: 0.0, k: [k, k], room: [1.0, 1.0] }
}

#[test]
fn queue_keeps_one_entry_per_field_in_flight() {
	let mut q = Queue::default();
	q.sent([5, 1], 0);
	q.sent([7, 2], 1);
	assert_eq!(q.pending(1.0, [0.0; 2]), [12.0, 3.0]);
	q.taken(2);
	assert_eq!(q.cur, [5, 1]);
	// Half way through the blend toward the field with 5 counts: 2.5 of them still to come, plus the 7 in flight and 1 not sent.
	assert_eq!(q.pending(0.5, [1.0, 0.0]), [10.5, 2.5]);
	q.taken(1);
	assert_eq!(q.cur, [7, 2]);
	assert_eq!(q.pending(1.0, [0.0; 2]), [0.0, 0.0]);
	// A queue that lost step heals to the tick's count: zeros in front, the oldest dropped.
	q.sent([3, 3], 2);
	assert_eq!(q.sent.len(), 3);
	assert_eq!(q.sent.back(), Some(&[3, 3]));
	q.taken(1);
	assert_eq!(q.cur, [3, 3]);
}

#[test]
fn no_counts_or_off_turn_nothing() {
	assert_eq!(turn_of(&drive(0.003), [0.0, 0.0]), None);
	assert_eq!(turn_of(&Drive { on: false, ..drive(0.003) }, [10.0, 0.0]), None);
	assert_eq!(turn_of(&drive(0.0), [10.0, 5.0]), None);
}

#[test]
fn yaw_is_capped_and_pitch_stays_in_the_room() {
	let d = Drive { on: true, len: 3.0, k: [-0.003, -0.003], room: [0.2, 0.05] };
	let (y, p, len) = turn_of(&d, [1000.0, 100.0]).unwrap();
	assert_eq!((y, len), (-YAW_MAX, 3.0));
	// 100 counts down = -0.3 rad, only 0.05 rad of room below.
	assert_eq!(p, -0.05);
	let (_, p, _) = turn_of(&d, [0.0, -1000.0]).unwrap();
	assert_eq!(p, 0.2);
}

#[test]
fn first_person_turns_in_place_third_person_orbits_the_pivot() {
	let c = ([1.0, 2.0, 3.0], 0.3, -0.2, 0.7);
	let t = turn_cam(c, 0.1, 0.05, 0.0);
	assert_eq!(t.0, c.0);
	assert!((t.1 - 0.4).abs() < 1e-6 && (t.2 + 0.15).abs() < 1e-6 && t.3 == 0.7);
	let fwd = |y: f32, p: f32| {
		let ((sy, cy), (sp, cp)) = (y.fsin_cos(), p.fsin_cos());
		[cy * cp, sy * cp, sp]
	};
	let len = 4.0;
	let o = turn_cam(c, -0.25, 0.1, len);
	let (f0, f1) = (fwd(c.1, c.2), fwd(o.1, o.2));
	for i in 0..3 {
		// The pivot `len` ahead stays where it was: the hero stays put on screen while the view turns round him.
		assert!(((c.0[i] + f0[i] * len) - (o.0[i] + f1[i] * len)).abs() < 1e-4, "{i}: {c:?} {o:?}");
	}
	// The pitch stops short of straight up / down, a pitch already past the stop is not pulled back.
	assert!(turn_cam(c, 0.0, 3.0, 0.0).2 <= PITCH_LIMIT);
	assert_eq!(turn_cam(([0.0; 3], 0.0, 1.52, 1.0), 0.1, 0.0, 0.0).2, 1.52);
}

/// The shown yaw with the late turn is the yaw of every count so far at every blend step, also across a tick: no jump, no lag.
#[test]
fn shown_yaw_is_the_newest_counts_across_ticks() {
	let g = 0.003_f32;
	let d = Drive { on: true, len: 0.0, k: [g, g], room: [1.0, 1.0] };
	let counts: [i16; 6] = [0, 12, 12, 30, -8, 0];
	let mut q = Queue::default();
	// Field n's camera yaw = g x the counts of fields 0..=n (the game's turn); the tick takes field n-1 and hands out field n.
	let yaw_after = |n: usize| counts[..=n].iter().map(|&c| c as f32 * g).sum::<f32>();
	q.sent([counts[0], 0], 0);
	for n in 1..counts.len() {
		q.taken(1);
		q.sent([counts[n], 0], 0);
		let (prev, cur) = (if n >= 2 { yaw_after(n - 2) } else { 0.0 }, yaw_after(n - 1));
		for step in 0..3 {
			let a = step as f32 / 3.0;
			let blended = prev + (cur - prev) * a;
			let shown = blended + turn_of(&d, q.pending(a, [0.0; 2])).map_or(0.0, |t| t.0);
			assert!((shown - yaw_after(n)).abs() < 1e-5, "field {n} step {step}: {shown} vs {}", yaw_after(n));
		}
	}
}
