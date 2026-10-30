//! The vendor shop's checks of the in-game mouse test (lane mmenu stage 6): hover == keys on every icon, wheel notches == key presses,
//! a stray click buys nothing, click-click on an icon == Cross Cross, and the right click cancels the prompt. Every comparison reads the
//! shop's words the worker publishes each field (`mouse_vendor::SEEN`: sel, stage, exit pending, quantity, bolts, ammo 10, ammo 16,
//! ammo-row mask | ammo bar present << 8). A purchase is compared by its DELTAS (bolts and ammo before / after), keys first.
//!
//! Manual: `$env:RAC_TEST_STATE='<oracle>/states/s12'` (Eudora, shop open, 4 rows), then
//! `cargo test --release --lib retail::mouse_latency_test::vendor -- --ignored --nocapture` and read the VENDOR lines.

use super::*;
use crate::retail::mouse_vendor::SEEN;
use std::sync::atomic::Ordering;

fn seen() -> Vec<u32> {
	SEEN.iter().map(|a| a.load(Ordering::Relaxed)).collect()
}

fn icon(i: u32) -> (f32, f32) {
	(144.0 + 56.0 * i as f32 + 28.0, 388.0)
}

fn point(r: &mut Retail, (x, y): (f32, f32)) {
	let (fx, fy) = frac(r, x, y);
	r.mouse_cursor(fx, fy);
}

/// By keys to icon `to` from `from` (Right / Left presses).
fn walk(r: &mut Retail, from: u32, to: u32) {
	keys(r, if to > from { K::ArrowRight } else { K::ArrowLeft }, from.abs_diff(to));
	pump(r, 30);
}

/// (bolts, ammo 10, ammo 16) spent / gained between two `seen()` readings.
fn delta(a: &[u32], b: &[u32]) -> [i64; 3] {
	[4, 5, 6].map(|i| b[i] as i64 - a[i] as i64)
}

