//! The Gadgetron shop screen's 2D layout (decomp/notes/vendor.md "Shop screen", retail PAL capture):
//! green LED ticker across the top, bolt counter top right (the shared HUD), left panel with the item name
//! and price, a large spinning item in the centre, "Cross Buy" bottom left, "Triangle Exit" bottom right and
//! a row of item boxes at the bottom centre with the selected one framed in yellow.
//! Panel content follows the third PAL audit metrics (item name (6,8), price right-aligned at (118,101), Exit
//! centred at (40,20), icons 48x48 from (12,6) at a 56 pitch). The LED ticker (UI-5e) and the noise / sweep overlays
//! (UI-5c) follow vendor.md. [P]: panel art (plain rects) and the live class-12 vendor portrait are not drawn.

use crate::frontend::Text;
use crate::game::vendor::{self, Shop};
use crate::hud as art;
use crate::inventory::Inventory;
use crate::overlay::{self, Quad};

type Font = (usize, Vec<art::Glyph>);

/// Inner-rect margins per screen (world units; overlay data 0x1ca698 + 16*i): start offset along A and B, then
/// half the total shrink along A and B (FUN_002b2a60: origin += nA*m0 + nB*m1, A' = nA*(|A| - 2 m2), B' = nB*(|B| - 2 m3)).
const MARGIN: [[f32; 4]; 6] = [[0.01, 0.01, 0.009, 0.01], [0.01, 0.03, 0.03, 0.03], [0.03, 0.0, 0.0, 0.0], [0.02, 0.06, 0.04, 0.035], [0.07, 0.05, 0.03, 0.03], [0.03, 0.06, 0.02, 0.03]];

/// Joints of the screens' attach points 4i, 4i+1, 4i+2 (class 11 point table via FUN_00253bb0; read with `anim::attach_joint`).
const POINTS: [[usize; 3]; 6] = [[27, 26, 29], [15, 14, 17], [33, 32, 35], [9, 8, 11], [21, 20, 23], [3, 2, 5]];

/// The six screens' inner rects (window px x, y, w, h): screen i spans vendor joints origin, axis A end and axis B end
/// (`POINTS`); `joints` are world positions of joints 0..36, `project` maps a world point to window pixels.
pub fn screen_rects(joints: &[[f32; 3]], project: &dyn Fn([f32; 3]) -> Option<[f32; 2]>) -> [[f32; 4]; 6] {
	let mut out = [[0.0; 4]; 6];
	for i in 0..6 {
		let (j0, a, b) = (joints[POINTS[i][0]], joints[POINTS[i][1]], joints[POINTS[i][2]]);
		let (va, vb) = ([a[0] - j0[0], a[1] - j0[1], a[2] - j0[2]], [b[0] - j0[0], b[1] - j0[1], b[2] - j0[2]]);
		let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
		let (la, lb) = (len(va), len(vb));
		let m = MARGIN[i];
		let at = |s: f32, t: f32| [0, 1, 2].map(|k| j0[k] + va[k] / la * s + vb[k] / lb * t);
		let (p0, p1) = (at(m[0], m[1]), at(m[0] + la - 2.0 * m[2], m[1] + lb - 2.0 * m[3]));
		if let (Some(q0), Some(q1)) = (project(p0), project(p1)) {
			out[i] = [q0[0], q0[1], q1[0] - q0[0], q1[1] - q0[1]];
		}
		if std::env::var("RAC_SHOP_DEBUG").is_ok() {
			eprintln!("screen {i}: joints {j0:?} {a:?} {b:?} rect {:?}", out[i]);
		}
	}
	out
}

/// UI-5d popup margins (FUN_002b3370 OV:112838-112844): start offset along A and B, then half the length change.
const POPUP_MARGIN: [f32; 4] = [0.03, 0.04, 0.03, 0.02];

/// UI-5d: the purchase popup's rect (window px x, y, w, h) from the prompt prop's attach points 0 / 1 / 2 (origin, A end, B end;
/// class 1137: joints 2 / 1 / 4; NOV-35a),
/// the same arithmetic as one `screen_rects` screen (vendor.md UI-5d).
pub fn popup_rect(joints: &[[f32; 3]; 3], project: &dyn Fn([f32; 3]) -> Option<[f32; 2]>) -> Option<[f32; 4]> {
	let (j0, a, b) = (joints[0], joints[1], joints[2]);
	let (va, vb) = ([a[0] - j0[0], a[1] - j0[1], a[2] - j0[2]], [b[0] - j0[0], b[1] - j0[1], b[2] - j0[2]]);
	let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
	let (la, lb) = (len(va), len(vb));
	let m = POPUP_MARGIN;
	let at = |s: f32, t: f32| [0, 1, 2].map(|k| j0[k] + va[k] / la * s + vb[k] / lb * t);
	let (q0, q1) = (project(at(m[0], m[1]))?, project(at(m[0] + la - 2.0 * m[2], m[1] + lb - 2.0 * m[3]))?);
	Some([q0[0], q0[1], q1[0] - q0[0], q1[1] - q0[1]])
}

/// Glyph advance of a character code in font 2 (0 adds nothing).
fn adv(font: &Font, c: char) -> i32 {
	font.1.get(c as usize).map_or(0, |g| g.adv as i32)
}

/// UI-5d: FUN_0021d678's split of `s` into lines for a box `wbox` wide (vendor.md UI-5d Order). Returns (start, end)
/// char index pairs, end INCLUSIVE after the trailing-space step (an end before start is an empty line), and the
/// advance sum the last line's scan ended with.
fn split_lines(font: &Font, s: &[char], wbox: i32) -> (Vec<(usize, i32)>, i32) {
	let at = |i: usize| s.get(i).map_or(0u32, |&c| c as u32);
	let (mut lines, mut start) = (Vec::new(), 0usize);
	let mut sum;
	if at(0) == 0 {
		return (lines, 0);
	}
	loop {
		sum = 0;
		let (mut i, mut brk) = (start, start);
		while wbox > 0 {
			let ch = at(i);
			if ch == 0x20 || ch < 0x10 {
				brk = i;
			}
			if ch < 2 {
				break;
			}
			i += 1;
			sum += adv(font, char::from_u32(ch).unwrap_or('\0'));
			if sum >= wbox {
				break;
			}
		}
		let end = if brk == start { i } else { brk };
		let drawn = if at(end) == 0x20 || at(end) < 0x10 { end as i32 - 1 } else { end as i32 };
		lines.push((start, drawn));
		if at(end) == 0 || end + 1 >= s.len() || at(end + 1) == 0 {
			break;
		}
		start = end + 1;
	}
	(lines, sum)
}

