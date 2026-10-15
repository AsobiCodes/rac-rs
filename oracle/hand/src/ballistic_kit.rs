//! Ballistic arc helpers: apex height, target field setup, position jitter and the launch block.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::{w4, Cpu};

/// Frame sizes (bytes below sp at entry) of the retail functions; their spill slots are written to RAM.
const APEX_FRAME: u32 = 0x60;
const SET_FIELDS_FRAME: u32 = 0x10;
const JITTER_FRAME: u32 = 0x30;
const LAUNCH_FRAME: u32 = 0x30;

/// z component offset of a vec3 (position or velocity).
const VEC_Z: u32 = 0x8;
/// Offsets inside the ballistic object.
const OBJ_GRAVITY: u32 = 0x10;
const OBJ_FIELD_14: u32 = 0x14;
const OBJ_FIELD_18: u32 = 0x18;
const OBJ_HEIGHT: u32 = 0x1c;
const OBJ_COUNT: u32 = 0x20;

/// MIPS register numbers used by the spills.
const GPR_S0: usize = 16;
const GPR_S1: usize = 17;
const GPR_S2: usize = 18;
const GPR_S3: usize = 19;
const GPR_RA: usize = 31;
/// Scratch registers the retail code writes.
const GPR_A0: usize = 4;
const FPR_F0: usize = 0;
const FPR_F1: usize = 1;
const FPR_F2: usize = 2;
const FPR_F12: usize = 12;
const FPR_F13: usize = 13;
const FPR_F20: usize = 20;
const FPR_F21: usize = 21;

/// Frame address of a retail function with the given frame size.
fn frame(c: &Cpu, size: u32) -> u32 {
    (c.g(29) as u32).wrapping_sub(size)
}

/// Sets float register `n` (f0..f19 only; the retail scratch registers).
fn put_f(c: &mut Cpu, n: usize, v: F) {
    c.f[n] = v.0;
}

/// 0x24bda8 arc_apex_z(pos*, vel*, n*, g) -> z: apex height z + vz*n - g*(n*n+n)/2 with n = trunc(vz/g);
/// writes n to *n when n* is not null.
pub fn arc_apex_z(c: &mut Cpu) -> bool {
    let (pos, vel, out_n) = (c.arg(0), c.arg(1), c.arg(2));
    let g = F(c.f[FPR_F12]);
    let sp = frame(c, APEX_FRAME);
    // sq s0..s3, sq ra, swc1 f20/f21 into the frame (the registers themselves are restored unchanged).
    c.wv(sp, V(w4(c.r[GPR_S0])));
    c.wv(sp + 0x10, V(w4(c.r[GPR_S1])));
    c.wv(sp + 0x20, V(w4(c.r[GPR_S2])));
    c.wv(sp + 0x30, V(w4(c.r[GPR_S3])));
    c.wv(sp + 0x40, V(w4(c.r[GPR_RA])));
    c.wf(sp + 0x50, F(c.f[FPR_F20]));
    c.wf(sp + 0x58, F(c.f[FPR_F21]));

    let vz = c.rf(vel + VEC_Z);
    let z = c.rf(pos + VEC_Z);
    let q = vz / g;
    let n = q.to_int();
    let nf = F::from_int(n);
    // n*n + n is even, so the arithmetic shift is the exact halving.
    let tri = n.wrapping_mul(n).wrapping_add(n) >> 1;
    let vz_n = vz * nf;
    let lifted = z + vz_n;
    let apex = lifted - F::from_int(tri) * g;
    if out_n != 0 {
        c.w32(out_n, n as u32);
    }
    // Leftover registers: v0 = n*n (the mult LO), a0 = n*(n+1)/2, f1 = z + vz*n, f2 = vz*n.
    c.ret((n.wrapping_mul(n)) as u32);
    c.s(GPR_A0, tri as i64 as u64);
    put_f(c, FPR_F1, lifted);
    put_f(c, FPR_F2, vz_n);
    // The retail cvt.w.s converts f12 in place, so f12 keeps the integer n bits.
    c.f[FPR_F12] = n as u32;
    c.fret(apex);
    true
}

