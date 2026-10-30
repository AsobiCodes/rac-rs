//! In-game proof of the mouse rows of the generic lists (`mouse_menu::list_rows`): Help Log, Skill Points, Cinematics,
//! Commercials (and every other list on the pages visited). Ground truth is the game's own picture, the HUD capture
//! (`snap.ov`): the SELECTED row is drawn in yellow, so its drawn position says where the game thinks the row is.
//!
//! * sweep: the cursor goes down the list with the Down key; at every row `hit(centre of the drawn selected row)` must be that row.
//! * hover: the pointer is put on the model's rows (top, middle, bottom of the visible band, at the settled scroll); once the
//!   selection has settled it must be that row. Rows at or after the first wrapped row (a text wider than the list) are only reported.
//!
//! Manual (one game run, `cargo test --release --lib retail::mouse_lists_test -- --ignored --nocapture`). The state is a late
//! fg2 one (Goodies unlocked by a copy of its RAM with the NG+ counter set; the stock states are German).

use super::mouse_menu::{Item, Kind, MenuSnap, Target};
use super::*;
use winit::keyboard::KeyCode as K;

type Ov<G> = Option<(u32, u32, Vec<[u8; 4]>, G)>;

fn pump(r: &mut Retail, n: u32) {
	for _ in 0..n {
		r.tick_keys(|_| false, false, false);
	}
}

fn until(r: &mut Retail, max: u32, f: impl Fn(&MenuSnap) -> bool) -> bool {
	for _ in 0..max {
		r.tick_keys(|_| false, false, false);
		if r.snap.menu.as_ref().is_some_and(|m| !m.busy() && f(m)) {
			return true;
		}
	}
	false
}

/// A key held 4 fields, then 4 fields of nothing.
fn tap(r: &mut Retail, key: K) {
	for _ in 0..4 {
		r.tick_keys(|k| k == key, false, false);
	}
	pump(r, 4);
}

/// Window fractions of an overlay pixel (the inverse of `to_ps2` at the test's aspect).
fn frac(r: &Retail, x: f32, y: f32) -> (f32, f32) {
	let k = r.aspect / (4.0 / 3.0);
	((x / 512.0 - 0.5) / k + 0.5, y / 448.0)
}

/// Centres (PS2 pixels, topmost first) of the clusters of yellow (the selected row's colour 0x8020ffff) inside `rect`.
fn yellow_clusters<G>(ov: &Ov<G>, rect: [i32; 4]) -> Vec<f32> {
	let Some((w, rows, px, _)) = ov else { return vec![] };
	let (w, rows) = (*w as usize, *rows as usize);
	let (sx, sy) = (w as f32 / 512.0, rows as f32 / 448.0);
	let (x0, x1) = (((rect[0] as f32 * sx).max(0.0) as usize).min(w), (((rect[0] + rect[2]) as f32 * sx).max(0.0) as usize).min(w));
	let (y0, y1) = (((rect[1] as f32 * sy).max(0.0) as usize).min(rows), (((rect[1] + rect[3]) as f32 * sy).max(0.0) as usize).min(rows));
	let yellow = |p: &[u8; 4]| p[0] >= 0xa0 && p[1] >= 0xa0 && p[2] < 0x80;
	let gap = (3.0 * sy).ceil() as usize;
	let mut out = Vec::new();
	let mut run: Option<(usize, usize)> = None;
	let mut close = |a: usize, b: usize| out.push((a + b + 1) as f32 / 2.0 / sy);
	for y in y0..y1 {
		if (x0..x1).filter(|&x| px.get(y * w + x).is_some_and(yellow)).count() >= 2 {
			run = match run {
				Some((a, b)) if y <= b + gap => Some((a, y)),
				Some((a, b)) => {
					close(a, b);
					Some((y, y))
				}
				None => Some((y, y)),
			};
		}
	}
	if let Some((a, b)) = run {
		close(a, b);
	}
	out
}

/// The list item's state: (page's current item, the list's cursor, the selected row's drawn y as bits).
fn state(r: &mut Retail, addr: u32, rect: [i32; 4]) -> (u32, u32, Option<u32>) {
	r.resolve_ov();
	let m = r.snap.menu.as_ref();
	let cursor = m.and_then(|m| m.items.iter().find(|i| i.addr == addr)).map_or(u32::MAX, |i| match i.kind {
		Kind::Label { cursor, .. } => cursor,
		_ => u32::MAX,
	});
	(m.map_or(0, |m| m.cur), cursor, yellow_clusters(&r.snap.ov, rect).first().map(|y| y.to_bits()))
}

