//! Small geometry and ballistics helpers: polygon test, quadratic roots, projectile and ballistic-aim maths,
//! rotation builders and matrix column normalisation.
use crate::h;
use crate::ps2::*;
use crate::rcall::*;
use crate::trig;
use ee_oracle::cpu::Cpu;

const HALF: F = F(0x3f00_0000);
/// Retail vector add: out = a + b (VU0).
const VEC_ADD: u32 = 0x1f1810;
/// Retail vector subtract: out = a - b.
const VEC_SUB: u32 = 0x1f1840;
/// Retail vector scale: out = v * s (VU0).
const VEC_SCALE: u32 = 0x1f1898;
/// Retail length of the xy part of a vector.
const LENGTH_XY: u32 = 0x1f19a0;
/// Retail integer to float conversion.
const INT_TO_FLOAT: u32 = 0x1f27d8;
/// Retail quaternion product (VU0).
const QUAT_MUL: u32 = 0x1f2400;
/// Retail VU0 set-length (out = v scaled to len).
const VEC_SET_LENGTH: u32 = 0x1f1a98;
/// Retail VU0 sine and cosine micro calls.
const FAST_SIN: u32 = 0x1f1d98;
const FAST_COS: u32 = 0x1f1d80;
/// Retail fast atan2 (EE side with VU0 polynomial).
const FAST_ATAN2: u32 = 0x1f1e48;
/// Retail zero of a 4-float vector.
const ZERO_VEC4: u32 = 0x1f17f8;
/// Retail rotation-matrix builders about z and y.
const EULER_Z: u32 = 0x1f2008;
const EULER_Y: u32 = 0x1f2028;
/// Retail 4x4 matrix product (VU0).
const MAT4_MUL: u32 = 0x1f2370;

/// 0x24bee0 solve_quadratic(a, b, c, root_hi*, root_lo*) -> count: roots (-b +- sqrt|disc|) / 2a stored larger
/// first; returns 1 (single root, disc 0), 2 (disc > 0) or 0 (disc < 0, roots of |disc|).
pub fn solve_quadratic(c: &mut Cpu) -> bool {
    let (a, b, k) = (c.farg(0), c.farg(1), c.farg(2));
    let (r1, r2) = (c.arg(0), c.arg(1));
    let saved = save(c);
    let disc = b * b - a * F(0x4080_0000) * k;
    if disc == F::ZERO {
        c.wf(r1, (-b) / (a + a));
        restore(c, &saved);
        c.ret(1);
        return true;
    }
    let root = sqrt_q(c, disc.abs());
    let (nb, two_a) = (-b, a + a);
    let (x1, x2) = ((nb + root) / two_a, (nb - root) / two_a);
    c.wf(r1, x1);
    c.wf(r2, x2);
    if x1 < x2 {
        c.wf(r2, x1);
        c.wf(r1, x2);
    }
    restore(c, &saved);
    c.ret(if F::ZERO < disc { 2 } else { 0 });
    true
}

/// 0x24be48 ballistic_pos(out, base, vel, n, g): out = base + vel * n, lowered by g * ((n * n + n) >> 1) in z.
pub fn ballistic_pos(c: &mut Cpu) -> bool {
    let (out, base, vel, n, g) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.farg(0));
    let (sp, b) = push_frame(c, 0x70);
    let (_, steps) = h::call(c, INT_TO_FLOAT, &[n], &[]);
    h::call(c, VEC_SCALE, &[b, vel], &[steps]);
    let sum = (n.wrapping_mul(n).wrapping_add(n) as i32) >> 1;
    let (_, sum_f) = h::call(c, INT_TO_FLOAT, &[sum as u32], &[]);
    let z = c.rf(b + 8) - sum_f * g;
    c.wf(b + 8, z);
    h::call(c, VEC_ADD, &[out, base, b], &[]);
    pop_frame(c, &sp);
    true
}

