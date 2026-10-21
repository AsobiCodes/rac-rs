//! KER-10: the robo shack sign class 825 (decomp/notes/kerwan_environment.md "KER-10 port spec"), updater
//! FUN_002d3788: a constant spin about z every tick. Its moving-surface publish needs no call here: `actor_pass`
//! snapshots the standing prop's pose before the updates and `Game::carry` applies the old -> new rotation (zero
//! displacement) through the 0x20 carry bit of its placed flags 0x60.


/// KER-10: the sign class (1 placement, Moby 923).
pub(in crate::game) const SHACK_SIGN: u32 = 825;


