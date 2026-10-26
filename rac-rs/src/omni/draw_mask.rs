//! The mods' solid rectangles and bars as small pictures in game pixels, for the sharp HUD (viewer/gpu/draw/pass_omni.rs).
//!
//! A mod builds shapes from rects: the Arsenal Mastery nanotech box cuts its lower right corner with stacked 2-unit rects, which the plain
//! quads draw as a hard staircase of 5-9 window pixels a step at 1080p and up. Retail's own art has the same shape as PS2 pixels, and the
//! sharp HUD smooths it with the edge-aware sampler. So with the sharp HUD on, the solids are drawn like retail art: `raster` paints a run of
//! solids into a game-pixel picture (exact area coverage per pixel, straight alpha "over" in submit order) and the pass draws that picture
//! through the same sampler. Nothing here touches the GPU (pure functions, unit tested).
//!
//! `build` keeps the draw order exactly: consecutive rects / bars form one `Run::Solids`, text and icons (game-art quads) form `Run::Quads`
//! between them. `pieces` splits a solids run into pictures that do not overlap (so each stays small and the order inside a picture is the
//! submit order); a run that is too large or has too many rects falls back to the plain quads of draw_quads.rs.

use super::cabi::OmniDrawCmd;
use super::draw::Frame;
use super::draw_art::Art;
use super::draw_map::Layout;
use super::draw_quads::{self as dq, KIND_BAR, KIND_ICON, KIND_RECT, KIND_TEXT};
use crate::overlay::Quad;

/// A solid rectangle in game pixels with its straight RGBA colour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Solid {
	pub x: f32,
	pub y: f32,
	pub w: f32,
	pub h: f32,
	pub colour: [f32; 4],
}

/// Draw order: solids and game-art quads as the list submitted them.
pub enum Run {
	Solids(Vec<Solid>),
	Quads(Vec<Quad>),
}

/// Transparent texels around a picture, so the sampler never meets a cut edge.
const PAD: i32 = 2;
/// A run with more rects than this is drawn as plain quads.
const MAX_SOLIDS: usize = 512;
/// A picture larger than this (side, area in texels) is drawn as plain quads.
const MAX_SIDE: i32 = 1100;
const MAX_AREA: i64 = 600_000;

/// A picture of solids: `rgba` is `w * h` straight-alpha texels, its top-left texel is game pixel (`x`, `y`).
#[derive(Clone, Debug, PartialEq)]
pub struct Mask {
	pub x: i32,
	pub y: i32,
	pub w: usize,
	pub h: usize,
	pub rgba: Vec<[u8; 4]>,
}

/// One piece of a solids run, in draw order.
#[derive(Clone, Debug, PartialEq)]
pub enum Piece {
	Mask(Mask),
	/// Too large to rasterise: drawn as plain quads.
	Plain(Vec<Solid>),
}

fn solid_of(x: f32, y: f32, w: f32, h: f32, colour: [f32; 4]) -> Option<Solid> {
	// The same rule as draw_quads::solid: invisible or empty draws nothing (NaN too).
	(colour[3] > 0.0 && w > 0.0 && h > 0.0 && x.is_finite() && y.is_finite() && w.is_finite() && h.is_finite()).then_some(Solid { x, y, w, h, colour })
}

/// The list as runs, `hud` layer first (as draw_quads::build).
pub fn build(f: &Frame, l: &Layout, art: Option<&Art>) -> Vec<Run> {
	let mut runs: Vec<Run> = Vec::new();
	let push_solid = |runs: &mut Vec<Run>, s: Option<Solid>| {
		let Some(s) = s else { return };
		match runs.last_mut() {
			Some(Run::Solids(v)) => v.push(s),
			_ => runs.push(Run::Solids(vec![s])),
		}
	};
	let art_quads = |runs: &mut Vec<Run>, c: &OmniDrawCmd| {
		let mut q = Vec::new();
		if c.kind == KIND_TEXT {
			dq::text(&mut q, l, art, f, c);
		} else {
			dq::icon(&mut q, l, art, c);
		}
		if q.is_empty() {
			return;
		}
		match runs.last_mut() {
			Some(Run::Quads(v)) => v.extend(q),
			_ => runs.push(Run::Quads(q)),
		}
	};
	for layer in [0u32, 1] {
		for c in f.cmds.iter().filter(|c| (c.layer != 0) as u32 == layer) {
			match c.kind {
				KIND_TEXT | KIND_ICON => art_quads(&mut runs, c),
				KIND_RECT => push_solid(&mut runs, solid_of(c.x, c.y, c.w, c.h, dq::rgba(c.color))),
				KIND_BAR => {
					push_solid(&mut runs, solid_of(c.x, c.y, c.w, c.h, dq::rgba(c.color2)));
					let frac = if c.value.is_finite() { c.value.clamp(0.0, 1.0) } else { 0.0 };
					push_solid(&mut runs, solid_of(c.x, c.y, c.w * frac, c.h, dq::rgba(c.color)));
				}
				_ => {}
			}
		}
	}
	runs
}

