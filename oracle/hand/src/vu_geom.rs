//! VU0 geometry helpers: a fast atan2 approximation, a 4x4 row-vector product and a quaternion product.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Polynomial coefficients c0..c3 (vf14) and c4..c7 (vf15) of the atan approximation.
const ATAN_COEF_LO: u32 = 0x001c_2520;
const ATAN_COEF_HI: u32 = 0x001c_2530;
/// Table of (A, B) float pairs, indexed by quadrant (8 * swapped + 16 * sign(b) + 32 * sign(a)).
const ATAN_TABLE: u32 = 0x001c_2540;
/// pi/4 as f32 bits.
const PI_OVER_4: F = F(0x3f49_0fdb);

/// Packs four VU lanes into a 128-bit GPR value (lane 0 in the low word).
fn pack(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |acc, (i, &l)| acc | (l as u128) << (32 * i))
}

/// 0x1f1e48 fast_atan2(a = f12, b = f13) -> f0: atan2(b, a) from the min/max ratio t = (min - max) / (min + max),
/// an 8-term odd polynomial in t plus pi/4, scaled and offset by the quadrant's table pair.
pub fn fast_atan2(c: &mut Cpu) -> bool {
    let (a, b) = (c.farg(0), c.farg(1));
    let c14 = c.rv(ATAN_COEF_LO);
    c.set_vf(14, c14);
    let (aa, ab) = (a.abs(), b.abs());
    let diff = aa - ab;
    let swapped = diff.0 >> 31;
    let (lo, hi) = if swapped == 1 { (aa, ab) } else { (ab, aa) };
    let sum = lo + hi;
    let lo_minus_hi = lo - hi;
    c.s(15, ATAN_COEF_LO as u64);
    c.s(3, diff.0 as i32 as i64 as u64);
    c.f[14] = 0;
    c.f[1] = lo.0;
    c.f[2] = hi.0;
    c.f[3] = lo_minus_hi.0;
    c.f[4] = sum.0;
    if hi <= F::ZERO {
        c.fret(F::ZERO);
        return true;
    }
    let one = F::ZERO.vu_add(F::ONE);
    let t = lo_minus_hi / sum;
    let c15 = c.rv(ATAN_COEF_HI);
    c.set_vf(15, c15);
    // vf1 = (t, t, t, t) as vaddx.yz / vmulx.w leave it
    let mut p = V([t.0, F::ZERO.vu_add(t).0, F::ZERO.vu_add(t).0, F::ONE.vu_mul(t).0]);
    let sq = p * p; // vf2 = t^2
    let quad = sq * sq; // vf3 = t^4
    let oct = quad * quad; // vf4 = t^8
    p = p.with(p * sq, Y); // vf1.y = t^3
    p = p.with(p * quad, Z | W); // vf1.zw = t^5
    p = p.with(p * sq, W); // vf1.w = t^7
    let high = p * oct; // vf2 = t^9, t^11, t^13, t^15
    let low = p * c14; // vf1 = c0 t, c1 t^3, c2 t^5, c3 t^7
    let high = high * c15; // vf2 = c4 t^9 .. c7 t^15
    let vf5 = c.vf(5).with_xyz(one, one, one);
    c.set_vf(5, vf5);
    // ACC chain (vadday.x, then vmadda.x for the lanes of vf5 = 1)
    let mut ax = low.x().vu_add(low.y());
    ax = ax.vu_add(one.vu_mul(low.z()));
    ax = ax.vu_add(one.vu_mul(low.w()));
    ax = ax.vu_add(one.vu_mul(high.x()));
    ax = ax.vu_add(one.vu_mul(high.y()));
    ax = ax.vu_add(one.vu_mul(high.z()));
    let poly = ax.vu_add(one.vu_mul(high.w()));
    let old_acc = c.acc();
    c.set_acc(V([ax.0, old_acc.0[1], old_acc.0[2], old_acc.0[3]]));
    let vf1 = low.with(V::splat(poly), X);
    c.set_vf(1, vf1);
    c.set_vf(2, high);
    c.set_vf(3, quad);
    c.set_vf(4, oct);
    // scalar tail: q = (pi/4 + P) * A + B with (A, B) from the quadrant table
    let sa = a.0 >> 31;
    let sb = b.0 >> 31;
    let tab = ATAN_TABLE + 8 * swapped + 16 * sb + 32 * sa;
    let (ta, tb) = (c.rf(tab), c.rf(tab + 4));
    let q = (PI_OVER_4 + poly) * ta;
    let result = q + tb;
    c.f[1] = ta.0;
    c.f[2] = tb.0;
    c.f[3] = q.0;
    c.f[4] = poly.0;
    c.r[1] = pack(vf1);
    c.s(2, (16 * sb) as u64);
    c.s(3, (8 * swapped) as u64);
    c.s(4, tab as u64);
    c.fret(result);
    true
}

