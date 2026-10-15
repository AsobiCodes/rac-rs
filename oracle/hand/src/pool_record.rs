//! Pool chunk allocation with a record table (retail 0x284478..0x284558).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Word at 0x1c13a0: start of the chunk pool.
const POOL_BASE: u32 = 0x001c_13a0;
/// Word at 0x1c13a4: end of the chunk pool.
const POOL_LIMIT: u32 = 0x001c_13a4;
/// Word at 0x1c13d0: address of the next free chunk.
const NEXT_FREE: u32 = 0x001c_13d0;
/// Word at 0x1c13d4: number of records in use.
const RECORD_COUNT: u32 = 0x001c_13d4;
/// Record table at 0x1c13d8, 16 bytes per record: {chunk address, size in 16-byte units, tag, unused}.
const RECORDS: u32 = 0x001c_13d8;
/// The record table holds at most this many records.
const MAX_RECORDS: u32 = 0x40;
/// Retail helper (0x118b20) that takes a 16-byte request block and a count of 1.
const REQUEST_HELPER: u32 = 0x0011_8b20;
/// Return value when the pool has no room for the chunk (addiu v0, zero, -1).
const NO_ROOM: u64 = u64::MAX;
/// Return value when the record table is full (addiu v0, zero, -2).
const TABLE_FULL: u64 = u64::MAX - 1;
/// Bytes per record and per 16-byte unit.
const UNIT: u32 = 16;
/// MIPS register numbers the tail writes.
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;

/// Sets a 32-bit value in a register, sign-extended as the 64-bit register holds it.
fn set_reg(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x284478 pool_record_push(owner, units_shift, units, tag) -> index: reserves `units` 16-byte units at the end of the
/// chunk pool, appends {chunk, units, tag} to the record table and returns the new record's index (-1 pool full,
/// -2 table full). The request block passed to 0x118b20 holds owner, the chunk address, units_shift * 16, and 0.
pub fn push(c: &mut Cpu) -> bool {
    let owner = c.arg(0) as u32;
    let units_shift = c.arg(1) as u32;
    let units = c.arg(2) as u32;
    let tag = c.arg(3) as u32;
    let bytes = units.wrapping_mul(UNIT);
    let saves = [
        (16, 0x10, h::SQ),
        (17, 0x20, h::SQ),
        (18, 0x30, h::SQ),
        (19, 0x40, h::SQ),
        (31, 0x50, h::SQ),
    ];
    let result = h::frame(c, 0x60, &saves, |c, sp| {
        let used = c.r32(NEXT_FREE).wrapping_sub(c.r32(POOL_BASE));
        let room = c.r32(POOL_LIMIT).wrapping_sub(used) as i32;
        // Early returns leave the scratch registers as the retail prefix does (v1, a2).
        if room < bytes as i32 {
            set_reg(c, V1, used);
            let next = c.r32(NEXT_FREE);
            set_reg(c, A2, next);
            return NO_ROOM;
        }
        if c.r32(RECORD_COUNT) == MAX_RECORDS {
            set_reg(c, V1, MAX_RECORDS);
            let next = c.r32(NEXT_FREE);
            set_reg(c, A2, next);
            return TABLE_FULL;
        }
        let head = c.r32(NEXT_FREE);
        c.w32(sp, owner);
        c.w32(sp + 4, head);
        c.w32(sp + 8, units_shift.wrapping_mul(UNIT));
        c.w32(sp + 12, 0);
        h::call_keep(c, REQUEST_HELPER, &[sp, 1], &[]);
        let index = c.r32(RECORD_COUNT);
        let head = c.r32(NEXT_FREE);
        let record = RECORDS + index * UNIT;
        c.w32(record, head);
        c.w32(record + 4, units);
        c.w32(record + 8, tag);
        c.w32(NEXT_FREE, head.wrapping_add(bytes));
        c.w32(RECORD_COUNT, index + 1);
        // Scratch registers as the retail tail leaves them.
        set_reg(c, V1, record);
        set_reg(c, A0, index << 4);
        set_reg(c, A1, record);
        set_reg(c, A2, record);
        set_reg(c, A3, index + 1);
        set_reg(c, T0, head);
        set_reg(c, T1, head.wrapping_add(bytes));
        index as u64
    });
    c.ret(result as u32);
    true
}
