//! KER-27 / KER-26: the path conveyor. The class 899 emitter keeps a line of class 898 carriers walking its path; the
//! carriers move the hero standing on them (decomp/notes/kerwan_environment.md "KER-27 port spec", "KER-26 port spec").


/// KER-27: the conveyor carrier class (0x382), spawned only by the emitter.
pub(in crate::game) const CARRIER: u32 = 898;
/// KER-26: the conveyor emitter class (0x383).
pub(in crate::game) const BELT_EMITTER: u32 = 899;





