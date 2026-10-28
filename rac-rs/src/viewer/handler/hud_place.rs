//! HUD position and scale (Pause -> Options -> Graphics -> Display: HUD position 4:3 / Corners, HUD scale 75 .. 150; retail/pcdisp.rs).
//!
//! In retail mode the HUD and the menus are one picture, the GS capture overlay (a 4:3 picture, `RETAIL_OV_TEX`), drawn centred in
//! the window (edge_fill.rs). To put the HUD in the window's corners the picture is cut into cells (uv space): two TOP CORNER cells,
//! `CELL_W` wide (left from the left edge, right from the right edge) and `CELL_H` high (`shape`), and the rest. The corner cells hold what the
//! gameplay HUD has: the nanotech box (top left) and the bolt / ammo box (top right); they are drawn anchored to their outer corner:
//! with `Corners` that corner is the window's corner, and with a scale the cell grows or shrinks about it. Measured on gameplay
//! pictures (fg1 3_0044, 3_0086, 8_0328, 13_0622 at 1920x1080, 4:3 picture 1440 wide): nanotech box u 0.02 .. 0.30, v 0.02 .. 0.09,
//! bolt box u 0.78 .. 0.96, v 0.02 .. 0.09, the four orbs of the top centre u 0.37 .. 0.62; the message boxes at the bottom span u 0.07 ..
//! 0.93 (a whole box cannot sit in a corner), so there are NO bottom cells: the bottom, the centre column (aim, subtitles, message
//! boxes) and the Pokitaru breath tank (right edge, v 0.25 .. 0.68) stay where retail has them. What a moved cell leaves behind is
//! filled with a stretched copy of its inner boundary column / bottom row (HUD free), so fades and letterbox bars stay full width.
//!
//! The TOP HELP PROMPTS ("T Activate Gadgetron Vendor", "T Enter ship", the Hoven teleporter's, one or two lines, any language) are one
//! bar centred at x 256 that is wider than the cells' gap (u 0.21 .. 0.79 for the vendor's): a cell reaching into its rows would carry its
//! ends to the corners. The bar sits right under the boxes (PAL: box bottom v 0.0926 = row 41.5 of 448, bar sprite top row 41.5, its first
//! visible pixels v 0.0991; USA (NTSC): everything about 12 rows lower, visible from row 56.47), so no fixed height serves both discs: the
//! capture measures every picture (`retail::HudGuard`, `gscap::edge_cut::guard`; it travels with the picture, `Gpu::retail_ov_guard`) and the
//! cells end one row inside the bar's transparent top margin (guard.rs `PROMPT_MARGIN_ROWS`: PAL row 43.4, USA 55.4) of the highest CENTRED widget (`shape`). Without one they are `CELL_H` high. The Gadgetron title letters
//! (shop exit) start above the boxes' bottom: then no cell height works and the picture keeps its 4:3 place until they are gone. A widget
//! of the CORNER that reaches across the cell's inner edge (the hoverboard race's progress bar and position panel, top left) is carried whole:
//! the cell is widened to its end and made as tall as it. `hud_hold.rs` keeps a prompt's guard for a few pictures after its bar is gone
//! (the text fades later than the bar: proven, without the hold the last 2 fields of the USA vendor prompt tear) and shows the 4:3 fallback
//! only when two pictures in a row ask for it, except at once when the highest centred widget is a prompt bar (it fades in over several
//! fields: a debounced first picture was drawn torn); both work in live gameplay only (`live`: a menu's or shop's guard is never held).
//!
//! Only gameplay uses it (`live`, `frame_render.rs`: game mode 0, not paused, no shop, no menu): pause menu, shops, cutscenes and the front
//! end keep the 4:3 picture as retail has it. A window narrower than 4:3 keeps it too. The two cells never overlap each other: the
//! scale is cut where they would touch (`Geo::scale`: 147 % at every width with the 4:3 position, growing with the window only with Corners). Known limits: a scale above 100 draws a
//! fade that crosses a cell's inner edge twice in the overlap strip; the HUD of a mod (`map_rect`) moves with the cell it is in; OPEN (S3): a widened
//! cell (race bar and position panel: both cells 0.88 of the width) lowers that cut while the bar shows, so above ~113 % (4:3 position,
//! also Corners in a 4:3 window) or ~136 % (Corners at 16:10) the whole HUD shrinks for the duration of the race and jumps back when the bar
//! goes; the shrink also lasts the 8-picture hold after the bar. Corners at 16:9 or wider never shrinks (the cut is 150.8 %, above the
//! maximum 150). Giant Clank's bar is the same case [I] (not measured). Clipping only the widened cell would cut the bar's right end at high
//! scale: a design call, not done.
use super::super::{Gpu, RETAIL_OV_TEX};
use crate::overlay::Quad;

