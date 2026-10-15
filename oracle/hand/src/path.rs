//! Fading colour blocks, path-point distance and closest-point-on-segment queries, quaternion vector rotation and
//! small class/vector utilities of the 0x250000 overlay area.
use crate::h::*;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const PI: F = F(0x4049_0fdb);
const TAU: F = F(0x40c9_0fdb);

/// 0x2500f8 colour_fade_step(owner, fade): counts the fade's timer (h[0]) down and, per frame, mixes the three
/// colour bytes between the end colour (+4..6) and the target (+7..9; 0 = keep) with the fraction of the active
/// phase (phase flag h[1]: 0 fades out over duration h[6], 1 back over duration h[7]), then applies the colour with
/// 0x242bb8; when the timer runs out in phase 1 it applies the end colour.
pub fn colour_fade_step(c: &mut Cpu) -> bool {
    let (owner, fade) = (c.arg(0), c.arg(1));
    if c.r16(fade) as i16 == 0 {
        return true;
    }
    let timer_done = run(c, crate::mathx::tick_i16, &[fade], &[]).0 != 0;
    let mut phase = c.r16(fade + 2) as i16;
    if timer_done {
        if phase != 0 {
            let (b4, b5, b6) = (c.r8(fade + 4), c.r8(fade + 5), c.r8(fade + 6));
            call(c, 0x242bb8, &[owner, b4 as u32, b5 as u32, b6 as u32], &[]);
            return true;
        }
        c.w16(fade + 2, 1);
        let dur = c.r16(fade + 0xe) as i16 as i32;
        let t = scale_int(c, dur);
        c.w16(fade, t as u16);
        phase = c.r16(fade + 2) as i16;
    }
    let back = phase != 0;
    let dur_off = if back { 0xe } else { 0xc };
    let dur = c.r16(fade + dur_off) as i16 as i32;
    let total = F::from_int(scale_int(c, dur));
    let left = F::from_int(c.r16(fade) as i16 as i32);
    let frac = (total - left) / total;
    let mut out = [0u32; 3];
    for (i, o) in out.iter_mut().enumerate() {
        let end = c.r8(fade + 4 + i as u32) as i32;
        let tgt = c.r8(fade + 7 + i as u32) as i32;
        *o = if tgt == 0 {
            end as u32
        } else {
            let (from, to) = if back { (tgt, end) } else { (end, tgt) };
            (F::from_int(from) + F::from_int(to - from) * frac).to_int() as u32
        };
    }
    call(c, 0x242bb8, &[owner, out[0], out[1], out[2]], &[]);
    true
}

/// 0x250630 path_point_dist(p, q, mode) -> distance: 2D (xy) when mode > 0 and |p.z - q.z| < mode, else 3D.
pub fn path_point_dist(c: &mut Cpu) -> bool {
    let (p, q, mode) = (c.arg(0), c.arg(1), c.farg(0));
    let dz = c.rf(p + 8) - c.rf(q + 8);
    let flat = F::ZERO < mode && fabs(dz) < mode;
    with_stack(c, 0x60, |c, buf| {
        copy_q(c, buf, p);
        copy_q(c, buf + 0x10, q);
        let callee = if flat { 0x1f1a20 } else { 0x1f19e8 };
        let d = call_keep(c, callee, &[buf, buf + 0x10], &[]).1;
        c.fret(d);
    });
    if F::ZERO < mode {
        // retail's fabs(dz) call leaves f12 = dz (abs.s writes only f0); the dist helpers do not touch f12.
        c.set_farg(0, dz);
    }
    true
}

/// 0x250898 path_segment_closest(out, pos, a, b, mode) -> distance: closest point on segment [a,b] to pos. With
/// mode > 0 and all three heights within mode of each other, the xy-plane closest point (0x2506c0) is used, with
/// the height interpolated along the segment; otherwise the 3D projection, clamped to the nearer endpoint when it
/// leaves the segment's box.
pub fn path_segment_closest(c: &mut Cpu) -> bool {
    let (out, pos, a, b, mode) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.farg(0));
    let near_z = |c: &mut Cpu, p: u32| fabs(c.rf(pos + 8) - c.rf(p + 8)) < mode;
    if F::ZERO < mode && near_z(c, a) && near_z(c, b) {
        let d = call(c, 0x2506c0, &[out, pos, a, b], &[]).1;
        let along = dist_xy(c, a, out);
        let seg = dist_xy(c, a, b);
        with_stack(c, 0x20, |c, buf| {
            copy_q(c, buf, a);
            copy_q(c, buf + 0x10, b);
            call(c, 0x24e278, &[out, buf, buf + 0x10], &[along / seg]);
        });
        c.fret(d);
        return true;
    }
    with_stack(c, 0x60, |c, buf| {
        let (dir, base, dir_copy) = (buf, buf + 0x10, buf + 0x20);
        vec_sub(c, dir, b, a);
        copy_q(c, base, a);
        vec_set_length(c, dir, dir, F::ONE);
        copy_q(c, dir_copy, dir);
        let pos_dot = dot3(c, pos, dir);
        let base_dot = dot3(c, base, dir_copy);
        let len2 = dot3(c, dir, dir_copy);
        let t = -(-pos_dot + base_dot) / len2;
        vec_scale(c, out, dir, t);
        vec_add(c, out, out, base);
        let outside = (0..3).any(|i| {
            let (o, lo, hi) = (c.rf(out + 4 * i), c.rf(a + 4 * i), c.rf(b + 4 * i));
            (lo < o && hi < o) || (o < lo && o < hi)
        });
        if outside {
            let (da, db) = (dist3(c, out, a), dist3(c, out, b));
            copy_q(c, out, if da < db { a } else { b });
        }
        copy_q(c, buf + 0x30, pos);
        copy_q(c, buf + 0x40, out);
        let d = dist3(c, buf + 0x30, buf + 0x40);
        c.fret(d);
    });
    true
}

