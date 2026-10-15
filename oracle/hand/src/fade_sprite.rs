//! Fading sprite particles (type 0x1b): the spawn routine and the per-frame update.

use crate::h;
use crate::mathx;
use crate::ps2::*;
use crate::slot_bits;
use crate::talk_pad_scan;
use ee_oracle::cpu::Cpu;

/// Particle type of the fading sprite.
const FADE_TYPE: u32 = 0x1b;
/// Retail helper called once after the position copy in the spawn (takes no arguments).
const SPAWN_HELPER: u32 = 0x0011_60d8;
/// Retail 16-byte vector add used by the update (`out = a + b`, here `pos += vel`).
const VEC3_ADD: u32 = 0x001f_1810;
/// Word holding a pointer to the sprite's colour byte (`lw $v0, 0x21ec($a1)` with `a1 = 0x1b0000`).
const COLOUR_PTR: u32 = 0x001b_21ec;
/// Retail float-to-int helper (`cvt.w.s` of its f12 argument); called as retail so its f12 leftover matches.
const FLOAT_TO_INT: u32 = 0x001f_27e8;
/// Float 4.0: the spawn scales its integer result by it.
const FOUR: F = F(0x4080_0000);
/// Register numbers the spawn and update leave behind (retail scratch registers).
const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
/// Offsets inside a sprite particle.
const OFF_TAG: u32 = 0x01;
const OFF_BLEND: u32 = 0x03;
const OFF_KIND: u32 = 0x02;
const OFF_FLAGS: u32 = 0x04;
const OFF_TIMER_SCALE: u32 = 0x08;
const OFF_SCALE: u32 = 0x0c;
const OFF_POS: u32 = 0x10;
const OFF_VEL: u32 = 0x20;
const OFF_TIMER: u32 = 0x30;
const OFF_DIV: u32 = 0x34;
const OFF_WORD: u32 = 0x38;

/// Writes the low 64 bits of GPR `r` (what retail's 32-bit ops set); the upper half stays as it was.
fn set_gpr(c: &mut Cpu, r: usize, v: u64) {
    c.r[r] = (c.r[r] & !(u64::MAX as u128)) | v as u128;
}

/// The 64-bit GPR value of a 32-bit result (sign-extended, as the MIPS 32-bit ops leave it).
fn sext(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// 0x25ffc0 spawn_fade27(pos, vel, word_a, word_b, scale) -> particle: allocates a type 0x1b fading sprite, copies pos
/// and vel into it, sets its scale, colour and fade fields; returns it (0 when no slot is free).
pub fn spawn_fade27(c: &mut Cpu) -> bool {
    let pos = c.arg(0);
    let vel = c.arg(1);
    let word_a = c.arg(2);
    let word_b = c.arg(3);
    let scale = c.farg(0);
    let (p, _) = h::run_keep(c, talk_pad_scan::particle_alloc_mode0, &[FADE_TYPE], &[]);
    if p == 0 {
        c.ret(0);
        return true;
    }
    let q = c.rv(pos);
    c.wv(p + OFF_POS, q);
    let (helper_v0, _) = h::call_keep(c, SPAWN_HELPER, &[], &[]);
    c.w32(p + OFF_FLAGS, word_a);
    // Retail's delay slot stores the helper's v0 here, before the float call below.
    c.w8(p + OFF_TIMER_SCALE, helper_v0 as u8);
    set_gpr(c, AT, 0x4080_0000);
    let (k, _) = h::call_keep(c, FLOAT_TO_INT, &[], &[FOUR]);
    c.w8(p + OFF_TIMER_SCALE + 1, k.wrapping_add(0x40) as u8);
    c.w8(p + OFF_BLEND, 0x48);
    c.w32(p + OFF_SCALE, scale.0);
    c.w8(p + OFF_TAG, 0);
    set_gpr(c, A1, 0x001b_0000);
    let tag = c.r8(p + 7) as i8 as i32 as u32;
    set_gpr(c, A0, sext(tag));
    let colour_addr = c.r32(COLOUR_PTR);
    set_gpr(c, V0, sext(colour_addr));
    let colour = c.r8(colour_addr);
    set_gpr(c, V1, colour as u64);
    c.w8(p + OFF_KIND, colour);
    c.w32(p + OFF_WORD, tag);
    let v = c.rv(vel);
    c.wv(p + OFF_VEL, v);
    c.w32(p + OFF_DIV, word_b);
    c.w32(p + OFF_TIMER, word_b);
    c.ret(p);
    true
}

/// 0x2600a0 fade_sprite_update(p): moves sprite p by its velocity, rescales its flags by the timer ratio, ticks its
/// timer, and releases the slot when the timer ends. Declines when the divisor is zero (retail traps there).
pub fn fade_sprite_update(c: &mut Cpu) -> bool {
    let p = c.arg(0);
    let divisor = c.r32(p + OFF_DIV);
    if divisor == 0 {
        return false;
    }
    h::call_keep(c, VEC3_ADD, &[p + OFF_POS, p + OFF_POS, p + OFF_VEL], &[]);
    let word = c.r32(p + OFF_TIMER);
    let product = c.r32(p + OFF_WORD).wrapping_mul(word);
    let quotient = (product as i32).wrapping_div(divisor as i32) as u32;
    let shifted = quotient << 24;
    let flags = (c.r32(p + OFF_FLAGS) & 0x00ff_ffff).wrapping_add(shifted);
    c.w32(p + OFF_FLAGS, flags);
    set_gpr(c, A1, sext(divisor));
    set_gpr(c, A2, 0x00ff_ffff);
    set_gpr(c, A3, sext(word));
    set_gpr(c, V0, sext(shifted));
    set_gpr(c, V1, sext(flags));
    let (ended, _) = h::run_keep(c, mathx::tick_i32, &[p + OFF_TIMER], &[]);
    if ended != 0 {
        h::run_keep(c, slot_bits::slot_release, &[p], &[]);
    }
    true
}