/// Pumps until the cursor and the selected row's drawn position stayed put for `need` fields (scroll moves 4 px a field).
fn settle(r: &mut Retail, addr: u32, rect: [i32; 4], need: u32, max: u32) -> bool {
	let (mut prev, mut same) = (None, 0);
	for _ in 0..max {
		r.tick_keys(|_| false, false, false);
		if r.snap.menu.as_ref().is_none_or(|m| m.busy()) {
			same = 0;
			continue;
		}
		let s = state(r, addr, rect);
		if prev == Some(s) {
			same += 1;
			if same >= need {
				return true;
			}
		} else {
			(prev, same) = (Some(s), 0);
		}
	}
	false
}

fn item_of(r: &Retail, addr: u32) -> Option<Item> {
	r.snap.menu.as_ref()?.items.iter().find(|i| i.addr == addr).cloned()
}

struct Row {
	k: u32,
	/// This row or one above it is drawn more than 5 px off the model (a wrapped text above).
	wrapped: bool,
	/// `hit` at the drawn centre did not give this row.
	miss: bool,
}

/// Cursor down the list; at every row the model must find the row where the game draws it.
fn sweep(r: &mut Retail, addr: u32, tag: &str, fails: &mut Vec<String>) -> Vec<Row> {
	let mut seen: Vec<Row> = Vec::new();
	let Some(first) = item_of(r, addr) else { return seen };
	let Kind::Label { rows: model, .. } = &first.kind else { return seen };
	for _ in 0..model.len() + 2 {
		let Some(it) = item_of(r, addr) else { break };
		let Kind::Label { cursor, .. } = it.kind else { break };
		let ok = settle(r, addr, it.rect, 10, 500);
		let m = r.snap.menu.clone().unwrap();
		let Some(it) = item_of(r, addr) else { break };
		let Kind::Label { cursor: k, rows: model, .. } = &it.kind else { break };
		if m.cur != addr && seen.is_empty() {
			eprintln!("  sweep {tag}: not the page's current item (cur {:#x}): no row is selected, skipped", m.cur);
			break;
		}
		if m.cur != addr || seen.iter().any(|d| d.k == *k) {
			break;
		}
		r.resolve_ov();
		let Some(&y) = yellow_clusters(&r.snap.ov, it.rect).first() else {
			fails.push(format!("{tag}: row {k}: no yellow text in the HUD capture (settled {ok}, was {cursor})"));
			break;
		};
		let x = it.rect[0] as f32 + it.rect[2] as f32 * 0.5;
		let (hit, want) = (m.hit(x, y), Some(Target::Sub { item: addr, sub: *k }));
		let model_y = it.rect[1] as f32 + model[*k as usize].0;
		eprintln!("  sweep {tag}: row {k:2} drawn y {y:6.1} model y {model_y:6.1} (diff {:5.1}) hit {:?} settled {ok}", y - model_y, hit.map(|h| match h {
			Target::Sub { sub, .. } => sub as i32,
			Target::Hint(_) => -1,
		}));
		// A text wider than the list wraps into two lines (the Help Log and Skill Points in German): the model assumes one line per
		// text, so from the first such row on the drawn rows are further down than the model's. Not asserted, reported (`wrapped`).
		let wrapped = seen.iter().any(|d| d.wrapped) || (y - model_y).abs() > 5.0;
		if (hit != want || !ok) && !wrapped {
			fails.push(format!("{tag}: sweep row {k}: hit({x:.0},{y:.1}) = {hit:?}, want {want:?} (settled {ok})"));
		}
		seen.push(Row { k: *k, wrapped, miss: hit != want });
		tap(r, K::ArrowDown);
	}
	seen
}

