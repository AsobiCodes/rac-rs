//! rac_game: the rac-rs engine as a library crate (the exe in exe/ links it with the recompiled retail code and calls `main`).
//! `src/main.rs` is the real crate root: it is `include!`d below, so it cannot carry a `//!` header of its own. It holds the module
//! list and the command line; the game runs in retail mode (`view ISO LEVEL --retail DIR` or `--retail-cold`), the only game path.
//!
//! Map of the top-level modules, by job:
//! - Window, renderer, retail mode: `viewer` (winit window, wgpu renderer, per-frame handler), `retail` (runs the recompiled game, reads
//!   its RAM each field, feeds input, effects, sound), `gsfull` (the game's own complete GS picture), `fmv` (PSS movies: demux, MPEG-2).
//! - Disc and file formats: `iso` (sector access), `toc`, `wad` (LZ decompression), `bytes` (bounds-checked reads), `level`, `gameplay`
//!   (per-level instance tables), `terrain`, `ties`, `shrubs`, `mobies`, `anim`, `mesh`, `sky`, `hud` (HUD graphics), `gadgets`, `map`, `scene`.
//! - Level drawing helpers: `water`, `ocean`, `fire`, `strips`, `shadows`, `shipgloss`, `dynlight`, `occlusion`, `camgrid`, `overlay` (2D quads).
//! - Audio: `audio` (software mixer), `spumix` (SPU2 voices), `spurev` (SPU2 reverb), `gauss`, `sound` (sound banks), `music`, `ambience`.
//! - Menus and saves: `frontend` (title scene, page menu), `help`, `inventory`, `shop`, `save`, `settings`.
//! - Collision and camera: `physics`, `collision`, `camera`.
//! - Mods: `omni` (OmniLoader host: Luau script mods through omniloader.dll; pad reports and hook points; nothing runs without a mod folder).
//! - Dev and test aids: `devtune`, `loadlog`, `render` (software rasteriser for checking decoded geometry), `env_doc_tests`,
//!   `size_limits_tests` (file and fn size limits, SIZE_CAPS.txt), `fmath_scan_tests` (no C-runtime float maths in retail/ and omni/).
//! - Float maths with the same bits on every OS for code that writes game memory: `fmath`.
//! - Old hand port (retired 2026-10-08, still compiled): `game`, `player`, `particles`, `flow`.
// The game as a library (crate rac_game): src/main.rs is its root, included as it is. The exe (package rac-rs-exe in exe/)
// links it with the recompiled retail code, crate ee-game, which no crate of the game depends on: so they compile at the
// same time as the 427 generated crates. Since stage E3 the code lives in the crates rac-formats, rac-engine and rac1
// (rac-rs/crates/); this crate is the command line and the source-tree tests.

include!("main.rs");

/// The exe passes `ee_game::register` here before it calls `main` (kept in rac1, whose modules call it).
pub use rac1::set_register;

#[cfg(test)] mod boundary_tests;
#[cfg(test)] mod env_doc_tests;
#[cfg(test)] mod size_limits_tests;
