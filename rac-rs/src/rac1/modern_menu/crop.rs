//! One crop of the retail overlay picture into a hole of a modern page (v0.5 screens, lane v05c). Retail draws some pictures only
//! into the overlay texture (the 512 x 448 GS capture): Ratchet with the selected weapon on the pause menu's Weapons / Gadgets / Items
//! pages, the planet picture on the Galactic Map. The modern page leaves a hole where the picture goes (`pause_grid::holed_panel`) and
//! the host pushes ONE overlay quad into that hole (`place`): a rectangle of the overlay texture (`src`, in 512 x 448 pixels, cut
//! down by `inset` so the retail panel's own frame stays out), shown at the aspect the picture has on a 4:3 screen (`display_rect`).
//! This file is the only place that knows the overlay's size and its pixel aspect; `pause_host/grid.rs` and `ship_host.rs` call it.
use crate::overlay::Quad;
use rac_engine::ui::kit::Scale;
use rac_engine::ui::prim::Rect;

/// The overlay texture's picture space (the game's 512 x 448 screen).
pub const OV_W: f32 = 512.0;
pub const OV_H: f32 = 448.0;
/// A crop of the 512 x 448 picture shows 7/6 wider than its pixel ratio, since the picture is drawn 4:3 (512 : 448 = 8 : 7, and 4 : 3 is
/// 7/6 wider).
const ASPECT_NUM: f32 = 7.0;
const ASPECT_DEN: f32 = 6.0;

/// `r` shrunk to the largest rect of aspect w : h that fits inside it, centred (a degenerate w or h leaves `r`).
pub fn contain(r: Rect, w: f32, h: f32) -> Rect {
	if w <= 0.0 || h <= 0.0 {
		return r;
	}
	let s = (r.w / w).min(r.h / h);
	let (cw, ch) = (w * s, h * s);
	Rect::new(r.x + (r.w - cw) * 0.5, r.y + (r.h - ch) * 0.5, cw, ch)
}

/// A region (x, y, w, h in overlay pixels) cut down by `d` pixels on every side (`d` below 0 counts as 0; never below one pixel).
pub fn inset(region: [f32; 4], d: f32) -> [f32; 4] {
	let d = d.max(0.0);
	[region[0] + d, region[1] + d, (region[2] - 2.0 * d).max(1.0), (region[3] - 2.0 * d).max(1.0)]
}

/// Texture coordinates (u0, v0, u1, v1) of a region of the overlay picture: the texture always spans the whole 512 x 448 picture.
pub fn uv_of(src: [f32; 4]) -> [f32; 4] {
	[src[0] / OV_W, src[1] / OV_H, (src[0] + src[2]) / OV_W, (src[1] + src[3]) / OV_H]
}

/// Where a crop of `src` shows inside `view` (design space): the largest rect of the crop's DISPLAY aspect, centred. A crop of w x h
/// overlay pixels shows with aspect w / h * 7 / 6.
pub fn display_rect(view: Rect, src: [f32; 4]) -> Rect {
	contain(view, src[2] * ASPECT_NUM, src[3] * ASPECT_DEN)
}

/// The overlay quad that draws the crop `uv` into the design-space `hole` of a window `win` (pixels).
pub fn quad(win: (u32, u32), hole: Rect, uv: [f32; 4]) -> Quad {
	let r = Scale::fit(win.0, win.1).to_window(hole);
	Quad { tex: crate::viewer::RETAIL_OV_TEX, rect: [r.x, r.y, r.w, r.h], uv, colour: [1.0; 4], angle: 0.0, corners: None }
}

/// Pushes the crop's quad (`quad`) at the end of the HUD list, over what the page left below its hole.
pub fn place(hud: &mut Vec<Quad>, win: (u32, u32), hole: Rect, uv: [f32; 4]) {
	hud.push(quad(win, hole, uv));
}

#[cfg(test)]
mod tests;
