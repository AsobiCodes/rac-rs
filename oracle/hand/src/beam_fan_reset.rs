//! Fan spawn loop of the weapon effects (0x2adea8) and the morph beam reset (0x2af980).

use crate::h;
use crate::mathx;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Upper half of the 0x1d0000 data page loaded by `lui` in the fan loop.
const DATA_HI: u32 = 0x1d_0000;
/// Upper half of the 0x160000 page holding the gameplay globals and the float constants.
const GLOBAL_HI: u32 = 0x16_0000;

/// Halfword flags read by the fan loop (0x1d0000 + 0x3c0): zero means no spawn for that slot.
const FAN_FLAG_TABLE: u32 = DATA_HI + 0x3c0;
/// Halfword scale inputs read by the fan loop (0x1d0000 + 0x3d8).
const FAN_SCALE_TABLE: u32 = DATA_HI + 0x3d8;
/// Spawn record base the fan loop writes through (0x1d0000 + 0xa0), 0x50 bytes per slot.
const FAN_SPAWN_BASE: u32 = DATA_HI + 0xa0;
const FAN_SPAWN_STRIDE: u32 = 0x50;
/// Slots per fan (the loop counter starts at 9 and runs down to 0).
const FAN_SLOTS: usize = 10;
/// Float constant the scale input is multiplied by (0x160000 + 0x1624).
const FAN_SCALE_CONST: u32 = GLOBAL_HI + 0x1624;
/// gp-relative float pair passed to the spawn routine in f12 and f13.
const FAN_GP_F12: i32 = -0x5620;
const FAN_GP_F13: i32 = -0x561c;
/// 0.2f: the spread passed in f14.
const FAN_SPREAD: F = F(0x3e4c_cccd);
/// Spawn routine (the function right after the fan loop) and the tail call that follows the loop.
const FAN_SPAWN_FN: u32 = 0x2adf70;
const FAN_TAIL_FN: u32 = 0x2ae548;

/// Geometry latch word (0x160000 + 0x170c): zero means no morph beam to reset.
const BEAM_LATCH: u32 = GLOBAL_HI + 0x170c;
/// gp-relative word cleared before the beam setup, and float inputs for the scale (f1) and its store.
const BEAM_GP_CLEAR: i32 = -0x54f4;
const BEAM_GP_SPEED: i32 = -0x55d4;
const BEAM_GP_SCALE: i32 = -0x54e4;
/// Beam record offset of the source vector pointer, and the point-table offset passed to vec_scale.
const BEAM_SRC_PTR: u32 = 0x78;
const BEAM_VEC_OFF: u32 = 0xd0;
/// Point table the beam resets (0x1d0460 for the first quad, then 11 points at 0x10 spacing).
const BEAM_PATH_A: u32 = DATA_HI + 0x460;
const BEAM_PATH_B: u32 = DATA_HI + 0x450;
const BEAM_POINT_COUNT: u32 = 11;
const BEAM_PATH_STEP: u32 = 0x10;
/// Point-spacing helper (0x1f1810) and the vec_scale input that retail passes through the stack.
const BEAM_POINT_FN: u32 = 0x1f_1810;
/// Cleared globals and the three-quad copy target after the point loop.
const BEAM_CLEAR_A: i32 = -0x5598;
const BEAM_CLEAR_B: i32 = -0x5590;
const BEAM_CLEAR_C: i32 = -0x558c;
const BEAM_COPY_DST: u32 = DATA_HI + 0x15d0;
const BEAM_CLEAR_D: u32 = GLOBAL_HI + 0x1720;
const BEAM_CLEAR_E_BASE: u32 = GLOBAL_HI + 0x1768;
const BEAM_CLEAR_F: u32 = GLOBAL_HI + 0x1774;

fn gp(c: &Cpu, off: i32) -> u32 {
    (c.g(28) as u32).wrapping_add(off as u32)
}

/// Sets a GPR to a 32-bit value, sign-extended as MIPS 32-bit ops leave it.
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// A retail `jal`: arguments go to a0-a3 and f12-f14, then the callee runs and every register it leaves (s0-s7
/// included) stays as it is, as after a retail call.
fn jal(c: &mut Cpu, addr: u32, ints: &[u32], floats: &[F]) {
    for (i, &a) in ints.iter().enumerate() {
        c.set_arg(i, a);
    }
    for (i, &x) in floats.iter().enumerate() {
        c.set_farg(i, x);
    }
    c.call(addr);
}

/// 0x2adea8 fan_emit_spawns(a0) -> void: for each of 10 slots with a nonzero flag, scales the slot's input by a
/// constant, converts it to an int and emits a spawn record through 0x2adf70; then tail-calls 0x2ae548 with a0.
pub fn fan_emit_spawns(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    h::frame(
        c,
        0x60,
        &[
            (20, 0x40, h::SQ),
            (19, 0x30, h::SQ),
            (18, 0x20, h::SQ),
            (17, 0x10, h::SQ),
            (16, 0x00, h::SQ),
            (31, 0x50, h::SQ),
        ],
        |c, _sp| fan_body(c),
    );
    true
}