/// 0x24c120 point_in_polygon_xy(pt, verts, n) -> 0/1: even-odd crossing test of pt's y against the n edges of the
/// polygon (16-byte vertices, x at +0, y at +4), counting crossings left of pt.x.
pub fn point_in_polygon_xy(c: &mut Cpu) -> bool {
    let (pt, verts, n) = (c.arg(0), c.arg(1), c.arg(2) as i32);
    let mut inside = false;
    if n > 0 {
        let (px, py) = (c.rf(pt), c.rf(pt + 4));
        for i in 0..n as u32 {
            let cur = verts + 16 * i;
            let next = verts + 16 * ((i + 1) % n as u32);
            let (cx, cy, nx, ny) = (c.rf(cur), c.rf(cur + 4), c.rf(next), c.rf(next + 4));
            if (cy < py && py <= ny) || (ny < py && py <= cy) {
                let x = cx + (py - cy) / (ny - cy) * (nx - cx);
                if x < px {
                    inside = !inside;
                }
            }
        }
    }
    c.ret(inside as u32);
    true
}

/// 0x24d7b0 ballistic_aim(from, to, time_out*, speed, g) -> vz: flight time = horizontal distance / speed (stored
/// to time_out when non-null); returns the launch vz that lands on `to` under gravity g (0 for zero time).
pub fn ballistic_aim(c: &mut Cpu) -> bool {
    let (from, to, time_out) = (c.arg(0), c.arg(1), c.arg(2));
    let (speed, g) = (c.farg(0), c.farg(1));
    let (sp, b) = push_frame(c, 0x60);
    h::call(c, VEC_SUB, &[b, to, from], &[]);
    let (_, dist) = h::call(c, LENGTH_XY, &[b], &[]);
    let t = dist / speed;
    if time_out != 0 {
        c.wf(time_out, t);
    }
    let vz = if t == F::ZERO {
        F::ZERO
    } else {
        let drop = g * (t * t) * HALF;
        let dz = c.rf(from + 8) - c.rf(to + 8);
        -(dz + drop) / t
    };
    pop_frame(c, &sp);
    c.fret(vz);
    true
}

/// 0x24cb10 euler_to_quat(out, angles*): out = qx * qy * qz from axis quaternions of the three angles.
pub fn euler_to_quat(c: &mut Cpu) -> bool {
    let (out, ang) = (c.arg(0), c.arg(1));
    let (sp, b) = push_frame(c, 0x80);
    for (axis, q) in [(0u32, b + 0x20), (1, b + 0x10), (2, b)] {
        let a = c.rf(ang + 4 * axis);
        h::run(c, trig::axis_quat, &[q, axis], &[a]);
    }
    h::call(c, QUAT_MUL, &[out, b + 0x20, b + 0x10], &[]);
    h::call(c, QUAT_MUL, &[out, out, b], &[]);
    pop_frame(c, &sp);
    true
}

/// 0x24ed30 mat3_normalize_columns(m): each column (elements at +4k, +0x10 + 4k, +0x20 + 4k) set to unit length.
pub fn mat3_normalize_columns(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let sp0 = c.g(29);
    let (_, b) = push_frame(c, 0x50);
    for col in 0..3u32 {
        c.w32(b + 0xc, 0);
        for i in 0..3u32 {
            let f = c.rf(m + 4 * col + 0x10 * i);
            c.wf(b + 4 * i, f);
        }
        h::call_keep(c, VEC_SET_LENGTH, &[b, b], &[F::ONE]);
        for i in 0..3u32 {
            let f = c.rf(b + 4 * i);
            c.wf(m + 4 * col + 0x10 * i, f);
        }
    }
    // The last copy-back loop leaves v0 = 0, v1 = 3, a0 = -1 and f0 = the last element it stored.
    c.s(2, 0);
    c.s(3, 3);
    c.s(4, u64::MAX);
    c.f[0] = c.rf(b + 8).0;
    c.s(29, sp0);
    true
}

