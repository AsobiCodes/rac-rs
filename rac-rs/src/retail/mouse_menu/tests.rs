//! Tests of `mouse_menu` (moved out of mouse_menu.rs by lane mmenu so the file has room; the module is `mouse_menu::tests`).

use super::*;

fn label(addr: u32, y: i32, rows: usize, cursor: u32) -> Item {
	let h = 35 * (rows as i32 + 1) / 2;
	let rowh = h as f32 / (rows as f32 + 1.0);
	Item { addr, slot: 0, rect: [155, y, 203, h], kind: Kind::Label { rows: (0..rows).map(|i| ((i as f32 + 1.0) * rowh - 1.0, true)).collect(), rowh, cursor }, texts: Vec::new() }
}

fn root() -> MenuSnap {
	MenuSnap { page: 1, id: 2, state: 2, cur: 0x20, front: false, items: vec![label(0x10, 44, 1, 0), label(0x20, 93, 1, 0), label(0x30, 144, 1, 0), label(0x40, 193, 1, 0)] }
}

#[test]
fn cursor_mapping_follows_the_overlay_transform() {
	// 1280x720: the 4:3 picture is centred, its left edge is 1/8 of the window width in.
	let (x, y) = to_ps2(0.5, 0.5, 1280.0 / 720.0, None);
	assert!((x - 256.0).abs() < 1e-3 && (y - 224.0).abs() < 1e-3, "{x} {y}");
	let (x, _) = to_ps2(0.0, 0.5, 1280.0 / 720.0, None);
	assert!((x + 85.333).abs() < 0.01, "{x}");
	let (x, y) = to_ps2(0.125, 1.0, 1280.0 / 720.0, None);
	assert!(x.abs() < 0.01 && (y - 448.0).abs() < 1e-3, "{x} {y}");
	// 4:3 window: identity scale.
	let (x, y) = to_ps2(0.25, 0.75, 4.0 / 3.0, None);
	assert!((x - 128.0).abs() < 1e-3 && (y - 336.0).abs() < 1e-3);
	// GS-full letterbox: 16:9 window, 4:3 picture centred at full height.
	let (x, _) = to_ps2(0.125, 0.5, 16.0 / 9.0, Some(4.0 / 3.0));
	assert!(x.abs() < 0.01, "{x}");
	// A wide picture (aspect 16:9) in a 4:3 window: letterboxed top and bottom.
	let (_, y) = to_ps2(0.5, 0.5 - 0.5 * (4.0 / 3.0) / (16.0 / 9.0), 4.0 / 3.0, Some(16.0 / 9.0));
	assert!(y.abs() < 0.01, "{y}");
}

#[test]
fn hit_testing_buttons_rows_and_hints() {
	let mut m = root();
	assert_eq!(m.hit(200.0, 60.0), Some(Target::Sub { item: 0x10, sub: 0 }));
	assert_eq!(m.hit(200.0, 110.0), Some(Target::Sub { item: 0x20, sub: 0 }));
	assert_eq!(m.hit(200.0, 85.0), None);
	assert_eq!(m.hit(100.0, 60.0), None);
	// A 4-row list: rows are picked by their bands.
	let l = label(0x50, 300, 4, 0);
	let Kind::Label { rows, .. } = &l.kind else { unreachable!() };
	let (y1, y3) = (300.0 + rows[1].0, 300.0 + rows[3].0);
	m.items.push(l);
	assert_eq!(m.hit(200.0, y1), Some(Target::Sub { item: 0x50, sub: 1 }));
	assert_eq!(m.hit(200.0, y3 + 1.0), Some(Target::Sub { item: 0x50, sub: 3 }));
	// Hint line, with a margin; a Value page and the ring.
	m.items.push(Item { addr: 0x60, slot: 5, rect: [20, 400, 80, 14], kind: Kind::Hint(TRIANGLE), texts: Vec::new() });
	assert_eq!(m.hit(18.0, 402.0), Some(Target::Hint(TRIANGLE)));
	m.items.push(Item { addr: 0x70, slot: 3, rect: [400, 100, 100, 90], kind: Kind::Value { n: 2, cursor: 0 }, texts: Vec::new() });
	assert_eq!(m.hit(450.0, 100.0 + 30.0), Some(Target::Sub { item: 0x70, sub: 0 }));
	assert_eq!(m.hit(450.0, 100.0 + 62.0), Some(Target::Sub { item: 0x70, sub: 1 }));
	let ring = Item { addr: 0x80, slot: 1, rect: [100, 100, 300, 300], kind: Kind::Ring { cursor: 0 }, texts: Vec::new() };
	let (sx, sy) = ring_slot(ring.rect, 2);
	let m2 = MenuSnap { items: vec![ring], ..root() };
	assert_eq!(m2.hit(sx + 3.0, sy - 2.0), Some(Target::Sub { item: 0x80, sub: 2 }));
	assert_eq!(m2.hit(250.0, 250.0), None);
	assert!(ring_slot([100, 100, 300, 300], 0).1 < 250.0 - 100.0);
}

/// A toy game: the pad moves the current item up / down the list, Cross is counted, one button per pulse.
fn play(m: &mut MenuSnap, b: u16, crossed: &mut u32) {
	let order = [0x10u32, 0x20, 0x30, 0x40];
	let i = order.iter().position(|&a| a == m.cur).unwrap();
	if b & DOWN != 0 {
		m.cur = order[(i + 1) % 4];
	}
	if b & UP != 0 {
		m.cur = order[(i + 3) % 4];
	}
	if b & CROSS != 0 {
		*crossed += 1;
	}
}

#[test]
fn hover_steers_the_selection_and_click_confirms() {
	let mut m = root();
	let mut st = State::default();
	let (mut crossed, mut presses) = (0, Vec::new());
	fn run(st: &mut State, m: &mut MenuSnap, fields: u32, presses: &mut Vec<u32>, crossed: &mut u32) {
		let mut prev = 0u16;
		for f in 0..fields {
			let b = st.step(Some(&*m), 0);
			if b & !prev != 0 {
				presses.push(f);
				play(m, b, crossed);
			}
			prev = b;
		}
	}
	// No mouse movement: nothing happens, whatever lies under the cursor.
	st.pos = Some((200.0, 60.0));
	run(&mut st, &mut m, 20, &mut presses, &mut crossed);
	assert!(presses.is_empty() && m.cur == 0x20);
	// Hover the first button: Up once.
	st.cursor(200.0, 61.0);
	run(&mut st, &mut m, 30, &mut presses, &mut crossed);
	assert_eq!(m.cur, 0x10);
	assert_eq!(presses.len(), 1);
	// Hover the last one: Down x3 (the toy list wraps, the rect direction says Down); this toy takes each press at once, so
	// every step ends after one released field: presses 2 fields apart (PRESS 1 + the landed released field).
	presses.clear();
	st.cursor(200.0, 200.0);
	run(&mut st, &mut m, 60, &mut presses, &mut crossed);
	assert_eq!(m.cur, 0x40);
	assert_eq!(presses.len(), 3);
	assert!(presses.windows(2).all(|w| w[1] - w[0] == 2), "{presses:?}");
	assert_eq!(crossed, 0);
	// A click on no item confirms the highlighted one (Cross, like Enter); a click on an item selects it and presses Cross.
	st.cursor(10.0, 10.0);
	st.click();
	run(&mut st, &mut m, 20, &mut presses, &mut crossed);
	assert_eq!((m.cur, crossed), (0x40, 1));
	st.cursor(200.0, 110.0);
	st.click();
	run(&mut st, &mut m, 80, &mut presses, &mut crossed);
	assert_eq!((m.cur, crossed), (0x20, 2));
	// The keyboard takes over: a held menu button cancels a goal in progress.
	st.cursor(200.0, 200.0);
	let b = st.step(Some(&m), CROSS);
	assert_eq!(b, 0);
	assert!(st.goal.is_none());
	// No menu: events are dropped, nothing is pressed.
	st.click();
	assert_eq!(st.step(None, 0), 0);
	assert!(!st.click);
}

