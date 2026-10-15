//! Actor sway: the per-frame bob and orientation wobble of an actor (retail 0x2e1f40) and the one-time seeding of its
//! sway record (retail 0x2e2030). The record is reached through the pointer at actor+0x78.
use crate::ps2::*;
use crate::{h, mathx, rand, trig, vecmath};
use ee_oracle::cpu::Cpu;

/// Mode byte of the actor: 1 = sway moves the bob height and the three orientation angles.
const A_MODE: u32 = 0x20;
/// Pointer to the scale record of the actor; its word at +0x24 is the sway scale.
const A_SCALE_PTR: u32 = 0x24;
/// Bob height (+0x18), sway scale output (+0x2c), orientation angles (+0x40..+0x4c), sway record pointer (+0x78).
const A_BOB: u32 = 0x18;
const A_SCALE_OUT: u32 = 0x2c;
const A_ANGLE: u32 = 0x40;
const A_STATE: u32 = 0x78;
/// Sway record: bob height at +0x18 and the angle rates at +0x60 (roll), +0x64 (yaw), +0x68 (pitch).
const S_BOB: u32 = 0x18;
const S_RATE_ROLL: u32 = 0x60;
const S_RATE_YAW: u32 = 0x64;
const S_RATE_PITCH: u32 = 0x68;
/// gp-relative float constants: time scale (gp - 0x4fa0), bob bias (gp - 0x4f9c), bob amplitude (gp - 0x4f98), last
/// angle rate (gp - 0x4f84); the random range word (gp - 0x4f60). The damping constant is at 0x15ed6c. All are read from
/// RAM at call time.
const GP_TIME: u32 = 0x4fa0;
const GP_BIAS: u32 = 0x4f9c;
const GP_AMP: u32 = 0x4f98;
const GP_RATE: u32 = 0x4f84;
const GP_RANGE: u32 = 0x4f60;
const DAMP_ADDR: u32 = 0x15ed6c;
/// Float constant 0x3c8efa35 that the retail body builds with lui/ori.
const RATE_IMM: u32 = 0x3c8e_fa35;
/// Retail generator: pointer to the state struct (lui/lw at 0x12f76c), state word at +0x58, LCG multiplier and addend.
const RAND_STATE_PTR: u32 = 0x12_f76c;
const RAND_STATE_WORD: u32 = 0x58;
const RAND_MUL: u32 = 0x41c6_4e6d;
const RAND_ADD: u32 = 0x3039;
/// Retail rand_below (0x24a418): runs when n is 0 (division by zero).
const RAND_BELOW_RETAIL: u32 = 0x24_a418;
/// Retail callees: the per-frame step and the rare follow-up.
const STEP_ADDR: u32 = 0x2e2110;
const FOLLOW_ADDR: u32 = 0x2e25b0;

/// Sets a GPR to a sign-extended 32-bit value, as `lw` / `addiu` leave it.
fn set_gpr(c: &mut Cpu, n: usize, v: u32) {
    c.r[n] = v as i32 as i128 as u128;
}

/// angle_add(a, b) through the hand, keeping the scratch registers the callee leaves.
fn angle_add(c: &mut Cpu, a: F, b: F) -> F {
    let (_, r) = h::run_keep(c, vecmath::angle_add, &[], &[a, b]);
    r
}

/// rand_below(n) as retail computes it (0x24a418 over the generator 0x1160d8), with the registers it leaves: a0 the LCG
/// multiplier, a1 the state struct pointer, v1 the new state word, v0 the result. The hand copy of rand_below leaves
/// those scratch registers alone, so the sway step keeps the retail values here. n == 0 runs the retail routine.
fn rand_below_retail(c: &mut Cpu, n: u32) -> u32 {
    if n == 0 {
        return h::call_keep(c, RAND_BELOW_RETAIL, &[0], &[]).0;
    }
    let ptr = c.r32(RAND_STATE_PTR);
    let x = c.r32(ptr + RAND_STATE_WORD).wrapping_mul(RAND_MUL).wrapping_add(RAND_ADD);
    c.w32(ptr + RAND_STATE_WORD, x);
    set_gpr(c, 4, RAND_MUL);
    set_gpr(c, 5, ptr);
    set_gpr(c, 3, x);
    let draw = (((x & 0x7fff_ffff) as i32) >> 16) & 0x7fff;
    let r = draw.wrapping_rem(n as i32) as u32;
    set_gpr(c, 2, r);
    r
}

/// 0x2e1f40 actor_sway(actor) -> (): writes the sway scale to +0x2c; in mode 1 it advances the bob height and the three
/// orientation angles by sin-driven steps, then runs the step 0x2e2110 and, when rand_below returns 0, 0x2e25b0.
/// Retail prologue saves of 0x2e1f40: s0 at +0x00, s1 at +0x10, ra at +0x20 (frame 0x30).
const SWAY_SAVES: [(usize, u32, u8); 3] = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (31, 0x20, h::SQ)];

