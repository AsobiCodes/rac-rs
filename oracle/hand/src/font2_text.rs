//! Font-2 text drawing: an x adjustment from a width call (right-aligned, or half the width for centred text), a
//! texture upload, then the draw call. The three entries share one body.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Width callees: 0x1eda18 for the right and first centred entry, 0x1eda38 for the second centred entry.
const WIDTH_A: u32 = 0x1e_da18;
const WIDTH_B: u32 = 0x1e_da38;
/// Draw call (text list with x, y, and style arguments).
const DRAW: u32 = 0x1e_da60;
/// Texture upload (retail; the fxtex hand differs on slot 2).
const FX_TEX_UPLOAD: u32 = 0x1e_bb00;
/// Style table pointers passed in t2 (`lui 0x1c` plus an addiu).
const TABLE_A: u32 = 0x1c_35f0;
const TABLE_B: u32 = 0x1c_3990;
/// Retail frame size and the register saves of its prologue (s0-s4 and ra).
const FRAME: u32 = 0x60;
const SAVES: [(usize, u32, u8); 6] = [
    (16, 0x00, h::SQ),
    (17, 0x10, h::SQ),
    (18, 0x20, h::SQ),
    (19, 0x30, h::SQ),
    (20, 0x40, h::SQ),
    (31, 0x50, h::SQ),
];

/// The entry arguments, read before any retail call changes the registers.
struct Args {
    a0: u32,
    a1: u64,
    a2: u64,
    a3: u64,
    t0: u64,
}

fn args(c: &mut Cpu) -> Args {
    Args { a0: c.arg(0), a1: c.g(5), a2: c.g(6), a3: c.g(7), t0: c.g(8) }
}

/// Shared body: x minus a width from `width_fn` (halved when `halve`), texture upload of `slot`, then the draw call
/// with style table `table`. Returns the adjusted x.
fn draw_text(c: &mut Cpu, a: &Args, width_fn: u32, halve: bool, slot: u32, table: u32) -> u32 {
    let mut width = h::call_keep(c, width_fn, &[a.a3 as u32, a.t0 as u32], &[]).0;
    if halve {
        width = ((width as i32) >> 1) as u32;
    }
    let x = a.a0.wrapping_sub(width);
    h::call_keep(c, FX_TEX_UPLOAD, &[slot], &[]);
    c.r[9] = c.r[2];
    c.s(10, table as u64);
    // The draw call gets the entry registers at full 64-bit width (retail moves them through s-registers).
    let (r, f) = (c.r, c.f);
    c.s(4, x as i32 as i64 as u64);
    c.s(5, a.a1);
    c.s(6, a.a2);
    c.s(7, a.a3);
    c.s(8, a.t0);
    c.call(DRAW);
    for i in (16..24).chain(28..32) {
        c.r[i] = r[i];
    }
    for i in 20..32 {
        c.f[i] = f[i];
    }
    x
}

/// 0x1ee1f8 font2_draw_right(a0 x, a1, a2, a3, t0): draws font-2 text right-aligned at x minus its width (0x1eda18),
/// texture slot 2, then the draw call with style table 0x1c35f0.
pub fn font2_draw_right(c: &mut Cpu) -> bool {
    let a = args(c);
    h::frame(c, FRAME, &SAVES, |c, _sp| {
        draw_text(c, &a, WIDTH_A, false, 2, TABLE_A);
    });
    true
}

/// 0x1ee3b0 font2_draw_centred(a0 x, a1, a2, a3, t0) -> x: draws font-2 text centred on x (x minus width/2), texture
/// slot 2, then the draw call with style table 0x1c35f0; returns the adjusted x.
pub fn font2_draw_centred(c: &mut Cpu) -> bool {
    let a = args(c);
    h::frame(c, FRAME, &SAVES, |c, _sp| {
        let x = draw_text(c, &a, WIDTH_A, true, 2, TABLE_A);
        c.ret(x);
    });
    true
}

/// 0x1ee448 font2_draw_centred_b(a0 x, a1, a2, a3, t0) -> x: as font2_draw_centred, with width callee 0x1eda38,
/// texture slot 3 and style table 0x1c3990; returns the adjusted x.
pub fn font2_draw_centred_b(c: &mut Cpu) -> bool {
    let a = args(c);
    h::frame(c, FRAME, &SAVES, |c, _sp| {
        let x = draw_text(c, &a, WIDTH_B, true, 3, TABLE_B);
        c.ret(x);
    });
    true
}