/// The viewer's timing (retail.rs `tick_keys`): the snap a step sees is 2 fields old. A game that takes the press in the
/// field it arrives steps every 2 fields and never overshoots; a game that ignores presses keeps the full 3-field pulses.
#[test]
fn steering_follows_the_game_without_overshoot() {
	for (takes, gap) in [(true, 2u32), (false, 3)] {
		let mut m = root();
		let mut st = State::default();
		let mut seen: std::collections::VecDeque<MenuSnap> = [m.clone(), m.clone()].into();
		let (mut prev, mut presses, mut crossed) = (0u16, Vec::new(), 0);
		st.cursor(200.0, 200.0);
		for f in 0..60u32 {
			let b = st.step(Some(&seen[0]), 0);
			if b & !prev != 0 {
				presses.push(f);
				if takes {
					play(&mut m, b, &mut crossed);
				}
			}
			prev = b;
			seen.pop_front();
			seen.push_back(m.clone());
		}
		if takes {
			assert_eq!(m.cur, 0x40, "reached, no overshoot");
			assert_eq!(presses.len(), 2, "{presses:?}");
		} else {
			assert!(presses.len() >= 3, "{presses:?}");
		}
		assert!(presses.windows(2).all(|w| w[1] - w[0] == gap), "takes {takes}: {presses:?}");
	}
}

/// A hover made while a page flies in is kept for when it is idle; a click then is dropped.
#[test]
fn hover_waits_for_a_page_flying_in() {
	let mut m = root();
	m.state = 1;
	let mut st = State::default();
	st.cursor(200.0, 61.0);
	st.click();
	for _ in 0..5 {
		assert_eq!(st.step(Some(&m), 0), 0);
	}
	m.state = 2;
	assert_eq!(st.step(Some(&m), 0), UP, "the hover steers once the page is idle");
	assert!(!st.goal.unwrap().confirm, "the click during the fly-in was dropped");
}

#[test]
fn ring_and_grid_steering() {
	let ring = Item { addr: 0x80, slot: 1, rect: [100, 100, 300, 300], kind: Kind::Ring { cursor: 1 }, texts: Vec::new() };
	let grid = Item { addr: 0x90, slot: 2, rect: [0, 0, 100, 100], kind: Kind::Grid { rows: 3, cols: 4, cursor: 0 }, texts: Vec::new() };
	let m = MenuSnap { id: 5, state: 2, cur: 0x90, items: vec![ring, grid], ..Default::default() };
	assert_eq!(m.toward(Target::Sub { item: 0x80, sub: 3 }), Some(R1));
	assert_eq!(m.toward(Target::Sub { item: 0x80, sub: 6 }), Some(L1));
	assert_eq!(m.toward(Target::Sub { item: 0x90, sub: 6 }), Some(DOWN));
	assert!(m.selected(Target::Sub { item: 0x80, sub: 1 }));
	assert!(m.selected(Target::Sub { item: 0x90, sub: 0 }));
	assert_eq!(m.hit(60.0, 10.0), Some(Target::Sub { item: 0x90, sub: 2 }));
	assert_eq!(m.hit(10.0, 90.0), Some(Target::Sub { item: 0x90, sub: 8 }));
}

#[test]
fn panels_sideways_steps_and_stuck_goals() {
	let panel = |addr, y| Item { addr, slot: 2, rect: [100, y, 138, 60], kind: Kind::Panel, texts: Vec::new() };
	let list = Item { addr: 0x90, slot: 3, rect: [300, 100, 120, 200], kind: Kind::Value { n: 3, cursor: 0 }, texts: Vec::new() };
	let m = MenuSnap { id: 4, state: 4, cur: 0x20, items: vec![panel(0x10, 80), panel(0x20, 200), panel(0x30, 280), list], ..Default::default() };
	assert_eq!(m.hit(150.0, 300.0), Some(Target::Sub { item: 0x30, sub: 0 }));
	assert_eq!(m.toward(Target::Sub { item: 0x30, sub: 0 }), Some(DOWN));
	assert_eq!(m.toward(Target::Sub { item: 0x10, sub: 0 }), Some(UP));
	assert!(m.selected(Target::Sub { item: 0x20, sub: 0 }));
	// From a Value item a sideways goal gets no Left / Right (it would change the value) ...
	let v = MenuSnap { cur: 0x90, ..m.clone() };
	assert_eq!(v.toward(Target::Sub { item: 0x10, sub: 0 }), None);
	// ... and a goal that the game does not follow is dropped after three presses.
	let mut st = State::default();
	st.cursor(150.0, 300.0);
	let mut pressed = 0;
	for _ in 0..200 {
		if st.step(Some(&m), 0) != 0 {
			pressed += 1;
		}
	}
	// One field of press per pulse; the first press plus three repeats that change nothing.
	assert!((3..=5).contains(&pressed), "{pressed}");
	assert!(st.goal.is_none());
}

/// The Help page (id 7) as Novalis lays it out: the description panel beside the list is a text item (flags 0x90, u32
/// text ids per row, stride 4), not a list of rows, so hovering it is no goal; the list (flags 0x1000) keeps its rows.
#[test]
fn help_description_panel_is_not_a_row() {
	let mut w = std::collections::HashMap::<u32, u32>::new();
	let (pp, page, desc, list) = (0x1ba0f4u32, 0x1b3d98u32, 0x1b3f40u32, 0x1b3f98u32);
	w.insert(pp, page);
	w.insert(pp - 4, 7);
	w.insert(page + 0x3c, 7);
	w.insert(page + 0x40, list);
	w.insert(page + 0x48, desc);
	w.insert(page + 0x50, list);
	for (it, rect) in [(desc, [21, 134, 155, 148]), (list, [195, 91, 123, 235])] {
		for (k, v) in rect.into_iter().enumerate() {
			w.insert(it + 0x18 + 4 * k as u32, v);
		}
	}
	// Description: Help Log / Controls / Moves / Weapons / Gadgets texts, then the zero id and the next item's code words.
	for (k, t) in [0x4fd6, 0x4efd, 0x4efe, 0x4eff, 0x4f00, 0, 0x28_d0a0, 0x28_d648].into_iter().enumerate() {
		w.insert(0x1b3f28 + 4 * k as u32, t);
	}
	w.extend([(desc, 0x28_d0a0), (desc + 0x30, 0x90), (desc + 0x34, 0x1b3f28), (desc + 0x38, 4)]);
	// List: five type-3 records.
	for (k, t) in [0x4fd5u32, 0x4ef1, 0x4ef2, 0x4ee2, 0x4ee3].into_iter().enumerate() {
		w.insert(0x1b3ee0 + 12 * k as u32, t | 3 << 16);
	}
	w.extend([(list, 0x28_df18), (list + 0x30, 0x1000), (list + 0x34, 0x1b3ee0)]);
	let rd = |a: u32| w.get(&a).copied().unwrap_or(0);
	let m = read_ptr(&rd, pp).unwrap();
	assert_eq!(m.item(desc).map(|i| &i.kind), Some(&Kind::Other));
	assert!(matches!(&m.item(list).unwrap().kind, Kind::Label { rows, .. } if rows.len() == 5 && rows.iter().all(|r| r.1)));
	assert_eq!(m.hit(98.0, 193.0), None);
	assert_eq!(m.hit(256.0, 91.0 + 116.5), Some(Target::Sub { item: list, sub: 2 }));
}

