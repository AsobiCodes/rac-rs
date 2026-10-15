//! Hero surface push and gait-block scaling from the L4_03 record (0x205268 and 0x206e90).
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Global block the record code reads (`lui 0x14` + `addiu -0xcb0`).
const BLOCK: u32 = 0x0013_F350;
/// Probe distance (float) that must be under the threshold for a push.
const PROBE_LIMIT: F = F(0x3e19_999a);
/// Lower bound of the heading test (float).
const HEADING_MIN: F = F(0x3f5f_66f3);
/// Scale factor applied to the surface offset (0x42580000 = 54.0).
const PUSH_GAIN: F = F(0x4258_0000);
/// Address of the float constant the push is scaled by (`lui 0x16`, `-0x1290`).
const PUSH_CONST: u32 = 0x0015_ED70;

/// 0x205268 surface_push(): when the hero's probe value (0x2dc) is under 0.15, the timer (0x108) and the speed (0x208)
/// are positive, and the heading test passes, pushes the surface offset (0xe0, 0xe4, 0xe8) away from the surface.
pub fn surface_push(c: &mut Cpu) -> bool {
    let saved_f20 = c.f[20];
    h::frame(c, 0x50, &[(16, 0x10, h::SQ), (17, 0x20, h::SQ), (31, 0x30, h::SQ)], |c, sp| {
        c.wf(sp + 0x40, F(saved_f20));
        push_body(c, BLOCK, sp);
    });
    true
}

fn push_body(c: &mut Cpu, s0: u32, sp: u32) {
    if c.r16(s0 + 0x30e) == 0 || !(c.rf(s0 + 0x2dc) < PROBE_LIMIT) {
        return;
    }
    if !(F::ZERO < c.rf(s0 + 0x108)) || c.r8(s0 + 0x257) == 0 || !(F::ZERO < c.rf(s0 + 0x208)) {
        return;
    }
    let s1 = s0 + 0x200;
    let (_, len) = h::run_keep(c, vecmath::length_xy, &[s1], &[]);
    let speed = c.rf(s0 + 0x208);
    let (_, angle) = h::call_keep(c, 0x1f1e48, &[], &[speed, len]);
    if !(HEADING_MIN <= angle) {
        return;
    }
    let block = c.rv(s1);
    c.wv(sp, block);
    c.wf(sp + 8, F::ZERO);
    h::call_keep(c, 0x125180, &[sp, sp], &[]);
    let p = (-c.rf(s0 + 0xe0)) * c.rf(sp);
    let q = c.rf(s0 + 0xe4) * c.rf(sp + 4);
    let d = p - q;
    if !(F::ZERO < d) {
        return;
    }
    h::run_keep(c, vecmath::vec_scale, &[sp, sp], &[d]);
    h::call_keep(c, 0x1f1810, &[s0 + 0xe0, s0 + 0xe0, sp], &[]);
    let bump = c.rf(PUSH_CONST) * PUSH_GAIN;
    let e8 = c.rf(s0 + 0xe8) - (bump + bump);
    c.wf(s0 + 0xe8, e8);
}

/// 0x206e90 scale_block(k): scales the vectors at 0x13f430, 0x13f450, 0x13f460 and 0x13f4a0 by k, and the gait value
/// at 0x13f4e4 by k.
pub fn scale_block(c: &mut Cpu) -> bool {
    let k = c.farg(0);
    let saved_f20 = c.f[20];
    let s0 = BLOCK + 0x100;
    h::frame(c, 0x30, &[(16, 0x00, h::SQ), (31, 0x10, h::SQ)], |c, sp| {
        c.wf(sp + 0x20, F(saved_f20));
        for out in [s0, s0 + 0x10, s0.wrapping_sub(0x20), s0 + 0x50] {
            h::run_keep(c, vecmath::vec_scale, &[out, out], &[k]);
        }
    });
    let gait = BLOCK + 0x194;
    let v = c.rf(gait) * k;
    c.wf(gait, v);
    true
}