/// UI-5d: the balanced line split of FUN_0021d678 (box width `wbox`): narrow the box by 16 while the last line is shorter
/// than a third of it, and fall back to the original width when that adds a line.
pub fn box_lines(font: &Font, s: &str, wbox: i32) -> Vec<(usize, usize)> {
	let chars: Vec<char> = s.chars().collect();
	let (mut lines, mut last) = split_lines(font, &chars, wbox);
	let n1 = lines.len();
	let mut cur = wbox;
	while lines.len() >= 2 && last < cur / 3 {
		cur -= 16;
		let (l2, s2) = split_lines(font, &chars, cur);
		if l2.len() > n1 {
			lines = split_lines(font, &chars, wbox).0;
			break;
		}
		lines = l2;
		last = s2;
	}
	lines.into_iter().map(|(a, e)| (a, (e + 1).max(a as i32) as usize)).collect()
}

/// NOV-30h: FUN_002b1d60's two-pass box text in a `w` x `h` texel panel: the measure pass splits the text (box width
/// 2 * min(w - w/2, w/2)), the draw pass starts at y (h - lines*16) >> 1 with 16-texel lines, each centred on w >> 1 and
/// kept while it overlaps [0, h]. Returns (texel x, texel y, line text).
pub fn buy_lines(font: &Font, s: &str, w: i32, h: i32) -> Vec<(i32, i32, String)> {
	let chars: Vec<char> = s.chars().collect();
	let wbox = 2 * (w - (w >> 1)).min(w >> 1);
	let lines = box_lines(font, s, wbox);
	let y0 = (h - lines.len() as i32 * 16) >> 1;
	let mut v = Vec::new();
	for (i, &(a, e)) in lines.iter().enumerate() {
		let y = y0 + 16 * i as i32;
		if y + 16 < 0 || y > h {
			continue;
		}
		let line: String = chars[a.min(chars.len())..e.min(chars.len())].iter().collect();
		let width: i32 = line.chars().map(|c| adv(font, c)).sum();
		v.push(((w >> 1) - (width >> 1), y, line));
	}
	v
}

/// NOV-30h: FUN_0021cbf8's glyph run (shop panels 3 and 4 only): codes 8..15 and zero-advance codes draw nothing and do
/// not advance; 0x80..0xa7 first blit the accent cell of entry c + 0x40 at its signed (+3, +2) byte offsets; codes below
/// 0x20 are 24x16 cells, codes above 0x20 16x16; space only advances. One blit per glyph, no drop shadow.
fn text_glyphs(out: &mut Vec<Quad>, font: &Font, s: &str, x: f32, y: f32, kx: f32, ky: f32, colour: [f32; 4]) {
	let cell = |out: &mut Vec<Quad>, g: &art::Glyph, x: f32, y: f32, cw: f32| {
		let (u, v) = (g.u as f32, g.v as f32);
		out.push(Quad { tex: font.0, rect: [x, y, cw * kx, 16.0 * ky], uv: [u / 256.0, v / 128.0, (u + cw) / 256.0, (v + 16.0) / 128.0], colour, angle: 0.0, corners: None });
	};
	let mut x = x;
	for c in s.chars() {
		let code = c as u32;
		let Some(g) = font.1.get(code as usize) else { continue };
		if (8..=15).contains(&code) || g.adv == 0 {
			continue;
		}
		if (0x80..=0xa7).contains(&code) {
			if let Some(e) = font.1.get(code as usize + 0x40) {
				cell(out, e, x + (e.adv as i8) as f32 * kx, y + (e.dy as i8) as f32 * ky, 16.0);
			}
		}
		if code < 0x20 {
			cell(out, g, x, y + g.dy as f32 * ky, 24.0);
		} else if code > 0x20 {
			cell(out, g, x, y + g.dy as f32 * ky, 16.0);
		}
		x += g.adv as f32 * kx;
	}
}

/// NOV-30h: the Exit panel text id of FUN_002b1fb8: 0x4ee0 while browsing; with the popup open nothing when the row
/// cannot be bought (ammo full, or bolts below the unit / item price), else 0x4ee0 for ammo and 0x524b for a weapon.
fn exit_text(shop: &Shop, row: &vendor::Row, bolts: u32, inv: &Inventory) -> Option<u32> {
	if shop.stage == 0 {
		return Some(vendor::TEXT_EXIT);
	}
	let Some(p) = vendor::price(row.item) else { return Some(vendor::TEXT_EXIT) };
	if row.ammo {
		if p.max <= inv.ammo[row.item as usize] || bolts < p.ammo {
			return None;
		}
		return Some(vendor::TEXT_EXIT);
	}
	let price = if inv.have[35] { p.alt } else { p.item };
	(bolts >= price).then_some(vendor::TEXT_NO)
}

/// 1234567 -> "1,234,567".
pub fn commas(n: u32) -> String {
	let s = n.to_string();
	let mut out = String::new();
	for (i, c) in s.chars().enumerate() {
		if i > 0 && (s.len() - i) % 3 == 0 {
			out.push(',');
		}
		out.push(c);
	}
	out
}

/// NOV-30d: the struck price's width over glyph table 0x1c3550 (advance 13 per digit, 6 per comma; a price string holds
/// only those).
fn strike_width(s: &str) -> f32 {
	s.chars().map(|c| if c == ',' { 6.0 } else { 13.0 }).sum()
}