/// The Eudora pause menu savestate: the root page and its six buttons read out of a real RAM image.
#[test]
fn reads_the_pause_menu_from_ram() {
	let Ok(ram) = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/../oracle/states/s13/eeMemory.bin")) else { return };
	let rd = |a: u32| u32::from_le_bytes(ram[(a & MASK) as usize..][..4].try_into().unwrap());
	assert_eq!(read(&rd, 4, 0), None);
	let m = read(&rd, 4, MODE).unwrap();
	assert_eq!((m.id, m.state, m.cur), (2, 2, 0x1b2948));
	assert_eq!(m.items.len(), 6, "{:?}", m.items);
	assert!(m.items.iter().all(|i| matches!(&i.kind, Kind::Label { rows, .. } if rows.len() == 1)));
	assert_eq!(m.items[0].rect, [155, 44, 203, 35]);
	// Hovering the first button at its centre, in a 1280x720 window.
	let (x, y) = to_ps2((155.0 + 100.0) / 512.0 * 0.75 + 0.125, (44.0 + 17.0) / 448.0, 1280.0 / 720.0, None);
	assert_eq!(m.hit(x, y), Some(Target::Sub { item: 0x1b27b8, sub: 0 }));
	assert_eq!(m.toward(Target::Sub { item: 0x1b27b8, sub: 0 }), Some(UP));
}

/// `Retail::picture_point` (the mods' `omni.pointer`) is this map over `SCREEN`: the 4:3 picture's corners are 0..1.
#[test]
fn picture_corners_are_zero_and_one() {
	let frac = |fx: f32, fy: f32, aspect: f32, full: Option<f32>| {
		let (x, y) = to_ps2(fx, fy, aspect, full);
		[x / SCREEN.0, y / SCREEN.1]
	};
	// A 16:9 window, 4:3 picture in the middle (uv x in [0.125, 0.875]): the window's centre is the picture's.
	let wide = 16.0 / 9.0;
	let (l, r) = (0.5 - 0.5 * (4.0 / 3.0) / wide, 0.5 + 0.5 * (4.0 / 3.0) / wide);
	let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4;
	assert!(near(frac(l, 0.0, wide, None), [0.0, 0.0]) && near(frac(r, 1.0, wide, None), [1.0, 1.0]) && near(frac(0.5, 0.5, wide, None), [0.5, 0.5]));
	assert!(frac(0.0, 0.5, wide, None)[0] < 0.0, "left of the picture is below 0");
	// A 4:3 window: the whole window is the picture.
	assert!(near(frac(0.0, 0.0, 4.0 / 3.0, None), [0.0, 0.0]) && near(frac(1.0, 1.0, 4.0 / 3.0, None), [1.0, 1.0]));
	// GS-full (picture aspect given): letterboxed at the largest centred size.
	assert!(near(frac(l, 0.0, wide, Some(4.0 / 3.0)), [0.0, 0.0]) && near(frac(r, 1.0, wide, Some(4.0 / 3.0)), [1.0, 1.0]));
}

/// A list item for the toy games below: `n` rows 33 px apart (the Galactic Map's pitch) in an item at (20, 20).
fn rows33(n: u32, cursor: u32) -> MenuSnap {
	let rows = (0..n).map(|i| (20.5 + 33.0 * i as f32, true)).collect();
	let kind = Kind::Label { rows, rowh: 33.0, cursor };
	MenuSnap { page: 1, id: crate::retail::menu_keys::GALAXY, state: 2, cur: 0x10, front: false, items: vec![Item { addr: 0x10, slot: 0, rect: [20, 20, 144, 33 * n as i32 + 8], kind, texts: Vec::new() }] }
}

fn row_y(i: u32) -> f32 {
	20.0 + 20.5 + 33.0 * i as f32
}

fn list_cursor(m: &MenuSnap) -> u32 {
	MenuSnap::cursor_of(&m.items[0]).unwrap()
}

/// The toy list game: a press moves the cursor `step` rows (1 = as retail), clamped to the list.
fn list_play(m: &mut MenuSnap, b: u16, step: u32) {
	let n = match &m.items[0].kind {
		Kind::Label { rows, .. } => rows.len() as u32,
		_ => unreachable!(),
	};
	if let Kind::Label { cursor, .. } = &mut m.items[0].kind {
		if b & DOWN != 0 {
			*cursor = (*cursor + step).min(n - 1);
		}
		if b & UP != 0 {
			*cursor = cursor.saturating_sub(step);
		}
	}
}

/// Runs `fields` fields with no keys: the pointer shakes `shake` px sideways every field, presses are collected on their rising edge.
fn run_list(st: &mut State, m: &mut MenuSnap, fields: u32, at_row: u32, shake: f32, step: u32, takes: bool) -> Vec<u16> {
	let (mut prev, mut presses) = (0u16, Vec::new());
	for f in 0..fields {
		st.cursor(100.0 + (f % 2) as f32 * shake, row_y(at_row));
		let b = st.step(Some(&*m), 0);
		if b & !prev != 0 {
			presses.push(b & !prev);
			if takes {
				list_play(m, b & !prev, step);
			}
		}
		prev = b;
	}
	presses
}

/// GitHub #16: the pointer rests on a row and shakes by a pixel or two; the player presses Down on the keyboard. The keys win:
/// the mouse adds nothing while they work, a shake under `KEY_MOVE_PX` after the last key aims nowhere, and a real move steers.
#[test]
fn keys_win_over_a_shaking_pointer() {
	let mut m = rows33(12, 7);
	let mut st = State::default();
	st.cursor(100.0, row_y(7));
	let (mut mouse, mut last_kbd) = (0u16, 0u16);
	for f in 0..80u32 {
		st.cursor(100.0 + (f % 2) as f32 * 1.5, row_y(7));
		// Three Down taps of two fields each; the game takes a key in the field it goes down.
		let kbd = if [10u32, 20, 30].iter().any(|&t| (t..t + 2).contains(&f)) { DOWN } else { 0 };
		if kbd & !last_kbd != 0 {
			list_play(&mut m, kbd, 1);
		}
		last_kbd = kbd;
		mouse |= st.step(Some(&m), kbd);
	}
	assert_eq!((mouse, list_cursor(&m)), (0, 10), "every key press stays");
	// The pointer leaves the anchor by less than KEY_MOVE_PX: still the keyboard's menu.
	st.cursor(100.0, row_y(7) + 5.0);
	assert!((0..30).all(|_| st.step(Some(&m), 0) == 0));
	assert_eq!(list_cursor(&m), 10);
	// A real move to row 4: the mouse steers there (Up x6) and stops.
	let presses = run_list(&mut st, &mut m, 120, 4, 0.0, 1, true);
	assert_eq!((list_cursor(&m), presses.len()), (4, 6), "{presses:?}");
}

