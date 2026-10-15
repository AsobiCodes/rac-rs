//! Particle point helpers: a VU0 frustum side test over a point list, and the particle jitter that offsets a position
//! by a random polar vector (both retail code, rewritten here).
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Set while the frustum loop does not match retail (see frustum_point_flags).
const FRUSTUM_DECLINED: bool = true;

/// Frustum plane rows in retail RAM (`lui 0x17` + `addiu -0x3240`): rows at +0x00..+0x70, as in `sphere_visible`.
const FRUSTUM: u32 = 0x16_cdc0;
/// Basis rows the frustum rows are mixed by (`lui 0x1c` + `addiu -0x2720`).
const BASIS: u32 = 0x1b_d8e0;
/// Table vector of the jitter (`lui 0x16` + `addiu 0x6ec0`).
const JITTER_TABLE: u32 = 0x16_6ec0;
/// Second table of the jitter (`lui 0x17` + `addiu 0x3f60`).
const MOVE_TABLE: u32 = 0x17_3f60;
/// Test helper of the jitter (`jal 0x1e24d8`): its v0 decides whether the move runs.
const TEST_HELPER: u32 = 0x1e_24d8;
/// Random polar vector helper (rand_polar_vec, retail).
const RAND_POLAR: u32 = 0x24_a5c8;
/// Vector helper 0x1f1810 (vec_add of retail): out = a + b.
const VEC_ADD: u32 = 0x1f_1810;
/// Vector helper 0x1f1840 (out, a, b) used by the move.
const MOVE_HELPER: u32 = 0x1f_1840;
/// Stack frame of the jitter.
const JITTER_FRAME: u32 = 0x30;
/// 0.5 (f12 of the polar call).
const HALF: F = F(0x3f00_0000);
/// 6.0 (f13 of the polar call).
const SIX: F = F(0x40c0_0000);
/// 0.75 (the scale of the move).
const THREE_QUARTERS: F = F(0x3f40_0000);
/// Retail `lw` offset of the test helper's argument (`lw a3, -0x1a44(v0)` with v0 = 0x140000).
const TEST_ARG_ADDR: u32 = 0x14_0000 - 0x1a44;

/// Stores a 128-bit GPR value at `a` (what `sq` does).
fn store_quad(c: &mut Cpu, a: u32, q: u128) {
    for i in 0..4 {
        c.w32(a + 4 * i, (q >> (32 * i)) as u32);
    }
}

/// The lanes one point leaves in the VU registers (the values of the last point are what the function leaves).
struct PointStep {
    p: V,
    acc: V,
    n: V,
    abs_xy: V,
    v5: V,
    v6: V,
    v7: V,
    v8: V,
}

/// One point: the VU0 side tests of `p` (the raw position) against the mixed frustum rows.
fn point_step(raw: V, mix: &[V; 3], m20: V, m22: V, m24: V) -> PointStep {
    let p = raw * m24.w();
    let acc = V::madd(mix[0] * V::splat(p.x()), mix[1], V::splat(p.y()));
    let n = V::madd(acc, mix[2], V::splat(p.z()));
    let v8 = m22 * p.w();
    // vmulz (27cfa4) reads vf2.z = n.z as the retail loop does (the p.z variant fails every call).
    let v5 = m20 * V::splat(n.z());
    let abs_xy = V([n.x().abs().0, n.y().abs().0, 0, 0]);
    let v6 = abs_xy - v8;
    let v7 = v5 - v6;
    PointStep { p, acc, n, abs_xy, v5, v6, v7, v8 }
}

