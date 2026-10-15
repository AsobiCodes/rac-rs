//! Record table walk: `rec_emit_loop` (0x2845d8) emits `count` units of one table record through the emitter at 0x11b1c8
//! in chunks of at most 0xffff units; `rec_word_at` (0x2846f0) is the record lookup it uses when its count is -1.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Table of 64 records of 16 bytes at 0x1c13d8: word 0 is a base address, word 1 a count.
const REC_TABLE: u32 = 0x001c_13d8;
/// First argument of the emitter: a table in the same segment as REC_TABLE.
const EMIT_TABLE: u32 = 0x001c_13a8;
/// 0x11b1c8 (out-of-line emitter, args: table, 1, flag, &cursor, 0x10, dst, bytes, 0; advances the word at &cursor).
const EMIT: u32 = 0x0011_b1c8;
/// Records in the table; an index at or past this is rejected.
const REC_COUNT: u32 = 0x40;
/// Returned for a bad index or an empty record (-3).
const BAD_INDEX: u32 = 0xffff_fffd;
/// Returned when the record is too short for base + count (-1).
const SHORT: u32 = 0xffff_ffff;
/// Largest number of units emitted per call of the emitter.
const CHUNK_MAX: i32 = 0xffff;
/// Frame size retail reserves on entry (`addiu sp, sp, -0x90`).
const FRAME: u32 = 0x90;
/// Stack offset of the cursor word the emitter advances (the callee's &cursor argument).
const CURSOR_OFF: u32 = 0x10;

/// Word 1 (the count) of record `index`, or -3 when index >= 0x40.
fn record_count(c: &mut Cpu, index: u32) -> u32 {
    if index < REC_COUNT {
        c.r32(REC_TABLE + index * 16 + 4)
    } else {
        BAD_INDEX
    }
}

/// 0x2846f0 rec_word_at(index) -> i32: word 1 of record `index` in the table at 0x1c13d8, or -3 when index >= 0x40.
pub fn rec_word_at(c: &mut Cpu) -> bool {
    let index = c.arg(0);
    let v = record_count(c, index);
    c.ret(v);
    true
}

/// 0x2845d8 rec_emit_loop(dst, index, base, count, flag) -> i32: emits `count` units of record `index` starting at
/// `base` through the emitter, advancing `dst`; count -1 means the record's own count. Returns 0 when done, -1 when the
/// record is shorter than base + count, -3 for a bad index or an empty record.
pub fn rec_emit_loop(c: &mut Cpu) -> bool {
    let mut dst = c.arg(0);
    let index = c.arg(1);
    let base = c.arg(2);
    let mut count = c.arg(3);
    let flag = c.arg(4);
    if count == u32::MAX {
        count = record_count(c, index);
    }
    if index >= REC_COUNT {
        c.ret(BAD_INDEX);
        return true;
    }
    let rec = REC_TABLE + index * 16;
    let len = c.r32(rec + 4) as i32;
    if len == 0 {
        c.ret(BAD_INDEX);
        return true;
    }
    if len < base.wrapping_add(count) as i32 {
        c.ret(SHORT);
        return true;
    }
    let sp = (c.g(29) as u32).wrapping_sub(FRAME);
    let cursor_at = sp + CURSOR_OFF;
    let cursor = c.r32(rec).wrapping_add(base << 4);
    c.w32(cursor_at, cursor);
    let mut rem = count as i32;
    while rem != 0 {
        let chunk = if CHUNK_MAX < rem { CHUNK_MAX } else { rem };
        let bytes = (chunk << 4) as u32;
        c.w32(sp, 0);
        rem -= chunk;
        h::call_keep(c, EMIT, &[EMIT_TABLE, 1, flag, cursor_at, 0x10, dst, bytes, 0], &[]);
        dst = dst.wrapping_add(bytes);
        let advanced = c.r32(cursor_at).wrapping_add(bytes);
        c.w32(cursor_at, advanced);
    }
    c.ret(0);
    true
}
