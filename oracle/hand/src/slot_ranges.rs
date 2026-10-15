//! Slot-range bookkeeping of the Eudora overlay: claims a slot's size and clears the top bit of its ranges.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Table base (retail a1/a2 register base, lui 0x18 - 0x1a28).
const TABLE: u32 = 0x17e5d8;
/// Offset of the record-array pointer inside the table.
const RECORDS: u32 = 0x18;
/// Offset of the first range list pointer inside the table.
const LIST_A: u32 = 0x28;
/// Offset of the second range list pointer inside the table.
const LIST_B: u32 = 0x24;
/// Offset of the per-slot claim words inside a record array.
const CLAIM: u32 = 0x74;
/// Offsets of the range bounds inside a record array (first and second range list).
const BOUND_A: u32 = 0x14;
const BOUND_B: u32 = 0x34;
/// Size is rounded up to 16 bytes and the top bit of each range entry is cleared.
const SIZE_MASK: u32 = 0xffff_fff0;
const TOP_BIT_CLEAR: u32 = 0x7fff_ffff;

/// 0x2282f0 mark_slot_ranges(slot, size): claims a slot with a 16-byte rounded size and adds it to both range lists.
pub fn mark_slot_ranges(c: &mut Cpu) -> bool {
    let slot = c.arg(0);
    let size = c.arg(1).wrapping_add(0xf) & SIZE_MASK;
    let records = c.r32(TABLE + RECORDS);
    let claim = records.wrapping_add(CLAIM).wrapping_add(slot.wrapping_mul(4));
    if c.r32(claim) != 0 {
        return true;
    }
    c.w32(claim, size);
    let (start, end) = bounds(c, records, slot, BOUND_A);
    let list = c.r32(TABLE + LIST_A);
    add_size_to_list(c, list, start, end, size);
    let (start, end) = bounds(c, records, slot, BOUND_B);
    let list = c.r32(TABLE + LIST_B);
    add_size_to_list(c, list, start, end, size);
    true
}

/// Range of list entries owned by `slot`: from the previous slot's bound (0 for slot 0) up to this slot's bound.
fn bounds(c: &mut Cpu, records: u32, slot: u32, bound: u32) -> (u32, u32) {
    let end = c.r32(records.wrapping_add(slot.wrapping_mul(4)).wrapping_add(bound));
    let start = if slot == 0 {
        0
    } else {
        c.r32(records.wrapping_add(slot.wrapping_sub(1).wrapping_mul(4)).wrapping_add(bound))
    };
    (start, end)
}

/// For entries start..end of the list at `list`: clears the top bit of the first word, then adds `size`.
fn add_size_to_list(c: &mut Cpu, list: u32, start: u32, end: u32, size: u32) {
    // Retail uses signed slt for the loop test, so compare as i32.
    let mut i = start as i32;
    while i < end as i32 {
        let e = list.wrapping_add((i as u32).wrapping_mul(8));
        let v = c.r32(e) & TOP_BIT_CLEAR;
        c.w32(e, v.wrapping_add(size));
        i += 1;
    }
}
