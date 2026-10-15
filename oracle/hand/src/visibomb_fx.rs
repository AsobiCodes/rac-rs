//! Visibomb render-state restore and missile cleanup: copies the held object's transform words and flags into the
//! render globals, resets the fog and draw caps when asked, sets the clear colour, and tears the missile down.

use crate::ps2::*;
use crate::{gs_pkt_misc, h};
use ee_oracle::cpu::Cpu;

const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const S0: usize = 16;
const S1: usize = 17;
const GP: usize = 28;
const SP: usize = 29;
const RA: usize = 31;

/// Upper half of the render-globals block (the `lui $at, 0x16` base).
const RENDER_HI: u32 = 0x0016_0000;
/// Upper half of the parameter block (`lui 0x17` then `addiu -0x34c0` gives 0x16CB40).
const PARAM_BLOCK: u32 = 0x0016_CB40;
/// Upper half of the object-state slot (`lui a3, 0x17`).
const PARAM_HI: u32 = 0x0017_0000;
/// Frame-clear RGBAQ and fog/draw-cap constants written when `enable` is nonzero.
const FOG_TIMER_VALUE: u32 = 0x1f4;
const DRAW_CAP_WORD: u32 = 0x1f4000;
const FOG_NEAR_F: u32 = 0x48fa_0000;
const FOG_FAR_F: u32 = 0x4434_0000;
const FOG_SCALE_F: u32 = 0x43fa_0000;
/// Gp-relative slot that receives the object's third flag byte.
const GP_FLAG_OFF: u32 = 0x77ba;
/// Link pointer cleared by the cleanup, and the object-table words it reads.
const LINK_OFF: u32 = 0x14;
const STATE_PTR_OFF: u32 = 0x78;
const SEG_0X14_ADDR: u32 = 0x0014_0000;
const SEG_0X14_OFF: u32 = 0x1330;
const SEG_0X16_OFF: u32 = 0x7114;

/// Adds a signed displacement to a 32-bit base address (MIPS `lw/sw` offset arithmetic).
fn disp(base: u32, off: i32) -> u32 {
    base.wrapping_add(off as u32)
}

/// Splits a 128-bit GPR into the four little-endian words a `sq` stores.
fn quad(x: u128) -> V {
    V([x as u32, (x >> 32) as u32, (x >> 64) as u32, (x >> 96) as u32])
}

/// Sets the low 64 bits of a GPR to a sign-extended 32-bit value, as `lw` / `lui` / `addiu` leave it.
fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Sets the low 64 bits of a GPR to a zero-extended byte, as `lbu` leaves it.
fn set_byte(c: &mut Cpu, reg: usize, v: u8) {
    c.s(reg, v as u64);
}

/// 0x2a7b98 visibomb_render_restore(obj, enable) -> (): copies the object's four float words and two flag bytes
/// into the render globals at 0x15f448..; when `enable` is nonzero also resets the fog words and the draw cap,
/// then sets the frame-clear colour from the parameter block at 0x16CB40.
pub fn render_restore(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let enable = c.arg(1);
    let sp = c.g(SP) as u32;
    let ra = c.r[RA];
    c.wv(disp(sp, -0x10), quad(ra));

    let block = c.r32(disp(obj, STATE_PTR_OFF as i32));
    set_gpr(c, A2, block);
    c.w32(disp(RENDER_HI, -0xcf4), 0);
    set_gpr(c, A0, 1);

    for i in 0..4u32 {
        let f = c.rf(block.wrapping_add(0x30 + 4 * i));
        c.wf(disp(RENDER_HI, -0xbb8 + 4 * i as i32), f);
        c.f[i as usize] = f.0;
    }

    let flag0 = c.r8(block.wrapping_add(0x40));
    c.w8(disp(RENDER_HI, -0xbbc), flag0);
    set_byte(c, V0, flag0);
    let flag1 = c.r8(block.wrapping_add(0x41));
    c.w8(disp(RENDER_HI, -0xbbb), flag1);
    set_byte(c, V1, flag1);
    let flag2 = c.r8(block.wrapping_add(0x42));
    set_byte(c, V0, flag2);
    let gp = c.g(GP) as u32;
    c.w8(disp(gp, -(GP_FLAG_OFF as i32)), flag2);

    set_gpr(c, A3, PARAM_HI);
    c.w32(disp(PARAM_HI, -0x5f08), 1);

    if enable != 0 {
        c.f[2] = FOG_NEAR_F;
        c.f[1] = FOG_FAR_F;
        c.f[0] = FOG_SCALE_F;
        c.wf(disp(RENDER_HI, 0xf00), F(FOG_NEAR_F));
        c.wf(disp(RENDER_HI, 0xf60), F(FOG_FAR_F));
        c.wf(disp(RENDER_HI, 0x424), F(FOG_SCALE_F));
        set_gpr(c, V1, FOG_TIMER_VALUE);
        c.w32(disp(RENDER_HI, -0x90), FOG_TIMER_VALUE);
        set_gpr(c, V0, DRAW_CAP_WORD);
        c.w32(disp(RENDER_HI, 0xfc), DRAW_CAP_WORD);
    }

    set_gpr(c, AT, RENDER_HI);
    let a2 = c.r32(PARAM_BLOCK + 0x244);
    let a0 = c.r32(PARAM_BLOCK + 0x23c);
    let a1 = c.r32(PARAM_BLOCK + 0x240);
    set_gpr(c, A0, a0);
    set_gpr(c, A1, a1);
    set_gpr(c, A2, a2);
    h::run_keep(c, gs_pkt_misc::set_clear_rgbaq, &[], &[]);
    // retail's set_clear_rgbaq also shifts a2 left 16 in place (dsll $a2, 0x10); the hand leaves it alone.
    let shifted = c.g(A2) << 16;
    c.s(A2, shifted);
    true
}

/// 0x2a7ec8 visibomb_cleanup(missile, obj) -> (): unlinks the held object's state, clears the camera return and
/// the hero state word, runs the render restore, frees the missile moby, and leaves v0 = 0x160000.
pub fn cleanup(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let missile = c.arg(0);
    let obj = c.arg(1);
    let sp = c.g(SP) as u32;
    let frame = disp(sp, -0x30);
    let s0 = c.r[S0];
    let s1 = c.r[S1];
    let ra = c.r[RA];
    c.wv(frame.wrapping_add(0x10), quad(s1));
    c.wv(frame, quad(s0));
    c.wv(frame.wrapping_add(0x20), quad(ra));

    let hdr = c.r32(obj);
    let state = c.r32(disp(hdr, STATE_PTR_OFF as i32));
    set_gpr(c, V0, state);
    if state != 0 {
        c.w32(state.wrapping_add(4), 0);
    }
    set_gpr(c, V0, SEG_0X14_ADDR);
    c.w32(disp(SEG_0X14_ADDR, SEG_0X14_OFF as i32), 0);

    let link = c.r32(obj.wrapping_add(LINK_OFF));
    set_gpr(c, A0, link);
    h::call_keep(c, 0x2ef0_a8, &[link], &[]);
    c.w32(obj.wrapping_add(LINK_OFF), 0);

    h::call_keep(c, 0x21_5538, &[0, 1], &[]);
    h::run_keep(c, render_restore, &[missile, 1], &[]);
    h::call_keep(c, 0x1f_6c20, &[], &[]);
    h::call_keep(c, 0x24_10e8, &[missile], &[]);

    set_gpr(c, V0, RENDER_HI);
    c.w32(disp(RENDER_HI, SEG_0X16_OFF as i32), 0);
    true
}
