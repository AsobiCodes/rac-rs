//! Mouse in the retail menus (the page-menu engine FUN_00218da0: pause menu and its pages, Quick Select ring, galactic
//! map screen). The mouse presses pad buttons, exactly like a hand on the pad; for the rows of text lists and value pages it
//! asks the worker for an instant selection instead (`mouse_instant.rs`: the same words a D-pad move writes, at once).
//!
//! * `read` (worker thread, every field in mode 3) copies the menu's state out of RAM: the current page, its items with
//!   their screen rectangles (512x448 PS2 pixels, the space the overlay is drawn in) and each item's own cursor.
//! * `State` (main thread) turns the mouse into pad presses: hovering steers the game's own selection by D-pad / L1 / R1
//!   pulses until the selection read from RAM is the hovered one, a left click adds Cross (on no item: the screen's confirm
//!   button, the one Enter presses, for the highlighted item), a right click is the screen's
//!   back button, the wheel steps up / down (left / right over values, L1 / R1 over the ring), and the button-hint lines
//!   press their button. See <dev>/scratch_play/handoff/mouse_lean.md for the RAM layout [C].

use crate::fmath::Fm;
use crate::retail::disc::va;
use super::mouse_instant::{Cmd, Request};
use super::mouse_slider::{self, Slide};
pub const UP: u16 = 0x1000;
pub const RIGHT: u16 = 0x2000;
pub const DOWN: u16 = 0x4000;
pub const LEFT: u16 = 0x8000;
pub const CROSS: u16 = 0x40;
pub const CIRCLE: u16 = 0x20;
pub const TRIANGLE: u16 = 0x10;
pub const L1: u16 = 4;
pub const R1: u16 = 8;
/// Every button the menus read (a keyboard press of one hands the menu back to the keyboard).
pub(super) const MENU_BTNS: u16 = UP | RIGHT | DOWN | LEFT | CROSS | CIRCLE | TRIANGLE | 0x80 | L1 | R1 | 0x800;

/// The overlay's pixel space.
pub(super) const SCREEN: (f32, f32) = (512.0, 448.0);
/// Fields a pulse holds the button, and fields it then stays released (the menu's auto-repeat needs a fresh press; the pad's
/// "new press" word is held & !previous, so one field down and one up is an edge: boot FUN_00216e20 +0x1a4). A steering pulse
/// ends after its first released field once the snap shows the selection moved (`State::step`): 2 fields a step when the game
/// takes the press at once, 3 when it does not (v0.4.1: 2 + 3, so 3 and 5).
const PRESS: u8 = 1;
const RELEASE: u8 = 2;
/// The v0.4.1 rhythm, kept for RAC_MOUSE_INSTANT=0 (`State::with_instant(false)`): the old behaviour as it was, also to measure against.
const SLOW_PRESS: u8 = 2;
const SLOW_RELEASE: u8 = 3;
/// Presses spent steering to one goal before giving up; also the budget of one hovered target over all its goals (`State::spent`).
const MAX_PRESSES: u32 = 24;
const MASK: u32 = 0x01ff_ffff;
/// Overlay pixels the pointer must travel from where it last aimed before a hover aims again: a resting hand's jitter (or a
/// list scrolling under a still pointer) is no new aim. After a keyboard / pad menu input the hover waits for `KEY_MOVE_PX`.
const JITTER_PX: f32 = 4.0;
const KEY_MOVE_PX: f32 = 8.0;
/// Fields a click / right click / wheel notch waits while the keyboard holds a menu button, then it is dropped.
const PEND_FIELDS: u8 = 25;
/// The generic list draw (Eudora FUN_0026dd30, Novalis FUN_002907c8, Veldin FUN_0027aa80: the same code on all 19 levels,
/// relocated): its scroll step `lw v0,0x44(s1); b +3; addu v0,v0,v1; lw v0,0x44(s1); addiu v0,v0,4; sw v0,0x44(s1)` at
/// entry + 0x418 has no relocated word, so the draw is recognised by its code (`list_draw`).
const LIST_DRAW_SIG: [u32; 6] = [0x8e22_0044, 0x1000_0003, 0x0043_1021, 0x8e22_0044, 0x2442_0004, 0xae22_0044];
const LIST_DRAW_SIG_AT: u32 = 0x418;

/// Page-menu mode word 0x15f5a4.
pub const MODE: u32 = 3;
/// The word that points at the current page, per level (index = the level number at 0x15ed84); the page's kind word
/// (2 idle, 1 flying in, 0x14 closing) sits 4 bytes before it.
const PAGE_PTR: [u32; 19] = [
	0x1b9cf4, 0x1ba0f4, 0x1ba274, 0x1b9d74, 0x1b9df4, 0x1ba174, 0x1ba3f4, 0x1b9cf4, 0x1ba474, 0x1b9df4, 0x1ba0f4, 0x1ba674, 0x1ba0f4, 0x1b9f74,
	0x1ba3f4, 0x1ba274, 0x1ba0f4, 0x1ba574, 0x1ba674,
];
/// `PAGE_PTR` on the USA disc (tools/racrs_va.py: every address mapped in its level; generated, do not edit).
const PAGE_PTR_USA: [u32; 19] = [
	0x1b9cf4, 0x1ba174, 0x1ba2f4, 0x1b9df4, 0x1b9e74, 0x1ba174, 0x1ba474, 0x1b9d74, 0x1ba4f4, 0x1b9e74, 0x1ba174, 0x1ba6f4, 0x1ba174, 0x1b9ff4,
	0x1ba3f4, 0x1ba2f4, 0x1ba174, 0x1ba5f4, 0x1ba6f4,
];

