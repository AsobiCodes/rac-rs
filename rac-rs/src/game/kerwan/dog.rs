//! KER-4: the robot dog 573. (decomp/notes/kerwan_*.md; moved from game/kerwan.rs.)


/// KER-4: the robot dog (updater FUN_002c54e8, 87 placements; decomp/notes/kerwan_enemies.md "KER-4 port spec").
pub(in crate::game) const DOG: u32 = 573;











// KER-1: the class 574 walker (updater FUN_002c6900, 19 placements; decomp/notes/kerwan_enemies.md "KER-1a port spec").
// KER-1a holds the pre-switch head and state 0; the hit head (KER-1h), states 1-15 (KER-1b..1f) and the tail (KER-1g)
// are the placeholders `walker_hit` / `walker_switch` / `walker_tail` until their pieces land.