/// A game that moves two rows a press passes an odd goal: the next step would reverse, so the mouse gives up there instead of
/// swinging over it, and a pointer that shakes 6 px every field (a new aim each field) does not start it again.
#[test]
fn an_overshooting_game_gives_up_after_the_reversal() {
	let mut m = rows33(12, 0);
	let mut st = State::default();
	let presses = run_list(&mut st, &mut m, 300, 3, 6.0, 2, true);
	assert_eq!((presses.clone(), list_cursor(&m)), (vec![DOWN, DOWN], 4), "{presses:?}");
	assert!(st.gave_up && st.goal.is_none());
	// Another page starts over.
	let other = MenuSnap { id: 0xc, ..m.clone() };
	st.step(Some(&other), 0);
	assert!(!st.gave_up && st.hover.is_none() && st.spent == 0);
}

/// A game that ignores the presses is pressed three times, not for as long as the pointer shakes.
#[test]
fn a_deaf_game_is_pressed_three_times_only() {
	let mut m = rows33(12, 0);
	let mut st = State::default();
	let presses = run_list(&mut st, &mut m, 400, 5, 6.0, 1, false);
	assert_eq!(presses.len(), 3, "{presses:?}");
	assert!(st.gave_up);
	// A click is a new try.
	st.click();
	let presses = run_list(&mut st, &mut m, 100, 5, 0.0, 1, false);
	assert_eq!(presses.len(), 3, "{presses:?}");
}

/// A click made while a key is down waits for the key (and goes out then); one that waits longer than PEND_FIELDS is dropped.
#[test]
fn a_click_waits_for_the_keys() {
	for (down_fields, crossed_end, cursor_end) in [(10u32, 1u32, 3u32), (40, 0, 7)] {
		let mut m = rows33(12, 7);
		let mut st = State::default();
		st.cursor(100.0, row_y(3));
		st.click();
		let (mut crossed, mut prev) = (0u32, 0u16);
		for f in 0..150u32 {
			let kbd = if f < down_fields { DOWN } else { 0 };
			let b = st.step(Some(&m), kbd);
			assert!(kbd == 0 || b == 0, "the mouse presses nothing while a key is down");
			if b & !prev != 0 {
				if b & CROSS != 0 {
					crossed += 1;
				} else {
					list_play(&mut m, b & !prev, 1);
				}
			}
			prev = b;
		}
		assert_eq!((crossed, list_cursor(&m)), (crossed_end, cursor_end), "keys down for {down_fields} fields");
	}
}

/// A key cuts a steering pulse in flight at once; the pulse of a click is let finish.
#[test]
fn a_key_cuts_a_steering_pulse_but_not_a_click() {
	let m = rows33(12, 0);
	let mut st = State::default();
	st.cursor(100.0, row_y(5));
	// PRESS is one field: the pulse is in flight (its release) in the next one.
	assert_eq!(st.step(Some(&m), 0), DOWN);
	assert_eq!(st.step(Some(&m), UP), 0, "the steering pulse is cut");
	assert!((0..30).all(|_| st.step(Some(&m), 0) == 0), "and nothing aims again without a move");
	// A click off every item presses Cross for the highlighted one; a key then does not cancel that press.
	st.cursor(5.0, 5.0);
	st.click();
	assert_eq!(st.step(Some(&m), 0), CROSS);
	// PRESS is one field: the key in the release field adds nothing (and does not start a second press).
	assert_eq!(st.step(Some(&m), DOWN), 0);
}

#[test]
fn the_left_stick_is_a_menu_direction() {
	assert_eq!(stick_bits([0x80; 4]), 0);
	assert_eq!(stick_bits([0, 0, 0x80, 0x80]), 0, "the right stick is not a menu input");
	assert_eq!(stick_bits([0x80, 0x80, 0x80, 0x00]), UP);
	assert_eq!(stick_bits([0x80, 0x80, 0x80, 0xff]), DOWN);
	assert_eq!(stick_bits([0x80, 0x80, 0x00, 0x80]), LEFT);
	assert_eq!(stick_bits([0x80, 0x80, 0xff, 0x80]), RIGHT);
	assert_eq!(stick_bits([0x80, 0x80, 0x00, 0xff]), LEFT | DOWN);
	// Retail's pad writer: a direction when |byte - 0x7f| > 0x2f, i.e. byte <= 0x4f or >= 0xaf.
	assert_eq!(stick_bits([0x80, 0x80, 0x50, 0xae]), 0, "0x50 and 0xae are still idle");
	assert_eq!(stick_bits([0x80, 0x80, 0x4f, 0xaf]), LEFT | DOWN, "0x4f and 0xaf are directions");
	assert_eq!(stick_bits([0x80, 0x80, 0x40, 0xc0]), LEFT | DOWN, "a half push is one");
	assert_ne!(stick_bits([0x80, 0x80, 0x80, 0x00]) & MENU_BTNS, 0);
}

/// The stick alone (WASD on the maps, a gamepad) works the menu through `step` like a key: a steering pulse is cut, the goal
/// dropped, nothing aims again without a real move.
#[test]
fn the_stick_alone_takes_the_menu_from_the_mouse() {
	let m = rows33(12, 0);
	let mut st = State::default();
	st.cursor(100.0, row_y(5));
	assert_eq!(st.step(Some(&m), 0), DOWN);
	let down = stick_bits([0x80, 0x80, 0x80, 0xff]);
	assert_eq!(down, DOWN);
	assert_eq!(st.step(Some(&m), down), 0, "the pulse is cut by the pushed stick");
	assert!((0..30).all(|_| st.step(Some(&m), down) == 0), "and nothing is pressed while it is pushed");
	assert!((0..30).all(|_| st.step(Some(&m), stick_bits([0x80; 4])) == 0), "nor after it is released");
	// A stick on the edge of retail's threshold (0xaf-1 = 0xae, still idle) does not hold the menu: the pointer's move steers again.
	st.cursor(100.0, row_y(8));
	assert_eq!(st.step(Some(&m), stick_bits([0x80, 0x80, 0x80, 0xae])), DOWN);
}

/// A right click or wheel notch made while a key is down waits for the key and goes out after it; one that waits longer than
/// `PEND_FIELDS` is dropped.
#[test]
fn right_click_and_wheel_wait_for_the_keys_and_are_dropped_after_pend_fields() {
	for (down_fields, fires) in [(10u32, true), (40, false)] {
		let m = rows33(12, 4);
		let mut st = State::default();
		st.cursor(5.0, 5.0);
		st.back();
		let out: Vec<u16> = (0..120).map(|f| st.step(Some(&m), if f < down_fields { DOWN } else { 0 })).collect();
		assert!(out[..down_fields as usize].iter().all(|&b| b == 0), "the mouse presses nothing while a key is down");
		assert_eq!(out.iter().any(|&b| b == m.back_button()), fires, "right click after {down_fields} key fields: {out:?}");
		st.wheel(-1.0);
		let out: Vec<u16> = (0..120).map(|f| st.step(Some(&m), if f < down_fields { DOWN } else { 0 })).collect();
		assert!(out[..down_fields as usize].iter().all(|&b| b == 0));
		assert_eq!(out.iter().any(|&b| b == DOWN), fires, "wheel after {down_fields} key fields: {out:?}");
	}
}

