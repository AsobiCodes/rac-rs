//! Spark particle spawn and per-tick drift (the emitter records shared by the Veldin and Novalis overlays).
use crate::h;
use crate::ps2::*;
use crate::slot_bits;
use crate::talk_pad_scan;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Particle pool mode that spark records are allocated from.
const SPARK_MODE: u32 = 0x2f;
/// 2.0f: the argument float_to_int gets for the spark's phase byte.
const TWO: F = F(0x4000_0000);
/// Colour word (low bytes 0x80, 0x80, 0x80) OR-ed under the random life byte.
const TINT: u32 = 0x0080_8080;
/// Address of the pointer (0x1b0000 page, offset 0x223c) to the byte copied into the spark's kind field.
const KIND_PTR: u32 = 0x001b_223c;
/// Type byte stored at +3 of a spark.
const SPARK_TYPE: u8 = 0x48;
/// 0.95f: damping applied to the spark's velocity each tick.
const DAMP: F = F(0x3f79_999a);
/// Float added to the spark's height (+0xc) each tick (a gravity-like step).
const GRAVITY: F = F(0x4665_b000);
/// Added to the life word (+4) each tick: 0xfe000000 is -0x2000000.
const LIFE_STEP: u32 = 0xfe00_0000;
/// Retail page the kind pointer lives in (lui 0x1b).
const KIND_PAGE: u32 = 0x001b_0000;
/// Retail rand_below(n): called as retail so the scratch registers it leaves match.
const RAND_BELOW: u32 = 0x0024_a418;
/// Retail float_to_int(x): it leaves its integer result bits in f12, so it is called as retail.
const FLOAT_TO_INT: u32 = 0x001f_27e8;

/// Fills a freshly allocated spark record: copies the position quad at `pos` to +0x10, the direction quad at `dir`
/// to +0x20, and sets the life word (+4), type, phase (+9), kind (+2), height (+0xc) and kind byte (+8).
fn fill_spark(c: &mut Cpu, rec: u32, pos: u32, dir: u32, scale: F) {
    let q = c.mem.rd128(pos);
    c.mem.wr128(rec + 0x10, q);
    c.r[2] = q;
    let life = h::call_keep(c, RAND_BELOW, &[8], &[]).0 as u32;
    let word = (life.wrapping_add(0x28) << 24) | TINT;
    c.s(2, word as i32 as i64 as u64);
    c.s(3, TINT as u64);
    c.w32(rec + 4, word);
    c.s(1, TWO.0 as u64);
    let phase = h::call_keep(c, FLOAT_TO_INT, &[], &[TWO]).0 as u32;
    c.w8(rec + 9, phase.wrapping_add(0x20) as u8);
    c.w8(rec + 3, SPARK_TYPE);
    c.s(4, KIND_PAGE as u64);
    let dst = rec + 0x20;
    c.s(5, dst as u64);
    c.w8(rec + 1, 0);
    let kind_ptr = c.r32(KIND_PTR);
    c.s(2, kind_ptr as i32 as i64 as u64);
    let kind = c.r8(kind_ptr);
    c.s(3, kind as u64);
    c.w32(rec + 0xc, scale.0);
    c.w8(rec + 2, kind);
    let dq = c.mem.rd128(dir);
    c.mem.wr128(dst, dq);
    c.r[2] = dq;
    let roll = h::call_keep(c, RAND_BELOW, &[0x100], &[]).0 as u8;
    c.w8(rec + 8, roll);
}

/// 0x263ef0 spark_spawn(pos, dir, scale = f12) -> v0: allocates a spark record (pool mode 0x2f), fills it from the
/// quads at *pos and *dir with the given height scale, and returns it (0 when the pool is full).
pub fn spark_spawn(c: &mut Cpu) -> bool {
    let pos = c.arg(0);
    let dir = c.arg(1);
    let scale = c.farg(0);
    let saves = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (18, 0x20, h::SQ), (31, 0x30, h::SQ)];
    let f20 = c.f[20];
    h::frame(c, 0x50, &saves, |c, sp| {
        c.w32(sp + 0x40, f20);
        c.f[20] = scale.0;
        let rec = h::run_keep(c, talk_pad_scan::particle_alloc_mode0, &[SPARK_MODE], &[]).0 as u32;
        if rec != 0 {
            fill_spark(c, rec, pos, dir, scale);
        }
        c.s(2, rec as u64);
        c.f[20] = f20;
        true
    })
}

/// 0x263fc0 spark_tick(spark) -> none: damps the velocity at +0x20 by 0.95, moves the position at +0x10 by it, adds
/// the gravity step to the height at +0xc, counts the life word at +4 down and releases the spark once it is negative.
pub fn spark_tick(c: &mut Cpu) -> bool {
    let s = c.arg(0);
    let vel = s + 0x20;
    let pos = s + 0x10;
    let saves = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (31, 0x20, h::SQ)];
    h::frame(c, 0x30, &saves, |c, _sp| {
        c.s(1, DAMP.0 as u64);
        c.f[12] = DAMP.0;
        h::run_keep(c, vecmath::vec_scale, &[vel, vel], &[DAMP]);
        h::call_keep(c, 0x001f_1810, &[pos, pos, vel], &[]);
        c.s(1, GRAVITY.0 as u64);
        let height = c.rf(s + 0xc) + GRAVITY;
        c.f[0] = height.0;
        c.f[1] = GRAVITY.0;
        c.wf(s + 0xc, height);
        let life = c.r32(s + 4).wrapping_add(LIFE_STEP);
        c.s(2, life as i32 as i64 as u64);
        let kind = c.r8(s + 8);
        let kind_b = c.r8(s + 0x30);
        c.s(4, kind_b as u64);
        let sum = kind as u32 + kind_b as u32;
        c.s(3, sum as u64);
        c.w32(s + 4, life);
        c.w8(s + 8, sum as u8);
        if (life as i32) < 0 {
            c.s(4, s as u64);
            h::run_keep(c, slot_bits::slot_release, &[s], &[]);
        }
        true
    })
}
