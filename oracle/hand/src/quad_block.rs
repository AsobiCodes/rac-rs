//! Hand rewrites of two HUD quad helpers: a double-buffered quad pass and the label draw that wraps it.

use crate::font2_text;
use crate::h;
use crate::ps2::Ee;
use crate::quad_pair;
use ee_oracle::cpu::Cpu;

/// Allocator (0x22e350) returning a pointer; called with this size and flag 0 then 1.
const ALLOC: u32 = 0x22_e350;
const ALLOC_SIZE: u32 = 0x7580;
/// Tail helper (0x22e9a8) that finishes the pass.
const QUAD_TAIL: u32 = 0x22_e9a8;
/// Stack frame and saved registers of the quad pass (s0-s6, ra).
const PASS_FRAME: u32 = 0x80;
const PASS_SAVES: [(usize, u32, u8); 8] = [
    (16, 0x00, h::SQ),
    (17, 0x10, h::SQ),
    (18, 0x20, h::SQ),
    (19, 0x30, h::SQ),
    (20, 0x40, h::SQ),
    (21, 0x50, h::SQ),
    (22, 0x60, h::SQ),
    (31, 0x70, h::SQ),
];
/// Stack frame and saved registers of the label draw (s0-s4, ra).
const LABEL_FRAME: u32 = 0x60;
const LABEL_SAVES: [(usize, u32, u8); 6] = [
    (16, 0x00, h::SQ),
    (17, 0x10, h::SQ),
    (18, 0x20, h::SQ),
    (19, 0x30, h::SQ),
    (20, 0x40, h::SQ),
    (31, 0x50, h::SQ),
];
/// Level is clamped to this maximum (sign-extended top byte of the colour word).
const LEVEL_MAX: i32 = 0x50;
/// Colour-word mask for the first font call (lui 0xff00).
const ALPHA_MASK: u32 = 0xff00_0000;
/// Width of the quad pass step subtracted from the label x.
const PASS_STEP: u32 = 0x20;

/// 0x22fff0 quad_block_pass(a0 base, a1 y, a2 length, a3 w, t0 level): two allocations, two quad_pair calls and
/// the tail helper, splitting the span of `length` bytes at `base` into two halves.
pub fn quad_block_pass(c: &mut Cpu) -> bool {
    let base = c.arg(0);
    let y = c.arg(1);
    let len = c.arg(2);
    let w = c.arg(3);
    let t0 = c.arg(4);
    h::frame(c, PASS_FRAME, &PASS_SAVES, |c, _sp| {
        let first = h::call_keep(c, ALLOC, &[ALLOC_SIZE, 0], &[]).0;
        let second = h::call_keep(c, ALLOC, &[ALLOC_SIZE, 1], &[]).0;
        h::run_keep(c, quad_pair::quad_pair, &[second, base, y, PASS_STEP, w, t0], &[]);
        h::run_keep(
            c,
            quad_pair::quad_pair,
            &[first, base.wrapping_add(PASS_STEP), y, len.wrapping_sub(0x40), w, t0],
            &[],
        );
        let end = base.wrapping_add(len);
        h::call_keep(c, QUAD_TAIL, &[second, end.wrapping_sub(PASS_STEP), y, PASS_STEP, w, t0], &[]);
    });
    true
}

/// 0x2300c8 quad_label_draw(a0 x, a1 y, a2 colour, a3 extra, t1 z): draws the font-2 label centred on x, then the
/// quad pass under it sized by the colour's top byte (clamped to 0x50), then the label again.
pub fn quad_label_draw(c: &mut Cpu) -> bool {
    let x = c.arg(0);
    let y = c.arg(1);
    let colour = c.arg(2);
    let extra = c.arg(3);
    let t1 = c.arg(5);
    h::frame(c, LABEL_FRAME, &LABEL_SAVES, |c, _sp| {
        let level = ((colour as i32) >> 24).min(LEVEL_MAX);
        let shadow_x = h::run_keep(
            c,
            font2_text::font2_draw_centred_b,
            &[x.wrapping_add(1), y.wrapping_add(1), colour & ALPHA_MASK, extra, u32::MAX],
            &[],
        )
        .0;
        let pass_x = shadow_x.wrapping_sub(PASS_STEP);
        let span = x.wrapping_sub(pass_x) << 1;
        h::run_keep(
            c,
            quad_block_pass,
            &[pass_x, y.wrapping_sub(8), span, PASS_STEP, level as u32, t1],
            &[],
        );
        h::run_keep(c, font2_text::font2_draw_centred_b, &[x, y, colour, extra, u32::MAX], &[]);
    });
    true
}