/// 0x27cf08 frustum_point_flags(points, count, flags) -> void: for each 32-byte point (xyz, scaled by its w), mixes the
/// point with the frustum rows and the basis rows on VU0, and stores flags[i] = 1 when both side tests (y, then x) are
/// non-negative, else 0. The points pointer and the flags pointer advance by 0x20 and 1 per point.
pub fn frustum_point_flags(c: &mut Cpu) -> bool {
    // Declined: the vf5/v0 result of the loop still differs from retail on the last point (vf5.xy, r2 hi), and the
    // cause is not found. Retail runs this entry.
    if FRUSTUM_DECLINED {
        return false;
    }
    let mut points = c.arg(0);
    let mut count = c.arg(1);
    let mut flags = c.arg(2);
    let rows = [c.rv(FRUSTUM), c.rv(FRUSTUM + 0x10), c.rv(FRUSTUM + 0x20)];
    let m20 = c.rv(FRUSTUM + 0x40);
    let m21 = c.rv(FRUSTUM + 0x50);
    let m22 = c.rv(FRUSTUM + 0x60);
    let m23 = c.rv(FRUSTUM + 0x70);
    let m24 = c.rv(FRUSTUM + 0x30);
    let basis = [c.rv(BASIS), c.rv(BASIS + 0x10), c.rv(BASIS + 0x20), c.rv(BASIS + 0x30)];
    // vf25..vf27: the frustum rows mixed by each of the first three basis rows (xyz lanes).
    let mix: [V; 3] = std::array::from_fn(|j| {
        let b = basis[j];
        let acc = V::madd(rows[0] * V::splat(b.x()), rows[1], V::splat(b.y()));
        V::madd(acc, rows[2], V::splat(b.z()))
    });

    let mut last;
    loop {
        let s = point_step(c.rv(points), &mix, m20, m22, m24);
        let (x7, y7) = (s.v7.x().0, s.v7.y().0);
        // bltz on the y lane (bit 63 of the qmfc2 value) first, then on the x lane.
        let inside = y7 >> 31 == 0 && x7 >> 31 == 0;
        c.w8(flags, u8::from(inside));
        flags += 1;
        points += 0x20;
        count = count.wrapping_sub(1);
        last = s;
        if count == 0 {
            break;
        }
    }

    // The VU registers the retail loop leaves: the last point's values, the loaded rows, and the untouched lanes.
    let x7 = last.v7.x().0;
    let w = last.p.w();
    c.set_vf(1, last.p);
    c.set_vf(2, c.vf(2).with(last.n, XYZ).with(last.abs_xy, X | Y));
    c.set_vf(3, c.vf(3).with(V::ZERO_W1 + V::splat(w), X).with(V::ZERO_W1 - V::splat(w), Y));
    c.set_vf(5, c.vf(5).with(last.v5, X | Y));
    c.set_vf(6, rows[0].with(last.v6, X | Y));
    c.set_vf(7, rows[1].with(last.v7, X | Y));
    c.set_vf(8, rows[2].with(last.v8, X | Y));
    for (n, raw) in [(11, basis[0]), (12, basis[1]), (13, basis[2]), (14, basis[3])] {
        c.set_vf(n, raw);
    }
    for (n, raw) in [(20, m20), (21, m21), (22, m22), (23, m23), (24, m24)] {
        c.set_vf(n, raw);
    }
    for (j, m) in mix.iter().enumerate() {
        let old = c.vf(25 + j);
        c.set_vf(25 + j, old.with(*m, XYZ));
    }
    let acc_old = c.acc();
    c.set_acc(acc_old.with(last.acc, XYZ));

    c.s(1, BASIS as u64);
    c.s(2, (x7 as u64) << 32);
    c.s(3, 1);
    c.s(4, points as u64);
    c.s(5, 0);
    c.s(6, flags as u64);
    true
}

/// 0x27cfe8 particle_jitter(pos) -> void: writes a random polar vector (rand_polar_vec, radius from 0.5 to 6) into
/// pos, adds the table at 0x166ec0, and when the 0x1e24d8 test returns non-zero, adds the 0x173f60 vector and scales
/// the result by 0.75 before the final add.
pub fn particle_jitter(c: &mut Cpu) -> bool {
    let pos = c.arg(0);
    let (saved_s0, saved_s1, saved_ra) = (c.r[16], c.r[17], c.r[31]);
    h::with_stack(c, JITTER_FRAME, |c, frame| {
        store_quad(c, frame, saved_s0);
        store_quad(c, frame + 0x10, saved_s1);
        store_quad(c, frame + 0x20, saved_ra);

        c.s(1, 0x40c0_0000);
        h::call_keep(c, RAND_POLAR, &[pos], &[HALF, SIX]);
        c.s(2, 0x16_0000);
        h::call_keep(c, VEC_ADD, &[pos, pos, JITTER_TABLE], &[]);

        c.s(2, 0x14_0000);
        let a3 = c.r32(TEST_ARG_ADDR);
        h::call_keep(c, TEST_HELPER, &[JITTER_TABLE, pos, 0x82, a3, 0], &[]);
        c.s(6, JITTER_TABLE as u64);
        if c.g(2) != 0 {
            h::call_keep(c, MOVE_HELPER, &[pos, MOVE_TABLE, JITTER_TABLE], &[]);
            c.s(1, 0x3f40_0000);
            h::run_keep(c, vecmath::vec_scale, &[pos, pos], &[THREE_QUARTERS]);
            h::call_keep(c, VEC_ADD, &[pos, pos, JITTER_TABLE], &[]);
        }
    });
    true
}