/// What one item is, as far as the mouse cares.
#[derive(Clone, Debug, PartialEq)]
pub enum Kind {
	/// A button or list of text rows (records at +0x34); `rows` holds each record's centre (pixels below the item's top)
	/// and whether it is selectable; `cursor` is the row cursor at +0x40.
	Label { rows: Vec<(f32, bool)>, rowh: f32, cursor: u32 },
	/// A value page's rows (HelpDesk, Subtitles, Camera, Sound): `n` rows at (i+1)*h/(n+1).
	Value { n: u32, cursor: u32 },
	/// The weapon grid: cell index = row * cols + col.
	Grid { rows: u32, cols: u32, cursor: u32 },
	/// The Quick Select ring (8 slots).
	Ring { cursor: u32 },
	/// A selectable panel without a cursor of its own (the Gadgets page's four groups): selected when it is the current item.
	Panel,
	/// A button-hint line: a click presses this pad button.
	Hint(u16),
	Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
	pub addr: u32,
	pub slot: usize,
	/// x, y, w, h in overlay pixels.
	pub rect: [i32; 4],
	pub kind: Kind,
	/// The text id of each record of the item's label list (+0x34), empty when the item has none (title_boot.rs steers by them).
	pub texts: Vec<u16>,
}

/// The menu on screen this field.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MenuSnap {
	/// Page struct address, page id (root 2, Weapons 3, Gadgets 4, Quick Select 5, Items 6, Help 7, Options 8, map 0xb ...).
	pub page: u32,
	pub id: u32,
	/// The page's kind word: 2 on the root, the page id on the sub pages when idle; 1 = flying in, 0x14 = closing (`busy`).
	pub state: u32,
	/// The page's current item (address).
	pub cur: u32,
	pub items: Vec<Item>,
	/// The title front end's menu (its page ids are its own; `menu_keys::screen`).
	pub front: bool,
}

fn ram(a: u32) -> bool {
	// Also the PC options region (pcext.rs: the Graphics page, its items and the Options list copy live there).
	((0x10_0000..0x200_0000).contains(&(a & MASK)) || (super::pcext::BASE..super::pcext::BASE + 0x1_0000).contains(&a)) && a & !MASK == 0
}

/// Text ids of the hint lines and the pad button each one names (decomp/notes/ingame_pause_menu.md, hud.md).
pub(super) fn hint_button(text: u16) -> Option<u16> {
	match text {
		0x4ee0 | 0x524b => Some(TRIANGLE),
		0x4edf | 0x4eec | 0x5248 | 0x524a | 0x4eee => Some(CROSS),
		0x524f | 0x5249 | 0x4eef => Some(CIRCLE),
		_ => None,
	}
}

/// Reads the menu out of RAM (`rd` = 32-bit word read). None outside the page-menu mode or when the pointers look wrong.
pub fn read(rd: &dyn Fn(u32) -> u32, level: u32, mode: u32) -> Option<MenuSnap> {
	if mode != MODE {
		return None;
	}
	read_ptr(rd, *(*crate::retail::disc::pick(&PAGE_PTR, &PAGE_PTR_USA)).get(level as usize)?)
}

/// The front end's (frontbin) page pointer: FUN_00218758 keeps the current page in 0x1d5b74 and its kind word in 0x1d5b70.
pub const FRONT_PAGE_PTR: u32 = 0x1d5b74;

/// The front end's menu (its mode word is 0x15f5e4, `title_boot::FRONT_MODE`): the same page-menu engine as the pause menu.
pub fn read_front(rd: &dyn Fn(u32) -> u32, mode: u32) -> Option<MenuSnap> {
	if mode != MODE {
		return None;
	}
	read_ptr(rd, va(FRONT_PAGE_PTR)).map(|m| MenuSnap { front: true, ..m })
}

/// The menu whose current-page pointer lives at `pp`.
pub fn read_ptr(rd: &dyn Fn(u32) -> u32, pp: u32) -> Option<MenuSnap> {
	let page = rd(pp);
	if !ram(page) {
		return None;
	}
	let rd16 = |a: u32| (rd(a & !3) >> (8 * (a & 2))) & 0xffff;
	let id = rd(page + 0x3c);
	let mut snap = MenuSnap { page, id, state: rd(pp - 4), cur: rd(page + 0x40) & MASK, items: Vec::new(), front: false };
	for slot in 0..14usize {
		let it = rd(page + 0x44 + 4 * slot as u32);
		if !ram(it) || snap.items.iter().any(|i| i.addr == it) {
			continue;
		}
		let rect = [rd(it + 0x18) as i32, rd(it + 0x1c) as i32, rd(it + 0x20) as i32, rd(it + 0x24) as i32];
		let [x, y, w, h] = rect;
		// Parked items (x 585 / -179) and empty rectangles are not on screen.
		if w <= 0 || h <= 0 || w > 2048 || h > 2048 || x >= SCREEN.0 as i32 || x + w <= 0 || y >= SCREEN.1 as i32 || y + h <= 0 {
			continue;
		}
		let kind = classify(rd, &rd16, id, slot, it, h);
		let texts = item_texts(rd, &rd16, it);
		snap.items.push(Item { addr: it, slot, rect, kind, texts });
	}
	Some(snap)
}

/// The text ids of an item's 12-byte label records (+0x34), up to a zero id; empty when they do not look like text ids.
fn item_texts(rd: &dyn Fn(u32) -> u32, rd16: &dyn Fn(u32) -> u32, it: u32) -> Vec<u16> {
	let data = rd(it + 0x34);
	if !ram(data) {
		return Vec::new();
	}
	let v: Vec<u16> = (0..14).map(|k| rd16(data + 12 * k) as u16).take_while(|&t| t != 0).collect();
	if v.first().is_some_and(|t| (0x4e00..=0x5700).contains(t)) { v } else { Vec::new() }
}

