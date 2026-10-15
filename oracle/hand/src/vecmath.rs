//! Vector and matrix helpers (boot-area twins in every overlay; Eudora addresses).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const PI: F = F(0x4049_0fdb);

/// v.x * m[0] + v.y * m[1] + v.z * m[2] + v.w * m[3], in the VU accumulator's order (VMULAx, VMADDAy, VMADDAz,
/// VMADDw).
pub fn mul_rows(m: [V; 4], v: V) -> V {
    let acc = m[0] * v.x();
    let acc = V::madd(acc, m[1], V::splat(v.y()));
    let acc = V::madd(acc, m[2], V::splat(v.z()));
    V::madd(acc, m[3], V::splat(v.w()))
}

fn rd_mat(c: &mut Cpu, a: u32) -> [V; 4] {
    [c.rv(a), c.rv(a + 0x10), c.rv(a + 0x20), c.rv(a + 0x30)]
}

/// The VU lanes of a 128-bit GPR value (lane 0 in the low word).
fn lanes(q: u128) -> V {
    V(std::array::from_fn(|i| (q >> (32 * i)) as u32))
}

/// A 128-bit GPR value holding the VU lanes (the inverse of `lanes`).
fn quad(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |q, (i, &l)| q | (l as u128) << (32 * i))
}

/// The upper 64 bits of $at, which `mfc1 $at, $f` leaves alone.
fn at_upper(c: &Cpu) -> u64 {
    (c.r[1] >> 64) as u64
}

/// $at after `mfc1 $at, $f`: low 64 bits = `bits` sign-extended, upper 64 bits = `up`.
fn gpr_after_mfc1(bits: u32, up: u64) -> u128 {
    ((up as u128) << 64) | (bits as i32 as i64 as u64) as u128
}

/// 0x1f1810 vec_add(out, a, b): out = (a.xyz + b.xyz, a.w).
pub fn vec_add(c: &mut Cpu) -> bool {
    let (a, b) = (c.rv(c.arg(1)), c.rv(c.arg(2)));
    c.wv(c.arg(0), a.with(a + b, XYZ));
    true
}

/// 0x1f1898 vec_scale(out, v, s): out = (v.xyz * s, v.w).
pub fn vec_scale(c: &mut Cpu) -> bool {
    let s = c.farg(0);
    let at = gpr_after_mfc1(s.0, at_upper(c));
    let v = c.rv(c.arg(1));
    let r = v.with(v * s, XYZ);
    c.set_vf(1, r);
    c.set_vf(2, lanes(at));
    c.r[1] = at;
    c.wv(c.arg(0), r);
    true
}

/// 0x1f18b0 vec_scale_self(_, v, s): v.xyz *= s. Same body as vec_scale (0x1f1898) with the store going to a1: retail
/// leaves vf1 = the result, vf2 = the scale in every lane and $at = mfc1 $f12.
pub fn vec_scale_self(c: &mut Cpu) -> bool {
    let s = c.farg(0);
    let at = gpr_after_mfc1(s.0, at_upper(c));
    let p = c.arg(1);
    let v = c.rv(p);
    let r = v.with(v * s, XYZ);
    c.set_vf(1, r);
    c.set_vf(2, lanes(at));
    c.r[1] = at;
    c.wv(p, r);
    true
}

/// 0x1f19e8 dist3(a, b) -> |a.xyz - b.xyz|.
pub fn dist3(c: &mut Cpu) -> bool {
    let d = c.rv(c.arg(0)) - c.rv(c.arg(1));
    let sq = d * d;
    let q = sq.x().vu_add(sq.y()).vu_add(sq.z()).q_sqrt();
    c.fret(F::ZERO.vu_add(q));
    true
}

/// 0x1f1a20 dist_xy(a, b) -> |a.xy - b.xy|.
pub fn dist_xy(c: &mut Cpu) -> bool {
    let (a, b) = (c.rv(c.arg(0)), c.rv(c.arg(1)));
    let d = a.with(a - b, X | Y);
    let sq = d.with(d * d, X | Y);
    let len = F::ZERO.vu_add(sq.x().vu_add(sq.y()).q_sqrt());
    let r = sq.with(V::splat(len), X);
    c.set_vf(1, r);
    c.set_vf(2, b);
    c.r[4] = quad(r);
    c.fret(len);
    true
}

