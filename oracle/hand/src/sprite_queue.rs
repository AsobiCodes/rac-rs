//! Sprite track update (retail 0x1e00e8) and screen-sprite queue flush (retail 0x1e08a8).
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Track record: +0 sprite pointer, +4 active flag (1 = run), +8 range, +0x10 halfword offsets, +0x30 halfwords.
const TRACK: u32 = 0x167140;
const TRACK_FLAG: u32 = TRACK + 4;
/// Vector the track distance is measured from.
const TRACK_ORIGIN: u32 = 0x166ec0;
/// Table of integer positions/sizes (converted to float before use).
const TBL: u32 = 0x13e500;
/// Hero state word; while it reads HERO_STATE_CLEAR the sprite queue is discarded.
const HERO_STATE: u32 = 0x1413d4;
const HERO_STATE_CLEAR: u32 = 0x72;
/// Screen-sprite queue: 0x30-byte items, item count at +0xc0.
const QUEUE: u32 = 0x169140;
const QUEUE_COUNT: u32 = QUEUE + 0xc0;
/// Per-item scale (1/16).
const F_SCALE: F = F(0x3d80_0000);
const F_FIVE: F = F(0x40a0_0000);
const F_NEG_FIVE: F = F(0xc0a0_0000);
const F_BASE: F = F(0x40b0_0000);

fn int_at(c: &mut Cpu, addr: u32) -> F {
    F::from_int(c.r32(addr) as i32)
}

fn clamp01(x: F) -> F {
    if F::ONE < x {
        F::ONE
    } else if x < F::ZERO {
        F::ZERO
    } else {
        x
    }
}

/// Stores a 64-bit doubleword (sd) at `addr`.
fn sd(c: &mut Cpu, addr: u32, v: i64) {
    c.w32(addr, v as u32);
    c.w32(addr + 4, (v >> 32) as u32);
}

/// 0x1e00e8 track_sprites(): when the track is active and in range, place its 16 sprite entries and queue them.
pub fn track_sprites(c: &mut Cpu) -> bool {
    if c.r32(TRACK_FLAG) != 1 {
        c.w32(TRACK_FLAG, 0);
        return true;
    }
    let ptr = c.r32(TRACK);
    if c.r8(ptr + 0x20) as u32 & 0x80 != 0 {
        c.w32(TRACK_FLAG, 0);
        return true;
    }
    if c.rf(TRACK + 8) <= F::ZERO {
        return true;
    }
    let dist = h::run(c, vecmath::dist3, &[TRACK_ORIGIN, ptr + 0x10], &[]).1;
    h::with_stack(c, 0x150, |c, sp| track_body(c, sp, ptr, dist));
    true
}

fn track_body(c: &mut Cpu, sp: u32, ptr: u32, dist: F) {
    let a = int_at(c, TBL + 8);
    c.wf(sp + 0x20, a);
    let b = int_at(c, TBL + 0xc);
    c.wf(sp + 0x24, b);
    h::call(c, 0x1e94a0, &[sp + 0x10, ptr + 0x10], &[]);
    let x0 = int_at(c, TBL + 0x10);
    let x = (c.rf(sp + 0x10) - x0) * F_SCALE;
    c.wf(sp + 0x10, x);
    let y0 = int_at(c, TBL + 0x14);
    let y = (c.rf(sp + 0x14) - y0) * F_SCALE;
    c.wf(sp + 0x14, y);
    c.wf(sp + 0x30, a - x);
    c.wf(sp + 0x34, b - y);

    for i in 0..16u32 {
        let s3: u32 = if i != 0 { 0x20 } else { 0x40 };
        let idx = c.r16(TRACK + 0x30 + 2 * i) as i16 as i32 + 6;
        let s7 = h::call(c, 0x1ebb00, &[idx as u32], &[]).0 as i32 as i64;
        let g = c.rf(TRACK + 0x90 + 4 * i);
        let f2 = (a - x) * g;
        let f3 = (b - y) * g;
        let u = a + f2;
        let v = b + f3;
        c.wf(sp + 0x40, f2);
        c.wf(sp + 0x44, f3);
        c.wf(sp + 0x50, u);
        c.wf(sp + 0x54, v);

        let f21 = clamp01(dist - F_FIVE);
        let f20 = clamp01((a - x).abs() / a * F_NEG_FIVE + F_BASE);
        let q = clamp01((b - y).abs() / b * F_NEG_FIVE + F_BASE);
        let f21 = f21 * (f20 * q);

        let h1 = c.r16(TRACK + 0x10 + 2 * i) as i16 as i32;
        let int1 = (f21 * F::from_int(h1)).to_int() as u32;
        let s3f = F::from_int(s3 as i32);
        let g2 = c.rf(TRACK + 0xd0 + 4 * i);
        let flags = c.r32(TRACK + 0x50 + 4 * i) & 0x00ff_ffff;
        let s0v = ((int1 << 24) | flags) as i32 as i64;
        c.w32(sp + 0x60, 4 * i + 4);
        let int2 = (s3f * g2).to_int() as u32;

        sd(c, sp, s0v);
        sd(c, sp + 8, s7);
        let fh = F::from_int((int2 as i32) >> 1);
        let int3 = (u - fh).to_int() as u32;
        let int4 = (v - fh).to_int() as u32;
        h::call(c, 0x1ecaf8, &[int3, int4, int2, int2, 0, 0, s3, s3], &[]);
    }
    c.w32(TRACK + 8, 0);
}

/// 0x1e08a8 draw_sprite_queue(): flush the screen-sprite queue (dropped while the hero is in state 0x72).
pub fn draw_sprite_queue(c: &mut Cpu) -> bool {
    if c.r32(HERO_STATE) == HERO_STATE_CLEAR {
        c.w32(QUEUE_COUNT, 0);
    }
    let count = c.r32(QUEUE_COUNT) as i32;
    if count == 0 {
        return true;
    }
    h::with_stack(c, 0xc0, |c, sp| queue_body(c, sp, count));
    true
}

fn queue_body(c: &mut Cpu, sp: u32, count: i32) {
    let a = int_at(c, TBL + 8);
    c.wf(sp, a);
    let b = int_at(c, TBL + 0xc);
    c.wf(sp + 4, b);
    if count > 0 {
        let mut item = QUEUE;
        loop {
            queue_item(c, sp, item);
            item += 0x30;
            if c.r32(QUEUE_COUNT) as i32 <= (item - QUEUE) as i32 / 0x30 {
                break;
            }
        }
    }
    c.w32(QUEUE_COUNT, 0);
}

fn queue_item(c: &mut Cpu, sp: u32, item: u32) {
    let ptr = c.r32(item + 0x20);
    if ptr == 0 {
        return;
    }
    let kind = c.r8(ptr + 0x20) as u32;
    if kind == 0xfe || kind == 0xfd {
        return;
    }
    if c.r16(item + 0x24) as i16 == 0 {
        let q = c.rv(sp);
        c.wv(sp + 0x10, q);
    } else {
        h::call(c, 0x1e94a0, &[sp + 0x10, item], &[]);
        let x0 = int_at(c, TBL + 0x10);
        let x = (c.rf(sp + 0x10) - x0) * F_SCALE;
        c.wf(sp + 0x10, x);
        let y0 = int_at(c, TBL + 0x14);
        let y = (c.rf(sp + 0x14) - y0) * F_SCALE;
        c.wf(sp + 0x14, y);
    }
    let x = c.rf(sp + 0x10);
    let y = c.rf(sp + 0x14);
    h::call(c, 0x1e04a0, &[item], &[x, y]);
}
