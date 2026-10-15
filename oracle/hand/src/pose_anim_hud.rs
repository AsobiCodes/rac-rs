//! Pose probe, anim start and the Trespasser HUD backdrop. Each retail call that leaves scratch registers behind is
//! mirrored with `h::run_keep` / `h::call_keep`, so the caller sees the same v0/v1/a0-a3/t0-t3/f0-f19 as after a jal.

use crate::ps2::*;
use crate::{actor, euler_basis, fxtex, h, mathx, objects, tex_strip_pkt};
use ee_oracle::cpu::Cpu;

const GP: usize = 28;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;
const T2: usize = 10;
const T3: usize = 11;

/// Retail probe 0x251010, called with eight integer arguments and seven float arguments (f12..f18).
const PROBE_FN: u32 = 0x0025_1010;
/// Pose node index sampled into the stack vector.
const POSE_NODE: u32 = 12;
/// Integer arguments of the probe (a3, t0, t1, t2, t3) and its stack words (1, -1, 0).
const PROBE_A3: u32 = 10;
const PROBE_T0: u32 = 3;
const PROBE_T1: u32 = 4;
const PROBE_T2: i32 = -1;
const PROBE_T3: u32 = 1;
const PROBE_STACK_V0: u32 = 1;
const PROBE_STACK_V1: i32 = -1;
const PROBE_SCALE: F = F(0x47c3_5000);
const ONE: F = F(0x3f80_0000);
const TWO: F = F(0x4000_0000);
const FIFTEEN: F = F(0x4170_0000);

/// Word read at 0x1413d4 (lui 0x14 + lw 0x13d4): the flag that must be 8 for the anim start.
const FLAG_WORD: u32 = 0x0014_13d4;
const FLAG_VALUE: u32 = 8;
/// `lui v0, 0x14` leaves this in v0 on every path.
const LUI_14: u64 = 0x0014_0000;
/// Object byte at +0x53: the kind that skips the anim start.
const KIND_OFF: u32 = 0x53;
const KIND_SKIP: u32 = 6;
/// Input of the scaling helper 0x1f14b8 and the anim index passed to objects::anim_start.
const SCALE_INPUT: u32 = 10;
const ANIM_INDEX: u32 = 6;

/// HUD table at 0x1d2e70 (lui 0x1d + addiu 0x2e70): four 32-byte records of two 64-bit words each.
const HUD_TABLE: u32 = 0x001d_2e70;
const HUD_RECORDS: u32 = 4;
const HUD_RECORD_SIZE: u32 = 0x20;
/// gp-relative word slots (offsets from gp).
const GP_TEX_REC: i32 = -0x5214;
const GP_ARG_A1: i32 = -0x527c;
const GP_ARG_A2: i32 = -0x5278;
const GP_ARG_A3: i32 = -0x519c;
const GP_ARG_V0: i32 = -0x5280;
const GP_ARG_V1: i32 = -0x5284;
const TEX_REC_BIAS: u32 = 0x28;

/// Reads the gp-relative word at gp + off.
fn gp_word(c: &mut Cpu, gp: u32, off: i32) -> u32 {
    c.r32((gp as i32).wrapping_add(off) as u32)
}

/// 0x2d0f30 pose_probe_call(obj) -> void: zeroes a stack vector, samples pose node 12 of obj into a second stack
/// vector, then calls the retail probe 0x251010 with those vectors and fixed scale and stack words.
pub fn pose_probe_call(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    h::frame(c, 0x70, &[(16, 0x40, h::SQ), (17, 0x50, h::SQ), (31, 0x60, h::SQ)], |c, sp| {
        let zeroed = sp + 0x30;
        let sampled = sp + 0x20;
        h::run_keep(c, euler_basis::zero_vec4, &[zeroed], &[]);
        h::run_keep(c, actor::pose_node_pos, &[obj, POSE_NODE, sampled], &[]);
        c.w32(sp, PROBE_STACK_V0);
        c.w32(sp + 8, PROBE_STACK_V1 as u32);
        c.w32(sp + 0x10, 0);
        c.s(V0, PROBE_STACK_V0 as u64);
        c.s(V1, PROBE_STACK_V1 as i64 as u64);
        let ints = [obj, zeroed, sampled, PROBE_A3, PROBE_T0, PROBE_T1, PROBE_T2 as u32, PROBE_T3];
        let floats = [F(0), F(0), TWO, ONE, PROBE_SCALE, ONE, FIFTEEN];
        h::call_keep(c, PROBE_FN, &ints, &floats);
    });
    true
}