/// A click, right click or wheel notch waiting for the keys is dropped when the keys change the page: it was meant for the page
/// that is gone.
#[test]
fn a_pending_click_does_not_survive_a_page_change() {
	let (m, other) = (rows33(12, 4), MenuSnap { id: 0xc, ..rows33(12, 4) });
	for what in 0..3 {
		let mut st = State::default();
		st.cursor(100.0, row_y(4));
		assert_eq!(st.step(Some(&m), 0), 0);
		match what {
			0 => st.click(),
			1 => st.back(),
			_ => st.wheel(1.0),
		}
		assert_eq!(st.step(Some(&m), DOWN), 0);
		// The key moved the game to another page; the key is still down, then up.
		assert_eq!(st.step(Some(&other), DOWN), 0);
		let out: Vec<u16> = (0..60).map(|_| st.step(Some(&other), 0)).collect();
		assert!(out.iter().all(|&b| b == 0), "pending {what} fired on the new page: {out:?}");
	}
}

/// Keys start a new episode: presses the mouse spent on the hovered row before do not use up its `MAX_PRESSES`.
#[test]
fn keys_reset_the_press_budget_of_the_hovered_row() {
	let (m, mut st) = (rows33(12, 0), State::default());
	st.cursor(100.0, row_y(5));
	st.step(Some(&m), 0);
	assert!(st.spent > 0);
	st.step(Some(&m), DOWN);
	assert_eq!(st.spent, 0);
}

/// The wheel moves the selection off the pointer's row on purpose; the click of the notch nudges the pointer by a pixel or two,
/// which must not aim back at the old row (the #16 "jumps back", wheel flavour).
#[test]
fn a_wheel_notch_and_a_nudged_pointer_do_not_aim_back() {
	let mut m = rows33(12, 3);
	let mut st = State::default();
	st.cursor(100.0, row_y(5));
	let mut prev = 0u16;
	let mut run = |st: &mut State, m: &mut MenuSnap, fields: u32, presses: &mut Vec<u16>| {
		for _ in 0..fields {
			let b = st.step(Some(&*m), 0);
			if b & !prev != 0 {
				presses.push(b & !prev);
				list_play(m, b & !prev, 1);
			}
			prev = b;
		}
	};
	let mut presses = Vec::new();
	run(&mut st, &mut m, 40, &mut presses);
	assert_eq!((list_cursor(&m), presses.len()), (5, 2), "the pointer steered to row 5: {presses:?}");
	// The pointer drifts 3 px (under JITTER_PX: no aim), the wheel goes down one row, the pointer drifts 2 px more.
	st.cursor(103.0, row_y(5));
	run(&mut st, &mut m, 4, &mut presses);
	st.wheel(-1.0);
	run(&mut st, &mut m, 12, &mut presses);
	assert_eq!(list_cursor(&m), 6, "the wheel moved down one row: {presses:?}");
	st.cursor(105.0, row_y(5));
	presses.clear();
	run(&mut st, &mut m, 60, &mut presses);
	assert_eq!((list_cursor(&m), presses.len()), (6, 0), "nothing steers back: {presses:?}");
}

/// Two items: the first holds the selection, the goal is a row of the second. The first press (Down) lands the second item's
/// cursor anywhere (row 4 here, below the goal), so the Up that follows is no overshoot and steering does not give up.
#[test]
fn steering_from_item_to_item_does_not_give_up_on_the_reversal() {
	let rect = |y, h| [20, y, 144, h];
	let two = |cur: u32, cursor: u32| {
		let a = Item { addr: 0x10, slot: 0, rect: rect(20, 40), kind: Kind::Label { rows: vec![(20.5, true)], rowh: 33.0, cursor: 0 }, texts: Vec::new() };
		let rows = (0..6).map(|i| (20.5 + 33.0 * i as f32, true)).collect();
		let b = Item { addr: 0x20, slot: 1, rect: rect(100, 210), kind: Kind::Label { rows, rowh: 33.0, cursor }, texts: Vec::new() };
		MenuSnap { page: 1, id: crate::retail::menu_keys::GALAXY, state: 2, cur, items: vec![a, b], front: false }
	};
	let mut m = two(0x10, 0);
	let mut st = State::default();
	st.cursor(100.0, 100.0 + 20.5 + 33.0);
	let (mut prev, mut presses) = (0u16, Vec::new());
	for _ in 0..200 {
		let b = st.step(Some(&m), 0);
		if b & !prev != 0 {
			presses.push(b & !prev);
			if b & DOWN != 0 && m.cur == 0x10 {
				m = two(0x20, 4);
			} else if b & UP != 0 && m.cur == 0x20 {
				if let Kind::Label { cursor, .. } = &mut m.items[1].kind {
					*cursor = cursor.saturating_sub(1);
				}
			}
		}
		prev = b;
	}
	assert_eq!((presses.clone(), MenuSnap::cursor_of(&m.items[1]), m.cur), (vec![DOWN, UP, UP, UP], Some(1), 0x20));
	assert!(!st.gave_up, "{presses:?}");
}

/// The Galactic Map's planet list (GitHub #16) as the generic list draw lays it out: 18 records {text 0x4eb9 + p, type 1, text2
/// 0x4ecc + p}, flags 0x138, rect (20, 86, 144, 251), scroll `scroll`; `sig` = the item's draw function is the generic one.
fn galaxy(scroll: u32, sig: bool) -> MenuSnap {
	galaxy_n(18, scroll, sig)
}

fn galaxy_n(n: u32, scroll: u32, sig: bool) -> MenuSnap {
	let mut w = std::collections::HashMap::<u32, u32>::new();
	let (pp, page, list, data, draw) = (0x1ba0f4u32, 0x1b3d98u32, 0x1b3f98u32, 0x1b5000u32, 0x28_df18u32);
	w.extend([(pp, page), (pp - 4, 2), (page + 0x3c, crate::retail::menu_keys::GALAXY), (page + 0x40, list), (page + 0x44, list)]);
	for (k, v) in [20, 86, 144, 251].into_iter().enumerate() {
		w.insert(list + 0x18 + 4 * k as u32, v);
	}
	w.extend([(list, 0x28_dc00), (list + 4, draw), (list + 0x30, 0x138), (list + 0x34, data), (list + 0x40, 3), (list + 0x44, scroll)]);
	if sig {
		for (k, v) in LIST_DRAW_SIG.into_iter().enumerate() {
			w.insert(draw + LIST_DRAW_SIG_AT + 4 * k as u32, v);
		}
	}
	for p in 0..n {
		w.insert(data + 12 * p, (0x4eb9 + p) | 1 << 16);
		w.insert(data + 12 * p + 8, 0x4ecc + p);
	}
	let rd = |a: u32| w.get(&a).copied().unwrap_or(0);
	read_ptr(&rd, pp).unwrap()
}

/// Retail's list rows are 33 px apart (font 10, gap 13, a second text) and the list scrolls: the old model (26 px, no scroll)
/// put the hover on the wrong planet.
#[test]
fn galactic_map_rows_are_33_px_apart_and_scroll() {
	let row = |i| Some(Target::Sub { item: 0x1b3f98, sub: i });
	let at = |m: &MenuSnap, local_y: f32| m.hit(80.0, 86.0 + local_y);
	let m = galaxy(0, true);
	assert!(matches!(&m.items[0].kind, Kind::Label { rows, rowh, cursor: 3 } if rows.len() == 18 && *rowh == 33.0));
	assert_eq!((at(&m, 8.0 + 33.0 * 5.0 + 12.0), at(&m, 8.0 + 33.0 * 6.0 + 12.0)), (row(5), row(6)));
	assert_eq!(at(&m, 245.0), row(7), "the last visible row; rows 8 and up lie below the item");
	// Scrolled by 104 px (the cursor on a low planet): row 10 is under local y 244, rows 0..2 are above the item.
	let m = galaxy(104, true);
	assert_eq!(at(&m, 244.0), row(10));
	assert_eq!(at(&m, 15.5), row(3));
	assert_eq!(at(&m, 1.0), row(3), "row 2 has scrolled above the item and is never hit");
	// Without the generic draw's code the old row model stays (this is the row it would have picked).
	assert_eq!(at(&galaxy(104, false), 244.0), row(9));
}

