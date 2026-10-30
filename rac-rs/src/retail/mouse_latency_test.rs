//! In-game latency table and key-press equality of the mouse's instant selection (`mouse_instant.rs`, lane mmenu, v0.4.2).
//!
//! * latency: on every page of the pause menu the pointer visits the page's targets in a zigzag (first, last, second, ...; long jumps),
//!   once with the old pad pulses (`State::with_instant(false)`) and once instant; the fields from the pointer's arrival until the
//!   snapshot shows the target selected (the snapshot is two fields behind the game, the same in both modes) are counted. A left click
//!   on an unselected root item counts the fields until its page is open.
//! * equality: from the same state the selection is moved by the D-pad keys (Down / Up k times) and by one pointer hover, and after the
//!   menu settled the whole `MenuSnap` (page, current item, every item's cursor / rows / rectangles) must be the same: the hop leaves
//!   the game in the state "the player pressed Down k times".
//!
//! Manual (one game run): `cargo test --release --lib retail::mouse_latency_test -- --ignored --nocapture` (fg1/1_0018, Novalis, 16:9).

use super::mouse_menu::{ring_slot, Item, Kind, MenuSnap, State, Target};
use super::{mouse_answers, mouse_slider};

mod checks;
mod vendor;
use super::*;
use winit::keyboard::KeyCode as K;

fn pump(r: &mut Retail, n: u32) {
	for _ in 0..n {
		r.tick_keys(|_| false, false, false);
	}
}

fn until(r: &mut Retail, max: u32, f: impl Fn(&MenuSnap) -> bool) -> Option<u32> {
	for n in 1..=max {
		r.tick_keys(|_| false, false, false);
		if r.snap.menu.as_ref().is_some_and(|m| !m.busy() && f(m)) {
			return Some(n);
		}
	}
	None
}

fn frac(r: &Retail, x: f32, y: f32) -> (f32, f32) {
	let k = r.aspect / (4.0 / 3.0);
	((x / 512.0 - 0.5) / k + 0.5, y / 448.0)
}

/// The overlay pixel the pointer must be on for target `sub` of `it`.
fn centre(it: &Item, sub: u32) -> (f32, f32) {
	let [x, y, w, h] = it.rect.map(|v| v as f32);
	match &it.kind {
		Kind::Label { rows, .. } => (x + w * 0.5, y + rows.get(sub as usize).map_or(h * 0.5, |r| r.0)),
		Kind::Value { n, .. } => (x + w * 0.5, y + (sub as f32 + 1.0) * h / (*n as f32 + 1.0)),
		Kind::Grid { rows, cols, .. } => (x + ((sub % cols) as f32 + 0.5) * w / *cols as f32, y + ((sub / cols) as f32 + 0.5) * h / *rows as f32),
		Kind::Ring { .. } => ring_slot(it.rect, sub),
		_ => (x + w * 0.5, y + h * 0.5),
	}
}

fn kind_name(k: &Kind) -> &'static str {
	match k {
		Kind::Label { .. } => "list",
		Kind::Value { .. } => "value",
		Kind::Grid { .. } => "grid",
		Kind::Ring { .. } => "ring",
		_ => "other",
	}
}

/// A fresh mouse state with the instant selection on or off (the worker's request is cancelled).
fn set_mode(r: &mut Retail, instant: bool) {
	r.mm = State::with_instant(instant);
	mouse_instant::send(mouse_instant::Cmd::Cancel);
	pump(r, 3);
}

/// The pointer arrives on target `sub` of `it`: fields until the snapshot shows it selected.
fn hover_fields(r: &mut Retail, it: &Item, sub: u32, max: u32) -> Option<u32> {
	let (x, y) = centre(it, sub);
	let (fx, fy) = frac(r, x, y);
	let want = Target::Sub { item: it.addr, sub };
	r.mouse_cursor(fx, fy);
	until(r, max, |m| m.selected(want))
}