/// v.xyz scaled to length `len` (w kept); a zero vector gives (0, 0, 0, v.w).
pub fn set_length(v: V, len: F) -> V {
    let sq = v * v;
    let len2 = sq.x().vu_add(sq.y()).vu_add(sq.z());
    if len2.0 == 0 {
        v.with_xyz(F::ZERO, F::ZERO, F::ZERO)
    } else {
        v.with(v * len.q_rsqrt(len2), XYZ)
    }
}

/// 0x1f1a98 vec_set_length(out, v, len): out = (v.xyz * len / |v.xyz|, v.w); a zero vector gives (0, 0, 0, v.w).
pub fn vec_set_length(c: &mut Cpu) -> bool {
    let v = c.rv(c.arg(1));
    c.wv(c.arg(0), set_length(v, c.farg(0)));
    true
}

/// 0x1f1c90 vec4_mul_mat4(out, v, m): out = v * m (row vector).
pub fn vec4_mul_mat4(c: &mut Cpu) -> bool {
    let v = c.rv(c.arg(1));
    let m = rd_mat(c, c.arg(2));
    c.wv(c.arg(0), mul_rows(m, v));
    true
}

/// 0x1f2370 mat4_mul(out, a, b): out = b * a (row i of out = row i of b times a; rows written in order, so `out`
/// may alias `b`).
pub fn mat4_mul(c: &mut Cpu) -> bool {
    let (out, a, b) = (c.arg(0), c.arg(1), c.arg(2));
    let a = rd_mat(c, a);
    for i in 0..4 {
        let row = c.rv(b + i * 0x10);
        c.wv(out + i * 0x10, mul_rows(a, row));
    }
    true
}

/// a wrapped once into [-pi, pi): one 2pi step down when a >= pi, one up when a < -pi.
pub fn wrap_once(a: F) -> F {
    let below = a < -PI;
    let mut r = a;
    if !(r < PI) {
        r = r - PI - PI;
    }
    if below {
        r = r + PI + PI;
    }
    r
}

/// 0x1f2680 angle_add(a, b) -> a + b wrapped once into [-pi, pi).
pub fn angle_add(c: &mut Cpu) -> bool {
    let r = wrap_once(c.farg(0) + c.farg(1));
    c.fret(r);
    true
}

/// 0x1f26c8 angle_sub(a, b) -> a - b wrapped once into [-pi, pi).
pub fn angle_sub(c: &mut Cpu) -> bool {
    let r = wrap_once(c.farg(0) - c.farg(1));
    c.fret(r);
    true
}

/// 0x1f1828 vec_add_self(_, dst, b): dst.xyz += b.xyz.
pub fn vec_add_self(c: &mut Cpu) -> bool {
    let (p, b) = (c.arg(1), c.rv(c.arg(2)));
    let a = c.rv(p);
    let r = a.with(a + b, XYZ);
    c.set_vf(1, r);
    c.set_vf(2, b);
    c.wv(p, r);
    true
}

/// 0x1f1840 vec_sub(out, a, b): out = (a.xyz - b.xyz, a.w).
pub fn vec_sub(c: &mut Cpu) -> bool {
    let (a, b) = (c.rv(c.arg(1)), c.rv(c.arg(2)));
    c.wv(c.arg(0), a.with(a - b, XYZ));
    true
}

/// (a.xyz + (b.xyz - a.xyz) * t, a.w) (as 0x1f1870).
pub fn lerp(a: V, b: V, t: F) -> V {
    let d = (b - a).with(a, W) * t;
    a.with(a + d, XYZ)
}

/// 0x1f1870 vec_lerp(out, a, b, t): out = (a.xyz + (b.xyz - a.xyz) * t, a.w).
pub fn vec_lerp(c: &mut Cpu) -> bool {
    let (a, b) = (c.rv(c.arg(1)), c.rv(c.arg(2)));
    let d = (b - a).with(a, W) * c.farg(0);
    c.wv(c.arg(0), a.with(a + d, XYZ));
    true
}

