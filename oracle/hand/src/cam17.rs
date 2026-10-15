//! Camera class-17 gate (retail 0x2f0b90) and the record release (retail 0x2f1210) that goes with it. Both take the actor;
//! its slot (32-byte records in the table at 0x15ef50, the record pointer at slot+0x1c) holds the camera state.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::{w4, Cpu};

/// Slot table base pointer (lui 0x16 / lw -0x10b0) and the slot index stride (<< 5).
const SLOT_TABLE_WORD: u32 = 0x15_ef50;
const SLOT_SHIFT: u32 = 5;
/// Actor: slot index (i16 at +0x84). Slot: record pointer at +0x1c.
const ACT_SLOT: u32 = 0x84;
const SLOT_REC: u32 = 0x1c;
/// Record: kind (u16 at +0x2c), state (i16 at +0x48), mode (i16 at +0x4a), timer (i16 at +0x2e), hold word (+0x20) and
/// hold half (+0x26).
const REC_STATE: u32 = 0x48;
const REC_MODE: u32 = 0x4a;
const REC_KIND: u32 = 0x2c;
const REC_TIMER: u32 = 0x2e;
const REC_HOLD_WORD: u32 = 0x20;
const REC_HOLD_HALF: u32 = 0x26;
/// Kinds the gate looks at: 1 and 10 (0xa) take the timer test, 11 (0xb) the mode byte test, 2 the word test.
const KIND_ONE: i32 = 1;
const KIND_TEN: i32 = 0xa;
const KIND_ELEVEN: i32 = 0xb;
const KIND_TWO: i32 = 2;
const MODE_BYTE_WANT: u32 = 2;
/// Camera block (0x13f350): timer half at +0x30c (must be 0 to skip), word at +0x2284 (0xd), and the mode word at
/// gp - 0x7e7c (0xe) which the gate reads first.
const CAM_BLOCK: u32 = 0x13_f350;
const CAM_TIMER: u32 = 0x30c;
const CAM_WORD: u32 = 0x2284;
const CAM_WORD_WANT: u32 = 0xd;
const GP_MODE: u32 = 0x7e7c;
const GP_MODE_WANT: u32 = 0xe;
/// Mode byte (0x1413f4, must be 2 for kind 11), state word (0x1413dc, 0x1a for kind 2).
const MODE_BYTE_ADDR: u32 = 0x14_13f4;
const STATE_WORD_ADDR: u32 = 0x14_13dc;
const STATE_WORD_WANT: u32 = 0x1a;
/// Gate word read by the release (lui 0x16 / lw -0xa5c): zero sends it straight to the second step.
const GATE_WORD: u32 = 0x15_f5a4;
/// Retail steps: the gate's action (0x2f02e8), the release's first step (0x2f0ca8, returns non-zero on success) and
/// its fallback (0x2f0df8).
const GATE_STEP: u32 = 0x2f_02e8;
const RELEASE_TRY: u32 = 0x2f_0ca8;
const RELEASE_FALLBACK: u32 = 0x2f_0df8;

/// Sets a GPR to a sign-extended 32-bit value, as `lw` / `addiu` leave it.
fn set_gpr(c: &mut Cpu, n: usize, v: u32) {
    c.r[n] = v as i32 as i128 as u128;
}

/// Sign-extends a halfword as `lh` does.
fn s16(v: u16) -> i32 {
    v as i16 as i32
}

/// Address of the actor's slot: table + (slot index << 5).
fn slot_addr(c: &mut Cpu, actor: u32, table: u32) -> u32 {
    let idx = s16(c.r16(actor + ACT_SLOT));
    ((idx << SLOT_SHIFT) as u32).wrapping_add(table)
}

/// Sets v0, v1 and a0 as the exit of the gate leaves them.
fn exit_regs(c: &mut Cpu, v0: u32, v1: u32, a0: u32) {
    set_gpr(c, 2, v0);
    set_gpr(c, 3, v1);
    set_gpr(c, 4, a0);
}

/// The gate's action call: v0 = t, v1 = 2, a0 = the actor, then the retail step with the scratch registers it leaves.
fn gate_step(c: &mut Cpu, actor: u32, v0: u32, v1: u32) -> bool {
    exit_regs(c, v0, v1, actor);
    h::call_keep(c, GATE_STEP, &[actor], &[]);
    set_gpr(c, 2, 0);
    true
}

