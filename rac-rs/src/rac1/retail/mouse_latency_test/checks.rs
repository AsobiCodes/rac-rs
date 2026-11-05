//! The grid / wheel / slider / question checks of the in-game mouse test (`mouse_latency_in_game`, lane mmenu stage 3): every one compares
//! what the mouse did with what the keys do from the same state.

use super::*;

fn opposite_key(k: K) -> K {
	match k {
		K::ArrowDown => K::ArrowUp,
		K::ArrowUp => K::ArrowDown,
		K::ArrowLeft => K::ArrowRight,
		_ => K::ArrowLeft,
	}
}

/// `equality` for any key path (a grid: rows by Down / Up, then columns by Right / Left).
fn equality_path(r: &mut Retail, tag: &str, it: &Item, sub: u32, path: &[(K, u32)]) -> bool {
	set_mode(r, true);
	let start = settled(r, 40, 400);
	let trace = |r: &Retail| {
		r.snap.menu.as_ref().map(|m| (m.cur, m.items.iter().filter_map(|i| if let Kind::Grid { cursor, .. } = i.kind { Some((i.addr, cursor)) } else { None }).collect::<Vec<_>>()))
	};
	eprintln!("  KEYTRACE {tag} start {:x?}", trace(r));
	for &(k, n) in path {
		for _ in 0..n {
			keys(r, k, 1);
			pump(r, 10);
			eprintln!("  KEYTRACE {tag} after {k:?}: {:x?}", trace(r));
		}
	}
	let by_keys = settled(r, 60, 600);
	for &(k, n) in path.iter().rev() {
		keys(r, opposite_key(k), n);
	}
	let again = settled(r, 60, 600);
	let (x, y) = centre(it, sub);
	let (fx, fy) = frac(r, x, y);
	let t0 = std::time::Instant::now();
	r.mouse_cursor(fx, fy);
	let by_mouse = settled(r, 60, 600);
	let (back_ok, same) = (start == again, by_keys == by_mouse);
	eprintln!("EQUALITY {tag}: {path:?} to row {sub}: keys then back restores the start {back_ok}; hover == keys {same} ({:?})", t0.elapsed());
	if !same {
		eprintln!("  keys : {by_keys}\n  mouse: {by_mouse}");
	}
	for &(k, n) in path.iter().rev() {
		keys(r, opposite_key(k), n);
	}
	settled(r, 40, 400);
	back_ok && same
}

/// Wheel notches on the current item (all in one field) against the same number of D-pad presses: the settled menus must be equal.
fn wheel_equality(r: &mut Retail, tag: &str, key: K, notches: u32, fails: &mut Vec<String>) {
	set_mode(r, true);
	let start = settled(r, 40, 400);
	keys(r, key, notches);
	let by_keys = settled(r, 60, 600);
	keys(r, opposite_key(key), notches);
	let again = settled(r, 60, 600);
	for _ in 0..notches {
		r.mouse_wheel(if key == K::ArrowDown { -1.0 } else { 1.0 });
	}
	let n = until(r, 40, |m| format!("{:?}", Some(m)) != start);
	let by_wheel = settled(r, 60, 600);
	eprintln!("EQUALITY wheel {tag}: {notches} notches {key:?}: first change after {n:?} fields; wheel == keys {} (keys back restores {})", by_keys == by_wheel, start == again);
	if start != again || by_keys != by_wheel {
		eprintln!("  keys : {by_keys}\n  wheel: {by_wheel}");
		fails.push(format!("wheel {tag}: != keys"));
	}
	keys(r, opposite_key(key), notches);
	settled(r, 40, 400);
}

/// The grid cursor's path to cell `t` by keys.
fn grid_path(cols: u32, c: u32, t: u32) -> Vec<(K, u32)> {
	let (dr, dc) = ((t / cols) as i32 - (c / cols) as i32, (t % cols) as i32 - (c % cols) as i32);
	let mut v = Vec::new();
	if dr != 0 {
		v.push((if dr > 0 { K::ArrowDown } else { K::ArrowUp }, dr.unsigned_abs()));
	}
	if dc != 0 {
		v.push((if dc > 0 { K::ArrowRight } else { K::ArrowLeft }, dc.unsigned_abs()));
	}
	v
}