/// Width of a corner cell as a fraction of the 4:3 picture's width (left cell from the left edge, right cell from the right edge).
/// `retail::HudGuard`'s capture looks for pieces that cross this edge (`gscap::edge_cut::guard::GUARD_COL`; a test keeps them equal).
pub const CELL_W: f32 = 0.34;
/// The tallest the overlay corner cells get (fraction of the picture's height): with nothing across a cell's inner edge they are this
/// high (the nanotech / ammo and bolt boxes end well above). The capture's `CELL_ROWS` is the same number (test).
pub const CELL_H: f32 = 0.18;
/// Bottom of the nanotech / bolt / ammo boxes (fraction of the picture's height, measured on the overlay dumps: PAL box y 10 -> 41.5 rows,
/// USA (NTSC) box content to row 52.3): a piece that crosses a cell's inner edge above this row cannot be kept out of the boxes by a
/// shorter cell.
pub const BOX_BOTTOM_PAL: f32 = 0.0926;
/// The USA (NTSC) disc's `BOX_BOTTOM_PAL`.
pub const BOX_BOTTOM_USA: f32 = 0.1168;
/// A corner widget's end gets this much room (3 px of the 512 picture: the overlay is sampled linearly, the stretch beside the cell repeats the
/// cell's last column, which must be clear of the widget so no streak of its edge crosses the gap); its rows get two rows of 448 (one row is
/// not enough: at HUD scale below 100 % on a 448-row overlay the linear sampling blends half of the position panel's last row in under the cell).
const REACH_PAD_U: f32 = 3.0 / 512.0;
const REACH_PAD_V: f32 = 2.0 / 448.0;
/// The cells together may take at most this much of the picture's width (a gap stays between them).
const MAX_CELLS_W: f32 = 0.9;
/// Height of the band in which a MOD's draw (`map_rect`) moves with its corner: a mod's bar sits under the retail box (Arsenal Mastery's
/// level bar: v ~0.09 .. 0.105), which can be below the overlay cell when a prompt shortens it.
pub const MOD_BAND_H: f32 = 0.18;

/// The shape of the two top corner cells for one picture: both `h` high (fraction of the picture's height), `lw` / `rw` wide (fractions of
/// its width, from the left / right edge).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Shape {
	pub h: f32,
	pub lw: f32,
	pub rw: f32,
}

/// Whether the cells can end at `row` (a fraction of the picture's height) and still hold the boxes.
pub fn fits_above_boxes(row: f32, usa: bool) -> bool {
	row >= if usa { BOX_BOTTOM_USA } else { BOX_BOTTOM_PAL } - 0.0005
}

/// The cells for a picture with this guard (`retail::HudGuard`): `CELL_H` high and `CELL_W` wide without one; down to the first visible row
/// of the highest centred widget (a top help prompt: never inside a cell's rows, drawn in place); widened and made taller to carry a
/// corner widget whole. `None` (the picture keeps its 4:3 place) when a centred widget starts above the boxes' bottom (the Gadgetron title
/// letters over the ammo box), when a corner widget reaches into the rows a centred one needs, or when the cells would meet.
pub fn shape(g: &crate::retail::HudGuard, usa: bool) -> Option<Shape> {
	let mut h = CELL_H;
	if let Some(c) = g.centre {
		if !fits_above_boxes(c, usa) {
			return None;
		}
		h = c.min(CELL_H);
	}
	let (mut lw, mut rw) = (CELL_W, CELL_W);
	let mut need = 0.0f32;
	if let Some(r) = g.left {
		lw = lw.max(r.u + REACH_PAD_U);
		need = need.max(r.bottom + REACH_PAD_V);
	}
	if let Some(r) = g.right {
		rw = rw.max(1.0 - r.u + REACH_PAD_U);
		need = need.max(r.bottom + REACH_PAD_V);
	}
	if need > h {
		if g.centre.is_some_and(|c| need > c) {
			return None;
		}
		h = need;
	}
	(lw + rw <= MAX_CELLS_W).then_some(Shape { h, lw, rw })
}

/// The settings: `corners` = HUD position Corners, `scale` = HUD scale as a factor (1.0 = 100 %).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Place {
	pub corners: bool,
	pub scale: f32,
	/// Height and widths of the top corner cells (`shape`).
	pub h: f32,
	pub lw: f32,
	pub rw: f32,
}

impl Place {
	/// From the Display rows (pcdisp.rs): the HUD position and the scale in percent.
	pub fn new(corners: bool, scale_pct: u32) -> Place {
		Place { corners, scale: scale_pct as f32 / 100.0, h: CELL_H, lw: CELL_W, rw: CELL_W }
	}