/// 0x1f2370 mat4_mul_rows(out, a, b): out row i = b row i times the matrix a (row-vector products, VU0 order).
pub fn mat4_mul_rows(c: &mut Cpu) -> bool {
    let (out, m, src) = (c.arg(0), c.arg(1), c.arg(2));
    let rows: [V; 4] = std::array::from_fn(|k| c.rv(m + 16 * k as u32));
    let end = src + 0x40;
    let (mut o, mut p) = (out, src);
    let (mut last_b, mut last_acc, mut last_out) = (V::ZERO_W1, V::ZERO_W1, V::ZERO_W1);
    loop {
        let b = c.rv(p);
        let acc = rows[0] * b.x();
        let acc = V::madd(acc, rows[1], V::splat(b.y()));
        let acc = V::madd(acc, rows[2], V::splat(b.z()));
        let row = V::madd(acc, rows[3], V::splat(b.w()));
        c.wv(o, row);
        (last_b, last_acc, last_out) = (b, acc, row);
        o += 16;
        p += 16;
        if p == end {
            break;
        }
    }
    for (k, r) in rows.iter().enumerate() {
        c.set_vf(4 + k, *r);
    }
    c.set_vf(8, last_b);
    c.set_vf(9, last_out);
    c.set_acc(last_acc);
    c.set_arg(0, o);
    c.set_arg(2, end);
    c.set_arg(3, end);
    true
}

/// 0x1f2400 quat_mul(out, a, b): out.xyz = b.xyz * a.w + a.xyz * b.w + cross(b, a), out.w = a.w * b.w - a.xyz . b.xyz.
pub fn quat_mul(c: &mut Cpu) -> bool {
    // Declined: the cross-term zero signs (vf7 x/y, ACC.y) differ from retail and the cause is unclear. Retail runs this entry.
    let _ = c;
    return false;
    let (out, pa, pb) = (c.arg(0), c.arg(1), c.arg(2));
    let (a, b) = (c.rv(pa), c.rv(pb));
    let one = F::ZERO.vu_add(F::ONE);
    let vf9 = c.vf(9).with_xyz(one, one, one);
    let vf3 = c.vf(3).with(V::splat(b.w().vu_mul(a.w())), W);
    let prod = b * a;
    let vf4 = c.vf(4).with(prod, XYZ);
    let by_aw = b * a.w();
    let vf5 = c.vf(5).with(by_aw, XYZ);
    let by_bw = a * b.w();
    let vf6 = c.vf(6).with(by_bw, XYZ);
    // VOPMULA / VOPMSUB: cross(b, a) in vf7, the first half left in ACC
    let acc_x = b.y().vu_mul(a.z());
    let acc_y = b.z().vu_mul(a.x());
    let acc_z = b.x().vu_mul(a.y());
    let cross = [
        acc_x.vu_sub(b.z().vu_mul(a.y())),
        acc_y.vu_sub(b.x().vu_mul(a.z())),
        acc_z.vu_sub(b.y().vu_mul(a.x())),
    ];
    let vf7 = c.vf(7).with_xyz(cross[0], cross[1], cross[2]);
    // dot = (x + y) + z through ACC.x, then vf4.x
    let ax = prod.x().vu_add(prod.y());
    let dot = ax.vu_add(one.vu_mul(prod.z()));
    let vf4 = vf4.with(V::splat(dot), X);
    let old_acc = c.acc();
    c.set_acc(V([ax.0, acc_y.0, acc_z.0, old_acc.0[3]]));
    let sums: [F; 3] = std::array::from_fn(|i| by_aw.lane(i).vu_add(by_bw.lane(i)).vu_add(cross[i]));
    let w = vf3.w().vu_sub(dot);
    let vf8 = V([sums[0].0, sums[1].0, sums[2].0, w.0]);
    c.wv(out, vf8);
    c.set_vf(1, a);
    c.set_vf(2, b);
    c.set_vf(3, vf3);
    c.set_vf(4, vf4);
    c.set_vf(5, vf5);
    c.set_vf(6, vf6);
    c.set_vf(7, vf7);
    c.set_vf(8, vf8);
    c.set_vf(9, vf9);
    true
}
