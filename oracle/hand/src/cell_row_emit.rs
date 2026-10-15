//! Hand versions of two level-4 helpers: the row walker over the cell table (retail 0x1ec660, which issues GS register
//! writes through vif_emit::gs_set_reg and subdivides runs through 0x1ec948) and a 12-byte record clear (0x1f17e8).
//! Retail leaves its scratch registers (v0, v1, t0, a0..a3) as its last instructions set them; the code below keeps
//! every one of those writes so the state after the return matches.
use crate::h;
use crate::ps2::*;
use crate::vif_emit;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const GP: usize = 28;

/// Retail subdivision routine: a0..a3 = run start / length / 0 / step, t0 = flag word.
const SUBCELL: u32 = 0x1ec948;
/// First operand of the GS helper (the A+D register address).
const GS_REG_AD: u64 = 0x42;
/// Mask for the 64-bit GS value: 0xff00 << 24 | 0xff.
const SEL_MASK: u64 = 0x0000_00ff_0000_00ff;
/// The sign-extended `lui 0xff00` mask tested against the flag words.
const TOP_MASK: u64 = 0xffff_ffff_ff00_0000;
/// gp-relative slot holding the pointer to the cell record.
const RECORD_PTR_OFF: u32 = 0x78f0;
/// Cell table in the level data: the row count is a halfword at +0x152, the step at +0x150.
const TABLE: u32 = 0x0015_1780;
const COUNT_OFF: u32 = 0x152;
const STEP_OFF: u32 = 0x150;
/// Frame size and callee-saved slots of the row walker.
const FRAME: u32 = 0x60;
const SAVES: [(usize, u32, u8); 6] = [
    (16, 0x00, h::SQ),
    (17, 0x10, h::SQ),
    (18, 0x20, h::SQ),
    (19, 0x30, h::SQ),
    (20, 0x40, h::SQ),
    (31, 0x50, h::SQ),
];

/// The cell record pointer (lw from gp - 0x78f0).
fn record(c: &mut Cpu) -> u32 {
    let gp = c.g(GP) as u32;
    c.r32(gp.wrapping_sub(RECORD_PTR_OFF))
}

/// Re-reads the record pointer into v1, as retail does after each call.
fn reload_v1(c: &mut Cpu) -> u32 {
    let rec = record(c);
    c.s(V1, rec as i32 as i64 as u64);
    rec
}

/// lw: 32-bit load, sign-extended to 64 bits.
fn lw(c: &mut Cpu, a: u32) -> u64 {
    c.r32(a) as i32 as i64 as u64
}

/// ld: 64-bit load.
fn ld(c: &mut Cpu, a: u32) -> u64 {
    c.r32(a) as u64 | (c.r32(a.wrapping_add(4)) as u64) << 32
}

/// lh: 16-bit load, sign-extended.
fn lh(c: &mut Cpu, a: u32) -> u64 {
    c.r16(a) as i16 as i64 as u64
}

/// GS write through the hand helper: a0 = 0x42, a1 = value & SEL_MASK. The helper's scratch registers stay as left.
fn gs_write(c: &mut Cpu, value: u64) {
    c.s(A0, GS_REG_AD);
    c.s(A1, value & SEL_MASK);
    h::run_keep(c, vif_emit::gs_set_reg, &[], &[]);
}

/// A 64-bit record slot that is zero or GS-written: a1 = slot & SEL_MASK on every path, the write only when set.
fn sel_write(c: &mut Cpu, raw: u64) {
    if raw != 0 {
        gs_write(c, raw);
    } else {
        c.s(A1, 0);
    }
}

/// Subdivides the run of cells from `s0` with the length in the record slot `len_off`, when the flag word at
/// `flag_off` has its top byte set (sign-extended). The flag word goes to t0 as its zero-extended low half.
fn run_block(c: &mut Cpu, rec: u32, flag_off: u32, len_off: u32, s0: i32, count: i32) {
    let flag = lw(c, rec.wrapping_add(flag_off));
    let top = flag & TOP_MASK;
    c.s(V0, top);
    // branch delay slot: dsll32 always runs; the call path shifts it back below
    c.s(T0, (flag & 0xffff_ffff) << 32);
    if top == 0 {
        return;
    }
    let len = lw(c, rec.wrapping_add(len_off)) as i32;
    let last = count.wrapping_sub(1);
    let step = lh(c, TABLE + STEP_OFF);
    let start = s0.wrapping_add(len);
    c.s(T0, flag & 0xffff_ffff);
    let before_last = start < last;
    c.s(V0, before_last as u64);
    c.s(A0, s0 as i64 as u64);
    let a1 = if before_last { start } else { last };
    c.s(A1, a1 as i64 as u64);
    c.s(V1, last as i64 as u64);
    c.s(A2, 0);
    c.s(A3, step);
    h::call_keep(c, SUBCELL, &[], &[]);
}

/// 0x1ec660 cell_row_emit(): walks the cell row described by the gp record: a flagged whole-row run, then per cell a
/// GS write, a run from the current cell and a second run, advancing by the record's strides until the row count.
pub fn cell_row_emit(c: &mut Cpu) -> bool {
    h::frame(c, FRAME, &SAVES, |c, _sp| {
        let count = c.r16(TABLE + COUNT_OFF) as i16 as i32;
        let rec = reload_v1(c);

        // optional GS write from the 64-bit slot at +0x08
        let sel08 = ld(c, rec + 0x08);
        c.s(V0, sel08);
        if sel08 != 0 {
            gs_write(c, sel08);
            reload_v1(c);
        }

        // whole-row run with the flag word at +0x04 (length = the row count)
        let flag = lw(c, rec + 0x04);
        let top = flag & TOP_MASK;
        c.s(V0, top);
        c.s(T0, (flag & 0xffff_ffff) << 32);
        if top != 0 {
            let step = lh(c, TABLE + STEP_OFF);
            c.s(T0, flag & 0xffff_ffff);
            c.s(A0, 0);
            c.s(A1, count as i64 as u64);
            c.s(A2, 0);
            c.s(A3, step);
            h::call_keep(c, SUBCELL, &[], &[]);
        }

        if count > 0 {
            let mut s0: i32 = 0;
            loop {
                let rec = record(c);
                c.s(V0, rec as i32 as i64 as u64);
                let sel18 = ld(c, rec + 0x18);
                sel_write(c, sel18);
                let rec = reload_v1(c);

                run_block(c, rec, 0x14, 0x10, s0, count);
                let rec = reload_v1(c);

                let len10 = lw(c, rec + 0x10);
                c.s(V0, len10);
                s0 = s0.wrapping_add(len10 as i32);
                let sel28 = ld(c, rec + 0x28);
                sel_write(c, sel28);
                let rec = reload_v1(c);

                run_block(c, rec, 0x24, 0x20, s0, count);
                let rec = reload_v1(c);

                let len20 = lw(c, rec + 0x20);
                c.s(V1, len20);
                s0 = s0.wrapping_add(len20 as i32);
                let rec = record(c);
                c.s(V0, rec as i32 as i64 as u64);
                if s0 >= count {
                    break;
                }
            }
        }
    });
    true
}

/// 0x1f17e8 zero_record_12(a0 = record): clears the 64-bit word at +0 and the word at +8.
pub fn zero_record_12(c: &mut Cpu) -> bool {
    let rec = c.arg(0);
    c.w32(rec, 0);
    c.w32(rec.wrapping_add(4), 0);
    c.w32(rec.wrapping_add(8), 0);
    true
}