	/// The same with the cells of `shape`.
	pub fn with_shape(self, s: Shape) -> Place {
		Place { h: s.h, lw: s.lw, rw: s.rw, ..self }
	}

	/// Whether the picture needs the cell layout at all (the defaults keep today's picture, edge_fill.rs).
	pub fn active(&self) -> bool {
		self.corners || (self.scale - 1.0).abs() > 1e-4
	}
}

/// Window geometry of the 4:3 picture: `sw` x `sh` window, `x0` the left margin, `pw` the picture's width.
#[derive(Clone, Copy)]
struct Geo {
	sw: f32,
	sh: f32,
	x0: f32,
	pw: f32,
}

impl Geo {
	fn new(sw: f32, sh: f32) -> Option<Geo> {
		let pw = sh * 4.0 / 3.0;
		let x0 = (sw - pw) * 0.5;
		(sh > 0.0 && x0 >= -0.5).then_some(Geo { sw, sh, x0: x0.max(0.0), pw })
	}

	/// The scale the cells really get: the HUD scale row's, but never more than lets the two top cells touch. With Corners they sit in
	/// the window's corners (together at most `sw` wide), else they grow about the 4:3 picture's top corners (at most `pw`).
	fn scale(&self, p: Place) -> f32 {
		p.scale.min(if p.corners { self.sw } else { self.pw } / ((p.lw + p.rw) * self.pw))
	}
}

/// One top corner cell: which side, its uv rectangle and its footprint in the 4:3 picture (old) and where it is drawn (dest).
struct Cell {
	left: bool,
	/// u0, v0, u1, v1 of the cell in the overlay.
	uv: [f32; 4],
	/// The footprint of the cell in the 4:3 picture as drawn today (window pixels): x, y, w, h.
	old: [f32; 4],
	/// Where the cell is drawn now: x, y, w, h.
	dest: [f32; 4],
}

fn cells(g: Geo, p: Place) -> [Cell; 2] {
	let scale = g.scale(p);
	let mk = |left: bool| {
		let (u0, u1) = if left { (0.0, p.lw) } else { (1.0 - p.rw, 1.0) };
		let old = [g.x0 + u0 * g.pw, 0.0, (u1 - u0) * g.pw, p.h * g.sh];
		// The outer corner of the cell in the picture and where it goes (the top edge never moves).
		let ax_old = if left { g.x0 } else { g.x0 + g.pw };
		let ax_new = if !p.corners { ax_old } else if left { 0.0 } else { g.sw };
		let (w, h) = (old[2] * scale, old[3] * scale);
		let x = if left { ax_new } else { ax_new - w };
		Cell { left, uv: [u0, 0.0, u1, p.h], old, dest: [x, 0.0, w, h] }
	};
	[mk(true), mk(false)]
}

/// The v ranges (0..1) from `from` down where the side margin of a cut-flag `flag` (2 left, 1 right) is drawn: the rows the capture did
/// not mark as cut (`cut` = `retail::edge_cut_rows`, one entry per overlay row), or one run without marks.
fn margin_runs(cut: Option<&[u8]>, flag: u8, from: f32) -> Vec<(f32, f32)> {
	let Some(cut) = cut.filter(|c| !c.is_empty()) else { return vec![(from, 1.0)] };
	let n = cut.len();
	let cutrow = |r: usize| cut.get(r).is_some_and(|b| b & flag != 0);
	let (mut r, mut out) = ((from * n as f32).round() as usize, Vec::new());
	while r < n {
		if cutrow(r) {
			r += 1;
			continue;
		}
		let s = r;
		while r < n && !cutrow(r) {
			r += 1;
		}
		out.push((s as f32 / n as f32, r as f32 / n as f32));
	}
	out
}

