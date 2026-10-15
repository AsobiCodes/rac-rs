//! Hand-written Rust for three functions of the l4_03 overlay: the countdown-slot ticker (0x20a050), the hero
//! edge guard (0x20a258) and the Comet-Strike wrench release (0x20a798).
use crate::geom;
use crate::h;
use crate::hero_space;
use crate::mathx;
use crate::objects;
use crate::ps2::*;
use crate::scene;
use crate::trail;
use crate::trig;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Eight 8-byte countdown slots: an i16 counter at slot+0, a neighbour pair at slot-2 and slot+2.
const SLOTS: u32 = 0x14_152c;
/// Retail routine that fires when a slot counter reaches 2.
const SLOT_FIRE: u32 = 0x20_9f28;
/// Global game block; the hero block sits at +0xe0 inside it.
const GLOBALS: u32 = 0x13_f350;
const HERO: u32 = GLOBALS + 0xe0;
/// Retail vector-init routine (zeroes a vector).
const VEC_INIT: u32 = 0x1f_17f8;
/// Retail routine that builds a 3x3 frame from a vector (out, v).
const FRAME_FROM_VEC: u32 = 0x1f_1fe8;
/// Retail routine that stops the hero.
const HERO_STOP: u32 = 0x20_6e90;
/// Retail routine that tests the wrench target (returns nonzero when valid).
const TARGET_CHECK: u32 = 0x20_4408;
/// Constant table: angle limits and aim offsets.
const ANGLE_CONSTS: u32 = 0x16_6d80;
/// Trail record used by the wrench afterimage.
const TRAIL: u32 = 0x14_0b00;

/// 0x20a050 tick_slots() -> (): ticks the eight countdown slots at 0x14152c; a counter that reaches 2 fires
/// 0x209f28 with its two neighbour halfwords and clears the halfword before it.
pub fn tick_slots(c: &mut Cpu) -> bool {
    let mut last = 0;
    for slot in 0..8u32 {
        let counter = SLOTS + 8 * slot;
        c.set_arg(0, counter);
        last = h::run(c, mathx::tick_i16, &[counter], &[]).0;
        if last == 2 {
            let a0 = c.r16(counter - 2) as i16 as i32 as u32;
            let a1 = c.r16(counter + 2) as i16 as i32 as u32;
            c.w16(counter - 4, 0);
            last = h::call(c, SLOT_FIRE, &[a0, a1], &[]).0;
            c.set_arg(0, a0);
            c.set_arg(1, a1);
        }
    }
    c.ret(last);
    true
}

/// 0x20a258 hero_edge_guard(vel, limit) -> (): when the hero's floor probe is low and it is not parked, probes
/// ahead along its up axis; if no walkable ground is found it stops the hero (0x206e90) unless the pitch is steep.
pub fn hero_edge_guard(c: &mut Cpu) -> bool {
    let vel = c.farg(0);
    let limit = c.farg(1);
    if h::run(c, hero_space::length_off_up, &[HERO], &[]).1 == F(0) {
        return true;
    }
    let low_floor = F(0x3f00_0000) < c.rf(GLOBALS + 0x229c);
    if low_floor
        && c.r32(GLOBALS + 0x208c) != 6
        && c.r16(GLOBALS + 0x1f4) == 0
        && c.r8(GLOBALS + 0x12e7) == 0
    {
        return true;
    }
    if c.r16(GLOBALS + 0x30e) != 0 {
        return true;
    }
    h::with_stack(c, 0x60, |c, sp| probe_ahead(c, sp, vel, limit));
    true
}

/// The probe part of the hero edge guard, working in the scratch frame at `sp`.
fn probe_ahead(c: &mut Cpu, sp: u32, vel: F, limit: F) -> bool {
    let speed = if c.r32(GLOBALS + 0x2084) == 0x20 { F(0x4000_0000) } else { vel };
    h::vec_scale(c, sp, HERO, speed);
    if F(0) < limit {
        let ahead = h::run(c, hero_space::length_off_up, &[sp], &[]).1;
        if ahead < limit {
            h::run(c, hero_space::set_length_xy_up, &[sp, sp], &[F(0)]);
            h::run(c, hero_space::point_ahead, &[sp, sp], &[limit]);
        }
    }
    h::run(c, hero_space::set_up_component, &[sp, sp], &[F(0)]);
    h::vec_add(c, sp, sp, GLOBALS + 0x80);
    let copy = sp + 0x10;
    for i in 0..4 {
        let w = c.r32(sp + 4 * i);
        c.w32(copy + 4 * i, w);
    }
    h::run(c, hero_space::point_up, &[sp, sp], &[F(0x3e99_999a)]);

    let mut lift = if c.r32(GLOBALS + 0x2084) == 0x20 { F(0xbeb3_3333) } else { F(0xbe4c_cccd) };
    if c.r8(GLOBALS + 0x20a4) == 2 {
        lift = F(0xbf33_3333);
    }
    h::run(c, hero_space::point_up, &[copy, copy], &[lift]);

    let actor = c.r32(GLOBALS + 0x2080);
    let v0 = h::call(c, 0x1e_24d8, &[sp, copy, 4, actor, 0], &[]).0;
    let mut keep = F(0x3f80_0000);
    if v0 != 0 {
        let frame = 0x17_3f40;
        if (c.r32(frame + 0x1c) as i32) > 0 {
            let pitch = h::run(c, hero_space::pitch_of, &[frame + 0x40], &[]).1;
            if pitch <= F(0x3f5f_66f3) {
                keep = F(0);
            }
        }
    }
    if keep != F(0) {
        h::call(c, HERO_STOP, &[], &[]);
    }
    true
}

