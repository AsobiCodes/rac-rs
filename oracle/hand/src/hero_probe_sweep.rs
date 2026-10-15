//! Hero probe sweep, tilt/spring settle and mode selector from overlay l4_04. The retail control flow is kept:
//! frame-gated probes (0x214450), the yaw spring settle (0x214ca8) and the mode pick (0x214ed8). Retail callees
//! with no Rust twin are called through `h::call`.
use crate::calls::{preserved, HERO};
use crate::h;
use crate::ps2::*;
use crate::{hero_front_probe, hero_space, motion, path, trig, vecmath};
use ee_oracle::cpu::Cpu;

/// Register number of gp, used for the gp-relative words and floats.
const GP_REG: usize = 28;
/// Frame counter word; the probe sweep runs on frames divisible by 3, and its probe pass on multiples of 5.
const FRAME_COUNTER: u32 = 0x15_f5ac;
/// Float -20.0 (the sweep scale and the spring range base).
const NEG_TWENTY_ADDR: u32 = 0x15_ed6c;
/// Float spring base (second spring constant source).
const SPRING_BASE_ADDR: u32 = 0x15_ed64;
/// Yaw record of the camera/target block (0x173f40).
const YAW_BLOCK: u32 = 0x17_3f40;
/// Point the sweep measures its first hit against (0x173f60).
const SWEEP_TARGET: u32 = 0x17_3f60;

/// Hero state offsets used by this file.
const OFF_INIT_FLAG: u32 = 0x20ad;
const OFF_ACTOR: u32 = 0x2080;
const OFF_KIND: u32 = 0x208c;
const OFF_TILT_GATE: u32 = 0x257;
const OFF_FEET_RADIUS: u32 = 0x234;
const OFF_SEARCH: u32 = 0x258;
const OFF_SEARCH_HEIGHT: u32 = 0x25c;
const OFF_SEARCH_FLAG: u32 = 0x260;
const OFF_PROBE_DIST: u32 = 0x248;
const OFF_PROBE_ANGLE: u32 = 0x250;
const OFF_PROBE_FLAG_A: u32 = 0x254;
const OFF_PROBE_FLAG_B: u32 = 0x255;
const OFF_CHECK_WORD: u32 = 0x300;
const OFF_SPRING_VEL: u32 = 0x244;
const OFF_STEP: u32 = 0x1ea;

/// Retail routines called without a Rust twin.
const INIT_ROUTINE: u32 = 0x20_6928;
const SEGMENT_TEST: u32 = 0x1e_24d8;
const PROBE_SWEEP: u32 = 0x1e_35c8;

/// Float constants with their exact bits.
const F_0_7: u32 = 0x3f33_3333;
const F_4_0: u32 = 0x4080_0000;
const F_3_0: u32 = 0x4040_0000;
const F_1_1: u32 = 0x3f8c_cccd;
const F_1_0: u32 = 0x3f80_0000;
const F_0_5: u32 = 0x3f00_0000;
const F_0_1: u32 = 0x3dcc_cccd;
const F_NEG_0_4: u32 = 0xbecc_cccd;
const F_20_0: u32 = 0x41a0_0000;
const F_NEG_20_0: u32 = 0xc1a0_0000;
const F_0_2: u32 = 0x3e4c_cccd;
const F_0_9: u32 = 0x3f66_6666;
const F_0_05: u32 = 0x3d4c_cccd;
const F_0_03: u32 = 0x3cf5_c28f;
const F_0_3: u32 = 0x3e99_999a;
const F_2_8: u32 = 0x4033_3333;

/// 0x214450: frame-gated hero probe. Every 3rd frame it refreshes the probe distance, angle and flags; every 5th
/// frame it runs a two-point sweep and sets the search state at 0x258..0x25c.
pub fn hero_probe_update(c: &mut Cpu) -> bool {
    preserved(c, false, false, probe_update_body)
}