/// 0x2f0b90 cam17_gate(actor) -> 0: for the actor's slot record in the class-17 states, runs the retail step 0x2f02e8
/// on the actor; always returns 0.
pub fn cam17_gate(c: &mut Cpu) -> bool {
    let actor = c.arg(0);
    // sq ra, 0(sp): the frame slot takes ra's 128 bits (the register is restored unchanged).
    let sp = (c.g(29) as u32).wrapping_sub(0x10);
    let ra = c.r[31];
    c.wv(sp, V(w4(ra)));
    set_gpr(c, 7, actor);
    let table = c.r32(SLOT_TABLE_WORD);
    let slot = slot_addr(c, actor, table);
    let rec = c.r32(slot + SLOT_REC);
    set_gpr(c, 3, table);
    set_gpr(c, 2, slot);
    set_gpr(c, 5, rec);
    set_gpr(c, 4, 1);
    let state = s16(c.r16(rec + REC_STATE));
    if state == KIND_ONE && s16(c.r16(rec + REC_MODE)) != 0 {
        exit_regs(c, 0, 1, 1);
        return true;
    }
    // Join: kind = the halfword at +0x2c (a2 zero-extended, v1 sign-extended).
    let kind = c.r16(rec + REC_KIND);
    set_gpr(c, 6, kind as u32);
    let t = s16(kind);
    let mut v0 = KIND_ELEVEN as u32;
    let mut v1 = t as u32;
    let mut a0 = 1u32;
    if t == KIND_ELEVEN {
        let mode = c.r8(MODE_BYTE_ADDR) as u32;
        a0 = MODE_BYTE_WANT;
        v0 = 0;
        v1 = mode;
        if mode != MODE_BYTE_WANT {
            exit_regs(c, 0, v1, a0);
            return true;
        }
    }
    // 0x2f0bf4: a0 = t, v1 = 1.
    a0 = t as u32;
    v1 = 1;
    if t == KIND_ONE || t == KIND_TEN {
        v0 = s16(c.r16(rec + REC_TIMER)) as u32;
        if v0 == 0 {
            a0 = CAM_BLOCK;
            v1 = s16(c.r16(CAM_BLOCK + CAM_TIMER)) as u32;
            if v1 != 0 {
                v1 = c.r32((c.g(28) as u32).wrapping_sub(GP_MODE));
                v0 = GP_MODE_WANT;
                if v1 != GP_MODE_WANT {
                    exit_regs(c, 0, v1, a0);
                    return true;
                }
                v1 = c.r32(CAM_BLOCK + CAM_WORD);
                v0 = CAM_WORD_WANT;
                if v1 != CAM_WORD_WANT {
                    exit_regs(c, 0, v1, a0);
                    return true;
                }
                v0 = 0;
            }
        }
    }
    // 0x2f0c50: v1 = 2, v0 = t.
    v1 = KIND_TWO as u32;
    v0 = t as u32;
    if t != KIND_TWO {
        return gate_step(c, actor, v0, v1);
    }
    v0 = 0x14_0000;
    a0 = STATE_WORD_WANT;
    v1 = c.r32(STATE_WORD_ADDR);
    if v1 != STATE_WORD_WANT {
        c.w16(rec + REC_TIMER, 0);
        exit_regs(c, 0, v1, a0);
        return true;
    }
    gate_step(c, actor, v0, v1)
}

/// 0x2f1210 cam17_release(actor) -> 0: for an actor whose record holds no claim (+0x26 zero), releases it: when the gate
/// word is set and the retail step 0x2f0ca8 succeeds, clears the claim words; otherwise runs 0x2f0df8. Always returns 0.
pub fn cam17_release(c: &mut Cpu) -> bool {
    let actor = c.arg(0);
    // The three sq's of the frame (s1, ra, s0 as they were on entry).
    let sp = (c.g(29) as u32).wrapping_sub(0x30);
    c.wv(sp + 0x10, V(w4(c.r[17])));
    c.wv(sp + 0x20, V(w4(c.r[31])));
    c.wv(sp, V(w4(c.r[16])));
    let table = c.r32(SLOT_TABLE_WORD);
    let slot = slot_addr(c, actor, table);
    set_gpr(c, 3, slot);
    let rec = c.r32(slot + SLOT_REC);
    if s16(c.r16(rec + REC_HOLD_HALF)) != 0 {
        set_gpr(c, 2, 0);
        return true;
    }
    let gate = c.r32(GATE_WORD);
    set_gpr(c, 2, gate);
    if gate != 0 {
        let (ok, _) = h::call_keep(c, RELEASE_TRY, &[actor], &[]);
        if ok != 0 {
            c.w32(rec + REC_HOLD_WORD, 0);
            c.w16(rec + REC_HOLD_HALF, 0);
            set_gpr(c, 2, 0);
            return true;
        }
    }
    set_gpr(c, 2, 0);
    set_gpr(c, 4, actor);
    h::call_keep(c, RELEASE_FALLBACK, &[actor], &[]);
    set_gpr(c, 2, 0);
    true
}