/// Item names by id (all_text via `inventory::name_text`; the literals only cover a missing text table, e.g. tests).
fn name(text: &Text, item: u8) -> String {
	let n = crate::inventory::name_text(item);
	let s = if n != 0 { text.get(n) } else { "" };
	match (s.is_empty(), item) {
		(false, _) => s.to_string(),
		(true, 16) => "Pyrocitor".to_string(),
		(true, 10) => "Bomb Glove".to_string(),
		_ => format!("Item {item}"),
	}
}

/// UI-5e LED font glyph (vendor.md UI-5e, FUN_002b1168): destination rect in target px and source texels (u, v, du, dv)
/// on sprite sheet 0xe935.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LedGlyph {
	pub rect: [f32; 4],
	pub uv: [f32; 4],
}

/// Texel origin and advance of an LED glyph byte (`None`: skipped with no advance). Space is (None-less) handled by the caller.
fn led_glyph(ch: u8) -> Option<((i32, i32), i32)> {
	Some(match ch {
		b'!' => ((18, 36), 5),
		b'\'' => ((0, 45), 5),
		b',' => ((27, 36), 5),
		b'-' => ((45, 36), 9),
		b'.' => ((36, 36), 5),
		b'A'..=b'Z' => {
			let k = (ch - b'A') as i32;
			const ADV: [i32; 26] = [10, 10, 11, 10, 9, 9, 11, 10, 4, 10, 10, 9, 11, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10, 10];
			((9 * (k % 6), 9 * (k / 6)), ADV[k as usize])
		}
		_ => return None,
	})
}

/// UI-5e: the LED font walk. `x0`, `y` in target px, `frame` the blink counter (hidden while `frame % 33 < 8` for a
/// character following 'b'). Returns the drawn glyph quads in target space.
pub fn led_quads(s: &str, x0: i32, y: i32, scale: f32, frame: u32) -> Vec<LedGlyph> {
	let (mut x, mut blink) = (x0, false);
	let w = (9.0 * scale) as i32;
	let mut out = Vec::new();
	for ch in s.bytes() {
		if ch == b'b' {
			blink = true;
		}
		if ch < 0x20 || ch >= 0x5b {
			continue;
		}
		let (cell, adv) = if ch == b' ' {
			(None, 9)
		} else {
			match led_glyph(ch) {
				Some((c, a)) => (Some(c), a),
				None => continue,
			}
		};
		let hidden = std::mem::take(&mut blink) && frame % 33 < 8;
		if let Some((u, v)) = cell.filter(|_| !hidden && x + 9 > 0 && x < 0x100) {
			out.push(LedGlyph { rect: [x as f32, y as f32, w as f32, w as f32], uv: [u as f32, v as f32, 9.0, 9.0] });
		}
		x += (adv as f32 * scale) as i32;
	}
	out
}

/// Cuts a glyph's target-space rect to the columns outside the blanked ranges [0, 4) and [0xe2, 0xe6), uv in proportion.
fn led_cut(g: LedGlyph) -> Vec<LedGlyph> {
	let [x, y, w, h] = g.rect;
	let mut out = Vec::new();
	for (lo, hi) in [(4.0, 226.0), (230.0, f32::INFINITY)] {
		let (a, b) = (x.max(lo), (x + w).min(hi));
		if a >= b {
			continue;
		}
		let [u, v, du, dv] = g.uv;
		out.push(LedGlyph { rect: [a, y, b - a, h], uv: [u + du * (a - x) / w, v, du * (b - a) / w, dv] });
	}
	out
}

/// PAL source-height factor of the panel blit (f32 0x3f6db6dc = 13/14).
const PAL_SRC_H: f32 = 0.92857146;
/// NOV-30a: retail w0 is in main-projection PS2 px (256/0.63 per tan); the port measures panel widths in square
/// window px (224/(0.63*0.756) per tan), so the blit source width is w0 x 256 * 0.756 / 224 = 0.864.
const PANEL_W_RETAIL: f32 = 256.0 * 0.756 / 224.0;

/// Window px per texel of a panel (FUN_002b2a60): the content is drawn 1 texel = 1 PS2 screen px in the full
/// rect's size (`full` window px / `k`), and the blit takes texels (0,0)-(w0 - 1, int(h0 * 13/14 - 1)) of it
/// onto the grown rect `grown`.
pub fn panel_scale(full: [f32; 4], grown: [f32; 4], k: f32) -> (f32, f32) {
	let (w0, h0) = (full[2] / k, full[3] / k);
	(grown[2] / (w0 - 1.0), grown[3] / ((h0 * PAL_SRC_H - 1.0) as i32 as f32))
}
/// REN-9b: texture coordinates of the 512x128 item target shown by panel 1: texels (0,0)-(w0 - 1, int(h0 * 13/14 - 1)) with
/// (w0, h0) = the full rect in PS2 px (same box as `panel_scale`).
pub fn panel_uv(full: [f32; 4], k: f32) -> [f32; 4] {
	let (w0, h0) = (full[2] / k * PANEL_W_RETAIL, full[3] / k);
	[0.0, 0.0, (w0 - 1.0) as i32 as f32 / 512.0, (h0 * PAL_SRC_H - 1.0) as i32 as f32 / 128.0]
}
/// Icon strip (FUN_002b1540): 48x48 icons from (12, 6) every 56 px, the selected one in a 56x56 frame, in the panel's
/// own texel space (so it scales through `panel_scale` like the text). Eight or more rows scroll cyclically [P: not
/// ported, Novalis has two].
const ICON: f32 = 48.0;
const ICON_PITCH: f32 = 56.0;
const ICON_ORIGIN: [f32; 2] = [12.0, 6.0];