/// The targets of the page worth visiting: lists / value rows / grids / ring slots, at most 5 per item, in a zigzag order.
fn targets(m: &MenuSnap) -> Vec<(Item, u32)> {
	let mut all = Vec::new();
	let mut items: Vec<&Item> = m.items.iter().collect();
	items.sort_by_key(|i| (i.rect[1], i.rect[0]));
	for it in items {
		let subs: Vec<u32> = match &it.kind {
			Kind::Label { rows, .. } => (0..rows.len() as u32).filter(|&i| rows[i as usize].1).collect(),
			Kind::Value { n, .. } => (0..*n).collect(),
			Kind::Grid { rows, cols, .. } => vec![0, rows * cols - 1, rows * cols / 2],
			Kind::Ring { .. } => vec![3, 6],
			_ => vec![],
		};
		let mut pick = Vec::new();
		for k in [0, subs.len().wrapping_sub(1), 1, subs.len().wrapping_sub(2), subs.len() / 2] {
			if let Some(&s) = subs.get(k) {
				if !pick.contains(&s) {
					pick.push(s);
				}
			}
		}
		all.extend(pick.into_iter().map(|s| (it.clone(), s)));
	}
	// Zigzag: first, last, second, second to last ...: every step is a long jump.
	let (mut out, mut lo, mut hi) = (Vec::new(), 0usize, all.len());
	while lo < hi {
		out.push(all[lo].clone());
		lo += 1;
		if lo < hi {
			hi -= 1;
			out.push(all[hi].clone());
		}
	}
	out
}

/// Visits the page's targets; returns (kind, fields or None) per target and prints one line per target.
fn latency_page(r: &mut Retail, tag: &str) -> Vec<(&'static str, Option<u32>)> {
	let Some(m) = r.snap.menu.clone() else { return vec![] };
	let mut res = Vec::new();
	for (it, sub) in targets(&m) {
		let n = hover_fields(r, &it, sub, 200);
		let now = r.snap.menu.as_ref().and_then(|m| m.items.iter().find(|i| i.addr == it.addr).cloned()).unwrap_or(it.clone());
		eprintln!("lat:   {tag} {} item {:#x} row {sub}: {n:?} fields", kind_name(&now.kind), it.addr);
		res.push((kind_name(&it.kind), n));
		pump(r, 12);
	}
	res
}

