//! HUD position and scale, the host's half (R&C1): whether the picture on screen is live gameplay and the overlay quads of a frame. The
//! geometry (cells, `Place`, `map_rect`, tests) lives in the engine since the dev s138 merge: `engine/viewer/hud_place.rs`.
use super::super::{Gpu, RETAIL_OV_TEX};
use crate::overlay::Quad;
pub use crate::viewer::hud_place::*;

/// The breath tank (underwater without the O2 mask: HUD slot 4, decomp/notes/hero_swim.md "S1 breath HUD": anchor x 492 right-aligned,
/// 42 x 192 PS2 px about row 208; it slides in across the picture's right edge, gscap/edge_cut.rs): the right-edge band that HUD position
/// Corners moves to the window's edge with the top boxes (GitHub #65). Measured on PAL (fg1 11_0421, hero diving, 1280x720): tank frame
/// rows 115 .. 301 of 448, x 451 .. 490; its strip while sliding in is rows 111 .. 303 (edge_cut.rs). The USA disc draws the HUD about 15
/// rows lower [I: from the top prompts, guard.rs; the tank not measured on USA]. So the band is rows 104 .. 324 (both discs with a few
/// rows to spare) from x 440. It moves only on pictures where the capture finds the tank there and nothing else across the band's edges
/// (`Guard::side`; review R8-1: a HelpDesk box of 7+ lines reaches above row 324 and to x 478, decomp/notes/ingame_menu_leftovers.md P1-a).
const BREATH_TANK: SideBand = SideBand { u0: 440.0 / 512.0, v0: 104.0 / 448.0, v1: 324.0 / 448.0 };

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
	// The PAL help panels and their glyphs share one 512x448 overlay. Display it with square
	// pixels instead of widening the entire picture to 4:3 as for the original PS2 HUD.
	if let Some(aspect) = r.classic_help_aspect() {
		let width = (sh * aspect).min(sw);
		let height = width / aspect;
		return vec![Quad {
			tex: RETAIL_OV_TEX, rect: [(sw - width) * 0.5, (sh - height) * 0.5, width, height],
			uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None,
		}];
	}
	let guard = g.retail_ov_guard.guard();
	// The tank's band moves only on pictures that hold the tank there with nothing else across its edges (the capture measures it:
	// gscap/edge_cut/guard.rs `Guard::side`; review R8-1: a tall HelpDesk box, a marker, no tank at all keep their 4:3 place).
	rac_engine::gscap::edge_cut::guard::set_side(BREATH_TANK.u0, BREATH_TANK.v0, BREATH_TANK.v1);
	let place = Place::new(crate::retail::pcdisp::hud_corners(), crate::retail::pcdisp::hud_scale_pct());
	let place = if guard.side { place.with_side(BREATH_TANK) } else { place };
	// The cells end above the highest top help prompt of the picture on screen (its own guard, `Gpu::retail_ov_guard`) and carry the corner's
	// widgets whole; where that is not possible the picture keeps its 4:3 place.
	let place = shape(&guard, crate::retail::disc::usa()).map(|s| place.with_shape(s));
	if let Some(place) = place.filter(|p| p.active() && live(r)) {
		let cut = crate::retail::edge_cut_rows();
		if let Some(qs) = quads(RETAIL_OV_TEX, sw, sh, ovw, ovh, place, cut.as_deref()) {
			g.hud_place = Some(place);
			return qs;
		}
	}
	super::edge_fill::quads(RETAIL_OV_TEX, sw, sh, k, ovh)
}
