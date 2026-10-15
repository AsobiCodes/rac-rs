//! Hero per-frame helpers: ground snap and spring (0x206098), push-vector decay (0x206360), and the dot of a
//! 16-byte vector with the hero heading (0x2063f8). The hero state block lives at 0x13f350.
use crate::h;
use crate::motion;
use crate::ps2::*;
use crate::scene;
use crate::trig;
use ee_oracle::cpu::Cpu;

/// Hero state block (retail `s0`).
const HERO: u32 = 0x0013_f350;
/// Push vector written by the spherical-to-xyz step of `hero_push_decay`.
const PUSH_VEC: u32 = 0x0013_fc70;

const OFF_E0: u32 = 0xe0;
const OFF_E8: u32 = 0xe8;
const OFF_HEADING: u32 = 0x98;
const OFF_108: u32 = 0x108;
const OFF_210: u32 = 0x210;
const OFF_234: u32 = 0x234;
const OFF_257: u32 = 0x257;
const OFF_270: u32 = 0x270;
const OFF_2A0: u32 = 0x2a0;
const OFF_2D8: u32 = 0x2d8;
const OFF_2DC: u32 = 0x2dc;
const OFF_2084: u32 = 0x2084;
const OFF_20B3: u32 = 0x20b3;
const OFF_30E: u32 = 0x30e;
const OFF_POS: u32 = 0x80;
const OFF_Z: u32 = 0x88;
const OFF_VEL: u32 = 0x304;

/// Constants stored in retail RAM (gp-free, loaded with lui/lwc1 at 0x16xxxx).
const CONST_STEP_SCALE: u32 = 0x0015_ed60;
const CONST_SPRING_BIAS: u32 = 0x0015_ed64;
const CONST_SPRING_GAIN: u32 = 0x0015_ed6c;

const EPS_SNAP: u32 = 0x38d1_b717;
const EPS_SLACK: u32 = 0x3c23_d70a;
const RATIO_SHORT: u32 = 0x3e57_0a3d;
const RATIO_TINY: u32 = 0x3d4c_cccd;
const HALF: u32 = 0x3f00_0000;
const SPRING_K_Z: u32 = 0x3d69_78d5;
const SPRING_K_V: u32 = 0x3e99_999a;

/// 0x206098 hero_step_snap(): while airborne (flag at 0x20b3) pulls the position toward its target; on the ground
/// it zeroes the spring velocity or springs z back to the ground height.
pub fn hero_step_snap(c: &mut Cpu) -> bool {
    if c.r8(HERO + OFF_20B3) != 0 {
        snap_airborne(c);
    } else {
        snap_grounded(c);
    }
    true
}

/// Flag set: blend the position toward the target at 0x2a0, capped by the step length.
fn snap_airborne(c: &mut Cpu) {
    let s0 = HERO;
    let f2 = h::dot3(c, s0 + OFF_E0, s0 + OFF_270);
    if c.r16(s0 + OFF_30E) != 0 {
        return;
    }
    let f1 = c.rf(s0 + OFF_2DC);
    if !(f1 < F::ZERO) {
        return;
    }
    if f1 < f2 + F(EPS_SNAP) || c.r32(s0 + OFF_2084) == 0x3e {
        copy_vec(c, s0 + OFF_2A0, s0 + OFF_POS);
        return;
    }
    pull_toward_target(c);
}

/// pos += normalize(target - pos) * step, with the step length clamped to a fraction of the constant at 0x15ed60.
fn pull_toward_target(c: &mut Cpu) {
    let s0 = HERO;
    h::with_stack(c, 0x40, |c, tmp| {
        h::vec_sub(c, tmp, s0 + OFF_2A0, s0 + OFF_POS);
        let len = h::length3(c, tmp);
        let scale = c.rf(CONST_STEP_SCALE);
        let step = if !(len < scale * F(RATIO_SHORT)) {
            len * F(HALF)
        } else if scale * F(RATIO_TINY) < len {
            scale * F(RATIO_TINY)
        } else {
            len
        };
        h::vec_set_length(c, tmp, tmp, step);
        h::vec_add(c, s0 + OFF_POS, s0 + OFF_POS, tmp);
    });
}