/// UI-5b: the content of a panel is clipped to its source box (the blit shows texels up to w0 - 1 and the PAL source
/// height, which `panel_scale` maps onto the panel rect): every un-rotated quad pushed since `from` is cut to `clip`,
/// its texture coordinates cut in proportion; quads wholly outside are dropped.
pub fn clip_quads(out: &mut Vec<Quad>, from: usize, clip: [f32; 4]) {
	let mut keep = Vec::with_capacity(out.len() - from.min(out.len()));
	for q in out.drain(from.min(out.len())..) {
		if q.angle != 0.0 {
			keep.push(q);
			continue;
		}
		let [x0, y0, w, h] = q.rect;
		let (x1, y1) = (x0 + w, y0 + h);
		let (cx0, cy0, cx1, cy1) = (x0.max(clip[0]), y0.max(clip[1]), x1.min(clip[0] + clip[2]), y1.min(clip[1] + clip[3]));
		if cx0 >= cx1 || cy0 >= cy1 {
			continue;
		}
		let (ta, tb) = ((cx0 - x0) / w, (cx1 - x0) / w);
		let (tc, td) = ((cy0 - y0) / h, (cy1 - y0) / h);
		let [u0, v0, u1, v1] = q.uv;
		keep.push(Quad { rect: [cx0, cy0, cx1 - cx0, cy1 - cy0], uv: [u0 + (u1 - u0) * ta, v0 + (v1 - v0) * tc, u0 + (u1 - u0) * tb, v0 + (v1 - v0) * td], ..q });
	}
	out.extend(keep);
}

/// UI-5b2 selection frame pulse level v (blue byte; red = green = 2v), vendor.md UI-5b2.
pub fn frame_level(pulse: u32) -> u32 {
	(((pulse & 0xf) as i32) * 4 - 0x20).unsigned_abs() + 0x40
}

