//! R&C1's modern Options screen (v0.5, stage S3): the rows table the engine's `ui` draws (`tabs`, `rows_*`, `costs`, `text`), the
//! menu style key (`style`). The values travel through `retail/pcext/ui_bridge.rs`; the door is `retail/modern_swallow.rs`.
mod costs;
pub mod btn_text;
pub mod crop;
pub mod hide;
pub mod host;
pub mod icons;
pub mod look;
pub mod pause;
pub mod pause_ach;
pub mod pause_grid;
mod pause_help;
pub mod pause_keys;
pub mod pause_host;
mod pause_pc;
pub mod pause_quick;
mod rows_camera;
mod rows_display;
mod rows_picture;
mod rows_effects;
mod rows_game;
mod rows_graphics;
mod rows_system;
pub(crate) mod rows_fgen;
pub(crate) mod rows_upscale;
pub mod style;
mod tabs;
mod text;
pub mod title;
pub mod title_host;
pub mod title_slots;
pub mod vendor;
pub mod vendor_cut;
pub mod vendor_host;
pub mod vendor_nav;
pub mod vendor_portrait;
pub mod map;
pub mod map_host;
pub mod ship;
pub mod ship_anim;
pub mod ship_host;
pub mod ship_marks;
pub mod ship_pic;
mod vendor_text;
pub mod watchdog;
#[cfg(test)]
mod map_tests;
#[cfg(test)]
mod pause_ach_tests;
#[cfg(test)]
mod pause_grid_tests;
#[cfg(test)]
mod pause_keys_tests;
#[cfg(test)]
mod pause_pc_tests;
#[cfg(test)]
mod pause_help_tests;
#[cfg(test)]
mod pause_quick_tests;
#[cfg(test)]
pub(crate) mod pause_tests;
#[cfg(test)]
mod ship_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vendor_tests;

pub use tabs::{menu, PACK, PRESETS};