/// 0x2d41e8 anim_start_if_flag(obj) -> void: when the word at 0x1413d4 is 8 and obj's byte at +0x53 is not 6, scales
/// the constant 10 with 0x1f14b8 and starts anim 6 on obj with that result as the speed.
pub fn anim_start_if_flag(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    h::frame(c, 0x20, &[(16, 0x0, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        let flag = c.r32(FLAG_WORD);
        c.s(V0, LUI_14);
        c.s(V1, FLAG_VALUE as u64);
        c.s(A0, flag as i32 as i64 as u64);
        if flag != FLAG_VALUE {
            return;
        }
        let kind = c.r8(obj + KIND_OFF) as u32;
        c.s(V0, KIND_SKIP as u64);
        c.s(V1, kind as u64);
        if kind == KIND_SKIP {
            return;
        }
        let (scaled, _) = h::run_keep(c, mathx::float_to_int_scaled, &[SCALE_INPUT], &[]);
        h::run_keep(c, objects::anim_start, &[obj, ANIM_INDEX, 0, scaled], &[]);
    });
    true
}

/// 0x2d67c0 trespass_hud_backdrop() -> void: four passes over the HUD table at 0x1d2e70; each uploads the texture
/// of the record (gp-relative slots) and emits one textured strip into a stack packet.
pub fn trespass_hud_backdrop(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let gp = c.g(GP) as u32;
    let saves = [(16, 0x40, h::SQ), (17, 0x50, h::SQ), (18, 0x60, h::SQ), (19, 0x70, h::SQ), (31, 0x80, h::SQ)];
    h::frame(c, 0x90, &saves, |c, sp| {
        let packet = sp;
        let quad_a = sp + 0x20;
        let quad_b = sp + 0x30;
        for i in 0..HUD_RECORDS {
            let rec = HUD_TABLE + i * HUD_RECORD_SIZE;
            let t0 = c.mem.rd64(rec);
            let t1 = c.mem.rd64(rec + 8);
            let t2 = c.mem.rd64(rec + 0x10);
            let t3 = c.mem.rd64(rec + 0x18);
            let a1 = gp_word(c, gp, GP_ARG_A1);
            let a2 = gp_word(c, gp, GP_ARG_A2);
            let a3 = gp_word(c, gp, GP_ARG_A3);
            let v1 = gp_word(c, gp, GP_ARG_V1);
            let v0 = gp_word(c, gp, GP_ARG_V0);
            let a0 = gp_word(c, gp, GP_TEX_REC).wrapping_add(TEX_REC_BIAS);
            c.w32(sp + 0x38, a1);
            c.w32(sp + 0x3c, a2);
            c.w32(sp + 0x20, a3);
            c.mem.wr64(sp, t0);
            c.w32(sp + 0x2c, a3);
            c.w32(sp + 0x28, a3);
            c.w32(sp + 0x24, a3);
            c.w32(sp + 0x30, v1);
            c.w32(sp + 0x34, v0);
            c.mem.wr64(sp + 8, t1);
            c.mem.wr64(sp + 0x10, t2);
            c.mem.wr64(sp + 0x18, t3);
            // The 64-bit doubleword registers are set directly: the int arguments of h::run_keep are 32-bit.
            c.s(V0, v0 as i32 as i64 as u64);
            c.s(V1, v1 as i32 as i64 as u64);
            c.s(T0, t0);
            c.s(T1, t1);
            c.s(T2, t2);
            c.s(T3, t3);
            h::run_keep(c, fxtex::fx_tex_upload, &[a0, a1, a2, a3], &[]);
            // move a3, v0 (full 64 bits); addiu t0, zero, 1
            let tex = c.g(V0);
            c.s(A3, tex);
            c.s(T0, 1);
            h::run_keep(c, tex_strip_pkt::emit_textured_strip, &[packet, quad_b, quad_a], &[]);
        }
    });
    true
}