/// The plain quads of solids (the fallback, and the classic look): one stretched block-glyph quad each.
pub fn plain_quads(out: &mut Vec<Quad>, l: &Layout, solids: &[Solid]) {
	for s in solids {
		dq::solid(out, l, s.x, s.y, s.w, s.h, s.colour);
	}
}

/// Integer game-pixel box [x0, y0, x1, y1) of a rect.
fn bbox(s: &Solid) -> [i32; 4] {
	let c = |v: f32| v.clamp(-1.0e6, 1.0e6);
	[c(s.x).floor() as i32, c(s.y).floor() as i32, c(s.x + s.w).ceil() as i32, c(s.y + s.h).ceil() as i32]
}

fn grow(b: [i32; 4], by: i32) -> [i32; 4] {
	[b[0] - by, b[1] - by, b[2] + by, b[3] + by]
}

fn touch(a: [i32; 4], b: [i32; 4]) -> bool {
	a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3]
}

fn union(a: [i32; 4], b: [i32; 4]) -> [i32; 4] {
	[a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]
}

/// Splits a run into pictures. `clip` = the game pixels the window shows [x0, y0, x1, y1) (integers): nothing outside is painted. Solids whose
/// boxes (grown by the padding) touch share a picture, in their submit order; separate boxes are independent (they do not overlap, so their
/// order does not matter). A group that is too big, or a run of more than 512 rects, comes back as `Plain`. Empty groups (all outside the clip)
/// are dropped. The pieces are in the order of their first rect.
pub fn pieces(solids: &[Solid], clip: [i32; 4]) -> Vec<Piece> {
	if solids.len() > MAX_SOLIDS {
		return vec![Piece::Plain(solids.to_vec())];
	}
	// (box of the group, (submit index, rect) of its members)
	let mut groups: Vec<([i32; 4], Vec<(usize, Solid)>)> = Vec::new();
	for (i, s) in solids.iter().enumerate() {
		let b = bbox(s);
		let mut merged = (b, vec![(i, *s)]);
		let mut k = 0;
		while k < groups.len() {
			if touch(grow(groups[k].0, PAD), grow(merged.0, PAD)) {
				let (gb, mut gm) = groups.remove(k);
				merged.0 = union(merged.0, gb);
				gm.append(&mut merged.1);
				merged.1 = gm;
			} else {
				k += 1;
			}
		}
		merged.1.sort_by_key(|m| m.0);
		let first = merged.1[0].0;
		// Keep the groups ordered by their first member.
		let at = groups.iter().position(|g| g.1[0].0 > first).unwrap_or(groups.len());
		groups.insert(at, merged);
	}
	groups
		.into_iter()
		.filter_map(|(b, members)| {
			let members: Vec<Solid> = members.into_iter().map(|m| m.1).collect();
			let [x0, y0, x1, y1] = [(b[0] - PAD).max(clip[0] - PAD), (b[1] - PAD).max(clip[1] - PAD), (b[2] + PAD).min(clip[2] + PAD), (b[3] + PAD).min(clip[3] + PAD)];
			if x1 - PAD <= x0 + PAD || y1 - PAD <= y0 + PAD {
				return None;
			}
			let (w, h) = (x1 - x0, y1 - y0);
			if w > MAX_SIDE || h > MAX_SIDE || w as i64 * h as i64 > MAX_AREA {
				return Some(Piece::Plain(members));
			}
			Some(Piece::Mask(raster(&members, [x0, y0, x1, y1])))
		})
		.collect()
}

