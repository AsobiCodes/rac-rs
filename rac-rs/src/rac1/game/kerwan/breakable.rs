//! KER-29: the breakable prop class 1827 (decomp/notes/kerwan_environment.md "KER-29 port spec"). The Kerwan twin of
//! the Aridia white flower (ARI-35): same state machine and break, one surface-point piece instead of five.


/// KER-29: the breakable prop (updater FUN_002e2278, 39 placements), its fragment piece and its static remains.
pub(in crate::game) const BREAKABLE: u32 = 1827;


/// KER-10: the statue class 817 (updater FUN_002d3608, 1 placement): the same state machine as 1827, broken into three
/// explicit fragments (one per class, in this order) with no surface points and no static remains.
pub(in crate::game) const STATUE: u32 = 817;



