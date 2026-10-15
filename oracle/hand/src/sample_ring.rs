//! Ring-buffer sample logger (retail 0x237f78): takes the log2 of two sizes, keeps a running step value in RAM and
//! appends one 16-byte record to a 64-entry table while the table has room.

use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Running step accumulator in RAM (read, advanced and written back on every call).
const STEP_GLOBAL: u32 = 0x0015_EE74;
/// Number of records written so far (the table index for the next record).
const COUNT_GLOBAL: u32 = 0x0015_F418;
/// Base of the 64-entry record table, 16 bytes per record.
const TABLE: u32 = 0x0016_CE80;
const TABLE_LEN: i32 = 0x40;
/// Step added to the accumulator on every call, and stored in the frame as the first local.
const STEP: u32 = 0x400;
/// Offset added to the object pointer to get the new record owner pointer (s0 + 0x420).
const OWNER_BIAS: u32 = 0x420;
/// Bytes of the retail stack frame.
const FRAME: u32 = 0x80;
/// Frame slots: saved s0 and ra, and the locals the retail code writes.
const SAVE_S0: u32 = 0x60;
const SAVE_RA: u32 = 0x70;
const LOCAL_R1: u32 = 0x00;
const LOCAL_OWNER: u32 = 0x04;
const LOCAL_STEP: u32 = 0x14;
const LOCAL_R2: u32 = 0x50;
const LOCAL_R1_COPY: u32 = 0x4c;

/// 0x237f78 ring_push(obj) -> v0: logs log2(obj.a) and log2(obj.b) with the running step; returns the packed word
pub fn ring_push(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(c, FRAME, &[(16, SAVE_S0, h::SQ), (31, SAVE_RA, h::SQ)], |c, sp| {
        // Retail's jal delay slots store v0 before each call: sp+0 gets obj+0x20, sp+0x4c gets the first log.
        c.w32(sp + LOCAL_STEP, STEP);
        c.w32(sp + LOCAL_R1, obj.wrapping_add(0x20));
        let a = c.r32(obj.wrapping_add(8));
        let r1 = h::run(c, mathx::log2_floor, &[a], &[]).0;
        c.w32(sp + LOCAL_R1_COPY, r1);
        let a = c.r32(obj.wrapping_add(0xc));
        let r2 = h::run(c, mathx::log2_floor, &[a], &[]).0;
        c.w32(sp + LOCAL_R2, r2);

        // Packed word: shifted step bits, the two log values, and the table flag bit.
        let step_old = c.r32(STEP_GLOBAL) as i32;
        let step_new = step_old.wrapping_add(STEP as i32);
        let r1i = r1 as i32;
        let shift_a = (r1i.wrapping_sub(6)).max(0) as u32 & 31;
        let bit_a = (1u32 << shift_a) as i32 as i64 as u64;
        let t4 = (step_old >> 8) as i64;
        let t3 = (step_new >> 8) as i64;
        let mut packed = (t3 as u64) | (bit_a << 14);
        packed |= (r1 as u64) << 26 | 0x0130_0000;
        packed |= (r2 as u64) << 30;
        let shift_b = r1i.wrapping_add(r2 as i32) as u32 & 31;
        let bit_b = (1u32 << shift_b) as i32;
        c.w32(STEP_GLOBAL, step_new.wrapping_add(bit_b) as u32);
        packed |= ((t4 as u64) << 37) | (0x8000u64 << 19);
        packed |= 1u64 << 63;
        c.s(2, packed);

        let count = c.r32(COUNT_GLOBAL) as i32;
        let owner = obj.wrapping_add(OWNER_BIAS);
        c.w32(sp + LOCAL_OWNER, owner);
        if count < TABLE_LEN {
            let e = TABLE.wrapping_add((count as u32) << 4);
            c.w32(e, obj.wrapping_add(0x20));
            c.w16(e + 4, 0);
            c.w16(e + 6, t4 as u16);
            c.w32(e + 8, owner);
            c.w8(e + 0xc, r1 as u8);
            c.w8(e + 0xd, r2 as u8);
            c.w16(e + 0xe, t3 as u16);
            c.w32(COUNT_GLOBAL, count.wrapping_add(1) as u32);
        }
    });
    true
}
