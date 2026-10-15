//! Hero tilt recovery, carried-velocity decay and the hero cue forwarder (retail 0x2096f8, 0x209850, 0x209f28).
use crate::h::{call, run};
use crate::hero_space;
use crate::motion;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Hero state block (retail `$s0` in the tilt code): 0x140000 - 0xcb0.
const HERO: u32 = 0x13_F350;
/// Carried-velocity vector inside the hero block: 0x140000 - 0xb60.
const CARRY: u32 = 0x13_F4A0;
/// Euler x/y springs inside the hero block (`HERO + 0x90` / `+ 0x94`).
const EULER_X: u32 = HERO + 0x90;
const EULER_Y: u32 = HERO + 0x94;
/// Euler angular velocities (`HERO + 0xa0` / `+ 0xa4`).
const EULER_VEL_X: u32 = HERO + 0xa0;
const EULER_VEL_Y: u32 = HERO + 0xa4;
/// Scratch vector the carried velocity is added into (`CARRY - 0x70`).
const CARRY_ADD: u32 = CARRY - 0x70;

/// Retail function addresses that are not hand-written yet.
const RETAIL_SWING_STATE: u32 = 0x20_9530;
const RETAIL_ZERO_VEC: u32 = 0x1f_17f8;
const RETAIL_CUE: u32 = 0x27_e470;

/// Constants in ROM read through the `lui 0x16` / `lwc1` pair.
const SPRING_GAIN_ADDR: u32 = 0x15_ED64;
const SPRING_LIMIT_ADDR: u32 = 0x15_ED6C;
/// 0x3c75c28f, 0x3e99999a and 0x40a78d36 (loaded with `lui`/`ori` into f22, f21, f20).
const SCALE_A: u32 = 0x3c75_c28f;
const SCALE_B: u32 = 0x3e99_999a;
const SCALE_C: u32 = 0x40a7_8d36;
/// Float bit pattern of -0.0 (the zero word negated with `neg.s`).
const NEG_ZERO: u32 = 0x8000_0000;
/// The carried length below this is treated as zero (0x3a83126f).
const CARRY_EPSILON: u32 = 0x3a83_126f;

/// 0x2096f8 hero_tilt_recover(): springs the hero's euler x/y (0x13f3e0/e4) back to 0 outside swing/tumble states.
pub fn hero_tilt_recover(c: &mut Cpu) -> bool {
    let flag = if c.r16(HERO + 0x1f8) != 0 { 1 } else { c.r8(HERO + 0x20b3) as u32 };
    if flag != 0 {
        if flag < 3 {
            call(c, RETAIL_SWING_STATE, &[flag], &[]);
        }
        return true;
    }
    let state = c.r32(HERO + 0x2084) as i32;
    if (state.wrapping_sub(0x25) as u32) < 2 {
        return true;
    }
    let mode = c.r32(HERO + 0x208c);
    if matches!(mode, 0x11 | 0x15 | 0x16) || matches!(state, 0x77 | 0x31 | 0x2d | 0x2c) {
        return true;
    }
    let gain = c.rf(SPRING_GAIN_ADDR);
    let limit = c.rf(SPRING_LIMIT_ADDR);
    let (scale_a, scale_b, scale_c) = (F(SCALE_A), F(SCALE_B), F(SCALE_C));
    let (k, d, max) = (gain * scale_a, gain * scale_b, limit * scale_c);
    let target = F(NEG_ZERO);
    run(c, motion::turn_spring_angle, &[EULER_X, EULER_VEL_X, 0], &[target, k, d, max]);
    run(c, motion::turn_spring_angle, &[EULER_Y, EULER_VEL_Y, 0], &[target, k, d, max]);
    true
}

/// 0x209850 hero_carry_decay(step): shortens the carried velocity (0x13f4a0) by `step` (floor 0) and adds it to
/// 0x13f430; below 0.001 the carry is zeroed instead.
pub fn hero_carry_decay(c: &mut Cpu) -> bool {
    let step = c.farg(0);
    let len = run(c, hero_space::length_off_up, &[CARRY], &[]).1;
    if F(CARRY_EPSILON) < len {
        let zero = F(0);
        run(c, hero_space::set_up_component, &[CARRY, CARRY], &[zero]);
        let mut rest = len - step;
        if rest < zero {
            rest = zero;
        }
        run(c, hero_space::set_length_xy_up, &[CARRY, CARRY], &[rest]);
        run(c, vecmath::vec_add, &[CARRY_ADD, CARRY_ADD, CARRY], &[]);
    } else {
        call(c, RETAIL_ZERO_VEC, &[CARRY], &[]);
    }
    true
}

/// 0x209f28 hero_cue(cue) -> v0: plays `cue` through 0x27e470; in Hologuise state (3) first copies the hero's
/// quadword at +0x80 into the body block, and returns -1 when that block is missing.
pub fn hero_cue(c: &mut Cpu) -> bool {
    let cue = c.arg(0);
    // Retail leaves a2 holding the body pointer (or the 0x2080 word) and a3 holding the cue; the callee sees both.
    let mut a2 = c.r32(HERO + 0x2080);
    if c.r8(HERO + 0x20a4) == 3 {
        let body = c.r32(HERO + 0xa88);
        a2 = body;
        if body == 0 {
            c.ret(u32::MAX);
            return true;
        }
        let q = c.rv(HERO + 0x80);
        c.wv(body + 0x10, q);
    }
    let a1 = c.arg(1);
    let ret = call(c, RETAIL_CUE, &[cue, a1, a2, cue], &[]).0;
    c.ret(ret);
    true
}
