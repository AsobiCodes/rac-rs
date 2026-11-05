//! The retail planet picture on the modern Galactic Map (v0.5 screens, lane v05c). On the planet list (page 0xf) the game draws the
//! selected planet's picture into the overlay texture, in its bottom-left panel (s4); the modern page leaves a hole in its top-right card
//! (`ship::detail`, `pause_grid::holed_panel`) and `ship_host::place_planet` pushes ONE overlay quad into that hole (`crop::place`).
//! This file knows where the panel is in the 512 x 448 overlay picture and how the crop sits in the card; the crop math itself is
//! `crop.rs`. While the page flies in (`FLYING`) the retail panel is not in place yet, so no crop is placed and the hole is filled with
//! the card's colour (`settle`). The only PC number is the `ship_pic_inset` RAC_TUNE knob (ENV.md).
use super::crop;
use super::ship::Built;
use super::vendor::rrect;
use rac_engine::ui::prim::{Rect, Rgba};

/// `MenuSnap::state` of a page that flies in (`mouse_menu::read_flying`): the retail panel is not at its place yet.
pub const FLYING: u32 = 1;
/// The planet picture panel (s4) in the overlay picture: x, y, w, h in 512 x 448 pixels. x, w, h [C: decomp/notes/level_transition.md
/// "s4 is 307 x 120", s2 starts at x = 183]; y = 304 measured on the PAL classic picture (`gate/refs/galaxy_map_cl.png`) [C]; the USA
/// row is not verified [I] (same 448-row overlay).
pub const PANEL: [f32; 4] = [183.0, 304.0, 307.0, 120.0];
/// Overlay pixels cut off each side of the panel so its bevelled frame stays out of the picture.
pub const INSET: f32 = 3.0;
/// `MenuSnap::state` of a menu that closes (`MenuSnap::busy`).
const CLOSING: u32 = 0x14;
/// The card's fill (`ship::card`), also what the hole shows while the page flies in.
pub const CARD_FILL: Rgba = Rgba::rgb(0x0C1322);

/// The crop of the panel with `inset` overlay pixels cut off each side.
pub fn src(inset: f32) -> [f32; 4] {
	crop::inset(PANEL, inset)
}

/// Where the picture shows inside the card (design space): the panel's display aspect (about 3.08 : 1), centred. Always cut by the
/// default `INSET`, so the page's draw list does not move with the knob.
pub fn hole_in(card: Rect) -> Rect {
	crop::display_rect(card, src(INSET))
}

/// The texture coordinates of the crop for `inset` (the `ship_pic_inset` knob moves only these, never the hole).
pub fn uv(inset: f32) -> [f32; 4] {
	crop::uv_of(src(inset))
}

/// The `ship_pic_inset` knob (RAC_TUNE, default `INSET`), read when the quad is placed.
pub fn inset_now() -> f32 {
	crate::devtune::f32("ship_pic_inset", INSET)
}

/// The page of a menu in state `state`: while it flies in (`FLYING`) or closes, the retail panel is not in place, so the crop goes away
/// and the hole is filled with the card's colour; settled, the page is left as built.
pub fn settle(built: &mut Built, state: u32) {
	if !matches!(state, FLYING | CLOSING) {
		return;
	}
	if let Some(hole) = built.planet.take() {
		// One pixel in from the card's edges, so the card's own border stays on top.
		rrect(&mut built.prims, Rect::new(hole.x + 1.0, hole.y, (hole.w - 2.0).max(0.0), hole.h), 0.0, CARD_FILL, None);
	}
}