/// The mouse steers the Galactic Map's list by the retail rows, to the row under the pointer, and a key press undoes nothing.
#[test]
fn the_pointer_on_the_galactic_map_list_steers_to_the_retail_row() {
	let mut m = galaxy(104, true);
	let mut st = State::default();
	st.cursor(80.0, 86.0 + 244.0);
	let (mut prev, mut presses) = (0u16, 0u32);
	for _ in 0..200 {
		let b = st.step(Some(&m), 0);
		if b & !prev & DOWN != 0 {
			presses += 1;
			if let Kind::Label { cursor, .. } = &mut m.items[0].kind {
				*cursor += 1;
			}
		}
		prev = b;
	}
	assert_eq!((presses, list_cursor(&m)), (7, 10));
}

/// The menu closes with the pointer resting over the place of the next one's row: its anchor survives, so a 1.5 px jitter right
/// after the next menu opens does not aim (and steer) there; a real move (6 px) does.
#[test]
fn a_jitter_right_after_a_menu_opens_does_not_move_the_selection() {
	let (m, mut st) = (rows33(12, 0), State::default());
	let mut out = Vec::new();
	st.cursor(100.0, row_y(5));
	assert_eq!(st.step(None, 0), 0, "no menu");
	st.cursor(100.0, row_y(5) + 1.5);
	out.extend((0..30).map(|_| st.step(Some(&m), 0)));
	assert!(out.iter().all(|&b| b == 0) && st.goal.is_none(), "the jitter aimed: {out:?}");
	st.cursor(100.0, row_y(5) + 6.0);
	out.clear();
	out.extend((0..30).map(|_| st.step(Some(&m), 0)));
	assert!(out.iter().any(|&b| b & DOWN != 0), "a 6 px move aims at row 5: {out:?}");
}

/// A pad / keyboard player's desk bump with the cursor hidden is tracked, not aimed (`State::track`); a click that follows lands
/// where the pointer is.
#[test]
fn a_hidden_cursor_bump_does_not_aim_but_a_click_lands_there() {
	let (m, mut st) = (rows33(12, 0), State::default());
	st.track(100.0, row_y(5));
	let out: Vec<u16> = (0..30).map(|_| st.step(Some(&m), 0)).collect();
	assert!(out.iter().all(|&b| b == 0) && st.goal.is_none(), "the bump aimed: {out:?}");
	st.click();
	let out: Vec<u16> = (0..30).map(|_| st.step(Some(&m), 0)).collect();
	assert!(out.iter().any(|&b| b & DOWN != 0), "the click steers to row 5: {out:?}");
}

/// The other generic lists (Help Log, Skill Points, Cinematics ...): without a second text a record takes fh + 2 + 8 = 22 px
/// (font 12), with flag 0x10 the gap is fh + 3, without it h / (records + 1); a one-record list stays one button.
#[test]
fn list_rows_follow_the_flags_and_the_second_text() {
	let rows = |recs: &[(u32, u32, u32)], flags, h, scroll| match list_rows(recs, flags, h, scroll, 0) {
		Kind::Label { rows, rowh, .. } => (rows.iter().map(|r| r.0).collect::<Vec<f32>>(), rowh),
		k => panic!("{k:?}"),
	};
	let one = [(0x4fd5u32, 1u32, 0u32); 3];
	// Help Log (0x410): gap 15, first text y = 15 - 6 = 9, band from 5, pitch 22.
	assert_eq!(rows(&one, 0x410, 100, 0), (vec![16.0, 38.0, 60.0], 22.0));
	// Skill Points (0x200, no 0x10): gap = 240 / 4 = 60, first text y = 54; font 12, no text2.
	assert_eq!(rows(&one, 0x200, 240, 0), (vec![61.0, 83.0, 105.0], 22.0));
	// A second text adds the gap (60) to every record; scrolled by 20 the first band starts 20 px higher.
	let two = [(0x4fd5u32, 1u32, 0x5000u32); 3];
	assert_eq!(rows(&two, 0x200, 240, 20), (vec![71.0, 153.0, 235.0], 82.0));
	// Font 14 (flags 4): pitch 16 + 8; font 10 (flags 8, wins over 4): 12 + 8.
	assert_eq!(rows(&one, 0x410 | 4, 100, 0).1, 24.0);
	assert_eq!(rows(&one, 0x410 | 4 | 8, 100, 0).1, 20.0);
	// A list of one record is a single button: the old model (flags 0x138: font 10, row height 13) stays.
	let m = galaxy_n(1, 0, true);
	assert!(matches!(&m.items[0].kind, Kind::Label { rows, rowh, .. } if rows.len() == 1 && *rowh == 13.0), "{:?}", m.items[0].kind);
}

/// Instant selection (mouse_instant.rs): a hover or click on a text row hands the worker a request instead of pulsing the pad.
#[test]
fn instant_mode_asks_the_worker_instead_of_pulsing() {
	let m = root();
	let mut st = State::with_instant(true);
	// Hover the first button: a request, no pad bits, the goal is the worker's now.
	st.cursor(200.0, 61.0);
	assert_eq!(st.step(Some(&m), 0), 0);
	assert_eq!(st.take_cmd(), Some(Cmd::Submit(Request { page_id: 2, item: 0x10, sub: 0, confirm: false, slide: None })));
	assert!(st.goal.is_none());
	// A click on another button: a request with the click; the pointer moving to it first cancelled the hover's.
	st.cursor(200.0, 200.0);
	st.click();
	assert_eq!(st.step(Some(&m), 0), 0);
	assert_eq!(st.take_cmd(), Some(Cmd::Submit(Request { page_id: 2, item: 0x40, sub: 0, confirm: true, slide: None })));
	// A click on the row that is already selected (as the snapshot sees it) is a request with the click too: the worker checks the live
	// selection and presses the Cross itself, so a selection it moved since the snapshot is never confirmed by mistake (mmenu F2).
	st.cursor(200.0, 110.0);
	st.click();
	assert_eq!(st.step(Some(&m), 0), 0, "no Cross from a two-field-old snapshot");
	assert_eq!(st.take_cmd(), Some(Cmd::Submit(Request { page_id: 2, item: 0x20, sub: 0, confirm: true, slide: None })));
	// A key or button held takes the menu away: the request is cancelled. No menu: cancelled too.
	st.cursor(200.0, 200.0);
	assert_eq!(st.step(Some(&m), CROSS), 0);
	assert_eq!(st.take_cmd(), Some(Cmd::Cancel));
	assert_eq!(st.step(None, 0), 0);
	assert_eq!(st.take_cmd(), Some(Cmd::Cancel));
	// A button-hint line keeps its pad press; and a state without instant mode never sends a command.
	let mut off = State::default();
	off.cursor(200.0, 61.0);
	off.step(Some(&m), 0);
	assert_eq!(off.take_cmd(), None);
}

// ---- stage 4: wheel as one request, Sound Options bars, Yes / No answers ----