/// 0x1f18c8 vec4_scale(out, v, s): out = v * s on all four lanes.
pub fn vec4_scale(c: &mut Cpu) -> bool {
    let s = c.farg(0);
    let at = gpr_after_mfc1(s.0, at_upper(c));
    let v = c.rv(c.arg(1));
    let r = v * s;
    c.set_vf(1, r);
    c.set_vf(2, lanes(at));
    c.r[1] = at;
    c.wv(c.arg(0), r);
    true
}

/// 0x1f18f8 vec4_mul(out, a, b): out = a * b lane by lane.
pub fn vec4_mul(c: &mut Cpu) -> bool {
    let (a, b) = (c.rv(c.arg(1)), c.rv(c.arg(2)));
    let r = a * b;
    c.set_vf(1, r);
    c.set_vf(2, b);
    c.wv(c.arg(0), r);
    true
}

/// 0x1f1910 vec_div_scalar(out, v, s): out = (v.xyz * (1 / s), v.w), reciprocal by the VU divide unit.
pub fn vec_div_scalar(c: &mut Cpu) -> bool {
    let s = c.farg(0);
    let at = gpr_after_mfc1(s.0, at_upper(c));
    let v = c.rv(c.arg(1));
    let r = v.with(v * F::ONE.q_div(s), XYZ);
    c.set_vf(1, r);
    c.set_vf(2, lanes(at));
    c.r[1] = at;
    c.wv(c.arg(0), r);
    true
}

/// x + y + z of a product vector, in the VU accumulator's order (x + y, then + 1 * z).
pub fn sum3(m: V) -> F {
    m.x().vu_add(m.y()).vu_add(F::ONE.vu_mul(m.z()))
}

/// a.xyz . b.xyz (as 0x1f1930).
pub fn dot(a: V, b: V) -> F {
    sum3(a * b)
}

/// 0x1f1930 dot3(a, b) -> a.xyz . b.xyz.
pub fn dot3(c: &mut Cpu) -> bool {
    let (a, b) = (c.rv(c.arg(0)), c.rv(c.arg(1)));
    let p = a.with(a * b, XYZ);
    // vadday.x ACC: x + y; vmaddz.x VF1: ACC.x + 1 * z (VF3 = (1, 1, 1, old w) from vaddw).
    c.set_acc(c.acc().with(V::splat(p.x().vu_add(p.y())), X));
    let sum = sum3(p);
    let r = p.with(V::splat(sum), X);
    let vf3 = c.vf(3).with_xyz(F::ONE, F::ONE, F::ONE);
    c.set_vf(1, r);
    c.set_vf(2, b);
    c.set_vf(3, vf3);
    c.r[4] = quad(r);
    c.fret(sum);
    true
}

/// 0x1f1958 cross3(out, a, b): out.xyz = b x a (retail operand order); out.w is whatever VF3.w holds (retail's scratch register).
pub fn cross3(c: &mut Cpu) -> bool {
    let (a, b) = (c.rv(c.arg(1)), c.rv(c.arg(2)));
    // vopmula leaves the first products in ACC; vopmsub subtracts the second ones into VF3.
    let (p1x, p1y, p1z) = (b.y().vu_mul(a.z()), b.z().vu_mul(a.x()), b.x().vu_mul(a.y()));
    let x = p1x.vu_sub(b.z().vu_mul(a.y()));
    let y = p1y.vu_sub(b.x().vu_mul(a.z()));
    let z = p1z.vu_sub(b.y().vu_mul(a.x()));
    let acc = c.acc().with_xyz(p1x, p1y, p1z);
    let out = c.vf(3).with_xyz(x, y, z);
    c.set_acc(acc);
    c.set_vf(1, a);
    c.set_vf(2, b);
    c.set_vf(3, out);
    c.wv(c.arg(0), out);
    true
}

/// |v.xyz| (as 0x1f1970).
pub fn length(v: V) -> F {
    F::ZERO.vu_add(sum3(v * v).q_sqrt())
}

