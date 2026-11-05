//! The modern UI: menus drawn by the engine in the approved mockup look, at display rate, over the blurred game. Two layers:
//! `kit/` is the generic widget kit (plain data in, rectangles and `Prim`s out, hit testing, scale), shared by every screen;
//! the options screen is the first screen on top of it (`rows` = the tables a game pack hands in through `Pack::ui`, `model` /
//! `model_edit` = its state and value editing, `screen` = model -> kit data, `host` / `host_mouse` = the `Ui` that takes keys, pad
//! and mouse). Nothing here names a game; rendering (fonts, GPU) is a separate stage that consumes `Ui::frame()`.
pub mod atlas;
pub mod backdrop;
mod backdrop_gpu;
pub mod demo;
pub mod fit;
pub mod font;
pub mod glyph_cache;
pub mod gpu;
mod gpu_pipes;
pub mod host;
mod host_mouse;
pub mod images;
pub mod inline;
pub mod input;
pub mod instances;
pub mod keycaps;
pub mod kit;
pub mod live;
pub mod model;
mod model_edit;
pub mod prim;
pub mod rows;
pub mod screen;
pub mod shapes;
pub mod sound;
pub mod theme;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod tests_gpu;
#[cfg(test)]
mod tests_layout;
#[cfg(test)]
mod tests_nav;
#[cfg(test)]
mod tests_render;
#[cfg(test)]
pub(crate) mod testkit;

pub use host::Ui;
pub use input::{Action, Device, GlyphSet, MouseButton, MouseEvent, Outcome, PadSample, UiKey};
pub use model::Change;
pub use rows::{Cost, Needs, NeedsRule, Notice, RowKind, UiChrome, UiMenu, UiPack, UiPresetDef, UiRow, UiSection, UiSound, UiTab, LANGS, T5};

use std::sync::atomic::{AtomicBool, Ordering};

/// The modern UI is up (set by the game side when it opens / closes it): the game's own menu input and drawing step aside.
static OPEN: AtomicBool = AtomicBool::new(false);

pub fn is_open() -> bool {
	OPEN.load(Ordering::Relaxed)
}

pub fn set_open(open: bool) {
	OPEN.store(open, Ordering::Relaxed);
}
