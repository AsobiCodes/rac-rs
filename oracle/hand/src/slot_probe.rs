//! Slot search helpers: probe the candidates around the current slot (0x2391a8) and map a slot id to its index
//! in the zero-terminated id table (0x2392e0).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Base of the slot state block; the current slot id is the word at +0x224.
const STATE_BASE: u32 = 0x18_42f0;
const STATE_CUR: u32 = 0x224;
/// Word holding the pointer to the zero-terminated slot-id table (20 entries max).
const TABLE_PTR_SLOT: u32 = 0x16_0220;
/// Byte flag that selects the bank offset added to slot ids.
const BANK_FLAG: u32 = 0x13_74e1;
const BANK_OFFSET: u32 = 0x100;
/// gp-relative word holding the pointer to the candidate-value table.
const GP_CANDIDATE_OFS: i32 = -0x69e0;
/// Validator called on each candidate; returns -1 when the candidate is rejected.
const PROBE: u32 = 0x23_9160;
const TABLE_LEN: i32 = 0x14;
const NOT_FOUND: u32 = u32::MAX;
/// The alternating scan stops when the step counter reaches this value.
const SCAN_END: i32 = 4;

/// 0x2391a8 probe_slot_alt() -> slot: probe the current slot, then its neighbours in the id table
/// (alternating -1, +1, -2, +2 ..) until the validator accepts one; returns that candidate, or -1.
pub fn probe_slot_alt(c: &mut Cpu) -> bool {
    let bank = if c.r8(BANK_FLAG) != 0 { BANK_OFFSET } else { 0 };
    let cur = c.r32(STATE_BASE + STATE_CUR);
    let first = cur.wrapping_add(bank);
    if h::call_keep(c, PROBE, &[first], &[]).0 == NOT_FOUND {
        c.ret(first);
        return true;
    }
    let cur = c.r32(STATE_BASE + STATE_CUR);
    let mut s2: i32 = 0;
    if (cur as i32) < TABLE_LEN {
        let table = c.r32(TABLE_PTR_SLOT);
        if c.r32(table) != cur {
            let mut k: u32 = 0;
            while c.r32(table.wrapping_add(4 * k)) != cur {
                k += 1;
            }
            s2 = k as i32;
        }
    }
    let mut s1: i32 = 1;
    loop {
        let idx = s2 + s1;
        if (0..TABLE_LEN).contains(&idx) {
            let gp_slot = (c.g(28) as u32).wrapping_add(GP_CANDIDATE_OFS as u32);
            let table = c.r32(gp_slot);
            let val = c.r32(table.wrapping_add(idx as u32 * 4));
            if val != 0 {
                let cand = val.wrapping_add(bank);
                if h::call_keep(c, PROBE, &[cand], &[]).0 == NOT_FOUND {
                    c.ret(cand);
                    return true;
                }
            }
        }
        s1 = if s1 < 1 { 1 - s1 } else { -s1 };
        if s1 == SCAN_END {
            break;
        }
    }
    c.ret(NOT_FOUND);
    true
}

/// 0x2392e0 find_slot_index(id) -> index: position of id in the slot-id table (scan stops at a zero entry
/// or after 20 entries), or -1.
pub fn find_slot_index(c: &mut Cpu) -> bool {
    let id = c.arg(0);
    let table = c.r32(TABLE_PTR_SLOT);
    if c.r32(table) == 0 {
        c.ret(NOT_FOUND);
        return true;
    }
    for k in 0..TABLE_LEN as u32 {
        if c.r32(table.wrapping_add(4 * k)) == id {
            c.ret(k);
            return true;
        }
        if c.r32(table.wrapping_add(4 * (k + 1))) == 0 {
            break;
        }
    }
    c.ret(NOT_FOUND);
    true
}