/// Draws the six panels into `out` (inner rects from `screen_rects`; content in retail px scaled to each rect). The item
/// models are world mobies (`Shop::item_poses`, NOV-13.3), not drawn here. `icon(item, ammo)` gives the
/// texture slot of icon 0xea60 + item (item record +0x38), frame 0 for a weapon row, frame 2 for an ammo row.
pub fn draw(out: &mut Vec<Quad>, w: f32, h: f32, font: &Font, text: &Text, shop: &Shop, bolts: u32, inv: &Inventory, rects: Option<[[f32; 4]; 6]>, icon: &dyn Fn(u8, bool) -> Option<usize>, panel_tex: Option<usize>, portrait_tex: Option<usize>, fx: Option<[(usize, f32, f32); 3]>, led: Option<(usize, f32, f32)>, popup: Option<[f32; 4]>, popup_tex: Option<usize>) -> Option<usize> {
	// NOV-30e: 0x80f0f0f0 through the GS modulate (texel * vertex >> 7) on font-2 texels of 0x80: 240 / 128 on our
	// 128 / 255 texel samples reproduces retail's 240 (0.94); the struck grey 0x80808080 is 128 / 128 = 1.0.
	let white = [240.0 / 128.0, 240.0 / 128.0, 240.0 / 128.0, 1.0];
	let grey = [1.0, 1.0, 1.0, 1.0];
	let row = shop.row();
	// Opening: the screen fade, then the panels grow from their centres over 8 ticks [P: from the clock].
	if shop.state == 0 {
		overlay::rect(out, [0.0, 0.0, w, h], [0.0, 0.0, 0.0, shop.fade()]);
		return None;
	}
	let Some(r) = rects.filter(|r| r.iter().all(|q| q[2] > 1.0 && q[3].abs() > 1.0)) else { return None };
	let full = r.map(|q| [q[0], q[1].min(q[1] + q[3]), q[2], q[3].abs()]);
	let k = h / 448.0;
	let _ = w;
	let t = (shop.clock * 60.0 / 8.0).min(1.0);
	let r = r.map(|q| {
		let (cx, cy) = (q[0] + q[2] / 2.0, q[1] + q[3] / 2.0);
		[cx - q[2] * t / 2.0, cy - q[3] * t / 2.0, q[2] * t, q[3] * t]
	});
	let norm = |q: [f32; 4]| [q[0], q[1].min(q[1] + q[3]), q[2], q[3].abs()];
	let r = r.map(norm);
	for (i, q) in r.iter().enumerate() {
		match panel_tex.filter(|_| i == 1) {
			// REN-9b: panel 1 shows the 512x128 item target over the grown rect.
			Some(tex) => out.push(Quad { tex, rect: *q, uv: panel_uv(full[1], k), colour: [1.0; 4], angle: 0.0, corners: None }),
			None => match portrait_tex.filter(|_| i == 2) {
				// BRIDGE-37: panel 2 shows the 512x128 portrait target (same box as panel 1).
				Some(tex) => out.push(Quad { tex, rect: *q, uv: panel_uv(full[2], k), colour: [1.0; 4], angle: 0.0, corners: None }),
				// NOV-30e: every panel target is cleared opaque black (FUN_00222fa0 colour 0) and blitted as a full replace.
				None => overlay::rect(out, *q, [0.0, 0.0, 0.0, 1.0]),
			},
		}
	}
	let line = |out: &mut Vec<Quad>, i: usize, s: &str, x: f32, y: f32, c: [f32; 4]| {
		let (kx, ky) = panel_scale(full[i], r[i], k);
		let from = out.len();
		text_glyphs(out, font, s, r[i][0] + x * kx, r[i][1] + y * ky, kx, ky, c);
		clip_quads(out, from, r[i]);
	};
	// Text width in texels (font 2 glyph advances, no scale).
	let width = |_i: usize, s: &str| crate::help::width(font, s);
	// 0: the LED ticker (UI-5e): 18 spaces + the box text from -counter at y 8, scale 2, columns 0..4 / 0xe2..0xe6 blanked.
	if let Some((slot, sw, sh)) = led {
		let (sx, sy) = panel_scale(full[0], r[0], k);
		let from = out.len();
		let id = shop.ticker.unwrap_or(0);
		let boxed = format!("{}{}", " ".repeat(18), text.get(id));
		for g in led_quads(&boxed, -(shop.ticker_x as i32), 8, 2.0, shop.frame).into_iter().flat_map(led_cut) {
			let rect = [r[0][0] + g.rect[0] * sx, r[0][1] + g.rect[1] * sy, g.rect[2] * sx, g.rect[3] * sy];
			let uv = [g.uv[0] / sw, g.uv[1] / sh, (g.uv[0] + g.uv[2]) / sw, (g.uv[1] + g.uv[3]) / sh];
			out.push(Quad { tex: slot, rect, uv, colour: [0.5, 0.5, 0.5, 1.0], angle: 0.0, corners: None });
		}
		clip_quads(out, from, r[0]);
	}
	// 1 (FUN_002b1838, NOV-30d): name at (6, 8); a weapon row's price right-aligned on (118, 101), struck and grey with the
	// alternate price at (118, 85) above it while `have[35]`; an ammo row's label 0x4f5d at (24, 24) and its unit price.
	// All 0x80f0f0f0; affordability never changes this panel.
	let p = vendor::price(row.item);
	line(out, 1, &name(text, row.item), 6.0, 8.0, white);
	if let Some(p) = &p {
		if row.ammo {
			line(out, 1, text.get(vendor::TEXT_AMMO_LABEL), 24.0, 24.0, white);
			let s = commas(p.ammo);
			line(out, 1, &s, 118.0 - width(1, &s), 101.0, white);
		} else {
			let s = commas(p.item);
			let struck = inv.have[35];
			line(out, 1, &s, 118.0 - width(1, &s), 101.0, if struck { grey } else { white });
			if struck {
				// Seven strike sprites x in [117 - ws + i, 123 - i), y in [109, 112), rgba (0x44, 0x95, 0x95, 0x20 + 0x10 i).
				let ws = strike_width(&s);
				let (kx, ky) = panel_scale(full[1], r[1], k);
				let from = out.len();
				for i in 0..7 {
					let (x0, x1) = (117.0 - ws + i as f32, 123.0 - i as f32);
					let c = [0x44 as f32 / 255.0, 0x95 as f32 / 255.0, 0x95 as f32 / 255.0, (0x20 + 0x10 * i) as f32 / 128.0];
					overlay::rect(out, [r[1][0] + x0 * kx, r[1][1] + 109.0 * ky, (x1 - x0) * kx, 3.0 * ky], c);
				}
				clip_quads(out, from, r[1]);
				let a = commas(p.alt);
				line(out, 1, &a, 118.0 - width(1, &a), 85.0, white);
			}
		}
	}
	// 3, 4: button prompts (panel 2 content is only the portrait actor, no text: FUN_002b1488), NOV-30h. Buy (FUN_002b1d60)
	// always draws 0x5234 as box text centred in the opening-scaled panel; Exit (FUN_002b1fb8) centres 0x4ee0 / 0x524b on
	// x 40 at y 20, or nothing. Both 0x80f0f0f0 through FUN_0021cbf8 (wide button glyph, no shadow).
	let prompt = |out: &mut Vec<Quad>, i: usize, x: i32, y: i32, s: &str| {
		let (kx, ky) = panel_scale(full[i], r[i], k);
		let from = out.len();
		text_glyphs(out, font, s, r[i][0] + x as f32 * kx, r[i][1] + y as f32 * ky, kx, ky, white);
		clip_quads(out, from, r[i]);
	};
	let (bw, bh) = ((r[3][2] / k) as i32, (r[3][3] / k) as i32);
	for (x, y, s) in buy_lines(font, text.get(vendor::TEXT_BUY), bw, bh) {
		prompt(out, 3, x, y, &s);
	}
	if let Some(id) = exit_text(shop, &row, bolts, inv) {
		let s = text.get(id);
		prompt(out, 4, 40 - (s.chars().map(|c| adv(font, c)).sum::<i32>() >> 1), 20, s);
	}
	// 5: item icons, the selected one in a yellow 56x56 frame.
	let (sx, sy) = panel_scale(full[5], r[5], k);
	let from = out.len();
	for (i, rw) in shop.rows.iter().enumerate() {
		let (x, y) = (r[5][0] + (ICON_ORIGIN[0] + i as f32 * ICON_PITCH) * sx, r[5][1] + ICON_ORIGIN[1] * sy);
		let q = [x, y, ICON * sx, ICON * sy];
		if i == shop.sel {
			// UI-5b2 (vendor.md): outer frame 4 texels around the icon, pulsing colour; inner black rect inset 2 from it.
			let (mx, my) = (4.0 * sx, 4.0 * sy);
			let v = frame_level(shop.pulse) as f32 / 255.0;
			overlay::rect(out, [q[0] - mx, q[1] - my, q[2] + 2.0 * mx, q[3] + 2.0 * my], [2.0 * v, 2.0 * v, v, 1.0]);
			let (ix, iy) = (2.0 * sx, 2.0 * sy);
			overlay::rect(out, [q[0] - mx + ix, q[1] - my + iy, q[2] + 2.0 * (mx - ix), q[3] + 2.0 * (my - iy)], [0.0, 0.0, 0.0, 1.0]);
		}
		match icon(rw.item, rw.ammo) {
			Some(slot) => out.push(Quad { tex: slot, rect: q, uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None }),
			None => overlay::rect(out, q, [0.1, 0.1, 0.12, 1.0]),
		}
	}
	clip_quads(out, from, r[5]);
	// UI-5c: per panel, after its content, the noise and sweep overlays (this tick's results from `Shop::screen_tick`).
	if let Some(fx) = fx {
		for i in 0..6 {
			let (sx, sy) = panel_scale(full[i], r[i], k);
			fx_overlays(out, shop, fx, i, [r[i][0], r[i][1]], (sx, sy), (r[i][2] / k, r[i][3] / k), r[i]);
		}
	}
	// UI-5d (vendor.md): the purchase popup, drawn after the panel loop (FUN_002b3370).
	let rect = popup.filter(|_| shop.stage != 0)?;
	let rect = [rect[0], rect[1].min(rect[1] + rect[3]), rect[2], rect[3].abs()];
	if rect[2] < 1.0 || rect[3] < 1.0 {
		return None;
	}
	let (w0, h0) = ((rect[2] / k) as i32, (rect[3] / k) as i32);
	let split = out.len();
	match popup_tex {
		Some(tex) => out.push(Quad { tex, rect, uv: [0.0, 0.0, w0 as f32 / 512.0, h0 as f32 / 128.0], colour: [1.0; 4], angle: 0.0, corners: None }),
		None => overlay::rect(out, rect, [0.0, 0.0, 0.0, 1.0]),
	}
	let from = out.len();
	// Content text in target px mapped by `k` from the rect origin, clipped to it.
	let put = |out: &mut Vec<Quad>, s: &str, x: i32, y: i32| {
		text_glyphs(out, font, s, rect[0] + x as f32 * k, rect[1] + y as f32 * k, k, k, white);
	};
	let wid = |s: &str| s.chars().map(|c| adv(font, c)).sum::<i32>();
	// FUN_0021d678: box text, flags 3 (centred both ways) or 1 (centred horizontally at y 6).
	let boxed = |out: &mut Vec<Quad>, id: u32, flags: u8| {
		let s = text.get(id);
		let chars: Vec<char> = s.chars().collect();
		let wbox = 2 * (w0 - w0 / 2).min(w0 / 2);
		let lines = box_lines(font, s, wbox);
		let mut y = if flags & 2 != 0 { h0 / 2 - ((lines.len() as i32 * 16) >> 1) } else { 6 };
		for (a, e) in lines {
			let line: String = chars[a.min(chars.len())..e.min(chars.len())].iter().collect();
			if y + 16 >= 0 && y <= h0 {
				put(out, &line, w0 / 2 - (wid(&line) >> 1), y);
			}
			y += 16;
		}
	};
	let p = vendor::price(row.item);
	match &p {
		Some(p) if row.ammo => {
			if inv.ammo[row.item as usize] >= p.max {
				boxed(out, 0x5236, 3);
			} else if bolts < p.ammo {
				boxed(out, 0x5238, 3);
			} else {
				boxed(out, 0x5237, 1);
				let q = shop.qty as i32;
				put(out, &q.to_string(), if q < 10 { 0x25 } else if q < 100 { 0x1e } else { 0x17 }, 0x32);
				put(out, "<", 0xc, 0x32);
				put(out, ">", 0x40, 0x32);
				let total = (q as u32 * p.ammo).to_string();
				put(out, &total, 0x40 - wid(&total), 0x4c);
			}
		}
		Some(p) => {
			if p.item_cost(inv.have[35]) <= bolts {
				put(out, text.get(0x5239), 6, 6);
				put(out, text.get(0x524e), 2, 0x20);
				put(out, text.get(0x524b), 2, 0x3e);
			} else {
				boxed(out, 0x5238, 3);
			}
		}
		None => {}
	}
	clip_quads(out, from, rect);
	// UI-5c panel 6: noise, sweep, then the bevel (fx 0x19 over the whole target, texels 0..64).
	if let Some(fx) = fx {
		fx_overlays(out, shop, fx, 6, [rect[0], rect[1]], (k, k), (w0 as f32, h0 as f32), rect);
	}
	Some(split)
}