fn seen() -> Vec<u32> {
	mouse_slider::SEEN.iter().map(|a| a.load(std::sync::atomic::Ordering::Relaxed)).collect()
}

/// A held Right / Left for `n` fields, then settled.
fn hold(r: &mut Retail, key: K, n: u32) {
	for _ in 0..n {
		r.tick_keys(|k| k == key, false, false);
	}
	pump(r, 20);
}

/// The pointer on a Sound Options bar: the click, a drag and the wheel against held Right / Left, compared on the volume words and the
/// mixer variables the game derives (`mouse_slider::SEEN`).
fn slider_checks(r: &mut Retail, pg: &MenuSnap, fails: &mut Vec<String>) {
	let Some(it) = pg.items.iter().find(|i| matches!(i.kind, Kind::Value { n: 3, .. })).cloned() else { return };
	eprintln!("SLIDER page {:#x} item {:#x} rect {:?}", pg.id, it.addr, it.rect);
	for (row, name) in [(0u32, "effects"), (1, "music")] {
		set_mode(r, true);
		hover_fields(r, &it, row, 200);
		pump(r, 20);
		// A start in the middle of the range (the boot volumes sit at an end): all the way down by held Left, then up by held Right.
		hold(r, K::ArrowLeft, 360);
		let low = seen();
		hold(r, K::ArrowRight, 150);
		eprintln!("SLIDERTRACE {name}: after Left 360 {low:?}; after Right 150 {:?}", seen());
		let v0 = seen()[row as usize] as i32;
		hold(r, K::ArrowRight, 20);
		let by_keys = seen();
		hold(r, K::ArrowLeft, 20);
		let again = seen();
		// Click at the pixel of v0 + 60 (the value 20 held fields reach): the same words as the keys.
		let (_, y) = centre(&it, row);
		let aim = |r: &mut Retail, v: i32| {
			let (fx, fy) = frac(r, mouse_slider::x_of(it.rect, v), y);
			r.mouse_cursor(fx, fy);
		};
		aim(r, v0 + 60);
		r.mouse_click();
		let n = (1..=60).find(|_| {
			r.tick_keys(|_| false, false, false);
			seen()[row as usize] as i32 == v0 + 60
		});
		pump(r, 20);
		let by_click = seen();
		eprintln!("SLIDER {name}: v0 {v0}; keys {by_keys:?}; click {by_click:?} after {n:?} fields; click == keys {}; keys back restores {}", by_click == by_keys, again[row as usize] as i32 == v0);
		if by_click != by_keys || again[row as usize] as i32 != v0 {
			fails.push(format!("slider {name}: click != held keys"));
		}
		// Drag: the button is still held, the pointer goes to v0 + 300 and the button is released there.
		aim(r, v0 + 300);
		pump(r, 6);
		let dragged = seen()[row as usize] as i32;
		r.mouse_release();
		pump(r, 10);
		let vt = mouse_slider::value_at(it.rect, mouse_slider::x_of(it.rect, v0 + 300));
		let want = v0 + 3 * ((vt - v0) as f32 / 3.0).round() as i32;
		eprintln!("SLIDER {name}: drag to {} -> {dragged} (lattice value {want})", v0 + 300);
		if dragged != want {
			fails.push(format!("slider {name}: drag {dragged} != {want}"));
		}
		// Wheel: one notch = 16 held fields, against the keys.
		let before = seen();
		hold(r, K::ArrowLeft, 16);
		let keys_down = seen();
		hold(r, K::ArrowRight, 16);
		r.mouse_wheel(-1.0);
		pump(r, 20);
		let wheel_down = seen();
		eprintln!("SLIDER {name}: wheel down {} -> {} (keys: {} -> {}); all words equal {}", before[row as usize], wheel_down[row as usize], before[row as usize], keys_down[row as usize], wheel_down == keys_down);
		if wheel_down != keys_down {
			fails.push(format!("slider {name}: wheel != 16 held fields"));
		}
		// Back to the start by keys for the next row.
		for _ in 0..4 {
			hold(r, K::ArrowLeft, 100);
		}
	}
}