/// 0x24c4b8 set_ballistic_fields(obj, a, b, f) -> obj: stores a and b at obj+0x10/+0x14, f at obj+0x1c, zero at
/// obj+0x20, clears the 16-byte vector at obj; returns obj.
pub fn set_ballistic_fields(c: &mut Cpu) -> bool {
    let (obj, a, b) = (c.arg(0), c.arg(1), c.arg(2));
    let f = F(c.f[FPR_F12]);
    let sp = frame(c, SET_FIELDS_FRAME);
    c.wv(sp, V(w4(c.r[GPR_RA])));
    c.w32(obj + OBJ_GRAVITY, a);
    c.w32(obj + OBJ_FIELD_14, b);
    c.wf(obj + OBJ_HEIGHT, f);
    c.w32(obj + OBJ_COUNT, 0);
    c.wv(obj, V::default());
    c.ret(obj);
    true
}

/// 0x2524a0 jitter_pos(pos*, r): adds a random float in [-r, r) to each of x, y, z of the vector at pos*.
pub fn jitter_pos(c: &mut Cpu) -> bool {
    let pos = c.arg(0);
    let r = F(c.f[FPR_F12]);
    let neg = -r;
    let sp = frame(c, JITTER_FRAME);
    c.wv(sp, V(w4(c.r[GPR_S0])));
    c.wv(sp + 0x10, V(w4(c.r[GPR_RA])));
    c.wf(sp + 0x20, F(c.f[FPR_F20]));
    c.wf(sp + 0x28, F(c.f[FPR_F21]));

    let mut last = F::ZERO;
    for k in 0..3u32 {
        put_f(c, FPR_F12, neg);
        put_f(c, FPR_F13, r);
        h::run_keep(c, crate::rand::rand_float, &[], &[neg, r]);
        let d = F(c.f[FPR_F0]);
        last = c.rf(pos + 4 * k) + d;
        c.wf(pos + 4 * k, last);
    }
    // The last rand call left f0 = its draw; retail leaves f1 = the new z and f12/f13 = -r / r.
    put_f(c, FPR_F12, neg);
    put_f(c, FPR_F13, r);
    put_f(c, FPR_F1, last);
    true
}

/// 0x254ce0 launch_block(obj, d, h): sets height = sqrt(2*h*g), t = 2*sqrt(2*h*g)/g, +0x18 = 2d/t, +0x14 = 2d/t^2
/// (g is the field at obj+0x10).
pub fn launch_block(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let d = F(c.f[FPR_F12]);
    let height = F(c.f[FPR_F13]);
    let sp = frame(c, LAUNCH_FRAME);
    c.wv(sp, V(w4(c.r[GPR_S0])));
    c.wv(sp + 0x10, V(w4(c.r[GPR_RA])));
    c.wf(sp + 0x20, F(c.f[FPR_F20]));

    let g = c.rf(obj + OBJ_GRAVITY);
    let two_h = height + height;
    // sqrt_q leaves its own scratch state (exactly as the retail call); f0 holds the root.
    h::run_keep(c, crate::mathx::sqrt_q, &[], &[two_h * g]);
    let root = F(c.f[FPR_F0]);
    let two_d = d + d;
    let t = (root + root) / g;
    let t2 = t * t;
    let f18 = two_d / t;
    let f14 = two_d / t2;
    c.wf(obj + OBJ_HEIGHT, root);
    c.wf(obj + OBJ_FIELD_18, f18);
    c.wf(obj + OBJ_FIELD_14, f14);
    put_f(c, FPR_F13, two_h);
    put_f(c, FPR_F1, f18);
    put_f(c, FPR_F2, g);
    c.fret(t2);
    true
}
