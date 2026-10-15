//! Spawns a type-0x13 streak particle between two points: a random tint word, a tail byte, the flag byte from a
//! global pointer, a copy of both points, and a width/half-fade pair. Only the particle record is written.
use crate::h;
use crate::mathx;
use crate::ps2::*;
use crate::rand;
use ee_oracle::cpu::Cpu;

/// Particle type code of a streak.
const STREAK_TYPE: u32 = 0x13;
/// Retail `rand_below` range for the tint word (also the multiplier applied to the random value).
const TINT_RANGE: u32 = 6;
/// Bias added to the scaled random value before the tag bits are or-ed in.
const TINT_BIAS: u32 = 0x40;
/// Tag bits or-ed into the tint word (lui 0x7840 | ori 0x4000).
const TINT_TAG: u32 = 0x7840_4000;
/// Value converted by `float_to_int` (2.0); its result plus `TAIL_BIAS` is the tail byte.
const TAIL_FLOAT: F = F(0x4000_0000);
const TAIL_BIAS: u32 = 0x20;
/// Byte stored at record offset 3 (the particle's kind).
const KIND_BYTE: u32 = 0x48;
/// Byte stored at record offset 1.
const SUB_KIND: u8 = 3;
/// Address of the word whose target byte is copied into record offset 2 (lui 0x1b, lw 0x21cc).
const FLAG_PTR_ADDR: u32 = 0x1b_21cc;
/// Width written at record offset 0x1c and the half-fade written at record offset 0x2c.
const WIDTH: F = F(0x3db8_51ec);
const HALF: F = F(0x3f00_0000);
/// Record offsets.
const OFF_TAIL: u32 = 0x9;
const OFF_KIND: u32 = 0x3;
const OFF_SUB: u32 = 0x1;
const OFF_FLAG: u32 = 0x2;
const OFF_TINT: u32 = 0xc;
const OFF_TINT_COPY: u32 = 0x4;
const OFF_FROM: u32 = 0x10;
const OFF_DELTA: u32 = 0x20;
const OFF_TO: u32 = 0x30;
const OFF_WIDTH: u32 = 0x1c;
const OFF_FADE: u32 = 0x2c;
/// Retail callees: particle allocator and vec_sub(out, a, b).
const ALLOC_PARTICLE: u32 = 0x259a38;
const VEC_SUB: u32 = 0x1f1840;

/// Low 64 bits of a 128-bit register (the part `lui` and `lbu` write).
const LOW64: u128 = u64::MAX as u128;

/// Copies one 16-byte vector (lq/sq pair) and returns it (the value retail leaves in v0).
fn copy_vec(c: &mut Cpu, dst: u32, src: u32) -> u128 {
    let v = c.mem.rd128(src);
    c.mem.wr128(dst, v);
    v
}

/// 0x25e988 spawn_streak(from, to) -> v0: allocates a streak record (v0 = 0 when the allocator has none) and
/// fills it: tint word, tail byte, kind bytes, copies of `from` and `to`, delta = from - 2 * to, width and fade.
pub fn spawn_streak(c: &mut Cpu) -> bool {
    let from = c.arg(0);
    let to = c.arg(1);
    let saves = [
        (16, 0x00, h::SQ),
        (17, 0x10, h::SQ),
        (18, 0x20, h::SQ),
        (19, 0x30, h::SQ),
        (31, 0x40, h::SQ),
    ];
    h::frame(c, 0x50, &saves, |c, _sp| {
        let rec = h::call_keep(c, ALLOC_PARTICLE, &[STREAK_TYPE], &[]).0;
        if rec == 0 {
            return true;
        }
        let rnd = h::run_keep(c, rand::rand_below, &[TINT_RANGE], &[]).0;
        let tint = rnd.wrapping_mul(TINT_RANGE).wrapping_add(TINT_BIAS) | TINT_TAG;
        c.w32(rec + OFF_TINT, tint);
        let tail = h::run_keep(c, mathx::float_to_int, &[TINT_RANGE], &[TAIL_FLOAT]).0;
        c.f[12] = tail;
        c.w32(rec + OFF_TINT_COPY, tint);
        c.w8(rec + OFF_TAIL, tail.wrapping_add(TAIL_BIAS) as u8);
        c.w8(rec + OFF_KIND, KIND_BYTE as u8);
        c.w8(rec + OFF_SUB, SUB_KIND);
        let flag_ptr = c.r32(FLAG_PTR_ADDR);
        let flag = c.r8(flag_ptr);
        c.w8(rec + OFF_FLAG, flag);
        copy_vec(c, rec + OFF_FROM, from);
        let to_quad = copy_vec(c, rec + OFF_TO, to);
        let delta = rec + OFF_DELTA;
        h::call_keep(c, VEC_SUB, &[delta, from, rec + OFF_TO], &[]);
        h::call_keep(c, VEC_SUB, &[delta, delta, rec + OFF_TO], &[]);
        c.wf(rec + OFF_WIDTH, WIDTH);
        c.wf(rec + OFF_FADE, HALF);
        c.f[0] = WIDTH.0;
        c.f[1] = HALF.0;
        // Scratch registers as retail leaves them: v0 = last quadword loaded, v1 = the flag byte, at = 0.5 bits.
        c.r[2] = to_quad;
        c.r[3] = (c.r[3] & !LOW64) | flag as u128;
        c.r[1] = (c.r[1] & !LOW64) | HALF.0 as u128;
        true
    })
}
