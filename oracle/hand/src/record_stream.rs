//! Record-stream cursor: `next_record` walks the packed record stream (16-bit headers, the low 15 bits give the
//! offset of the next record's base) from a per-type start record, or from the current cursor, until the record
//! at a1 is reached, then loads the record after it and reports its flag byte.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// gp-relative: the type byte of the record (+0x21) must not exceed this signed limit; and the saved start record.
const GP_TYPE_LIMIT: i32 = -0x6c8c;
const GP_START: i32 = -0x6b74;
/// Per-type table of start records (4 bytes per type).
const START_TABLE: u32 = 0x1a_b940;
/// Stream base word: each header adds (value << 8) to it.
const STREAM_BASE: u32 = 0x15_ff58;
/// Stream position, last header value and current record pointer.
const POS: u32 = 0x16_008c;
const HEADER: u32 = 0x16_0090;
const CUR: u32 = 0x16_0094;
/// Record offsets: the type byte, and the flag byte whose sign bit selects the continuation.
const REC_TYPE: u32 = 0x21;
const REC_FLAG: u32 = 0x20;
/// Sign bit of a header halfword: marks the end of the stream.
const HEADER_END: u16 = 0x8000;

/// Reads the header at `np` (sets the stream position and the cursor from it) and returns the new record.
fn take(c: &mut Cpu, base: u32, np: u32) -> u32 {
    c.w32(POS, np);
    let header = c.r16(np) & !HEADER_END;
    c.w16(HEADER, header);
    let cur = base.wrapping_add((header as u32) << 8);
    c.w32(CUR, cur);
    cur
}

fn finish(c: &mut Cpu, v: i64) -> bool {
    c.s(2, v as u64);
    true
}

/// 0x24bbf8 next_record(out, rec, want_a, want_b) -> 0 or -1: stores the record after the cursor into *out (0 when
/// no record follows); the walk resumes from the saved start record when the flag byte of the record allows it.
pub fn next_record(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    let out = c.arg(0);
    let rec = c.arg(1);
    let (want_a, want_b) = (c.arg(2), c.arg(3));
    c.w32(out, 0);
    let base = c.r32(STREAM_BASE);
    let mut t0;
    if rec == c.r32(CUR) {
        let pos = c.r32(POS);
        if (c.r16(pos) as i16) < 0 {
            return finish(c, -1);
        }
        t0 = pos;
    } else {
        let limit = c.r32(gp.wrapping_add(GP_TYPE_LIMIT as u32)) as i32;
        let kind = c.r8(rec.wrapping_add(REC_TYPE)) as i32;
        if limit < kind {
            return finish(c, -1);
        }
        c.w32(CUR, 0);
        let start = c.r32(START_TABLE.wrapping_add((kind as u32).wrapping_mul(4)));
        c.w32(gp.wrapping_add(GP_START as u32), start);
        if start == 0 {
            return finish(c, -1);
        }
        let mut pos = start.wrapping_sub(2);
        loop {
            let np = pos.wrapping_add(2);
            t0 = np;
            let cur = take(c, base, np);
            if (c.r16(np) as i16) < 0 {
                return finish(c, -1);
            }
            if cur == rec {
                break;
            }
            pos = np;
        }
    }
    loop {
        let cur = take(c, base, t0.wrapping_add(2));
        c.w32(out, cur);
        let flag = c.r8(cur.wrapping_add(REC_FLAG)) & 0x80 != 0;
        let resume = if want_a != 0 { want_b != 0 && !flag } else { want_b != 0 || flag };
        if !resume {
            return finish(c, 0);
        }
        let saved = c.r32(gp.wrapping_add(GP_START as u32));
        if (c.r16(saved) as i16) < 0 {
            c.w32(out, 0);
            return finish(c, -1);
        }
        t0 = saved;
    }
}