fn probe_update_body(c: &mut Cpu) -> bool {
    if c.r8(HERO + OFF_INIT_FLAG) == 0 {
        h::call(c, INIT_ROUTINE, &[], &[]);
    }
    let frame = c.r32(FRAME_COUNTER) as i32;
    let gp = c.g(GP_REG) as u32;
    // Retail a0 is the frame counter on the non-3 path; on the 3rd-frame path it is the gp word unless the probe hits.
    let mut a0 = frame;
    if frame % 3 == 0 {
        a0 = c.r32(gp.wrapping_sub(0x7654)) as i32;
        c.wf(HERO + OFF_PROBE_DIST, F(F_4_0));
        c.w8(HERO + OFF_PROBE_FLAG_A, 0);
        c.w8(HERO + OFF_PROBE_FLAG_B, 0);
        let hit = h::run(c, hero_front_probe::hero_front_probe, &[HERO + OFF_PROBE_DIST], &[F(F_0_7), F(F_4_0)]).0;
        if hit != 0 {
            let len = h::run(c, vecmath::length_xy, &[YAW_BLOCK + 0x40], &[]).1;
            let dy = c.rf(YAW_BLOCK + 0x48);
            let angle = h::run(c, trig::fast_atan2, &[], &[dy, len]).1;
            c.wf(HERO + OFF_PROBE_ANGLE, angle);
            let owner = c.r32(YAW_BLOCK + 0x18);
            if owner != 0 {
                c.w8(HERO + OFF_PROBE_FLAG_B, 1);
                if h::run(c, path::is_class_500_540, &[owner], &[]).0 != 0 {
                    c.w8(HERO + OFF_PROBE_FLAG_A, 1);
                }
            }
            a0 = frame;
        }
    }
    if a0 % 5 != 0 {
        return true;
    }
    c.w32(HERO + OFF_SEARCH_FLAG, 0);
    c.w32(HERO + OFF_SEARCH, 0);
    if c.r32(HERO + OFF_CHECK_WORD) == 0 {
        return true;
    }
    h::with_stack(c, 0xd0, |c, buf| probe_sweep(c, buf));
    true
}

/// The sweep of 0x214450: two sample points along the hero's forward axis, a distance test on the first, then a
/// loop over x that records the first clear sample's height.
fn probe_sweep(c: &mut Cpu, buf: u32) {
    let zero = F(0);
    let step_x = F(F_1_1);
    let (p0, p1, p2) = (buf, buf + 0x10, buf + 0x20);
    h::run(c, hero_space::point_from_offset, &[p0], &[step_x, zero, F(F_1_0)]);
    h::run(c, hero_space::point_from_offset, &[p1], &[step_x, zero, F(F_NEG_20_0)]);
    if c.rf(p1 + 8) < F(F_0_5) {
        c.wf(p1 + 8, F(F_0_5));
    }
    let actor = c.r32(HERO + OFF_ACTOR);
    let hit = h::call(c, SEGMENT_TEST, &[p0, p1, 2, actor, 0], &[]).0;
    let mut dist = F(F_20_0);
    if hit != 0 {
        let d = h::run(c, vecmath::dist3, &[p0, SWEEP_TARGET], &[]).1;
        if !(F(F_3_0) < d) {
            return;
        }
        dist = d;
    }
    let half = F(F_0_5);
    let step = F(F_0_1);
    let mut f20 = F(F_NEG_0_4);
    loop {
        h::run(c, hero_space::point_from_offset, &[p2], &[f20 + step_x, zero, zero]);
        let radius = c.rf(HERO + OFF_FEET_RADIUS);
        let blocked = h::call(c, PROBE_SWEEP, &[p2, 2, 0], &[radius]).0;
        if blocked == 0 {
            c.w32(HERO + OFF_SEARCH_FLAG, 1);
            c.wf(HERO + OFF_SEARCH, if hit != 0 { dist } else { F(F_20_0) });
            let height = radius + f20;
            c.wf(HERO + OFF_SEARCH_HEIGHT, if height < zero { zero } else { height });
            return;
        }
        f20 = f20 + step;
        if !(f20 < half) {
            return;
        }
    }
}

