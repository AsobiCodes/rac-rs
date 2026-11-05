//! Ko-fi drawings: the pictures are the same every run, carry the exact names, and the gate opens in the Veldin garage on every visit, and only with the row On.

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
	for (i, want) in ["Scrolt", "korixx", "reTokyo", "khalifax10", "DeSynkro", "DarkBlaze", "Smoe05", "eardrummer", "Chris", "Ryan Palmer", "fleece", "Justintime4u2bu", "Ga", "THUND3R_WOLF", "MK", "Ratchetenjoyer", "Tonyswan", "Luke4600"].iter().enumerate() {
		let d = scenes::draw(i);
		assert_eq!(d.name, *want);
		assert_eq!(ROSTER[i].name, *want);
		assert!(d.texts.iter().any(|t| t == want || t == &format!("{want}!")), "{want}: {:?}", d.texts);
		assert!(d.texts.iter().any(|t| t.starts_with("THANK YOU")), "{want}: {:?}", d.texts);
	}
}

#[test]
fn pictures_are_deterministic_and_inked() {
	for i in 0..ROSTER.len() {
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
fn the_sheets_differ() {
	let h: Vec<u64> = (0..ROSTER.len()).map(|i| fnv(&scenes::draw(i))).collect();
	for a in 0..h.len() {
		for b in a + 1..h.len() {
			assert_ne!(h[a], h[b], "sheets {a} and {b}");
		}
	}
}

/// The first three sheets keep their pixels whatever is added to the roster (user: the approved look stays). Pinned hashes of
/// `fnv` over the RGBA bytes; the 4th sheet (khalifax10) is new and only has to be stable (`pictures_are_deterministic_and_inked`).
#[test]
fn the_approved_three_keep_their_pixels() {
	const PINNED: [u64; 3] = [5257662899526470010, 8297048128938873610, 4592143315255363368];
	let got: Vec<u64> = (0..3).map(|i| fnv(&scenes::draw(i))).collect();
	assert_eq!(got, PINNED, "pixel hashes of Scrolt, korixx, reTokyo (computed from the dev tree before this change)");
}

#[test]
fn gate_cases() {
	assert!(gate(0), "Veldin: every visit");
	assert!(!gate(1), "another level");
	assert!(!gate(usize::MAX as u32));
}

#[test]
fn the_row_decides_and_defaults_to_off() {
	// A fresh process: Off. With the row On the sheets are wanted in the garage only; `note` outside the game or elsewhere clears it.
	assert!(!shown());
	set_shown(true);
	note(true, 0);
	assert!(active() == enabled(), "on in the garage (unless RAC_KOFI=0)");
	note(true, 3);
	assert!(!active(), "another level");
	note(false, 0);
	assert!(!active(), "front end");
	set_shown(false);
	note(true, 0);
	assert!(!active(), "row Off: never, with any Lighting");
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
	assert_eq!(places.len(), ROSTER.len(), "one place per supporter");
	// 1x1 stubs with the supporters' names: the test is about `zip` and the names, not about the pictures.
	let stub = |name: &'static str| Drawing { name, texts: Vec::new(), w: 1, h: 1, rgba: vec![[0; 4]] };
	let mut art: Vec<Drawing> = ROSTER.iter().map(|e| stub(e.name)).collect();
	assert_eq!(sheets(&art, &places).count(), ROSTER.len());
	art.push(stub("extra"));
	let hung: Vec<&str> = sheets(&art, &places).map(|(d, _)| d.name).collect();
	assert_eq!(hung.len(), ROSTER.len(), "an extra drawing without a wall spot must not panic");
	assert_eq!(sheets(&art, &places[..2]).count(), 2, "a placement missing for the last drawings");
}

#[test]
fn every_sheet_hangs_inside_the_garage() {
	// The garage's walls as `RAC_TUNE kofi_map=1` found them (x 137.7..155.6, y 86.1..102.8): every corner of every sheet's
	// hand-picked place, tilt included, stays inside (with a margin; kofi/walls.rs then puts it on the wall itself).
	for (e, p) in ROSTER.iter().zip(placements()) {
		for c in corners(&p, scenes::W + 2 * scenes::MARGIN, scenes::H + 2 * scenes::MARGIN, 0.03) {
			assert!((137.0..156.5).contains(&c[0]) && (85.5..103.5).contains(&c[1]) && (31.0..39.0).contains(&c[2]), "{}: {c:?}", e.name);
		}
	}
}

/// Dev preview, not a check (ignored): writes every sheet (or those whose name contains `KOFI_PREVIEW_ONLY`) as
/// `<KOFI_PREVIEW>/NN_name.ppm` over grey, to look at a drawing without the game (not RAC_*: test.ps1 drops those). PowerShell:
/// `$env:KOFI_PREVIEW='<lane>\pp\kofi'; scratch_perf_tools\test.ps1 -Lane <lane> -Pkgs rac1 -Filter 'kofi::tests::preview -- --ignored'`
#[test]
#[ignore]
fn preview() {
	let Ok(dir) = std::env::var("KOFI_PREVIEW") else { return };
	let only = std::env::var("KOFI_PREVIEW_ONLY").unwrap_or_default();
	std::fs::create_dir_all(&dir).unwrap();
	for (i, e) in ROSTER.iter().enumerate().filter(|(_, e)| e.name.contains(only.as_str())) {
		let d = scenes::draw(i);
		let mut out = format!("P6\n{} {}\n255\n", d.w, d.h).into_bytes();
		for p in &d.rgba {
			let a = p[3] as u32;
			out.extend((0..3).map(|k| ((p[k] as u32 * a + 90 * (255 - a)) / 255) as u8));
		}
		let name: String = e.name.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
		std::fs::write(format!("{dir}/{i:02}_{name}.ppm"), out).unwrap();
	}
}