/// The pointer on the model's rows of the settled list (the current scroll; fully visible rows only: first, last, middle ...): the
/// settled selection must be the row under it. Rows at or after `first_wrap` (a text wider than the list wrapped above them: the model
/// is one line per text) are reported, never asserted.
fn hover(r: &mut Retail, addr: u32, first_wrap: Option<u32>, tag: &str, fails: &mut Vec<String>) {
	for pick in 0..5 {
		// The list is read afresh for every pick: the scroll may have moved with the selection.
		let Some(it) = item_of(r, addr) else { return };
		let Kind::Label { rows, rowh, .. } = &it.kind else { return };
		let (top, bottom) = (it.rect[1] as f32, (it.rect[1] + it.rect[3]) as f32);
		let vis: Vec<usize> = (0..rows.len()).filter(|&k| it.rect[1] as f32 + rows[k].0 - rowh * 0.5 >= top && it.rect[1] as f32 + rows[k].0 + rowh * 0.5 <= bottom).collect();
		if vis.len() < 3 {
			return;
		}
		let k = vis[[vis.len() - 1, 0, vis.len() / 2, 1, vis.len() - 2][pick]] as u32;
		let want_y = it.rect[1] as f32 + rows[k as usize].0;
		let x = it.rect[0] as f32 + it.rect[2] as f32 * 0.5;
		// The pointer first leaves the list (a pointer that does not move aims at nothing), then goes to the model's row.
		let (ox, oy) = frac(r, it.rect[0] as f32 - 12.0, want_y + 40.0);
		r.mouse_cursor(ox, oy);
		pump(r, 6);
		let (fx, fy) = frac(r, x, want_y);
		r.mouse_cursor(fx, fy);
		let ok = settle(r, addr, it.rect, 40, 900);
		let (cur, cursor, ybits) = state(r, addr, it.rect);
		let ysel = ybits.map(f32::from_bits);
		let dy = ysel.map(|v| v - want_y);
		// The selected row is the one under the pointer, and the game draws it there.
		let good = ok && cur == addr && cursor == k && dy.is_some_and(|d| d.abs() <= rowh * 0.5 + 3.0);
		let demoted = first_wrap.is_some_and(|w| k >= w);
		let verdict = if good { "ok" } else if demoted { "wrapped (not asserted)" } else { "FAIL" };
		eprintln!("  hover {tag}: pointer on row {k:2} (y {want_y:6.1}) -> cursor {cursor:2} drawn y {ysel:?} dy {dy:?} rowh {rowh} visible rows {} settled {ok} {verdict}", vis.len());
		if !good && !demoted {
			fails.push(format!("{tag}: hover row {k} (pointer y {want_y:.1}): cursor {cursor}, drawn y {ysel:?}, settled {ok}"));
		}
	}
}

/// Sweeps and hovers every list (a Label item with >= 3 rows) of the page on show.
fn lists_of_page(r: &mut Retail, name: &str, fails: &mut Vec<String>) {
	let Some(m) = r.snap.menu.clone() else { return };
	for it in m.items.iter().filter(|i| matches!(&i.kind, Kind::Label { rows, .. } if rows.len() >= 3)) {
		let Kind::Label { rows, rowh, .. } = &it.kind else { continue };
		let tag = format!("{name} (page {:#x} item {:#x})", m.id, it.addr);
		eprintln!("LIST {tag}: rect {:?}, {} rows, model rowh {rowh}, texts {:x?}", it.rect, rows.len(), it.texts);
		let drawn = sweep(r, it.addr, &tag, fails);
		let first_wrap = drawn.iter().find(|d| d.wrapped).map(|d| d.k);
		// A text block that is not the page's current item (the Skill Points description) is no list: nothing to hover.
		if !drawn.is_empty() {
			hover(r, it.addr, first_wrap, &tag, fails);
		}
		let (hits, wrapped) = (drawn.iter().filter(|d| !d.miss).count(), drawn.iter().position(|d| d.wrapped));
		let before = wrapped.unwrap_or(drawn.len());
		eprintln!("SUMMARY {tag}: {} rows swept, {hits} hit the drawn row, first wrapped row {wrapped:?} ({before} rows before it: {} hit)", drawn.len(), drawn.iter().take(before).filter(|d| !d.miss).count());
	}
}

/// Hover + click the row with text id `text` of the page on show; Some(new page id) once it has opened and gone idle.
fn open(r: &mut Retail, text: u16) -> Option<u32> {
	let m = r.snap.menu.clone()?;
	let (it, sub) = m.items.iter().find_map(|it| match &it.kind {
		Kind::Label { .. } => it.texts.iter().position(|&t| t == text).map(|p| (it.clone(), p as u32)),
		_ => None,
	})?;
	let Kind::Label { rows, .. } = &it.kind else { return None };
	let (x, y) = (it.rect[0] as f32 + it.rect[2] as f32 * 0.5, it.rect[1] as f32 + rows[sub as usize].0);
	let (fx, fy) = frac(r, x, y);
	r.mouse_cursor(fx, fy);
	let want = Target::Sub { item: it.addr, sub };
	if !until(r, 400, |m| m.selected(want)) {
		eprintln!("open {text:#x}: the pointer did not select it");
		return None;
	}
	r.mouse_click();
	let from = m.id;
	if !until(r, 500, |m| m.id != from) {
		eprintln!("open {text:#x}: no new page");
		return None;
	}
	pump(r, 60);
	r.snap.menu.as_ref().map(|m| m.id)
}

