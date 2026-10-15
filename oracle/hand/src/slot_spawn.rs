//! Slot-table spawn helpers (retail 0x256f08..0x2570d8). Both fill a record at 0x1516d0 (fields at +0x50..+0x68)
//! and hand the values to the spawner at 0x12ec08.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const SP: usize = 29;
const RA: usize = 31;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;
const T2: usize = 10;
const T3: usize = 11;
const T4: usize = 12;
const T5: usize = 13;
const T6: usize = 14;
const T7: usize = 15;

/// Spawner called with the record values (retail 0x12ec08).
const SPAWNER: u32 = 0x12ec08;
/// Callback address stored on the stack for the spawner (retail 0x258358).
const CALLBACK: u32 = 0x258358;
/// Record base (retail 0x1516d0): the spawn fields sit at +0x50..+0x68.
const RECORD: u32 = 0x0015_16d0;
/// Stack frame of both functions.
const FRAME: u32 = 0x30;

/// Table of 0x20-byte entries for spawn_slot_a, indexed by (a0 - 40000) * 8 (retail 0x137b80).
const SLOT_A_BASE: u32 = 0x0013_7b80;
/// Offset from the indexed slot-A entry to the pointer read by spawn_slot_a.
const SLOT_A_OFF: u32 = 0x13c0;
/// Table of words for spawn_slot_b, indexed by a0 * 4 + count * 0x258 (retail 0x11c178).
const SLOT_B_TABLE: u32 = 0x0011_c178;
/// Word holding the count multiplied into the spawn_slot_b index (retail 0x15ed88).
const SLOT_B_COUNT: u32 = 0x0015_ed88;
/// Stride multiplied by the count in spawn_slot_b.
const SLOT_B_STRIDE: i64 = 0x258;
/// Byte flag that selects the sign-extended a2 path in spawn_slot_b (retail 0x15ee1c).
const SLOT_B_FLAG: u32 = 0x0015_ee1c;

