//! Debris moby updater (retail 0x2c6038): drift, gravity, roll bounce, fade and destroy.

use crate::actor;
use crate::h;
use crate::mathx;
use crate::motion;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Gravity per frame: a float in overlay data, read from RAM at call time.
const GRAVITY_ADDR: u32 = 0x15ed70;
/// Frames of gravity applied per update: 15.0.
const GRAVITY_FRAMES: F = F(0x4170_0000);
/// gp-relative float that scales the drift of the velocity (gp - 0x5398).
const GP_DRIFT: u32 = 0x5398;
/// gp-relative float that scales the lifetime (gp - 0x7e98), loaded by float_to_int_scaled.
const GP_LIFE: u32 = 0x7e98;
/// Roll limit pi/4 (bits 0x3f490fdb).
const ROLL_LIMIT: F = F(0x3f49_0fdb);
/// Scale passed to float_to_int_scaled for the lifetime test.
const LIFE_SCALE: u32 = 30;
/// Constant the scaled-lifetime helper leaves in $at and f3 (0.25).
const QUARTER: F = F(0x3e80_0000);
/// Constants angle_add leaves in f14/f15 (+pi, -pi).
const PI: F = F(0x4049_0fdb);
const NEG_PI: F = F(0xc049_0fdb);

/// MIPS and FPU register numbers written by the retail body.
const AT: usize = 1;
const V1: usize = 3;
const F_ONE: usize = 1;
const F_TWO: usize = 2;
const F_THREE: usize = 3;
const F_FOUR: usize = 4;
const F_FIVE: usize = 5;
const F_PI: usize = 14;
const F_NEG_PI: usize = 15;

/// Velocity block offsets (pointer at moby + 0x78).
const VEL_X: u32 = 0x00;
const VEL_Y: u32 = 0x04;
const VEL_Z: u32 = 0x08;
const VEL_FLOOR: u32 = 0x20;
const VEL_SPIN_INT: u32 = 0x24;
const VEL_PITCH_RATE: u32 = 0x2c;
const VEL_YAW_RATE: u32 = 0x28;
const VEL_LIFE: u32 = 0x1c;

/// Moby offsets.
const MOBY_VEL: u32 = 0x78;
const MOBY_POS: u32 = 0x10;
const MOBY_PITCH: u32 = 0x40;
const MOBY_YAW: u32 = 0x48;
const MOBY_FADE: u32 = 0x23;
const MOBY_FLAG: u32 = 0x31;

/// Retail vector step on the position with the velocity block.
const VEC_STEP: u32 = 0x1f1810;

fn gp_float(c: &mut Cpu, back: u32) -> F {
    let a = (c.g(28) as u32).wrapping_sub(back);
    c.rf(a)
}

/// Retail angle_add leaves +pi and -pi in f14/f15 (and $v0/$at, which later callees overwrite).
fn angle_add(c: &mut Cpu, a: F, b: F) -> F {
    let (_, r) = h::run_keep(c, vecmath::angle_add, &[], &[a, b]);
    c.f[F_PI] = PI.0;
    c.f[F_NEG_PI] = NEG_PI.0;
    r
}

/// Runs the free_moby hand; its scratch registers stay as its emulation leaves them (refile_cells rewrites $a1/$v1
/// on some paths, so the retail values are not a fixed mask or the moby pointer).
fn free(c: &mut Cpu, m: u32) {
    h::run_keep(c, actor::free_moby, &[m], &[]);
}

/// 0x2c6038 debris_update(m): one frame of a debris moby. Steps the position with its velocity block, drifts
/// x/y by the gp scale, pulls z down by 15 frames of gravity and clamps it at the floor, rolls the yaw and the
/// pitch (|pitch| kept, the spin sign flipped past pi/4), then counts the life down: the moby is freed when the
/// life counter ends, or when its fade byte ends after the life falls below the scaled threshold.
pub fn debris_update(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let vel = c.r32(m + MOBY_VEL);
    let pos = m + MOBY_POS;
    h::call_keep(c, VEC_STEP, &[pos, pos, vel], &[]);

    // Drift and gravity on the velocity block.
    let drift = gp_float(c, GP_DRIFT);
    let fall = c.rf(GRAVITY_ADDR) * GRAVITY_FRAMES;
    let z = c.rf(vel + VEL_Z) - fall;
    let y = c.rf(vel + VEL_Y) * drift;
    let x = c.rf(vel + VEL_X) * drift;
    c.wf(vel + VEL_Y, y);
    c.wf(vel + VEL_X, x);
    let floor = c.rf(vel + VEL_FLOOR);
    c.wf(vel + VEL_Z, if z < floor { floor } else { z });
    c.f[F_TWO] = z.0;
    c.f[F_THREE] = x.0;
    c.f[F_FOUR] = drift.0;
    c.f[F_FIVE] = floor.0;

    // Yaw roll: moby yaw += velocity yaw rate, wrapped.
    let yaw_in = c.rf(m + MOBY_YAW);
    let yaw_rate = c.rf(vel + VEL_YAW_RATE);
    let yaw = angle_add(c, yaw_in, yaw_rate);
    c.wf(m + MOBY_YAW, yaw);

    // Pitch roll: the spin integer scales the pitch rate; |pitch| is stored and a spin past pi/4 flips its sign.
    let spin = c.r32(vel + VEL_SPIN_INT) as i32;
    let pitch_in = c.rf(m + MOBY_PITCH);
    let pitch_rate = c.rf(vel + VEL_PITCH_RATE) * F::from_int(spin);
    let pitch = angle_add(c, pitch_in, pitch_rate);
    // The retail store of the pitch sits in the delay slot of the fabs call, so it stores the signed pitch.
    c.wf(m + MOBY_PITCH, pitch);
    let (_, mag) = h::run_keep(c, motion::fabs, &[], &[pitch]);
    c.f[12] = pitch.0;
    c.f[13] = pitch_rate.0;
    c.f[F_ONE] = ROLL_LIMIT.0;
    if ROLL_LIMIT < mag {
        c.w32(vel + VEL_SPIN_INT, spin.wrapping_neg() as u32);
    }

    // Life countdown: tick_i32 leaves the stored counter in $at.
    let (tick, _) = h::run_keep(c, mathx::tick_i32, &[vel + VEL_LIFE], &[]);
    let stored = c.r32(vel + VEL_LIFE);
    c.s(AT, stored as u64);
    if tick != 0 {
        free(c, m);
        return true;
    }
    let flag = c.r8(m + MOBY_FLAG) as u32;
    c.ret(flag);
    if flag == 0 {
        free(c, m);
        return true;
    }
    let (scaled, _) = h::run_keep(c, mathx::float_to_int_scaled, &[LIFE_SCALE], &[]);
    // float_to_int_scaled leaves its scratch values: $at, f1 = the result bits, f2 = the lifetime scale, f3 = 0.25.
    c.s(AT, QUARTER.0 as u64);
    c.f[F_ONE] = scaled;
    c.f[F_TWO] = gp_float(c, GP_LIFE).0;
    c.f[F_THREE] = QUARTER.0;
    let life = c.r32(vel + VEL_LIFE) as i32;
    let below = life < scaled as i32;
    c.s(V1, below as u64);
    if !below {
        return true;
    }
    let (fade, _) = h::run_keep(c, mathx::tick_u8, &[m + MOBY_FADE], &[]);
    let fade_byte = c.r8(m + MOBY_FADE);
    c.s(AT, fade_byte as u64);
    if fade != 0 {
        free(c, m);
    }
    true
}
