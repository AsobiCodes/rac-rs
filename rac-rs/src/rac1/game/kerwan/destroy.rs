//! KER-31: the placements of classes 915 / 916 / 917 (decomp/notes/kerwan_environment.md "KER-31 port spec"), shared
//! updater FUN_002dbc40: on their first update they despawn themselves. The updater's distance and angle math has no
//! store and its results are unused, so only the despawn is ported. No kill record is written: every level load and
//! death restore places them again and they despawn again.

/// KER-31: the three self-despawning classes (14 placements, mode bits 0x20, placed flags 0x4060).
pub(in crate::game) const SELF_DESTROY_915: u32 = 915;
pub(in crate::game) const SELF_DESTROY_916: u32 = 916;
pub(in crate::game) const SELF_DESTROY_917: u32 = 917;