fn set(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

fn set_hi_lo(c: &mut Cpu, product: i64) {
    c.lo = c.lo & !(u64::MAX as u128) | (product as i32 as i64 as u64) as u128;
    c.hi = c.hi & !(u64::MAX as u128) | ((product >> 32) as i32 as i64 as u64) as u128;
}

/// 0x256f08 spawn_slot_a(a0, a1, a2) -> (): looks up the slot-A entry for a0 (a0 - 40000, 8 bytes each); when the
/// entry is non-zero and the record at 0x1516d0 is free, fills the record from a0, a1, a2 and calls the spawner.
pub fn spawn_slot_a(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let (a0, a1, a2) = (c.g(A0) as u32, c.g(A1) as u32, c.g(A2) as u32);
    let sp0 = c.g(SP) as u32;
    let sp = sp0.wrapping_sub(FRAME);
    set(c, SP, sp);
    c.mem.wr128(sp + 0x20, c.r[RA]);
    let off = a0.wrapping_add(0xffff_63c0) << 3;
    let entry = SLOT_A_BASE.wrapping_add(off);
    set(c, V0, off);
    set(c, V1, entry);
    set(c, T5, a0);
    set(c, T7, a1);
    set(c, T6, a2);
    let slot = c.r32(entry.wrapping_add(SLOT_A_OFF));
    set(c, A0, slot);
    if slot == 0 {
        set(c, SP, sp0);
        return true;
    }
    set(c, V0, 0x0015_0000);
    set(c, T4, RECORD);
    let flag = c.r32(RECORD + 0x50);
    set(c, V1, flag);
    if flag != 0 {
        set(c, SP, sp0);
        return true;
    }
    let tbl = RECORD + 0x50;
    set(c, V1, tbl);
    set(c, A1, 1);
    set(c, A3, 10);
    set(c, T1, 0xbb80);
    set(c, T2, 0x21);
    set(c, V0, 0xffff_ffff);
    let t0 = ((a2 << 16) as i32 >> 16) as u32;
    set(c, T0, t0);
    c.w16(RECORD + 0x5a, 1);
    c.w32(RECORD + 0x64, 10);
    c.w32(RECORD + 0x68, 0xbb80);
    c.w32(sp, 0x21);
    c.w32(sp + 8, CALLBACK);
    c.w32(RECORD + 0x50, 0xffff_ffff);
    c.w16(RECORD + 0x54, a0 as u16);
    c.w16(RECORD + 0x58, a1 as u16);
    c.w16(RECORD + 0x56, a2 as u16);
    c.w16(RECORD + 0x60, 0);
    c.mem.wr64(sp + 0x10, tbl as u64);
    set(c, SP, sp);
    h::call_keep(c, SPAWNER, &[slot, 0, 0, 0, t0, 0, 2, 0], &[]);
    set(c, SP, sp0);
    true
}

/// 0x256fe0 spawn_slot_b(a0, a1, a2) -> (): indexes the word table at 0x11c178 by a0 and the count at 0x15ed88; when
/// the entry is non-zero and the record at 0x1516d0 is free, fills the record from a0, a1, a2 and calls the spawner.
pub fn spawn_slot_b(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let (a0, a1, a2) = (c.g(A0) as u32, c.g(A1) as u32, c.g(A2) as u32);
    let sp0 = c.g(SP) as u32;
    let sp = sp0.wrapping_sub(FRAME);
    set(c, SP, sp);
    let count = c.r32(SLOT_B_COUNT) as i32 as i64;
    set(c, A3, SLOT_B_STRIDE as u32);
    let product = count * SLOT_B_STRIDE;
    set_hi_lo(c, product);
    let lo = product as i32 as u32;
    set(c, V0, lo);
    set(c, T0, a0);
    set(c, A0, SLOT_B_TABLE);
    let v1 = (a0 << 2).wrapping_add(lo);
    set(c, V1, v1);
    let entry = v1.wrapping_add(SLOT_B_TABLE);
    set(c, T3, entry);
    c.mem.wr128(sp + 0x20, c.r[RA]);
    set(c, T1, a2);
    set(c, T2, a1);
    let slot = c.r32(entry);
    set(c, V0, slot);
    if slot == 0 {
        set(c, SP, sp0);
        return true;
    }
    set(c, V0, 0x0015_0000);
    set(c, A3, RECORD);
    let flag = c.r32(RECORD + 0x50);
    set(c, V1, flag);
    if flag != 0 {
        set(c, SP, sp0);
        return true;
    }
    set(c, V0, 0xffff_ffff);
    let fl = c.r8(SLOT_B_FLAG) as u32;
    set(c, A2, fl);
    set(c, V1, 1);
    set(c, A0, 10);
    set(c, A1, 0xbb80);
    c.w32(RECORD + 0x50, 0xffff_ffff);
    c.w16(RECORD + 0x5a, 1);
    c.w16(RECORD + 0x54, a0 as u16);
    c.w16(RECORD + 0x58, a1 as u16);
    c.w32(RECORD + 0x64, 10);
    c.w32(RECORD + 0x68, 0xbb80);
    c.w16(RECORD + 0x60, 0);
    c.w16(RECORD + 0x56, a2 as u16);
    let t0 = if fl == 0 {
        0
    } else {
        set(c, V0, a2 << 16);
        ((a2 << 16) as i32 >> 16) as u32
    };
    set(c, T0, t0);
    set(c, V0, 0x0015_1720);
    set(c, A1, CALLBACK);
    let slot = c.r32(entry);
    set(c, V1, 0x21);
    c.w32(sp, 0x21);
    c.w32(sp + 8, CALLBACK);
    c.mem.wr64(sp + 0x10, 0x0015_1720u64);
    set(c, SP, sp);
    h::call_keep(c, SPAWNER, &[slot, 0, 0, 0, t0, 0, 2, 0], &[]);
    set(c, SP, sp0);
    true
}
