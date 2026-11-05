//! The vendor's live portrait in the modern shop (v0.5 test build 4, lane v5g). Retail's shop screen has a panel with the animated
//! green vendor (a class-12 actor that idles, talks and reacts to the browsing). rac-rs draws him natively: `frame_render` poses him
//! every field into the 512 x 128 `PORTRAIT_PANEL_TEX` target and pushes one HUD quad of it at the rect of the 3D screen. The modern
//! page hides the retail screens (`frame_ui::hide_retail` drops the target quads), so the guy would vanish; the user wants him in the
//! detail card, where the weapon picture used to be. So the detail card leaves a HOLE (`hole`, `card`) and `place` puts ONE quad into it:
//! a black backdrop (retail's panel interior) and the portrait quad of this frame, moved into the hole at its own display aspect. The
//! quad is retail's, so he animates exactly as in the Classic shop (the pose comes from the game's own clock). Pure geometry here; the
//! only static is read through `vendor_host::portrait`.
use super::crop;
use super::pause_grid::holed_panel;
use super::vendor::{rrect, DETAIL};
use crate::overlay::Quad;
use rac_engine::ui::kit::Scale;
use rac_engine::ui::prim::{Border, Prim, Rect, Rgba};
use rac_engine::ui::theme::{HAIRLINE, PANEL};

/// The retail portrait panel's display aspect (w : h): about 295 x 230 px on the PAL classic picture (`gate/refs/vendor_cl.png`, 1280 x 720)
/// [C]. `place` fits the real quad of the frame into the hole at ITS aspect; this number only sizes the hole.
pub const ASPECT: f32 = 1.28;
/// The band of the detail card the guy sits in, as distances from the card's top (below the name and the ammo bar) and bottom (above
/// the price line, 114 px tall with its button).
const BAND_TOP: f32 = 112.0;
const BAND_BOTTOM: f32 = 126.0;
/// Space between the card's side and the hole.
const SIDE: f32 = 22.0;
const HOLE_RADIUS: f32 = 6.0;

/// The hole of the detail card `card` (design space): the largest rect of the retail panel's aspect that fits the free band, centred.
pub fn hole(card: Rect) -> Rect {
	let band = Rect::new(card.x + SIDE, card.y + BAND_TOP, card.w - 2.0 * SIDE, (card.h - BAND_TOP - BAND_BOTTOM).max(0.0));
	crop::contain(band, ASPECT, 1.0)
}

/// The detail card's panel with the hole cut out (the world and the quad `place` pushes show through) and a hairline around the hole.
pub fn card(hole: Rect) -> Vec<Prim> {
	let mut out = holed_panel(DETAIL, hole, 16.0, PANEL, Border { width: 1.0, colour: HAIRLINE });
	rrect(&mut out, hole, HOLE_RADIUS, Rgba::rgba(0, 0, 0, 0.0), Some((1.0, HAIRLINE)));
	out
}

/// The window-pixel rects of the pushed quads: (backdrop = the whole hole, portrait = the retail quad's rect fitted into the hole at its
/// own aspect). `retail` = the rect of retail's portrait quad in window pixels (only its w : h counts).
pub fn rects(win: (u32, u32), hole: Rect, retail: Option<[f32; 4]>) -> ([f32; 4], Option<[f32; 4]>) {
	let scale = Scale::fit(win.0, win.1);
	let back = scale.to_window(hole);
	let inner = retail.filter(|r| r[2] > 0.0 && r[3] > 0.0).map(|r| {
		let fitted = crop::contain(hole, r[2], r[3]);
		let w = scale.to_window(fitted);
		[w.x, w.y, w.w, w.h]
	});
	([back.x, back.y, back.w, back.h], inner)
}

/// Pushes the hole's backdrop (black, like the retail panel's interior the target is added onto) and, when the game drew the portrait
/// this frame, its quad moved into the hole. `retail` = the portrait quad `hide_retail` took out of the HUD list.
pub fn place(hud: &mut Vec<Quad>, win: (u32, u32), hole: Rect, retail: Option<Quad>) {
	let (back, inner) = rects(win, hole, retail.as_ref().map(|q| q.rect));
	crate::overlay::rect(hud, back, [0.0, 0.0, 0.0, 1.0]);
	if let (Some(q), Some(rect)) = (retail, inner) {
		hud.push(Quad { rect, ..q });
	}
}

#[cfg(test)]
mod tests;
