//! Font-2 text width and draw wrappers: each picks a glyph table and forwards to the shared width or draw routine
//! (the draw path first uploads an fx texture slot).
use crate::fxtex;
use crate::h::{self, SQ};
use crate::sprite_text;
use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

/// Glyph tables passed in a2 by the two width wrappers.
const GLYPHS_35F0: u32 = 0x001c_35f0;
const GLYPHS_3990: u32 = 0x001c_3990;
/// Glyph table passed in t2 by the draw path (slot 1), and the one by the slot-2 path.
const GLYPHS_3250: u32 = 0x001c_3250;
/// Retail draw routine: (a0..a3, t0 = style, t1 = texture word, t2 = glyph table).
const FONT_GLYPHS_DRAW: u32 = 0x001e_da60;

/// Width of `s` (first `n` chars) over glyph table 0x1c35f0 (wrapper of 0x1ed9a8).
pub fn text_width_35f0(c: &mut Cpu) -> bool {
    text_width_with(c, GLYPHS_35F0)
}

/// Width of `s` (first `n` chars) over glyph table 0x1c3990 (wrapper of 0x1ed9a8).
pub fn text_width_3990(c: &mut Cpu) -> bool {
    text_width_with(c, GLYPHS_3990)
}

/// Shared body of the width wrappers: a frame saving ra, then the retail text_width_n with the table in a2.
fn text_width_with(c: &mut Cpu, glyphs: u32) -> bool {
    let (s, n) = (c.arg(0), c.arg(1));
    h::frame(c, 0x10, &[(31, 0x0, SQ)], |c, _| {
        h::run_keep(c, sprite_text::text_width_n, &[s, n, glyphs], &[]);
    });
    true
}

/// Uploads fx texture slot 1, then draws font-2 glyphs with table 0x1c3250 (0x1edce0).
pub fn fx_upload_draw_slot1(c: &mut Cpu) -> bool {
    upload_then_draw(c, 1, GLYPHS_3250)
}

/// Uploads fx texture slot 2, then draws font-2 glyphs with table 0x1c35f0 (0x1edd60).
pub fn font2_upload_draw(c: &mut Cpu) -> bool {
    upload_then_draw(c, 2, GLYPHS_35F0)
}

/// Shared body: fx_tex_upload(slot) gives the texture word in v0, which becomes t1 of the draw call; the draw
/// call gets the caller's a0..a3 and t0 unchanged and the glyph table in t2. The callee's v0 is the result.
fn upload_then_draw(c: &mut Cpu, slot: u32, glyphs: u32) -> bool {
    // Full 128-bit argument registers as the entry left them (retail keeps their upper halves).
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
        h::run_keep(c, fxtex::fx_tex_upload, &[slot], &[]);
        // The texture word stays in v0 (full register) and becomes t1; t2 is the glyph table.
        let tex = c.r[2];
        c.r[4..9].copy_from_slice(&args);
        c.r[9] = tex;
        c.s(10, glyphs as i32 as i64 as u64);
        h::call_keep(c, FONT_GLYPHS_DRAW, &[], &[]);
    });
    true
}
