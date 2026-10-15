//! Hero aim and steering helpers from the L4_03 record: a 16-slot ring copy, the aim vector update, and the aim lock.
use crate::h::{self, with_stack};
use crate::hero_space;
use crate::motion;
use crate::ps2::*;
use crate::trig;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Global block the aim code reads (`lui 0x14` + `addiu -0xcb0`).
const BASE: u32 = 0x0013_F350;
/// Upper half of the block address (`lui 0x14`), which retail leaves in v0 on some paths.
const HI: u32 = 0x0014_0000;
/// Hero scratch block at `HI - 0xbd0`, used as the target of the aim vector.
const ALT: u32 = 0x0013_F430;
/// Ring copy destination (`HI - 0xc30`).
const RING_DST: u32 = 0x0013_F3D0;
/// Ring source table (`RING_DST + 0x1a80`).
const RING_TABLE: u32 = 0x0014_0E50;
/// Ring cursor word (`BASE + 0x21b0`).
const RING_INDEX: u32 = BASE + 0x21B0;
/// Ring count word (`BASE + 0x21b4`).
const RING_COUNT: u32 = BASE + 0x21B4;
/// Retail constant at `0x160000 - 0x1294`: the upper clamp used in the aim vector.
const CLAMP_LIMIT_ADDR: u32 = 0x0015_ED6C;
/// Float 1.0.
const ONE: F = F(0x3f80_0000);
/// Float 0.5.
const HALF: F = F(0x3f00_0000);

/// Stores the listed registers into the retail frame at `sp`: each entry is (byte offset, register number).
fn save_frame(c: &mut Cpu, sp: u32, ints: &[(u32, usize)], floats: &[(u32, usize)]) {
    for &(off, reg) in ints {
        let v = c.r[reg];
        c.mem.wr128(sp + off, v);
    }
    for &(off, reg) in floats {
        let v = F(c.f[reg] as u32);
        c.wf(sp + off, v);
    }
}

/// 0x206850 ring_slot_copy() -> v0: pulls the next ring slot into 0x13f3d0 (up to 16 times, stops when 0x206450 says 1).
pub fn ring_slot_copy(c: &mut Cpu) -> bool {
    with_stack(c, 0x70, |c, sp| {
        save_frame(c, sp, &[(0x00, 16), (0x10, 17), (0x20, 18), (0x30, 19), (0x40, 20), (0x50, 21), (0x60, 31)], &[]);
        let mut last = 0u128;
        let mut stopped = false;
        for i in 0..16u32 {
            let (done, _) = h::call(c, 0x206450, &[i], &[]);
            if done == 1 {
                stopped = true;
                break;
            }
            let idx = ring_next_index(c);
            last = c.mem.rd128(RING_TABLE + idx.wrapping_shl(4));
            c.mem.wr128(RING_DST, last);
        }
        if stopped {
            c.ret(1);
        } else {
            c.r[2] = last;
        }
    });
    true
}

/// Ring cursor step of 0x206850: when the count is at least 2, takes one slot off the cursor (mod 32) and
/// returns the new cursor; otherwise returns the current cursor.
fn ring_next_index(c: &mut Cpu) -> u32 {
    let count = c.r32(RING_COUNT) as i32;
    if count < 2 {
        return c.r32(RING_INDEX);
    }
    c.w32(RING_COUNT, count.wrapping_sub(1) as u32);
    let cur = c.r32(RING_INDEX) as i32;
    let a0 = cur.wrapping_add(0x1F);
    let mut v0 = cur.wrapping_add(0x3E);
    if a0 >= 0 {
        v0 = a0;
    }
    v0 = (v0 >> 5) << 5;
    let next = a0.wrapping_sub(v0) as u32;
    c.w32(RING_INDEX, next);
    next
}

/// 0x206f10 aim_vector(f12 scale): retargets the aim vector from the hero's yaw or gravity-frame state (mode 0),
/// or places the aim point ahead of the hero (modes 1 and 2); other modes do nothing.
pub fn aim_vector(c: &mut Cpu) -> bool {
    let scale = c.farg(0);
    with_stack(c, 0x80, |c, sp| {
        save_frame(c, sp, &[(0x30, 16), (0x40, 17), (0x50, 31)], &[(0x60, 20), (0x68, 21), (0x70, 22)]);
        match c.r8(BASE + 0x20B3) {
            0 => aim_mode0(c, sp, scale),
            1 | 2 => aim_point_ahead(c, sp, scale, ALT),
            _ => {}
        }
    });
    true
}

