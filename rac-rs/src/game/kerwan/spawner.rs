//! KER-5 / KER-6a: the lobbing spawner 455 and its lobs 545. (decomp/notes/kerwan_*.md; moved from game/kerwan.rs.)


/// KER-5: the lobbing spawner (updater FUN_002be898, placements 408 / 409; decomp/notes/kerwan_enemies.md "KER-5 port
/// spec").
pub(in crate::game) const SPAWNER: u32 = 455;
/// KER-5: the spawner's lobbed bomb (no placements: a pool, `Game::add_lob_pool`; its update is KER-6a).
pub(in crate::game) const LOB: u32 = 545;