/// 0x1f1970 length3(v) -> |v.xyz|.
pub fn length3(c: &mut Cpu) -> bool {
    let v = c.rv(c.arg(0));
    let sq = v.with(v * v, XYZ);
    c.set_acc(c.acc().with(V::splat(sq.x().vu_add(sq.y())), X));
    let len = F::ZERO.vu_add(sum3(sq).q_sqrt());
    let r = sq.with(V::splat(len), X);
    let vf2 = c.vf(2).with_xyz(F::ONE, F::ONE, F::ONE);
    c.set_vf(1, r);
    c.set_vf(2, vf2);
    c.r[4] = quad(r);
    c.fret(len);
    true
}

/// 0x1f19a0 length_xy(v) -> |v.xy|.
pub fn length_xy(c: &mut Cpu) -> bool {
    let v = c.rv(c.arg(0));
    let sq = v.with(v * v, XYZ);
    let len = F::ZERO.vu_add(sq.x().vu_add(sq.y()).q_sqrt());
    let r = sq.with(V::splat(len), X);
    c.set_vf(1, r);
    c.r[4] = quad(r);
    c.fret(len);
    true
}

/// 0x1f1ae8 vec_set_length_xy(out, v, len): out = (v.xy * len / |v.xy|, v.z, v.w); a zero xy gives (0, 0, v.z, v.w).
pub fn vec_set_length_xy(c: &mut Cpu) -> bool {
    let s = c.farg(0);
    let up = at_upper(c);
    let v = c.rv(c.arg(1));
    let vf2 = c.vf(2).with(v * v, X | Y);
    let len2 = vf2.x().vu_add(vf2.y());
    let vf2 = vf2.with(V::splat(len2), X);
    let vf3 = lanes(gpr_after_mfc1(s.0, up));
    // qmfc2 $at, vf2 + dsll32 $at, $at, 0: the low word of VF2.x moves to bits 32..63.
    let at = (quad(vf2) & !(u64::MAX as u128)) | ((len2.0 as u64) << 32) as u128;
    let out = if len2.0 == 0 {
        v.with(V::splat(F::ZERO.vu_add(F::ZERO)), X | Y)
    } else {
        v.with(v * s.q_rsqrt(len2), X | Y)
    };
    c.set_vf(1, out);
    c.set_vf(2, vf2);
    c.set_vf(3, vf3);
    c.r[1] = at;
    c.wv(c.arg(0), out);
    true
}

/// 0x1f1b30 vec_clamp_length(out, v, max) -> 1: when |v.xyz| >= max (and non-zero) writes out = (v.xyz * max / |v|,
/// v.w) and returns 1; otherwise writes nothing and returns 0.
pub fn vec_clamp_length(c: &mut Cpu) -> bool {
    let v = c.rv(c.arg(1));
    let max = c.farg(0);
    let up = at_upper(c);
    let ones = V([F::ONE.0; 4]);
    // VU0 temporaries as retail leaves them: VF2 = squares, ACC.x = x^2 + y^2, VF2.x = |v|^2.
    let mut acc = c.acc();
    let mut vf2 = c.vf(2).with(v * v, XYZ);
    acc.0[0] = vf2.x().vu_add(vf2.y()).0;
    let sum = V::madd(acc, ones, V::splat(vf2.z()));
    vf2 = vf2.with(sum, X);
    let q = vf2.x().q_sqrt();
    let len = F::ZERO.vu_add(q);
    vf2 = vf2.with(V([len.0, 0, 0, 0]), X);
    let vf4 = c.vf(4).with(V([max.vu_sub(q).0, 0, 0, 0]), X);
    // beqz tests the low 64 bits of $at (VF2 lanes x, y); bgtz tests VF4.x as signed.
    let low = (vf2.0[0] as u64) | ((vf2.0[1] as u64) << 32);
    let early = low == 0 || (vf4.0[0] as i32) > 0;
    c.set_vf(1, v);
    c.set_vf(2, vf2);
    c.set_vf(3, lanes(gpr_after_mfc1(max.0, up)));
    c.set_vf(4, vf4);
    c.set_acc(acc);
    c.r[1] = quad(vf2);
    if early {
        c.ret(0);
        return true;
    }
    let vf1 = v.with(v * max.q_div(len), XYZ);
    c.set_vf(1, vf1);
    c.wv(c.arg(0), vf1);
    c.ret(1);
    true
}

