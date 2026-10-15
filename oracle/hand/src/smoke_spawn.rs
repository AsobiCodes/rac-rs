//! Type-4 smoke particle spawn, shared by the Novalis, Aridia and Kerwan effects.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const ALLOC_PARTICLE: u32 = 0x259a38;
const FLOAT_TO_INT: u32 = 0x1f27e8;
const RAND_DRAW: u32 = 0x1160d8;
/// Global pointer to the generator table; the byte it points at seeds the spawn.
const RAND_TABLE_PTR: u32 = 0x1b_2190;
/// 4.0f, converted to int (4) and biased by -0x60 for the byte at +9.
const FOUR_BITS: u32 = 0x4080_0000;
const BIAS: i32 = 0x60;
/// Kind byte at +3: 0x48 when the flag argument is set, 0x44 otherwise.
const KIND_FLAGGED: u8 = 0x48;
const KIND_PLAIN: u8 = 0x44;

// Particle record offsets (the allocated block `p`).
const OFF_SEED: u32 = 2;
const OFF_KIND: u32 = 3;
const OFF_COUNT: u32 = 4;
const OFF_ROLL: u32 = 8;
const OFF_BIAS: u32 = 9;
const OFF_SPEED: u32 = 0xa;
const OFF_VEC_A: u32 = 0x10;
const OFF_VEC_B: u32 = 0x20;
// Second vector block, relative to OFF_VEC_B.
const OFF_B_COUNT: u32 = 0x10;
const OFF_B_ARG: u32 = 0x14;
const OFF_B_LIFE: u32 = 0x18;
const OFF_B_RATE: u32 = 0x1a;
const OFF_B_SPEED: u32 = 0x1c;

/// 0x25bad8 part_type04_spawn(pos_a, pos_b, count, arg, speed, rate, life, flag) -> arg (or 0 if no record).
/// Allocates one type-4 particle, copies both position vectors into it and fills its fields; returns `arg`.
pub fn part_type04_spawn(c: &mut Cpu) -> bool {
    let pos_a = c.arg(0);
    let pos_b = c.arg(1);
    let count = c.arg(2);
    let arg = c.arg(3);
    let speed = c.arg(4);
    let rate = c.arg(5);
    let life = c.arg(6);
    let flag = c.arg(7);

    let p = h::call_keep(c, ALLOC_PARTICLE, &[4], &[]).0;
    if p == 0 {
        c.ret(0);
        return true;
    }

    let vec_b = p + OFF_VEC_B;
    let va = c.rv(pos_a);
    c.wv(p + OFF_VEC_A, va);
    c.w32(p + OFF_COUNT, count);

    // Bias byte: float 4.0 -> int 4, then minus 0x60. Retail's `lui $at` sets at before the call.
    c.r[1] = FOUR_BITS as u128;
    let four = h::call_keep(c, FLOAT_TO_INT, &[], &[F(FOUR_BITS)]).0 as i32;
    c.w8(p + OFF_BIAS, (four - BIAS) as u8);

    let kind = if flag != 0 { KIND_FLAGGED } else { KIND_PLAIN };
    c.w8(p + OFF_KIND, kind);
    c.w8(p + 1, 0);

    let table = c.r32(RAND_TABLE_PTR);
    let seed = c.r8(table);
    c.w8(p + OFF_SEED, seed);
    let roll = h::call_keep(c, RAND_DRAW, &[seed as u32], &[]).0;
    c.w8(p + OFF_ROLL, roll as u8);
    c.w32(p + 0xc, 0);

    let va = c.rv(pos_a);
    c.wv(p + OFF_VEC_A, va);
    let vb = c.rv(pos_b);
    c.wv(vec_b, vb);
    c.w16(p + OFF_SPEED, speed as u16);
    c.w16(vec_b + OFF_B_RATE, rate as u16);
    c.w32(vec_b + OFF_B_SPEED, speed);
    c.w32(vec_b + OFF_B_COUNT, count);
    c.w16(vec_b + OFF_B_LIFE, life as u16);
    c.w32(vec_b + OFF_B_ARG, arg);

    c.r[3] = vec_b as u128; // retail leaves the record's second block in v1
    c.ret(arg);
    true
}