fn summary(page: u32, mode: &str, res: &[(&'static str, Option<u32>)]) {
	for kind in ["list", "value", "grid", "ring", "other"] {
		let v: Vec<Option<u32>> = res.iter().filter(|x| x.0 == kind).map(|x| x.1).collect();
		if v.is_empty() {
			continue;
		}
		let ok: Vec<u32> = v.iter().flatten().copied().collect();
		let mean = ok.iter().sum::<u32>() as f32 / ok.len().max(1) as f32;
		eprintln!("LATENCY page {page:#04x} {mode:7} {kind:5}: {} targets, mean {mean:5.1} fields, max {:3}, unreached {}", v.len(), ok.iter().max().unwrap_or(&0), v.len() - ok.len());
	}
}

/// Pumps until the page's whole snapshot stayed the same for `need` fields; returns its text.
fn settled(r: &mut Retail, need: u32, max: u32) -> String {
	let (mut prev, mut same) = (String::new(), 0);
	for _ in 0..max {
		r.tick_keys(|_| false, false, false);
		let s = format!("{:?}", r.snap.menu);
		same = if s == prev { same + 1 } else { 0 };
		prev = s;
		if same >= need {
			break;
		}
	}
	prev
}

/// A key held 4 fields, then 4 of nothing, `n` times.
fn keys(r: &mut Retail, key: K, n: u32) {
	for _ in 0..n {
		for _ in 0..4 {
			r.tick_keys(|k| k == key, false, false);
		}
		pump(r, 4);
	}
}

/// From the current selection, `steps` rows / items down (negative: up) by keys and by one hover, back by keys: both end states compared.
/// The first target must be selected and `it`'s row `sub` is `steps` D-pad steps away.
fn equality(r: &mut Retail, tag: &str, it: &Item, sub: u32, steps: i32) -> bool {
	set_mode(r, true);
	let key = if steps > 0 { K::ArrowDown } else { K::ArrowUp };
	let back = if steps > 0 { K::ArrowUp } else { K::ArrowDown };
	let start = settled(r, 40, 400);
	keys(r, key, steps.unsigned_abs());
	let by_keys = settled(r, 60, 600);
	keys(r, back, steps.unsigned_abs());
	let again = settled(r, 60, 600);
	let (x, y) = centre(it, sub);
	let (fx, fy) = frac(r, x, y);
	r.mouse_cursor(fx, fy);
	let by_mouse = settled(r, 60, 600);
	let (back_ok, same) = (start == again, by_keys == by_mouse);
	eprintln!("EQUALITY {tag}: {steps:+} steps to row {sub}: keys then back restores the start {back_ok}; hover == keys {same}");
	if !same {
		eprintln!("  keys : {by_keys}\n  mouse: {by_mouse}");
	}
	// Back to the start for the next check (by keys: the exact reverse).
	keys(r, back, steps.unsigned_abs());
	settled(r, 40, 400);
	back_ok && same
}

#[test]
#[ignore]
fn mouse_latency_in_game() {
	unsafe {
		std::env::set_var("RAC_RETAIL_HUD", "0");
		std::env::set_var("RAC_PLAYER_CARD", "0");
	}
	let dir = std::env::var("RAC_TEST_STATE").unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/fg1/1_0018").into());
	enable(&dir).unwrap();
	let mut r = Retail::load().unwrap();
	r.aspect = 16.0 / 9.0;
	pump(&mut r, std::env::var("RAC_TEST_IDLE").ok().and_then(|v| v.parse().ok()).unwrap_or(200));
	for _ in 0..4 {
		r.tick_keys(|k| k == K::Enter, false, false);
	}
	assert!(until(&mut r, 600, |m| m.id == 2).is_some(), "pause menu did not open: mode {}", r.snap.mode);
	pump(&mut r, 30);
	let root = r.snap.menu.clone().unwrap();
	let mut items: Vec<Item> = root.items.iter().filter(|i| matches!(i.kind, Kind::Label { .. })).cloned().collect();
	items.sort_by_key(|i| i.rect[1]);
	let mut fails = Vec::new();
	for it in &items {
		let (x, y) = centre(it, 0);
		let (fx, fy) = frac(&r, x, y);
		eprintln!("ROOT item {:#x} rect {:?} centre ({x:.0},{y:.0}) window fraction ({fx:.4},{fy:.4}) cursor {}", it.addr, it.rect, if let Kind::Label { cursor, .. } = it.kind { cursor } else { 0 });
	}
	// Equality on the root: item 0 -> item 3 (three Down presses) and item 6 -> item 2 (four Up presses).
	set_mode(&mut r, true);
	if hover_fields(&mut r, &items[0], 0, 200).is_none() {
		fails.push("root: item 0 not selected".to_string());
	}
	if items.len() > 3 && !equality(&mut r, "root 0->3", &items[3], 0, 3) {
		fails.push("root 0->3: hover != keys".into());
	}
	// Latency on the root and the click latency, both modes.
	for instant in [false, true] {
		let mode = if instant { "instant" } else { "pulses" };
		set_mode(&mut r, instant);
		summary(2, mode, &latency_page(&mut r, &format!("root {mode}")));
		for (a, b) in [(0usize, 4usize), (6, 1)] {
			let (Some(ia), Some(ib)) = (items.get(a), items.get(b)) else { continue };
			hover_fields(&mut r, ia, 0, 200);
			pump(&mut r, 30);
			let (x, y) = centre(ib, 0);
			let (fx, fy) = frac(&r, x, y);
			r.mouse_cursor(fx, fy);
			r.mouse_click();
			let n = until(&mut r, 400, |m| m.id != 2);
			eprintln!("LATENCY click {mode:7} root item {a} -> item {b}: page opened after {n:?} fields (page {:#x})", r.snap.menu.as_ref().map_or(0, |m| m.id));
			pump(&mut r, 40);
			r.mouse_back();
			if until(&mut r, 400, |m| m.id == 2).is_none() {
				fails.push(format!("click {mode} {a}->{b}: no way back to the root"));
			}
			pump(&mut r, 30);
		}
	}
	// Every page: equality on its first list with >= 4 selectable rows, latency in both modes.
	for it in &items {
		set_mode(&mut r, true);
		hover_fields(&mut r, it, 0, 200);
		pump(&mut r, 10);
		r.mouse_click();
		let Some(_) = until(&mut r, 400, |m| m.id != 2) else {
			eprintln!("page of {:#x}: did not open", it.addr);
			continue;
		};
		pump(&mut r, 60);
		let Some(pg) = r.snap.menu.clone().filter(|m| !m.busy()) else {
			fails.push(format!("page of {:#x}: never idle", it.addr));
			continue;
		};
		visit(&mut r, &pg, &mut fails);
		if pg.id == 8 {
			options_rows(&mut r, &mut fails);
		}
		set_mode(&mut r, true);
		r.mouse_back();
		if until(&mut r, 400, |m| m.id == 2).is_none() {
			fails.push(format!("page {:#x}: right click did not return", pg.id));
			for _ in 0..3 {
				r.tick_keys(|k| k == K::Enter, false, false);
			}
			until(&mut r, 400, |m| m.id == 2);
		}
		pump(&mut r, 20);
	}
	for f in &fails {
		eprintln!("latency FAIL: {f}");
	}
	assert!(r.stop.is_none() && !r.failed);
	assert!(fails.is_empty(), "{} problems (see FAIL lines)", fails.len());
}

/// Equality on the page's first list (>= 4 selectable first rows) or value item (>= 3 rows) that is the current item, then the latency
/// of the pointer on the page's targets in both modes.
fn visit(r: &mut Retail, pg: &MenuSnap, fails: &mut Vec<String>) {
	for i in &pg.items {
		eprintln!("GEOM page {:#x} item {:#x} slot {} rect {:?} {:?}", pg.id, i.addr, i.slot, i.rect, i.kind);
	}
	let pick = pg.items.iter().find(|i| i.addr == pg.cur && match &i.kind {
		Kind::Label { rows, .. } => rows.len() >= 4 && rows.iter().take(4).all(|x| x.1),
		Kind::Value { n, .. } => *n >= 3,
		_ => false,
	});
	if let Some(l) = pick {
		let (c, n) = match l.kind {
			Kind::Label { cursor, ref rows, .. } => (cursor as i32, rows.len().min(4) as i32),
			Kind::Value { cursor, n } => (cursor as i32, n as i32),
			_ => (0, 0),
		};
		let to = if c + 2 < n { c + 2 } else { 0 };
		if to != c && !equality(r, &format!("page {:#x} {} {:#x}", pg.id, kind_name(&l.kind), l.addr), l, to as u32, to - c) {
			fails.push(format!("page {:#x}: hover != keys", pg.id));
		}
	}
	for instant in [false, true] {
		let mode = if instant { "instant" } else { "pulses" };
		set_mode(r, instant);
		let res = latency_page(r, &format!("page {:#04x} {mode}", pg.id));
		summary(pg.id, mode, &res);
	}
	checks::more_checks(r, pg, fails);
	set_mode(r, true);
}

/// The Options page (id 8): every row that opens a page is clicked, the page visited like the others, then the right click back.
fn options_rows(r: &mut Retail, fails: &mut Vec<String>) {
	let Some(opt) = r.snap.menu.clone() else { return };
	let Some((it, rows)) = opt.items.iter().find_map(|i| match &i.kind {
		Kind::Label { rows, .. } if rows.len() >= 3 => Some((i.clone(), rows.iter().map(|x| x.1).collect::<Vec<_>>())),
		_ => None,
	}) else {
		return;
	};
	for (sub, ok) in rows.into_iter().enumerate() {
		if !ok {
			continue;
		}
		set_mode(r, true);
		if hover_fields(r, &it, sub as u32, 200).is_none() {
			fails.push(format!("options row {sub}: not selected"));
			continue;
		}
		pump(r, 10);
		r.mouse_click();
		if until(r, 300, |m| m.id != 8).is_none() {
			pump(r, 30);
			continue;
		}
		pump(r, 60);
		if let Some(pg) = r.snap.menu.clone().filter(|m| !m.busy() && m.id != 8) {
			eprintln!("lat: options row {sub} opened page {:#x}: {:?}", pg.id, pg.items.iter().map(|i| (kind_name(&i.kind), i.addr)).collect::<Vec<_>>());
			visit(r, &pg, fails);
		}
		r.mouse_back();
		if until(r, 400, |m| m.id == 8).is_none() {
			fails.push(format!("options row {sub}: right click did not return"));
			return;
		}
		pump(r, 20);
	}
}
