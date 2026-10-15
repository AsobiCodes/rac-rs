//! Mask-gated primitive emitter. Source bytes are read in groups of three index bytes. A group whose three mask-table
//! bytes AND to nonzero is skipped; otherwise three 0x20-byte records are written at the packet cursor. The cursor
//! lives in a gp-relative slot. After all groups, a header of two quadwords is written at the packet start.

use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// gp-relative slot that holds the packet write cursor.
const CURSOR_SLOT_GP: i32 = -0x5ac0;
/// Bytes per emitted record: a doubleword pair, then a 16-byte block.
const REC_SIZE: u32 = 0x20;
/// Row stride of the record and mask tables, as a shift (index << 4).
const ROW_SHIFT: u32 = 4;
/// The mask table is read at rec_tab + 15 + (index << 4).
const MASK_BIAS: u32 = 0xf;
/// Index bytes per group; each group gets one pass of three records when it is not masked out.
const GROUP_LEN: u32 = 3;
/// Header tag words. The low word carries the record count minus one.
const HDR_LO: u32 = 0x1000_0000;
const HDR_HI: u32 = 0x5000_0000;
const HDR_EOP: u32 = 0x8000;
/// Register value written by each record (at +0x18), and by the trailing tag.
const REC_REG_AD: u64 = 5;
const TAIL_REG_AD: u64 = 0xe;
/// Quadword count of a pass with no records (only the reserved header slot): retail skips the header and cursor update.
const NO_RECORD_QUADS: u32 = 2;

// MIPS register numbers used by the retail body.
const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T6: usize = 14;
const T7: usize = 15;
const T8: usize = 24;
const T9: usize = 25;
const GP: usize = 28;

/// Sets a register to a 32-bit result, sign-extended the way MIPS 32-bit ops leave it.
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Writes a 64-bit doubleword at `p`.
fn store64(c: &mut Cpu, p: u32, v: u64) {
    c.w32(p, v as u32);
    c.w32(p.wrapping_add(4), (v >> 32) as u32);
}

/// Writes one 0x20-byte record at `out` for table index `idx`. Returns the record's low halfword (left in v1) and the
/// address of its table row (left in v0).
fn emit_record(c: &mut Cpu, out: u32, idx_tab: u32, rec_tab: u32, idx: u32) -> (u32, u32) {
    let word = c.r32(idx_tab.wrapping_add(idx << 2)) as i32 as i64 as u64;
    store64(c, out, word);
    store64(c, out.wrapping_add(8), 1);
    let row = rec_tab.wrapping_add(idx << ROW_SHIFT);
    let lo = c.r32(row) & 0xffff;
    let hi = c.r32(row.wrapping_add(4)) & 0xffff;
    c.w32(out.wrapping_add(0x10), (hi << 16) | lo);
    c.w32(out.wrapping_add(0x14), 0);
    store64(c, out.wrapping_add(0x18), REC_REG_AD);
    (lo, row)
}

/// 0x27cb20 mask_prim_emit(count, src, idx_tab, rec_tab) -> v0: for each of `count` groups of three index bytes at src,
/// skips the group if its three mask bytes AND to nonzero, else emits three records. If any record was emitted, writes
/// the two-quadword header at the packet start and stores the new cursor in the gp slot. Returns the header count.
pub fn mask_prim_emit(c: &mut Cpu) -> bool {
    let slot = (c.g(GP) as u32).wrapping_add(CURSOR_SLOT_GP as u32);
    let start = c.r32(slot);
    // The first 0x20 bytes are reserved for the header; records start after them (retail 0x27cb2c).
    let mut out = start.wrapping_add(0x20);
    let mut count = c.g(A0) as u32;
    let mut src = c.g(A1) as u32;
    let idx_tab = c.g(A2) as u32;
    let rec_tab = c.g(A3) as u32;
    let mask_tab = rec_tab.wrapping_add(MASK_BIAS);

    // Scratch registers as the last group leaves them.
    let mut at = 0u32;
    let mut v0 = 0u32;
    let mut v1 = 0u32;
    let mut t6 = 0u32;
    let mut t7 = 0u32;

    loop {
        count = count.wrapping_sub(1);
        let group = src;
        let mut masks = [0u32; 3];
        for (k, m) in masks.iter_mut().enumerate() {
            let idx = c.r8(group.wrapping_add(k as u32)) as u32;
            *m = c.r8(mask_tab.wrapping_add(idx << ROW_SHIFT)) as u32;
        }
        let bits = masks[0] & masks[1] & masks[2];
        src = src.wrapping_add(4);
        t6 = GROUP_LEN;
        t7 = group;
        if bits != 0 {
            at = bits;
            v0 = masks[2];
            v1 = mask_tab;
        } else {
            for k in 0..GROUP_LEN {
                let idx = c.r8(group.wrapping_add(k)) as u32;
                let (lo, row) = emit_record(c, out, idx_tab, rec_tab, idx);
                out = out.wrapping_add(REC_SIZE);
                v1 = lo;
                v0 = row;
            }
            at = REC_REG_AD as u32;
            t6 = 0;
            t7 = group.wrapping_add(GROUP_LEN);
        }
        if count == 0 {
            break;
        }
    }

    // Quadword count of everything written in this pass.
    let quads = out.wrapping_sub(start) >> 4;
    let last_field = quads.wrapping_sub(1);
    if quads == NO_RECORD_QUADS {
        v0 = last_field;
    } else {
        c.w32(start, HDR_LO | last_field);
        c.w32(start.wrapping_add(4), 0);
        c.w32(start.wrapping_add(8), 0);
        c.w32(start.wrapping_add(0xc), HDR_HI | last_field);
        let end_count = last_field.wrapping_sub(1);
        c.w32(start.wrapping_add(0x10), end_count | HDR_EOP);
        c.w32(start.wrapping_add(0x14), HDR_LO);
        store64(c, start.wrapping_add(0x18), TAIL_REG_AD);
        c.w32(slot, out);
        v0 = end_count;
        at = TAIL_REG_AD as u32;
    }

    set32(c, A0, count);
    set32(c, A1, src);
    set32(c, AT, at);
    set32(c, V0, v0);
    set32(c, V1, v1);
    set32(c, A2, idx_tab);
    set32(c, A3, rec_tab);
    set32(c, T6, t6);
    set32(c, T7, t7);
    set32(c, T8, out);
    set32(c, T9, start);
    true
}