fn classify(rd: &dyn Fn(u32) -> u32, rd16: &dyn Fn(u32) -> u32, id: u32, slot: usize, it: u32, h: i32) -> Kind {
	if id == 5 && slot == 1 {
		return Kind::Ring { cursor: rd(it + 0x50) & 7 };
	}
	// The title's Graphics page (pcext.rs `title_options`): the front end's value item, 5-word rows, row cursor at +0x38.
	if it == super::pcext::TGVAL || it == super::pcmods::TVAL {
		let n = (0..16).take_while(|k| rd(rd(it + 0x34) + 20 * k) != 0).count() as u32;
		return Kind::Value { n: n.max(1), cursor: rd(it + 0x38) };
	}
	if matches!(id, 0x13 | 0x1b | 0x1d | 0x20) && slot == 3 {
		let n = match id {
			0x13 => 2,
			0x1b => 1,
			0x20 => 3,
			// Camera Options (0x1d): the rows of the item's table (3 as retail, 8 with the PC options' rows: Mouse speed, Camera, Strafe, Buy all, Lock-on, retail/pcext.rs;
			// the Graphics page's copy has 13).
			_ => match (0..16).take_while(|k| rd(rd(it + 0x34) + 24 * k) != 0).count() as u32 {
				0 => 3,
				n => n,
			},
		};
		return Kind::Value { n, cursor: rd(it + if id == 0x20 { 0x40 } else { 0x38 }) };
	}
	// The icon grids (update FUN_00278e30: cursor +0x3c, rows +0x40, columns +0x44, cells +0x48): Weapons (3), Quick Select (5), the four
	// Gadgets groups (4, slots 2 / 3 / 9 / 10) and the Items page (6).
	if matches!(id, 3 | 5 | 6) || (id == 4 && matches!(slot, 2 | 3 | 9 | 10)) {
		let (rows, cols, cells) = (rd(it + 0x40), rd(it + 0x44), rd(it + 0x48));
		if (1..=8).contains(&rows) && (1..=8).contains(&cols) && ram(cells) {
			return Kind::Grid { rows, cols, cursor: rd(it + 0x3c) };
		}
	}
	if id == 4 && matches!(slot, 2 | 3 | 9 | 10) {
		return Kind::Panel;
	}
	let data = rd(it + 0x34);
	if !ram(data) {
		return Kind::Other;
	}
	let flags = rd(it + 0x30);
	// A text item showing the current item's row (draw FUN_0028d648 on Novalis, flags 0x80: +0x34 = u32 text ids, +0x38 =
	// their stride 4; a list's +0x38 is its Up neighbour item instead): the description panel beside the Help / Options
	// lists. Never the selection: the page's current item only changes through page+0x80, which those lists (flags 0x1000,
	// list update FUN_0028df18) never write, and Left / Right are not read there.
	if flags & 0x80 != 0 && rd(it + 0x38) == 4 {
		return Kind::Other;
	}
	// 12-byte records {text id, type, arg, text2, pulse} up to a zero text id.
	let list = list_draw(rd, it);
	let mut recs = Vec::new();
	for k in 0..if list { 256 } else { 14 } {
		let a = data + 12 * k;
		let text = rd16(a);
		if text == 0 {
			break;
		}
		recs.push((text, rd16(a + 2), rd16(a + 8)));
	}
	let Some(&(first, _, _)) = recs.first() else { return Kind::Other };
	if !(0x4e00..=0x5700).contains(&first) {
		return Kind::Other;
	}
	if rd(it) == 0 && recs.len() == 1 {
		return hint_button(first as u16).map_or(Kind::Other, Kind::Hint);
	}
	if list && recs.len() > 1 {
		return list_rows(&recs, flags, h, rd(it + 0x44) as i32, rd(it + 0x40));
	}
	let n = recs.len() as f32;
	let fh = if flags & 4 != 0 {
		14.0
	} else if flags & 8 != 0 {
		10.0
	} else {
		12.0
	};
	let rowh = if flags & 0x10 != 0 { fh + 3.0 } else { h as f32 / (n + 1.0) };
	let mut prefix = 0.0;
	let rows = recs
		.iter()
		.map(|&(_, ty, t2)| {
			let cy = (prefix + 1.0) * rowh - 1.0;
			prefix += if t2 != 0 { 2.0 } else { 1.0 };
			(cy, ty != 0)
		})
		.collect();
	Kind::Label { rows, rowh, cursor: rd(it + 0x40) }
}

/// Is the item drawn by the generic list draw (`LIST_DRAW_SIG`)? Its rows follow `list_rows`.
fn list_draw(rd: &dyn Fn(u32) -> u32, it: u32) -> bool {
	let draw = rd(it + 4);
	ram(draw) && draw & 3 == 0 && LIST_DRAW_SIG.iter().zip(0u32..).all(|(&w, k)| rd(draw + LIST_DRAW_SIG_AT + 4 * k) == w)
}

/// The rows as the generic list draw lays them out [C: Eudora FUN_0026dd30, asm 0x26dd30..0x26e1a0]: font height fh = 12, 14
/// with flags 4, 10 with flags 8 (8 wins); line height fh + 2; gap = fh + 3 with flags 0x10, else h / (records + 1); the first
/// row's text starts at gap - fh / 2 - scroll (+0x44, moved 4 px per field until the current row is in view), each record takes
/// its text's lines (fh + 2 each), then gap more when it has a second text (+8), then 8. One line per text is assumed. [C,
/// measured 2026-10-28] Texts wider than the list wrap (German Help Log from row 1, Skill Points from row 3): from the first
/// wrapped row on, the model is fh + 2 px per extra line too high; exact for single-line lists (Help, Goodies, Cinematics, Galactic
/// Map). Known limit (porting the wrap count of FUN_001ee4e0 would fix it). A row's band runs from 4 px above
/// its text to 4 px above the next row's, so rows scrolled out of the item fall outside its rect and are never hit.
fn list_rows(recs: &[(u32, u32, u32)], flags: u32, h: i32, scroll: i32, cursor: u32) -> Kind {
	let fh: i32 = if flags & 8 != 0 {
		10
	} else if flags & 4 != 0 {
		14
	} else {
		12
	};
	let gap = if flags & 0x10 != 0 { fh + 3 } else { h / (recs.len() as i32 + 1) };
	let mut y = gap - fh / 2 - scroll;
	let mut least = i32::MAX;
	let rows = recs
		.iter()
		.map(|&(_, ty, t2)| {
			let pitch = fh + 2 + if t2 != 0 { gap } else { 0 } + 8;
			let row = ((y - 4) as f32 + pitch as f32 * 0.5, ty != 0);
			least = least.min(pitch);
			y += pitch;
			row
		})
		.collect();
	Kind::Label { rows, rowh: least as f32, cursor }
}

