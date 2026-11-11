//! The Ratchet & Clank 1 pack (engine split stage E1, 2026-10-26; plan: ENGINE_SPLIT.md): retail mode's glue to
//! R&C1's RAM, the PAL / USA disc map, the PC rows in R&C1's own menus, Modern camera / strafe / aim, the per-planet pieces, and
//! the old hand-port leftovers (game, player, particles, flow, camera, ...; being removed). It may use `engine` and `formats`.
//! Stage E1 is a pure move: the crate root re-exports every module, so old paths still work.
//! Stage E3 (2026-10-26): its own crate, `rac1` (manifest rac-rs/crates/rac1/Cargo.toml); it depends on `rac-engine` and
//! `rac-formats`. The exe (rac-rs/exe) links it with the recompiled retail code (`set_register`).

// `ee_game::...` in every module resolves to this crate's root (`register` below), so no call site changes.
extern crate self as ee_game;

pub mod ambience;
pub mod trophies;
pub mod trophy_icons;
pub mod camera;
pub mod cues;
pub mod fire;
pub mod flow;
pub use rac_engine::fmath;
pub mod frontend;
pub mod frontend_lang;
pub mod frontend_lines;
pub mod gadgets;
pub mod game;
pub mod help;
pub mod hero_fur;
pub mod diag_facts;
pub mod host;
pub mod interp;
pub mod inventory;
pub mod kofi;
pub mod map;
pub mod modern_menu;
mod fire_look;
mod weapon_lights;
mod metal_materials;
mod metal_equipment;
mod smoke_table;
pub mod ocean;
pub mod omni_bind;
pub mod pack_bind;
pub mod kbm_actions;
pub mod particles;
pub mod planet_looks;
pub mod player;
pub mod retail;
pub mod save;
pub mod scene;
pub mod settings;
pub mod shadows;
pub mod shipgloss;
pub mod shop;
pub mod strips;
pub mod toc;
pub mod update_notice_linux;
pub mod water;

// The modules of the crates below, under their old root paths (`crate::wad`, `crate::viewer`, ...).
use rac_engine::{achievements, audio, awake, devtune, dialog_linux, dynlight, field, fmv, furspec, fxpool, gfx, gscap_guard, gsfull, hot, loadlog, looks, omni, overlay, pack, pcopts, posefix, prims, prop, render, runlog, shadowgeom, splitview, spumix, texfilter, texpack, viewer, warp, app_name};
use rac_formats::{anim, anim_pose, bytes, camgrid, collision, gameplay, gs_state, hud, iso, level, mesh, mobies, music, occlusion, physics, rand, shrubs, sky, sound, terrain, ties, vumath, wad};

static REGISTER: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

/// The exe passes `ee_game::register` here before it calls `rac_game::main`.
pub fn set_register(f: fn()) {
	let _ = REGISTER.set(f);
}

/// `ee_game::register()`: registers the recompiled retail code (the function the exe passed in).
#[cfg(not(test))]
pub fn register() {
	(REGISTER.get().expect("ee_game::register was not passed in (exe/src/main.rs calls rac_game::set_register first)"))()
}

/// Unit tests have no exe: they use ee-game directly (a dev-dependency, so it does not hold up the normal build).
#[cfg(test)]
pub fn register() {
	ee_game_for_tests::register()
}

#[cfg(test)]
mod format_tests;
#[cfg(test)]
mod engine_tests;
#[cfg(test)]
mod fmath_scan_tests;
