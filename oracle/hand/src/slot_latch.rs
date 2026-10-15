//! Slot block at 0x1516d0: `probe_latch` runs a probe and latches a halfword, `store_mark` stores a value through a
//! slot pointer and marks its halfword, and `find_index` searches a range table for a key.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// The slot block (retail s0 in `probe_latch`, the base of the halfwords used by `store_mark`).
const SLOT_BASE: u32 = 0x0015_16d0;
/// The lui value retail leaves in v0 before the probe call (SLOT_BASE minus 0x16d0).
const SLOT_HI: u32 = 0x0015_0000;
/// Probe state halfword inside the slot block (+8).
const SLOT_STATE: u32 = SLOT_BASE + 8;
/// Retail probe called by `probe_latch`: returns nonzero when the probe succeeds.
const PROBE: u32 = 0x0012_eef0;
/// Retail helper called by `store_mark` with three halfwords from the slot block.
const HELPER: u32 = 0x0025_7278;
/// Mode word read by `find_index` (lw at 0x160000 - 0x127c).
const MODE_ADDR: u32 = 0x0016_0000 - 0x127c;
/// Range table: pairs of start/end indices, one pair per mode (lui 0x1c + 0x4538).
const RANGE_TABLE: u32 = 0x001c_4538;
/// Item table: one word per index (lui 0x18 - 0x6d48).
const ITEM_TABLE: u32 = 0x0018_0000 - 0x6d48;
/// Modes at or above this value have no range: `find_index` returns 0.
const MODE_COUNT: u32 = 0x13;

/// Sign-extends a 32-bit value to the 64-bit register form.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Sign-extends a halfword to the 32-bit value a MIPS `lh` produces.
fn sx16(v: u16) -> u32 {
    v as i16 as i32 as u32
}

/// 0x2582e8 probe_latch(mode) -> 2 or 1: for mode 1 clears the slot's state halfword, calls the probe 0x12eef0 and
/// sets the halfword to 2 when the probe returns nonzero; returns 2 for mode 1 and 1 for any other mode.
pub fn probe_latch(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let mode = c.arg(0);
    h::frame(c, 0x20, &[(31, 0x10, h::SQ), (16, 0x0, h::SQ)], |c, _sp| {
        if mode != 1 {
            c.ret(1);
            return;
        }
        c.s(2, sx(SLOT_HI));
        c.s(16, sx(SLOT_BASE));
        c.w16(SLOT_STATE, 0);
        let (probe, _) = h::call_keep(c, PROBE, &[], &[]);
        if probe != 0 {
            c.w16(SLOT_STATE, 2);
        }
        c.ret(2);
    });
    true
}

/// 0x258358 store_mark(value, slot) -> 2, 1 or the helper's result: when slot is null, does nothing. Otherwise stores
/// value at *slot; with value 0 calls 0x257278 with the three halfwords of the slot block and returns its result;
/// else when the halfword at slot+0xa is 1 sets it to 2 and returns 2, and returns 1 for any other halfword.
pub fn store_mark(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let value = c.arg(0);
    let slot = c.arg(1);
    if slot == 0 {
        // Retail's branch delay slot stores ra at sp+0 before the early exit, and the exit reloads it: ra is unchanged.
        let sp = (c.g(29) as u32).wrapping_sub(0x10);
        c.mem.wr128(sp, c.r[31]);
        return true;
    }
    h::frame(c, 0x10, &[(31, 0x0, h::SQ)], |c, _sp| {
        c.w32(slot, value);
        if value == 0 {
            let a0 = sx16(c.r16(SLOT_BASE + 0x54));
            let a2 = sx16(c.r16(SLOT_BASE + 0x56));
            let a1 = sx16(c.r16(SLOT_BASE + 0x58));
            c.s(2, sx(SLOT_BASE));
            h::call_keep(c, HELPER, &[a0, a1, a2], &[]);
            return;
        }
        let flag = c.r16(slot + 0xa) as i16;
        c.s(3, flag as i64 as u64);
        if flag == 1 {
            c.w16(slot + 0xa, 2);
            c.ret(2);
        } else {
            c.ret(1);
        }
    });
    true
}

/// 0x258950 find_index(key) -> index, 0 or -1: with the mode word below 0x13, searches the item table over the mode's
/// [start, end) range for key and returns its index, or -1 when absent; returns 0 for a mode of 0x13 or more.
pub fn find_index(c: &mut Cpu) -> bool {
    let key = c.g(4);
    c.s(6, key);
    let mode = c.r32(MODE_ADDR);
    if mode >= MODE_COUNT {
        // Retail's lw leaves the mode word in v1.
        c.s(3, sx(mode));
        c.ret(0);
        return true;
    }
    let range = RANGE_TABLE.wrapping_add(mode.wrapping_mul(4));
    let start = c.r32(range) as i32;
    let end = c.r32(range.wrapping_add(4)) as i32;
    c.s(3, sx(range));
    c.s(5, sx(end as u32));
    if start >= end {
        c.s(4, sx(start as u32));
        c.ret(u32::MAX);
        return true;
    }
    let mut i = start;
    while i < end {
        let item_addr = ITEM_TABLE.wrapping_add((i as u32).wrapping_mul(4));
        let item = c.r32(item_addr);
        c.s(3, sx(item_addr));
        if sx(item) == key {
            c.s(4, sx(i as u32));
            c.ret(i as u32);
            return true;
        }
        i += 1;
    }
    c.s(3, sx(ITEM_TABLE.wrapping_add((i as u32).wrapping_mul(4))));
    c.s(4, sx(i as u32));
    c.ret(u32::MAX);
    true
}
