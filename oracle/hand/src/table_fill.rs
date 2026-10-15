//! Rebuilds the five-entry (word, word) table at 0x1ba2b8 from the two seed words in the state block at 0x1b9df0.

use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// State block: seed words at +0x108 (a3) and +0x10c (t0).
const STATE: u32 = 0x001b_9df0;
/// Table of (word, word) entries, 8 bytes each.
const TABLE: u32 = 0x001b_a2b8;
/// Per-entry step of the first and second runs.
const STEP_A: u32 = 0x0001_1800;
/// Per-entry step of the tail run.
const STEP_B: u32 = 0x0004_f000;
/// The table is zero-padded up to this entry index.
const PAD_END: u32 = 5;
/// Value the retail code leaves in v0 (its lui delay slot) when no padding runs.
const V0_NO_PAD: u32 = 0x001c_0000;

/// Writes a 32-bit value into a GPR, sign-extended to 128 bits like the MIPS 32-bit ops.
fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.r[reg] = (v as i32 as i128) as u128;
}

/// Writes one table entry: the low word, then the flag word.
fn put_entry(c: &mut Cpu, index: u32, lo: u32, hi: u32) {
    let e = TABLE + 8 * index;
    c.w32(e, lo);
    c.w32(e + 4, hi);
}

/// 0x276550 fill_table(mode) -> none: rebuilds the table at 0x1ba2b8 from the seed words in the state block.
/// Mode 0 writes entry 0 and zero-pads entries 1..4; nonzero mode writes entries 0..4 and needs no padding.
pub fn fill_table(c: &mut Cpu) -> bool {
    let nonzero = c.arg(0) != 0;
    let mut seed_t0 = c.r32(STATE + 0x10c);
    let mut seed_a3 = c.r32(STATE + 0x108);

    // Mode picks the first run length, the extra entry after it, and the tail run length.
    let (first, extra, tail) = if nonzero { (2, 1, 2) } else { (1, 0, 0) };
    let split = first + extra;

    // Run 1: entries 0..first, word from seed_a3 stepping by STEP_A, flag 0.
    for i in 0..first {
        put_entry(c, i, seed_a3, 0);
        seed_a3 = seed_a3.wrapping_add(STEP_A);
    }
    let mut a1 = first;

    // Run 2: entries first..split, word from seed_t0 stepping by STEP_A, flag 0.
    if first < split {
        for i in first..split {
            put_entry(c, i, seed_t0, 0);
            seed_t0 = seed_t0.wrapping_add(STEP_A);
        }
        a1 = STEP_A;
    }

    // Run 3: entries split..end, word from seed_a3 stepping by STEP_B, flag 1.
    let end = split + tail;
    if tail > 0 {
        for i in split..end {
            put_entry(c, i, seed_a3, 1);
            seed_a3 = seed_a3.wrapping_add(STEP_B);
        }
        a1 = STEP_B;
    }

    // Run 4: zero-pad the remaining entries up to PAD_END.
    for i in end..PAD_END {
        put_entry(c, i, 0, 0);
    }
    let pad_end = end.max(PAD_END);
    let v0 = if end < PAD_END { 0 } else { V0_NO_PAD };

    set_gpr(c, 2, v0);
    set_gpr(c, 3, TABLE + 8 * pad_end);
    set_gpr(c, 4, pad_end);
    set_gpr(c, 5, a1);
    set_gpr(c, 6, end);
    set_gpr(c, 7, seed_a3);
    set_gpr(c, 8, seed_t0);
    set_gpr(c, 9, split);
    set_gpr(c, 10, tail);
    true
}
