//! Slot range fix-up for the per-slot table at 0x17e5d8 (Eudora overlay rec/l4_05).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Block of table pointers and counts (retail data).
const BLOCK: u32 = 0x0017_e5d8;
/// Word at BLOCK holding the slot table base.
const OFF_TABLE: u32 = 0x18;
/// Word at BLOCK holding the 8-byte entry list used by the halfword clear (slot 0 only).
const OFF_HALF_LIST: u32 = 0x24;
/// Word at BLOCK holding the 8-byte entry list used by the pair fix-up.
const OFF_PAIR_LIST: u32 = 0x28;
/// Per-slot field: entry range start (previous slot's end is used as the start).
const FIELD_START: u32 = 0x14;
/// Per-slot field: entry range end, and the count when slot 0 is requested.
const FIELD_END: u32 = 0x34;
/// Per-slot field: weight subtracted from the first word of each entry.
const FIELD_WEIGHT: u32 = 0x74;
/// Bit OR-ed into the first word of each entry after the subtraction.
const HIGH_BIT: u32 = 0x8000_0000;

/// Table word address for slot `idx` at field `off`.
fn slot_word(c: &mut Cpu, idx: u32, off: u32) -> u32 {
    let tab = c.r32(BLOCK + OFF_TABLE);
    tab.wrapping_add(idx << 2).wrapping_add(off)
}

/// Clears the halfword at +4 of every entry of the slot-0 list, for the count stored at tab+0x34.
fn clear_halves(c: &mut Cpu) {
    let count = |c: &mut Cpu| {
        let tab = c.r32(BLOCK + OFF_TABLE);
        c.r32(tab.wrapping_add(FIELD_END)) as i32
    };
    let mut k: u32 = 0;
    if (k as i32) < count(c) {
        loop {
            let list = c.r32(BLOCK + OFF_HALF_LIST);
            c.w16(list.wrapping_add(k << 3).wrapping_add(4), 0);
            k = k.wrapping_add(1);
            if (k as i32) >= count(c) {
                break;
            }
        }
    }
}

/// Entries `from..to` of the list at `list_off`: first word minus `weight`, with HIGH_BIT set.
fn fix_pairs(c: &mut Cpu, list_off: u32, from: i32, to: i32, weight: u32) {
    let mut j = from;
    while j < to {
        let list = c.r32(BLOCK + list_off);
        let a = list.wrapping_add((j as u32) << 3);
        let w = c.r32(a).wrapping_sub(weight) | HIGH_BIT;
        c.w32(a, w);
        j += 1;
    }
}

/// 0x228450 slot_range_fixup(idx) -> v0 = slot address: fixes the entry ranges of slot idx and clears its weight.
pub fn slot_range_fixup(c: &mut Cpu) -> bool {
    let idx = c.arg(0);
    let slot = {
        let tab = c.r32(BLOCK + OFF_TABLE);
        tab.wrapping_add(idx << 2)
    };
    let weight = c.r32(slot.wrapping_add(FIELD_WEIGHT));
    // Previous slot's fields give the range starts (slot 0 starts at 0).
    let prev = slot_word(c, idx.wrapping_sub(1), 0);
    let (start1, start2) = if idx == 0 {
        clear_halves(c);
        (0, 0)
    } else {
        (c.r32(prev.wrapping_add(FIELD_START)), c.r32(prev.wrapping_add(FIELD_END)))
    };
    let end1 = c.r32(slot.wrapping_add(FIELD_START)) as i32;
    fix_pairs(c, OFF_PAIR_LIST, start1 as i32, end1, weight);
    let end2 = c.r32(slot.wrapping_add(FIELD_END)) as i32;
    fix_pairs(c, OFF_HALF_LIST, start2 as i32, end2, weight);
    c.w32(slot.wrapping_add(FIELD_WEIGHT), 0);
    c.ret(slot);
    true
}
