//! Angle/length scaling, a vec4 lerp and masked word swaps (the same helpers in the Kerwan, Veldin and Novalis overlays).
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use crate::vu_geom;
use ee_oracle::cpu::Cpu;

/// Constant that the angle hand tests the vector's w lane against (retail loads it into f1 first).
const W_MARK: F = F(0x45af_df66);

/// Sign-extends a loaded word the way `lw` does.
fn sext(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Linear interpolation as retail computes it: a + (b - a) * t.
fn lerp_f(a: F, b: F, t: F) -> F {
    a + (b - a) * t
}

/// 0x24d708 angle_scale(v, out_angle, len, z) -> f0: *out_angle = atan2(v.x, v.y); when v.w == W_MARK also
/// *len *= |v.xy| and *z *= v.z. Returns the scaled z in f0.
pub fn angle_scale(c: &mut Cpu) -> bool {
    let vec = c.arg(0);
    let out_angle = c.arg(1);
    let len_ptr = c.arg(2);
    let z_ptr = c.arg(3);
    let saves = [(18, 0x30, h::SQ), (17, 0x20, h::SQ), (16, 0x10, h::SQ), (31, 0x40, h::SQ)];
    h::frame(c, 0x50, &saves, |c, sp| {
        // lq v0, 0(a0) and sq v0, 0(sp): the vector is copied into the frame.
        c.r[2] = c.mem.rd128(vec);
        c.mem.wr128(sp, c.r[2]);
        let x = c.rf(sp);
        let y = c.rf(sp + 4);
        let z = c.rf(sp + 8);
        let w = c.rf(sp + 0xc);
        c.f[1] = W_MARK.0;
        let (_, angle) = h::run_keep(c, vu_geom::fast_atan2, &[], &[x, y]);
        c.wf(out_angle, angle);
        if w == W_MARK {
            let len = h::run_keep(c, vecmath::length_xy, &[sp], &[]).1;
            let len_new = c.rf(len_ptr) * len;
            c.wf(len_ptr, len_new);
            c.w32(sp + 0xc, 0);
            let z_new = c.rf(z_ptr) * z;
            c.wf(z_ptr, z_new);
            c.f[1] = len_new.0;
            c.f[2] = z.0;
            c.fret(z_new);
        } else {
            c.fret(angle);
        }
        true
    })
}

/// 0x24e268 lerp_f(a = f12, b = f13, t = f14) -> f0: a + (b - a) * t.
pub fn lerp_scalar(c: &mut Cpu) -> bool {
    let (a, b, t) = (c.farg(0), c.farg(1), c.farg(2));
    c.fret(lerp_f(a, b, t));
    true
}

/// 0x24e278 lerp_vec4(out = a0, a = a1, b = a2, t = f12): out[i] = a[i] + (b[i] - a[i]) * t for the four words.
/// Leaves retail's last-lane registers: f12 = a[3], f13 = b[3], f14 = t, f0 = out[3].
pub fn lerp_vec4(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let (pa, pb) = (c.arg(1), c.arg(2));
    let t = c.farg(0);
    // addiu sp, sp, -0x60: the frame is written in place and sp is left as it was.
    let sp = (c.g(29) as u32).wrapping_sub(0x60);
    let qa = c.mem.rd128(pa);
    let qb = c.mem.rd128(pb);
    c.r[2] = qa;
    c.r[3] = qb;
    c.mem.wr128(sp, qa);
    c.mem.wr128(sp + 0x10, qb);
    c.mem.wr128(sp + 0x30, c.r[17]);
    c.mem.wr128(sp + 0x20, c.r[16]);
    c.mem.wr128(sp + 0x40, c.r[31]);
    c.w32(sp + 0x50, c.f[20] as u32);
    let mut last = (F(0), F(0), F(0));
    let mut res = F(0);
    for i in 0..4u32 {
        let a = c.rf(sp + 4 * i);
        let b = c.rf(sp + 0x10 + 4 * i);
        res = lerp_f(a, b, t);
        c.wf(out + 4 * i, res);
        last = (a, b, res);
    }
    c.f[12] = last.0 .0;
    c.f[13] = last.1 .0;
    c.f[14] = t.0;
    c.fret(res);
    true
}

/// 0x24ec48 swap_masked(p0 = a0, p1 = a1, p2 = a2, mask = a3): swaps *p0 and *p1 if mask bit 0, *p1 and *p2 if
/// bit 1, *p0 and *p2 if bit 2. v0/v1 end as retail's last lw/andi leave them.
pub fn swap_masked(c: &mut Cpu) -> bool {
    let (p0, p1, p2) = (c.arg(0), c.arg(1), c.arg(2));
    let mask = c.arg(3);
    if mask & 1 != 0 {
        let (a, b) = (c.r32(p1), c.r32(p0));
        c.w32(p0, a);
        c.w32(p1, b);
        c.s(2, sext(a));
        c.s(3, sext(b));
    }
    if mask & 2 != 0 {
        let (a, b) = (c.r32(p2), c.r32(p1));
        c.w32(p1, a);
        c.w32(p2, b);
        c.s(2, sext(a));
        c.s(3, sext(b));
    }
    if mask & 4 != 0 {
        let (a, b) = (c.r32(p0), c.r32(p2));
        c.w32(p2, a);
        c.w32(p0, b);
        c.s(2, sext(a));
        c.s(3, sext(b));
    } else {
        c.s(2, 0);
    }
    true
}
