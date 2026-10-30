//! Tests of the Display section (pcdisp.rs). The settings are process-wide atomics, so the tests run one after the other in one
//! `#[test]` per topic and put the defaults back first.
use super::*;
use std::sync::MutexGuard;

/// The settings are process-wide: one test at a time, together with every other test that reads them (`pcopts::globals_guard`).
fn guard() -> MutexGuard<'static, ()> {
	crate::retail::pcopts::globals_guard()
}

fn defaults() {
	init(&PcOptions::parse(""));
}

#[test]
fn the_file_sets_the_start_values_and_defaults_are_todays() {
	let _g = guard();
	defaults();
	assert_eq!((hud_corners(), hud_scale_pct(), fps_setting(), render_scale_pct()), (false, 100, 0, 100));
	init(&PcOptions::parse("hudpos=corners\nhudscale=130\nfps=off\nrenderscale=150\n"));
	assert_eq!((hud_corners(), hud_scale_pct(), fps_setting(), render_scale_pct()), (true, 130, u32::MAX, 150));
	init(&PcOptions::parse("hudpos=nonsense\nhudscale=1000\nfps=70\nrenderscale=90\n"));
	assert_eq!((hud_corners(), hud_scale_pct(), fps_setting(), render_scale_pct()), (false, 150, 60, 100), "snapped to a listed value");
	init(&PcOptions::parse("hudscale=3\nfps=display\nrenderscale=abc\n"));
	assert_eq!((hud_scale_pct(), fps_setting(), render_scale_pct()), (75, 0, 100));
	defaults();
}

#[test]
fn values_snap_and_steps_wrap() {
	let _g = guard();
	assert_eq!([snap_hud(0), snap_hud(77), snap_hud(78), snap_hud(100), snap_hud(999)], [75, 75, 80, 100, 150]);
	assert_eq!([parse_fps("30"), parse_fps("144"), parse_fps("100"), parse_fps("0"), parse_fps("junk"), parse_fps("OFF")], [30, 144, 120, 0, 0, u32::MAX]);
	assert_eq!([parse_rscale("50"), parse_rscale("110"), parse_rscale("200%"), parse_rscale("")], [50, 100, 200, 100]);
	assert_eq!([step_index(0, 6, true), step_index(5, 6, false), step_index(2, 6, false), step_index(2, 6, true)], [5, 0, 3, 1]);
	defaults();
	// HUD scale: 100 -> 105 .. 150 -> 75 (Cross / Right wrap), 75 -> 150 (Left wraps).
	assert_eq!(stepped(0, false), 105);
	SCALE.store(150, Ordering::Relaxed);
	assert_eq!(stepped(0, false), 75);
	SCALE.store(75, Ordering::Relaxed);
	assert_eq!(stepped(0, true), 150);
	// FPS: Display -> 30 -> 60 -> 120 -> 144 -> Off -> Display; Left from Display is Off.
	let mut seq = vec![];
	for _ in 0..6 {
		let n = stepped(1, false);
		FPS.store(n, Ordering::Relaxed);
		seq.push(n);
	}
	assert_eq!(seq, [30, 60, 120, 144, u32::MAX, 0]);
	assert_eq!(stepped(1, true), u32::MAX);
	// Render scale: 100 -> 125 -> 150 -> 200 -> 50; Left from 50 is 200.
	assert_eq!(stepped(2, false), 125);
	RSCALE.store(200, Ordering::Relaxed);
	assert_eq!(stepped(2, false), 50);
	RSCALE.store(50, Ordering::Relaxed);
	assert_eq!(stepped(2, true), 200);
	defaults();
}