/// 0x24fd90 axis_angle_quat(out, axis, angle): out = (axis * sin(angle / 2), cos(angle / 2)).
pub fn axis_angle_quat(c: &mut Cpu) -> bool {
    let (out, axis, half) = (c.arg(0), c.arg(1), c.farg(0) * HALF);
    let sp0 = c.g(29);
    let (_, b) = push_frame(c, 0x40);
    let (_, sin) = h::call_keep(c, FAST_SIN, &[], &[half]);
    h::call_keep(c, VEC_SCALE, &[out, axis], &[sin]);
    let (_, cos) = h::call_keep(c, FAST_COS, &[], &[half]);
    c.wf(out + 0xc, cos);
    c.s(29, sp0);
    let _ = b;
    true
}

/// 0x24fdf8 vec_pull_to_limit(limit*, v*): every negative component of v is pulled along v - limit so that it
/// ends 0.1 inside the limit's component.
pub fn vec_pull_to_limit(c: &mut Cpu) -> bool {
    let (lim_p, v) = (c.arg(0), c.arg(1));
    let sp0 = c.g(29);
    let (_, b) = push_frame(c, 0xb0);
    let t = b + 0x10;
    let lim = c.rv(lim_p);
    c.wv(b, lim);
    for i in 0..3u32 {
        if c.rf(v + 4 * i) < F::ZERO {
            h::call_keep(c, VEC_SUB, &[t, v, b], &[]);
            let k = ((c.rf(b + 4 * i) - F(0x3dcc_cccd)) / c.rf(t + 4 * i)).abs();
            h::call_keep(c, VEC_SCALE, &[t, t], &[k]);
            h::call_keep(c, VEC_ADD, &[v, b, t], &[]);
        }
    }
    c.s(29, sp0);
    true
}

/// Retail fast_atan2(a, b) through the retail routine (leaves its VU and scratch state as retail does).
fn retail_atan2(c: &mut Cpu, a: F, b: F) -> F {
    h::call_keep(c, FAST_ATAN2, &[], &[a, b]).1
}

/// 0x24fef0 mat4_to_euler(m, out*): yaw/pitch/roll angles (out[2], out[1], out[0]) that undo the rotation of m,
/// by rotating a copy of m about z then y and reading its remaining rotation.
pub fn mat4_to_euler(c: &mut Cpu) -> bool {
    let (m, out) = (c.arg(0), c.arg(1));
    let sp0 = c.g(29);
    let (_, b) = push_frame(c, 0xe0);
    let (copy, rot) = (b + 0x40, b + 0x80);
    for row in 0..4u32 {
        let v = c.rv(m + 0x10 * row);
        c.wv(copy + 0x10 * row, v);
    }
    h::call_keep(c, ZERO_VEC4, &[copy + 0x30], &[]);
    c.wf(copy + 0x3c, F::ONE);
    let (x, y) = (c.rf(copy), c.rf(copy + 4));
    let yaw = retail_atan2(c, x, y);
    c.w32(rot, 0);
    c.w32(rot + 4, 0);
    c.wf(rot + 8, -yaw);
    h::call_keep(c, EULER_Z, &[b, rot], &[]);
    h::call_keep(c, MAT4_MUL, &[copy, b, copy], &[]);
    let (x, z) = (c.rf(copy), c.rf(copy + 8));
    let pitch = retail_atan2(c, x, -z);
    c.w32(rot, 0);
    c.w32(rot + 8, 0);
    c.wf(rot + 4, -pitch);
    h::call_keep(c, EULER_Y, &[b, rot], &[]);
    h::call_keep(c, MAT4_MUL, &[copy, b, copy], &[]);
    let (y, z) = (c.rf(copy + 0x14), c.rf(copy + 0x18));
    let roll = retail_atan2(c, y, z);
    c.wf(out + 8, yaw);
    c.wf(out + 4, pitch);
    c.wf(out, roll);
    c.w32(rot + 4, 0);
    c.wf(rot, -roll);
    c.w32(rot + 8, 0);
    c.f[1] = (-roll).0;
    c.s(29, sp0);
    true
}
