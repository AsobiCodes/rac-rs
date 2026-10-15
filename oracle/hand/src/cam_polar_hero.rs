//! Camera polar eye offset, camera eye-to-polar angles and the hero hand setup (retail 0x1dfde8, 0x1dfe98, 0x1f8028).
use crate::h::{self, SQ};
use crate::ps2::*;
use crate::hero_anim;
use ee_oracle::cpu::Cpu;

/// Camera config block (0x13f350) and the flags word at 0x140000 that the hero hand setup writes.
const CFG: u32 = 0x0014_0000 - 0xcb0;
const FLAGS: u32 = 0x0014_0000;
/// Retail `call 0x215538(0, 1)`: the hero hand's follow-up call.
const HAND_SETUP_CALL: u32 = 0x0021_5538;
/// Retail helpers (vector, angle and VU trig); called as retail so their VU and scratch leftovers match.
const VEC_ADD: u32 = 0x001f_1810;
const VEC_SUB: u32 = 0x001f_1840;
const LENGTH_XY: u32 = 0x001f_19a0;
const LENGTH3: u32 = 0x001f_1970;
const ATAN2: u32 = 0x001f_1e48;
const ANGLE_SUB: u32 = 0x001f_26c8;
const COS: u32 = 0x001f_1d80;
const SIN: u32 = 0x001f_1d98;

/// Cosine through the hand `trig::fast_cos`.
fn cos(c: &mut Cpu, x: F) -> F {
    h::call_keep(c, COS, &[], &[x]).1
}

/// Sine through the hand `trig::fast_sin`.
fn sin(c: &mut Cpu, x: F) -> F {
    h::call_keep(c, SIN, &[], &[x]).1
}

/// 0x1dfde8 cam_polar_eye(out, polar, target): polar = (az, el, dist) -> out = target + dist * (cos el cos az,
/// cos el sin az, sin el), with out.w = 0.
pub fn cam_polar_eye(c: &mut Cpu) -> bool {
    let (out, polar, target) = (c.arg(0), c.arg(1), c.arg(2));
    h::frame(c, 0x50, &[(31, 0x30, SQ), (18, 0x20, SQ), (17, 0x10, SQ), (16, 0x0, SQ)], |c, sp| {
        c.w32(sp + 0x40, c.f[20]);
        let el = c.rf(polar + 4);
        let cos_el = cos(c, el);
        let az = c.rf(polar);
        let cos_az = cos(c, az);
        let dist = c.rf(polar + 8);
        c.wf(out, dist * cos_el * cos_az);
        let el = c.rf(polar + 4);
        let cos_el = cos(c, el);
        let az = c.rf(polar);
        let sin_az = sin(c, az);
        let dist = c.rf(polar + 8);
        c.wf(out + 4, dist * cos_el * sin_az);
        let el = c.rf(polar + 4);
        let sin_el = sin(c, el);
        c.w32(out + 0xc, 0);
        let dist = c.rf(polar + 8);
        c.wf(out + 8, dist * sin_el);
        h::call_keep(c, VEC_ADD, &[out, out, target], &[]);
        c.f[20] = c.r32(sp + 0x40);
    });
    true
}

/// 0x1dfe98 cam_eye_to_polar(out, a, b): v = a - b (vec_sub into a stack buffer); out = (atan2(v.x, v.y),
/// atan2(|v.xy|, v.z), |v|); returns 0. The angle_sub call's result is unused, as in retail.
pub fn cam_eye_to_polar(c: &mut Cpu) -> bool {
    let (out, a, b) = (c.arg(0), c.arg(1), c.arg(2));
    h::frame(c, 0x40, &[(16, 0x10, SQ), (31, 0x20, SQ)], |c, sp| {
        c.w32(sp + 0x30, c.f[20]);
        h::call_keep(c, VEC_SUB, &[sp, a, b], &[]);
        let len = h::call_keep(c, LENGTH_XY, &[sp], &[]).1;
        let z = c.rf(sp + 8);
        let elevation = h::call_keep(c, ATAN2, &[], &[len, z]).1;
        c.wf(out + 4, elevation);
        let x = c.rf(sp);
        let y = c.rf(sp + 4);
        let angle = h::call_keep(c, ATAN2, &[], &[x, y]).1;
        let prev = c.rf(out);
        h::call_keep(c, ANGLE_SUB, &[], &[angle, prev]);
        c.wf(out, angle);
        let dist = h::call_keep(c, LENGTH3, &[sp], &[]).1;
        c.wf(out + 8, dist);
        c.ret(0);
        c.f[20] = c.r32(sp + 0x30);
    });
    true
}

/// 0x1f8028 hero_hand_init(): sets the hand's default blend values in the config block, clears the hand flags and
/// runs the hero hand clear and the follow-up call 0x215538(0, 1).
pub fn hero_hand_init(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(31, 0x0, SQ)], |c, _sp| {
        c.wf(CFG + 0x234, F(0x3ee6_6666));
        c.wf(CFG + 0xa94, F(0x3f80_0000));
        c.wf(CFG + 0x228, F(0x3f4c_cccd));
        c.wf(CFG + 0x22c, F(0x3f33_3333));
        c.w32(FLAGS + 0x1668, 0);
        c.wf(CFG + 0x220, F(0x3f4c_cccd));
        c.wf(CFG + 0x224, F(0x3f33_3333));
        c.w8(CFG + 0x22cb, 0);
        c.w32(CFG + 0x2084, 0);
        h::run_keep(c, hero_anim::hero_loop_clear_hand, &[], &[]);
        h::call_keep(c, HAND_SETUP_CALL, &[0, 1], &[]);
    });
    true
}