/// 0x214ca8: when the tilt gate passes, rescales the step at 0x1ea; then, if that step is set and the hero is
/// close enough to its target, springs the angle state and writes the cos/sin velocity outputs at 0xf0..0xf8.
pub fn hero_tilt_settle(c: &mut Cpu) -> bool {
    preserved(c, false, false, tilt_settle_body)
}

fn tilt_settle_body(c: &mut Cpu) -> bool {
    if c.r8(HERO + OFF_TILT_GATE) != 0 && tilt_gate(c) {
        let step = h::scale_int(c, 5);
        c.w16(HERO + OFF_STEP, step as u16);
    }
    if c.r16(HERO + OFF_STEP) as i16 == 0 {
        c.w32(HERO + OFF_SPRING_VEL, 0);
        return true;
    }
    let dist = h::run(c, vecmath::dist_xy, &[HERO + 0x80, HERO + 0x210], &[]).1;
    let radius = c.rf(HERO + OFF_FEET_RADIUS);
    let near = radius * F(F_0_9);
    if !(dist < near) {
        return true;
    }
    let dx = c.rf(HERO + 0x80) - c.rf(HERO + 0x210);
    let dy = c.rf(HERO + 0x84) - c.rf(HERO + 0x214);
    let mut angle = h::run(c, trig::fast_atan2, &[], &[dx, dy]).1;
    if dist < radius * F(F_0_05) {
        angle = c.rf(HERO + 0x98);
    }
    let k_range = c.rf(NEG_TWENTY_ADDR);
    let k_base = c.rf(SPRING_BASE_ADDR);
    let k_max = k_range * F(F_2_8);
    let k_damp = k_base * F(F_0_03);
    let k_step = k_base * F(F_0_3);
    let spring_in = dist - near;
    h::with_stack(c, 0x60, |c, buf| {
        c.wf(buf, spring_in);
        h::run(c, motion::spring_settle_hand, &[buf, HERO + OFF_SPRING_VEL], &[F(0), k_damp, k_step, k_max]);
    });
    let cos = h::run(c, trig::fast_cos, &[], &[angle]).1;
    let vel = c.rf(HERO + OFF_SPRING_VEL);
    c.wf(HERO + 0xf0, cos * vel);
    let sin = h::run(c, trig::fast_sin, &[], &[angle]).1;
    c.wf(HERO + 0xf8, F(0));
    let vel = c.rf(HERO + OFF_SPRING_VEL);
    c.wf(HERO + 0xf4, sin * vel);
    true
}

/// Gate of the tilt settle: the 0x2dc speed test, the 0x160 limit against gp float, the 0xe0 length test, the
/// 0x229c test and the mode word 0x208c.
fn tilt_gate(c: &mut Cpu) -> bool {
    if !(F(0) < c.rf(HERO + 0x2dc)) {
        return false;
    }
    let gp = c.g(GP_REG) as u32;
    let limit = c.rf(gp.wrapping_sub(0x7e94)) * F(F_3_0);
    if !(c.rf(HERO + 0x160) < limit) {
        return false;
    }
    let len = h::run(c, vecmath::length3, &[HERO + 0xe0], &[]).1;
    if !(len < c.rf(NEG_TWENTY_ADDR) * F(F_3_0)) {
        return false;
    }
    if !(c.rf(HERO + 0x229c) < F(F_0_2)) {
        return false;
    }
    matches!(c.r32(HERO + OFF_KIND), 0 | 1 | 2 | 4 | 10)
}

/// 0x214ed8: picks a mode code from the helper at 0x214f38. The helper's result differs from retail in the worker's
/// checks (it reads hero state it does not model), so this path is left to retail: `return false`.
pub fn hero_mode_select(_c: &mut Cpu) -> bool {
    false
}
