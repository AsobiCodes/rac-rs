//! Gold-bolt tally and record-slot link for the planet tables (rec/l4_09).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Gold table: 20 planets x 4 bytes, contiguous.
const GOLD_BASE: u32 = 0x0014_BEC0;
const GOLD_BYTES: u32 = 20 * 4;
/// Bolts can never exceed this count.
const GOLD_MAX: u32 = 40;

/// Index word read by record_link.
const INDEX_ADDR: u32 = 0x0015_FF40;
/// Record list: 12-byte records, word 0 = key, word 1 = value, word 2 = link.
const RECORD_BASE: u32 = 0x001D_C800;
const RECORD_STRIDE: u32 = 12;
const RECORD_END: u32 = u32::MAX;
/// Slot tables indexed by the index word.
const SLOT_TABLE: u32 = 0x0019_7400;
const SLOT_OUT: u32 = 0x0019_7780;
/// Offset inside the slot's target object that receives the record link.
const LINK_FIELD: u32 = 0x2C;

/// 0x256938 gold_bolts_found() -> u32: counts the nonzero gold bytes over 20 planets x 4, clamped to 0..40.
pub fn gold_bolts_found(c: &mut Cpu) -> bool {
    let mut count = 0u32;
    for i in 0..GOLD_BYTES {
        if c.r8(GOLD_BASE + i) != 0 {
            count += 1;
        }
    }
    c.ret(count.min(GOLD_MAX));
    true
}

/// 0x256a38 record_link(key) -> void: finds the first record whose key is -1 or `key`, copies its value into
/// the slot table at the index word, and if that slot's target is set, stores the record link into it.
pub fn record_link(c: &mut Cpu) -> bool {
    let key = c.arg(0);
    let idx = c.r32(INDEX_ADDR);
    let mut rec = RECORD_BASE;
    loop {
        let k = c.r32(rec);
        if k == RECORD_END || k == key {
            break;
        }
        rec += RECORD_STRIDE;
    }
    let value = c.r32(rec + 4);
    let slot = idx.wrapping_mul(4);
    // Retail tests the slot word read before the SLOT_OUT store (beqz t1 in the store's delay slot).
    let pre = c.r32(SLOT_TABLE.wrapping_add(slot));
    c.w32(SLOT_OUT.wrapping_add(slot), value);
    if pre != 0 {
        let link = c.r32(rec + 8);
        let target = c.r32(SLOT_TABLE.wrapping_add(slot));
        c.w32(target.wrapping_add(LINK_FIELD), link);
    }
    true
}
