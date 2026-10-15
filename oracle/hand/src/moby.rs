//! Moby (actor) per-frame helpers.
use crate::cells::refile_cells;
use crate::ps2::*;
use crate::trig::{vu_cos, vu_sin};
use ee_oracle::cpu::Cpu;

/// Bounding spheres of mobys without an animation frame (16 bytes per entry, indexed by moby +0x50).
const DEFAULT_SPHERES: u32 = 0x196e00;

/// VU0 micro 0xd18 (vcallms 0x1a3): the Euler rotation rows in VF20..VF22 from the angles in VF1.
const MICRO_EULER_ROWS: u32 = 0xd18;

/// rows * m (row vectors, three rows): each row r becomes r.x * m[0] + r.y * m[1] + r.z * m[2] (VU0 micro 0xf28).
fn rotate_rows(rows: &mut [V; 3], m: [V; 3]) {
    for r in rows.iter_mut() {
        let acc = m[0] * r.x();
        let acc = V::madd(acc, m[1], V::splat(r.y()));
        *r = V::madd(acc, m[2], V::splat(r.z()));
    }
}

/// VU0 micro 0xd18: the rotation rows of Euler angles (x, y, z), applied in that order (an angle that is exactly zero
/// skips its step).
pub fn euler_rows(angles: V) -> [V; 3] {
    let zero = V::default();
    let one = F::ONE;
    let mut rows = [V([one.0, 0, 0, 0]), V([0, one.0, 0, 0]), V([0, 0, one.0, 0])];
    // The MAC zero flags of angles + 0 decide the steps.
    let a = angles + zero;
    let lane = |f: F| F::ZERO.vu_add(f);
    if a.x().get() != 0.0 {
        let x = angles.x();
        let (s, c) = (vu_sin(x), vu_cos(x));
        rows[1] = V([0, one.0, 0, 0]).with(V([0, lane(c).0, lane(s).0, 0]), Y | Z);
        rows[2] = V([0, F::ZERO.vu_sub(s).0, lane(c).0, 0]);
    }
    if a.y().get() != 0.0 {
        let y = lane(a.y());
        let (s, c) = (vu_sin(y), vu_cos(y));
        let m = [
            V([lane(c).0, 0, F::ZERO.vu_sub(s).0, 0]),
            V([0, one.0, 0, 0]),
            V([lane(s).0, 0, lane(c).0, 0]),
        ];
        rotate_rows(&mut rows, m);
    }
    if a.z().get() != 0.0 {
        let z = lane(a.z());
        let (s, c) = (vu_sin(z), vu_cos(z));
        let m = [V([lane(c).0, lane(s).0, 0, 0]), V([F::ZERO.vu_sub(s).0, lane(c).0, 0, 0]), V([0, 0, one.0, 0])];
        rotate_rows(&mut rows, m);
    }
    rows
}

/// Register numbers (MIPS) the retail function writes.
const REG_AT: usize = 1;
const REG_V0: usize = 2;
const REG_V1: usize = 3;
const REG_A0: usize = 4;
const REG_A1: usize = 5;
const REG_A2: usize = 6;
const REG_A3: usize = 7;
const REG_T0: usize = 8;
const REG_T9: usize = 25;

/// Float bits of 1024.0 (the position scale of the world sphere).
const POS_SCALE: u32 = 0x4480_0000;

/// A 32-bit value as the 64-bit sign-extended GPR value.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// The four lanes as one 128-bit value (lane 0 lowest).
fn bits(v: V) -> u128 {
    v.0.iter().enumerate().fold(0u128, |q, (i, &l)| q | (l as u128) << (32 * i))
}

/// The 128-bit value as four lanes (lane 0 lowest).
fn unbits(q: u128) -> V {
    V(std::array::from_fn(|i| (q >> (32 * i)) as u32))
}

