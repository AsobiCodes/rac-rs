//! What the generated crates export (their `FNS` tables) and the hand-written-function test they inline.

/// One recompiled function: its code range (validated against RAM by `hash`), block labels and body.
pub struct Fn {
    pub entry: u32,
    pub end: u32,
    pub hash: u64,
    pub labels: &'static [u32],
    pub f: fn(&mut crate::cpu::Cpu),
}

/// The first fields of ee-oracle's `rec::Table` (both `#[repr(C)]`). Only ever a view.
#[repr(C)]
#[non_exhaustive]
pub struct RecHot {
    pub lo: u32,
    pub hand_bits: Vec<u64>,
}

impl RecHot {
    /// A hand-written function replaces the one at `entry` (ee-oracle `Table::has_hand`).
    #[inline(always)]
    pub fn has_hand(&self, entry: u32) -> bool {
        let i = (entry.wrapping_sub(self.lo) / 4) as usize;
        self.hand_bits.get(i >> 6).is_some_and(|w| w >> (i & 63) & 1 != 0)
    }
}
