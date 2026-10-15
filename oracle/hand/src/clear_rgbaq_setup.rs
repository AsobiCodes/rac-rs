//! 0x2a7a78 frame-clear setup: writes the constant words and the 0x40-byte frame state block (through the struct at
//! a0+0x78), then calls set_clear_rgbaq(0x40, 0x60, 0x40). The callee's v0/v1/a1 results stay as retail leaves them.

use crate::h;
use crate::ps2::{Ee, F};
use ee_oracle::cpu::Cpu;

const A0: usize = 4;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;
const T2: usize = 10;
const T3: usize = 11;
const T4: usize = 12;
const GP: usize = 28;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x2a7a78 setup_frame_clear(a0) -> (): see module doc.
pub fn setup_frame_clear(c: &mut Cpu) -> bool {
    let a0 = c.g(A0) as u32;
    let a3 = c.r32(a0.wrapping_add(0x78));
    let f0 = c.rf(0x15_f448);
    let f1 = c.rf(0x15_f44c);
    let f2 = c.rf(0x15_f450);
    let f3 = c.rf(0x15_f454);
    c.w32(0x15_f30c, 1);
    c.wf(a3.wrapping_add(0x30), f0);
    c.wf(a3.wrapping_add(0x34), f1);
    c.wf(a3.wrapping_add(0x38), f2);
    c.wf(a3.wrapping_add(0x3c), f3);
    let b0 = c.r8(0x15_f444);
    c.w8(a3.wrapping_add(0x40), b0);
    let b1 = c.r8(0x15_f445);
    c.w8(a3.wrapping_add(0x41), b1);
    let b2 = c.r8(0x15_f446);
    c.w8(a3.wrapping_add(0x42), b2);
    c.w8(0x15_f445, 0x60);
    c.w8(0x15_f446, 0x40);
    c.wf(0x15_f44c, F(0x4800_0000));
    c.wf(0x15_f450, F(0x437f_0000));
    c.w32(0x16_a0f8, 0);
    c.wf(0x16_0f00, F(0x4810_0000));
    c.wf(0x16_0424, F(0x4310_0000));
    c.w32(0x15_ff70, 0x90);
    c.w32(0x16_00fc, 0x5_0000);
    c.w8(0x15_f444, 0x40);
    c.wf(0x16_0f60, F(0x4310_0000));
    c.w32(0x15_f448, 0);
    let gp = c.g(GP) as u32;
    c.w32(gp.wrapping_sub(0x77ac), 0);
    set_gpr(c, T0, 0x40);
    set_gpr(c, T1, 0x60);
    set_gpr(c, T2, 0x17_0000);
    set_gpr(c, T3, 0x5_0000);
    set_gpr(c, T4, 0x90);
    set_gpr(c, A3, a3);
    c.f[0] = 0x437f_0000;
    c.f[1] = 0x4810_0000;
    c.f[2] = 0x4800_0000;
    c.f[3] = f3.0;
    c.f[4] = 0x4310_0000;
    let _ = h::run_keep(c, crate::gs_pkt_misc::set_clear_rgbaq, &[0x40, 0x60, 0x40], &[]);
    set_gpr(c, 6, 0x40_0000);
    true
}
