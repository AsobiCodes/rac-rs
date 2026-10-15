//! Moby slot release and two mode-indexed float-to-int table writers of the Eudora overlay.

use crate::actor;
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Base of the per-slot array (0x50 bytes per slot, indexed by a0).
const SLOT_TABLE: u32 = 0x0013_F350;
const SLOT_STRIDE: u32 = 0x50;
/// Offsets inside a slot.
const SLOT_MOBY: u32 = 0x1090;
const SLOT_SECOND: u32 = 0x1094;
const SLOT_VALUE: u32 = 0x10A4;
const SLOT_STATE: u32 = 0x10B4;
const SLOT_EXTRA: u32 = 0x10B8;
/// State value meaning "already released".
const STATE_RELEASED: u32 = 3;
/// Slot index that keeps its second moby pointer.
const SLOT_KEEP_SECOND: u32 = 3;
/// Moby kind bytes that mark an entry as already freed (no release callback).
const KIND_FREED_A: u8 = 0xFE;
const KIND_FREED_B: u8 = 0xFD;
/// Moby +0x74: optional per-moby release callback.
const MOBY_CALLBACK: u32 = 0x74;
/// Moby +0x20: kind byte.
const MOBY_KIND: u32 = 0x20;
/// Argument to float_to_int_scaled used by both table writers.
const SCALE_INPUT: u32 = 0x46;
/// Destination halfword of the first writer (second slot of the table) and of the second writer.
const TABLE_A_CELL: u32 = 0x0013_F52C;
const TABLE_A_TAG: u32 = 0x0014_161E;
const TABLE_B_CELL: u32 = 0x0013_F52A;

/// 0x202d50 slot_release(index, value): stores value into the slot; if its moby is still live, marks it released and
/// runs the moby's callback (unless the kind byte says it is already freed); then clears the slot's moby pointers,
/// freeing them (the second pointer is kept for slot 3).
pub fn slot_release(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let index = c.arg(0);
    let value = c.arg(1);
    let slot = SLOT_TABLE.wrapping_add(index.wrapping_mul(SLOT_STRIDE));
    c.w32(slot + SLOT_VALUE, value);
    if c.r32(slot + SLOT_STATE) != STATE_RELEASED {
        let moby = c.r32(slot + SLOT_MOBY);
        if moby != 0 {
            c.w32(slot + SLOT_STATE, STATE_RELEASED);
            let kind = c.r8(moby + MOBY_KIND);
            if kind != KIND_FREED_A && kind != KIND_FREED_B {
                let callback = c.r32(moby + MOBY_CALLBACK);
                if callback != 0 {
                    let _ = h::call(c, callback, &[moby], &[]);
                }
            }
        }
    }
    c.w32(slot + SLOT_STATE, 0);
    c.w32(slot + SLOT_EXTRA, 0);
    let moby = c.r32(slot + SLOT_MOBY);
    if moby != 0 {
        let _ = h::run(c, actor::free_moby, &[moby], &[]);
        c.w32(slot + SLOT_MOBY, 0);
    }
    let second = c.r32(slot + SLOT_SECOND);
    if second != 0 && index != SLOT_KEEP_SECOND {
        let _ = h::run(c, actor::free_moby, &[second], &[]);
        c.w32(slot + SLOT_SECOND, 0);
    }
    true
}

/// Mode indices (mode - 9) that write the scaled value through the first table writer.
fn writes_table_a(idx: u32) -> bool {
    matches!(idx, 0 | 2 | 4 | 7 | 14)
}

/// Mode indices (mode - 9) that write the scaled value through the second table writer.
fn writes_table_b(idx: u32) -> bool {
    matches!(idx, 0 | 2 | 4 | 5 | 6 | 7 | 9 | 10 | 13 | 14 | 17)
}

/// 0x202e28 mode_table_a(mode): for modes 9, 11, 13, 16, 23 stores a scaled int (float_to_int_scaled(70)) into
/// the halfword at 0x13f52c, and writes the tag 3 to the halfword at 0x14161e first; other modes do nothing.
pub fn mode_table_a(c: &mut Cpu) -> bool {
    let idx = c.arg(0).wrapping_sub(9);
    if idx < 15 && writes_table_a(idx) {
        c.w16(TABLE_A_TAG, 3);
        let (scaled, _) = h::run(c, mathx::float_to_int_scaled, &[SCALE_INPUT], &[]);
        c.w16(TABLE_A_CELL, scaled as u16);
    }
    true
}

/// 0x202e88 mode_table_b(mode): for modes in the listed set (mode - 9 in 0..18) stores float_to_int_scaled(70) into
/// the halfword at 0x13f52a; other modes do nothing.
pub fn mode_table_b(c: &mut Cpu) -> bool {
    let idx = c.arg(0).wrapping_sub(9);
    if idx < 18 && writes_table_b(idx) {
        let (scaled, _) = h::run(c, mathx::float_to_int_scaled, &[SCALE_INPUT], &[]);
        c.w16(TABLE_B_CELL, scaled as u16);
    }
    true
}
