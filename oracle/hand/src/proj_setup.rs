//! Hand-rewrite of retail code at 0x1ea0b0. Reads two signed 16-bit values at 0x151780 (+0x150, +0x152),
//! fills the 0x13e500 viewport block and the float constants at 0x16cb40, and spills s0/s1/ra/f20 to the stack.

use ee_oracle::cpu::Cpu;
use crate::ps2::*;

/// Viewport block (integer halves, scaled by 16).
const BLOCK: u32 = 0x13e500;
/// Float table base (s0 in retail).
const TABLE: u32 = 0x16cb40;
/// Source words: the two u16 values at +0x150 (x) and +0x152 (y).
const SRC: u32 = 0x151780;
/// Centre offset used by the viewport halves.
const CENTRE: i32 = 0x800;
/// Shift that scales the viewport halves.
const SCALE_SHIFT: u32 = 4;
/// Stack frame size the retail prologue reserves (addiu sp,sp,-0x40).
const FRAME: u32 = 0x40;

/// Float bit patterns written to the table.
const F_32: u32 = 0x4200_0000;
const F_SCALE_A: u32 = 0x4936_0000;
const F_0_63: u32 = 0x3f21_47ae;
const F_524288: u32 = 0x4900_0000;
const F_255: u32 = 0x437f_0000;

/// Final values of the scratch registers after the retail code.
const V0_BLOCK: u64 = 0x13e500;
const A3_LAST: u64 = 0x140000;
const AT_LAST: u64 = 0x437f_0000;

/// A 128-bit register value as a quadword (lane 0 in the low bits).
fn quad(q: u128) -> V {
    V([q as u32, (q >> 32) as u32, (q >> 64) as u32, (q >> 96) as u32])
}

/// 0x1ea0b0 proj_setup() -> (void): fills the viewport block at 0x13e500 and the float table at 0x16cb40 from
/// the two u16 values at 0x151780+0x150/0x152.
pub fn proj_setup(c: &mut Cpu) -> bool {
    let sp = (c.g(29) as u32).wrapping_sub(FRAME);
    // Prologue spills (retail sq/swc1 into the frame; the epilogue reloads them, so the registers stay).
    c.wv(sp, quad(c.r[16]));
    c.wv(sp + 0x10, quad(c.r[17]));
    c.wv(sp + 0x20, quad(c.r[31]));
    c.w32(sp + 0x30, c.f[20]);

    let x = c.r16(SRC + 0x150) as i16 as i32;
    let y = c.r16(SRC + 0x152) as i16 as i32;
    let t1 = x >> 1;
    let t2 = y >> 1;
    let a0 = ((t2 + CENTRE) as u32) << SCALE_SHIFT;
    let a2 = ((CENTRE - t1) as u32) << SCALE_SHIFT;
    let a3 = ((CENTRE - t2) as u32) << SCALE_SHIFT;
    let t0 = ((t1 + CENTRE) as u32) << SCALE_SHIFT;

    c.w32(BLOCK, x as u32);
    c.w32(BLOCK + 4, y as u32);
    c.w32(BLOCK + 8, t1 as u32);
    c.w32(BLOCK + 0xc, t2 as u32);
    c.w32(BLOCK + 0x10, a2);
    c.w32(BLOCK + 0x14, a3);
    c.w32(BLOCK + 0x18, t0);
    c.w32(BLOCK + 0x1c, a0);

    c.w32(TABLE + 0xa0, F_32);
    c.w32(TABLE + 0xa4, F_SCALE_A);
    c.w32(TABLE + 0xb0, F_0_63);

    let half = F(0x3f00_0000);
    let four = F(0x4080_0000);
    let rx = F::from_int(x) * half;
    let ry = F::from_int(y) * half;
    let rx4 = rx * four;
    let ry4 = ry * four;
    c.wf(TABLE + 0x200, rx);
    c.wf(TABLE + 0x204, ry);
    c.wf(TABLE + 0x208, rx4);
    c.wf(TABLE + 0x20c, ry4);
    c.w32(TABLE + 0x218, 0);
    c.w32(TABLE + 0x21c, F_524288);
    c.w32(TABLE + 0x228, F_255);
    c.w32(TABLE + 0x22c, 0);

    // Scratch registers as retail leaves them.
    c.s(1, AT_LAST);
    c.s(2, V0_BLOCK);
    c.s(3, x as i64 as u64);
    c.s(4, y as i64 as u64);
    c.s(5, y as i64 as u64);
    c.s(6, a2 as u64);
    c.s(7, a3 as u64);
    c.s(8, t0 as u64);
    c.s(9, t1 as i64 as u64);
    c.s(10, t2 as i64 as u64);
    c.s(11, A3_LAST);
    c.f[0] = ry.0;
    c.f[1] = rx4.0;
    c.f[2] = F_255;
    c.f[3] = four.0;
    c.f[4] = F_524288;
    c.f[5] = ry4.0;
    let _ = a0;
    true
}
