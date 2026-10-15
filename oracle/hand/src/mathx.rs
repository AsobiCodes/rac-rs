//! Scalar, angle and memory helpers (boot-area twins in every overlay; Eudora addresses).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// gp-relative float scale shared by `fixed_to_int` and `scale_float` (gp - 0x7e98).
const GP_SCALE: u32 = 0x7e98;
/// gp-relative arccos polynomial coefficients c0..c3 (gp - 0x6520).
const GP_ACOS_POLY: u32 = 0x6520;
const HALF_PI: F = F(0x3fc9_0fdb);
const QUARTER: F = F(0x3e80_0000);

fn gp_float(c: &mut Cpu, back: u32) -> F {
    let a = (c.g(28) as u32).wrapping_sub(back);
    c.rf(a)
}

/// 0x1f14a8 scale_float(x) -> x * gp-scale.
pub fn scale_float(c: &mut Cpu) -> bool {
    let k = gp_float(c, GP_SCALE);
    c.fret(k * c.farg(0));
    true
}

/// (int)(n * gp-scale + 0.5) as 0x1f14b8 computes it.
pub fn scale_int(c: &mut Cpu, n: i32) -> i32 {
    let k = gp_float(c, GP_SCALE);
    (QUARTER + QUARTER + F::from_int(n) * k).to_int()
}

/// 0x1f14b8 float_to_int_scaled(n) -> (int)(n * gp-scale + 0.5): n converted to float, multiply-accumulated onto
/// 0.25 + 0.25, truncated (cvt.w.s).
pub fn float_to_int_scaled(c: &mut Cpu) -> bool {
    let k = gp_float(c, GP_SCALE);
    let n = F::from_int(c.arg(0) as i32);
    let acc = QUARTER + QUARTER;
    c.ret((acc + n * k).to_int() as u32);
    true
}

/// Counter tick on a value: 0 -> 1 (nothing stored); otherwise stores max(v, 1) - 1 and returns 0 while it is still
/// positive, 2 when it just reached 0 (or below).
pub fn tick_down(v: i32) -> (Option<i32>, u32) {
    if v == 0 {
        return (None, 1);
    }
    let n = v.max(1) - 1;
    (Some(n), if n > 0 { 0 } else { 2 })
}

/// 0x1f1500 tick_i32(p) -> 1 if *p == 0, else *p = max(*p, 1) - 1 and 0 (still counting) / 2 (reached 0).
pub fn tick_i32(c: &mut Cpu) -> bool {
    let p = c.arg(0);
    let (n, r) = tick_down(c.r32(p) as i32);
    if let Some(n) = n {
        c.w32(p, n as u32);
    }
    c.ret(r);
    true
}

/// 0x1f1530 tick_i16(p): `tick_i32` on a signed halfword.
pub fn tick_i16(c: &mut Cpu) -> bool {
    let p = c.arg(0);
    let (n, r) = tick_down(c.r16(p) as i16 as i32);
    if let Some(n) = n {
        c.w16(p, n as u16);
    }
    c.ret(r);
    true
}

/// 0x1f1560 tick_u8(p): `tick_i32` on an unsigned byte.
pub fn tick_u8(c: &mut Cpu) -> bool {
    let p = c.arg(0);
    let (n, r) = tick_down(c.r8(p) as i32);
    if let Some(n) = n {
        c.w8(p, n as u8);
    }
    c.ret(r);
    true
}

/// PLZCW count of one word: leading bits equal to bit 31, minus one.
fn plzcw(w: u32) -> u32 {
    (if w >> 31 != 0 { !w } else { w }).leading_zeros() - 1
}

/// 0x1f1590 log2_floor(n) -> 30 - plzcw(n): index of the top set bit of a positive n, minus one.
pub fn log2_floor(c: &mut Cpu) -> bool {
    let a = c.g(4);
    let (lo, hi) = (plzcw(a as u32), plzcw((a >> 32) as u32));
    c.s(4, (hi as u64) << 32 | lo as u64);
    c.ret(30u32.wrapping_sub(lo));
    true
}

/// 0x1f15d8 fill_words(dst, val, n): stores val to dst, dst+4 ... in steps of 4 while n (counted down by 4) > 0.
pub fn fill_words(c: &mut Cpu) -> bool {
    let (mut dst, val, mut n) = (c.arg(0), c.arg(1), c.arg(2) as i32);
    loop {
        c.w32(dst, val);
        n -= 4;
        dst = dst.wrapping_add(4);
        if n <= 0 {
            break;
        }
    }
    c.set_arg(0, dst);
    c.set_arg(2, n as u32);
    true
}

/// 0x1f1600 zero_qwords(dst, n): clears 16 bytes at a time while n (counted down by 16) > 0.
pub fn zero_qwords(c: &mut Cpu) -> bool {
    let (mut dst, mut n) = (c.arg(0), c.arg(1) as i32);
    loop {
        c.wv(dst, V([0; 4]));
        n -= 16;
        dst = dst.wrapping_add(16);
        if n <= 0 {
            break;
        }
    }
    true
}

