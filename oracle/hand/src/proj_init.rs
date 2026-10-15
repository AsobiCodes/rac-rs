//! Hand rewrites of three retail init routines: the offscreen projection setup, a clear of a block of game globals,
//! and a one-time init guard.

use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Projection block base (`lui 0x14` + `addiu -0x1b00`); the first word is the batch's first argument.
const PROJ_BLOCK: u32 = 0x13E500;
/// Projection state base (`lui 0x17` + `addiu -0x34c0`).
const PROJ_STATE: u32 = 0x16CB40;
/// Float bit patterns stored into the projection state.
const FLOAT_524288: u32 = 0x4900_0000;
const FLOAT_32: u32 = 0x4200_0000;
const HALF: F = F(0x3f00_0000);
const FOUR: F = F(0x4080_0000);
/// Offset of the halved-size word written after the projection block (the `0x800` centre).
const CENTRE: i32 = 0x800;

/// Game globals block base (`lui 0x16`).
const GLOBALS: u32 = 0x160000;
/// One-time init flag (`lw` at 0x15f5b8) and the routine it calls when the flag is clear.
const INIT_FLAG: u32 = GLOBALS - 0xa48;
const INIT_CALLEE: u32 = 0x269f00;

/// 0x1ea808 offscreen_setup(a0 width, a1 height, f12..f16 camera floats) -> void: fills the offscreen projection
/// block and state, then calls 0x1ea1e8 to build the projection.
pub fn offscreen_setup(c: &mut Cpu) -> bool {
    let a0 = c.arg(0);
    let a1 = c.arg(1);
    let fx = c.farg(0);
    let f13 = c.farg(1);
    let f14 = c.farg(2);
    let f15 = c.farg(3);
    let f16 = c.farg(4);
    let saved_f20_24 = [c.f[20], c.f[21], c.f[22], c.f[23], c.f[24]];
    h::frame(c, 0x60, &[(16, 0x0, h::SQ), (17, 0x10, h::SQ), (31, 0x20, h::SQ)], |c, sp| {
        for (i, bits) in saved_f20_24.iter().enumerate() {
            c.w32(sp + 0x30 + 8 * i as u32, *bits);
        }
        let t0 = a0 as i32;
        let s1 = a1 as i32;
        let t1 = t0 >> 1;
        let t2 = s1 >> 1;
        let (a1_v, a2_v, a3_v, v1_v) = (
            (CENTRE - t1) << 4,
            (CENTRE - t2) << 4,
            (t1 + CENTRE) << 4,
            (t2 + CENTRE) << 4,
        );
        let blk = PROJ_BLOCK;
        c.w32(blk, a0);
        c.w32(blk + 0x4, a1);
        c.w32(blk + 0x8, t1 as u32);
        c.w32(blk + 0xc, t2 as u32);
        c.w32(blk + 0x10, a1_v as u32);
        c.w32(blk + 0x14, a2_v as u32);
        c.w32(blk + 0x18, a3_v as u32);
        c.w32(blk + 0x1c, v1_v as u32);

        let st = PROJ_STATE;
        c.w32(st + 0xb0, fx.0);
        c.w32(st + 0xa4, FLOAT_524288);
        c.w32(st + 0xa0, FLOAT_32);
        c.w32(st + 0x218, f13.0);
        c.w32(st + 0x21c, f14.0);
        c.w32(st + 0x228, f15.0);
        c.w32(st + 0x22c, f16.0);

        let (_, fa) = h::run_keep(c, mathx::int_to_float, &[a0], &[]);
        let fa = fa * HALF;
        c.w32(st + 0x200, fa.0);
        let (_, fb) = h::run_keep(c, mathx::int_to_float, &[s1 as u32], &[]);
        let fb = fb * HALF;

        c.w32(st + 0x204, fb.0);
        c.w32(st + 0x20c, (fb * FOUR).0);
        c.w32(st + 0x208, (fa * FOUR).0);

        // Scratch registers as retail leaves them before its jal 0x1ea1e8.
        c.f[0] = fb.0;
        c.f[1] = (fa * FOUR).0;
        c.f[2] = FOUR.0;
        c.f[3] = (fb * FOUR).0;
        c.f[20] = HALF.0;
        c.f[21] = f16.0;
        c.f[22] = f13.0;
        c.f[23] = f14.0;
        c.f[24] = f15.0;
        c.s(1, FOUR.0 as i32 as i64 as u64);
        c.s(2, (PROJ_BLOCK) as i32 as i64 as u64);
        c.s(3, v1_v as i64 as u64);
        c.s(4, s1 as i64 as u64);
        c.s(5, a1_v as i64 as u64);
        c.s(6, a2_v as i64 as u64);
        c.s(7, a3_v as i64 as u64);
        c.s(8, a0 as i64 as u64);
        c.s(9, t1 as i64 as u64);
        c.s(10, t2 as i64 as u64);
        c.s(11, 0x140000);
        h::call_keep(c, 0x1ea1e8, &[], &[]);
        // Retail restores f20-f24 from its frame before returning.
        for (i, bits) in saved_f20_24.iter().enumerate() {
            c.f[20 + i] = *bits;
        }
    });
    true
}

/// 0x1eac38 clear_globals() -> void: zeroes thirteen game-global words (lui 0x16 block and gp-relative slots).
pub fn clear_globals(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    for off in [0xbdc, 0xbd4, 0xbd0, 0xbd8, 0xbcc, 0xd10, 0xd0c, 0xbfc, 0xbf8, 0xa18] {
        c.w32(GLOBALS - off, 0);
    }
    for off in [0x1250u32, 0x1254, 0x1258] {
        c.w32(GLOBALS + off, 0);
    }
    for off in [0x78f4u32, 0x78e0, 0x78d0, 0x59a4] {
        c.w32(gp.wrapping_sub(off), 0);
    }
    c.s(1, GLOBALS as u64);
    true
}

/// 0x1eb7b8 init_once() -> v0: calls 0x269f00(0) unless the init flag at 0x15f5b8 is already set; returns the flag.
pub fn init_once(c: &mut Cpu) -> bool {
    h::frame(c, 0x10, &[(31, 0x0, h::SQ)], |c, _sp| {
        let flag = c.r32(INIT_FLAG);
        if flag != 0 {
            c.s(2, flag as i32 as i64 as u64);
        } else {
            h::call_keep(c, INIT_CALLEE, &[0], &[]);
        }
    });
    true
}