/// The sway body runs with the retail frame (sp lowered by 0x30), so callees that take sp-relative values
/// (for example the `move a2, sp` of 0x2e25b0) leave the same registers as retail.
pub fn actor_sway(c: &mut Cpu) -> bool {
    h::frame(c, 0x30, &SWAY_SAVES, |c, _sp| sway_body(c))
}

fn sway_body(c: &mut Cpu) -> bool {
    let actor = c.arg(0);
    let gp = c.g(28) as u32;
    let time_k = c.rf(gp.wrapping_sub(GP_TIME));
    let state = c.r32(actor + A_STATE);
    let mode = c.r8(actor + A_MODE) as u32;
    let scale_rec = c.r32(actor + A_SCALE_PTR);
    let scale = c.rf(scale_rec.wrapping_add(0x24)) * time_k;
    c.wf(actor + A_SCALE_OUT, scale);
    c.f[0] = scale.0;
    c.f[1] = time_k.0;
    set_gpr(c, 2, scale_rec);
    set_gpr(c, 3, mode);
    set_gpr(c, 4, 1);

    if mode == 1 {
        let pitch_angle = c.rf(actor + A_ANGLE + 12);
        let (_, sin) = h::run_keep(c, trig::fast_sin, &[], &[pitch_angle]);
        let bias = c.rf(gp.wrapping_sub(GP_BIAS));
        c.f[3] = bias.0;
        let bob = c.rf(state + S_BOB) + bias;
        let amp = c.rf(gp.wrapping_sub(GP_AMP)) * sin;
        c.wf(actor + A_BOB, bob + amp);

        let a = c.rf(actor + A_ANGLE);
        let b = c.rf(state + S_RATE_PITCH);
        let v = angle_add(c, a, b);
        c.wf(actor + A_ANGLE, v);
        let a = c.rf(actor + A_ANGLE + 4);
        let b = c.rf(state + S_RATE_YAW);
        let v = angle_add(c, a, b);
        c.wf(actor + A_ANGLE + 4, v);
        let a = c.rf(actor + A_ANGLE + 8);
        let b = c.rf(state + S_RATE_ROLL);
        let v = angle_add(c, a, b);
        c.wf(actor + A_ANGLE + 8, v);

        let damp = c.rf(DAMP_ADDR);
        let rate = (c.rf(gp.wrapping_sub(GP_RATE)) * F(RATE_IMM)) * damp;
        let a = c.rf(actor + A_ANGLE + 12);
        c.f[1] = RATE_IMM;
        c.f[2] = damp.0;
        let v = angle_add(c, a, rate);
        c.wf(actor + A_ANGLE + 12, v);
    }

    h::call_keep(c, STEP_ADDR, &[actor], &[]);
    let range = c.r32(gp.wrapping_sub(GP_RANGE));
    if rand_below_retail(c, range) == 0 {
        h::call_keep(c, FOLLOW_ADDR, &[actor], &[]);
    }
    true
}

/// 0x2e2030 actor_sway_seed(actor) -> (): fills the sway record (pointer at actor+0x78): four random samples in [0, 255]
/// as floats at +0x20, four constants at +0x30, four scaled integers (63 + 64k) at +0x40 and four constants at +0x50.
pub fn actor_sway_seed(c: &mut Cpu) -> bool {
    let actor = c.arg(0);
    let state = c.r32(actor + A_STATE);
    for i in 0..4 {
        let (v, _) = h::run_keep(c, rand::rand_below, &[0xff], &[]);
        c.wf(state + 0x20 + 4 * i, F::from_int(v as i32));
    }
    c.wf(state + 0x30, F(0xbf80_0000));
    c.wf(state + 0x34, F(0xc010_0000));
    c.wf(state + 0x38, F(0x3fa0_0000));
    c.wf(state + 0x3c, F(0x4020_0000));
    for i in 0..4 {
        let (v, _) = h::run_keep(c, mathx::float_to_int_scaled, &[63 + 64 * i], &[]);
        c.w32(state + 0x40 + 4 * i, v);
    }
    c.wf(state + 0x50, F(0x4020_0000));
    c.wf(state + 0x54, F(0x4040_0000));
    c.wf(state + 0x58, F(0x4040_0000));
    c.wf(state + 0x5c, F(0x4020_0000));
    // The last float_to_int_scaled call leaves its own constants in f2 (1.0) and f3 (0.25); f0 and f1 are the stores'.
    c.f[0] = 0x4020_0000;
    c.f[1] = 0x4040_0000;
    c.f[2] = 0x3f80_0000;
    c.f[3] = 0x3e80_0000;
    true
}