/// The body of 0x2adea8 between its prologue and epilogue. The saved registers are restored by `frame`.
fn fan_body(c: &mut Cpu) {
    let fan_arg = c.arg(0);
    set32(c, 5, DATA_HI);
    set32(c, 3, DATA_HI);
    set32(c, 2, DATA_HI);
    let mut spawn = FAN_SPAWN_BASE;
    let mut scale_at = FAN_SCALE_TABLE;
    let mut flag_at = FAN_FLAG_TABLE;
    for _ in 0..FAN_SLOTS {
        let flag = c.r16(flag_at) as i16 as i32 as u32;
        set32(c, 2, flag);
        if flag == 0 {
            spawn = spawn.wrapping_add(FAN_SPAWN_STRIDE);
        } else {
            let input = c.r16(scale_at) as i16 as i32 as u32;
            set32(c, 4, input);
            let (_, f0) = h::run_keep(c, mathx::int_to_float, &[input], &[]);
            c.f[12] = (f0 * F(c.r32(FAN_SCALE_CONST))).0;
            let (scaled, _) = h::run_keep(c, mathx::float_to_int, &[], &[F(c.f[12])]);
            set32(c, 5, scaled);
            c.f[12] = c.r32(gp(c, FAN_GP_F12));
            c.f[13] = c.r32(gp(c, FAN_GP_F13));
            set32(c, 4, spawn);
            set32(c, 6, scaled);
            set32(c, 1, FAN_SPREAD.0);
            c.f[14] = FAN_SPREAD.0;
            set32(c, 7, 5);
            jal(c, FAN_SPAWN_FN, &[spawn, scaled, scaled, 5], &[F(c.f[12]), F(c.f[13]), F(c.f[14])]);
            spawn = spawn.wrapping_add(FAN_SPAWN_STRIDE);
        }
        scale_at = scale_at.wrapping_add(2);
        flag_at = flag_at.wrapping_add(2);
    }
    set32(c, 4, fan_arg);
    jal(c, FAN_TAIL_FN, &[fan_arg], &[]);
}

/// 0x2af980 morph_beam_reset(a0 = beam record) -> void: when the geometry latch is set, rescales the beam's
/// 12-point path from the record's source vector, copies it into the path table and clears the beam's phase words.
pub fn morph_beam_reset(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    h::frame(
        c,
        0x50,
        &[(16, 0x10, h::SQ), (17, 0x20, h::SQ), (18, 0x30, h::SQ), (31, 0x40, h::SQ)],
        |c, sp| beam_body(c, sp),
    );
    true
}

/// The body of 0x2af980 between its prologue and epilogue (`sp` is the frame's scratch quad at +0).
fn beam_body(c: &mut Cpu, sp: u32) {
    let beam = c.arg(0);
    let latch = c.r32(BEAM_LATCH);
    set32(c, 2, latch);
    if latch == 0 {
        return;
    }
    let src = c.r32(beam.wrapping_add(BEAM_SRC_PTR));
    c.w32(gp(c, BEAM_GP_CLEAR), 0);
    set32(c, 4, 12);
    let (_, f0) = h::run_keep(c, mathx::int_to_float, &[12], &[]);
    let f1 = c.rf(gp(c, BEAM_GP_SPEED)) / f0;
    c.f[1] = f1.0;
    c.wf(gp(c, BEAM_GP_SCALE), f1);
    let vec_in = beam.wrapping_add(BEAM_VEC_OFF);
    set32(c, 5, vec_in);
    set32(c, 4, sp);
    h::run_keep(c, vecmath::vec_scale, &[sp, vec_in], &[-f1]);

    set32(c, 3, BEAM_PATH_A);
    c.r[2] = c.mem.rd128(src);
    c.mem.wr128(BEAM_PATH_A, c.r[2]);
    set32(c, 4, BEAM_PATH_B);
    let mut dst = BEAM_PATH_A + BEAM_PATH_STEP;
    let mut point = BEAM_PATH_B + BEAM_PATH_STEP;
    for _ in 0..BEAM_POINT_COUNT {
        jal(c, BEAM_POINT_FN, &[dst, point, sp], &[]);
        point = point.wrapping_add(BEAM_PATH_STEP);
        dst = dst.wrapping_add(BEAM_PATH_STEP);
    }

    set32(c, 2, DATA_HI);
    c.w32(gp(c, BEAM_CLEAR_A), 0);
    c.w32(gp(c, BEAM_CLEAR_B), 0);
    c.w32(gp(c, BEAM_CLEAR_C), 0);
    set32(c, 5, BEAM_COPY_DST);
    set32(c, 6, sp);
    set32(c, 1, GLOBAL_HI);
    c.w32(BEAM_CLEAR_D, 0);
    set32(c, 4, BEAM_CLEAR_E_BASE);
    set32(c, 1, GLOBAL_HI);
    c.w32(BEAM_CLEAR_F, 0);
    set32(c, 3, 2);
    let mut word_at = BEAM_CLEAR_E_BASE;
    let mut quad_at = BEAM_COPY_DST;
    let mut left: i32 = 2;
    loop {
        c.w32(word_at, 0);
        c.r[2] = c.mem.rd128(sp);
        c.mem.wr128(quad_at, c.r[2]);
        quad_at = quad_at.wrapping_add(0x10);
        left -= 1;
        word_at = word_at.wrapping_add(4);
        if left < 0 {
            break;
        }
    }
    set32(c, 4, word_at);
    set32(c, 5, quad_at);
    c.s(3, (-1i64) as u64);
}
