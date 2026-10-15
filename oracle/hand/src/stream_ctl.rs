//! Stream control block at 0x1516d0 (retail 0x257708..0x2577e8, 0x2577e8..0x25783c, 0x257a00..0x257a40 and
//! 0x258080..0x2580c0).
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
const T1: usize = 9;
const T2: usize = 10;
const T3: usize = 11;
const T4: usize = 12;
const T5: usize = 13;
const T6: usize = 14;
const T7: usize = 15;
const S0: usize = 16;
const RA: usize = 31;

/// The stream control block (the `0x150000 + 0x16d0` the entries address).
const CTL: u32 = 0x0015_16d0;
/// Word at +0x6c: nonzero while a start is pending (the start writes -1 there).
const CTL_PENDING: u32 = CTL + 0x6c;
/// Table of words indexed by the id (`id * 4 + 0x13a628`).
const ID_TABLE: u32 = 0x0013_a628;
/// Data address passed to the start routine as its second stack argument.
const START_PARAM: u32 = 0x0025_8418;
/// Retail start routine (called with the table entry in a0).
const START_FN: u32 = 0x0012_ec08;
/// Retail routine called by `stream_kick` when its arm halfword is set.
const KICK_FN: u32 = 0x0012_eea8;

/// 0x257708 stream_start(key, id, prio, vol) -> bool: when no start is pending and the id's table entry is set,
/// records the start parameters in the control block, calls the retail start routine and returns 1; else 0.
pub fn stream_start(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let key = c.g(A0);
    let id = c.arg(1);
    let vol = c.g(A2);
    let prio = c.g(A3);
    h::frame(c, 0x30, &[(RA, 0x20, h::SQ)], |c, sp| {
        c.s(T5, CTL as u64);
        c.s(T7, key);
        c.s(T3, vol);
        c.s(T6, prio);
        if c.r32(CTL_PENDING) != 0 {
            c.s(V0, 0);
            return true;
        }
        let slot = (id << 2).wrapping_add(ID_TABLE);
        let entry = c.r32(slot);
        c.s(A0, slot as u64);
        c.s(V1, entry as i32 as i64 as u64);
        if entry == 0 {
            c.s(V0, 0);
            return true;
        }
        let entry_val = entry as i32 as i64 as u64;
        c.s(V1, CTL_PENDING as u64);
        c.s(A0, entry_val);
        c.s(A1, 10);
        c.s(A2, 0xbb80);
        c.s(T1, 0x20);
        c.s(T2, START_PARAM as u64);
        c.s(V0, u64::MAX);
        c.s(T4, 1);
        let prio_s = (prio as u16 as i16 as i32) as i64 as u64;
        c.s(T0, prio_s);
        c.w16(CTL + 0x74, vol as u16);
        c.w32(CTL + 0x80, 10);
        c.w32(CTL + 0x84, 0xbb80);
        c.w32(CTL_PENDING, u32::MAX);
        c.w16(CTL + 0x70, key as u16);
        c.w16(CTL + 0x7c, 1);
        c.w16(CTL + 0x76, 1);
        c.w16(CTL + 0x72, prio as u16);
        c.w32(sp, 0x20);
        c.w32(sp + 8, START_PARAM);
        c.w32(sp + 0x10, CTL_PENDING);
        c.w32(sp + 0x14, 0);
        c.s(T2, 1);
        c.s(T3, 0);
        c.s(A1, 0);
        c.s(A2, 0);
        c.s(A3, 0);
        c.s(T1, 0);
        h::call_keep(c, START_FN, &[entry, 0, 0, 0, prio_s as u32, 0, 1, 0], &[]);
        c.s(V0, 1);
        true
    })
}

/// 0x2577e8 stream_slot_set(key, val): when the slot keyed by `key` (halfword at +0x38) is not already set
/// (or its byte at +0x22 is not -1), stores `key` there, or when the block is busy stores `val` and `key` at
/// +0x23 / +0x22. Returns nothing.
pub fn stream_slot_set(c: &mut Cpu) -> bool {
    let key = c.g(A0);
    let val = c.arg(1);
    c.s(A2, CTL as u64);
    c.s(A3, 0x0015_0000);
    let slot_key = c.r16(CTL + 0x38) as i16 as i64 as u64;
    if slot_key != key {
        c.s(V1, slot_key);
    } else {
        let tag = c.r8(CTL + 0x22) as i8 as i64 as u64;
        c.s(V1, tag);
        if tag == u64::MAX {
            c.s(V0, u64::MAX);
            return true;
        }
    }
    c.s(V0, CTL as u64);
    if c.r16(CTL + 0x3e) != 0 || c.r16(CTL + 0x76) != 0 {
        c.w8(CTL + 0x23, val as u8);
        c.w8(CTL + 0x22, key as u8);
    } else {
        c.w16(CTL + 0x38, key as u16);
    }
    true
}

/// 0x257a00 stream_hold_timers_set(v): for each of the three halfword flags at +0x40, +0x78 and +0x5c that has
/// bit 0x8000 set, stores `v` in the halfword after it. Returns the last flag bit tested (v0).
pub fn stream_hold_timers_set(c: &mut Cpu) -> bool {
    let val = c.g(A0) as u16;
    c.s(V1, CTL as u64);
    let mut last = 0u32;
    for off in [0x40u32, 0x78, 0x5c] {
        last = c.r16(CTL + off) as u32 & 0x8000;
        if last != 0 {
            c.w16(CTL + off + 2, val);
        }
    }
    c.s(V0, last as u64);
    true
}

/// 0x258080 stream_kick(): when the arm halfword at +0x8 is set, calls the retail routine 0x12eea8 and sets the
/// byte at +0xa to 1. Returns nothing.
pub fn stream_kick(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x20, &[(S0, 0x00, h::SQ), (RA, 0x10, h::SQ)], |c, _sp| {
        c.s(V0, 0x0015_0000);
        c.s(S0, CTL as u64);
        let armed = c.r16(CTL + 8) as i16 as i64 as u64;
        c.s(V1, armed);
        if armed == 0 {
            return true;
        }
        h::call_keep(c, KICK_FN, &[], &[]);
        c.s(V1, 1);
        c.w8(CTL + 0xa, 1);
        true
    })
}