/// Overlay quads for a window `sw` x `sh` whose overlay texture is `tw` x `th` texels, or None when the window is narrower than 4:3
/// (the caller keeps `edge_fill::quads`). `cut`: the rows where a HUD piece is cut at the picture's edge (`retail::edge_cut_rows`),
/// used for the side margins (rows below the cells with Corners, all rows with 4:3).
pub fn quads(tex: usize, sw: f32, sh: f32, tw: u32, th: u32, place: Place, cut: Option<&[u8]>) -> Option<Vec<Quad>> {
	let g = Geo::new(sw, sh)?;
	let q = |rect: [f32; 4], uv: [f32; 4]| Quad { tex, rect, uv, colour: [1.0; 4], angle: 0.0, corners: None };
	let (htx, hty) = (0.5 / tw.max(1) as f32, 0.5 / th.max(1) as f32);
	let mut out = vec![q([g.x0 + place.lw * g.pw, 0.0, (1.0 - place.lw - place.rw) * g.pw, g.sh], [place.lw, 0.0, 1.0 - place.rw, 1.0])];
	for c in cells(g, place) {
		let [u0, _, u1, v1] = c.uv;
		let [ox, oy, ow, oh] = c.old;
		let [dx, dy, dw, dh] = c.dest;
		// Below the cell: the rest of the picture's side column stays.
		out.push(q([ox, oh, ow, g.sh - oh], [u0, v1, u1, 1.0]));
		// Beside the cell: from its inner edge to the old inner edge, over the cell's old rows (a stretch of the inner column).
		let inner_u = if c.left { u1 - htx } else { u0 + htx };
		let (new_in, old_in) = if c.left { (dx + dw, ox + ow) } else { (ox, dx) };
		if new_in < old_in - 0.01 {
			out.push(q([new_in, oy, old_in - new_in, oh], [inner_u, 0.0, inner_u, v1]));
		}
		// Under the cell inside its old rows (the cell is smaller than its footprint): a stretch of its bottom row.
		if oy + oh - (dy + dh) > 0.01 {
			out.push(q([dx, dy + dh, dw, oy + oh - (dy + dh)], [u0, v1 - hty, u1, v1 - hty]));
		}
		// The side margin outside the picture: the stretched outer column (rows below the cell with Corners: the cell covers the rest).
		if g.x0 > 0.5 {
			let outer_u = if c.left { htx } else { 1.0 - htx };
			let flag = if c.left { 2u8 } else { 1u8 };
			let mx = if c.left { 0.0 } else { g.x0 + g.pw };
			for (a, b) in margin_runs(cut, flag, if place.corners { place.h } else { 0.0 }) {
				out.push(q([mx, a * g.sh, g.x0, (b - a) * g.sh], [outer_u, a, outer_u, b]));
			}
		}
		out.push(q(c.dest, c.uv));
	}
	Some(out)
}

/// Whether the picture on screen is live gameplay (game mode 0, not paused, no shop, no menu): the only place the cells are used here, and
/// the only place the guard's hold and 4:3 debounce (hud_hold.rs, `Gpu::set_retail_overlay`) work.
pub(super) fn live(r: &crate::retail::Retail) -> bool {
	r.gameplay() && !r.paused() && r.shop().is_none() && !r.menu_presents()
}

/// The overlay quads of a frame (frame_render.rs): HUD position / scale of the Display rows in gameplay (game mode 0, not paused, no
/// shop, no menu) on a window at least 4:3 wide, else the picture as retail draws it (edge_fill.rs). Sets `g.hud_place` for the mods'
/// HUD (pass_omni.rs) and tells pcdisp.rs the window size (the Render scale row shows it). `k` = window aspect / (4/3).
pub(super) fn overlay_quads(g: &mut Gpu, r: &crate::retail::Retail, k: f32) -> Vec<Quad> {
	let (sw, sh) = (g.config.width as f32, g.config.height as f32);
	let (ovw, ovh) = g.retail_ov.as_ref().map_or((0, 0), |o| (o.2, o.3));
	let win = g.win_size(); // the window, not the render size (rscale.rs): the Render scale row shows window x scale
	crate::retail::pcdisp::note_window_size(win.0, win.1);
	let place = Place::new(crate::retail::pcdisp::hud_corners(), crate::retail::pcdisp::hud_scale_pct());
	// The cells end above the highest top help prompt of the picture on screen (its own guard, `Gpu::retail_ov_guard`) and carry the corner's
	// widgets whole; where that is not possible the picture keeps its 4:3 place.
	let place = shape(&g.retail_ov_guard.guard(), crate::retail::disc::usa()).map(|s| place.with_shape(s));
	if let Some(place) = place.filter(|p| p.active() && live(r)) {
		let cut = crate::retail::edge_cut_rows();
		if let Some(qs) = quads(RETAIL_OV_TEX, sw, sh, ovw, ovh, place, cut.as_deref()) {
			g.hud_place = Some(place);
			return qs;
		}
	}
	super::edge_fill::quads(RETAIL_OV_TEX, sw, sh, k, ovh)
}

/// The window rectangle `r` (x, y, w, h; a mod's draw mapped into the 4:3 picture as today) moved and scaled with the top corner cell its
/// centre is in; every other rectangle is returned as it is. Same geometry as `quads`.
pub fn map_rect(r: [f32; 4], sw: f32, sh: f32, place: Place) -> [f32; 4] {
	let Some(g) = Geo::new(sw, sh) else { return r };
	if !place.active() {
		return r;
	}
	let (cx, cy) = (r[0] + r[2] * 0.5, r[1] + r[3] * 0.5);
	let u = (cx - g.x0) / g.pw;
	let v = cy / g.sh;
	let left = u < place.lw;
	if !(left || u > 1.0 - place.rw) || !(0.0..MOD_BAND_H.max(place.h)).contains(&v) || !(-0.5..=1.5).contains(&u) {
		return r;
	}
	let ax_old = if left { g.x0 } else { g.x0 + g.pw };
	let ax_new = if !place.corners { ax_old } else if left { 0.0 } else { g.sw };
	let k = g.scale(place);
	[ax_new + (r[0] - ax_old) * k, r[1] * k, r[2] * k, r[3] * k]
}