fn sound_page(cursor: u32) -> MenuSnap {
	let it = Item { addr: 0x300, slot: 0, rect: [2, 40, 512, 300], kind: Kind::Value { n: 3, cursor }, texts: Vec::new() };
	MenuSnap { page: 1, id: mouse_slider::PAGE, state: 2, cur: 0x300, front: false, items: vec![it] }
}

#[test]
fn a_wheel_over_a_text_list_is_one_request_for_all_notches() {
	// Five selectable rows, cursor on row 1: three notches down = row 4, ten = the last row, two up = row 0 (no wrap).
	let m = MenuSnap { items: vec![label(0x10, 44, 5, 1)], cur: 0x10, ..root() };
	let rq = |sub| Some(Request { page_id: 2, item: 0x10, sub, confirm: false, slide: None });
	assert_eq!(State::wheel_request(&m, -3), rq(4));
	assert_eq!(State::wheel_request(&m, -10), rq(4));
	assert_eq!(State::wheel_request(&m, 2), rq(0));
	// At the end the old pulse keeps retail's wrap: no request.
	let end = MenuSnap { items: vec![label(0x10, 44, 5, 4)], cur: 0x10, ..root() };
	assert_eq!(State::wheel_request(&end, -1), None);
	// A button (one row) is not a list.
	assert_eq!(State::wheel_request(&root(), -1), None);
}

#[test]
fn a_wheel_over_a_grid_moves_rows_in_the_same_column() {
	let grid = |cursor| MenuSnap { items: vec![Item { addr: 0x50, slot: 0, rect: [0, 0, 200, 150], kind: Kind::Grid { rows: 3, cols: 4, cursor }, texts: Vec::new() }], cur: 0x50, ..root() };
	let rq = |sub| Some(Request { page_id: 2, item: 0x50, sub, confirm: false, slide: None });
	assert_eq!(State::wheel_request(&grid(5), -1), rq(9));
	assert_eq!(State::wheel_request(&grid(5), 1), rq(1));
	assert_eq!(State::wheel_request(&grid(5), -5), rq(9));
	assert_eq!(State::wheel_request(&grid(9), -1), None);
}

#[test]
fn a_wheel_over_a_sound_bar_is_volume_steps() {
	let m = sound_page(1);
	let rq = State::wheel_request(&m, 2).unwrap();
	assert_eq!((rq.item, rq.sub, rq.slide), (0x300, 1, Some(mouse_slider::Slide::By(2 * mouse_slider::WHEEL))));
	let down = State::wheel_request(&m, -1).unwrap();
	assert_eq!(down.slide, Some(mouse_slider::Slide::By(-mouse_slider::WHEEL)));
	// The stereo row (2) is not a bar: the old pulse (Right / Left) stays.
	assert_eq!(State::wheel_request(&sound_page(2), 1), None);
}

#[test]
fn a_click_on_a_bar_sets_the_volume_and_a_held_button_drags() {
	let m = sound_page(0);
	let mut st = State::with_instant(true);
	// Row 1 (music) at y = 40 + 300 * 2/4 = 190; the bar runs 265..447, so x = 356 is half the volume.
	st.cursor(356.0, 190.0);
	st.click();
	assert_eq!(st.step(Some(&m), 0), 0);
	assert_eq!(st.take_cmd(), Some(Cmd::Submit(Request { page_id: 0x20, item: 0x300, sub: 1, confirm: false, slide: Some(mouse_slider::Slide::To(0x200)) })));
	// The button is still down: moving the pointer sets the volume under it, on the same row, even off the row's band.
	st.cursor(447.0, 260.0);
	st.step(Some(&m), 0);
	assert_eq!(st.take_cmd(), Some(Cmd::Submit(Request { page_id: 0x20, item: 0x300, sub: 1, confirm: false, slide: Some(mouse_slider::Slide::To(0x400)) })));
	// Released: the drag is over, a move aims like a hover again (the pointer is off every row: nothing).
	st.button_up();
	st.cursor(300.0, 190.0);
	st.step(Some(&m), 0);
	assert!(!matches!(st.take_cmd(), Some(Cmd::Submit(Request { slide: Some(_), .. }))));
	// A click left of the bar is a plain row click, no volume.
	let mut st = State::with_instant(true);
	st.cursor(100.0, 190.0);
	st.click();
	st.step(Some(&m), 0);
	assert!(matches!(st.take_cmd(), None | Some(Cmd::Submit(Request { slide: None, .. }))));
}

#[test]
fn a_stray_click_on_a_question_screen_does_nothing() {
	// Quit Game: item rect 100,100 300x200: 'No' band y 256..276 = Triangle, 'Yes' 276..296 = Circle; elsewhere nothing (before: confirm = Yes).
	let it = Item { addr: 0x200, slot: 0, rect: [100, 100, 300, 200], kind: Kind::Other, texts: Vec::new() };
	let m = MenuSnap { page: 1, id: crate::retail::mouse_answers::QUIT_GAME, state: 2, cur: 0x200, front: false, items: vec![it] };
	let mut st = State::with_instant(true);
	st.cursor(250.0, 150.0);
	st.click();
	assert_eq!(st.step(Some(&m), 0), 0, "a click beside the answers presses nothing");
	st.cursor(250.0, 262.0);
	st.click();
	assert_eq!(st.step(Some(&m), 0), TRIANGLE);
	// With instant off (RAC_MOUSE_INSTANT=0) the same safety holds (mmenu F6; before, a click beside the answers confirmed = Yes): a stray
	// click presses nothing, a click on an answer presses it, in both pulse rhythms.
	for mut off in [State::default(), State::with_instant(false)] {
		off.cursor(250.0, 150.0);
		off.click();
		assert_eq!(off.step(Some(&m), 0), 0, "instant off: a click beside the answers presses nothing");
		assert_eq!(off.take_cmd(), None);
		off.cursor(250.0, 262.0);
		off.click();
		assert_eq!(off.step(Some(&m), 0), TRIANGLE, "instant off: the answer under the pointer is pressed");
	}
}

/// The vendor shop as the mouse sees it (PAL, Novalis words as in mouse_vendor.rs): stage 0 = the browse strip, 2 = the quantity prompt.
fn shop_snap(sel: u32, stage: u32) -> MenuSnap {
	let rd = |a: u32| match a {
		0x1ca898 => sel,
		0x1ca89c => stage,
		0x1caa50 => 4,
		0x1ca840 => 1,
		_ => 0,
	};
	crate::retail::mouse_vendor::read(&rd, 1).unwrap()
}

#[test]
fn the_prompt_guard_holds_450_ms_and_only_while_a_prompt_is_up() {
	use std::time::{Duration, Instant};
	let t0 = Instant::now();
	let at = |ms: u64| t0.checked_add(Duration::from_millis(ms)).unwrap();
	assert_eq!(PROMPT_GUARD_MS, 450);
	assert!(!prompt_guard(None, at(0)), "no prompt: no guard");
	assert!(prompt_guard(Some(t0), t0));
	assert!(prompt_guard(Some(t0), at(449)));
	assert!(!prompt_guard(Some(t0), at(450)));
	assert!(!prompt_guard(Some(t0), at(60_000)));
	// A clock that reads earlier than the opening (never on a monotonic clock) still holds the click instead of panicking.
	assert!(prompt_guard(Some(at(100)), t0));
}

