//! Pause-menu HUD slot rows (Eudora overlay): fills 14 slot rows from a byte key table and 76-byte records, and a
//! small object-pair helper that calls the retail routine at 0x1f6468.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const RA: usize = 31;
const S0: usize = 16;
const S1: usize = 17;

/// s0 at entry: byte table indexed by a record index (zero = slot unused).
const KEY_TABLE: u32 = 0x13_D4C0;
/// s1 at entry: 76-byte records (u16 at +0x38, i16 at +0x40 and +0x44).
const RECORDS: u32 = 0x17_9BC0;
const RECORD_SIZE: i32 = 0x4c;
const REC_U16_38: u32 = 0x38;
const REC_I16_40: u32 = 0x40;
const REC_I16_44: u32 = 0x44;

/// Halfword index sources: slots 0-5 read at SRC_A, 6-8 at SRC_B, 9-11 at SRC_T6, 12-13 at SRC_C (10 bytes per slot).
const SRC_A: u32 = 0x1B_2E26;
const SRC_B: u32 = 0x1B_2E2A;
const SRC_C: u32 = 0x1B_2E2E;
const SRC_T6: u32 = 0x1B_2E2C;
const SRC_STRIDE: u32 = 10;
const SLOTS: u32 = 14;

/// Output: 10-byte slot rows (u16 flags at +0, zero at +2 and +4, index at +6, slot at +8), then two u32 arrays.
const ROWS: u32 = 0x1B_A3B8;
const WORDS_A: u32 = 0x1B_A448;
const WORDS_B: u32 = 0x1B_A480;
/// Used-slot count is stored at COUNT_BASE + 0x40.
const COUNT_BASE: u32 = 0x1B_5288;

/// Entry of the object-pair helper: retail routine called with two sums.
const PAIR_CALLEE: u32 = 0x1F_6468;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x271f50 fill_hud_slot_rows() -> 0: fills the 14 pause-menu slot rows for every slot whose key byte is set and
/// stores the used-slot count.
pub fn fill_hud_slot_rows(c: &mut Cpu) -> bool {
    h::frame(c, 0x20, &[(S0, 0x00, h::SQ), (S1, 0x10, h::SQ)], |c, _sp| {
        let mut row = ROWS;
        let mut word_a = WORDS_A;
        let mut word_b = WORDS_B;
        let mut count = 0u32;
        // Scratch values retail leaves behind when no slot is used.
        let (mut a0, mut a1) = (0x1B_2EA0u32, 0x1B_2E80u32);
        let mut t1 = 0u32;
        for slot in 0..SLOTS {
            let base = match slot {
                0..=5 => SRC_A,
                6..=8 => SRC_B,
                9..=11 => SRC_T6,
                _ => SRC_C,
            };
            let idx = c.r16(base + SRC_STRIDE * slot) as i16 as i32;
            t1 = idx as u32;
            let key = c.r8((KEY_TABLE as i32).wrapping_add(idx) as u32);
            if key != 0 {
                count += 1;
                let rec = RECORDS.wrapping_add(idx.wrapping_mul(RECORD_SIZE) as u32);
                let flags = c.r16(rec + REC_U16_38);
                a1 = c.r16(rec + REC_I16_44) as i16 as i32 as u32;
                a0 = c.r16(rec + REC_I16_40) as i16 as i32 as u32;
                c.w16(row, flags);
                c.w16(row + 2, 0);
                c.w16(row + 4, 0);
                c.w16(row + 6, idx as u16);
                c.w16(row + 8, slot as u16);
                c.w32(word_a, a0);
                c.w32(word_b, a1);
                row += 10;
                word_a += 4;
                word_b += 4;
            }
        }
        c.w32(COUNT_BASE + 0x40, count);

        // Registers as retail leaves them after the loop.
        c.ret(0);
        set_gpr(c, 3, COUNT_BASE); // v1
        set_gpr(c, 4, a0); // a0
        set_gpr(c, 5, a1); // a1
        set_gpr(c, 6, SRC_B + 10 * SLOTS); // a2
        set_gpr(c, 7, SRC_A + 10 * SLOTS); // a3
        set_gpr(c, 8, word_a); // t0
        set_gpr(c, 9, t1); // t1
        set_gpr(c, 10, SLOTS); // t2
        set_gpr(c, 11, row + 6); // t3
        set_gpr(c, 12, row); // t4
        set_gpr(c, 13, word_b); // t5
        set_gpr(c, 14, SRC_T6 + 10 * SLOTS); // t6
        set_gpr(c, 15, SRC_C + 10 * SLOTS); // t7
        set_gpr(c, 24, count); // t8
        set_gpr(c, 25, RECORD_SIZE as u32); // t9
    });
    true
}

/// 0x2721c0 slot_pair_setup(obj) -> 2: calls 0x1f6468 with the two sums of the object's word pairs, returns 2.
pub fn slot_pair_setup(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let a0 = c.r32(obj + 0x18);
    let a2 = c.r32(obj + 0x1C);
    let a1 = c.r32(obj + 0x20);
    let a3 = c.r32(obj + 0x24);
    h::frame(c, 0x10, &[(RA, 0x00, h::SQ)], |c, _sp| {
        h::call_keep(c, PAIR_CALLEE, &[a0, a0.wrapping_add(a1), a2, a2.wrapping_add(a3)], &[]);
    });
    c.ret(2);
    true
}