/// 0x1f1c68 vec3_mul_mat3(out, v, m): out = v.x * m0 + v.y * m1 + v.z * m2 + v.w * (0, 0, 0, 1).
pub fn vec3_mul_mat3(c: &mut Cpu) -> bool {
    let v = c.rv(c.arg(1));
    let m = [c.rv(c.arg(2)), c.rv(c.arg(2) + 0x10), c.rv(c.arg(2) + 0x20)];
    let acc0 = m[0] * v.x();
    let acc1 = V::madd(acc0, m[1], V::splat(v.y()));
    let acc2 = V::madd(acc1, m[2], V::splat(v.z()));
    let out = V::madd(acc2, V::ZERO_W1, V::splat(v.w()));
    c.set_vf(1, m[0]);
    c.set_vf(2, m[1]);
    c.set_vf(3, m[2]);
    c.set_vf(5, v);
    c.set_vf(6, out);
    c.set_acc(acc2);
    c.wv(c.arg(0), out);
    true
}

/// 0x1f1d00 vec4_to_i16x4(v, s) -> four signed halfwords (v * s truncated, clamped to [-32768, 32767]) packed x..w
/// into the low 64 bits of v0.
pub fn vec4_to_i16x4(c: &mut Cpu) -> bool {
    let s = c.farg(0);
    let up = at_upper(c);
    let v = (c.rv(c.arg(0)) * s).ftoi(0);
    let clamped: [u32; 4] = v.0.map(|w| (w as i32).clamp(-0x8000, 0x7fff) as u32);
    let mut packed = 0u64;
    for (i, &w) in clamped.iter().enumerate() {
        packed |= (w as u16 as u64) << (16 * i);
    }
    c.set_vf(1, v);
    c.set_vf(2, lanes(gpr_after_mfc1(s.0, up)));
    c.r[1] = quad(V(clamped));
    c.r[2] = packed as u128;
    c.r[3] = quad(V([0xffff_8000; 4]));
    true
}

/// 0x1f1f80 mat4_identity(out).
pub fn mat4_identity(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let one = F::ONE.0;
    let rows = [V([one, 0, 0, 0]), V([0, one, 0, 0]), V([0, 0, one, 0]), V::ZERO_W1];
    for (i, row) in rows.iter().enumerate() {
        c.set_vf(i + 1, *row);
    }
    for (i, row) in rows.iter().enumerate() {
        c.wv(out + 16 * i as u32, *row);
    }
    true
}

/// 0x1f1fb0 mat4_scale(out, s): out = diag(s, s, s, 1).
pub fn mat4_scale(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let up = at_upper(c);
    let s = F::ZERO.vu_add(c.farg(0));
    let rows = [V([s.0, 0, 0, 0]), V([0, s.0, 0, 0]), V([0, 0, s.0, 0]), V::ZERO_W1];
    c.set_vf(5, lanes(gpr_after_mfc1(c.farg(0).0, up)));
    for (i, row) in rows.iter().enumerate() {
        c.set_vf(i + 1, *row);
    }
    for (i, row) in rows.iter().enumerate() {
        c.wv(out + 16 * i as u32, *row);
    }
    true
}

/// VCLIPW.xyz clip bits of v against |v.w|: bit 2k = +axis k > |w|, bit 2k+1 = -axis k < -|w| (denormals as zero).
pub(crate) fn clip_bits(v: V) -> u32 {
    let flush = |f: F| if f.0 & 0x7f80_0000 == 0 { F(f.0 & 0x8000_0000) } else { f };
    let w = flush(v.w()).abs();
    let mut bits = 0;
    for l in 0..3 {
        let x = flush(v.lane(l));
        if w < x {
            bits |= 1 << (2 * l);
        }
        if x < -w {
            bits |= 2 << (2 * l);
        }
    }
    bits
}

/// 0x1f1748 clip_flags(v) -> VCLIPW.xyz of v against its own w, low six bits.
pub fn clip_flags(c: &mut Cpu) -> bool {
    let v = c.rv(c.arg(0));
    c.ret(clip_bits(v));
    true
}