/// Window position (fractions 0..1 of the client area) -> overlay pixels. The overlay quad covers the whole window with
/// the 4:3 picture centred (handler.rs: uv x in [0.5 - 0.5k, 0.5 + 0.5k], k = aspect / (4/3)); with GS-full (`full` = the
/// picture's aspect) the picture is letterboxed at the largest centred size instead.
pub fn to_ps2(fx: f32, fy: f32, aspect: f32, full: Option<f32>) -> (f32, f32) {
	match full {
		None => {
			let k = aspect / (4.0 / 3.0);
			((0.5 + (fx - 0.5) * k) * SCREEN.0, fy * SCREEN.1)
		}
		Some(ar) => {
			let (wf, hf) = if aspect >= ar { (ar / aspect, 1.0) } else { (1.0, aspect / ar) };
			(((fx - 0.5) / wf + 0.5) * SCREEN.0, ((fy - 0.5) / hf + 0.5) * SCREEN.1)
		}
	}
}

/// What a point on the menu means.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Target {
	/// A row / cell / slot of an item (Label, Value, Grid, Ring).
	Sub { item: u32, sub: u32 },
	/// A button-hint line.
	Hint(u16),
}

fn contains(r: [i32; 4], x: f32, y: f32, pad: f32) -> bool {
	x >= r[0] as f32 - pad && x < (r[0] + r[2]) as f32 + pad && y >= r[1] as f32 - pad && y < (r[1] + r[3]) as f32 + pad
}

/// The ring's slot centres (slot i at angle i*pi/4 - pi/2, radius min(w,h)/2 - 40 around the item's centre).
pub fn ring_slot(r: [i32; 4], i: u32) -> (f32, f32) {
	let (cx, cy) = (r[0] as f32 + r[2] as f32 * 0.5, r[1] as f32 + r[3] as f32 * 0.5);
	let rad = (r[2].min(r[3]) as f32) * 0.5 - 40.0;
	let a = i as f32 * std::f32::consts::FRAC_PI_4 - std::f32::consts::FRAC_PI_2;
	(cx + a.fcos() * rad, cy + a.fsin() * rad)
}

impl MenuSnap {
	/// The page is flying in or closing.
	pub fn busy(&self) -> bool {
		matches!(self.state, 1 | 0x14)
	}

	/// The item or row under (x, y), smallest item first (an item on top of a bigger one wins).
	pub fn hit(&self, x: f32, y: f32) -> Option<Target> {
		let mut best: Option<(i64, Target)> = None;
		for it in &self.items {
			let [rx, ry, rw, rh] = it.rect;
			let t = match &it.kind {
				Kind::Label { rows, rowh, .. } if contains(it.rect, x, y, 0.0) => {
					if rows.len() == 1 {
						rows[0].1.then_some(Target::Sub { item: it.addr, sub: 0 })
					} else {
						rows.iter().position(|&(cy, sel)| sel && (y - (ry as f32 + cy)).abs() <= rowh * 0.5 + 0.5).map(|i| Target::Sub { item: it.addr, sub: i as u32 })
					}
				}
				Kind::Value { n, .. } if contains(it.rect, x, y, 0.0) => {
					let i = ((y - ry as f32) / (rh as f32 / (*n as f32 + 1.0)) - 0.5).floor().clamp(0.0, *n as f32 - 1.0);
					Some(Target::Sub { item: it.addr, sub: i as u32 })
				}
				Kind::Grid { rows, cols, .. } if contains(it.rect, x, y, 0.0) => {
					let c = (((x - rx as f32) / (rw as f32 / *cols as f32)).floor()).clamp(0.0, *cols as f32 - 1.0) as u32;
					let r = (((y - ry as f32) / (rh as f32 / *rows as f32)).floor()).clamp(0.0, *rows as f32 - 1.0) as u32;
					Some(Target::Sub { item: it.addr, sub: r * cols + c })
				}
				Kind::Ring { .. } => (0..8u32)
					.map(|i| (i, ring_slot(it.rect, i)))
					.map(|(i, (sx, sy))| (i, (sx - x).fhypot(sy - y)))
					.filter(|&(_, d)| d <= 28.0)
					.min_by(|a, b| a.1.total_cmp(&b.1))
					.map(|(i, _)| Target::Sub { item: it.addr, sub: i }),
				Kind::Panel if contains(it.rect, x, y, 0.0) => Some(Target::Sub { item: it.addr, sub: 0 }),
				Kind::Hint(b) if contains(it.rect, x, y, 3.0) => Some(Target::Hint(*b)),
				_ => None,
			};
			if let Some(t) = t {
				let area = rw as i64 * rh as i64;
				if best.map_or(true, |(a, _)| area <= a) {
					best = Some((area, t));
				}
			}
		}
		best.map(|b| b.1)
	}

	fn item(&self, addr: u32) -> Option<&Item> {
		self.items.iter().find(|i| i.addr == addr)
	}

	/// The item's own cursor (row / cell / slot).
	fn cursor_of(it: &Item) -> Option<u32> {
		match it.kind {
			Kind::Label { cursor, .. } | Kind::Value { cursor, .. } | Kind::Grid { cursor, .. } | Kind::Ring { cursor } => Some(cursor),
			Kind::Panel => Some(0),
			_ => None,
		}
	}

	/// Is the game's selection already on this target? (The ring cursor moves with L1 / R1 whatever the current item is.)
	pub fn selected(&self, t: Target) -> bool {
		let Target::Sub { item, sub } = t else { return false };
		let Some(it) = self.item(item) else { return false };
		match it.kind {
			Kind::Ring { cursor } => cursor == sub,
			_ => self.cur == item && Self::cursor_of(it) == Some(sub),
		}
	}

	/// Does a press toward `t` move the cursor inside the target's own item (it holds the selection, or it is the ring)? Only then
	/// is the opposite press next an overshoot: stepping from item to item lands the new item's cursor anywhere.
	fn inside(&self, t: Target) -> bool {
		let Target::Sub { item, .. } = t else { return false };
		self.cur == item || self.item(item).is_some_and(|i| matches!(i.kind, Kind::Ring { .. }))
	}

