use super::*;

#[test]
fn pulses_stay_in_range_and_travel_wraps() {
	for i in 0..200 {
		let t = i as f64 * 0.37;
		let p = pulse(t, 2.4, 0.3);
		assert!((0.0..=1.0).contains(&p));
		assert!((0.0..1.0).contains(&travel(t, 2.2)));
	}
	assert!(pulse(0.0, 2.0, 0.0) < 1e-6 && (pulse(1.0, 2.0, 0.0) - 1.0).abs() < 1e-6);
}

#[test]
fn the_core_breathes_within_its_band_and_the_angle_stays_inside_one_turn() {
	for i in 0..400 {
		let a = anim_at(i as f64 * 1.7 + 1.0e6);
		assert!((1.0 - CORE_AMP - 1e-4..=1.0 + CORE_AMP + 1e-4).contains(&a.core_gain));
		assert!(a.angle >= 0.0 && a.angle < std::f32::consts::TAU);
		assert!(a.time >= 0.0 && a.time < 600.0);
	}
	assert!(TURN_PERIOD_S >= 360.0 && TURN_PERIOD_S <= 600.0, "one turn in 6-10 minutes");
}

#[test]
fn the_star_map_marks_small_bright_peaks_only() {
	let (w, h) = (1280usize, 720usize); // the disc facts are in painting pixels: the test sky is full size
	let mut px = vec![0u8; w * h * 4];
	for p in px.chunks_exact_mut(4) {
		p.copy_from_slice(&[10, 12, 30, 255]);
	}
	let mut put = |x: usize, y: usize, v: u8| px[(y * w + x) * 4..(y * w + x) * 4 + 3].copy_from_slice(&[v, v, v]);
	put(100, 100, 255); // a star
	put(101, 100, 120);
	for y in 500..520 {
		for x in 900..930 {
			put(x, y, 230); // a big bright blob: not a star
		}
	}
	mark_stars(&mut px, w as u32, h as u32);
	let a = |x: usize, y: usize| px[(y * w + x) * 4 + 3];
	assert!((1..255).contains(&a(100, 100)), "the star has a code");
	assert!((1..255).contains(&a(101, 100)) || a(101, 100) == 255);
	assert_eq!(a(5, 5), 255, "dark sky is no star");
	assert_eq!(a(915, 510), 255, "a big blob is no star");
}

#[test]
fn the_real_painting_has_many_stars_and_the_core_has_none() {
	let i = crate::texpack::png::load(super::super::ship_marks::ART_PNG).expect("the painting loads");
	let mut px = i.levels[0].clone();
	mark_stars(&mut px, i.w, i.h);
	let n = px.chunks_exact(4).filter(|p| p[3] != 255).count();
	assert!(n > 200 && n < 20000, "star pixels: {n}");
	let w = i.w as usize;
	let core = (640usize, 330usize);
	for y in core.1 - 30..core.1 + 30 {
		for x in core.0 - 30..core.0 + 30 {
			assert_eq!(px[(y * w + x) * 4 + 3], 255, "core pixel {x},{y}");
		}
	}
}
