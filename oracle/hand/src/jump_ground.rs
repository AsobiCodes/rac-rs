//! Jump pose save and scaled step chain (retail 0x2b1630..0x2b1664 and 0x2b21f8..0x2b2294).
//! The save block copies floats and a flag from the per-level table at 0x13f350 into the agent block (a0).

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const AT: usize = 1;

/// Per-level table the jump save block reads from.
const SAVE_TABLE: u32 = 0x0013_f350;
const SAVE_SRC_A: u32 = 0x1f20;
const SAVE_SRC_B: u32 = 0x1f24;
const SAVE_SRC_C: u32 = 0x1f28;
const SAVE_SRC_FLAG: u32 = 0x1f3f;
const SAVE_SRC_D: u32 = 0x1f40;


/// Vector helpers used by the step chain.
const BASIS_FN: u32 = 0x001f_1840;
const INT_TO_FLOAT: u32 = 0x001f_27d8;
const VEC_SCALE: u32 = 0x001f_1898;
const STEP_FN: u32 = 0x001f_1810;

const ONE: u32 = 0x3f80_0000;

/// Float argument registers f12..f15 as they were on entry (retail leaves them alone).
fn float_args(c: &mut Cpu) -> [F; 4] {
    [c.farg(0), c.farg(1), c.farg(2), c.farg(3)]
}

/// Puts f12..f15 back to the entry values.
fn restore_float_args(c: &mut Cpu, saved: [F; 4]) {
    for (i, v) in saved.iter().enumerate() {
        c.set_farg(i, *v);
    }
}

/// Sets a general register to a sign-extended 32-bit value.
fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Copies the jump pose block into the agent at a0 (floats at +0x44/+0x50/+0x54/+0x58, flag at +0x68).
/// Returns nothing; leaves v0 at the table base and v1 at the flag.
pub fn save_jump_pose(c: &mut Cpu) -> bool {
    let fa = float_args(c);
    let agent = c.arg(0);
    let a = c.rf(SAVE_TABLE + SAVE_SRC_A);
    c.wf(agent + 0x50, a);
    let b = c.rf(SAVE_TABLE + SAVE_SRC_B);
    c.wf(agent + 0x54, b);
    let cc = c.rf(SAVE_TABLE + SAVE_SRC_C);
    c.wf(agent + 0x58, cc);
    let flag = c.r8(SAVE_TABLE + SAVE_SRC_FLAG);
    c.w8(agent + 0x68, flag);
    let d = c.rf(SAVE_TABLE + SAVE_SRC_D);
    c.wf(agent + 0x44, d);
    set_gpr(c, V0, SAVE_TABLE);
    set_gpr(c, V1, flag as u32);
    c.set_farg(0, d);
    c.set_farg(1, b);
    restore_float_args(c, fa);
    true
}

/// Scales the basis at a0 by 1/count, copies the quad at a0 and applies the step helper `count` times.
/// Leaves v0 at the quad copy and a0 at the copy.
pub fn scaled_step_chain(c: &mut Cpu) -> bool {
    let src = c.arg(0);
    let a1 = c.arg(1);
    let count = c.arg(2);
    h::frame(c, 0x60, &[(18, 0x40, h::SQ), (16, 0x20, h::SQ), (17, 0x30, h::SQ), (31, 0x50, h::SQ)], |c, sp| {
        h::call_keep(c, BASIS_FN, &[sp, a1, src], &[]);
        let (_, n) = h::call_keep(c, INT_TO_FLOAT, &[count], &[]);
        set_gpr(c, AT, ONE);
        let inv = F(ONE) / n;
        h::call_keep(c, VEC_SCALE, &[sp, sp], &[inv]);

        let copy = sp + 0x10;
        let quad = c.rv(src);
        c.wv(copy, quad);
        let steps = count as i32;
        for _ in 0..steps.max(0) {
            h::call_keep(c, STEP_FN, &[copy, copy, sp], &[]);
        }
        set_gpr(c, A0, copy);
    });
    true
}
