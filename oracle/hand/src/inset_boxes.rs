//! Hand version of the level-4 seven-box drawer at 0x1ed6c0: draws boxes stepped in from a corner through 0x1ec948.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail box drawer (x, y, w, h, colour) -> void, at 0x1ec948 (no hand version yet).
const DRAW_BOX: u32 = 0x1e_c948;
/// Stack frame size of the retail function (`addiu sp,sp,-0xb0`).
const FRAME: u32 = 0xb0;
/// Colour base built by `lui 4; ori 0x404` (0x40404).
const COLOUR_BASE: u32 = 0x0004_0404;
/// The colour argument is shifted into the top byte (`sll t0,t0,0x18`).
const COLOUR_SHIFT: u32 = 24;
/// Callee-saved registers and their `sq` slots in the retail frame (s0..s7, fp, ra).
const SAVES: [(usize, u32, u8); 10] = [
    (16, 0x10, h::SQ),
    (17, 0x20, h::SQ),
    (18, 0x30, h::SQ),
    (19, 0x40, h::SQ),
    (20, 0x50, h::SQ),
    (21, 0x60, h::SQ),
    (22, 0x70, h::SQ),
    (23, 0x80, h::SQ),
    (30, 0x90, h::SQ),
    (31, 0xa0, h::SQ),
];

/// Adds the signed offset `d` to `v` with 32-bit wrap-around, as MIPS `addiu` does.
fn off(v: u32, d: i32) -> u32 {
    v.wrapping_add(d as u32)
}

/// 0x1ed6c0 draw_inset_boxes(x, y, w, h, colour) -> void: seven boxes around the corner (x, y) and the size (w, h),
/// each drawn by 0x1ec948 with the colour `(colour << 24) | 0x40404`.
pub fn draw_inset_boxes(c: &mut Cpu) -> bool {
    let (x, y, w, ht) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let colour = (c.arg(4) << COLOUR_SHIFT) | COLOUR_BASE;
    h::frame(c, FRAME, &SAVES, |c, sp| {
        // Retail keeps the first column and row offsets in its frame; the later calls reload them from there.
        c.w32(sp, off(x, 1));
        c.w32(sp + 4, off(y, -1));
        let boxes = [
            (x, y, w, ht),
            (off(x, 1), off(y, -1), off(w, -2), w),
            (off(x, 2), off(y, -2), off(w, -3), off(w, -2)),
            (off(x, 4), off(y, -4), off(w, -4), off(w, -3)),
            (off(x, 1), off(y, -1), ht, off(ht, 2)),
            (off(x, 2), off(y, -2), off(ht, 2), off(ht, 3)),
            (off(x, 4), off(y, -4), off(ht, 3), off(ht, 4)),
        ];
        for (bx, by, bw, bh) in boxes {
            h::call_keep(c, DRAW_BOX, &[bx, by, bw, bh, colour], &[]);
        }
    });
    true
}
