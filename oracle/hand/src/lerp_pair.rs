//! Three small EE routines from the level 4 overlay: a two-sample blend from a slot table, a callback dispatch
//! through a function table, and a store into the control block at 0x1000d400.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// MIPS register numbers used below.
const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const S0: usize = 16;
const GP: usize = 28;
const SP: usize = 29;
const RA: usize = 31;

/// Sets a general register to a sign-extended 32-bit value.
fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Reads a quadword back from memory as a 128-bit register value.
fn unquad(v: V) -> u128 {
    v.0[0] as u128 | (v.0[1] as u128) << 32 | (v.0[2] as u128) << 64 | (v.0[3] as u128) << 96
}

/// Sets float register `reg`.
fn set_fpr(c: &mut Cpu, reg: usize, f: F) {
    c.f[reg] = f.0;
}

/// A 128-bit register as a quadword (lane 0 is the low word).
fn quad(x: u128) -> V {
    V([x as u32, (x >> 32) as u32, (x >> 64) as u32, (x >> 96) as u32])
}

/// Sample table: 16 bytes per slot, 19 slots.
const SAMPLE_TABLE: u32 = 0x0018_2b70;
/// Word holding the replacement slot used when the requested slot is -1.
const WRAP_SLOT_CELL: u32 = 0x0015_ed84;
/// Slot that takes the special path when the high flag is set.
const SPECIAL_SLOT: i32 = 6;
/// Number of valid slots; anything outside 0..SLOT_COUNT reads slot 0.
const SLOT_COUNT: i32 = 19;
/// Requests at or above this set the high flag and are reduced by it.
const HIGH_FLAG: i32 = 100;
/// 2^-9, the scale applied to every blended sample.
const BLEND_SCALE: F = F(0x3b00_0000);
/// Bias added to the blended first output on the special path.
const SPECIAL_BIAS_A: F = F(0x4483_a000);
/// Bias the second output starts from on the special path.
const SPECIAL_BIAS_B: F = F(0x4439_0000);

/// 0x23cd40 blend_pair(out_a, out_b, n, x, y): blends two samples of slot n into out_a and out_b.
/// n >= 100 sets the high flag and subtracts 100; slot -1 is replaced by the wrap cell, other bad slots become 0.
/// Slot 6 with the flag uses the special bias constants.
pub fn blend_pair(c: &mut Cpu) -> bool {
    // move $t0, $a0 and move $t1, $a1 are addu with $zero: 32-bit results, sign-extended.
    set_gpr(c, 8, c.arg(0));
    set_gpr(c, 9, c.arg(1));
    let out_a = c.arg(0);
    let out_b = c.arg(1);
    let x = c.farg(0);
    let y = c.farg(1);
    let wrap_cell = c.r32(WRAP_SLOT_CELL) as i32;
    let mut slot = c.arg(2) as i32;
    let flag = slot >= HIGH_FLAG;
    if flag {
        slot -= HIGH_FLAG;
    }
    if slot == -1 {
        slot = wrap_cell;
    }
    if slot < 0 || slot >= SLOT_COUNT {
        slot = 0;
    }
    let offset = (slot as u32) << 4;
    let base = SAMPLE_TABLE;
    let flag_word = flag as u32;
    if flag && slot == SPECIAL_SLOT {
        let a = (c.rf(base + 0x6c) * y + SPECIAL_BIAS_A) * BLEND_SCALE;
        // Retail stores out_a before it reads table word 0x64, so the store is kept ahead of that read.
        c.wf(out_a, a);
        let x_term = c.rf(base + 0x64) * x;
        let b = (SPECIAL_BIAS_B - x_term) * BLEND_SCALE;
        c.wf(out_b, b);
        set_gpr(c, V0, base);
        set_gpr(c, V1, offset);
        set_gpr(c, A0, wrap_cell as u32);
        set_gpr(c, A1, SPECIAL_SLOT as u32);
        set_gpr(c, A2, SPECIAL_SLOT as u32);
        set_gpr(c, A3, flag_word);
        set_gpr(c, AT, SPECIAL_BIAS_B.0);
        set_fpr(c, 0, a);
        set_fpr(c, 1, x_term);
        set_fpr(c, 2, b);
        set_fpr(c, 3, BLEND_SCALE);
        return true;
    }
    let e = base + offset;
    let a = (c.rf(e) + c.rf(e + 4) * x) * BLEND_SCALE;
    c.wf(out_a, a);
    let b_term = c.rf(e + 0xc) * y;
    let b = (c.rf(e + 8) + b_term) * BLEND_SCALE;
    c.wf(out_b, b);
    set_gpr(c, V0, e);
    set_gpr(c, V1, offset);
    set_gpr(c, A0, e);
    set_gpr(c, A1, e);
    set_gpr(c, A2, e);
    set_gpr(c, A3, flag_word);
    set_gpr(c, AT, BLEND_SCALE.0);
    set_fpr(c, 0, b);
    set_fpr(c, 1, b_term);
    set_fpr(c, 2, BLEND_SCALE);
    true
}

