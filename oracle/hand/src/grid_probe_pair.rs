//! Grid probe pair (retail 0x24acb8 probe_clamped and 0x24b398 probe_scaled_pair, overlay rec/l4_08).
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Limits block at 0x173f40: word +0x18 forces the probe result to 0 when the flag is set; word +0x1c (signed) must be
/// greater than -1 for the probe result to be kept.
const LIMITS: u32 = 0x173f40;

/// 0x24acb8 probe_clamped(a0 = vec4, a1, a2 = flag; f12 = z lift, f13 = z floor) -> grid probe (0x1e24d8) result.
/// Two stack copies of the vec4 are made (frame sp and sp+0x10): the first gets z += f12, the second z = max(z - f13, 0).
pub fn probe_clamped(c: &mut Cpu) -> bool {
    let src = c.arg(0);
    let a1 = c.arg(1);
    let flag = c.arg(2);
    let lift = c.farg(0);
    let floor = c.farg(1);
    h::frame(c, 0x40, &[(16, 0x20, h::SQ), (31, 0x30, h::SQ)], |c, a| {
        let b = a + 0x10;
        for i in 0..4 {
            let w = c.r32(src + 4 * i);
            c.w32(a + 4 * i, w);
            c.w32(b + 4 * i, w);
        }
        let z = c.rf(a + 8) + lift;
        c.wf(a + 8, z);
        let d = c.rf(b + 8) - floor;
        c.wf(b + 8, if d < F::ZERO { F::ZERO } else { d });
        let (v0, _) = h::call_keep(c, 0x1e24d8, &[a, b, 0x22, a1, 0], &[]);
        let lim = c.r32(LIMITS + 0x1c) as i32;
        let mut res = if -1 < lim { v0 } else { 0 };
        if flag != 0 && c.r32(LIMITS + 0x18) != 0 {
            res = 0;
        }
        c.ret(res);
        c.set_arg(0, res);
        c.set_arg(1, lim as u32);
        c.s(3, LIMITS as u64);
    });
    true
}

/// 0x24b398 probe_scaled_pair(a0, a1, a2 = count, a3; f12, f13) -> 0x24b0f8(a0, a1, a3, a3; f12, count / 1024, f13).
pub fn probe_scaled_pair(c: &mut Cpu) -> bool {
    let a0 = c.arg(0);
    let a1 = c.arg(1);
    let a2 = c.arg(2);
    let a3 = c.arg(3);
    let f12 = c.farg(0);
    let f13 = c.farg(1);
    h::frame(c, 0x50, &[(16, 0, h::SQ), (17, 0x10, h::SQ), (18, 0x20, h::SQ), (31, 0x30, h::SQ)], |c, _sp| {
        let (_, f0) = h::run_keep(c, mathx::int_to_float, &[a2], &[]);
        let scale = f0 * F(0x3a80_0000);
        h::call_keep(c, 0x24b0f8, &[a0, a1, a3, a3], &[f12, scale, f13]);
    });
    true
}