/// 0x20a798 comet_strike_release() -> (): in hero states 1 or 0x1e with enough charge, aims the wrench from the
/// target offset (yaw/pitch) and launches it, then starts the wrench afterimage trail.
pub fn comet_strike_release(c: &mut Cpu) -> bool {
    h::with_stack(c, 0x130, |c, sp| release(c, sp))
}

fn release(c: &mut Cpu, sp: u32) -> bool {
    let state = c.r32(GLOBALS + 0x2084);
    let mut aim = false;
    if state == 1 || state == 0x1e {
        let scaled = h::run(c, mathx::float_to_int_scaled, &[0x14], &[]).0;
        aim = (scaled as i32) < (c.r32(GLOBALS + 0x198) as i32);
    }
    if aim {
        let target = c.r32(GLOBALS + 0x1090);
        let mut hit = false;
        if target != 0 && h::call(c, TARGET_CHECK, &[target], &[]).0 != 0 {
            let track = c.r32(target + 0x78);
            if c.r16(track + 0x7e) != 0 {
                hit = true;
                h::run(c, hero_space::point_from_offset, &[sp], &[F(0x3f00_0000), F(0), F(0x3f00_0000)]);
                h::vec_sub(c, sp + 0x10, track + 0x50, sp);
                let (dx, dy, dz) = (c.rf(sp + 0x10), c.rf(sp + 0x14), c.rf(sp + 0x18));
                let yaw = h::run(c, trig::fast_atan2, &[], &[dx, dy]).1;
                c.wf(GLOBALS + 0xa74, yaw);
                let len = h::run(c, vecmath::length_xy, &[sp + 0x10], &[]).1;
                let pitch = h::run(c, trig::fast_atan2, &[], &[len, dz]).1;
                c.wf(GLOBALS + 0xa78, pitch);
            }
        }
        if !hit {
            let base_yaw = c.rf(ANGLE_CONSTS + 0x158);
            c.wf(GLOBALS + 0xa74, base_yaw);
            let neg = -c.rf(ANGLE_CONSTS + 0x154);
            let r = h::run(c, vecmath::angle_add, &[], &[neg, F(0x3dfa_35dd)]).1;
            c.wf(GLOBALS + 0xa78, r);
        }
    }

    let obj = c.r32(GLOBALS + 0x1090);
    if h::call(c, TARGET_CHECK, &[obj], &[]).0 == 0 {
        return true;
    }
    c.w8(GLOBALS + 0x10aa, 1);
    if aim {
        h::call(c, SLOT_FIRE, &[0x1b, 0], &[]);
    }
    c.w8(obj + 0x20, 10);
    let track = c.r32(obj + 0x78);
    // v0 as left by retail at 0x20a944: 0xa, or inside the flag branch 1 / the flag byte. Retail reads the
    // aim angles through v0 (not through the global block), so the same small address is used here.
    let mut v0 = 0xau32;
    if c.rf(GLOBALS + 0x248) < F(0x3f99_999a) && F(0x3f29_c91f) < c.rf(GLOBALS + 0x250) {
        let flag = c.r8(GLOBALS + 0x254) as u32;
        if flag == 0 {
            c.w16(track + 0x76, 1);
            v0 = 1;
        } else {
            v0 = flag;
        }
    }
    let slot = track + 0x40;
    h::call(c, VEC_INIT, &[slot], &[]);
    if aim {
        let yaw = c.rf(v0 + 0xa74);
        let pitch = c.rf(v0 + 0xa78);
        h::run(c, scene::spherical_to_xyz, &[slot], &[F(0x3f80_0000), yaw, pitch]);
    } else {
        h::run(c, hero_space::point_ahead, &[slot, slot], &[F(0x3f80_0000)]);
    }
    h::call(c, VEC_INIT, &[sp], &[]);
    let frame = sp + 0x20;
    h::call(c, FRAME_FROM_VEC, &[frame, sp], &[]);
    if aim {
        for i in 0..3 {
            let k = c.rf(ANGLE_CONSTS + 0x150 + 4 * i);
            c.wf(sp + 4 * i, k);
        }
        h::call(c, FRAME_FROM_VEC, &[sp + 0x50, sp], &[]);
        h::run(c, vecmath::mat3_mul, &[frame, sp + 0x50, frame], &[]);
    } else {
        let actor = c.r32(GLOBALS + 0x2080);
        h::run(c, vecmath::mat3_mul, &[frame, actor + 0xc0, frame], &[]);
    }
    let m = sp + 0x80;
    h::run(c, vecmath::mat3_copy_w1, &[m, frame], &[]);
    h::run(c, geom::mat4_to_euler, &[m, obj + 0x40], &[]);

    c.w32(track + 0x64, 0);
    let decay = c.rf(0x15_ed6c) * F(0x41b8_0000);
    c.wf(track + 0x60, decay);
    h::run(c, trail::trail_start, &[obj, TRAIL], &[]);
    h::run(c, trail::trail_add_ghost, &[TRAIL, 0x30, 3], &[]);
    h::run(c, trail::trail_add_ghost, &[TRAIL, 0x17, 5], &[]);
    let scaled = h::run(c, mathx::float_to_int_scaled, &[5], &[]).0;
    h::run(c, objects::anim_start, &[obj, 6, 0, scaled], &[]);
    true
}
