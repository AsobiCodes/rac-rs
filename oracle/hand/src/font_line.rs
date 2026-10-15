//! Font line wrappers: they measure a line with retail helpers, shift the draw origin, and hand the line to the retail
//! glyph line renderer at 0x1eda60.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail glyph line renderer (decoded as font_glyphs_draw).
const GLYPH_LINE_DRAW: u32 = 0x1eda60;
/// Retail helper that returns the line width for (a3, t0).
const LINE_WIDTH: u32 = 0x1ed9f8;
/// Retail helper called with 1 to get the glyph scale factor.
const GLYPH_SCALE: u32 = 0x1ebb00;
/// Glyph table pointer passed to the renderer in t2 (lui 0x1c, addiu 0x3250).
const GLYPH_TABLE: u32 = 0x1c_3250;

/// 0x1ee168 font_line_draw_right(origin, a1, a2, a3, t0) -> v0: draws a line right-aligned at origin minus its width
pub fn line_draw_right(c: &mut Cpu) -> bool {
    // Capture the entry arguments before any retail call: the callees clobber a1..t0.
    let origin = c.arg(0);
    let (a1, a2, a3, t0) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4));
    let (width, _) = h::call(c, LINE_WIDTH, &[a3, t0], &[]);
    let x = origin.wrapping_sub(width);
    let (scale, _) = h::call(c, GLYPH_SCALE, &[1], &[]);
    let (v0, _) = h::call(c, GLYPH_LINE_DRAW, &[x, a1, a2, a3, t0, scale, GLYPH_TABLE], &[]);
    c.ret(v0);
    true
}

/// 0x1ee318 font_line_draw_center(origin, a1, a2, a3, t0) -> v0: draws a line centred on origin, returns the new x
pub fn line_draw_center(c: &mut Cpu) -> bool {
    let origin = c.arg(0);
    let (a1, a2, a3, t0) = (c.arg(1), c.arg(2), c.arg(3), c.arg(4));
    let (width, _) = h::call(c, LINE_WIDTH, &[a3, t0], &[]);
    let half = ((width as i32) >> 1) as u32;
    let x = origin.wrapping_sub(half);
    let (scale, _) = h::call(c, GLYPH_SCALE, &[1], &[]);
    h::call(c, GLYPH_LINE_DRAW, &[x, a1, a2, a3, t0, scale, GLYPH_TABLE], &[]);
    c.ret(x);
    true
}
