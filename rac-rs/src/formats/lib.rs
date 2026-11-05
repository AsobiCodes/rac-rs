//! Insomniac's file and RAM formats shared by the Ratchet & Clank games (engine split stage E1, 2026-10-26; plan:
//! ENGINE_SPLIT.md): WAD, level sections, terrain, ties, shrubs, sky, mobys, animation, collision, occlusion,
//! sound banks, music, HUD art. It must not reach into `engine` or `rac1` (boundary test `src/boundary_tests.rs`, today's
//! crossings in BOUNDARY_CAPS.txt, which only shrink). Stage E1 is a pure move: the crate root re-exports every module.
//! Stage E2 (2026-10-26) took the R&C1 disc map out (`rac1/toc/files.rs`, `rac1/cues.rs`, disc tests in `rac1/format_tests/`)
//! and brought the neutral `iso`, `gs_state`, `vumath` and `rand` in: nothing here names `rac1` or `engine` now.
//! Stage E3 (2026-10-26): its own crate, `rac-formats` (manifest rac-rs/crates/formats/Cargo.toml); it depends on no other
//! part of the game, so the compiler holds the boundary.
//! Stage E5 (2026-10-27): R&C2 levels decode through the same modules. `game` holds the per-game layout tables (`Game`, `Layout`);
//! `rc2` holds what only R&C2 needs (disc reads, level loader, gameplay tables, armor, RAM class decoder, placed ties).

pub mod anim;
pub mod anim_pose;
pub mod bytes;
pub mod camgrid;
pub mod collision;
pub mod gamelayout;
pub mod gameplay;
pub mod gs_state;
pub mod hud;
pub mod iso;
pub mod level;
pub mod mesh;
pub mod mobies;
pub mod music;
pub mod occlusion;
pub mod physics;
pub mod rand;
pub mod rc2;
pub mod shrubs;
pub mod sky;
pub mod sound;
pub mod terrain;
pub mod texhash;
pub mod texname;
pub mod ties;
pub mod vumath;
pub mod wad;