/// Table of callback pointers, indexed by the word at CALLBACK_INDEX.
const CALLBACK_TABLE: u32 = 0x0018_4638;
/// Index of the callback to run.
const CALLBACK_INDEX: u32 = 0x0015_eeb0;
/// Count of consecutive runs with the same index (reset when the index changes during the call).
const HOLD_COUNTER: u32 = 0x0016_1f00;
/// gp-relative slot receiving the new hold count.
const GP_HOLD_COPY: u32 = (-0x4d00i32) as u32;
/// Stack frame size of the dispatcher.
const FRAME: u32 = 0x20;

/// 0x23d178 run_callback(): calls the table entry picked by the index, then bumps the hold counter and clears
/// it when the index changed during the call.
pub fn run_callback(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let sp = c.g(SP) as u32;
    let frame = sp.wrapping_sub(FRAME);
    // Prologue: s0 and ra are saved in the frame (sq), so they are left there as retail does.
    let saved_s0 = c.r[S0];
    let saved_ra = c.r[RA];
    c.wv(frame, quad(saved_s0));
    c.wv(frame + 0x10, quad(saved_ra));
    let index = c.r32(CALLBACK_INDEX);
    let target = c.r32(CALLBACK_TABLE.wrapping_add(index << 2));
    set_gpr(c, SP, frame);
    // The callback runs as retail code, so the registers it leaves behind are kept as they are.
    set_gpr(c, A0, target);
    c.call(target);
    // Epilogue: lq reloads s0 and ra from the frame.
    c.r[S0] = unquad(c.rv(frame));
    c.r[RA] = unquad(c.rv(frame + 0x10));
    let count = c.r32(HOLD_COUNTER).wrapping_add(1);
    let index_now = c.r32(CALLBACK_INDEX);
    let gp = c.g(GP) as u32;
    c.w32(gp.wrapping_add(GP_HOLD_COPY), count);
    // The beq compares index_now with s0, which the callee preserves (ABI), so s0 still holds index.
    if index_now != index {
        c.w32(HOLD_COUNTER, 0);
        set_gpr(c, AT, HOLD_COUNTER & 0xffff_0000);
    }
    set_gpr(c, V0, count);
    set_gpr(c, V1, index_now);
    set_gpr(c, SP, sp);
    true
}

/// Control block: offset 0 gets the status word, 0x10, 0x20 and 0x80 get the three arguments.
const CTRL_BLOCK: u32 = 0x1000_d400;
/// Status word written and returned by store_ctrl.
const CTRL_STATUS: u32 = 0x100;

/// 0x23fbf8 store_ctrl(a, b, c) -> 0x100: stores a0, a1 and a2 at offsets 0x10, 0x20 and 0x80 of the control block
/// at 0x1000d400, writes the status word 0x100 at its start and returns it.
pub fn store_ctrl(c: &mut Cpu) -> bool {
    let (a0, a1, a2) = (c.arg(0), c.arg(1), c.arg(2));
    c.w32(CTRL_BLOCK + 0x80, a2);
    c.w32(CTRL_BLOCK + 0x20, a1);
    c.w32(CTRL_BLOCK + 0x10, a0);
    c.w32(CTRL_BLOCK, CTRL_STATUS);
    set_gpr(c, AT, CTRL_BLOCK);
    c.ret(CTRL_STATUS);
    true
}
