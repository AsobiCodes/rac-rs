//! Eased angle blending: `angle_rot` turns an angle toward another by an eased fraction, and `angle_blend` picks or
//! blends two 16-byte vectors with the same eased fraction.
use crate::h;
use ee_oracle::cpu::Cpu;
use crate::ps2::*;
use crate::trig;
use crate::vecmath;

/// Retail `jal` target for pi as a single: 0x40490fdb.
const PI: F = F(0x4049_0fdb);
const ONE: F = F(0x3f80_0000);
const HALF: F = F(0x3f00_0000);
const ZERO: F = F(0);

/// 0x1f1840 (out-of-line vector helper, args: tmp, b, a).
const VEC_DIFF: u32 = 0x1f1840;
/// 0x1f1810 (out-of-line vector helper, args: out, a, tmp).
const VEC_ADD: u32 = 0x1f1810;

/// Eased weight for a fraction `t` of a half turn: (1 - cos(t * pi)) / 2.
fn ease_weight(c: &mut Cpu, t: F) -> F {
    let (_, cos) = h::run(c, trig::fast_cos, &[], &[t * PI]);
    (ONE - cos) * HALF
}

/// 0x24a7b8 angle_rot(x, y, t) -> angle_add(x, angle_sub(y, x) * ease(t)): x moved toward y by the eased fraction t.
pub fn angle_rot(c: &mut Cpu) -> bool {
    let (x, y, t) = (c.farg(0), c.farg(1), c.farg(2));
    let old = [c.f[20], c.f[21], c.f[22]];
    h::frame(c, 0x30, &[(31, 0x0, h::SQ)], |c, sp| {
        c.w32(sp + 0x10, old[0]);
        c.w32(sp + 0x18, old[1]);
        c.w32(sp + 0x20, old[2]);
        let (_, d) = h::run(c, vecmath::angle_sub, &[], &[y, x]);
        let w = d * ease_weight(c, t);
        let (_, r) = h::run(c, vecmath::angle_add, &[], &[x, w]);
        c.f[12] = x.0;
        c.f[13] = w.0;
        c.f[1] = HALF.0;
        c.fret(r);
    });
    c.f[20] = old[0];
    c.f[21] = old[1];
    c.f[22] = old[2];
    true
}

/// 0x24a838 angle_blend(dst, a, b, t): dst = a when t == 0, b when t == 1, else a + (b - a) * ease(t) per lane
/// (retail: copies the 16-byte value at a or b, or runs the vector helpers on a temporary).
pub fn angle_blend(c: &mut Cpu) -> bool {
    let t = c.farg(0);
    let (dst, a, b) = (c.arg(0), c.arg(1), c.arg(2));
    let old_f20 = c.f[20];
    h::frame(
        c,
        0x60,
        &[(16, 0x10, h::SQ), (17, 0x20, h::SQ), (18, 0x30, h::SQ), (31, 0x40, h::SQ)],
        |c, sp| {
            c.w32(sp + 0x50, old_f20);
            if t == ZERO {
                let q = c.mem.rd128(a);
                c.mem.wr128(dst, q);
                c.r[2] = q;
            } else if t == ONE {
                let q = c.mem.rd128(b);
                c.mem.wr128(dst, q);
                c.r[2] = q;
            } else {
                let k = ease_weight(c, t);
                c.f[20] = k.0;
                c.f[0] = HALF.0;
                h::call_keep(c, VEC_DIFF, &[sp, b, a], &[]);
                c.f[20] = k.0;
                h::run_keep(c, vecmath::vec_scale, &[sp, sp], &[k]);
                h::call_keep(c, VEC_ADD, &[dst, a, sp], &[]);
            }
        },
    );
    c.f[20] = old_f20;
    true
}