/// 0x1f1628 mem_copy(dst, src, n): copies n bytes, by quadwords when dst, src and the end are 16-aligned, by words
/// when 4-aligned, else by bytes (forward, do-while: n = 0 would run away in retail, so it is left to retail).
pub fn mem_copy(c: &mut Cpu) -> bool {
    let (mut dst, mut src, n) = (c.arg(0), c.arg(1), c.arg(2));
    if n == 0 {
        return false;
    }
    let end = dst.wrapping_add(n);
    let all = dst | src | end;
    let step = if all & 0xf == 0 {
        16
    } else if all & 3 == 0 {
        4
    } else {
        1
    };
    while dst != end {
        match step {
            16 => {
                let q = c.rv(src);
                c.wv(dst, q);
            }
            4 => {
                let w = c.r32(src);
                c.w32(dst, w);
            }
            _ => {
                let b = c.r8(src);
                c.w8(dst, b);
            }
        }
        src = src.wrapping_add(step);
        dst = dst.wrapping_add(step);
    }
    true
}

/// 0x1f16c0 copy_qwords(dst, src, n): copies 16 bytes at a time while n (counted down by 16) > 0.
pub fn copy_qwords(c: &mut Cpu) -> bool {
    const V1: usize = 3;
    const A0: usize = 4;
    const A1: usize = 5;
    const A2: usize = 6;
    const STEP: i32 = 16;
    // addi results sign-extend into the low 64 bits of the GPR; the upper 64 bits of the 128-bit register keep their value.
    fn sext32(v: u32) -> u64 {
        v as i32 as i64 as u64
    }
    let mut dst = c.arg(0) as u32;
    let mut src = c.arg(1) as u32;
    let mut n = c.arg(2) as i32;
    let mut last = V([0; 4]);
    loop {
        last = c.rv(src);
        c.wv(dst, last);
        src = src.wrapping_add(STEP as u32);
        dst = dst.wrapping_add(STEP as u32);
        n = n.wrapping_sub(STEP);
        if n <= 0 {
            break;
        }
    }
    // Retail leaves the last quadword in v1 and the advanced pointers and counter in a0..a2.
    c.r[V1] = (0..4).fold(0u128, |acc, i| acc | (last.0[i] as u128) << (32 * i));
    c.s(A0, sext32(dst));
    c.s(A1, sext32(src));
    c.s(A2, sext32(n as u32));
    true
}

/// Upper lanes of a GPR after a 32-bit op (sign-extended word): all ones when the word is negative.
fn word_ext(bits: u32) -> u32 {
    if (bits as i32) < 0 { u32::MAX } else { 0 }
}

/// A GPR after MFC1 of `bits` into a register holding `old`: the low word is the bits, the next word the sign
/// extension, and the two upper words are left as they were.
fn mfc1_lanes(old: u128, bits: u32) -> V {
    V([bits, word_ext(bits), (old >> 64) as u32, (old >> 96) as u32])
}

/// The 128 bits of a VU register as a GPR value (qmfc2).
fn vq_gpr(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |acc, (i, &l)| acc | (l as u128) << (32 * i))
}

/// 0x1f1778 sqrt_q(x) -> sqrt(x) by the VU divide unit.
pub fn sqrt_q(c: &mut Cpu) -> bool {
    const A0: usize = 4;
    let x = c.farg(0);
    let q = F::ZERO.vu_add(x.q_sqrt());
    // mfc1 a0, qmtc2 vf1 <- a0, vsqrt, vaddq.x vf1 = vf0 + Q, qmfc2 a0 <- vf1.
    let a0 = mfc1_lanes(c.r[A0], x.0);
    let vf1 = V([q.0, a0.0[1], a0.0[2], a0.0[3]]);
    c.set_vf(1, vf1);
    c.r[A0] = vq_gpr(vf1);
    c.fret(q);
    true
}

/// 0x1f17b8 fmax(a, b).
pub fn fmax(c: &mut Cpu) -> bool {
    let r = c.farg(0).max(c.farg(1));
    c.fret(r);
    true
}

/// 0x1f17c0 fmin(a, b).
pub fn fmin(c: &mut Cpu) -> bool {
    let r = c.farg(0).min(c.farg(1));
    c.fret(r);
    true
}

/// 0x1f17d8 fclamp(x, lo, hi) -> min(max(x, lo), hi).
pub fn fclamp(c: &mut Cpu) -> bool {
    let r = c.farg(0).max(c.farg(1)).min(c.farg(2));
    c.fret(r);
    true
}

/// 0x1f1d50 pair_byte_sum(p, n) -> sum of the bytes of p, read in pairs while n (counted down by 2) > 0.
pub fn pair_byte_sum(c: &mut Cpu) -> bool {
    let (mut p, mut n) = (c.arg(0), c.arg(1) as i32);
    let mut sum = 0u32;
    loop {
        sum = sum.wrapping_add(c.r8(p) as u32).wrapping_add(c.r8(p.wrapping_add(1)) as u32);
        p = p.wrapping_add(2);
        n -= 2;
        if n <= 0 {
            break;
        }
    }
    c.ret(sum);
    true
}

