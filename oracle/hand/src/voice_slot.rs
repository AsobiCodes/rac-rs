//! Hand versions of three level-4 voice-slot helpers: an ownership test, a release step and a moby cue starter.
//! The slot table sits at 0x13E550 with 0x70-byte records: state byte +0x74, cue id +0x7e, owner word +0x88.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const S0: usize = 16;
const S1: usize = 17;
const RA: usize = 31;

/// Base of the voice-slot table in EE RAM (lui 0x14, addiu -0x1ab0).
const SLOT_TABLE: u32 = 0x0013_E550;
/// Bytes per slot record (the retail code multiplies the slot index by 0x70).
const SLOT_SIZE: i64 = 0x70;
/// Record offset of the state byte (0 idle, 1..2 live, 4 and 6 released, 7 finished).
const OFF_STATE: u32 = 0x74;
/// Record offset of the cue id halfword.
const OFF_CUE: u32 = 0x7e;
/// Record offset of the owner (moby) word.
const OFF_OWNER: u32 = 0x88;
/// Record offset of the word cleared when a slot finishes.
const OFF_AUX: u32 = 0x8c;
/// State value that frees a slot in `voice_release`.
const STATE_FINISHED: u32 = 7;
/// Retail voice allocator (0x27e1f8, no Rust yet): a0 = cue record, a2 = owner, t0 = flags.
const VOICE_ALLOC: u32 = 0x0027_e1f8;
/// Allocator flags passed in t0.
const ALLOC_FLAGS: u32 = 0x400;
/// Cue records are 32 bytes (the retail code shifts the cue id left by 5).
const CUE_SHIFT: u32 = 5;
/// Offset in a moby of the cue-table pointer's owner, and in that owner of the table base.
const MOBY_CUE_PTR: u32 = 0x24;
const CUE_TABLE_PTR: u32 = 0x28;
/// Offset in the cue owner of the cue count byte.
const CUE_COUNT: u32 = 0x0d;

fn get(c: &Cpu, n: usize) -> u32 {
    c.g(n) as u32
}

fn set(c: &mut Cpu, n: usize, v: u32) {
    c.s(n, v as i32 as i64 as u64);
}

fn record(idx: u32) -> u32 {
    SLOT_TABLE.wrapping_add(idx.wrapping_mul(SLOT_SIZE as u32))
}

/// Sets LO and HI as the retail `mult` does: each 32-bit half is sign-extended into its 64-bit part.
fn mult(c: &mut Cpu, product: i64) {
    let lo = product as i32 as i64 as u64 as u128;
    let hi = (product >> 32) as i32 as i64 as u64 as u128;
    c.lo = (c.lo & !(u64::MAX as u128)) | lo;
    c.hi = (c.hi & !(u64::MAX as u128)) | hi;
}

/// 0x27e148 voice_alive(owner a0, slot a1) -> v0: 1 when slot a1 belongs to owner a0 and its state is 1 or 2, else 0.
pub fn voice_alive(c: &mut Cpu) -> bool {
    let owner = get(c, A0);
    let slot = get(c, A1) as i32;
    let a0 = c.g(A0);
    c.s(A2, a0);
    if slot < 0 {
        set(c, V0, 0);
        return true;
    }
    let rec = record(slot as u32);
    mult(c, slot as i64 * SLOT_SIZE);
    set(c, V1, rec);
    let stored = c.r32(rec + OFF_OWNER);
    set(c, A0, stored);
    if stored != owner {
        set(c, V0, 0);
        return true;
    }
    let state = c.r8(rec + OFF_STATE) as u32;
    let live = u32::from(state.wrapping_sub(1) < 2);
    set(c, V1, live);
    set(c, V0, live);
    true
}

/// 0x27e1a0 voice_release(slot a0): state 7 frees the record (v0 = table base), state 0 gives v0 = 6,
/// state 6 gives v0 = 4, and any other state becomes 4 (v0 = 4).
pub fn voice_release(c: &mut Cpu) -> bool {
    let slot = get(c, A0) as i32;
    set(c, V1, SLOT_SIZE as u32);
    if slot < 0 {
        return true;
    }
    mult(c, slot as i64 * SLOT_SIZE);
    set(c, V0, SLOT_TABLE);
    set(c, A0, STATE_FINISHED);
    let rec = record(slot as u32);
    set(c, V1, rec);
    let state = c.r8(rec + OFF_STATE) as u32;
    set(c, A1, state);
    if state == STATE_FINISHED {
        c.w8(rec + OFF_STATE, 0);
        c.w32(rec + OFF_OWNER, 0);
        c.w32(rec + OFF_AUX, 0);
        return true;
    }
    if state == 0 {
        set(c, V0, 6);
        return true;
    }
    if state == 6 {
        set(c, V0, 4);
        return true;
    }
    c.w8(rec + OFF_STATE, 4);
    set(c, V0, 4);
    true
}

/// 0x27e470 moby_cue(cue a0, a1, moby a2) -> v0: starts cue `a0` on moby `a2` through the voice allocator and
/// returns the new slot index, or -1 when the moby, its cue table or the cue id is missing.
pub fn moby_cue(c: &mut Cpu) -> bool {
    h::frame(c, 0x30, &[(S1, 0x10, h::SQ), (S0, 0x00, h::SQ), (RA, 0x20, h::SQ)], |c, _sp| {
        let a0 = c.g(A0);
        c.s(S1, a0);
        let a2 = c.g(A2);
        c.s(S0, a2);
        start_cue(c, get(c, A0), get(c, A2));
    });
    true
}

/// The body of `moby_cue` after its prologue: `cue` is the cue id, `moby` the owner.
fn start_cue(c: &mut Cpu, cue: u32, moby: u32) {
    if moby == 0 {
        set(c, V0, u32::MAX);
        return;
    }
    let table_owner = c.r32(moby + MOBY_CUE_PTR);
    if table_owner == 0 {
        set(c, V0, u32::MAX);
        return;
    }
    let table = c.r32(table_owner + CUE_TABLE_PTR);
    set(c, V1, table);
    if table == 0 {
        set(c, V0, u32::MAX);
        return;
    }
    let count = c.r8(table_owner + CUE_COUNT) as i64;
    set(c, V0, count as u32);
    let cue_s = c.g(S1) as i64;
    let in_range = u32::from(cue_s < count);
    set(c, V0, in_range);
    // The delay slot of the test always runs: a0 = cue << 5.
    let shifted = cue << CUE_SHIFT;
    set(c, A0, shifted);
    if in_range == 0 {
        set(c, V0, u32::MAX);
        return;
    }
    let s0 = c.g(S0);
    c.s(A2, s0);
    set(c, A0, table.wrapping_add(shifted));
    set(c, A3, 0);
    set(c, T0, ALLOC_FLAGS);
    h::call_keep(c, VOICE_ALLOC, &[], &[]);
    let r = get(c, V0) as i32;
    set(c, A0, r as u32);
    if r < 0 {
        // Retail: the delay slot of `bltz` (addiu v1,zero,0x70) runs whether or not the branch is taken.
        set(c, V1, SLOT_SIZE as u32);
        set(c, V0, r as u32);
        return;
    }
    let entry = record(r as u32);
    mult(c, r as i64 * SLOT_SIZE);
    set(c, V1, entry);
    set(c, V0, SLOT_TABLE);
    c.w16(entry + OFF_CUE, cue as u16);
    c.w32(entry + OFF_OWNER, moby);
    set(c, V0, r as u32);
}
