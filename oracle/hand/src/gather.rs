//! Gathering per-part scratchpad data: the two `gather_*` routines build a unique, ascending list of the part indices
//! used by a set of records, ask retail to load those parts into the scratchpad (0x2469d0), and copy each requested
//! part's data out in request order.
use ee_oracle::cpu::Cpu;
use crate::ps2::*;
use crate::rcall::{restore, save};

/// Scratch work area: 0x80 flag bytes (one per part index), then the list of last indices per request.
const WORK: u32 = 0x19_6f00;
const LIST: u32 = WORK + 0x80;
/// Scratchpad base: 64 bytes per part.
const SPR: u32 = 0x7000_0000;
/// Retail routine that fills the scratchpad parts flagged in WORK.
const LOAD_PARTS_FN: u32 = 0x24_69d0;

/// GPR numbers of the registers retail's code uses as scratch or temporaries here.
const R_AT: usize = 1;
const R_V0: usize = 2;
const R_V1: usize = 3;
const R_A0: usize = 4;
const R_A1: usize = 5;
const R_A2: usize = 6;
const R_A3: usize = 7;
const R_T0: usize = 8;
const R_T6: usize = 14;
const R_T7: usize = 15;
const R_T8: usize = 24;
/// Saved registers and ra, which retail's pcpyud leaves with a zero upper half.
const R_HI_ZERO: [usize; 5] = [20, 21, 22, 23, 31];

/// What retail's scan leaves in registers: the table pointer (t8), a2 and a3 of the last request, and the max index.
struct Scan {
    table: u32,
    count_rem: u32,
    last: u32,
    max: u32,
}

/// 32-bit register write, sign-extended over all 128 bits (as the 32-bit MIPS ops leave the register).
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Upper doubleword as retail's pmaxw leaves it: signed max of each 32-bit word of a and b.
fn pmaxw_hi(a: u64, b: u64) -> u64 {
    let w = |x: u64, sh: u32| ((x >> sh) as u32 as i32).max((b >> sh) as u32 as i32) as u32 as u64;
    w(a, 0) | (w(a, 32) << 32)
}

/// Builds the flag bytes and per-request index list in WORK from the n requests (indices into the table at
/// (*(obj+0x24)+0x1c)); each request names a record whose bytes (count in its halfword) are the part indices.
fn build_part_list(c: &mut Cpu, obj: u32, n: u32, requests: u32) -> Scan {
    let table = {
        let a = c.r32(obj + 0x24);
        c.r32(a + 0x1c)
    };
    for i in 0..8 {
        c.wv(WORK + 0x10 * i, V([0; 4]));
    }
    let mut max = 0u32;
    let mut out = LIST;
    let (mut count_rem, mut last) = (0u32, 0u32);
    for i in 0..n.max(1) {
        let idx = c.r32(requests + 4 * i);
        let rec = c.r32(table.wrapping_add(idx << 2).wrapping_add(4));
        let count = c.r16(rec) as u32;
        let iters = count.max(1);
        count_rem = count.wrapping_sub(iters);
        last = 0;
        for k in 0..iters {
            last = c.r8(rec + 4 + k) as u32;
            c.w8(WORK + last, 1);
        }
        c.w8(out, last as u8);
        out += 1;
        max = max.max(last);
    }
    c.w8(WORK + 0x7f, (max + 1) as u8);
    c.w8(WORK + max, 0xff);
    Scan { table, count_rem, last, max }
}

/// 0x246710 gather_part_rows(obj, n, requests*, out*): after the parts are loaded, out[i] (16 bytes) = row 3 (the
/// qword at +0x30) of each requested part's 64-byte scratchpad record.
pub fn gather_part_rows(c: &mut Cpu) -> bool {
    let (obj, n, requests, out) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let count = n.max(1);
    // t6 (after move t6,zero) keeps its upper half, and pmaxw folds a3's upper half into it.
    let t6_hi = pmaxw_hi((c.r[R_T6] >> 64) as u64, (c.r[R_A3] >> 64) as u64);
    let scan = build_part_list(c, obj, n, requests);
    // Scratch registers as retail's scan leaves them when it calls the loader.
    set32(c, R_AT, 0xff);
    set32(c, R_V0, scan.max + 1);
    set32(c, R_V1, LIST + count);
    set32(c, R_A2, scan.count_rem);
    set32(c, R_A3, scan.last);
    set32(c, R_T0, scan.last + WORK);
    c.r[R_T6] = ((t6_hi as u128) << 64) | (scan.max + WORK) as u128;
    set32(c, R_T7, WORK);
    set32(c, R_T8, scan.table);
    // The loader's own scratch registers stay as it leaves them (call_keep).
    crate::h::call_keep(c, LOAD_PARTS_FN, &[obj, WORK], &[]);
    let mut last_part = 0;
    for i in 0..count {
        let part = c.r8(LIST + i) as u32;
        last_part = part;
        let row = c.rv(SPR + (part << 6) + 0x30);
        c.wv(out + 0x10 * i, row);
    }
    // Retail's final register values of the copy loop.
    set32(c, R_AT, n.wrapping_sub(count));
    set32(c, R_V0, out.wrapping_add(0x10 * count).wrapping_sub(0x10));
    set32(c, R_V1, LIST + count);
    set32(c, R_T6, SPR);
    set32(c, R_T7, WORK);
    set32(c, R_A0, SPR + (last_part << 6));
    c.r[R_A1] = c.mem.rd128(SPR + (last_part << 6) + 0x30);
    for r in R_HI_ZERO {
        c.r[r] &= u64::MAX as u128;
    }
    true
}

/// 0x246868 gather_part_matrices(obj, n, requests*, out*): like gather_part_rows but copies the whole 64-byte
/// record of each requested part to out[i] (64 bytes each).
pub fn gather_part_matrices(c: &mut Cpu) -> bool {
    let (obj, n, requests, out) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let saved = save(c);
    build_part_list(c, obj, n, requests);
    c.set_arg(0, obj);
    c.set_arg(1, WORK);
    c.call(LOAD_PARTS_FN);
    for i in 0..n.max(1) {
        let part = c.r8(LIST + i) as u32;
        for row in 0..4 {
            let q = c.rv(SPR + (part << 6) + 0x10 * row);
            c.wv(out + 0x40 * i + 0x10 * row, q);
        }
    }
    restore(c, &saved);
    true
}
