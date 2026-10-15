//! Hand-rewrite of retail 0x2e7530 (follow_ground_point): the camera's hero point, or, while the hero is airborne
//! (block float below 30.0), a ground point found by the segment probe at 0x1e24d8.

use crate::camera;
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Camera object block (retail s4 = 0x13f350).
const BLOCK: u32 = 0x13_f350;
/// Argument the hero-position copy is called with (retail lw 0x166f00); the copy ignores it.
const HERO_ARG: u32 = 0x16_6f00;
/// Offset vector the hero point is moved by (retail s1 + 0x30 = 0x166f40).
const OFFSET_VEC: u32 = 0x16_6f40;
/// Ground point the probe result is read from (retail 0x173f60).
const GROUND_POINT: u32 = 0x17_3f60;
/// Block offsets: the airborne height float, the follow flag (lh), and the probe's extra argument.
const HEIGHT: u32 = 0x2dc;
const FOLLOW_FLAG: u32 = 0x30c;
const PROBE_ARG: u32 = 0x2080;
/// Retail vector helpers: add (out, a, b) and the segment probe.
const VEC_ADD: u32 = 0x1f1810;
const SEGMENT_PROBE: u32 = 0x1e_24d8;
/// Probe mask passed in a2.
const PROBE_MASK: u32 = 0x12;
/// Height (30.0) at or above which the hero counts as grounded.
const GROUND_HEIGHT: F = F(0x41f0_0000);
/// Scales of the two ground offsets: +0.2 and -30.0.
const NEAR_SCALE: F = F(0x3e4c_cccd);
const FAR_SCALE: F = F(0xc1f0_0000);
/// Retail frame size; the saved registers sit at 0x40..0x90 and the scratch vectors at 0x00..0x30.
const FRAME: u32 = 0xa0;

/// Stores a 128-bit register as `sq` does: the low word at the lowest address.
fn save_quad(c: &mut Cpu, addr: u32, q: u128) {
    for i in 0..4 {
        c.w32(addr + 4 * i, (q >> (32 * i)) as u32);
    }
}

/// 0x2e7530 follow_ground_point(out, use_ground) -> 1/0/-1: out = hero point (follow flag clear), else the
/// hero point moved by the offset; a probe result (0) or the fallback point (-1) when the hero is airborne.
pub fn follow_ground_point(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let use_ground = c.arg(1);
    let sp = (c.g(29) as u32).wrapping_sub(FRAME);
    // Retail prologue: save ra and s0-s4 (never changed here) in the frame.
    save_quad(c, sp + 0x90, c.r[31]);
    save_quad(c, sp + 0x80, c.r[20]);
    save_quad(c, sp + 0x70, c.r[19]);
    save_quad(c, sp + 0x60, c.r[18]);
    save_quad(c, sp + 0x50, c.r[17]);
    save_quad(c, sp + 0x40, c.r[16]);

    if c.r16(BLOCK + FOLLOW_FLAG) == 0 {
        let hero_arg = c.r32(HERO_ARG);
        h::run(c, camera::follow_hero_pos, &[hero_arg, out], &[]);
        c.ret(1);
        return true;
    }

    // Hero point into the frame at sp+0x00.
    let hero_arg = c.r32(HERO_ARG);
    h::run(c, camera::follow_hero_pos, &[hero_arg, sp], &[]);
    let height = c.rf(BLOCK + HEIGHT);
    let airborne = !(GROUND_HEIGHT <= height);
    if airborne && use_ground != 0 {
        // Negated height: the offset is pushed the other way and the hero point is copied out.
        let s0 = sp + 0x10;
        h::run(c, vecmath::vec_scale, &[s0, OFFSET_VEC], &[-height]);
        h::call(c, VEC_ADD, &[out, sp, s0], &[]);
        c.ret(1);
        return true;
    }

    // Two ground candidates: sp+0x10 = hero + 0.2 * offset, sp+0x20 = hero - 30 * offset.
    let scratch = sp + 0x30;
    h::run(c, vecmath::vec_scale, &[scratch, OFFSET_VEC], &[NEAR_SCALE]);
    h::call(c, VEC_ADD, &[sp + 0x10, sp, scratch], &[]);
    h::run(c, vecmath::vec_scale, &[scratch, OFFSET_VEC], &[FAR_SCALE]);
    h::call(c, VEC_ADD, &[sp + 0x20, sp, scratch], &[]);

    let probe_arg = c.r32(BLOCK + PROBE_ARG);
    let hit = h::call(c, SEGMENT_PROBE, &[sp + 0x10, sp + 0x20, PROBE_MASK, probe_arg, 0], &[]).0;
    if hit != 0 {
        let ground = c.rv(GROUND_POINT);
        c.wv(out, ground);
        c.ret(0);
    } else {
        let hero = c.rv(sp);
        c.wv(out, hero);
        c.ret(u32::MAX);
    }
    true
}