	/// The pad button that moves the game's selection one step toward `t`.
	pub fn toward(&self, t: Target) -> Option<u16> {
		let Target::Sub { item, sub } = t else { return None };
		let it = self.item(item)?;
		if let Kind::Ring { cursor } = it.kind {
			return Some(if (sub + 8 - cursor) % 8 <= 4 { R1 } else { L1 });
		}
		if self.cur != item {
			// Another item holds the selection: step toward the goal's centre (dominant axis).
			let centre = |r: [i32; 4]| (r[0] as f32 + r[2] as f32 * 0.5, r[1] as f32 + r[3] as f32 * 0.5);
			let Some(c) = self.item(self.cur) else { return Some(DOWN) };
			let (a, b) = (centre(c.rect), centre(it.rect));
			let (dx, dy) = (b.0 - a.0, b.1 - a.1);
			if dy.abs() >= dx.abs() {
				return Some(if dy > 0.0 { DOWN } else { UP });
			}
			// Left / Right would change a value on a Label or Value item: only grids and panels get sideways steps.
			return matches!(c.kind, Kind::Grid { .. } | Kind::Panel).then_some(if dx > 0.0 { RIGHT } else { LEFT });
		}
		match it.kind {
			Kind::Label { cursor, .. } | Kind::Value { cursor, .. } => Some(if cursor < sub { DOWN } else { UP }),
			Kind::Grid { cols, cursor, .. } => {
				let (r, c) = (cursor / cols, cursor % cols);
				let (tr, tc) = (sub / cols, sub % cols);
				Some(if r != tr {
					if r < tr { DOWN } else { UP }
				} else if c < tc {
					RIGHT
				} else {
					LEFT
				})
			}
			_ => None,
		}
	}

	/// The selection as the steering sees it: the current item, its cursor and the ring's cursor (any change = a press landed).
	fn sig(&self) -> (u32, u32) {
		let ring = self.items.iter().find_map(|i| if let Kind::Ring { cursor } = i.kind { Some(cursor) } else { None }).unwrap_or(0);
		(self.cur, self.item(self.cur).and_then(MenuSnap::cursor_of).unwrap_or(0) * 16 + ring)
	}

	/// The right-click button: the screen's back / No button of the per-screen table (`menu_keys::screen`), the same one
	/// Esc presses.
	fn back_button(&self) -> u16 {
		super::menu_keys::screen(self).back
	}
}

/// A selection the mouse is steering toward.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Goal {
	target: Target,
	page: u32,
	/// Press Cross when the selection is there (a click).
	confirm: bool,
	presses: u32,
	/// The selection (current item, its cursor) when the last press went out, and how many presses in a row left it there.
	sig: (u32, u32),
	stuck: u8,
	/// The last steering button: the opposite one next means the selection passed the goal or something else moves it.
	last: u16,
}

impl Goal {
	fn new(target: Target, page: u32, confirm: bool) -> Goal {
		Goal { target, page, confirm, presses: 0, sig: (0, 0), stuck: 0, last: 0 }
	}
}

/// Milliseconds after the shop's quantity prompt opens during which a click is ignored. A double click on a shop icon is the first
/// click (move + Cross: the prompt opens a few fields later) and a second one 100 to 450 ms after it; the prompt's only target is the
/// same icon at the same pixels, so without this the second click would buy.
pub const PROMPT_GUARD_MS: u64 = 450;

/// True while a click must be ignored because the quantity prompt (up since `since`) opened less than `PROMPT_GUARD_MS` before `now`.
pub fn prompt_guard(since: Option<std::time::Instant>, now: std::time::Instant) -> bool {
	since.is_some_and(|t| now.saturating_duration_since(t) < std::time::Duration::from_millis(PROMPT_GUARD_MS))
}

/// The button that undoes `b` (0 for buttons without one).
fn opposite(b: u16) -> u16 {
	match b {
		UP => DOWN,
		DOWN => UP,
		LEFT => RIGHT,
		RIGHT => LEFT,
		L1 => R1,
		R1 => L1,
		_ => 0,
	}
}

/// The left stick as menu directions (a gamepad's stick, or WASD on the screens whose keys drive the stick: `menu_keys`
/// `sticks`, the Galactic Map). For `State::step`'s "the player works the menu" test only; the game gets the stick as it was.
pub fn stick_bits(s: [u8; 4]) -> u16 {
	// Retail's pad writer (boot FUN_00216e20) turns a stick into D-pad bits at |byte - 0x7f| > 0x2f; the same threshold here, so the
	// game never steps a menu on a push this test calls idle.
	let pick = |v: u8, lo: u16, hi: u16| match v as i32 - 0x7f {
		d if d < -0x2f => lo,
		d if d > 0x2f => hi,
		_ => 0,
	};
	pick(s[3], UP, DOWN) | pick(s[2], LEFT, RIGHT)
}

/// The mouse as the menu sees it, plus the pulse sequencer.
#[derive(Debug, Default)]
pub struct State {
	pos: Option<(f32, f32)>,
	moved: bool,
	click: bool,
	back: bool,
	wheel: i32,
	goal: Option<Goal>,
	pulse: u16,
	left: u8,
	/// A steering pulse in flight: the page and the selection when it went out.
	steer: Option<(u32, (u32, u32))>,
	/// Where the pointer was when the hover last aimed (`JITTER_PX`), and whether the keyboard / pad took the menu since
	/// (`KEY_MOVE_PX`).
	anchor: Option<(f32, f32)>,
	quiet: bool,
	/// The target under the pointer when it last aimed, presses spent on it over all its goals, and whether steering gave it up
	/// (unreachable, overshot or fought): no new goal for it until the pointer aims at something else.
	hover: Option<Target>,
	spent: u32,
	gave_up: bool,
	/// Fields a click / right click / wheel notch has waited for the keyboard's menu buttons to go up.
	pend: u8,
	/// The page the hover bookkeeping belongs to (None: no page seen yet).
	page: Option<u32>,
	/// Instant selection on (`mouse_instant::enabled`, set by `with_instant`): rows of text lists and value pages are asked of the
	/// worker; `cmd` is what the viewer hands it after this field's step (`take_cmd`).
	instant: bool,
	cmd: Option<Cmd>,
	/// RAC_MOUSE_INSTANT=0: the v0.4.1 pulse rhythm (2 fields down, 3 up).
	slow: bool,
	/// The left button is held, and the Sound Options bar (item, row) a drag that started on it sets (`mouse_slider.rs`).
	down: bool,
	drag: Option<(u32, u32)>,
	/// Since when the shop's quantity prompt is up (wall time, so PAL and NTSC agree): clicks right after it opened are a double
	/// click's second half, not a buy (`PROMPT_GUARD`).
	prompt_since: Option<std::time::Instant>,
}

