//! Key table and record queue helpers: retail 0x276868 (set flag bit 4 on a keyed entry), 0x2768b0 (clear it),
//! 0x277068 (push a 56-byte record onto an 8-slot queue) and 0x277100 (drop the queue head, shifting the rest down).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const AT: usize = 1;
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
const GP: usize = 28;
const SP: usize = 29;

/// Key table: five 8-byte entries, a key word followed by a flags word.
const KEY_TABLE: u32 = 0x1b_a2b8;
const KEY_COUNT: u32 = 5;
const FLAG_BIT: u32 = 0x4;
/// flag_clear mask (-5): every bit except bit 2.
const CLEAR_MASK: i32 = -5;

/// Record queue: 56-byte records from QUEUE_BASE, the live count is a word at QUEUE_LEN_WORD.
const QUEUE_BASE: u32 = 0x1b_a0c0;
const RECORD_SIZE: u32 = 0x38;
const QUEUE_SLOTS: i32 = 8;
/// Address of the last slot, where the retail copy loop stops.
const QUEUE_LAST: u32 = QUEUE_BASE + 7 * RECORD_SIZE;
const QUEUE_LEN_WORD: u32 = 0x16_0390;
/// gp-relative word that receives the new count in queue_pop.
const GP_COUNT_OFF: u32 = 0x6870;
/// Upper half of the global block address (lui at / lui t4 / lui v1, 0x16).
const GLOBALS_HI: u32 = 0x16_0000;

/// Sign-extends a 32-bit value into a 64-bit register, as the MIPS 32-bit ops do.
fn set_s(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Replaces the low 64 bits of a register and keeps the upper half (ldl/ldr behaviour).
fn set_low64(c: &mut Cpu, reg: usize, v: u64) {
    c.r[reg] = (c.r[reg] & !(u64::MAX as u128)) | v as u128;
}

fn read64(c: &mut Cpu, addr: u32) -> u64 {
    c.r32(addr) as u64 | (c.r32(addr.wrapping_add(4)) as u64) << 32
}

fn write64(c: &mut Cpu, addr: u32, v: u64) {
    c.w32(addr, v as u32);
    c.w32(addr.wrapping_add(4), (v >> 32) as u32);
}

/// Walks the key table the way retail does: entry k (1-based) is at KEY_TABLE + 8*(k-1). Leaves a1 and a2 as
/// retail does and returns the flags address of the first entry whose key is `key`.
fn walk_keys(c: &mut Cpu, key: u32) -> Option<u32> {
    for k in 1..=KEY_COUNT {
        let entry = KEY_TABLE + 8 * (k - 1);
        if c.r32(entry) == key {
            set_s(c, A2, k);
            set_s(c, A1, entry + 4);
            return Some(entry + 4);
        }
    }
    set_s(c, A2, KEY_COUNT);
    set_s(c, A1, KEY_TABLE + 4 + 8 * KEY_COUNT);
    None
}

/// Shared body of flag_set and flag_clear: `op` maps the old flags word to the new one.
fn flag_op(c: &mut Cpu, op: fn(u32) -> u32) -> bool {
    let key = c.r[A0] as u32;
    match walk_keys(c, key) {
        Some(flags) => {
            let v = op(c.r32(flags));
            c.w32(flags, v);
            set_s(c, V1, v);
            set_s(c, V0, 0);
        }
        None => set_s(c, V0, 1),
    }
    true
}

/// 0x276868 flag_set(key a0) -> v0: finds key in the 5-entry table, sets flag bit 4 on it; v0 = 0 if found, else 1
pub fn flag_set(c: &mut Cpu) -> bool {
    flag_op(c, |flags| flags | FLAG_BIT)
}

/// 0x2768b0 flag_clear(key a0) -> v0: finds key in the 5-entry table, clears flag bit 4 on it; v0 = 0 if found, else 1
pub fn flag_clear(c: &mut Cpu) -> bool {
    set_s(c, A3, CLEAR_MASK as u32);
    flag_op(c, |flags| flags & CLEAR_MASK as u32)
}

/// 0x277068 queue_push(a0..a3, t0..t3, 0x10..0x18(sp)) -> v0: appends a 56-byte record, v0 = 0, or -1 when full
pub fn queue_push(c: &mut Cpu) -> bool {
    let len = c.r32(QUEUE_LEN_WORD) as i32;
    set_s(c, T4, len as u32);
    let (a0, a1) = (c.r[A0], c.r[A1]);
    set_s(c, T5, a0 as u32);
    set_s(c, T6, a1 as u32);
    if len >= QUEUE_SLOTS {
        set_s(c, V0, -1i32 as u32);
        return true;
    }
    let sp = c.r[SP] as u32;
    let record = QUEUE_BASE.wrapping_add((len as u32).wrapping_mul(RECORD_SIZE));
    let args = [
        c.r32(sp.wrapping_add(0x20)),
        c.r32(sp),
        c.r32(sp.wrapping_add(0x08)),
        c.r32(sp.wrapping_add(0x10)),
        c.r32(sp.wrapping_add(0x18)),
    ];
    c.w32(QUEUE_LEN_WORD, (len + 1) as u32);
    c.w32(record, 0);
    c.w32(record + 0x04, a0 as u32);
    c.w32(record + 0x08, a1 as u32);
    c.w32(record + 0x0c, c.r[A2] as u32);
    c.w32(record + 0x10, c.r[A3] as u32);
    c.w32(record + 0x14, c.r[T0] as u32);
    c.w32(record + 0x18, c.r[T1] as u32);
    c.w32(record + 0x1c, c.r[T2] as u32);
    c.w32(record + 0x20, c.r[T3] as u32);
    c.w32(record + 0x34, args[0]);
    c.w32(record + 0x24, args[1]);
    c.w32(record + 0x28, args[2]);
    c.w32(record + 0x2c, args[3]);
    c.w32(record + 0x30, args[4]);
    set_s(c, AT, GLOBALS_HI);
    set_s(c, A0, args[3]);
    set_s(c, A1, args[4]);
    set_s(c, V1, record);
    set_s(c, V0, 0);
    c.lo = (len as i64 * RECORD_SIZE as i64) as u64 as u128;
    c.hi = 0;
    true
}

/// 0x277100 queue_pop() -> v0: drops the head record by shifting the others down one slot, stores count-1 to
/// gp-0x6870; v0 = 0
pub fn queue_pop(c: &mut Cpu) -> bool {
    let mut rec = QUEUE_BASE;
    let mut last = [0u64; 7];
    while rec < QUEUE_LAST {
        let mut words = [0u64; 7];
        for (i, w) in words.iter_mut().enumerate() {
            *w = read64(c, rec + RECORD_SIZE + 8 * i as u32);
        }
        for (i, w) in words.iter().enumerate() {
            write64(c, rec + 8 * i as u32, *w);
        }
        last = words;
        rec += RECORD_SIZE;
    }
    // Registers as the last loop iteration leaves them (only the low doubleword changes).
    set_s(c, A0, QUEUE_LAST);
    set_low64(c, A1, last[5]);
    set_low64(c, A2, last[6]);
    set_low64(c, A3, last[3]);
    let count = c.r32(QUEUE_LEN_WORD).wrapping_sub(1);
    set_s(c, V1, count);
    let gp = c.r[GP] as u32;
    c.w32(gp.wrapping_sub(GP_COUNT_OFF), count);
    set_s(c, V0, 0);
    true
}