/// Flag clear: the ground-following path.
fn snap_grounded(c: &mut Cpu) {
    let s0 = HERO;
    if c.r16(s0 + OFF_30E) != 0 {
        c.w32(s0 + OFF_VEL, 0);
        return;
    }
    let z = c.rf(s0 + OFF_Z);
    let ground = c.rf(s0 + OFF_2D8);
    if !(z < ground) {
        c.w32(s0 + OFF_VEL, 0);
        return;
    }
    let dz = (z - ground).abs();
    let slack = c.rf(s0 + OFF_E8).abs() + F(EPS_SLACK);
    if !(dz < slack) {
        // Far from the ground: spring with a bias taken from the signed offset at 0x108.
        let f2 = c.rf(s0 + OFF_108);
        let k = if f2 < F::ZERO {
            let t = c.rf(s0 + OFF_Z) - f2;
            let g = c.rf(s0 + OFF_2D8);
            c.wf(s0 + OFF_Z, t);
            if g < t {
                c.wf(s0 + OFF_Z, g);
            }
            c.rf(CONST_SPRING_BIAS)
        } else {
            F::ZERO
        };
        settle(c, k);
        return;
    }
    if c.r8(s0 + OFF_257) == 0 {
        let g = c.rf(s0 + OFF_2D8);
        c.wf(s0 + OFF_Z, g);
        return;
    }
    let dist = h::dist_xy(c, s0 + OFF_210, s0 + OFF_POS);
    let limit = c.rf(s0 + OFF_234) * F(HALF);
    if limit < dist {
        let g = c.rf(s0 + OFF_2D8);
        c.wf(s0 + OFF_Z, g);
        return;
    }
    settle(c, dist);
}

/// Runs the spring-settle helper on (z, vel) with ground height 0x2d8 and gain `k` scaling the spring constants.
fn settle(c: &mut Cpu, k: F) {
    let s0 = HERO;
    let target = c.rf(s0 + OFF_2D8);
    let kz = k * F(SPRING_K_Z);
    let kv = k * F(SPRING_K_V);
    let gain = c.rf(CONST_SPRING_GAIN);
    let ret = h::run(c, motion::spring_settle_hand, &[s0 + OFF_Z, s0 + OFF_VEL], &[target, kz, kv, gain + gain]);
    c.fret(ret.1);
}

fn copy_vec(c: &mut Cpu, src: u32, dst: u32) {
    let v = c.rv(src);
    c.wv(dst, v);
}

/// 0x206360 hero_push_decay(k, floor): rebuilds the push vector from the yaw/pitch/radius at 0x13fc80 scaled by `k`,
/// then shortens its length by `floor` (zeroing it when it is already at or below the floor).
pub fn hero_push_decay(c: &mut Cpu) -> bool {
    let (k, floor) = (c.farg(0), c.farg(1));
    let radius = c.rf(HERO + 0x930);
    let pitch = c.rf(HERO + 0x934);
    let yaw = c.rf(HERO + 0x938);
    h::run(c, scene::spherical_to_xyz, &[PUSH_VEC], &[radius, yaw, pitch]);
    h::vec_scale(c, PUSH_VEC, PUSH_VEC, k);
    let len = h::length3(c, PUSH_VEC);
    if len <= floor {
        h::call(c, 0x001f_17f8, &[PUSH_VEC], &[]);
    } else {
        h::vec_set_length(c, PUSH_VEC, PUSH_VEC, len - floor);
    }
    true
}

/// 0x2063f8 hero_forward_dot(v*) -> dot of the vector at `v` (xyz) with (cos, sin, 0) of the hero heading at 0x98.
pub fn hero_forward_dot(c: &mut Cpu) -> bool {
    let v = c.rv(c.arg(0));
    let heading = c.rf(HERO + OFF_HEADING);
    crate::h::with_stack(c, 0x40, |c, buf| {
        c.wv(buf, v);
        let cos = h::run(c, trig::fast_cos, &[], &[heading]).1;
        c.wf(buf + 0x10, cos);
        let sin = h::run(c, trig::fast_sin, &[], &[heading]).1;
        c.wf(buf + 0x14, sin);
        c.w32(buf + 0x18, 0);
        let d = h::dot3(c, buf, buf + 0x10);
        c.fret(d);
    });
    true
}