/// 0x2436c0 actor_transform_bsphere(moby): the rotation rows (+0xc0..+0xe0, from the Euler angles at +0x40 unless
/// flag 0x100 keeps the stored rows; flag 0x8000 mirrors the second row), the world bounding sphere at +0x00
/// (animation frame sphere, blended between frames +0x52 and +0x53 by +0x54, scaled by +0x2c, rotated and moved to
/// the position * 1024), the update counter +0xa8, and a grid refile when the sphere's grid cell changed.
pub fn actor_transform_bsphere(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let neg = c.r8(m + 0x20) as i8;
    let flags = c.r16(m + 0x34);
    c.s(REG_A1, neg as i64 as u64);
    c.s(REG_T0, flags as u64);
    if neg < 0 {
        return true;
    }
    let class = c.r32(m + 0x24);
    c.s(REG_V0, sx(class));
    c.s(REG_T9, sx(m));
    let frame = c.r8(m + 0x52) as u32;
    c.s(REG_A0, frame as u64);
    c.s(REG_A3, (flags & 0x8000) as u64);
    c.s(REG_A2, (flags & 0x100) as u64);
    let next = c.r8(m + 0x53) as u32;
    c.s(REG_A1, next as u64);
    let scale_word = c.r32(m + 0x2c);
    c.s(REG_V1, sx(scale_word));
    // Rows: VF20..VF22 from the micro (VF1 = the angles), or the stored rows with flag 0x100 (VF23 = VF0).
    let mut rows = if flags & 0x100 != 0 {
        let stored = [c.rv(m + 0xc0), c.rv(m + 0xd0), c.rv(m + 0xe0)];
        for (i, r) in stored.iter().enumerate() {
            c.set_vf(20 + i, *r);
        }
        let vf0 = c.vf(0);
        c.set_vf(23, vf0);
        stored
    } else {
        let angles = c.rv(m + 0x40);
        c.set_vf(1, angles);
        let _ = c.vu0_micro(MICRO_EULER_ROWS);
        [c.vf(20), c.vf(21), c.vf(22)]
    };
    let cur = c.r8(m + 0x71) as u32;
    c.s(REG_A2, cur as u64);
    // qmtc2 $v1, vf8 (in the branch delay slot).
    let vf8 = unbits(c.r[REG_V1]);
    c.set_vf(8, vf8);
    let vf18 = if frame != next {
        // Blend between the frame sphere (VF18) and the next one (VF19) by the float at +0x54.
        let a1v = (next << 2).wrapping_add(class);
        c.s(REG_A1, sx(a1v));
        c.s(REG_AT, 0xff);
        let from = if frame == 0xff {
            let i = c.r8(m + 0x50) as u32;
            let addr = (i << 4).wrapping_add(DEFAULT_SPHERES);
            c.s(REG_A0, addr as u64);
            c.s(REG_AT, DEFAULT_SPHERES as u64);
            c.rv(addr)
        } else {
            let a0v = (frame << 2).wrapping_add(class);
            let p = c.r32(a0v.wrapping_add(0x48));
            c.s(REG_A0, sx(p));
            c.rv(p)
        };
        let p_next = c.r32(a1v.wrapping_add(0x48));
        c.s(REG_A1, sx(p_next));
        let t_word = c.r32(m + 0x54);
        c.s(REG_A0, sx(t_word));
        let vf17 = unbits(c.r[REG_A0]);
        let vf19 = c.rv(p_next);
        c.set_vf(17, vf17);
        c.set_vf(19, vf19);
        let t = vf17.x();
        // vmulax ACC = VF19 * t; vmaddaw ACC += VF18 * 1; vmsubx VF18 = ACC - VF18 * t.
        let acc = vf19 * t;
        let acc = V::madd(acc, from, V::splat(F::ONE));
        c.set_acc(acc);
        V::msub(acc, from, V::splat(t))
    } else if frame != cur {
        c.w8(m + 0x71, frame as u8);
        let a0v = (frame << 2).wrapping_add(class);
        let p = c.r32(a0v.wrapping_add(0x48));
        c.s(REG_A0, sx(p));
        let f = c.rv(p);
        c.wv(m + 0xf0, f);
        f
    } else {
        c.rv(m + 0xf0)
    };
    // qmfc2 $at, vf20 (delay slot of the branch below).
    c.r[REG_AT] = bits(rows[0]);
    if flags & 0x8000 != 0 {
        let vf0 = c.vf(0);
        rows[1] = rows[1].with(vf0 - rows[1], XYZ);
        c.set_vf(21, rows[1]);
    }
    for (i, r) in rows.iter().enumerate() {
        c.wv(m + 0xc0 + 0x10 * i as u32, *r);
    }
    // vmulx.xyzw vf5 = VF18 * scale; lqc2 vf7 = position; lui/qmtc2 at -> vf6 = (1024, 0, vf20.z, vf20.w).
    let s = vf18 * vf8.x();
    let vf7 = c.rv(m + 0x10);
    c.s(REG_AT, POS_SCALE as u64);
    let vf6_in = unbits(c.r[REG_AT]);
    let vf7 = vf7.with(vf7 * vf6_in.x(), XYZ);
    let mut acc = c.acc();
    let a0 = rows[0] * s.x();
    acc = acc.with(a0, XYZ);
    let a1 = V::madd(acc, rows[1], V::splat(s.y()));
    acc = acc.with(a1, XYZ);
    let a2 = V::madd(acc, rows[2], V::splat(s.z()));
    acc = acc.with(a2, XYZ);
    let vf5 = s.with(V::madd(acc, vf7, V::splat(F::ONE)), XYZ);
    c.set_acc(acc);
    let vf6 = vf5.ftoi(0);
    let n = c.r16(m + 0xa8);
    c.s(REG_V0, n as u64);
    let cnt = c.r32(m + 0x94);
    c.wv(m, vf5);
    c.s(REG_V0, n as u64 + 1);
    c.w16(m + 0xa8, n.wrapping_add(1));
    // qmfc2 $at, vf6 (delay slot).
    c.r[REG_AT] = bits(vf6);
    c.set_vf(5, vf5);
    c.set_vf(6, vf6);
    c.set_vf(7, vf7);
    c.set_vf(18, vf18);
    if cnt == 0 {
        return true;
    }
    // Integer lanes of the sphere: x, y, z, radius.
    let at = vf6.0.map(|l| l as i32);
    let (v0, v1, a_dif) = (
        [at[3] as u32, at[3] as u32, 0, 0],
        [at[0].wrapping_add(at[3]), at[1].wrapping_add(at[3]), at[2], at[3]],
        [at[0].wrapping_sub(at[3]), at[1].wrapping_sub(at[3]), at[2], at[3]],
    );
    c.r[REG_V0] = bits(V(v0));
    c.r[REG_V1] = bits(V(v1.map(|w| (w >> 14) as u32)));
    c.r[REG_AT] = bits(V(a_dif.map(|w| (w >> 14) as u32)));
    // Grid cell: the bytes of (x - r, y - r, x + r, y + r) >> 14 of the integer sphere.
    let [x, y, _, r] = at;
    let b = |v: i32| (v >> 14) as u8;
    let cell = u32::from_le_bytes([b(x.wrapping_sub(r)), b(y.wrapping_sub(r)), b(x.wrapping_add(r)), b(y.wrapping_add(r))]);
    c.r[REG_A1] = cell as u128;
    // The old cell compares as a sign-extended word against the zero-extended new one.
    let old = c.r32(m + 0xa0);
    c.s(REG_A2, sx(old));
    c.s(REG_A3, (cell & 0xc0c0) as u64);
    if (cell == old && cell & 0x8000_0000 == 0) || cell & 0xc0c0 != 0 {
        return true;
    }
    c.s(REG_A0, sx(m));
    refile_cells(c, m, cell);
    true
}
