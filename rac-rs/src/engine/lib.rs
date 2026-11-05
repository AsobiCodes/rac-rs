//! The game-neutral engine (engine split stage E1, 2026-10-26; plan: ENGINE_SPLIT.md): window and frame loop,
//! wgpu renderer and the PC effects, audio, movies, GS capture and GS-full, input devices, texture packs, the mod host, run log.
//! It must not reach into `rac1` (boundary test `src/boundary_tests.rs`, today's crossings in BOUNDARY_CAPS.txt, which only
//! shrink). Stage E1 is a pure move: each module kept its name, and the crate root re-exports it, so old paths still work.
//! Stage E3 (2026-10-26): its own crate, `rac-engine` (manifest rac-rs/crates/engine/Cargo.toml); it depends on `rac-formats`
//! only, so the compiler holds the boundary (no path into `rac1` compiles).

pub mod achievements;
pub mod audio;
pub mod audio_out;
pub mod awake;
pub mod awake_linux;
pub mod devtune;
pub mod dialog_linux;
pub mod dynlight;
pub mod field;
pub mod fmv;
pub mod fxpool;
pub mod gauss;
pub mod gfx;
pub mod gscap;
pub mod gscap_guard;
pub mod gsfull;
pub mod fmath;
pub mod fxhash;
pub mod hot;
pub mod input;
pub mod loadlog;
pub mod looks;
pub mod furspec;
pub mod omni;
pub mod overlay;
pub mod pack;
pub mod pcopts;
pub mod posefix;
pub mod prims;
pub mod prop;
pub mod quit;
pub mod render;
pub mod rtss_crash;
pub mod rtss_layer;
pub mod runlog;
#[cfg(any(test, target_os = "linux"))]
pub mod runlog_linux;
pub mod runlog_policy;
pub mod shadowgeom;
pub mod splitview;
pub mod spumix;
pub mod spurev;
pub mod texfilter;
pub mod metal_option;
pub mod smoke_option;
pub mod smoke_rows;
pub mod shot_rows;
pub mod fire_option;
pub mod texpack;
pub mod ui;
pub mod viewer;
pub mod warp;
pub mod wine;
pub mod install;

pub use install::{app_name, install_root};

// The modules of the crates below, under their old root paths (`crate::wad`, `crate::viewer`, ...).
use rac_formats::{anim, bytes, gameplay, gs_state, hud, iso, level, mesh, mobies, occlusion, physics, shrubs, sky, sound, terrain, ties, vumath};
#[cfg(test)]
use rac_formats::anim_pose;
