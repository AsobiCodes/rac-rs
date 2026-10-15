//! Walker controller setup and the per-axis angle blend used by the walker's orbit step.
use crate::h;
use crate::ps2::*;
use crate::trig;
use crate::vecmath::wrap_once;
use ee_oracle::cpu::Cpu;

/// pi as an f32 constant (0x40490fdb)
const PI: F = F(0x4049_0fdb);
const ONE: F = F(0x3f80_0000);
const HALF: F = F(0x3f00_0000);

/// Walker controller block defaults.
const WALKER_RADIUS: u32 = 0x280;
const WALKER_VOFF: F = ONE;
const WALKER_STEPS: F = HALF;
const WALKER_GAIN: F = F(0x3e99_999a); // 0.3
const WALKER_CAP: F = F(0x402c_cccd); // 2.7
const WALKER_THRESHOLD: F = F(0x3fc8_f5c3); // 1.57
const WALKER_ARRIVE: F = F(0x3dcc_cccd); // 0.1

/// Block field offsets.
const OFF_RADIUS: u32 = 0x00;
const OFF_VOFF: u32 = 0x04;
const OFF_STEPS_A: u32 = 0x08;
const OFF_STEPS_B: u32 = 0x0c;
const OFF_ZERO_A: u32 = 0x10;
const OFF_ZERO_B: u32 = 0x14;
const OFF_ZERO_C: u32 = 0x18;
const OFF_GAIN_A: u32 = 0x1c;
const OFF_GAIN_B: u32 = 0x20;
const OFF_CAP: u32 = 0x24;
const OFF_THRESHOLD: u32 = 0x28;
const OFF_ARRIVE: u32 = 0x2c;

/// 0x24a918 orbit_blend(out, a, b, angle) -> (): writes out[i] = a[i] + (b[i] - a[i]) * w for the three
/// axes, where w = (1 - cos(angle * pi)) / 2 and every difference is wrapped to the angle range.
pub fn orbit_blend(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let a = c.arg(1);
    let b = c.arg(2);
    let angle = c.farg(0);
    let (_, cos) = h::run(c, trig::fast_cos, &[], &[angle * PI]);
    let weight = (ONE - cos) * HALF;
    let mut last_a = F(0);
    let mut last_step = F(0);
    let mut last_r = F(0);
    for i in 0..3u32 {
        let off = 4 * i;
        last_a = c.rf(a + off);
        let pb = c.rf(b + off);
        let delta = wrap_once(pb - last_a);
        last_step = delta * weight;
        last_r = wrap_once(last_a + last_step);
        c.wf(out + off, last_r);
    }
    // Scratch registers as the retail loop leaves them.
    c.f[0] = last_r.0;
    c.f[1] = HALF.0;
    c.f[12] = last_a.0;
    c.f[13] = last_step.0;
    // $at as the retail wrap helper leaves it: its PI constant in the low word.
    c.r[1] = ((0x33ee_d883_b713_bf63_u128) << 64 | 0x4049_0fdb_u128) as _;
    true
}

/// 0x24b418 walker_init(block) -> v0 0x280: fills the walker controller block with its default tuning
pub fn walker_init(c: &mut Cpu) -> bool {
    let p = c.arg(0);
    c.w32(p + OFF_RADIUS, WALKER_RADIUS);
    c.wf(p + OFF_VOFF, WALKER_VOFF);
    c.wf(p + OFF_STEPS_A, WALKER_STEPS);
    c.wf(p + OFF_STEPS_B, WALKER_STEPS);
    c.w32(p + OFF_ZERO_A, 0);
    c.w32(p + OFF_ZERO_B, 0);
    c.w32(p + OFF_ZERO_C, 0);
    c.wf(p + OFF_GAIN_A, WALKER_GAIN);
    c.wf(p + OFF_GAIN_B, WALKER_GAIN);
    c.wf(p + OFF_CAP, WALKER_CAP);
    c.wf(p + OFF_THRESHOLD, WALKER_THRESHOLD);
    c.wf(p + OFF_ARRIVE, WALKER_ARRIVE);
    // Scratch registers as the retail code leaves them.
    c.f[0] = WALKER_ARRIVE.0;
    c.f[1] = WALKER_VOFF.0;
    c.f[2] = WALKER_CAP.0;
    c.f[3] = WALKER_THRESHOLD.0;
    c.f[4] = WALKER_STEPS.0;
    c.f[5] = WALKER_GAIN.0;
    c.r[1] = WALKER_ARRIVE.0 as _;
    c.ret(WALKER_RADIUS);
    true
}