/// 0x1f2250 mat3_copy_w1(out, m): copies three rows, row 3 = (0, 0, 0, 1).
pub fn mat3_copy_w1(c: &mut Cpu) -> bool {
    let m = rd_mat(c, c.arg(1));
    for (i, row) in m.iter().take(3).enumerate() {
        c.wv(c.arg(0) + 16 * i as u32, *row);
    }
    c.wv(c.arg(0) + 0x30, V::ZERO_W1);
    true
}

/// 0x1f2270 mat3_copy(out, m): copies three rows.
pub fn mat3_copy(c: &mut Cpu) -> bool {
    let (out, src) = (c.arg(0), c.arg(1));
    let rows = [c.rv(src), c.rv(src + 0x10), c.rv(src + 0x20)];
    c.r[1] = quad(rows[0]);
    c.r[2] = quad(rows[1]);
    c.r[3] = quad(rows[2]);
    for (i, row) in rows.iter().enumerate() {
        c.wv(out + 16 * i as u32, *row);
    }
    true
}

/// 0x1f2290 mat3_transpose(out, m): out rows = columns of m with w = 0; row 3 = (0, 0, 0, 1).
pub fn mat3_transpose(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let m = [c.rv(c.arg(1)), c.rv(c.arg(1) + 0x10), c.rv(c.arg(1) + 0x20)];
    for (i, row) in m.iter().enumerate() {
        c.set_vf(1 + i, *row);
    }
    let vf0 = c.vf(0);
    c.wv(out + 0x30, vf0);
    for i in 0..3 {
        // VADDx per lane: dest lane k = VF0 lane k + row k's lane i; w = VF0.w - VF0.w.
        let lane = |k: usize| vf0.lane(k).vu_add(m[k].lane(i));
        let row = V([lane(0).0, lane(1).0, lane(2).0, vf0.w().vu_sub(vf0.w()).0]);
        c.set_vf(4 + i, row);
        c.wv(out + 16 * i as u32, row);
    }
    true
}

/// 0x1f2320 mat3_mul(out, a, b): row i of out = b[i].x * a0 + b[i].y * a1 + b[i].z * a2 (all four lanes).
pub fn mat3_mul(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let a = [c.rv(c.arg(1)), c.rv(c.arg(1) + 0x10), c.rv(c.arg(1) + 0x20)];
    let b = [c.rv(c.arg(2)), c.rv(c.arg(2) + 0x10), c.rv(c.arg(2) + 0x20)];
    for (i, row) in a.iter().enumerate() {
        c.set_vf(4 + i, *row);
    }
    for (i, row) in b.iter().enumerate() {
        c.set_vf(1 + i, *row);
    }
    // VMULAx / VMADDAy of row i; the VMADDz that writes the row leaves ACC alone.
    let partial = |i: usize| V::madd(a[0] * b[i].x(), a[1], V::splat(b[i].y()));
    c.set_acc(partial(2));
    let rows: [V; 3] = std::array::from_fn(|i| V::madd(partial(i), a[2], V::splat(b[i].z())));
    for (i, row) in rows.iter().enumerate() {
        c.set_vf(1 + i, *row);
        c.wv(out + 16 * i as u32, *row);
    }
    true
}

/// 0x1f2400 quat_mul(out, a, b): out.xyz = b.xyz * a.w + a.xyz * b.w + a.xyz x b.xyz, out.w = a.w * b.w - a.xyz . b.xyz.
pub fn quat_mul(c: &mut Cpu) -> bool {
    let (a, b) = (c.rv(c.arg(1)), c.rv(c.arg(2)));
    let prod = b * a;
    let by_aw = b * a.w();
    let by_bw = a * b.w();
    let cross = [
        a.y().vu_mul(b.z()).vu_sub(a.z().vu_mul(b.y())),
        a.z().vu_mul(b.x()).vu_sub(a.x().vu_mul(b.z())),
        a.x().vu_mul(b.y()).vu_sub(a.y().vu_mul(b.x())),
    ];
    let dot = prod.x().vu_add(prod.y()).vu_add(F::ONE.vu_mul(prod.z()));
    let w = b.w().vu_mul(a.w()).vu_sub(dot);
    let sum = |i: usize| by_aw.lane(i).vu_add(by_bw.lane(i)).vu_add(cross[i]);
    c.wv(c.arg(0), V::ZERO_W1.with_xyz(sum(0), sum(1), sum(2)).with(V::splat(w), W));
    true
}