/// UI-5c overlays of panel `i` (vendor.md UI-5c): `origin` window px of the target's (0, 0), `(sx, sy)` window px per
/// texel, `(wt, ht)` the target size in texels, clipped to `clip`; panel 6 (the popup) adds the bevel last.
fn fx_overlays(out: &mut Vec<Quad>, shop: &Shop, fx: [(usize, f32, f32); 3], i: usize, origin: [f32; 2], (sx, sy): (f32, f32), (wt, ht): (f32, f32), clip: [f32; 4]) {
	let map = |x: f32, y: f32, w: f32, h: f32| [origin[0] + x * sx, origin[1] + y * sy, w * sx, h * sy];
	let from = out.len();
	if let Some(fix) = shop.noise_fix[i] {
		let [ou, ov] = shop.noise_uv[i];
		let (tw, th) = (fx[0].1, fx[0].2);
		out.push(Quad { tex: fx[0].0, rect: map(0.0, 0.0, wt, ht), uv: [ou / tw, ov / th, (2.0 * ou + wt as i32 as f32) / tw, (2.0 * ov + ht as i32 as f32) / th], colour: [1.0, 1.0, 1.0, -(fix as f32) / 128.0], angle: 0.0, corners: None });
	}
	if let Some((a, c)) = shop.sweep_a[i] {
		let (tw, th) = (fx[1].1, fx[1].2);
		out.push(Quad { tex: fx[1].0, rect: map(0.0, -((0x200 - c as i32) as f32) * 0.03125, wt, ht + 16.0), uv: [0.0, 0.0, wt as i32 as f32 / tw, ((ht + 16.0) * 1.5) as i32 as f32 / th], colour: [0.625, 0.625, 0.625, a as f32 / 128.0], angle: 0.0, corners: None });
	}
	if i == 6 {
		let (tw, th) = (fx[2].1, fx[2].2);
		out.push(Quad { tex: fx[2].0, rect: map(0.0, 0.0, wt, ht), uv: [0.0, 0.0, 64.0 / tw, 64.0 / th], colour: [1.0; 4], angle: 0.0, corners: None });
	}
	clip_quads(out, from, clip);
}

#[cfg(test)]
mod tests {
	use super::*;

	fn font10() -> Font {
		(0, vec![art::Glyph { u: 0, v: 0, dy: 0, adv: 10 }; 256])
	}

