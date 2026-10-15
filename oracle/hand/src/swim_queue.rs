//! Byte queue of slot ids (0x1f7c08) and a blend between two float table entries (0x1fc0c0).
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Queue bytes (lui 0x14 + 0x1e08).
const QUEUE: u32 = 0x141E08;
/// Upper half of the queue's data page (t1 in the retail body).
const QUEUE_PAGE: u32 = 0x140000;
/// Upper half of the queue length word's page (at in the retail body).
const LEN_PAGE: u32 = 0x160000;
/// Queue length word (lui 0x16 - 0x11d0).
const QUEUE_LEN: u32 = 0x15EE30;
/// gp-relative word: how many queued bytes a lookup searches.
const QUEUE_COUNT_OFF: i32 = -0x7DD0;
/// Retail lookup: `(key, 0, 0)` -> slot id, or -1 when the key has none.
const SLOT_LOOKUP: u32 = 0x1F7BA0;
/// Upper half of the lerp's data page (s2 in the retail body).
const TBL_BASE: u32 = 0x13F350;
/// Word: bit 1 selects the wrapped index `n - 1` instead of the remainder.
const TBL_FLAG: u32 = TBL_BASE + 0xA98;
/// Float: the input value (split into whole and fraction).
const TBL_INPUT: u32 = TBL_BASE + 0xAA8;
/// Float: the unit weight the fraction is subtracted from.
const TBL_UNIT: u32 = TBL_BASE + 0xAAC;

/// MIPS register numbers of the registers the queue code leaves behind.
const AT: usize = 1;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;

/// Sets a 32-bit GPR value, sign-extended like a MIPS 32-bit op.
fn set_gpr(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// The registers the queue code writes (a0..a2, t0 may differ per path; the rest are fixed).
struct Regs {
    a0: i32,
    a1: i32,
    a2: i32,
    t0: u32,
}

/// Appends slot `id` at queue index `idx`, sets the length to `idx + 1` and leaves retail's registers behind;
/// returns the address of the new last byte (v0).
fn append(c: &mut Cpu, idx: i32, id: u32, r: &Regs) -> u32 {
    let addr = QUEUE.wrapping_add(idx as u32);
    c.w8(addr, id as u8);
    c.w32(QUEUE_LEN, (idx + 1) as u32);
    set_gpr(c, A0, r.a0 as u32);
    set_gpr(c, A1, r.a1 as u32);
    set_gpr(c, A2, r.a2 as u32);
    set_gpr(c, T0, r.t0);
    set_gpr(c, T1, QUEUE_PAGE);
    set_gpr(c, AT, LEN_PAGE);
    set_gpr(c, A3, id);
    set_gpr(c, V1, (idx + 1) as u32);
    addr
}

/// 0x1f7c08 queue_touch(key) -> slot address or -1: looks the key up (retail 0x1f7ba0); -1 when it has no slot.
/// Otherwise the slot id is taken out of the first `count` queued bytes when it is among them (the bytes after it
/// shift left) and appended at the end of the queue; the address of the new last byte is returned.
pub fn queue_touch(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let key = c.arg(0) as i16 as i32 as u32;
    let (id, _) = h::call_keep(c, SLOT_LOOKUP, &[key, 0, 0], &[]);
    if id == u32::MAX {
        set_gpr(c, A3, id);
        c.ret(id);
        return true;
    }
    let mut r = Regs {
        a0: c.g(A0) as u32 as i32,
        a1: 0,
        a2: c.g(A2) as u32 as i32,
        t0: c.g(T0) as u32,
    };
    // (index, bound) of the id's old place; it is removed only when index < bound.
    let victim = if c.r8(QUEUE) as u32 == id {
        // Retail's `lw a0, count(gp)` sits in the beq's delay slot, so the count is loaded on this path too.
        let gp = c.g(28) as u32;
        r.a0 = c.r32(gp.wrapping_add(QUEUE_COUNT_OFF as u32)) as i32;
        Some((0, r.a0))
    } else {
        let gp = c.g(28) as u32;
        r.a0 = c.r32(gp.wrapping_add(QUEUE_COUNT_OFF as u32)) as i32;
        if r.a0 <= 0 {
            let addr_idx = r.a0;
            append(c, addr_idx, id, &r);
            c.ret(QUEUE.wrapping_add(addr_idx as u32));
            return true;
        }
        r.t0 = QUEUE;
        r.a2 = r.a0;
        let mut i = 1;
        let hit = loop {
            if c.r8(QUEUE.wrapping_add(i as u32)) as u32 == id {
                break true;
            }
            if i >= r.a0 {
                break false;
            }
            i += 1;
        };
        r.a1 = i;
        if hit && i < r.a0 { Some((i, r.a0)) } else { None }
    };
    let mut len = c.r32(QUEUE_LEN) as i32;
    if let Some((pos, bound)) = victim {
        if pos < bound {
            if pos < bound - 1 {
                // Shift the bytes after the old place one step left (retail's loop leaves a0, a1, a2 there).
                for k in pos..bound - 1 {
                    let v = c.r8(QUEUE.wrapping_add(k as u32 + 1));
                    c.w8(QUEUE.wrapping_add(k as u32), v);
                }
                r.a2 = bound - 1;
                r.a0 = QUEUE.wrapping_add((bound - 1) as u32) as i32;
                r.a1 = bound - 1;
            }
            // Clear the vacated last byte and shorten the queue; the id is appended after it.
            c.w8(QUEUE.wrapping_add((bound - 1) as u32), 0);
            len -= 1;
            c.w32(QUEUE_LEN, len as u32);
        }
    }
    let addr = append(c, len, id, &r);
    c.ret(addr);
    true
}

/// 0x1fc0c0 table_lerp(table, n) -> float: splits the input into whole and fraction (mathx::float_to_int and
/// split_frac); blends table entry `whole` with entry `(whole + n - 1) % n` (or `n - 1` when the flag bit is set),
/// weighted by the fraction and by one minus it. Declines for n == 0 (retail divides by zero).
pub fn table_lerp(c: &mut Cpu) -> bool {
    let table = c.arg(0);
    let n = c.arg(1);
    if n == 0 {
        return false;
    }
    let saves = [(18, 0x30, h::SQ), (31, 0x60, h::SQ), (20, 0x50, h::SQ), (19, 0x40, h::SQ), (17, 0x20, h::SQ),
        (16, 0x10, h::SQ)];
    h::frame(c, 0x70, &saves, |c, sp| {
        let input = c.rf(TBL_INPUT);
        let (whole, _) = h::run(c, mathx::float_to_int, &[], &[input]);
        let last = n.wrapping_sub(1);
        let sum = whole.wrapping_add(last);
        let rem = (sum as i32).wrapping_rem(n as i32) as u32;
        let flag = c.r32(TBL_FLAG) & 2;
        let input = c.rf(TBL_INPUT);
        let (_, frac) = h::run(c, mathx::split_frac, &[sp], &[input]);
        let unit = c.rf(TBL_UNIT);
        let wrapped = if flag != 0 { last } else { rem };
        let hi = c.rf(table.wrapping_add(whole << 2));
        let lo = c.rf(table.wrapping_add(wrapped << 2));
        let blend = (unit - frac) * lo;
        c.fret(frac * hi + blend);
    });
    true
}
