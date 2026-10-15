//! Pulsing glow colours of the hero's three gadget mobys (Eudora overlay 0x227948).
use crate::calls::*;
use crate::ps2::*;
use crate::trig;
use ee_oracle::cpu::Cpu;

const ANGLE_ADD: u32 = 0x1f_2680;
const COLOR_LERP: u32 = 0x1f_2830;
const AXIS_QUAT: u32 = 0x1f_24c0;
const LAYER_ATTACH: u32 = 0x24_1e58;
const GAME_MODE: u32 = 0x15_f5a4;
const GP_MODE: u32 = -0x765ci32 as u32;
const GP_RATE: u32 = -0x7e94i32 as u32;
const HALF: F = F(0x3f00_0000);

/// Pulse state: glow moby pointers at +0x11d0 / +0x11d4 / +0x1220 and angles at +0x22e8 / +0x22ec / +0x22f0.
const H_GLOW_A: u32 = 0x11d0;
const H_GLOW_B: u32 = 0x11d4;
const H_GLOW_C: u32 = 0x1220;
const H_ANG_A: u32 = 0x22e8;
const H_ANG_B: u32 = 0x22ec;
const H_ANG_C: u32 = 0x22f0;
const M_COLOR: u32 = 0x90;

fn angle_add(c: &mut Cpu, a: F, d: F) -> F {
    call(c, ANGLE_ADD, &[], &[a, d]);
    c.fresult()
}

fn advance(c: &mut Cpu, off: u32, delta: F) -> F {
    let a = c.rf(HERO + off);
    angle_add(c, a, delta)
}

fn color_lerp(c: &mut Cpu, from: u32, to: u32, t: F) -> u32 {
    call(c, COLOR_LERP, &[from, to], &[t]);
    c.result()
}

/// 0.5 * sin(angle) + 0.5.
fn pulse(angle: F) -> F {
    trig::vu_sin(angle) * HALF + HALF
}

/// 0x227948 glow_pulse_update(): advances three pulse angles and recolours the glow mobys.
pub fn glow_pulse_update(c: &mut Cpu) -> bool {
    preserved(c, false, false, glow_pulse_update_body)
}

fn glow_pulse_update_body(c: &mut Cpu) -> bool {
    let mode2 = c.r32(GAME_MODE) == 2;
    let rate = c.rf((c.g(28) as u32).wrapping_add(GP_RATE));
    let glow_a = c.r32(HERO + H_GLOW_A);
    if glow_a != 0 {
        let k = if mode2 { F(0x3f9c_61aa) } else { F(0x4006_0a92) };
        let a = advance(c, H_ANG_A, rate * k);
        c.wf(HERO + H_ANG_A, a);
        let col = color_lerp(c, 0xd2_d2d2, 0x28_5050, pulse(a));
        c.w32(glow_a + M_COLOR, col);
    }
    let glow_b = c.r32(HERO + H_GLOW_B);
    if glow_b != 0 {
        let gp_mode2 = c.r32((c.g(28) as u32).wrapping_add(GP_MODE)) == 2;
        let rate = c.rf((c.g(28) as u32).wrapping_add(GP_RATE));
        if gp_mode2 {
            let a = advance(c, H_ANG_B, rate * F(0x3fc9_0fdb));
            c.wf(HERO + H_ANG_B, a);
            let col = color_lerp(c, 0x1e_1ed2, 0x1e_1e50, pulse(a));
            c.w32(glow_b + M_COLOR, col);
        } else {
            let a = advance(c, H_ANG_B, rate * F(0x403d_e44e));
            c.wf(HERO + H_ANG_B, a);
            let mut target = 0x1e_e628;
            if F::ZERO < a && a < F(0x3ff5_be0b) {
                target = 0x1e_1ed2;
            }
            let cur = c.r32(glow_b + M_COLOR);
            let col = color_lerp(c, cur, target, F(0x3d8f_5c29));
            c.w32(glow_b + M_COLOR, col);
        }
    }
    let glow_c = c.r32(HERO + H_GLOW_C);
    if glow_c != 0 {
        let gp_mode2 = c.r32((c.g(28) as u32).wrapping_add(GP_MODE)) == 2;
        let rate = c.rf((c.g(28) as u32).wrapping_add(GP_RATE));
        let k = if gp_mode2 { F(0x3fb2_b8c2) } else { F(0x3ff5_be0b) };
        let a = advance(c, H_ANG_B, rate * k);
        c.wf(HERO + H_ANG_C, a);
        let col = color_lerp(c, 0xaa_c8aa, 0x96_5050, pulse(a));
        c.w32(glow_c + M_COLOR, col);
        if c.r8(HERO + 0x1d91) == 0 {
            call(c, LAYER_ATTACH, &[glow_c, 0, HERO + 0x1d90], &[]);
        } else {
            let a = c.rf(HERO + H_ANG_C);
            call(c, AXIS_QUAT, &[HERO + 0x1da0, 0], &[a]);
        }
    }
    true
}
