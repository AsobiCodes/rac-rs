//! Slot table helpers: append a record (0x2380b8), find a free slot (0x238fb8), allocate a slot (0x239020).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Base of the slot tables (lui 0x18 + 0x42f0).
const SLOT_BASE: u32 = 0x0018_42F0;
/// Flag word A: bit 0x1000 marks a busy group.
const FLAGS_A: u32 = SLOT_BASE + 0x290;
/// Flag word B: nonzero when a second group is available.
const FLAGS_B: u32 = SLOT_BASE + 0x27C;
/// Table A (entry i at TBL_A + 4 * i).
const TBL_A: u32 = SLOT_BASE + 0x278;
/// Table B (entry i at TBL_B + 4 * i).
const TBL_B: u32 = SLOT_BASE + 0x28C;
/// Busy bit tested in the flag words.
const BUSY_BIT: u32 = 0x1000;
/// Record counter word (0x15f418).
const RECORD_COUNT: u32 = 0x0015_F418;
/// Record table base (0x16ce80), 16 bytes per record.
const RECORD_BASE: u32 = 0x0016_CE80;
/// Frame size of alloc_slot.
const FRAME: u32 = 0x20;
/// Retail helper called by alloc_slot when no slot is free.
const SLOT_HELPER: u32 = 0x0023_90C0;

/// Splits a 128-bit register into four 32-bit lanes (lane 0 = low bits).
fn quad(x: u128) -> V {
    V([x as u32, (x >> 32) as u32, (x >> 64) as u32, (x >> 96) as u32])
}

/// Sign-extends a 32-bit value to a 64-bit register value.
fn sx(x: u32) -> u64 {
    x as i32 as i64 as u64
}

/// 0x2380b8 log_append(a0 key, a1 kind, a2 w0, a3 w1, t0 hi, t1 lo) -> handle: appends a 16-byte record while the
/// counter is below 64 and returns the packed handle (0 is never stored; the handle is returned as retail does).
pub fn log_append(c: &mut Cpu) -> bool {
    let key = c.g(4);
    let kind = c.g(5);
    let w0 = c.g(6) as u32;
    let w1 = c.g(7) as u32;
    let t0s = (c.g(8) as u32 as i32) >> 8;
    let t1s = (c.g(9) as u32 as i32) >> 8;
    let pos = (key as u32 as i32).wrapping_sub(6);
    let shift = if pos >= 0 { pos as u32 & 31 } else { 0 };
    let bit = 1u32 << shift;
    let mut v0 = ((bit as i32 as i64 as u64) << 14) | (t1s as i64 as u64);
    v0 |= (key << 26) | 0x0130_0000;
    v0 |= kind << 30;
    let a0_y = ((t0s as i64 as u64) << 37) | (1u64 << 34);
    v0 |= a0_y;
    let count = sx(c.r32(RECORD_COUNT));
    let v1_flag = 1u64 << 63;
    let ret = v0 | v1_flag;
    let mut a0_out = a0_y;
    let mut v1_out = v1_flag;
    let mut a1_out = count;
    if (count as i64) < 0x40 {
        let idx = (count as u32) << 4;
        let base = RECORD_BASE.wrapping_add(idx);
        c.w32(base, w0);
        c.w16(base + 4, 0);
        c.w16(base + 6, t0s as u16);
        c.w32(base + 8, w1);
        c.w8(base + 0xC, key as u8);
        c.w8(base + 0xD, kind as u8);
        c.w16(base + 0xE, t1s as u16);
        let n = (count as u32).wrapping_add(1);
        c.w32(RECORD_COUNT, n);
        a0_out = sx(idx);
        v1_out = sx(base);
        a1_out = sx(n);
    }
    c.s(1, 0x0016_0000);
    c.s(2, ret);
    c.s(3, v1_out);
    c.s(4, a0_out);
    c.s(5, a1_out);
    c.s(8, t0s as i64 as u64);
    c.s(9, t1s as i64 as u64);
    c.s(10, ret);
    c.s(11, key);
    c.s(12, kind);
    true
}

/// 0x238fb8 find_free_slot(a0 flag) -> index: scans up to 5 entries (index n, or 4 - n when a0 is nonzero) and
/// returns the first index whose table A entry is set and table B entry is -1, or -1 when none matches.
pub fn find_free_slot(c: &mut Cpu) -> bool {
    let zero_first = c.g(4) == 0;
    let mut n: u32 = 0;
    let mut a1: u32;
    let mut a2: u32;
    let mut v1: u32;
    let ret: u32;
    loop {
        a1 = if zero_first { n } else { 4u32.wrapping_sub(n) };
        a2 = a1 << 2;
        v1 = c.r32(TBL_A + a2);
        let mut hit = false;
        if v1 != 0 {
            v1 = c.r32(TBL_B + a2);
            hit = v1 == u32::MAX;
        }
        n += 1;
        if hit {
            ret = a1;
            break;
        }
        if n >= 5 {
            ret = u32::MAX;
            a1 = 4u32.wrapping_sub(n);
            break;
        }
    }
    c.s(2, sx(ret));
    c.s(3, sx(v1));
    c.s(5, sx(a1));
    c.s(6, sx(a2));
    c.s(7, sx(n));
    c.s(8, sx(TBL_A));
    c.s(9, u64::MAX);
    c.s(10, 4);
    c.s(11, sx(TBL_B));
    true
}

/// 0x239020 alloc_slot() -> index: returns find_free_slot(1) when it finds one; otherwise scans the flag words
/// from index 2 and calls retail 0x2390c0(0, index), then returns the index.
pub fn alloc_slot(c: &mut Cpu) -> bool {
    let sp = c.g(29) as u32;
    let frame = sp.wrapping_sub(FRAME);
    let s0q = quad(c.r[16]);
    let raq = quad(c.r[31]);
    c.wv(frame, s0q);
    c.wv(frame + 0x10, raq);
    c.s(29, frame as u64);
    c.s(4, 1);
    find_free_slot(c);
    if c.g(2) != 0 {
        c.s(29, sp as u64);
        return true;
    }
    let a2: u32 = 0x0018_0000;
    let mut v1: u32 = SLOT_BASE;
    let mut v0 = c.r32(FLAGS_A) & BUSY_BIT;
    let mut s0: u32 = 1;
    let mut call_now = false;
    if v0 == 0 {
        v0 = c.r32(FLAGS_B);
        call_now = v0 != 0;
    }
    if !call_now {
        s0 = 2;
        loop {
            v0 = (s0 < 5) as u32;
            if v0 == 0 {
                break;
            }
            v1 = SLOT_BASE.wrapping_add(s0 << 2);
            v0 = c.r32(v1 + 0x28C) & BUSY_BIT;
            if v0 != 0 {
                s0 += 1;
                continue;
            }
            v0 = v1;
            v1 = c.r32(v0 + 0x278);
            if v1 == 0 {
                s0 += 1;
                continue;
            }
            break;
        }
    }
    c.s(2, v0 as u64);
    c.s(3, v1 as u64);
    c.s(6, a2 as u64);
    h::call_keep(c, SLOT_HELPER, &[0, s0], &[]);
    c.s(2, s0 as u64);
    c.s(29, sp as u64);
    true
}
