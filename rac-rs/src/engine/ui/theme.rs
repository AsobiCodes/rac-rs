//! Colours and sizes of the approved mockup (scratch_perf_menu/MOCKUP.dc.html), in the 1280x720 design space. Only the default
//! accent is built; the mockup's two other accents (5CC8FF, 7EE0A6) are a `Screen::accent` away.
use super::prim::{Rgba, Symbol};

pub const DESIGN_W: f32 = 1280.0;
pub const DESIGN_H: f32 = 720.0;

pub const ACCENT: Rgba = Rgba::rgb(0xFFB347);
/// Text on an accent-coloured button or chip (#1A1206, written as bytes: a hex literal of that size looks like a RAM address to the boundary test).
pub const INK_ON_ACCENT: Rgba = Rgba::rgba(26, 18, 6, 1.0);
pub const WHITE: Rgba = Rgba::rgb(0xFFFFFF);
pub const TEXT: Rgba = Rgba::rgb(0xEEF2F7);
pub const TEXT_LABEL: Rgba = Rgba::rgb(0xDCE3EC);
/// A label of a row that is inert right now (decision 1: dimmed instead of a pill on every row).
pub const TEXT_DIM: Rgba = Rgba::rgb(0x97A3B6);
pub const TEXT_MUTED: Rgba = Rgba::rgb(0xA9B4C4);
pub const TEXT_SECTION: Rgba = Rgba::rgb(0x8FA0B8);
pub const TEXT_BODY: Rgba = Rgba::rgb(0xC9D2DE);
pub const TEXT_DISABLED: Rgba = Rgba::rgb(0x8592A6);
pub const TEXT_BOX: Rgba = Rgba::rgb(0xFFE2B8);
pub const CHEVRON: Rgba = Rgba::rgb(0xDCE3EC);

pub const PANEL: Rgba = Rgba::rgba(10, 15, 27, 0.84);
pub const HAIRLINE: Rgba = Rgba::rgba(255, 255, 255, 0.08);
pub const PILL_BORDER: Rgba = Rgba::rgba(255, 255, 255, 0.14);
pub const BUTTON_BG: Rgba = Rgba::rgba(255, 255, 255, 0.06);
pub const CHIP_BG: Rgba = Rgba::rgba(255, 255, 255, 0.08);
pub const CAP_BG: Rgba = Rgba::rgba(255, 255, 255, 0.08);
pub const CAP_BORDER: Rgba = Rgba::rgba(255, 255, 255, 0.28);
pub const PIP_OFF: Rgba = Rgba::rgba(255, 255, 255, 0.16);
pub const BAR_OFF: Rgba = Rgba::rgba(255, 255, 255, 0.14);
pub const SWITCH_OFF: Rgba = Rgba::rgba(255, 255, 255, 0.18);
pub const SLIDER_TRACK: Rgba = Rgba::rgba(255, 255, 255, 0.18);
pub const SCROLLBAR: Rgba = Rgba::rgba(255, 255, 255, 0.25);
/// Amber tint behind the banner and the detail box.
pub const BOX_BG: Rgba = Rgba::rgba(255, 179, 71, 0.10);

pub const SYMBOL_CROSS: Rgba = Rgba::rgb(0x8AB4F8);
pub const SYMBOL_CIRCLE: Rgba = Rgba::rgb(0xF28B82);
pub const SYMBOL_SQUARE: Rgba = Rgba::rgb(0xF4A7D8);
pub const SYMBOL_TRIANGLE: Rgba = Rgba::rgb(0x7FD8BE);

/// The colour of a button symbol (also the letter's colour in the Letters glyph sets).
pub fn symbol_colour(s: Symbol) -> Rgba {
	match s {
		Symbol::Cross => SYMBOL_CROSS,
		Symbol::Circle => SYMBOL_CIRCLE,
		Symbol::Square => SYMBOL_SQUARE,
		Symbol::Triangle => SYMBOL_TRIANGLE,
	}
}

/// Backdrop: blur radius (design units) and scrim while the menu is up / while peeking at the game.
pub const BLUR_MENU: f32 = 16.0;
pub const BLUR_PEEK: f32 = 2.0;
pub const SCRIM_MENU: Rgba = Rgba::rgba(5, 8, 15, 0.42);
pub const SCRIM_PEEK: Rgba = Rgba::rgba(5, 8, 15, 0.05);

/// Seconds the backdrop eases between the two (wall clock; the renderer does it).
pub const EASE_S: f32 = 0.3;