/// 0x250ed0 wrap_angle_pi(x) -> x wrapped into [-pi, pi) by truncating division: frac((x + pi) / 2pi) * 2pi - pi.
pub fn wrap_angle_pi(c: &mut Cpu) -> bool {
    let q = (c.farg(0) + PI) / TAU;
    let frac = q - F::from_int(q.to_int());
    c.fret(frac * TAU - PI);
    true
}

/// 0x250f78 is_class_500_540(moby) -> 1 when the pointer lies in the moby table and its class (+0xa6) is 500..=540.
pub fn is_class_500_540(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let gp = c.g(28) as u32;
    let (lo, hi) = (c.r32(gp.wrapping_sub(0x6ca8)), c.r32(gp.wrapping_sub(0x6ca0)));
    let hit = m != 0 && m >= lo && m <= hi && (c.r16(m + 0xa6) as u32).wrapping_sub(500) < 0x29;
    c.ret(hit as u32);
    true
}

/// 0x250fb8 is_special_class(moby) -> 1 when the moby class (+0xa6) is one of 0xe2, 0xcc, 0xde, 0x3ee, 0xd6, 0xe1,
/// 0xd5, 0xdf.
pub fn is_special_class(c: &mut Cpu) -> bool {
    let class = c.r16(c.arg(0) + 0xa6) as i16;
    c.ret([0xe2, 0xcc, 0xde, 0x3ee, 0xd6, 0xe1, 0xd5, 0xdf].contains(&class) as u32);
    true
}

/// 0x2522f0 vec_clamp_len(v, max): shortens v (xyz) to length max when it is longer.
pub fn vec_clamp_len(c: &mut Cpu) -> bool {
    let (v, max) = (c.arg(0), c.farg(0));
    let len = call_keep(c, 0x1f1970, &[v], &[]).1;
    if max < len {
        call_keep(c, 0x1f1a98, &[v, v], &[max]);
    }
    c.s(4, v as i32 as i64 as u64);
    true
}

/// 0x252520 point_in_obb(point, index) -> 1 when the point lies inside the unit box of the transform entry `index`
/// of the table at [0x16006c] (entries 0x80 bytes: origin at +0x30, inverse rows at +0x40); index -1 gives 0.
pub fn point_in_obb(c: &mut Cpu) -> bool {
    let (point, index) = (c.arg(0), c.arg(1) as i32);
    if index == -1 {
        c.ret(0);
        return true;
    }
    let entry = c.r32(0x16006c).wrapping_add((index as u32) << 7);
    let inside = with_stack(c, 0x40, |c, buf| {
        call_keep(c, 0x1f1840, &[buf, point, entry + 0x30], &[]);
        c.w32(buf + 0xc, 0);
        call_keep(c, 0x1f1c68, &[buf + 0x10, buf, entry + 0x40], &[]);
        (0..3).all(|i| {
            let x = c.rf(buf + 0x10 + 4 * i);
            F(0xbf80_0000) <= x && x <= F::ONE
        })
    });
    c.ret(inside as u32);
    true
}

/// 0x252738 quat_rotate(out, v, q): out = q * (v, 0) * conj(q).
pub fn quat_rotate(c: &mut Cpu) -> bool {
    let (out, v, q) = (c.arg(0), c.arg(1), c.arg(2));
    quat_rotate_body(c, out, v, q);
    true
}

fn quat_rotate_body(c: &mut Cpu, out: u32, v: u32, q: u32) {
    with_stack(c, 0x60, |c, buf| {
        let (conj, vec4, mid) = (buf, buf + 0x10, buf + 0x20);
        vec_scale(c, conj, q, F(0xbf80_0000));
        let qw = c.rf(q + 0xc);
        c.wf(conj + 0xc, qw);
        copy_q(c, vec4, v);
        c.w32(vec4 + 0xc, 0);
        quat_mul(c, mid, q, vec4);
        quat_mul(c, out, mid, conj);
    });
}

/// 0x2527c8 rotate_about_axis(out, v, axis, angle): out = v rotated by angle about axis (a copy when |angle| < 1e-5).
pub fn rotate_about_axis(c: &mut Cpu) -> bool {
    let (out, v, axis, angle) = (c.arg(0), c.arg(1), c.arg(2), c.farg(0));
    if fabs(angle) < F(0x3727_c5ac) {
        copy_q(c, out, v);
        return true;
    }
    with_stack(c, 0x40, |c, buf| {
        vec_set_length(c, buf, axis, F::ONE);
        call(c, 0x24fd90, &[buf, buf], &[angle]);
        quat_rotate_body(c, out, v, buf);
    });
    true
}