/// 0x1f1db0 acos_approx(x) -> sign(x) * (pi/2 - poly(|x|) * sqrt(1 - |x|)): arccosine approximation (cubic from the
/// gp table, square root by the VU divide unit).
pub fn acos_approx(c: &mut Cpu) -> bool {
    let x = c.farg(0);
    let r = acos(c, x);
    c.fret(r);
    true
}

/// acos_approx on a value.
pub fn acos(c: &mut Cpu, x: F) -> F {
    const A0: usize = 4;
    let (mut x, mut sign) = (x, F::ONE);
    if x < F::ZERO {
        x = x.abs();
        sign = -F::ONE;
    }
    let base = (c.g(28) as u32).wrapping_sub(GP_ACOS_POLY);
    let coef: [F; 4] = std::array::from_fn(|i| c.rf(base + 4 * i as u32));
    let t = F::ONE - x;
    let root = t.q_sqrt();
    // mfc1 a0 (= 1 - |x|), qmtc2 vf1 <- a0, vsqrt, vaddq.x vf1 = vf0 + Q, qmfc2 a0 <- vf1.
    let a0 = mfc1_lanes(c.r[A0], t.0);
    let vf1 = V([(F::ZERO.vu_add(root)).0, a0.0[1], a0.0[2], a0.0[3]]);
    c.set_vf(1, vf1);
    c.r[A0] = vq_gpr(vf1);
    let x2 = x * x;
    let x3 = x * x2;
    let acc = coef[0] + F::ZERO;
    let acc = acc + x * coef[1];
    let acc = acc + x2 * coef[2];
    let poly = acc + x3 * coef[3];
    let r = poly * F::ZERO.vu_add(root);
    sign * (HALF_PI - r)
}

const PI: F = F(0x4049_0fdb);

/// 0x1f2710 angle_wrap(a) -> a wrapped into [-pi, pi) by repeated 2pi steps.
pub fn angle_wrap(c: &mut Cpu) -> bool {
    c.fret(wrap_angle(c.farg(0)));
    true
}

/// a wrapped into [-pi, pi) by repeated 2pi steps.
pub fn wrap_angle(mut a: F) -> F {
    while !(a < PI) {
        a = a - PI - PI;
    }
    while a < -PI {
        a = a + PI + PI;
    }
    a
}

/// 0x1f2788 angle_dist(a, b) -> |a - b|, folded to 2pi - |a - b| when that is at least pi.
pub fn angle_dist(c: &mut Cpu) -> bool {
    let d = (c.farg(0) - c.farg(1)).abs();
    c.fret(if d < PI { d } else { (PI + PI) - d });
    true
}

/// 0x1f27c0 split_frac(p, x) -> x - trunc(x), and stores trunc(x) as a float at p.
pub fn split_frac(c: &mut Cpu) -> bool {
    let x = c.farg(0);
    let t = F::from_int(x.to_int());
    c.wf(c.arg(0), t);
    c.fret(x - t);
    true
}

/// 0x1f27d8 int_to_float(n) -> (float)n.
pub fn int_to_float(c: &mut Cpu) -> bool {
    c.fret(F::from_int(c.arg(0) as i32));
    true
}

/// 0x1f27e8 float_to_int(x) -> (int)x (cvt.w.s).
pub fn float_to_int(c: &mut Cpu) -> bool {
    c.ret(c.farg(0).to_int() as u32);
    true
}

/// 0x1f27f8 fmod_q(a, b) -> (q - trunc(q)) * b with q = a / b by the VU divide unit.
pub fn fmod_q(c: &mut Cpu) -> bool {
    const AT: usize = 1;
    const V0: usize = 2;
    let (a, b) = (c.farg(0), c.farg(1));
    let at = mfc1_lanes(c.r[AT], a.0);
    let v0 = mfc1_lanes(c.r[V0], b.0);
    let q = F::ZERO.vu_add(a.q_div(b));
    let whole = V::splat(q).ftoi(0).itof(0).x();
    let res = q.vu_sub(whole).vu_mul(b);
    // VU side: vf1 = a (lanes y..w from a0 as mfc1 leaves them), vf2 = b, vf3.x = trunc(q) (y..w kept),
    // vf1.x = (q - trunc q) * b.
    let old3 = c.vf(3).0;
    let vf1 = V([res.0, at.0[1], at.0[2], at.0[3]]);
    c.set_vf(1, vf1);
    c.set_vf(2, v0);
    c.set_vf(3, V([whole.0, old3[1], old3[2], old3[3]]));
    c.r[AT] = vq_gpr(vf1);
    c.r[V0] = vq_gpr(v0);
    c.fret(res);
    true
}