#[cfg(test)]
mod tests {
	use super::*;

	fn covered(qs: &[Quad], x: f32, y: f32) -> usize {
		qs.iter().filter(|q| x >= q.rect[0] && x < q.rect[0] + q.rect[2] && y >= q.rect[1] && y < q.rect[1] + q.rect[3]).count()
	}

	/// Every window pixel is covered; at a scale of 100 or less no pixel is drawn twice.
	#[test]
	fn every_pixel_is_covered_and_not_doubled_up_to_100() {
		for (sw, sh) in [(1920.0, 1080.0), (3440.0, 1440.0), (1440.0, 1080.0), (1600.0, 1000.0)] {
			for corners in [false, true] {
				for pct in [75u32, 90, 100, 125, 150] {
					let place = Place::new(corners, pct);
					let qs = quads(7, sw, sh, 512, 448, place, None).unwrap();
					for iy in 0..90 {
						for ix in 0..160 {
							let (x, y) = ((ix as f32 + 0.5) / 160.0 * sw, (iy as f32 + 0.5) / 90.0 * sh);
							let n = covered(&qs, x, y);
							assert!(n >= 1, "{sw}x{sh} corners {corners} {pct}%: ({x},{y}) not covered");
							if pct <= 100 {
								assert_eq!(n, 1, "{sw}x{sh} corners {corners} {pct}%: ({x},{y}) drawn {n} times");
							}
						}
					}
				}
			}
		}
	}

	/// The two top cells never overlap each other, at any aspect ratio, position and scale (the scale is cut where they would touch).
	#[test]
	fn the_two_cells_never_overlap() {
		for (sw, sh) in [(1440.0, 1080.0), (1600.0, 1000.0), (1920.0, 1200.0), (1920.0, 1080.0), (2560.0, 1080.0), (3440.0, 1440.0), (5120.0, 1440.0)] {
			let g = Geo::new(sw, sh).unwrap();
			for corners in [false, true] {
				for pct in [75u32, 100, 125, 150] {
					let [l, r] = cells(g, Place::new(corners, pct));
					assert!(l.dest[0] + l.dest[2] <= r.dest[0] + 0.01, "{sw}x{sh} corners {corners} {pct}%: {:?} {:?}", l.dest, r.dest);
					let k = g.scale(Place::new(corners, pct));
					assert!(k <= pct as f32 / 100.0 && k >= 1.0f32.min(pct as f32 / 100.0));
				}
			}
		}
		// A 4:3 window at 150 % is the case that overlapped (0.51 + 0.51 of the picture's width): cut to 1 / (2 * CELL_W).
		let g = Geo::new(1440.0, 1080.0).unwrap();
		assert!((g.scale(Place::new(true, 150)) - 1.0 / (2.0 * CELL_W)).abs() < 1e-4);
		assert_eq!(g.scale(Place::new(true, 125)), 1.25, "below the limit nothing changes");
		let wide = Geo::new(3440.0, 1440.0).unwrap();
		assert_eq!(wide.scale(Place::new(true, 150)), 1.5);
		// Mod draws follow the same cut scale.
		let tl = [g.x0 + 40.0, 30.0, 100.0, 50.0];
		assert!((map_rect(tl, 1440.0, 1080.0, Place::new(true, 150))[2] - 100.0 / (2.0 * CELL_W)).abs() < 1e-3);
	}

	#[test]
	fn corners_go_to_the_window_corners() {
		let (sw, sh) = (3440.0, 1440.0);
		let g = Geo::new(sw, sh).unwrap();
		let cs = cells(g, Place::new(true, 100));
		assert_eq!(cs[0].dest[..2], [0.0, 0.0]);
		assert!((cs[1].dest[0] + cs[1].dest[2] - sw).abs() < 0.01 && cs[1].dest[1] == 0.0);
		assert!((cs[0].dest[2] - CELL_W * g.pw).abs() < 0.01, "unscaled: same size as in the picture");
		let small = cells(g, Place::new(true, 75));
		assert!((small[0].dest[2] - 0.75 * CELL_W * g.pw).abs() < 0.01 && (small[1].dest[3] - 0.75 * CELL_H * sh).abs() < 0.01);
	}