/// Length of the overlap of [a0, a1) and [b0, b1).
fn overlap(a0: f32, a1: f32, b0: f32, b1: f32) -> f32 {
	(a1.min(b1) - a0.max(b0)).max(0.0)
}

/// The solids painted into the game pixels `area` (x0, y0, x1, y1): each texel gets the exact covered share of every rect (box coverage),
/// composited in order with straight alpha "over" (the blend of the plain quads), then stored straight.
pub fn raster(solids: &[Solid], area: [i32; 4]) -> Mask {
	let (w, h) = ((area[2] - area[0]).max(0) as usize, (area[3] - area[1]).max(0) as usize);
	// Premultiplied accumulation.
	let mut acc = vec![[0.0f32; 4]; w * h];
	for s in solids {
		let b = bbox(s);
		let (px0, px1) = (b[0].max(area[0]), b[2].min(area[2]));
		let (py0, py1) = (b[1].max(area[1]), b[3].min(area[3]));
		for py in py0..py1 {
			let cy = overlap(py as f32, py as f32 + 1.0, s.y, s.y + s.h);
			if cy <= 0.0 {
				continue;
			}
			let row = (py - area[1]) as usize * w;
			for px in px0..px1 {
				let cx = overlap(px as f32, px as f32 + 1.0, s.x, s.x + s.w);
				let a = s.colour[3] * (cx * cy).min(1.0);
				if a <= 0.0 {
					continue;
				}
				let d = &mut acc[row + (px - area[0]) as usize];
				let keep = 1.0 - a;
				for c in 0..3 {
					d[c] = s.colour[c] * a + d[c] * keep;
				}
				d[3] = a + d[3] * keep;
			}
		}
	}
	let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
	let rgba = acc.iter().map(|p| if p[3] <= 1.0e-6 { [0, 0, 0, 0] } else { [q(p[0] / p[3]), q(p[1] / p[3]), q(p[2] / p[3]), q(p[3])] }).collect();
	Mask { x: area[0], y: area[1], w, h, rgba }
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::omni::draw::Slot;

	fn solid(x: f32, y: f32, w: f32, h: f32, colour: [f32; 4]) -> Solid {
		Solid { x, y, w, h, colour }
	}

	const WHITE: [f32; 4] = [1.0; 4];
	const BIG: [i32; 4] = [-100, -100, 700, 600];

	fn at(m: &Mask, x: i32, y: i32) -> [u8; 4] {
		m.rgba[(y - m.y) as usize * m.w + (x - m.x) as usize]
	}

	fn mask(p: &Piece) -> &Mask {
		match p {
			Piece::Mask(m) => m,
			Piece::Plain(_) => panic!("plain"),
		}
	}

	#[test]
	fn an_aligned_rect_covers_exactly_its_pixels() {
		let m = raster(&[solid(10.0, 20.0, 4.0, 3.0, [1.0, 0.5, 0.0, 1.0])], [8, 18, 16, 26]);
		assert_eq!((m.w, m.h), (8, 8));
		assert_eq!(at(&m, 10, 20), [255, 128, 0, 255]);
		assert_eq!(at(&m, 13, 22), [255, 128, 0, 255]);
		assert_eq!(at(&m, 14, 20)[3], 0);
		assert_eq!(at(&m, 10, 23)[3], 0);
		assert_eq!(at(&m, 9, 20)[3], 0);
	}

	#[test]
	fn a_fractional_edge_gets_its_exact_share() {
		// x 10.25..12.5: texel 10 is covered 0.75, texel 11 fully, texel 12 half; y 5..6 fully.
		let m = raster(&[solid(10.25, 5.0, 2.25, 1.0, WHITE)], [8, 3, 16, 9]);
		assert_eq!(at(&m, 10, 5), [255, 255, 255, 191]);
		assert_eq!(at(&m, 11, 5), [255, 255, 255, 255]);
		assert_eq!(at(&m, 12, 5), [255, 255, 255, 128]);
		assert_eq!(at(&m, 13, 5)[3], 0);
		// The colour of a part-covered texel stays the rect's colour (straight alpha).
		let m = raster(&[solid(0.5, 0.0, 1.0, 1.0, [0.2, 0.4, 0.6, 1.0])], [-1, -1, 3, 2]);
		assert_eq!(at(&m, 0, 0), [51, 102, 153, 128]);
	}

	#[test]
	fn rects_are_composited_over_in_submit_order() {
		let red = [1.0, 0.0, 0.0, 1.0];
		let half_blue = [0.0, 0.0, 1.0, 0.5];
		let a = raster(&[solid(0.0, 0.0, 2.0, 1.0, red), solid(1.0, 0.0, 2.0, 1.0, half_blue)], [-1, -1, 4, 2]);
		assert_eq!(at(&a, 0, 0), [255, 0, 0, 255]);
		// Half blue over opaque red: (0.5, 0, 0.5), alpha 1.
		assert_eq!(at(&a, 1, 0), [128, 0, 128, 255]);
		assert_eq!(at(&a, 2, 0), [0, 0, 255, 128]);
		// The other order gives red on top where they meet.
		let b = raster(&[solid(1.0, 0.0, 2.0, 1.0, half_blue), solid(0.0, 0.0, 2.0, 1.0, red)], [-1, -1, 4, 2]);
		assert_eq!(at(&b, 1, 0), [255, 0, 0, 255]);
	}

	#[test]
	fn the_staircase_stays_a_staircase_in_game_pixels() {
		// The nanotech corner: 2-unit steps, each 2 shorter than the one above. The picture holds them as they are (the sampler smooths).
		let cut = 6;
		let mut v = vec![solid(0.0, 0.0, 20.0, 20.0 - cut as f32, WHITE)];
		for k in (0..cut).step_by(2) {
			v.push(solid(0.0, 20.0 - cut as f32 + k as f32, 20.0 - k as f32 - 2.0, 2.0, WHITE));
		}
		let m = raster(&v, [-2, -2, 22, 22]);
		assert_eq!(at(&m, 17, 14)[3], 255);
		assert_eq!(at(&m, 18, 14)[3], 0);
		assert_eq!(at(&m, 15, 16)[3], 255);
		assert_eq!(at(&m, 16, 16)[3], 0, "the step below is 2 narrower");
		assert_eq!(at(&m, 14, 18)[3], 0);
		assert_eq!(at(&m, 13, 18)[3], 255);
	}

	#[test]
	fn raster_clips_to_the_area() {
		let m = raster(&[solid(-50.0, -50.0, 1000.0, 1000.0, WHITE)], [0, 0, 4, 3]);
		assert_eq!((m.w, m.h), (4, 3));
		assert!(m.rgba.iter().all(|t| *t == [255, 255, 255, 255]));
	}

	#[test]
	fn separate_rects_make_separate_pictures_in_order() {
		let v = [solid(10.0, 10.0, 5.0, 5.0, WHITE), solid(300.0, 300.0, 5.0, 5.0, WHITE), solid(12.0, 12.0, 5.0, 5.0, [1.0, 0.0, 0.0, 1.0])];
		let p = pieces(&v, BIG);
		assert_eq!(p.len(), 2, "the third touches the first");
		let (a, b) = (mask(&p[0]), mask(&p[1]));
		assert!(a.x < 12 && a.w < 30 && b.x > 200, "{} {}", a.x, b.x);
		// Inside the first picture the red rect is on top.
		assert_eq!(at(a, 13, 13), [255, 0, 0, 255]);
		assert_eq!(at(a, 10, 10), [255, 255, 255, 255]);
	}

	#[test]
	fn a_close_neighbour_joins_through_the_padding() {
		// 2 pixels apart: the padded boxes touch, so the sampler can look across the gap in one picture.
		let p = pieces(&[solid(0.0, 0.0, 4.0, 4.0, WHITE), solid(6.0, 0.0, 4.0, 4.0, WHITE)], BIG);
		assert_eq!(p.len(), 1);
		let far = pieces(&[solid(0.0, 0.0, 4.0, 4.0, WHITE), solid(20.0, 0.0, 4.0, 4.0, WHITE)], BIG);
		assert_eq!(far.len(), 2);
	}

	#[test]
	fn a_bridging_rect_merges_two_groups_in_submit_order() {
		let v = [solid(0.0, 0.0, 4.0, 4.0, [1.0, 0.0, 0.0, 1.0]), solid(40.0, 0.0, 4.0, 4.0, [0.0, 1.0, 0.0, 1.0]), solid(0.0, 1.0, 44.0, 2.0, [0.0, 0.0, 1.0, 1.0])];
		let p = pieces(&v, BIG);
		assert_eq!(p.len(), 1);
		let m = mask(&p[0]);
		// The bridge was drawn last: blue on top of both.
		assert_eq!(at(m, 1, 1), [0, 0, 255, 255]);
		assert_eq!(at(m, 41, 2), [0, 0, 255, 255]);
		assert_eq!(at(m, 1, 0), [255, 0, 0, 255]);
		assert_eq!(at(m, 41, 3), [0, 255, 0, 255]);
	}

	#[test]
	fn outside_the_clip_is_dropped_and_huge_runs_fall_back() {
		let off = pieces(&[solid(2000.0, 0.0, 10.0, 10.0, WHITE)], [0, 0, 512, 448]);
		assert!(off.is_empty());
		// A rect larger than the window is cut to the clip, which is small enough.
		let cut = pieces(&[solid(-1.0e5, -1.0e5, 2.0e5, 2.0e5, WHITE)], [0, 0, 512, 448]);
		assert_eq!(mask(&cut[0]).w, 512 + 2 * PAD as usize);
		// A clip beyond the limits: plain quads.
		let huge = pieces(&[solid(0.0, 0.0, 5000.0, 5000.0, WHITE)], [0, 0, 5000, 5000]);
		assert!(matches!(huge[0], Piece::Plain(_)));
		let many: Vec<Solid> = (0..MAX_SOLIDS + 1).map(|i| solid(i as f32 * 3.0, 0.0, 2.0, 2.0, WHITE)).collect();
		assert!(matches!(&pieces(&many, BIG)[..], [Piece::Plain(v)] if v.len() == MAX_SOLIDS + 1));
		assert!(pieces(&[], BIG).is_empty());
	}

	fn frame(cmds: &[OmniDrawCmd], pool: &[u8]) -> std::sync::Arc<Frame> {
		let s = Slot::new();
		s.submit(cmds, pool);
		s.current().expect("frame")
	}

	fn cmd(kind: u32, layer: u32, x: f32, w: f32) -> OmniDrawCmd {
		OmniDrawCmd { kind, layer, x, y: 0.0, w, h: 4.0, color: 0xFFFFFFFF, ..Default::default() }
	}

	#[test]
	fn build_keeps_the_submit_order_and_the_layer_split() {
		let l = Layout::plain(512.0, 448.0 * 0.75);
		let mut bar = cmd(KIND_BAR, 0, 10.0, 20.0);
		(bar.value, bar.color2) = (0.5, 0x000000FF);
		let mut txt = cmd(KIND_TEXT, 0, 0.0, 1.0);
		txt.text_len = 2;
		let list = [cmd(KIND_RECT, 1, 100.0, 4.0), bar, txt, cmd(KIND_RECT, 0, 50.0, 4.0), cmd(99, 0, 0.0, 0.0)];
		let runs = build(&frame(&list, b"AB"), &l, None);
		// hud layer: bar (two solids), text, rect; then the menu layer rect.
		let shape: Vec<(char, usize)> = runs.iter().map(|r| match r { Run::Solids(v) => ('S', v.len()), Run::Quads(v) => ('Q', v.len()) }).collect();
		assert_eq!(shape, vec![('S', 2), ('Q', 2), ('S', 2)], "{shape:?}");
		let Run::Solids(last) = &runs[2] else { panic!() };
		assert_eq!((last[0].x, last[1].x), (50.0, 100.0));
		// An invisible or empty rect adds nothing; consecutive solids with only an empty text between them stay one run.
		let mut z = cmd(KIND_RECT, 0, 0.0, 0.0);
		z.color = 0;
		let runs = build(&frame(&[cmd(KIND_RECT, 0, 1.0, 2.0), z, cmd(KIND_TEXT, 0, 0.0, 1.0), cmd(KIND_RECT, 0, 5.0, 2.0)], &[]), &l, None);
		assert!(matches!(&runs[..], [Run::Solids(v)] if v.len() == 2));
	}

	#[test]
	fn plain_quads_are_the_classic_solids() {
		let l = Layout::plain(512.0, 448.0 * 0.75);
		let mut q = Vec::new();
		plain_quads(&mut q, &l, &[solid(0.0, 0.0, 10.0, 10.0, WHITE), solid(0.0, 0.0, 0.0, 10.0, WHITE)]);
		assert_eq!(q.len(), 1);
		assert_eq!(q[0].tex, crate::overlay::FONT);
	}
}