/// Quit Game (page 0xd): the two answer zones exist, a click beside them does nothing, a click on 'No' goes back (never on 'Yes').
fn answers_checks(r: &mut Retail, pg: &MenuSnap, fails: &mut Vec<String>) {
	let zones = mouse_answers::zones(pg);
	eprintln!("ANSWERS page {:#x}: item rects {:?}; zones {zones:?}", pg.id, pg.items.iter().map(|i| i.rect).collect::<Vec<_>>());
	let (Some(no), true, Some(first)) = (zones.iter().find(|z| z.1 == mouse_menu::TRIANGLE), zones.len() == 2, pg.items.first()) else {
		fails.push("answers: no No/Yes zones on Quit Game".into());
		return;
	};
	set_mode(r, true);
	let aim = |r: &mut Retail, x: f32, y: f32| {
		let (fx, fy) = frac(r, x, y);
		r.mouse_cursor(fx, fy);
	};
	aim(r, first.rect[0] as f32 + 4.0, first.rect[1] as f32 + 4.0);
	r.mouse_click();
	pump(r, 40);
	let still = r.snap.menu.as_ref().map(|m| m.id);
	eprintln!("ANSWERS: a click beside the answers leaves the page: {still:?}");
	if still != Some(0xd) {
		fails.push(format!("answers: a click beside the answers changed the page to {still:?}"));
	}
	let [x0, y0, x1, y1] = no.0;
	aim(r, (x0 + x1) * 0.5, (y0 + y1) * 0.5);
	r.mouse_click();
	let n = until(r, 120, |m| m.id != 0xd);
	eprintln!("ANSWERS: a click on 'No' leaves the page after {n:?} fields");
	if n.is_none() {
		fails.push("answers: a click on 'No' did not go back".into());
	}
}

/// The grid / wheel / slider / question checks of one page (`visit`).
pub(super) fn more_checks(r: &mut Retail, pg: &MenuSnap, fails: &mut Vec<String>) {
	for it in pg.items.iter().filter(|i| i.addr == pg.cur) {
		if let Kind::Grid { rows, cols, .. } = it.kind {
			// The live cursor (earlier checks of this visit moved it; the page snapshot is stale).
			let cursor = r.snap.menu.as_ref().and_then(|m| m.items.iter().find(|i| i.addr == it.addr)).map_or(0, |i| if let Kind::Grid { cursor, .. } = i.kind { cursor } else { 0 });
			let t = if cursor == 0 { rows * cols - 1 } else { 0 };
			if t != cursor && !equality_path(r, &format!("page {:#x} grid {:#x}", pg.id, it.addr), it, t, &grid_path(cols, cursor, t)) {
				fails.push(format!("page {:#x}: grid hover != keys", pg.id));
			}
			// A notch inside the grid (the last row's Down leaves it for the next item, which keys cannot undo exactly).
			if rows > 1 {
				let key = if cursor / cols + 1 < rows { K::ArrowDown } else { K::ArrowUp };
				wheel_equality(r, &format!("page {:#x} grid", pg.id), key, 1, fails);
			}
		}
		if let Kind::Label { ref rows, cursor, .. } = it.kind {
			if rows.len() >= 4 && cursor == 0 && rows.iter().take(4).all(|x| x.1) {
				wheel_equality(r, &format!("page {:#x} list", pg.id), K::ArrowDown, 3, fails);
			}
		}
	}
	if pg.id == mouse_slider::PAGE {
		slider_checks(r, pg, fails);
	}
	if pg.id == mouse_answers::QUIT_GAME {
		answers_checks(r, pg, fails);
	}
}