	#[test]
	fn four_three_keeps_the_cells_in_the_picture_and_scales_about_the_top_corners() {
		let (sw, sh) = (1920.0, 1080.0);
		let g = Geo::new(sw, sh).unwrap();
		let cs = cells(g, Place::new(false, 150));
		assert!((cs[0].dest[0] - g.x0).abs() < 0.01 && cs[0].dest[1] == 0.0);
		assert!((cs[1].dest[0] + cs[1].dest[2] - (g.x0 + g.pw)).abs() < 0.01 && cs[1].dest[1] == 0.0);
		assert!(!Place::new(false, 100).active() && Place::new(true, 100).active() && Place::new(false, 105).active());
	}

	#[test]
	fn a_window_narrower_than_four_three_keeps_the_old_path() {
		assert!(quads(7, 1000.0, 1080.0, 512, 448, Place::new(true, 100), None).is_none());
		assert!(quads(7, 1440.0, 1080.0, 512, 448, Place::new(true, 100), None).is_some());
	}

	#[test]
	fn cut_rows_get_no_margin() {
		let mut cut = [0u8; 448];
		cut[0..224].fill(1); // the right edge is cut on the top half
		for corners in [false, true] {
			let qs = quads(7, 1920.0, 1080.0, 512, 448, Place::new(corners, 90), Some(&cut)).unwrap();
			let right_margin: Vec<_> = qs.iter().filter(|q| q.uv[0] == q.uv[2] && q.uv[0] > 0.9 && q.rect[0] >= 1680.0 - 0.5).collect();
			assert_eq!(right_margin.len(), 1, "corners {corners}: only the bottom half has a right margin");
			assert!(right_margin[0].rect[1] >= 540.0 - 1.0);
		}
	}

	fn inside(r: [f32; 4], x: f32, y: f32) -> bool {
		x >= r[0] && x < r[0] + r[2] && y >= r[1] && y < r[1] + r[3]
	}

	/// Whether the quad drawn LAST over the window point (x, y) (the one that is visible there) draws the picture's own pixel (uv = the
	/// point's place in the 4:3 picture).
	fn drawn_in_place(qs: &[Quad], g: Geo, x: f32, y: f32) -> bool {
		let (eu, ev) = ((x - g.x0) / g.pw, y / g.sh);
		let Some(q) = qs.iter().rev().find(|q| inside(q.rect, x, y)) else { return false };
		let (fx, fy) = ((x - q.rect[0]) / q.rect[2], (y - q.rect[1]) / q.rect[3]);
		let (u, v) = (q.uv[0] + (q.uv[2] - q.uv[0]) * fx, q.uv[1] + (q.uv[3] - q.uv[1]) * fy);
		(u - eu).abs() < 1e-3 && (v - ev).abs() < 1e-3
	}

	/// A guard with one centred widget whose first visible row is `row` (of 448).
	fn centred(row: f32) -> crate::retail::HudGuard {
		crate::retail::HudGuard { centre: Some(row / 448.0), ..Default::default() }
	}

	/// The guard of the hoverboard race: the progress bar (19.5, 14.5)-(275.5, 78.5) and the position panel (rows to 86.5), top left.
	fn race() -> crate::retail::HudGuard {
		let left = crate::retail::HudReach { u: 275.5 / 512.0, top: 14.5 / 448.0, bottom: 86.5 / 448.0 };
		crate::retail::HudGuard { left: Some(left), ..Default::default() }
	}

	/// The top help prompts ("T Activate Gadgetron Vendor", "T Enter ship") are drawn at v 0.0991 .. 0.1574 on the PAL disc and at 0.1260 ..
	/// 0.1891 on the USA disc (first visible row 44.4 / 56.47 of 448, measured; the cells end at 43.4 / 55.4), their box spans u 0.21 .. 0.79, across both cells'
	/// columns: no cell may reach them, so they are drawn intact in place with Corners and at every scale (the cells were 0.18 high and tore
	/// the vendor prompt in two: its ends went to the corners). Where the scaled-up corner cell itself lies (HUD scale above 100: the
	/// enlarged boxes are drawn over the picture) nothing is asserted; at 100 % and below no prompt pixel is under a cell.
	#[test]
	fn top_help_prompts_stay_intact_in_their_place() {
		for (usa, top, rows) in [(false, 43.4, [0.0995, 0.11, 0.13, 0.15, 0.1570]), (true, 55.4, [0.1265, 0.15, 0.17, 0.18, 0.1885])] {
			let cell_shape = shape(&centred(top), usa).unwrap();
			for (sw, sh) in [(1440.0, 1080.0), (1920.0, 1080.0), (2560.0, 1080.0), (3440.0, 1440.0), (5120.0, 1440.0)] {
				let g = Geo::new(sw, sh).unwrap();
				for corners in [false, true] {
					for pct in [75u32, 90, 100, 125, 150] {
						let place = Place::new(corners, pct).with_shape(cell_shape);
						let qs = quads(7, sw, sh, 1440, 1080, place, None).unwrap();
						let cs = cells(g, place);
						for v in rows {
							for iu in 0..=48 {
								let u = 0.02 + 0.96 * iu as f32 / 48.0;
								let (x, y) = (g.x0 + u * g.pw, v * sh);
								let under_cell = cs.iter().any(|c| inside(c.dest, x, y));
								assert!(!under_cell || pct > 100, "usa {usa} {sw}x{sh} corners {corners} {pct}%: prompt pixel u {u:.3} v {v} is under a cell");
								assert!(
									under_cell || drawn_in_place(&qs, g, x, y),
									"usa {usa} {sw}x{sh} corners {corners} {pct}%: prompt pixel u {u:.3} v {v} is not drawn in place"
								);
							}
						}
					}
				}
			}
		}
	}