impl State {
	/// A state that asks for instant selections when `on`.
	pub fn with_instant(on: bool) -> State {
		State { instant: on, slow: !on, ..State::default() }
	}

	/// Fields a pulse stays released.
	fn release(&self) -> u8 {
		if self.slow { SLOW_RELEASE } else { RELEASE }
	}

	/// The command for the worker this field's step left (None: nothing new).
	pub fn take_cmd(&mut self) -> Option<Cmd> {
		self.cmd.take()
	}

	fn command(&mut self, c: Cmd) {
		if self.instant {
			self.cmd = Some(c);
		}
	}

	/// The cursor moved to overlay pixels (x, y).
	pub fn cursor(&mut self, x: f32, y: f32) {
		if self.pos != Some((x, y)) {
			self.moved = true;
		}
		self.pos = Some((x, y));
	}

	/// The cursor moved to overlay pixels (x, y) while it is hidden (a pad / keyboard player bumped the desk): the place is kept (a
	/// click that shows the cursor lands there) but the move is no aim.
	pub fn track(&mut self, x: f32, y: f32) {
		self.pos = Some((x, y));
	}

	pub fn click(&mut self) {
		self.click = true;
		self.down = true;
	}

	/// The left button went up: a bar drag ends.
	pub fn button_up(&mut self) {
		self.down = false;
	}

	pub fn back(&mut self) {
		self.back = true;
	}

	/// Wheel notches: positive = scrolled up / away.
	pub fn wheel(&mut self, y: f32) {
		self.wheel += if y > 0.0 { 1 } else if y < 0.0 { -1 } else { 0 };
	}

	fn fire(&mut self, b: u16) -> u16 {
		self.command(Cmd::Cancel);
		self.steer = None;
		self.pulse = b;
		self.left = if self.slow { SLOW_PRESS + SLOW_RELEASE } else { PRESS + RELEASE };
		self.advance()
	}

	fn advance(&mut self) -> u16 {
		let out = if self.left > self.release() { self.pulse } else { 0 };
		self.left -= 1;
		out
	}

	/// A menu button is down on the keyboard or pad (`kbd` of `step`): the player works the menu by hand and the mouse gives way. A
	/// steering pulse in flight is cut at once, the goal is dropped and the pointer's place becomes the anchor the next hover must
	/// leave by `KEY_MOVE_PX` (a pointer resting where it is, jittering or under a scrolling list, no longer re-aims over the key
	/// presses). A click / right click / wheel notch waits (`PEND_FIELDS`) and goes out once the keys are up. A pulse the mouse fired
	/// for a click or wheel notch is let finish; the result is what it still presses.
	fn keys_held(&mut self) -> u16 {
		self.command(Cmd::Cancel);
		if self.steer.take().is_some() {
			self.left = 0;
		}
		self.goal = None;
		self.quiet = true;
		self.anchor = self.pos;
		self.moved = false;
		// The keys start a new episode: presses the mouse spent on the hovered row before do not count against it.
		self.spent = 0;
		if self.click || self.back || self.wheel != 0 {
			self.pend += 1;
			if self.pend > PEND_FIELDS {
				(self.click, self.back, self.wheel, self.pend) = (false, false, 0, 0);
			}
		} else {
			self.pend = 0;
		}
		if self.left > 0 { self.advance() } else { 0 }
	}

	/// Notes when the shop's quantity prompt opened (and that it is gone again).
	fn watch_prompt(&mut self, m: &MenuSnap) {
		match (super::mouse_vendor::in_prompt(m), self.prompt_since) {
			(true, None) => self.prompt_since = Some(std::time::Instant::now()),
			(false, Some(_)) => self.prompt_since = None,
			_ => {}
		}
	}

	/// The hover bookkeeping belongs to one page: another page starts over, and a click / right click / wheel notch still waiting
	/// for the keys (`keys_held`) was meant for the page that is gone: it must not fire on the new one.
	fn new_page(&mut self, id: u32) {
		if self.page != Some(id) {
			if self.page.is_some() {
				(self.click, self.back, self.wheel, self.pend) = (false, false, 0, 0);
			}
			self.command(Cmd::Cancel);
			self.drag = None;
			(self.page, self.hover, self.spent, self.gave_up) = (Some(id), None, 0, false);
		}
	}

	/// The pointer aims at `under`: it becomes the anchor, the keys no longer hold the menu, and a new target starts with a fresh
	/// press budget.
	fn aim_at(&mut self, under: Option<Target>) {
		self.anchor = self.pos;
		self.quiet = false;
		if under != self.hover {
			// The pointer aims at something else: an instant selection on its way to the old target is void.
			self.command(Cmd::Cancel);
			(self.hover, self.spent, self.gave_up) = (under, 0, false);
		}
	}

