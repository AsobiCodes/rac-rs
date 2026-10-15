//! Hand-rewrite of retail 0x21ca50: a heading-cone gate, then a segment probe (0x1e24d8) from the object block.

use crate::ps2::*;
use crate::h;
use ee_oracle::cpu::Cpu;

/// Object block the probe reads (retail s0 = 0x13f350).
const BLOCK: u32 = 0x13_f350;
/// Segment probe (retail 0x1e24d8).
const SEGMENT_PROBE: u32 = 0x1e_24d8;
/// ROM float the block's z is compared against (retail 0x173f68).
const LIMIT_ROM: u32 = 0x17_3f68;
/// Offsets inside the object block.
const OFF_LIMIT: u32 = 0x164;
const OFF_SCALE: u32 = 0x234;
const OFF_ATAN_X: u32 = 0x100;
const OFF_ATAN_Y: u32 = 0x104;
const OFF_HEADING: u32 = 0x98;
const OFF_POS: u32 = 0x80;
const OFF_Z: u32 = 0x88;
const OFF_PROBE_ARG: u32 = 0x2080;

/// Retail frame: 0x50 bytes, saves f20 at 0x40.
const FRAME: u32 = 0x50;
/// Half-scale factor applied to the sin/cos offsets (0.75).
const THREE_QUARTERS: F = F(0x3f40_0000);
/// Pi/6: the largest angle distance that passes the gate.
const MAX_ANGLE: F = F(0x3f06_0a92);
/// z offset added to the upper probe point.
const Z_UP: F = F(0x3ef0_a3d7);
/// Band the probe-z difference must lie in: -1.0 < d < 0.3699.
const BAND_HI: F = F(0x3ebd_70a4);
const BAND_LO: F = F(0xbf80_0000);

/// 0x21ca50 cone_probe_hit() -> v0: 1 when the block's heading passes the cone gate and the segment probe hits
/// with its z difference inside the band, else 0.
pub fn cone_probe_hit(c: &mut Cpu) -> bool {
    let v = h::frame(c, FRAME, &[(16, 0x20, h::SQ), (31, 0x30, h::SQ)], |c, sp| {
        c.w32(sp + 0x40, c.f[20]);
        probe(c, sp)
    });
    c.ret(v);
    true
}

/// Sets float register `n` (f0..f31) to `x`, as retail's arithmetic leaves it.
fn set_f(c: &mut Cpu, n: usize, x: F) {
    c.f[n] = x.0;
}

/// Sets the FPU condition flag (fcr31 bit 23) as the last c.lt.s/c.le.s of retail leaves it.
fn set_cc(c: &mut Cpu, flag: bool) {
    c.fcr31 = (c.fcr31 & !(1 << 23)) | ((flag as u32) << 23);
}

/// Calls retail code at `addr` and leaves every register as the callee left it (a plain `jal`).
fn raw(c: &mut Cpu, addr: u32, ints: &[u32], floats: &[F]) -> (u32, F) {
    for (i, &a) in ints.iter().enumerate() {
        c.set_arg(i, a);
    }
    for (i, &x) in floats.iter().enumerate() {
        c.set_farg(i, x);
    }
    c.call(addr);
    (c.result(), c.fresult())
}

fn probe(c: &mut Cpu, sp: u32) -> u32 {
    // Gate 1: the block's limit must not exceed scale / float(20).
    let n = raw(c, 0x1f14b8, &[20], &[]).0 as i32;
    let from_n = F::from_int(n);
    set_f(c, 2, from_n);
    let ratio = c.rf(BLOCK + OFF_SCALE) / from_n;
    let limit = c.rf(BLOCK + OFF_LIMIT);
    set_f(c, 1, limit);
    set_f(c, 0, ratio);
    set_cc(c, limit < ratio);
    if limit < ratio {
        return 0;
    }
    // Gate 2: the angle distance between atan2(x, y) and the heading must be at most pi/6.
    let heading = c.rf(BLOCK + OFF_HEADING);
    let (ax, ay) = (c.rf(BLOCK + OFF_ATAN_X), c.rf(BLOCK + OFF_ATAN_Y));
    let angle = raw(c, 0x1f1e48, &[], &[ax, ay]).1;
    let dist = raw(c, 0x1f2788, &[], &[angle, heading]).1;
    set_f(c, 1, MAX_ANGLE);
    set_cc(c, MAX_ANGLE < dist);
    if MAX_ANGLE < dist {
        return 0;
    }
    // Two points in the frame: x/y advanced along the heading, z from the block (up and down by a unit).
    let scale = c.rf(BLOCK + OFF_SCALE);
    for i in 0..4 {
        let w = c.r32(BLOCK + OFF_POS + 4 * i);
        c.w32(sp + 4 * i, w);
    }
    let cos = raw(c, 0x1f1d80, &[], &[heading]).1;
    let cos_scaled = cos * scale * THREE_QUARTERS;
    let x = c.rf(sp) + cos_scaled;
    c.wf(sp, x);
    set_f(c, 2, scale);
    set_f(c, 1, x);
    set_f(c, 0, cos_scaled);
    let sin = raw(c, 0x1f1d98, &[], &[heading]).1;
    let sin_scaled = sin * scale * THREE_QUARTERS;
    let y = c.rf(sp + 4) + sin_scaled;
    c.wf(sp + 4, y);
    set_f(c, 2, scale);
    set_f(c, 1, y);
    set_f(c, 0, sin_scaled);
    for i in 0..4 {
        let w = c.r32(sp + 4 * i);
        c.w32(sp + 0x10 + 4 * i, w);
    }
    let z = c.rf(BLOCK + OFF_Z);
    c.wf(sp + 8, z + Z_UP);
    c.wf(sp + 0x18, z - F::ONE);
    set_f(c, 2, Z_UP);
    set_f(c, 3, F::ONE);
    set_f(c, 1, z + Z_UP);
    set_f(c, 0, z - F::ONE);
    // Segment probe between the two points.
    let probe_arg = c.r32(BLOCK + OFF_PROBE_ARG);
    if raw(c, SEGMENT_PROBE, &[sp, sp + 0x10, 2, probe_arg, 0], &[]).0 == 0 {
        return 0;
    }
    // Hit: the z difference against the ROM limit must lie in the band.
    let rom = c.rf(LIMIT_ROM);
    let diff = rom - z;
    set_f(c, 2, diff);
    set_f(c, 1, BAND_HI);
    set_cc(c, diff < BAND_HI);
    if diff < BAND_HI {
        set_cc(c, BAND_LO < diff);
    }
    if diff < BAND_HI && BAND_LO < diff {
        set_f(c, 0, BAND_LO);
        1
    } else {
        set_f(c, 0, rom);
        0
    }
}