	/// The cells follow what the picture holds: `CELL_H` x `CELL_W` without a guard, whole rows above a top help prompt, widened and taller
	/// for the race bar, and no cells (the 4:3 place) where no shape can keep the pieces whole.
	#[test]
	fn the_cells_follow_the_guard() {
		use crate::retail::{HudGuard, HudReach};
		let plain = Shape { h: CELL_H, lw: CELL_W, rw: CELL_W };
		assert_eq!(shape(&HudGuard::default(), false), Some(plain));
		assert_eq!(shape(&HudGuard::default(), true), Some(plain));
		// PAL vendor prompt: sprite top 41.5 + 1.9 = row 43.4 of 448 (first visible pixel 44.4) -> the cells end there, under the boxes (row 41.5).
		let p = shape(&centred(43.4), false).unwrap();
		assert!((p.h - 0.0969).abs() < 0.0003 && p.h > BOX_BOTTOM_PAL && (p.lw, p.rw) == (CELL_W, CELL_W), "{p:?}");
		// USA vendor prompt: cut at row 55.4 (first visible pixel 56.47), box content to 52.3 (v 0.1168).
		let u = shape(&centred(55.4), true).unwrap();
		assert!((u.h * 448.0 - 55.4).abs() < 1e-3 && u.h >= BOX_BOTTOM_USA, "{u:?}");
		// A prompt far below keeps the old height.
		assert_eq!(shape(&centred(0.3 * 448.0), false), Some(plain));
		// Over the boxes (Gadgetron title letters v 0.07): no cells.
		assert_eq!(shape(&centred(0.07 * 448.0), false), None);
		assert_eq!(shape(&centred(0.09 * 448.0), true), None);
		// The race bar and panel: the left cell reaches the bar's end (+3 px) and the panel's last row (+2 rows); the right one stays.
		let r = shape(&race(), false).unwrap();
		assert!((r.lw - (275.5 + 3.0) / 512.0).abs() < 1e-5 && r.rw == CELL_W, "{r:?}");
		assert!((r.h - 88.5 / 448.0).abs() < 1e-5 && r.h > CELL_H, "{r:?}");
		assert_eq!(shape(&race(), true).map(|s| s.h), Some(r.h), "the same on the USA disc");
		// A race widget with a prompt far below still widens; a prompt that starts inside the widget's rows: no cells.
		assert!(shape(&HudGuard { centre: Some(0.3), ..race() }, false).is_some_and(|s| s.lw == r.lw && s.h == r.h));
		assert_eq!(shape(&HudGuard { centre: Some(43.4 / 448.0), ..race() }, false), None);
		// A narrow corner widget inside the old cell changes nothing; two wide ones would make the cells meet.
		let small = HudReach { u: 0.2, top: 0.03, bottom: 0.1 };
		assert_eq!(shape(&HudGuard { left: Some(small), right: Some(HudReach { u: 0.8, ..small }), ..Default::default() }, false), Some(plain));
		let wide = HudGuard { left: Some(HudReach { u: 0.6, ..small }), right: Some(HudReach { u: 0.4, ..small }), ..Default::default() };
		assert_eq!(shape(&wide, false), None);
		assert!(fits_above_boxes(0.0991, false) && !fits_above_boxes(0.0991, true) && fits_above_boxes(0.1259, true));
		assert!(MOD_BAND_H >= 0.105 && MOD_BAND_H >= CELL_H, "the mods' band covers the level bar under the ammo box");
		assert_eq!((CELL_W, CELL_H), crate::retail::gscap_guard_cells(), "the capture looks across the same inner edge and rows");
	}