#[test]
#[ignore]
fn mouse_vendor_in_game() {
	unsafe {
		std::env::set_var("RAC_RETAIL_HUD", "0");
		std::env::set_var("RAC_PLAYER_CARD", "0");
	}
	let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s12").into());
	enable(&dir).unwrap();
	let mut r = Retail::load().unwrap();
	r.aspect = 16.0 / 9.0;
	let mut fails: Vec<String> = Vec::new();
	let up = |r: &mut Retail| (0..600).any(|_| { r.tick_keys(|_| false, false, false); r.snap.vend.as_ref().is_some_and(|m| !m.busy()) });
	assert!(up(&mut r), "the shop never became ready (mode {})", r.snap.mode);
	pump(&mut r, 30);
	set_mode(&mut r, true);
	let m0 = r.snap.vend.clone().unwrap();
	let count = m0.items[0].rect[2] as u32 / 56;
	eprintln!("VENDOR GEOM rows {count} rect {:?} state {} words {:?}", m0.items[0].rect, m0.state, seen());
	// Hover == keys on every icon, from icon 0 (walk there first), and the fields the hover takes.
	walk(&mut r, count - 1, 0);
	for k in 1..count {
		walk(&mut r, 0, k);
		let by_keys = seen();
		walk(&mut r, k, 0);
		point(&mut r, icon(0));
		pump(&mut r, 10);
		point(&mut r, icon(k));
		let n = (1..=40).find(|_| { r.tick_keys(|_| false, false, false); r.snap.vend.as_ref().is_some_and(|m| m.items.first().is_some_and(|i| matches!(i.kind, Kind::Grid { cursor, .. } if cursor == k))) });
		pump(&mut r, 30);
		let by_mouse = seen();
		// Only the handle value of the ammo bar may differ (allocation history); compare presence, not the value.
		let same = by_keys == by_mouse;
		eprintln!("VENDOR EQUALITY hover 0 -> {k}: selected after {n:?} fields; hover == keys {same}");
		if !same {
			eprintln!("  keys {by_keys:?}\n  mouse {by_mouse:?}");
			fails.push(format!("hover 0 -> {k} != keys"));
		}
		walk(&mut r, k, 0);
	}
	// Wheel: two notches in one field against two Right presses.
	if count >= 3 {
		walk(&mut r, 0, 2);
		let by_keys = seen();
		walk(&mut r, 2, 0);
		r.mouse_wheel(-1.0);
		r.mouse_wheel(-1.0);
		pump(&mut r, 40);
		let ok = seen() == by_keys;
		eprintln!("VENDOR EQUALITY wheel 2 notches == 2 presses {ok}");
		if !ok {
			fails.push("wheel != keys".into());
		}
		walk(&mut r, 2, 0);
	}
	// A stray click beside the strip buys nothing and moves nothing.
	let before = seen();
	point(&mut r, (300.0, 100.0));
	r.mouse_click();
	pump(&mut r, 60);
	let ok = seen() == before;
	eprintln!("VENDOR stray click in browse changed nothing {ok}");
	if !ok {
		fails.push("stray click changed the shop".into());
	}
	// Click-click on an ammo icon (prompt, then confirm) against Cross Cross by keys, compared by what was bought.
	let Some(a) = (0..count).find(|i| seen()[7] >> i & 1 == 1) else {
		eprintln!("VENDOR no ammo row: purchase checks skipped");
		for f in &fails { eprintln!("VENDOR FAIL: {f}"); }
		assert!(fails.is_empty());
		return;
	};
	walk(&mut r, 0, a);
	let b0 = seen();
	keys(&mut r, K::Enter, 1);
	pump(&mut r, 20);
	let prompt = seen();
	// Prompt: a stray click and the stray wheel-free state keep the prompt open and the bolts; the right click cancels.
	point(&mut r, (300.0, 100.0));
	r.mouse_click();
	pump(&mut r, 40);
	let still = seen();
	r.mouse_back();
	pump(&mut r, 80);
	let back = seen();
	let ok = prompt[1] == 2 && still[1] == 2 && still[4] == b0[4] && back[1] == 0 && back[4] == b0[4];
	eprintln!("VENDOR prompt: stage {} -> stray click {} (bolts unchanged) -> right click {} {ok}", prompt[1], still[1], back[1]);
	if !ok {
		fails.push("prompt stray click / right click".into());
	}
	// Keys: Cross, Cross. Then back to icon 0 and the same by mouse (hover + Cross, Cross on the selected icon).
	keys(&mut r, K::Enter, 1);
	pump(&mut r, 20);
	keys(&mut r, K::Enter, 1);
	pump(&mut r, 150);
	let by_keys = delta(&b0, &seen());
	walk(&mut r, a, 0);
	let m0 = seen();
	point(&mut r, icon(a));
	r.mouse_click();
	pump(&mut r, 60);
	let in_prompt = seen()[1];
	// The fields above run unpaced; the double-click guard counts wall time (mouse_menu::PROMPT_GUARD_MS), so wait it out like a hand would.
	std::thread::sleep(std::time::Duration::from_millis(crate::retail::mouse_menu::PROMPT_GUARD_MS + 100));
	r.mouse_click();
	pump(&mut r, 150);
	let by_mouse = delta(&m0, &seen());
	let ok = in_prompt == 2 && by_keys == by_mouse && by_keys[0] != 0;
	eprintln!("VENDOR EQUALITY buy at icon {a}: keys (bolts, ammo10, ammo16) {by_keys:?}, mouse click-click {by_mouse:?}, prompt stage after click 1 = {in_prompt}: {ok}");
	if !ok {
		fails.push("click-click != Cross Cross".into());
	}
	for f in &fails {
		eprintln!("VENDOR FAIL: {f}");
	}
	assert!(r.stop.is_none() && !r.failed);
	assert!(fails.is_empty(), "{} problems (see VENDOR FAIL lines)", fails.len());
}
