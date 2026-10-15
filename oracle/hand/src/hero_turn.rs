//! Hero yaw spring, gait-0 turn and speed slew (retail 0x204ed8, 0x204fe0, 0x205130). All three work on the hero
//! global block at 0x13f350.
use crate::glove_slot;
use crate::h;
use crate::motion;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Base of the hero global block (`lui 0x14` + `addiu -0xcb0`).
const HERO_BASE: u32 = 0x0013_F350;
/// Retail helper run instead of the spring when the yaw-mode byte is set.
const YAW_MODE_HELPER: u32 = 0x0020_4E20;
/// Byte flag that selects the retail yaw-mode helper (base + 0x20b3).
const YAW_MODE_FLAG: u32 = HERO_BASE + 0x20B3;
/// Byte flag that enables the gait scale-up for slot values 15, 16 and 21 (base + 0x20a8).
const GAIT_FLAG: u32 = HERO_BASE + 0x20A8;
/// Halfword that scales the gait turn when non-zero (base + 0x3be).
const GAIT_HALF: u32 = HERO_BASE + 0x3BE;
/// Frame timer read by the gait turn (base + 0x229c).
const GAIT_TIMER: u32 = HERO_BASE + 0x229C;
/// Yaw angle, target and rate used by the spring (base + 0x180 .. 0x188).
const YAW_RATE: u32 = HERO_BASE + 0x180;
/// Spring state block passed to `motion::turn_spring_angle` (base + 0x98).
const YAW_STATE: u32 = HERO_BASE + 0x98;
/// Spring angle slot passed to `motion::turn_spring_angle` (base + 0x184).
const YAW_ANGLE: u32 = HERO_BASE + 0x184;
/// Output word that receives the spring result (base + 0x188).
const YAW_OUT: u32 = HERO_BASE + 0x188;
/// Slew target (base + 0x190) and slewed value (base + 0x194).
const SLEW_TARGET: u32 = HERO_BASE + 0x190;
const SLEW_VALUE: u32 = HERO_BASE + 0x194;
/// Read-only float constants in the 0x160000 page: the rate (0x15ed64) and the gain (0x15ed6c).
const CONST_RATE: u32 = 0x0016_0000 - 0x129C;
const CONST_GAIN: u32 = 0x0016_0000 - 0x1294;
/// Offset of the gp-relative float used by the gait scale-up path.
const GP_SCALE_OFF: u32 = 0x7E9C;

const ONE: F = F(0x3f80_0000);
const HALF: F = F(0x3f00_0000);
const RATE_SCALE: F = F(0x3c9b_a5e3);
const GAIN_SCALE: F = F(0x411f_2c8d);
const TENTH: F = F(0x3dcc_cccd);
const GAIT_UP_A: F = F(0x3f8c_cccd);
const GAIT_UP_B: F = F(0x3f99_999a);
const ALT_SCALE_A: F = F(0x3b44_9ba6);
const ALT_SCALE_B: F = F(0x3d23_d70a);
const ALT_SCALE_C: F = F(0x408b_a058);

/// 0x204ed8 hero_yaw_turn(a0, f12, f13, f14) -> v0/f0: springs the hero yaw toward the target; in yaw mode it
/// hands off to retail 0x204e20 instead.
pub fn hero_yaw_turn(c: &mut Cpu) -> bool {
    let a0 = c.arg(0);
    let (f12, f13, f14) = (c.farg(0), c.farg(1), c.farg(2));
    if c.r8(YAW_MODE_FLAG) != 0 {
        c.f[0] = f12.0;
        c.f[1] = f13.0;
        c.f[15] = f14.0;
        let (v0, f0) = h::call(c, YAW_MODE_HELPER, &[], &[]);
        c.ret(v0);
        c.fret(f0);
        return true;
    }
    let rate = c.rf(YAW_RATE);
    let (v0, f0) = h::run(c, motion::turn_spring_angle, &[YAW_STATE, YAW_ANGLE, a0], &[rate, f12, f13, f14]);
    c.wf(YAW_OUT, f0);
    c.ret(v0);
    c.fret(f0);
    true
}

/// 0x204fe0 hero_gait0_turn(a0) -> v0/f0: computes the gait-0 turn rates from the frame timer and the glove slot
/// value, then calls `hero_yaw_turn` with them.
pub fn hero_gait0_turn(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    let half = (c.rf(GAIT_TIMER) + ONE) * HALF;
    let rate = c.rf(CONST_RATE);
    let gain = c.rf(CONST_GAIN);
    let mut x = rate * RATE_SCALE * half;
    let mut y = rate * TENTH;
    let mut z = gain * GAIN_SCALE * half;
    if c.r16(GAIT_HALF) as i16 != 0 {
        x = x * GAIT_UP_A;
        z = z * GAIT_UP_B;
    }
    let (slot, _) = h::run(c, glove_slot::slot_value_b_fn, &[0], &[]);
    let slot = slot as i32;
    if (15..=16).contains(&slot) || slot == 21 {
        if c.r8(GAIT_FLAG) != 0 {
            let scale = c.rf(gp.wrapping_sub(GP_SCALE_OFF));
            x = scale * ALT_SCALE_A;
            y = scale * ALT_SCALE_B;
            z = gain * ALT_SCALE_C;
        }
    }
    let (v0, f0) = h::run(c, hero_yaw_turn, &[0], &[x, y, z]);
    c.ret(v0);
    c.fret(f0);
    true
}

/// 0x205130 hero_speed_slew(f12) -> v0/f0: slews the value at base+0x194 toward base+0x190 by `step` (f12 when the
/// value is below the target, else f13).
pub fn hero_speed_slew(c: &mut Cpu) -> bool {
    let (step_a, step_b) = (c.farg(0), c.farg(1));
    let target = c.rf(SLEW_TARGET);
    let value = c.rf(SLEW_VALUE);
    let step = if value < target { step_a } else { step_b };
    let (v0, f0) = h::run(c, motion::slew_toward, &[SLEW_VALUE], &[target, step]);
    c.ret(v0);
    c.fret(f0);
    true
}