	/// The race bar (top left, to u 0.54) sits whole in the widened left cell, which is drawn in the window's corner with Corners; the right
	/// cell and the middle are unchanged, and a mod's draw follows the widened cell.
	#[test]
	fn the_race_bar_moves_whole_with_the_widened_left_cell() {
		let (sw, sh) = (2560.0, 1080.0);
		let g = Geo::new(sw, sh).unwrap();
		let s = shape(&race(), false).unwrap();
		let place = Place::new(true, 100).with_shape(s);
		let qs = quads(7, sw, sh, 512, 448, place, None).unwrap();
		let left = qs.iter().find(|q| q.uv == [0.0, 0.0, s.lw, s.h]).expect("the widened left cell");
		assert_eq!(left.rect[..2], [0.0, 0.0], "in the window's corner");
		assert!((left.rect[2] - s.lw * g.pw).abs() < 0.01 && (left.rect[3] - s.h * sh).abs() < 0.01, "unscaled: the same size as in the picture");
		let right = qs.iter().find(|q| q.uv[2] == 1.0 && q.uv[0] > 0.5 && q.uv[1] == 0.0 && q.uv[3] == s.h).expect("the right cell");
		assert!((right.uv[0] - (1.0 - CELL_W)).abs() < 1e-6);
		let middle = &qs[0];
		assert!((middle.uv[0] - s.lw).abs() < 1e-6 && (middle.uv[2] - (1.0 - CELL_W)).abs() < 1e-6, "{:?}", middle.uv);
		// A mod's draw at u 0.45 (inside the widened cell, beyond CELL_W) moves with it; at u 0.60 it stays.
		let at = |u: f32| [g.x0 + u * g.pw, 30.0, 20.0, 20.0];
		assert!((map_rect(at(0.45), sw, sh, place)[0] - 0.45 * g.pw).abs() < 0.01);
		assert_eq!(map_rect(at(0.60), sw, sh, place), at(0.60));
		assert_eq!(map_rect(at(0.45), sw, sh, Place::new(true, 100)), at(0.45), "with the plain cells it is in the middle");
	}

	#[test]
	fn mod_draws_follow_their_cell() {
		let (sw, sh) = (3440.0, 1440.0);
		let g = Geo::new(sw, sh).unwrap();
		let p = Place::new(true, 100);
		let centre = [g.x0 + g.pw * 0.5 - 10.0, 100.0, 20.0, 20.0];
		assert_eq!(map_rect(centre, sw, sh, p), centre);
		let tl = [g.x0 + 40.0, 30.0, 100.0, 50.0];
		let m = map_rect(tl, sw, sh, p);
		assert!((m[0] - 40.0).abs() < 0.01 && m[1] == 30.0 && m[2] == 100.0);
		let tr = [g.x0 + g.pw - 140.0, 30.0, 100.0, 50.0];
		assert!((map_rect(tr, sw, sh, p)[0] - (sw - 140.0)).abs() < 0.01);
		let below = [g.x0 + 40.0, sh - 90.0, 100.0, 50.0];
		assert_eq!(map_rect(below, sw, sh, p), below, "only the top corner cells move");
		assert_eq!(map_rect(tl, sw, sh, Place::new(false, 100)), tl);
		assert!((map_rect(tl, sw, sh, Place::new(false, 150))[2] - 100.0 / (2.0 * CELL_W)).abs() < 1e-3, "150 % is cut where the two cells would touch");
	}

	/// Arsenal Mastery's level bar sits under the ammo box (v ~0.09 .. 0.105, partly below the overlay cell): it still moves to the corner.
	#[test]
	fn a_mod_bar_under_the_ammo_box_still_moves_to_the_corner() {
		let (sw, sh) = (2560.0, 1080.0);
		let g = Geo::new(sw, sh).unwrap();
		let p = Place::new(true, 100).with_shape(shape(&centred(43.4), false).unwrap());
		let bar = [g.x0 + 0.02 * g.pw, 0.090 * sh, 0.17 * g.pw, 0.015 * sh];
		let m = map_rect(bar, sw, sh, p);
		assert!((m[0] - 0.02 * g.pw).abs() < 0.01 && m[1] == bar[1], "the bar moves with the left cell: {m:?}");
		let right = [g.x0 + 0.80 * g.pw, 0.090 * sh, 0.15 * g.pw, 0.015 * sh];
		assert!((map_rect(right, sw, sh, p)[0] - (sw - 0.20 * g.pw)).abs() < 0.01);
		let low = [g.x0 + 0.02 * g.pw, 0.19 * sh, 0.1 * g.pw, 0.02 * sh];
		assert_eq!(map_rect(low, sw, sh, p), low, "below the band nothing moves");
	}
}