#[test]
fn every_text_is_short_font_bytes_in_every_language() {
	let _g = guard();
	defaults();
	note_refresh(144);
	note_window_size(3440, 1440);
	for lang in 0..5 {
		for id in ID0..ID_END {
			let s = text(id, lang);
			assert!(s.len() < SLOT as usize - 1 && s.len() <= 22, "{id:#x} {lang}: {s:?}");
			assert!(s.iter().all(|&b| (0x20..0x7f).contains(&b) || (0x80..=0xb0).contains(&b)), "{id:#x} {lang}: {s:?}");
			assert!(!s.contains(&b'%'), "0x25 draws the Cross glyph: {s:?}");
			assert_eq!(s.is_empty(), id > ID_VRS && !crate::retail::pcinput::IDS.contains(&id), "{id:#x} {lang}: ids 0x7fb0..=0x7fb8 (pcdisp) and 0x7fba, 0x7fbb, 0x7fbc, 0x7fbd (pcinput) have text, the rest none");
		}
		assert!(LABELS.iter().all(|t| t[lang].len() <= 21) && SECTION[lang].len() <= 21);
	}
	assert_eq!(text(ID_VSCALE, 0), b"< 100 >");
	assert_eq!(text(ID_VFPS, 0), b"< Display 144Hz >");
	assert_eq!(text(ID_VFPS, 1), b"< \x8ccran 144Hz >");
	assert_eq!(text(ID_VRS, 0), b"< 100 - 3440x1440 >");
	assert_eq!((text(ID_P43, 2), text(ID_CORNERS, 2), text(ID_HUDPOS, 3)), (b"4:3".to_vec(), b"Ecken".to_vec(), b"Posici\x9cn del HUD".to_vec()));
	SCALE.store(75, Ordering::Relaxed);
	FPS.store(30, Ordering::Relaxed);
	RSCALE.store(50, Ordering::Relaxed);
	assert_eq!((text(ID_VSCALE, 0), text(ID_VFPS, 0), text(ID_VRS, 0)), (b"< 75 >".to_vec(), b"< 30 >".to_vec(), b"< 50 - 1720x720 >".to_vec()));
	FPS.store(u32::MAX, Ordering::Relaxed);
	assert_eq!((text(ID_VFPS, 0), text(ID_VFPS, 3)), (b"< Off >".to_vec(), b"< Sin l\x94mite >".to_vec()));
	note_refresh(0);
	note_window_size(0, 0);
	FPS.store(0, Ordering::Relaxed);
	assert_eq!((text(ID_VFPS, 4), text(ID_VRS, 0)), (b"< Schermo >".to_vec(), b"< 50 >".to_vec()));
	defaults();
}

#[test]
fn the_pool_and_bytes_fit_the_reserved_block() {
	let _g = guard();
	assert_eq!(RANGE, POOL..POOL + 0x400);
	assert_eq!(POOL + SLOT * (ID_END - ID0), RANGE.end);
	assert!(RANGE.start >= super::super::pcmods::RANGE.end && RANGE.end <= super::super::pcext::BASE + 0x10000);
	assert!(RANGE.start >= super::super::pcctl::DBG_RANGE.end);
	for b in [POS_B, SCALE_B, FPS_B, RS_B] {
		assert!(super::super::pcctl::RANGE.contains(&b) && b > CBASE + 0x16 && b < CBASE + 0x20, "{b:#x}: CBASE+0x17.. is this section's");
	}
	assert_eq!([SCALE_B, FPS_B, RS_B], [POS_B + 1, POS_B + 2, POS_B + 3], "pcctl's table takes POS_B + 1..3 for the steppers");
}

/// A page built on fresh RAM starts the steppers at 0 and the position at the file's value; a step is written to the options file
/// once (`sync`), a default removes its key, and nothing is written before the page exists.
#[test]
fn a_change_goes_into_the_options_file_once() {
	let _g = guard();
	let mut o = PcOptions::default();
	let mut mem = Mem::new();
	assert!(!sync(&mut mem, &mut o), "page not built yet");
	init(&PcOptions::parse("hudpos=corners\n"));
	let rows: Vec<[u32; 6]> = (0..15u32).map(|i| [0x100 + i, 0x200 + i, 0x300, 0x301, 0, 0]).collect();
	super::super::pcctl::build_gfx(&mut mem, super::super::pcext::BASE + 0xaa0, &rows, true, false);
	assert_eq!((mem.rd8(POS_B), mem.rd8(SCALE_B), mem.rd8(FPS_B), mem.rd8(RS_B)), (1, 0, 0, 0));
	assert!(!sync(&mut mem, &mut o), "the first look changes nothing");
	// The player picks 4:3: the byte flips, the key goes away.
	o.extra_set("hudpos", Some("corners".into()));
	mem.wr8(POS_B, 0);
	assert!(sync(&mut mem, &mut o));
	assert_eq!((o.extra_get("hudpos"), hud_corners()), (None, false));
	assert!(!sync(&mut mem, &mut o), "once");
	// A Cross on the FPS row (the byte toggles), then one on the Render scale row.
	let level = 0;
	field(&mut mem, level, [false; 2]);
	mem.wr8(FPS_B, 1);
	field(&mut mem, level, [false; 2]);
	assert_eq!(fps_setting(), 30);
	assert!(sync(&mut mem, &mut o));
	assert_eq!(o.extra_get("fps"), Some("30"));
	mem.wr8(RS_B, 1);
	field(&mut mem, level, [false; 2]);
	assert_eq!(render_scale_pct(), 125, "Cross steps on from 100");
	mem.wr8(SCALE_B, 1);
	field(&mut mem, level, [false; 2]);
	assert_eq!(hud_scale_pct(), 105);
	assert!(sync(&mut mem, &mut o));
	assert_eq!((o.extra_get("fps"), o.extra_get("renderscale"), o.extra_get("hudscale")), (Some("30"), Some("125"), Some("105")));
	let text = o.to_text();
	assert!(text.contains("fps=30\n") && text.contains("renderscale=125\n") && text.contains("hudscale=105\n") && !text.contains("hudpos"), "{text}");
	assert_eq!(PcOptions::parse(&text).extra_get("renderscale"), Some("125"));
	defaults();
}