	/// The pointer moved: when it left the anchor by `JITTER_PX` (`KEY_MOVE_PX` after a key press) it aims at what is under it, and
	/// the selection is steered there unless steering already gave that target up. The same target again keeps its goal.
	fn hover_aim(&mut self, m: &MenuSnap, under: Option<Target>) {
		let Some((x, y)) = self.pos else { return };
		if let Some((ax, ay)) = self.anchor {
			if (x - ax).fhypot(y - ay) < if self.quiet { KEY_MOVE_PX } else { JITTER_PX } {
				return;
			}
		}
		let keep = under == self.hover && self.goal.is_some();
		self.aim_at(under);
		if keep {
			return;
		}
		self.goal = match under {
			// Hovering something already selected, a hint line or nothing: the mouse has no say.
			Some(t) if !matches!(t, Target::Hint(_)) && !m.selected(t) && !self.gave_up => Some(Goal::new(t, m.id, false)),
			_ => None,
		};
	}

	/// A left click: a hint line presses its button, an item becomes a goal that ends with Cross, and a click on no item confirms
	/// the highlighted one (the screen's confirm button, the one Enter / Space press: `menu_keys::screen`, Cross on most pages).
	/// A click is a fresh aim: whatever steering gave up before is tried again.
	fn click_at(&mut self, m: &MenuSnap, under: Option<Target>) -> Option<u16> {
		self.aim_at(under);
		(self.spent, self.gave_up) = (0, false);
		// The vendor's shop: a click on no icon does nothing (the screen's confirm button would be Cross, which buys).
		if under.is_none() && super::mouse_vendor::strict(m) {
			self.goal = None;
			return Some(0);
		}
		// The quantity prompt has just opened: this click is the second half of a double click on the icon that opened it.
		if prompt_guard(self.prompt_since, std::time::Instant::now()) {
			self.goal = None;
			return Some(0);
		}
		// A Sound Options bar: select the row and set the volume where the bar was hit; the button held drags it.
		if self.instant && self.bar_click(m, under) {
			self.goal = None;
			return Some(0);
		}
		// A question whose answers are pad buttons (Quit Game, the overwrite box): the answer under the pointer, nothing elsewhere. This
		// safety does not depend on the speed switch (RAC_MOUSE_INSTANT=0 keeps it): a stray click must never be the screen's confirm.
		if let Some((x, y)) = self.pos {
			if let Some(b) = super::mouse_answers::press_at(m, x, y) {
				self.goal = None;
				return Some(self.fire(b));
			}
			if under.is_none() && !super::mouse_answers::zones(m).is_empty() {
				self.goal = None;
				return Some(0);
			}
		}
		match under {
			Some(Target::Hint(b)) => {
				self.goal = None;
				Some(self.fire(b))
			}
			Some(t) => {
				self.goal = Some(Goal::new(t, m.id, true));
				None
			}
			None => {
				self.goal = None;
				Some(self.fire(super::menu_keys::screen(m).confirm))
			}
		}
	}

	/// A wheel notch: left / right over a value row, L1 / R1 over the ring, else up / down.
	fn wheel_press(&mut self, m: &MenuSnap, under: Option<Target>, wheel: i32) -> u16 {
		// The notch moves the selection off the pointer's row on purpose, like a key: the pointer's place becomes the anchor (a click's
		// nudge of a pixel or two must not aim back at the old row).
		(self.goal, self.anchor, self.quiet) = (None, self.pos, true);
		if let Some(r) = Self::wheel_request(m, wheel).or_else(|| super::mouse_vendor::wheel_request(m, wheel)).filter(|_| self.instant) {
			self.command(Cmd::Submit(r));
			return 0;
		}
		// The vendor's shop (mouse_vendor.rs): Left / Right over the strip, the quantity in the prompt (one press per notch, the rest queued).
		if super::mouse_vendor::strict(m) {
			if super::mouse_vendor::in_prompt(m) {
				self.wheel = wheel - wheel.signum();
			}
			return self.fire(super::mouse_vendor::wheel_button(m, wheel > 0));
		}
		let kind_of = |addr: u32| m.items.iter().find(|i| i.addr == addr).map(|i| i.kind.clone());
		let on = match under {
			Some(Target::Sub { item, .. }) => kind_of(item),
			_ => None,
		};
		let up = wheel > 0;
		let b = match on.or_else(|| kind_of(m.cur)) {
			Some(Kind::Value { .. }) => if up { RIGHT } else { LEFT },
			Some(Kind::Ring { .. }) => if up { L1 } else { R1 },
			_ => if up { UP } else { DOWN },
		};
		self.fire(b)
	}

	/// The instant selection for a goal on a row of a text list or value page (`mouse_instant`), when that is on.
	fn instant_request(&self, m: &MenuSnap, g: &Goal) -> Option<Request> {
		let Target::Sub { item, sub } = g.target else { return None };
		let it = m.item(item)?;
		(self.instant && matches!(it.kind, Kind::Label { .. } | Kind::Value { .. } | Kind::Grid { .. })).then_some(Request { page_id: m.id, item, sub, confirm: g.confirm, slide: None })
	}

	/// A click on a Sound Options bar: the instant request that selects its row and sets the volume at the pointer, and the drag it starts.
	fn bar_click(&mut self, m: &MenuSnap, under: Option<Target>) -> bool {
		let (Some(Target::Sub { item, sub }), Some((x, _))) = (under, self.pos) else { return false };
		let Some(rect) = mouse_slider::bar_item(m, item, sub).filter(|&r| mouse_slider::on_bar(r, x)) else { return false };
		self.drag = Some((item, sub));
		self.command(Cmd::Submit(Request { page_id: m.id, item, sub, confirm: false, slide: Some(Slide::To(mouse_slider::value_at(rect, x))) }));
		true
	}

	/// The button is held on a bar: a pointer move sets the volume under it (the row stays the one the drag started on, wherever the
	/// pointer goes). True while the drag lasts (no hover aims meanwhile).
	fn drag_step(&mut self, m: &MenuSnap, moved: bool) -> bool {
		let Some((item, sub)) = self.drag else { return false };
		let Some(rect) = mouse_slider::bar_item(m, item, sub).filter(|_| self.down) else {
			self.drag = None;
			return false;
		};
		if let (true, Some((x, _))) = (moved, self.pos) {
			self.command(Cmd::Submit(Request { page_id: m.id, item, sub, confirm: false, slide: Some(Slide::To(mouse_slider::value_at(rect, x))) }));
		}
		true
	}

