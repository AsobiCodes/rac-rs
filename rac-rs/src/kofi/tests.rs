//! Ko-fi drawings: the pictures are the same every run, carry the exact names, and the gate opens only on the first Veldin visit.

use super::*;

fn fnv(d: &Drawing) -> u64 {
	let mut h = 0xcbf2_9ce4_8422_2325u64;
	for p in &d.rgba {
		for b in p {
			h = (h ^ *b as u64).wrapping_mul(0x100_0000_01b3);
		}
	}
	h
}

#[test]
fn names_are_spelled_exactly() {
	for (i, want) in ["Scrolt", "korixx", "reTokyo"].iter().enumerate() {
		let d = scenes::draw(i);
		assert_eq!(d.name, *want);
		assert!(d.texts.iter().any(|t| t == want || t == &format!("{want}!")), "{want}: {:?}", d.texts);
		assert!(d.texts.iter().any(|t| t.starts_with("THANK YOU")), "{want}: {:?}", d.texts);
	}
}

#[test]
fn pictures_are_deterministic_and_inked() {
	for i in 0..3 {
		let (a, b) = (scenes::draw(i), scenes::draw(i));
		assert_eq!(fnv(&a), fnv(&b), "drawing {i} differs between runs");
		assert_eq!((a.w, a.h), (scenes::W + 2 * scenes::MARGIN, scenes::H + 2 * scenes::MARGIN));
		// The margin is transparent shadow, the paper opaque, and a good part of the sheet carries colour.
		assert_eq!(a.rgba[0][3], 0);
		let paper = &a.rgba[(a.h / 2) * a.w + a.w / 2];
		assert_eq!(paper[3], 255);
		let inked = a.rgba.iter().filter(|p| p[3] == 255 && (p[0].max(p[1]).max(p[2]) as i32 - p[0].min(p[1]).min(p[2]) as i32 > 60 || p[0].max(p[1]).max(p[2]) < 90)).count();
		assert!(inked > a.w * a.h / 25, "drawing {i} has only {inked} inked pixels");
	}
}

#[test]
fn the_three_sheets_differ() {
	let h: Vec<u64> = (0..3).map(|i| fnv(&scenes::draw(i))).collect();
	assert!(h[0] != h[1] && h[1] != h[2] && h[0] != h[2]);
}

#[test]
fn gate_cases() {
	assert!(gate(0, 0), "first Veldin visit");
	assert!(!gate(0, 1), "Veldin again after Novalis");
	assert!(!gate(1, 0), "another level");
	assert!(!gate(usize::MAX as u32, 0));
}

#[test]
fn quad_faces_the_room_and_is_not_mirrored() {
	// A wall at x = 100 that looks toward -x: the sheet's right-hand side (seen from the room) is toward -y.
	let p = Placement { c: [100.0, 50.0, 30.0], facing: std::f32::consts::PI, w: 2.0, tilt: 0.0 };
	let c = corners(&p, 100, 50, 0.1);
	assert!((c[0][0] - 99.9).abs() < 1e-3, "lifted off the wall toward -x: {c:?}");
	assert!(c[1][1] < c[0][1] - 1.9, "right of the sheet is toward -y: {c:?}");
	assert!(c[0][2] > c[3][2] + 0.9 && (c[0][2] - c[3][2] - 1.0).abs() < 1e-3, "top above bottom, height = w / aspect: {c:?}");
	// A tilt turns the sheet around the wall normal: the top edge climbs to the right.
	let t = corners(&Placement { tilt: 0.1, ..p }, 100, 50, 0.0);
	assert!(t[1][2] > t[0][2]);
}

#[test]
fn a_drawing_without_a_placement_is_left_out() {
	let places = placements();
	// 1x1 stubs with the supporters' names: the test is about `zip` and the names, not about the pictures.
	let stub = |name: &'static str| Drawing { name, texts: Vec::new(), w: 1, h: 1, rgba: vec![[0; 4]] };
	let art: Vec<Drawing> = scenes::SUPPORTERS.iter().map(|&n| stub(n)).collect();
	let mut four: Vec<Drawing> = scenes::SUPPORTERS.iter().map(|&n| stub(n)).collect();
	four.push(stub("extra"));
	let hung: Vec<&str> = sheets(&four, &places).map(|(d, _)| d.name).collect();
	assert_eq!(hung, ["Scrolt", "korixx", "reTokyo"], "the 4th drawing has no wall spot and must not panic");
	assert_eq!(sheets(&art, &places[..2]).count(), 2, "a placement missing for the last drawing");
}