/// Mode 0 of `aim_vector`: builds the direction from the two hero vectors, clamps the length and writes the
/// result into the hero block.
fn aim_mode0(c: &mut Cpu, sp: u32, scale: F) {
    let mut f20 = c.rf(BASE + 0x190);
    let f21 = c.rf(BASE + 0x180);
    let state = c.r32(BASE + 0x208C);
    let lh30a = c.r16(BASE + 0x30A) as i16;
    if state == 4 && lh30a == 0 && c.r16(BASE + 0x4A4) as i16 != 0 && c.r8(BASE + 0x257) != 0 {
        let gp_value = c.rf((c.g(28) as u32).wrapping_sub(0x7E94));
        f20 = gp_value * F(0x4006_6666);
    }
    if lh30a != 0 && c.r32(BASE + 0x2084) == 0xE {
        let limit = c.rf(CLAMP_LIMIT_ADDR);
        if limit < f20 {
            f20 = limit;
        }
    }

    let cos_v = h::run(c, trig::fast_cos, &[], &[f21]).1 * f20;
    c.wf(sp, cos_v);
    let sin_v = h::run(c, trig::fast_sin, &[], &[f21]).1 * f20;
    let e0 = c.rf(BASE + 0xE0);
    let e4 = c.rf(BASE + 0xE4);
    let e8 = c.rf(BASE + 0xE8);
    c.wf(sp + 4, sin_v);
    c.wf(sp + 8, e8);
    f20 = -(e0 * cos_v + e4 * sin_v);

    let len_dir = h::run(c, vecmath::length_xy, &[sp], &[]).1;
    let len_e0 = h::run(c, vecmath::length_xy, &[BASE + 0xE0], &[]).1;
    let zero = F(0);
    f20 = if len_dir == zero || len_e0 == zero { zero } else { (f20 / len_dir) / len_e0 };
    f20 = (f20 + ONE) * F(0x3fc0_0000);

    let bound = if c.r32(BASE + 0x2084) == 0x81 { F(0x3fd9_999a) } else { F(0x4040_0000) };
    if bound < f20 {
        f20 = bound;
    }
    if f20 < ONE {
        f20 = ONE;
    }

    let s0 = ALT;
    let scaled = scale * f20;
    h::vec_sub(c, sp, sp, s0);
    let len = h::run(c, vecmath::length_xy, &[sp], &[]).1;
    if scaled < len {
        h::run(c, vecmath::vec_set_length_xy, &[sp, sp], &[scaled]);
    }
    h::vec_add(c, s0, s0, sp);
    let copy = c.mem.rd128(s0);
    c.mem.wr128(s0 + 0x50, copy);
    let far = c.mem.rd128(s0 + 0x840);
    c.mem.wr128(sp + 0x10, far);
    let f0 = h::call(c, 0x2063F8, &[sp + 0x10], &[]).1;
    let v = c.rf(BASE + 0x168) - f0;
    c.wf(BASE + 0x194, v);
}

/// Modes 1 and 2 of `aim_vector` (retail 0x207154): sets the length of the hero's offset, slews it toward the
/// scale and places the point ahead of `s0`.
fn aim_point_ahead(c: &mut Cpu, sp: u32, scale: F, s0: u32) {
    let len = h::run(c, hero_space::length_off_up, &[s0], &[]).1;
    c.wf(sp + 0x20, len);
    let target = c.rf(s0 + 0xB0);
    h::run(c, motion::slew_toward, &[sp + 0x20], &[target, scale]);
    h::run(c, hero_space::set_length_xy_up, &[s0, s0], &[F(0)]);
    let ahead = c.rf(sp + 0x20);
    h::run(c, hero_space::point_ahead, &[s0, s0], &[ahead]);
}

/// 0x208018 aim_lock(f12 x, f13 y, f14 z): latches the aim target (retail 0x2009a0) when the lock is free and
/// stores the aim angle in the hero block.
pub fn aim_lock(c: &mut Cpu) -> bool {
    let (x, y, z) = (c.farg(0), c.farg(1), c.farg(2));
    with_stack(c, 0x60, |c, sp| {
        save_frame(c, sp, &[(0x00, 16), (0x10, 17), (0x20, 18), (0x30, 31)], &[(0x40, 20), (0x48, 21), (0x50, 22), (0x58, 23)]);
        let v0 = aim_lock_body(c, x, y, z);
        c.ret(v0);
    });
    true
}

/// Body of `aim_lock`; returns the value retail leaves in v0.
fn aim_lock_body(c: &mut Cpu, x: F, y: F, z: F) -> u32 {
    if c.r32(BASE + 0xA58) != 0 {
        return HI;
    }
    let mut f20 = c.rf(BASE + 0x98);
    if HALF < c.rf(BASE + 0x229C) {
        h::call(c, 0x204970, &[0], &[ONE]);
        f20 = c.rf(BASE + 0x180);
        c.w32(BASE + 0xA58, 1);
        c.w32(BASE + 0xA54, 0);
    }
    let (s0, _) = h::call(c, 0x2009A0, &[], &[x, f20, y, z]);
    if s0 == 0 {
        c.wf(BASE + 0xA5C, f20);
        return BASE;
    }
    let lock_free = c.r32(BASE + 0xA54) == 0;
    if !lock_free && c.r32(BASE + 0xA58) == 0 {
        c.wf(BASE + 0xA5C, f20);
        return BASE;
    }
    let dx = c.rf(s0 + 0x10) - c.rf(BASE + 0x80);
    let dy = c.rf(s0 + 0x14) - c.rf(BASE + 0x84);
    let angle = h::run(c, trig::fast_atan2, &[], &[dx, dy]).1;
    c.w32(BASE + 0xA54, s0);
    c.wf(BASE + 0xA5C, angle);
    BASE
}
