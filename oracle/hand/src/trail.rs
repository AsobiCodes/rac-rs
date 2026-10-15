//! Ghost trail records: a tracked moby's position/rotation history ring and the delayed copies that follow it
//! (0x254920..0x254d58).
use crate::actor;
use crate::h::*;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Trail record layout (all offsets from the record): +0x000 position ring (8 quadwords), +0x080 rotation ring
/// (8 quadwords), +0x100 per-ghost fade amount, +0x110 per-ghost delay (frames), +0x120 per-ghost moby, +0x130
/// ring cursor (i16), +0x132 history length (i16), +0x134 tracked moby, +0x138 ghost count, +0x13c active flag.
const RING_ROT: u32 = 0x80;
const FADE: u32 = 0x100;
const DELAY: u32 = 0x110;
const GHOSTS: u32 = 0x120;
const CURSOR: u32 = 0x130;
const LENGTH: u32 = 0x132;
const TRACKED: u32 = 0x134;
const COUNT: u32 = 0x138;
const ACTIVE: u32 = 0x13c;
const RING_LEN: i32 = 8;
const MAX_GHOSTS: u32 = 4;
const GHOST_FLAGS: u16 = 0x80a;

/// 0x254920 trail_start(target, trail): arms an idle trail on `target` with an empty history and no ghosts.
pub fn trail_start(c: &mut Cpu) -> bool {
    let (target, trail) = (c.arg(0), c.arg(1));
    if c.r32(trail + ACTIVE) == 0 {
        c.w32(trail + TRACKED, target);
        c.w32(trail + ACTIVE, 1);
        c.w16(trail + CURSOR, 0);
        c.w16(trail + LENGTH, 0);
        c.w32(trail + COUNT, 0);
    }
    true
}

/// 0x254948 trail_add_ghost(trail, fade, delay): adds a ghost moby (a clone of the tracked moby's class, flags
/// 0x80a) following the history `delay` frames behind; ignored when the trail holds 4 ghosts or delay >= 9.
pub fn trail_add_ghost(c: &mut Cpu) -> bool {
    let (trail, fade, delay) = (c.arg(0), c.arg(1), c.arg(2) as i32);
    let n = c.r32(trail + COUNT);
    if n == MAX_GHOSTS || delay >= 9 {
        return true;
    }
    let tracked = c.r32(trail + TRACKED);
    let class = c.r16(tracked + 0xa6);
    let ghost = run(c, actor::alloc_moby, &[class as i16 as u32], &[]).0;
    if ghost == 0 {
        return true;
    }
    let tracked = c.r32(trail + TRACKED);
    let v = c.r16(tracked + 0x32);
    c.w16(ghost + 0x32, v);
    let (lo, hi) = (c.r32(tracked + 0x38), c.r32(tracked + 0x3c));
    c.w8(ghost + 0x31, 1);
    c.w32(ghost + 0x38, lo);
    c.w32(ghost + 0x3c, hi);
    c.w16(ghost + 0x34, GHOST_FLAGS);
    c.w8(ghost + 0x30, 0);
    c.w32(ghost + 0x94, 0);
    if c.r16(ghost + 0xa6) == 0 {
        c.w8(ghost + 0x72, 0);
    }
    c.w8(ghost + 0x23, fade as u8);
    c.w32(trail + DELAY + 4 * n, delay as u32);
    c.w32(trail + FADE + 4 * n, fade as u32);
    c.w32(trail + GHOSTS + 4 * n, ghost);
    c.w32(trail + COUNT, n + 1);
    true
}

/// 0x254c60 trail_free(trail): releases every ghost moby and deactivates the trail.
pub fn trail_free(c: &mut Cpu) -> bool {
    let trail = c.arg(0);
    trail_free_body(c, trail);
    true
}

fn trail_free_body(c: &mut Cpu, trail: u32) {
    if c.r32(trail + ACTIVE) == 0 {
        return;
    }
    for i in 0..c.r32(trail + COUNT) as i32 {
        let ghost = c.r32(trail + GHOSTS + 4 * i as u32);
        if ghost != 0 {
            run(c, actor::free_moby, &[ghost], &[]);
            c.w32(trail + GHOSTS + 4 * i as u32, 0);
        }
    }
    c.w32(trail + ACTIVE, 0);
}

/// 0x254a28 trail_update(trail, fade_step): records the tracked moby's position and rotation in the ring; each
/// ghost whose delay is within the history copies the entry `delay` frames back (animation fields from the tracked
/// moby) and refreshes its bounding sphere. With fade_step != 0 the ghosts' fade bytes (+0x23) count down by it and
/// the trail is freed once all have reached 0. A missing or released tracked moby frees the trail.
pub fn trail_update(c: &mut Cpu) -> bool {
    let (trail, step) = (c.arg(0), c.arg(1) as i32);
    if c.r32(trail + ACTIVE) == 0 {
        return true;
    }
    let tracked = c.r32(trail + TRACKED);
    if tracked == 0 || matches!(c.r8(tracked + 0x20), 0xfe | 0xfd) {
        trail_free_body(c, trail);
        return true;
    }
    let cursor = c.r16(trail + CURSOR) as i16 as i32;
    copy_q(c, trail.wrapping_add((16 * cursor) as u32), tracked + 0x10);
    copy_q(c, trail.wrapping_add(RING_ROT).wrapping_add((16 * cursor) as u32), tracked + 0x40);
    let next = cursor + 1;
    c.w16(trail + CURSOR, (next % RING_LEN) as u16);
    let len = (c.r16(trail + LENGTH) as i16).wrapping_add(1);
    c.w16(trail + LENGTH, if len < RING_LEN as i16 + 1 { len as u16 } else { RING_LEN as u16 });
    let n = c.r32(trail + COUNT) as i32;
    for i in 0..n.max(0) as u32 {
        let delay = c.r32(trail + DELAY + 4 * i) as i32;
        if (c.r16(trail + LENGTH) as i16 as i32) < delay {
            continue;
        }
        let ghost = c.r32(trail + GHOSTS + 4 * i);
        let tracked = c.r32(trail + TRACKED);
        let (b52, b53) = (c.r8(tracked + 0x52), c.r8(tracked + 0x53));
        c.w8(ghost + 0x52, b52);
        c.w8(ghost + 0x53, b53);
        let back = c.r16(trail + CURSOR) as i16 as i32 - delay + RING_LEN;
        let (b50, b51) = (c.r8(tracked + 0x50), c.r8(tracked + 0x51));
        c.w8(ghost + 0x50, b50);
        c.w8(ghost + 0x51, b51);
        let blend = c.rf(tracked + 0x54);
        c.wf(ghost + 0x54, blend);
        call(c, 0x241140, &[ghost, back as u32], &[]);
        let slot = (back % RING_LEN) as u32;
        copy_q(c, ghost + 0x10, trail.wrapping_add(16u32.wrapping_mul(slot)));
        copy_q(c, ghost + 0x40, trail.wrapping_add(RING_ROT).wrapping_add(16u32.wrapping_mul(slot)));
        call(c, 0x2436c0, &[ghost], &[]);
    }
    if step == 0 {
        return true;
    }
    let mut all_gone = true;
    for i in 0..(c.r32(trail + COUNT) as i32).max(0) as u32 {
        let ghost = c.r32(trail + GHOSTS + 4 * i);
        if ghost != 0 {
            let fade = c.r8(ghost + 0x23) as i32;
            let left = fade - step.min(fade);
            c.w8(ghost + 0x23, left as u8);
            if left & 0xff != 0 {
                all_gone = false;
            }
        }
    }
    if all_gone {
        trail_free_body(c, trail);
    }
    true
}