	// UI-5d test 1: popup corners from the joints.
	#[test]
	fn popup_rect_corners() {
		let j = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 0.5]];
		let r = popup_rect(&j, &|p| Some([p[0] * 100.0, p[2] * 100.0])).unwrap();
		for (a, b) in r.iter().zip([3.0, 4.0, 94.0, 46.0]) {
			assert!((a - b).abs() < 1e-3, "{r:?}");
		}
	}

	// NOV-35a test 1: the attach-point corners (joints 2 / 1 / 4 of the prompt pose) give a real popup rect.
	#[test]
	fn popup_rect_from_attach_points() {
		let j = [[0.4607226, -0.41478366, 1.7793825], [0.4607226, 0.41478366, 1.7793825], [0.4607226, -0.41478366, 0.9498151]];
		let r = popup_rect(&j, &|p| Some([p[1] * 100.0, -p[2] * 100.0])).unwrap();
		for (a, b) in r.iter().zip([-38.478367, -173.93825, 76.956726, 78.95674]) {
			assert!((a - b).abs() < 1e-3, "{r:?}");
		}
	}

	// NOV-35a test 3: the old skeleton joints 0 / 1 / 2 of the same pose give a rect narrower than 1 px (the bug).
	#[test]
	fn popup_rect_from_skeleton_joints_is_degenerate() {
		let j = [[0.4607226, 0.0, 1.3645988], [0.4607226, 0.41478366, 1.7793825], [0.4607226, -0.41478366, 1.7793825]];
		let r = popup_rect(&j, &|p| Some([p[1] * 100.0, -p[2] * 100.0])).unwrap();
		for (a, b) in r.iter().zip([-0.7071068, -141.40962, -1.4142165, -75.885666]) {
			assert!((a - b).abs() < 1e-3, "{r:?}");
		}
		assert!(r[2] < 1.0);
	}

	// UI-5d test 2: greedy split with every advance 10, wbox 60: "AAAA" / "BBBB" (the last line 40 >= 20, no shrink).
	#[test]
	fn box_text_splits_and_balances() {
		assert_eq!(box_lines(&font10(), "AAAA BBBB", 60), vec![(0, 4), (5, 9)]);
		// A short last line narrows the box by 16 until the split changes: "AAAAA B" at 60 -> last line "B" (10 < 20).
		assert_eq!(box_lines(&font10(), "AAAAA B", 70).len(), 2);
	}

	#[test]
	fn frame_pulse_level() {
		// UI-5b2: v = abs((pulse & 0xf) * 4 - 0x20) + 0x40.
		assert_eq!([1, 8, 16].map(frame_level), [92, 64, 96]);
	}

	#[test]
	fn led_walk() {
		// 'P' k 15 -> u 27, v 18, advance 10 -> 20; 'I' advance 4 -> 8; space 18; '1' skipped.
		let g = led_quads("PI 1P", 0, 8, 2.0, 100);
		assert_eq!(g[0].uv, [27.0, 18.0, 9.0, 9.0]);
		assert_eq!(g[1].rect[0], 20.0);
		assert_eq!(g[2].rect[0], 20.0 + 8.0 + 18.0);
		assert_eq!(g.len(), 3);
		// Gate x + 9 > 0 && x < 0x100.
		assert!(led_quads("A", -9, 0, 2.0, 100).is_empty());
		assert_eq!(led_quads("A", -8, 0, 2.0, 100).len(), 1);
		assert_eq!(led_quads("A", 255, 0, 2.0, 100).len(), 1);
		assert!(led_quads("A", 256, 0, 2.0, 100).is_empty());
		// Blink: 'b' arms; frame 40 % 33 = 7 hides the next char, 41 shows it.
		assert!(led_quads("bA", 0, 0, 2.0, 40).is_empty());
		assert_eq!(led_quads("bA", 0, 0, 2.0, 41).len(), 1);
	}

	#[test]
	fn item_panel_uv_box() {
		// REN-9b / NOV-30a: a 128x128 window-px panel at k = 1 is w0 = 110.592 retail px, so the blit shows texels
		// 0..109 (int(w0 - 1)) x 0..117 of the 512x128 target.
		assert_eq!(panel_uv([0.0, 0.0, 128.0, 128.0], 1.0), [0.0, 0.0, 0.212890625, 0.9140625]);
	}

	#[test]
	fn panel_source_box_is_trimmed_by_pal_height() {
		let full = [10.0, 20.0, 128.0, 128.0];
		let (sx, sy) = panel_scale(full, full, 1.0);
		assert_eq!((sx, sy), (128.0 / 127.0, 128.0 / 117.0));
		assert_eq!(20.0 + 101.0 * sy, 20.0 + 101.0 * 128.0 / 117.0);
		let (sx, sy) = panel_scale([0.0, 0.0, 80.0, 60.0], [0.0, 0.0, 80.0, 60.0], 1.0);
		assert_eq!((sx, sy), (80.0 / 79.0, 60.0 / 54.0));
		// Half open: the same source box, so the scale halves; window px are divided by k first.
		let (fx, fy) = panel_scale([0.0, 0.0, 256.0, 256.0], [0.0, 0.0, 256.0, 256.0], 2.0);
		let (hx, hy) = panel_scale([0.0, 0.0, 256.0, 256.0], [0.0, 0.0, 128.0, 128.0], 2.0);
		assert_eq!((fx, fy), (256.0 / 127.0, 256.0 / 117.0));
		assert_eq!((hx, hy), (fx / 2.0, fy / 2.0));
	}

	// UI-5b: content past the source box is cut with its texture coordinates; the strip icons take the panel scale.
	#[test]
	fn panel_content_clips_to_the_source_box() {
		let q = |rect| Quad { tex: 0, rect, uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None };
		let mut out = vec![q([0.0, 0.0, 1.0, 1.0]), q([8.0, 2.0, 4.0, 4.0]), q([20.0, 0.0, 2.0, 2.0])];
		clip_quads(&mut out, 1, [0.0, 0.0, 10.0, 10.0]);
		assert_eq!(out.len(), 2);
		assert_eq!((out[1].rect, out[1].uv), ([8.0, 2.0, 2.0, 4.0], [0.0, 0.0, 0.5, 1.0]));
		// Test 1: a 128 x 128 panel, glyph row at texel y 101.
		let full = [10.0, 20.0, 128.0, 128.0];
		let (_, sy) = panel_scale(full, full, 1.0);
		assert_eq!(full[1] + 101.0 * sy, 20.0 + 101.0 * 128.0 / 117.0);
	}

	// UI-5b: panel 5's icons sit at (12 + 56 i, 6) texels scaled by panel_scale (60 px panel: sy = 60 / 54).
	#[test]
	fn icon_strip_uses_the_panel_scale() {
		let full = [100.0, 400.0, 80.0, 60.0];
		let (sx, sy) = panel_scale(full, full, 1.0);
		assert_eq!((sx, sy), (80.0 / 79.0, 60.0 / 54.0));
		assert!((100.0 + (ICON_ORIGIN[0] + ICON_PITCH) * sx - (100.0 + 68.0 * 80.0 / 79.0)).abs() < 1e-3);
	}

	// NOV-30d test 3: strike widths over table 0x1c3550.
	#[test]
	fn strike_widths() {
		assert_eq!(strike_width("2,500"), 58.0);
		assert_eq!(strike_width("5"), 13.0);
	}

	/// Panel 1 of a 300 x 300 rect layout at k = 1, with text 0x4f5d = "Ammo": (quads, panel-1 rect, texel scale).
	fn panel1(shop: &Shop, inv: &Inventory) -> (Vec<Quad>, [f32; 4], (f32, f32)) {
		let label = b"Ammo";
		let mut file = Vec::new();
		for v in [4u32, 1, (24 + label.len()) as u32, 24, vendor::TEXT_AMMO_LABEL, 0, 0] {
			file.extend_from_slice(&v.to_le_bytes());
		}
		file.extend_from_slice(label);
		let text = Text::parse(&file, 0).unwrap();
		let font = (77, font10().1);
		let rects: [[f32; 4]; 6] = std::array::from_fn(|i| [i as f32 * 400.0, 0.0, 300.0, 300.0]);
		let mut out = Vec::new();
		draw(&mut out, 1200.0, 448.0, &font, &text, shop, 0, inv, Some(rects), &|_, _| None, None, None, None, None, None, None);
		(out, rects[1], panel_scale(rects[1], rects[1], 1.0))
	}

	/// Font glyphs of colour `c` whose top sits on panel-1 texel row `y`, as texel x.
	fn glyphs_at(out: &[Quad], r: [f32; 4], (kx, ky): (f32, f32), y: f32, c: [f32; 4]) -> Vec<f32> {
		out.iter().filter(|q| q.tex == 77 && q.colour == c && (q.rect[1] - (r[1] + y * ky)).abs() < 1e-3 && q.rect[0] >= r[0] && q.rect[0] < r[0] + r[2]).map(|q| (q.rect[0] - r[0]) / kx).collect()
	}

	// NOV-30d test 4: a weapon row's struck price and the alternate price while `have[35]`.
	// NOV-30d test 5: an ammo row shows the unit price and label 0x4f5d, not qty x price or owned/max.
	// NOV-30e test 1: without targets every panel interior is one opaque black fill, never the old dark blue.
	// NOV-30e test 2: panel-1 text is drawn once per glyph (no black shadow), positions unchanged.
	// NOV-30b1: panel 2 (portrait) gets no text; the old title string 0x5314 is supplied but never drawn there.
	/// Font 2 with the NOV-30h advances (dat.py 0x1c38f0) and the decoded cells of 0x10, 'B', 0xa7 and 0xe7.
	fn font2() -> Font {
		let mut g = vec![art::Glyph { u: 0, v: 0, dy: 0, adv: 0 }; 256];
		for (c, a) in [(0x10u8, 18), (0x12, 18), (b' ', 4), (b'B', 12), (b'u', 10), (b'y', 9), (b'K', 11), (b'a', 9), (b'f', 7), (b'e', 9), (b'n', 9), (b'E', 11), (b'x', 10), (b'i', 4), (b't', 6), (b'N', 12), (b'o', 10), (b'A', 10)] {
			g[c as usize].adv = a;
		}
		g[0x10] = art::Glyph { u: 0, v: 96, dy: 0, adv: 18 };
		g[b'B' as usize] = art::Glyph { u: 16, v: 0, dy: 0, adv: 12 };
		g[0xa7] = art::Glyph { u: 112, v: 48, dy: 0, adv: 9 };
		g[0xe7] = art::Glyph { u: 48, v: 112, dy: 0, adv: 1 };
		(5, g)
	}

	// NOV-30h test 1: Buy box text lines (texel x, y).
	#[test]
	fn buy_lines_centre_in_the_panel() {
		let f = font2();
		assert_eq!(buy_lines(&f, "\u{10} Kaufen", 80, 40), vec![(2, 12, "\u{10} Kaufen".to_string())]);
		assert_eq!(buy_lines(&f, "\u{10} Buy", 80, 40), vec![(14, 12, "\u{10} Buy".to_string())]);
		assert_eq!(buy_lines(&f, "\u{10} Buy", 80, 41)[0].1, 12);
	}

	// NOV-30h tests 2 and 3: wide button cell, accent cell, colour codes skipped.
	#[test]
	fn text_glyphs_cells() {
		let f = font2();
		let mut out = Vec::new();
		text_glyphs(&mut out, &f, "\u{10}B", 0.0, 0.0, 1.0, 1.0, [1.0; 4]);
		assert_eq!(out.len(), 2);
		assert_eq!((out[0].rect, out[0].uv), ([0.0, 0.0, 24.0, 16.0], [0.0, 0.75, 0.09375, 0.875]));
		assert_eq!((out[1].rect, out[1].uv), ([18.0, 0.0, 16.0, 16.0], [0.0625, 0.0, 0.125, 0.125]));
		out.clear();
		text_glyphs(&mut out, &f, "\u{a7}", 0.0, 0.0, 1.0, 1.0, [1.0; 4]);
		assert_eq!(out.len(), 2);
		assert_eq!((out[0].rect, out[0].uv), ([1.0, 0.0, 16.0, 16.0], [0.1875, 0.875, 0.25, 1.0]));
		assert_eq!(out[1].uv, [0.4375, 0.375, 0.5, 0.5]);
		out.clear();
		text_glyphs(&mut out, &f, "\u{0c}A", 0.0, 0.0, 1.0, 1.0, [1.0; 4]);
		assert_eq!(out.len(), 1);
		assert_eq!(out[0].rect[0], 0.0);
	}

	// NOV-30h test 4: the Exit panel id and its centring.
}
