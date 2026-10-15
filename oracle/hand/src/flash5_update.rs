//! Pool type-5 flash update: countdown life, size growth and alpha fade below the scale threshold.

use crate::h;
use crate::mathx;
use crate::ps2::*;
use crate::slot_bits;
use ee_oracle::cpu::Cpu;

/// Integer argument passed to `float_to_int_scaled` (the scale threshold).
const SCALE_ARG: u32 = 6;
/// Life counter (tick_i32 target) inside the pool entry.
const OFF_LIFE: u32 = 0x20;
/// Packed word: low 24 bits are kept, the top byte holds the alpha fade.
const OFF_WORD: u32 = 0x04;
/// Current size (float), grown by the rate each frame.
const OFF_SIZE: u32 = 0x0c;
/// Size growth per frame (float), stored right after the life counter.
const OFF_RATE: u32 = 0x24;
const WORD_KEEP_MASK: u32 = 0x00ff_ffff;
/// Life multiplier of the alpha fade: life * 127 / scaled.
const FADE_MUL: u32 = 127;

const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const V1: usize = 3;

/// Sign-extended 32-bit register write, as the MIPS 32-bit ops leave it.
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// R5900 signed `div`: divide by zero gives 1 or -1 (by the sign of the dividend), MIN / -1 gives MIN.
fn r5900_div(n: i32, d: i32) -> i32 {
    if d == 0 {
        if n < 0 { 1 } else { -1 }
    } else if n == i32::MIN && d == -1 {
        i32::MIN
    } else {
        n / d
    }
}

/// 0x25bdf0 flash5_update(p): ticks the life counter; when it ends, releases the slot; otherwise grows the size
/// by the rate and rewrites the top byte of the word from life * 127 / scaled(size).
pub fn flash5_update(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    // Pure query (no memory writes). Retail divides by this value; a zero one traps there, so retail runs.
    let (scale, _) = h::run(c, mathx::float_to_int_scaled, &[SCALE_ARG], &[]);
    if scale == 0 {
        return false;
    }
    let p = c.arg(0);
    let life_ptr = p.wrapping_add(OFF_LIFE);
    let (expired, _) = h::run_keep(c, mathx::tick_i32, &[life_ptr], &[]);
    if expired != 0 {
        h::run_keep(c, slot_bits::slot_release, &[p], &[]);
        return true;
    }
    // retail: lwc1 $f0 = rate in the delay slot of the beql (taken when tick returned 0); add.s f1 = size + rate
    let rate = c.rf(p.wrapping_add(OFF_RATE));
    c.f[0] = rate.0;
    let size_ptr = p.wrapping_add(OFF_SIZE);
    let size = c.rf(size_ptr) + rate;
    c.wf(size_ptr, size);
    let (scaled, _) = h::run_keep(c, mathx::float_to_int_scaled, &[SCALE_ARG], &[]);
    let life = c.r32(life_ptr) as i32;
    if life < scaled as i32 {
        let (divisor, _) = h::run_keep(c, mathx::float_to_int_scaled, &[SCALE_ARG], &[]);
        let life_u = c.r32(life_ptr);
        let num = life_u.wrapping_mul(FADE_MUL) as i32;
        let fade = r5900_div(num, divisor as i32) as u32;
        let word_ptr = p.wrapping_add(OFF_WORD);
        let word = (c.r32(word_ptr) & WORD_KEEP_MASK) | fade.wrapping_shl(24);
        c.w32(word_ptr, word);
        set32(c, A2, life_u);
        set32(c, A1, word);
        set32(c, V1, fade.wrapping_shl(24));
        set32(c, A0, WORD_KEEP_MASK);
    }
    true
}