/// 0x1f2580 quat_to_mat3(q, out): writes the 3x3 rotation matrix of quaternion q (rows 0..2, w = 0).
pub fn quat_to_mat3(c: &mut Cpu) -> bool {
    let out = c.arg(1);
    let q = c.rv(c.arg(0));
    let t = q + q;
    c.set_vf(8, q);
    c.set_vf(9, t);
    // VF10..VF13 are partial products; a masked write keeps the lanes it does not name.
    let (p10, p11, p12, p13) = (t * q.w(), t * q.x(), t * q.y(), t * q.z());
    c.set_vf(10, c.vf(10).with(p10, XYZ));
    c.set_vf(11, c.vf(11).with(p11, XYZ));
    c.set_vf(12, c.vf(12).with(p12, Y | Z));
    c.set_vf(13, c.vf(13).with(p13, Z));
    let (xw, yw, zw) = (p10.x(), p10.y(), p10.z());
    let (xx, xy, xz) = (p11.x(), p11.y(), p11.z());
    let (yy, yz, zz) = (p12.y(), p12.z(), p13.z());
    let (zero, one) = (F::ZERO, F::ONE);
    let row0 = [one.vu_sub(yy).vu_sub(zz), xy.vu_sub(zw), xz.vu_add(yw)];
    let row1 = [zero.vu_add(zw).vu_add(xy), one.vu_sub(xx).vu_sub(zz), yz.vu_sub(xw)];
    let row2 = [zero.vu_sub(yw).vu_add(xz), zero.vu_add(xw).vu_add(yz), one.vu_sub(xx).vu_sub(yy)];
    for (i, r) in [row0, row1, row2].iter().enumerate() {
        let row = V([r[0].0, r[1].0, r[2].0, 0]);
        c.set_vf(14 + i, row);
        c.wv(out + 16 * i as u32, row);
    }
    true
}

/// 0x1f2830 color_lerp(a, b, t) -> packed RGBA bytes of a * (1 - t) + b * t (each channel truncated, low byte kept).
pub fn color_lerp(c: &mut Cpu) -> bool {
    let unpack = |p: u32| V(p.to_le_bytes().map(|b| b as u32));
    let (pa, pb, t) = (c.arg(0), c.arg(1), c.farg(0));
    let (ia, ib) = (unpack(pa), unpack(pb));
    // $a0, $a1 hold the unpacked bytes (pextlb/pextlh); $at = t sign-extended (mfc1), upper half kept.
    c.r[4] = quad(ia);
    c.r[5] = quad(ib);
    c.r[1] = gpr_after_mfc1(t.0, at_upper(c));
    let (a, b) = (ia.itof(0), ib.itof(0));
    c.set_vf(2, a);
    c.set_vf(3, b);
    let w = F::ONE.vu_sub(t);
    let acc = a * w;
    c.set_acc(acc);
    let m = V::madd(acc, b, V::splat(t)).ftoi(0);
    c.set_vf(1, m);
    // ppach / ppacb leave the four low bytes in $v0 and zeros above.
    let packed = u32::from_le_bytes(m.0.map(|l| l as u8));
    c.r[2] = packed as u128;
    true
}

/// 0x24cfc0 color_lerp_tail(a, b, t): retail wrapper that jumps straight to color_lerp.
pub fn color_lerp_tail(c: &mut Cpu) -> bool {
    // retail: addiu sp,-16; sq ra,0(sp); jal color_lerp; lq ra,0(sp); addiu sp,16
    let sp = c.g(29) as u32 - 16;
    c.wv(sp, lanes(c.r[31]));
    let (a, b, t) = (c.arg(0), c.arg(1), c.farg(0));
    crate::h::run_keep(c, color_lerp, &[a, b], &[t]);
    true
}