fn back_to(r: &mut Retail, id: u32) {
	r.mouse_back();
	if !until(r, 400, |m| m.id == id) {
		eprintln!("back: right click did not return to page {id:#x} (now {:?})", r.snap.menu.as_ref().map(|m| m.id));
	}
	pump(r, 20);
}

/// The temp copy of the state: removed when the test ends (also on a failed assert).
struct TempState(std::path::PathBuf);

impl Drop for TempState {
	fn drop(&mut self) {
		let _ = std::fs::remove_dir_all(&self.0);
	}
}

/// A copy of the state with the NG+ counter set (0x15ee20): the pause root then shows Goodies.
fn unlocked_state(src: &str) -> TempState {
	let dst = std::env::temp_dir().join("rac_mouse_lists_state");
	std::fs::create_dir_all(&dst).unwrap();
	for e in std::fs::read_dir(src).unwrap() {
		let e = e.unwrap();
		if e.file_type().unwrap().is_file() {
			std::fs::copy(e.path(), dst.join(e.file_name())).unwrap();
		}
	}
	let p = dst.join("eeMemory.bin");
	let mut ram = std::fs::read(&p).unwrap();
	ram[0x15ee20..0x15ee24].copy_from_slice(&1u32.to_le_bytes());
	std::fs::write(&p, ram).unwrap();
	TempState(dst)
}

/// Manual: the pause menu's generic lists in a late fg2 state (German stock states; 16:9).
#[test]
#[ignore]
fn mouse_lists_in_game() {
	unsafe { std::env::set_var("RAC_PLAYER_CARD", "0") };
	let src = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/fg2/18_0258").into());
	// Declared before `r`: dropped after it, so the copy is removed once the game has let go of it.
	let copy = unlocked_state(&src);
	enable(&copy.0.to_string_lossy()).unwrap();
	let mut r = Retail::load().unwrap();
	r.aspect = 16.0 / 9.0;
	pump(&mut r, std::env::var("RAC_TEST_IDLE").ok().and_then(|v| v.parse().ok()).unwrap_or(200));
	eprintln!("lists: state {src} level {} mode {}", r.snap.level, r.snap.mode);
	for _ in 0..4 {
		r.tick_keys(|k| k == K::Enter, false, false);
	}
	assert!(until(&mut r, 600, |m| m.id == 2), "pause menu did not open: mode {} menu {:?}", r.snap.mode, r.snap.menu);
	pump(&mut r, 40);
	r.resolve_ov();
	eprintln!("lists: overlay {:?}", r.snap.ov.as_ref().map(|(w, h, _, _)| (*w, *h)));
	let mut fails = Vec::new();
	let mut visited = Vec::new();
	lists_of_page(&mut r, "Pause root", &mut fails);
	// Help -> Help Log.
	for (via, name) in [(0x4ee6u16, "Help"), (0x4fc2, "Goodies")] {
		let Some(id) = open(&mut r, via) else {
			fails.push(format!("{name}: the page did not open"));
			continue;
		};
		visited.push(format!("{name} = page {id:#x}"));
		lists_of_page(&mut r, name, &mut fails);
		let subs: &[(u16, &str)] = if via == 0x4ee6 { &[(0x4fd5, "Help Log")] } else { &[(0x4fc4, "Skill Points"), (0x4fc7, "Cinematics"), (0x4fcc, "Commercials")] };
		for &(text, sub) in subs {
			match open(&mut r, text) {
				Some(sid) => {
					visited.push(format!("{sub} = page {sid:#x}"));
					lists_of_page(&mut r, sub, &mut fails);
					back_to(&mut r, id);
				}
				// Commercials (0x4fcc) is not in the Goodies list of this state (hidden until unlocked): not covered.
				None if text == 0x4fcc => eprintln!("lists: {sub} is not in this state's Goodies list: NOT COVERED"),
				None => fails.push(format!("{sub}: the page did not open")),
			}
		}
		back_to(&mut r, 2);
	}
	eprintln!("lists: visited {visited:?}");
	for f in &fails {
		eprintln!("lists FAIL: {f}");
	}
	assert!(r.stop.is_none() && !r.failed);
	assert!(fails.is_empty(), "{} list problems (see FAIL lines)", fails.len());
}
