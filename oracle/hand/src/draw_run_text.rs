//! Draw-queue runner and the text placement pair that feeds it (retail 0x1ebc98, 0x1ee168, 0x1ee318).

use crate::h;
use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

/// Word holding the number of queued draw callbacks.
const DRAW_QUEUE_COUNT: u32 = 0x0015_F424;
/// Table of callback function pointers; entry i is at DRAW_QUEUE_FN + 4 * i.
const DRAW_QUEUE_FN: u32 = 0x0016_D980;
/// Table of callback arguments, parallel to DRAW_QUEUE_FN.
const DRAW_QUEUE_ARG: u32 = 0x0016_DA80;
/// Retail text width measure (the Rust twin is sprite_text::text_width_1c3250).
const TEXT_WIDTH_1ED9F8: u32 = 0x001E_D9F8;
/// Retail glyph texture upload (the Rust twin is fxtex::fx_tex_upload).
const FX_TEX_UPLOAD_1EBB00: u32 = 0x001E_BB00;
/// Retail glyph placer that draws the measured text.
const TEXT_PLACE_1EDA60: u32 = 0x001E_DA60;
/// Return address of the draw callback jalr (0x1ebcdc).
const DRAW_CALLBACK_RET: u32 = 0x001E_BCE4;
/// Font table handed to the glyph placer (lui 0x1c, addiu 0x3250).
const FONT_TABLE_1C3250: u64 = 0x001C_3250;

/// Stack frame of the text placers: s0-s4 and ra, saved like the retail prologue.
const TEXT_FRAME_SIZE: u32 = 0x60;
const TEXT_SAVES: [(usize, u32, u8); 6] = [
    (16, 0x00, h::SQ),
    (17, 0x10, h::SQ),
    (18, 0x20, h::SQ),
    (19, 0x30, h::SQ),
    (20, 0x40, h::SQ),
    (31, 0x50, h::SQ),
];

/// Calls retail `addr` with the full 64-bit argument registers in `regs` (the placer's 64-bit ops read them).
/// Like h::call_keep: s0-s7, gp, sp, fp, ra and f20-f31 are put back; the other registers stay as the callee left them.
fn call_keep_raw(c: &mut Cpu, addr: u32, ret: u32, regs: &[(usize, u64)]) {
    call_keep_raw_s7(c, addr, ret, regs, true);
}

/// As call_keep_raw; with restore_s7 false, s7 stays as the callee left it (retail callbacks may leave it changed).
fn call_keep_raw_s7(c: &mut Cpu, addr: u32, ret: u32, regs: &[(usize, u64)], restore_s7: bool) {
    let (r, f) = (c.r, c.f);
    for &(i, v) in regs {
        c.s(i, v);
    }
    // The retail jal leaves ra at the instruction after its delay slot; the callee returns there.
    c.s(31, ret as u64);
    c.call_dyn(addr, ret);
    for i in (16..24).chain(28..32) {
        if i != 23 || restore_s7 {
            c.r[i] = r[i];
        }
    }
    for i in 20..32 {
        c.f[i] = f[i];
    }
}

/// 0x1ebc98 draw_queue_run(): calls every queued draw callback in registration order; returns 0 after a run.
pub fn draw_queue_run(c: &mut Cpu) -> bool {
    let saves = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (18, 0x20, h::SQ), (31, 0x30, h::SQ)];
    h::frame(c, 0x40, &saves, |c, _sp| {
        let mut count = c.r32(DRAW_QUEUE_COUNT) as i32;
        let mut v0 = count as u32;
        if count > 0 {
            let mut done = 0u32;
            loop {
                let func = c.r32(DRAW_QUEUE_FN + 4 * done);
                let arg = c.r32(DRAW_QUEUE_ARG + 4 * done) as i32 as i64 as u64;
                done += 1;
                call_keep_raw_s7(c, func, DRAW_CALLBACK_RET, &[(3, func as i32 as i64 as u64), (4, arg)], false);
                // Retail re-reads the count after every callback, so callbacks may queue more work.
                count = c.r32(DRAW_QUEUE_COUNT) as i32;
                if (done as i32) >= count {
                    break;
                }
            }
            v0 = 0;
        }
        c.ret(v0);
    });
    true
}

/// Measures the text (optionally halving the width), places it, and returns the shifted x.
/// Arguments are passed as the full 64-bit registers the caller holds: the retail placer uses 64-bit ops on them.
fn place_text(c: &mut Cpu, halve_width: bool, sites: [u32; 3]) -> u64 {
    // Retail args: a0=x, a1=row, a2=col, a3=text, t0=len (t0 is GPR 8, not t4).
    let (x, a1, a2, text, len) = (c.g(4), c.g(5), c.g(6), c.g(7), c.g(8));
    h::frame(c, TEXT_FRAME_SIZE, &TEXT_SAVES, |c, _sp| {
        call_keep_raw(c, TEXT_WIDTH_1ED9F8, sites[0], &[(4, text), (5, len)]);
        let mut width = c.g(2) as u32;
        if halve_width {
            width = ((width as i32) >> 1) as u32;
        }
        let x = (x as u32).wrapping_sub(width) as i32 as i64 as u64;
        call_keep_raw(c, FX_TEX_UPLOAD_1EBB00, sites[1], &[(4, 1)]);
        let glyph_fx = c.g(2);
        call_keep_raw(
            c,
            TEXT_PLACE_1EDA60,
            sites[2],
            &[(4, x), (5, a1), (6, a2), (7, text), (8, len), (9, glyph_fx), (10, FONT_TABLE_1C3250)],
        );
        x
    })
}

/// 0x1ee168 text_place_centred_x(x, row, col, text, len): x minus the text width, then draws the text.
pub fn text_place_1ee168(c: &mut Cpu) -> bool {
    place_text(c, false, [0x001E_E1A4, 0x001E_E1B0, 0x001E_E1D4]);
    true
}

/// 0x1ee318 text_place_half_x(x, row, col, text, len) -> x: like 0x1ee168 with half the width; returns new x.
pub fn text_place_1ee318(c: &mut Cpu) -> bool {
    let x = place_text(c, true, [0x001E_E354, 0x001E_E364, 0x001E_E388]);
    c.s(2, x);
    true
}