	/// Wheel notches on the current item as ONE instant request: `wheel` rows up / down a text list (selectable rows only, no wrap: at the
	/// ends the pulse keeps retail's own wrap), rows of a grid (at the grid's edge the pulse keeps the neighbour item), or volume steps on
	/// a Sound Options bar (`mouse_slider::WHEEL` per notch). None: other items keep the pulse.
	fn wheel_request(m: &MenuSnap, wheel: i32) -> Option<Request> {
		let it = m.item(m.cur)?;
		let (n, up) = (wheel.unsigned_abs(), wheel > 0);
		let rq = |sub, slide| Request { page_id: m.id, item: it.addr, sub, confirm: false, slide };
		match &it.kind {
			Kind::Label { rows, cursor, .. } if rows.len() >= 2 => {
				let sel: Vec<u32> = rows.iter().enumerate().filter(|(_, r)| r.1).map(|(i, _)| i as u32).collect();
				let at = sel.iter().position(|i| i == cursor)?;
				let to = if up { at.saturating_sub(n as usize) } else { (at + n as usize).min(sel.len() - 1) };
				(to != at).then(|| rq(sel[to], None))
			}
			Kind::Grid { rows, cols, cursor } => {
				let (r, c) = (cursor / cols, cursor % cols);
				let tr = if up { r.saturating_sub(n) } else { (r + n).min(rows - 1) };
				(tr != r).then(|| rq(tr * cols + c, None))
			}
			Kind::Value { cursor, .. } if mouse_slider::bar_item(m, it.addr, *cursor).is_some() => {
				Some(rq(*cursor, Some(Slide::By(if up { 1 } else { -1 } * mouse_slider::WHEEL * n as i32))))
			}
			_ => None,
		}
	}

	/// One steering step toward the goal: the button that moves the selection a step, or the goal is reached (a click then presses
	/// Cross), or it is given up: a button the game does not follow three presses in a row, the budget `MAX_PRESSES` of the hovered
	/// target spent, or the selection passed the goal (the next step would be the opposite of the last, inside the goal's own item).
	fn steer_goal(&mut self, m: &MenuSnap) -> u16 {
		let Some(mut g) = self.goal else { return 0 };
		// A click on a row the worker takes: the worker decides on the LIVE selection (this snapshot is two fields old and the worker may
		// have moved the selection since): Cross when the row is selected there, the move first when it is not. Never a Cross from here.
		if g.confirm {
			if let Some(r) = self.instant_request(m, &g) {
				self.command(Cmd::Submit(r));
				self.goal = None;
				return 0;
			}
		}
		if m.selected(g.target) {
			self.goal = None;
			return if g.confirm { self.fire(CROSS) } else { 0 };
		}
		if let Some(r) = self.instant_request(m, &g) {
			// The worker takes the selection from here (rows of text lists and value pages): no pulses.
			self.command(Cmd::Submit(r));
			self.goal = None;
			return 0;
		}
		let sig = m.sig();
		g.stuck = if g.presses > 0 && sig == g.sig { g.stuck + 1 } else { 0 };
		g.sig = sig;
		match m.toward(g.target) {
			Some(b) if self.spent < MAX_PRESSES && g.stuck < 3 && b != opposite(g.last) => {
				(g.presses, g.last) = (g.presses + 1, if m.inside(g.target) { b } else { 0 });
				self.spent += 1;
				self.goal = Some(g);
				let out = self.fire(b);
				self.steer = Some((m.id, sig));
				out
			}
			_ => {
				(self.goal, self.gave_up) = (None, true);
				0
			}
		}
	}

	/// One field: the pad bits the mouse adds. `menu` is the last snapshot (None = no menu: everything is dropped), `kbd` the pad
	/// buttons the player holds this field, keyboard or controller (`keys_held`): a menu button, or the left stick pushed to a
	/// direction (`stick_bits`), takes the menu away from the mouse.
	pub fn step(&mut self, menu: Option<&MenuSnap>, kbd: u16) -> u16 {
		let Some(m) = menu else {
			// The pointer's place stays the anchor: a 1 px jitter of a pointer resting over the next menu does not aim (the jitter
			// rule only works against an anchor).
			*self = State { pos: self.pos, anchor: self.pos, instant: self.instant, slow: self.slow, cmd: self.instant.then_some(Cmd::Cancel), ..State::default() };
			return 0;
		};
		self.new_page(m.id);
		self.watch_prompt(m);
		if kbd & MENU_BTNS != 0 {
			return self.keys_held();
		}
		self.pend = 0;
		if self.left > 0 {
			// A steering pulse whose press the game already took (the selection moved, or the page changed) ends after its first
			// released field: the next step goes out at once instead of after the full release.
			let landed = self.left < self.release() && self.steer.is_some_and(|(page, sig)| page != m.id || sig != m.sig());
			if !landed {
				return self.advance();
			}
			self.left = 0;
			self.steer = None;
		}
		// A page flying in or closing: wheel notches and clicks of this field are lost (the game ignores the pad then too, and a
		// double click must not confirm a row of the page it opened); a hover waits for the page.
		if m.busy() {
			self.goal = None;
			(self.click, self.back, self.wheel) = (false, false, 0);
			return 0;
		}
		let (moved, click, back, wheel) = (std::mem::take(&mut self.moved), std::mem::take(&mut self.click), std::mem::take(&mut self.back), std::mem::take(&mut self.wheel));
		if self.goal.is_some_and(|g| g.page != m.id) {
			self.goal = None;
		}
		let under = self.pos.and_then(|(x, y)| m.hit(x, y));
		if back {
			self.goal = None;
			return self.fire(m.back_button());
		}
		if click {
			if let Some(b) = self.click_at(m, under) {
				return b;
			}
		} else if self.drag_step(m, moved) {
			// A bar drag: the pointer sets the volume, it aims at nothing.
		} else if moved {
			self.hover_aim(m, under);
		}
		if wheel != 0 {
			return self.wheel_press(m, under, wheel);
		}
		self.steer_goal(m)
	}
}

#[cfg(test)]
mod tests;