#[test]
fn the_state_notes_when_the_prompt_opened_and_that_it_is_gone() {
	let (browse, prompt) = (shop_snap(1, 0), shop_snap(1, 2));
	let mut st = State::with_instant(true);
	st.step(Some(&browse), 0);
	assert!(st.prompt_since.is_none(), "the browse strip is no prompt");
	st.step(Some(&prompt), 0);
	let since = st.prompt_since.expect("the prompt was seen");
	st.step(Some(&prompt), 0);
	assert_eq!(st.prompt_since, Some(since), "the opening time is kept while the prompt stays");
	st.step(Some(&browse), 0);
	assert!(st.prompt_since.is_none(), "the prompt closed");
	st.step(Some(&prompt), 0);
	assert!(st.prompt_since.is_some());
	st.step(None, 0);
	assert!(st.prompt_since.is_none(), "no menu at all drops it too");
}

#[test]
fn a_click_inside_the_guard_is_dropped_and_one_after_it_goes_through() {
	use std::time::{Duration, Instant};
	let prompt = shop_snap(1, 2);
	let icon = (144.0 + 56.0 + 10.0, 380.0);
	let mut st = State::with_instant(true);
	st.step(Some(&prompt), 0);
	st.cursor(icon.0, icon.1);
	st.click();
	assert_eq!(st.step(Some(&prompt), 0), 0);
	assert!(!matches!(st.take_cmd(), Some(Cmd::Submit(_))), "the prompt opened a moment ago: a double click's second half buys nothing");
	assert!(st.goal.is_none());
	// The same click 500 ms after the prompt opened is a real one: the worker is asked for the Cross.
	st.prompt_since = Some(Instant::now().checked_sub(Duration::from_millis(500)).expect("the clock has run for 500 ms"));
	st.click();
	assert_eq!(st.step(Some(&prompt), 0), 0);
	assert_eq!(st.take_cmd(), Some(Cmd::Submit(Request { page_id: crate::retail::mouse_vendor::ID, item: prompt.cur, sub: 0, confirm: true, slide: None })));
	// And 440 ms after is still inside the guard.
	st.prompt_since = Some(Instant::now().checked_sub(Duration::from_millis(440)).unwrap());
	st.click();
	assert_eq!(st.step(Some(&prompt), 0), 0);
	assert!(!matches!(st.take_cmd(), Some(Cmd::Submit(_))));
}

/// A click on an already selected row hands the worker the request with the click (never a Cross from the snapshot); other item kinds,
/// and the pulse rhythm without the instant path, keep their Cross.
#[test]
fn a_click_on_a_selected_row_is_a_request_for_every_kind_the_worker_takes() {
	let at = |y: i32, kind: Kind| Item { addr: 0x50, slot: 0, rect: [100, y, 200, 120], kind, texts: Vec::new() };
	let page = |it: Item| MenuSnap { items: vec![it], cur: 0x50, ..root() };
	let click = |m: &MenuSnap, instant: bool, x: f32, y: f32| {
		let mut st = State::with_instant(instant);
		st.cursor(x, y);
		st.click();
		let b = st.step(Some(m), 0);
		(b, st.take_cmd())
	};
	let rq = |sub| Some(Cmd::Submit(Request { page_id: 2, item: 0x50, sub, confirm: true, slide: None }));
	// A text list (row 1 of 3 selected, the pointer on it), a value page (row 0), a grid (cell 5 of 3 x 4).
	let list = page(Item { rect: [100, 100, 200, 120], ..label(0x50, 100, 3, 1) });
	let Kind::Label { rows, .. } = &list.items[0].kind else { unreachable!() };
	let y1 = 100.0 + rows[1].0;
	assert_eq!(click(&list, true, 150.0, y1), (0, rq(1)));
	let value = page(at(100, Kind::Value { n: 3, cursor: 0 }));
	assert_eq!(click(&value, true, 150.0, 100.0 + 120.0 / 4.0 * 0.5), (0, rq(0)));
	let grid = page(at(100, Kind::Grid { rows: 3, cols: 4, cursor: 5 }));
	assert_eq!(click(&grid, true, 100.0 + 200.0 / 4.0 * 1.5, 100.0 + 120.0 / 3.0 * 1.5), (0, rq(5)));
	// Without the instant path: Cross from the pulse, no command.
	assert_eq!(click(&list, false, 150.0, y1), (CROSS, None));
	assert_eq!(click(&grid, false, 100.0 + 200.0 / 4.0 * 1.5, 100.0 + 120.0 / 3.0 * 1.5), (CROSS, None));
	// A panel is not taken by the worker (no cursor of its own): its Cross stays a pulse.
	let panel = page(at(100, Kind::Panel));
	let (b, cmd) = click(&panel, true, 150.0, 150.0);
	assert_eq!(b, CROSS);
	assert!(!matches!(cmd, Some(Cmd::Submit(_))), "{cmd:?}");
	// A hover (no click) on the selected row asks for nothing.
	let mut st = State::with_instant(true);
	st.cursor(150.0, y1);
	assert_eq!(st.step(Some(&list), 0), 0);
	assert!(!matches!(st.take_cmd(), Some(Cmd::Submit(_))));
}

/// The drag of a Sound Options bar ends when the left button goes up, and the viewer lets it go when the window loses the focus (a
/// release that happens over another window never arrives here): no more volume requests follow the pointer then (mmenu F7).
#[test]
fn losing_the_focus_ends_a_bar_drag() {
	let m = sound_page(0);
	let mut st = State::with_instant(true);
	st.cursor(356.0, 190.0);
	st.click();
	st.step(Some(&m), 0);
	assert!(matches!(st.take_cmd(), Some(Cmd::Submit(Request { slide: Some(_), .. }))));
	assert!(st.drag.is_some(), "the click on the bar started a drag");
	// Still held: a move sets the volume.
	st.cursor(400.0, 190.0);
	st.step(Some(&m), 0);
	assert!(matches!(st.take_cmd(), Some(Cmd::Submit(Request { slide: Some(_), .. }))));
	// The focus is lost: Retail::mouse_release = State::button_up. The pointer comes back over the bar and moves: no volume follows.
	st.button_up();
	for x in [300.0, 320.0, 440.0] {
		st.cursor(x, 190.0);
		st.step(Some(&m), 0);
		assert!(!matches!(st.take_cmd(), Some(Cmd::Submit(Request { slide: Some(_), .. }))), "x = {x}");
	}
	assert!(st.drag.is_none());
	// The bar click is an instant-path feature only: with RAC_MOUSE_INSTANT=0 it is a plain row click (no volume request, no drag).
	let mut off = State::with_instant(false);
	off.cursor(356.0, 190.0);
	off.click();
	off.step(Some(&m), 0);
	assert!(off.drag.is_none());
	assert!(!matches!(off.take_cmd(), Some(Cmd::Submit(_))));
	// The viewer's Focused(false) arm calls `App::lose_focus`, which calls Retail::mouse_release (the winit event loop cannot run in a
	// unit test: checked in the source).
	let src = include_str!("../../viewer/handler.rs");
	let arm = src.split("WindowEvent::Focused(f)").nth(1).expect("the Focused arm");
	let arm = arm.split("WindowEvent::CursorMoved").next().unwrap();
	assert!(arm.contains("!f") && arm.contains("self.lose_focus()"), "losing the focus (only then) lets go of the buttons");
	let play = include_str!("../../viewer/handler/play_input.rs");
	let body = play.split("fn lose_focus").nth(1).expect("lose_focus");
	assert!(body.contains("r.mouse_release()") && body.contains("self.fire_mouse = false"), "and releases the menu's mouse button too");
}
