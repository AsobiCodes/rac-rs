//! Mouse in the retail menus (the page-menu engine FUN_00218da0: pause menu and its pages, Quick Select ring, galactic
//! map screen). Nothing is written to the game: the mouse only ever presses pad buttons, exactly like a hand on the pad.
//!
//! * `read` (worker thread, every field in mode 3) copies the menu's state out of RAM: the current page, its items with
//!   their screen rectangles (512x448 PS2 pixels, the space the overlay is drawn in) and each item's own cursor.
//! * `State` (main thread) turns the mouse into pad presses: hovering steers the game's own selection by D-pad / L1 / R1
//!   pulses until the selection read from RAM is the hovered one, a left click adds Cross (on no item: the screen's confirm
//!   button, the one Enter presses, for the highlighted item), a right click is the screen's
//!   back button, the wheel steps up / down (left / right over values, L1 / R1 over the ring), and the button-hint lines
//!   press their button. See <dev>/scratch_play/handoff/mouse_lean.md for the RAM layout [C].

use crate::retail::disc::va;
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
const MENU_BTNS: u16 = UP | RIGHT | DOWN | LEFT | CROSS | CIRCLE | TRIANGLE | 0x80 | L1 | R1 | 0x800;

/// The overlay's pixel space.
const SCREEN: (f32, f32) = (512.0, 448.0);
/// Fields a pulse holds the button, and fields it then stays released (the menu's auto-repeat needs a fresh press). A steering
/// pulse ends after its first released field once the snap shows the selection moved (`State::step`): 3 fields a step
/// instead of 5 when the game takes the press at once, the full 5 when it does not.
const PRESS: u8 = 2;
const RELEASE: u8 = 3;
/// Presses spent steering to one goal before giving up.
const MAX_PRESSES: u32 = 24;
const MASK: u32 = 0x01ff_ffff;

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
fn hint_button(text: u16) -> Option<u16> {
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
	if it == super::pcext::TGVAL {
		let n = (0..16).take_while(|k| rd(rd(it + 0x34) + 20 * k) != 0).count() as u32;
		return Kind::Value { n: n.max(1), cursor: rd(it + 0x38) };
	}
	if matches!(id, 0x13 | 0x1b | 0x1d | 0x20) && slot == 3 {
		let n = match id {
			0x13 => 2,
			0x1b => 1,
			0x20 => 3,
			// Camera Options (0x1d): the rows of the item's table (3 as retail, 5 with the PC options' two rows, retail/pcext.rs;
			// the Graphics page's copy has 13).
			_ => match (0..16).take_while(|k| rd(rd(it + 0x34) + 24 * k) != 0).count() as u32 {
				0 => 3,
				n => n,
			},
		};
		return Kind::Value { n, cursor: rd(it + if id == 0x20 { 0x40 } else { 0x38 }) };
	}
	if id == 4 && matches!(slot, 2 | 3 | 9 | 10) {
		return Kind::Panel;
	}
	if matches!(id, 3 | 5) {
		let (rows, cols, cells) = (rd(it + 0x40), rd(it + 0x44), rd(it + 0x48));
		if (1..=8).contains(&rows) && (1..=8).contains(&cols) && ram(cells) {
			return Kind::Grid { rows, cols, cursor: rd(it + 0x3c) };
		}
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
	let mut recs = Vec::new();
	for k in 0..14 {
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
	(cx + a.cos() * rad, cy + a.sin() * rad)
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
					.map(|(i, (sx, sy))| (i, (sx - x).hypot(sy - y)))
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
}

impl State {
	/// The cursor moved to overlay pixels (x, y).
	pub fn cursor(&mut self, x: f32, y: f32) {
		if self.pos != Some((x, y)) {
			self.moved = true;
		}
		self.pos = Some((x, y));
	}

	pub fn click(&mut self) {
		self.click = true;
	}

	pub fn back(&mut self) {
		self.back = true;
	}

	/// Wheel notches: positive = scrolled up / away.
	pub fn wheel(&mut self, y: f32) {
		self.wheel += if y > 0.0 { 1 } else if y < 0.0 { -1 } else { 0 };
	}

	fn fire(&mut self, b: u16) -> u16 {
		self.steer = None;
		self.pulse = b;
		self.left = PRESS + RELEASE;
		self.advance()
	}

	fn advance(&mut self) -> u16 {
		let out = if self.left > RELEASE { self.pulse } else { 0 };
		self.left -= 1;
		out
	}


	/// One field: the pad bits the mouse adds. `menu` is the last snapshot (None = no menu: everything is dropped),
	/// `kbd` the keyboard's buttons this field (a menu button from the keyboard cancels the mouse goal).
	pub fn step(&mut self, menu: Option<&MenuSnap>, kbd: u16) -> u16 {
		let Some(m) = menu else {
			*self = State { pos: self.pos, ..State::default() };
			return 0;
		};
		if self.left > 0 {
			// A steering pulse whose press the game already took (the selection moved, or the page changed) ends after its first
			// released field: the next step goes out at once instead of after the full release.
			let landed = self.left < RELEASE && self.steer.is_some_and(|(page, sig)| page != m.id || sig != m.sig());
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
		if kbd & MENU_BTNS != 0 {
			self.goal = None;
			return 0;
		}
		if self.goal.is_some_and(|g| g.page != m.id) {
			self.goal = None;
		}
		let under = self.pos.and_then(|(x, y)| m.hit(x, y));
		if back {
			self.goal = None;
			return self.fire(m.back_button());
		}
		if click {
			match under {
				Some(Target::Hint(b)) => {
					self.goal = None;
					return self.fire(b);
				}
				Some(t) => self.goal = Some(Goal { target: t, page: m.id, confirm: true, presses: 0, sig: (0, 0), stuck: 0 }),
				// A click on no item confirms the highlighted one: the screen's confirm button, the one Enter / Space press
				// (`menu_keys::screen`: Cross on most pages).
				None => {
					self.goal = None;
					return self.fire(super::menu_keys::screen(m).confirm);
				}
			}
		} else if moved {
			match under {
				Some(t) if !matches!(t, Target::Hint(_)) && !m.selected(t) => self.goal = Some(Goal { target: t, page: m.id, confirm: false, presses: 0, sig: (0, 0), stuck: 0 }),
				// Hovering something already selected, a hint line or nothing: the mouse has no say.
				_ => self.goal = None,
			}
		}
		if wheel != 0 {
			self.goal = None;
			let on_value = |t: Option<Target>| match t {
				Some(Target::Sub { item, .. }) => m.items.iter().find(|i| i.addr == item).map(|i| i.kind.clone()),
				_ => None,
			};
			let kind = on_value(under).or_else(|| m.items.iter().find(|i| i.addr == m.cur).map(|i| i.kind.clone()));
			let up = wheel > 0;
			let b = match kind {
				Some(Kind::Value { .. }) => if up { RIGHT } else { LEFT },
				Some(Kind::Ring { .. }) => if up { L1 } else { R1 },
				_ => if up { UP } else { DOWN },
			};
			return self.fire(b);
		}
		let Some(mut g) = self.goal else { return 0 };
		if m.selected(g.target) {
			self.goal = None;
			return if g.confirm { self.fire(CROSS) } else { 0 };
		}
		let sig = m.sig();
		g.stuck = if g.presses > 0 && sig == g.sig { g.stuck + 1 } else { 0 };
		g.sig = sig;
		match m.toward(g.target) {
			Some(b) if g.presses < MAX_PRESSES && g.stuck < 3 => {
				g.presses += 1;
				self.goal = Some(g);
				let out = self.fire(b);
				self.steer = Some((m.id, sig));
				out
			}
			_ => {
				self.goal = None;
				0
			}
		}
	}
}

#[cfg(test)]
mod tests {
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
		// every step ends after one released field: presses 3 fields apart.
		presses.clear();
		st.cursor(200.0, 200.0);
		run(&mut st, &mut m, 60, &mut presses, &mut crossed);
		assert_eq!(m.cur, 0x40);
		assert_eq!(presses.len(), 3);
		assert!(presses.windows(2).all(|w| w[1] - w[0] == 3), "{presses:?}");
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
	/// field it arrives steps every 3 fields and never overshoots; a game that ignores presses keeps the full 5-field pulses.
	#[test]
	fn steering_follows_the_game_without_overshoot() {
		for (takes, gap) in [(true, 3u32), (false, 5)] {
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
		// 2 fields per pulse; the first press plus three repeats that change nothing.
		assert!((6..=10).contains(&pressed), "{pressed}");
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
}
