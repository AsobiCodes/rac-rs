//! Small level-state helpers: a slot store keyed by the current level, a jump-table flag
//! lookup, and an empty hook.

use crate::h::call_keep;
use crate::ps2::*;
use ee_oracle::cpu::{w4, Cpu};

/// Word in RAM holding the current level index.
const LEVEL_INDEX: u32 = 0x15_ED84;
/// Per-level word table; entry = table + level * 4 holds a signed offset.
const LEVEL_OFFSET_TABLE: u32 = 0x1C_4538;
/// Base of the slot array that the offset-adjusted index writes into.
const SLOT_BASE: u32 = 0x17_92B8;
/// Level count threshold: levels below this use the offset table.
const LEVEL_LIMIT: i32 = 0x13;

/// Jump table of ten handler addresses indexed by the high half of a0.
const DISPATCH_TABLE: u32 = 0x1D_AAE0;
/// Per-case byte-flag bases (case 1, 2, 3, 6), indexed by the a1 argument.
const FLAG_BASE_1: u32 = 0x13_DD40;
const FLAG_BASE_2: u32 = 0x13_D4C0;
const FLAG_BASE_3: u32 = 0x13_D4E8;
const FLAG_BASE_6: u32 = 0x13_D388;
/// Record table (16-byte records, word at +0xc) used by cases 4 and 5.
const RECORD_TABLE: u32 = 0x13_D5B0;
/// Limit on the a1 record index for cases 4 and 5.
const RECORD_LIMIT: i32 = 0x79;
/// Byte-flag table used by case 8, addressed by the two halves of a1.
const FLAG_BASE_8: u32 = 0x14_BEC0;
/// Word offset of the record payload inside a 16-byte record.
const RECORD_PAYLOAD: u32 = 0xC;
/// Number of valid dispatch cases.
const CASE_COUNT: u32 = 10;

/// Sets GPR `r` to a 32-bit value, sign-extended as the MIPS 32-bit ops leave it.
fn set_gpr(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x23ccf0 stage_slot_store(a0, a1): when the level index is below 0x13, writes a0 into the slot
/// at (a1 + offset[level]) * 4 + 0x1792b8; otherwise writes nothing.
pub fn stage_slot_store(c: &mut Cpu) -> bool {
    let a0 = c.arg(0);
    let a1 = c.arg(1);
    let level = c.r32(LEVEL_INDEX);
    if (level as i32) < LEVEL_LIMIT {
        let entry = LEVEL_OFFSET_TABLE.wrapping_add(level << 2);
        let offset = c.r32(entry);
        let index = a1.wrapping_add(offset);
        let slot = SLOT_BASE.wrapping_add(index << 2);
        c.w32(slot, a0);
        set_gpr(c, 2, slot);
        set_gpr(c, 3, offset);
        set_gpr(c, 4, entry);
        set_gpr(c, 5, SLOT_BASE);
        set_gpr(c, 6, index);
    } else {
        set_gpr(c, 2, 0);
        set_gpr(c, 3, level);
        set_gpr(c, 6, a1);
    }
    set_gpr(c, 7, a0);
    true
}

/// 0x23d148 nop_hook(): empty hook; returns at once.
pub fn nop_hook(_c: &mut Cpu) -> bool {
    true
}

/// 0x240328 dispatch_flag(a0, a1) -> v0: case on the high half of a0 (0..9) through a jump table;
/// each case returns a 0/1 flag read from a level table, a record field, or a callback.
/// Indices 10 and up return 0.
pub fn dispatch_flag(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let sp = c.g(29) as u32;
    c.wv(sp.wrapping_sub(0x10), V(w4(c.r[31])));
    let a0 = c.arg(0) << 16;
    let idx = ((a0 as i32) >> 16) as u32;
    let a1 = c.arg(1);
    let mut v0: u32 = 0;
    let mut v1 = idx;
    let mut a0_out = a0;
    if idx < CASE_COUNT {
        v1 = (idx << 2).wrapping_add(DISPATCH_TABLE);
        a0_out = c.r32(v1);
        match idx {
            0 => v0 = 1,
            1 => {
                v1 = a1.wrapping_add(FLAG_BASE_1);
                v0 = (c.r8(v1) != 0) as u32;
            }
            2 => {
                v1 = a1.wrapping_add(FLAG_BASE_2);
                v0 = (c.r8(v1) != 0) as u32;
            }
            3 => {
                v1 = a1.wrapping_add(FLAG_BASE_3);
                v0 = (c.r8(v1) != 0) as u32;
            }
            4 => {
                if (a1 as i32) < RECORD_LIMIT {
                    v1 = RECORD_TABLE.wrapping_add(a1 << 4);
                    v0 = (c.r32(v1.wrapping_add(RECORD_PAYLOAD)) != 0) as u32;
                }
            }
            5 => {
                if (a1 as i32) < RECORD_LIMIT {
                    v1 = RECORD_TABLE.wrapping_add(a1 << 4);
                    let payload = c.r32(v1.wrapping_add(RECORD_PAYLOAD)) as i32;
                    v0 = (payload >= 2) as u32;
                }
            }
            6 => {
                v1 = a1.wrapping_add(FLAG_BASE_6);
                v0 = (c.r8(v1) != 0) as u32;
            }
            7 => {
                // Retail calls the function pointer in a1 with a0 = the table entry.
                c.s(29, sp.wrapping_sub(0x10) as i32 as i64 as u64);
                let (r, _) = call_keep(c, a1, &[a0_out], &[]);
                c.s(29, sp as i32 as i64 as u64);
                set_gpr(c, 2, (r != 0) as u32);
                return true;
            }
            8 => {
                let hi = ((a1 as i32) >> 16) as u32;
                a0_out = FLAG_BASE_8;
                v1 = (a1 & 0xFFFF).wrapping_add(hi << 2).wrapping_add(FLAG_BASE_8);
                v0 = (c.r8(v1) != 0) as u32;
            }
            _ => {}
        }
    } else {
        v1 = idx;
    }
    set_gpr(c, 2, v0);
    set_gpr(c, 3, v1);
    set_gpr(c, 4, a0_out);
    true
}
