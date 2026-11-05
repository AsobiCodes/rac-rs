//! The widget kit of the modern UI (layer 1): plain data in, rectangles and prims out. Nothing here knows options rows, a game or a
//! window system. A screen fills a `data::Screen` (tabs, a list of banner / section / row items with their controls, a detail
//! panel, hints, the peek pill); `layout::build` places it in the 1280x720 design space with a `text::TextMeasure`;
//! `draw::prims` turns the layout into `Prim`s; `hit::hit` finds the widget under a point; `scale::Scale` maps design space to
//! any window. Input actions, repeat and devices are in `ui/input.rs` (shared by all screens). Layer 2 is a screen on top, today
//! the options screen (`ui/{rows,model,screen}.rs`); the pause menu, title menu, vendor and ship travel menu come later the same way.
pub mod data;
pub mod draw;
mod draw_detail;
mod draw_list;
pub mod hit;
pub mod layout;
pub mod layout_detail;
pub mod page;
pub mod page_draw;
#[cfg(test)]
mod page_tests;
pub mod scale;
pub mod shell;
pub mod text;

pub use data::{BannerData, BoxData, Chip, Control, Detail, Footer, Hint, HintKind, Item, Meter, RowData, Screen};
pub use hit::{hit, slider_t, Hit};
pub use layout::{build as layout_screen, Layout};
pub use scale::Scale;
pub use text::{FixedAdvance, TextMeasure};

use crate::ui::prim::Prim;

/// Layout and draw list of a screen in one call.
pub fn frame(s: &Screen, m: &dyn TextMeasure) -> (Layout, Vec<Prim>) {
	let l = layout::build(s, m);
	let p = draw::prims(s, &l);
	(l, p)
}
