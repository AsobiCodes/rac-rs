//! Font-2 glyph helpers: a width, texture upload and draw wrapper, a glyph record filler, and two VU0 xyz helpers.
use crate::font_tex;
use crate::fxtex;
use crate::h::{self, SQ};
use crate::ps2::{Ee, XYZ};
use ee_oracle::cpu::Cpu;

/// Glyph table passed in t2 by the draw call (the 0x1c3990 table).
const GLYPHS_3990: u32 = 0x001c_3990;
/// Retail draw routine (a0..a3, t0 = style, t1 = texture word, t2 = glyph table).
const FONT_GLYPHS_DRAW: u32 = 0x001e_da60;
/// Texture slot uploaded before the draw.
const TEX_SLOT: u32 = 3;

/// 0x1ee288 text_width_upload_draw(x, a1, a2, s, n) -> v0: x minus the width of s (n chars), uploads fx texture
/// slot 3, then draws with glyph table 0x1c3990; v0 is the draw's result.
pub fn text_width_upload_draw(c: &mut Cpu) -> bool {
    // The argument registers as the entry left them (full 128-bit values, as retail's moves copy them).
    let args: [u128; 5] = [c.r[4], c.r[5], c.r[6], c.r[7], c.r[8]];
    let saves = [
        (16, 0x00, SQ),
        (17, 0x10, SQ),
        (18, 0x20, SQ),
        (19, 0x30, SQ),
        (20, 0x40, SQ),
        (31, 0x50, SQ),
    ];
    h::frame(c, 0x60, &saves, |c, _| {
        // text_width(a0 = a3, a1 = t0)
        c.r[4] = args[3];
        c.r[5] = args[4];
        h::run_keep(c, font_tex::text_width_3990, &[], &[]);
        // subu: a 32-bit difference, sign-extended to 64 bits.
        let width = c.r[2] as u32;
        let x = (args[0] as u32).wrapping_sub(width) as i32 as i64 as u64;
        // fx_tex_upload(a0 = 3); its v0 is the texture word (t1).
        c.s(4, TEX_SLOT as u64);
        h::run_keep(c, fxtex::fx_tex_upload, &[], &[]);
        let tex = c.r[2];
        c.s(4, x);
        c.r[5] = args[1];
        c.r[6] = args[2];
        c.r[7] = args[3];
        c.r[8] = args[4];
        c.r[9] = tex;
        c.s(10, GLYPHS_3990 as i32 as i64 as u64);
        h::call_keep(c, FONT_GLYPHS_DRAW, &[], &[]);
    });
    true
}

/// 0x1eeab8 fill_glyph_record(rec, a1..a3, t0..t3, stack word) -> v0: stores eight halfwords into the record
/// (fields at 0..0xa, 0x10, 0x12), clears the four at 0xc, 0xe, 0x14, 0x16; v0 is the stack word.
pub fn fill_glyph_record(c: &mut Cpu) -> bool {
    let rec = c.arg(0);
    let sp = c.g(29) as u32;
    let stack_word = c.r32(sp);
    let fields = [
        (0x00, c.arg(1)),
        (0x02, c.arg(2)),
        (0x04, c.arg(3)),
        (0x06, c.arg(4)),
        (0x08, c.arg(5)),
        (0x0a, c.arg(6)),
        (0x10, c.arg(7)),
        (0x12, stack_word),
    ];
    for (off, v) in fields {
        c.w16(rec + off, v as u16);
    }
    for off in [0x0c, 0x0e, 0x14, 0x16] {
        c.w16(rec + off, 0);
    }
    c.ret(stack_word);
    true
}

/// 0x1f1858 vsub_xyz_inplace(a1, a2) -> (): the xyz lanes of the quad at a1 minus those at a2, written back to a1.
pub fn vsub_xyz_inplace(c: &mut Cpu) -> bool {
    let (dst, src) = (c.arg(1), c.arg(2));
    let a = c.rv(dst);
    let b = c.rv(src);
    let d = a.with(a - b, XYZ);
    c.set_vf(1, d);
    c.set_vf(2, b);
    c.wv(dst, d);
    true
}

/// 0x1f18e0 vmul_xyz_store(a0, a1, a2) -> (): the xyz lanes of the quad at a1 times those at a2, stored to a0.
pub fn vmul_xyz_store(c: &mut Cpu) -> bool {
    let (dst, a_addr, b_addr) = (c.arg(0), c.arg(1), c.arg(2));
    let a = c.rv(a_addr);
    let b = c.rv(b_addr);
    let p = a.with(a * b, XYZ);
    c.set_vf(1, p);
    c.set_vf(2, b);
    c.wv(dst, p);
    true
}
