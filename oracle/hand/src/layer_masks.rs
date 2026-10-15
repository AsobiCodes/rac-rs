//! Hand versions of two retail helpers from the camera/layer code: the per-table layer mask pass (0x2285d0) and the
//! camera bank swap (0x2287a8).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Table base in RAM (the layer table header; its word at +0x18 points at the per-index block array).
const TABLE: u32 = 0x0017_e5d8;
const BLOCK_PTR: u32 = 0x18;
const REC_PTR: u32 = 0x24;
const COUNT: u32 = 0x34;
const SLOT: u32 = 0x74;
/// Global word holding the mask seed (a signed value; the record field gets seed >> 8).
const MASK_SEED: u32 = 0x0015_ee88;
/// Size of one record in the record array.
const REC_STRIDE: u32 = 8;
/// Record field offsets: the halfword that receives seed >> 8, and the two byte weights.
const REC_SEED_OFF: u32 = 4;
const REC_LO_OFF: u32 = 6;
const REC_HI_OFF: u32 = 7;
/// Per-record helper (0x22f8f8), the one-time init helper for an unset slot (0x2282f0), and the record kind.
const RECORD_FN: u32 = 0x0022_f8f8;
const INIT_FN: u32 = 0x0022_82f0;
const RECORD_KIND: u32 = 0x1b;

/// Camera bank: the flag word and the source word are gp-relative; the banks are fixed RAM words.
const GP: usize = 28;
const GP_FLAG: u32 = 0x74f8;
const GP_SRC: u32 = 0x6ca8;
const BANK_A0: u32 = 0x0015_fa28;
const BANK_A1: u32 = 0x0015_fa2c;
const BANK_A2: u32 = 0x0015_fa30;
const BANK_A3: u32 = 0x0015_fa34;
const BANK_B1: u32 = 0x0015_ff5c;
const BANK_B2: u32 = 0x0015_ff60;
const BANK_B3: u32 = 0x0015_ff68;

/// Writes a 32-bit result into a scalar register, sign-extended as the MIPS 32-bit ops do.
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x2285d0 build_layer_masks(index, flags) -> void: for each record of the block at index, calls the record helper
/// with the seed shifted right by 8 and adds the record's weight bit (shifted by 2) to the seed.
pub fn build_layer_masks(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let index = c.arg(0);
    let flags = c.arg(2);
    let slot_addr = c.r32(TABLE + BLOCK_PTR).wrapping_add(index.wrapping_mul(4));
    if c.r32(slot_addr + SLOT) == 0 {
        h::call_keep(c, INIT_FN, &[0], &[]);
    }
    let blocks = c.r32(TABLE + BLOCK_PTR);
    let mut seed = c.r32(MASK_SEED);
    let first = if index == 0 { 0 } else { c.r32(blocks.wrapping_add((index - 1).wrapping_mul(4)) + COUNT) };
    let end = c.r32(blocks.wrapping_add(index.wrapping_mul(4)) + COUNT) as i32;
    let slot_ptr = blocks.wrapping_add(index.wrapping_mul(4));

    let mut i = first as i32;
    let mut recs_now = 0;
    while i < end {
        let recs = c.r32(TABLE + REC_PTR);
        let rec = recs.wrapping_add((i as u32).wrapping_mul(REC_STRIDE));
        let shifted = ((seed as i32) >> 8) as u32;
        let word0 = c.r32(rec);
        let lo = c.r8(rec + REC_LO_OFF) as u32;
        let hi = c.r8(rec + REC_HI_OFF) as u32;
        i += 1;
        let bit = 1u32 << (lo.wrapping_add(hi) & 31);
        h::call_keep(c, RECORD_FN, &[word0, shifted, RECORD_KIND, lo, hi, flags], &[]);
        seed = seed.wrapping_add(bit << 2);
        recs_now = c.r32(TABLE + REC_PTR);
        let stored = recs_now.wrapping_add(i.wrapping_sub(1) as u32 * REC_STRIDE + REC_SEED_OFF);
        c.w16(stored, shifted as u16);
    }
    if first as i32 >= end {
        // Loop never entered: retail leaves v0 at the slot address, v1 = 0, a0 = table, a1 = index * 4.
        set32(c, 2, slot_ptr);
        set32(c, 3, 0);
        set32(c, 4, TABLE);
        set32(c, 5, index.wrapping_mul(4));
    } else {
        // Last loop test leaves v0 = (i < end) and v1 = the record array pointer.
        set32(c, 2, (i < end) as u32);
        set32(c, 3, recs_now);
    }
    true
}

/// 0x2287a8 swap_camera_bank(flag) -> void: when the flag at gp-0x74f8 differs from a0, toggles it and swaps the
/// current camera words with the saved bank words (the first word takes gp-0x6ca8).
pub fn swap_camera_bank(c: &mut Cpu) -> bool {
    let want = c.arg(0);
    let gp = c.g(GP) as u32;
    let flag_addr = gp.wrapping_sub(GP_FLAG);
    let cur = c.r32(flag_addr);
    if want == cur {
        return true;
    }
    let src = c.r32(gp.wrapping_sub(GP_SRC));
    let flag = cur ^ 1;
    let (a0, a1, a2, a3) = (c.r32(BANK_A0), c.r32(BANK_A1), c.r32(BANK_A2), c.r32(BANK_A3));
    let (b1, b2, b3) = (c.r32(BANK_B1), c.r32(BANK_B2), c.r32(BANK_B3));
    c.w32(flag_addr, flag);
    c.w32(BANK_A0, src);
    c.w32(0x0015_ff58, a0);
    c.w32(BANK_A1, b1);
    c.w32(BANK_B1, a1);
    c.w32(BANK_A2, b2);
    c.w32(BANK_B2, a2);
    c.w32(BANK_B3, a3);
    c.w32(BANK_A3, b3);
    // Scratch registers as retail leaves them: v0 = old A2, v1 = old A3, a0 = old A1, a1 = old B3, a2 = flag,
    // a3 = old A0, t0 = old B2.
    set32(c, 2, a2);
    set32(c, 3, a3);
    set32(c, 4, a1);
    set32(c, 5, b3);
    set32(c, 6, flag);
    set32(c, 7, a0);
    set32(c, 8, b2);
    true
}
