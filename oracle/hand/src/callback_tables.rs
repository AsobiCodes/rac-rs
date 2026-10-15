//! Hand versions of two level-4 EE helpers on the callback state block at 0x13e550: a record walker and a table reset.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Base of the callback state block in RAM (`lui 0x14` + `addiu -0x1ab0`).
const TABLE: u32 = 0x13_E550;
/// Record count (word) in the state block.
const COUNT_OFF: u32 = 0xd90;
/// Record array base pointer (word) in the state block.
const RECORDS_OFF: u32 = 0xd94;
/// Size of one callback record in bytes.
const RECORD_SIZE: u32 = 0x90;
/// Offset of the callback pointer inside a record.
const CALLBACK_OFF: u32 = 4;
/// Stack frame of `run_entry_callbacks`.
const WALK_FRAME: u32 = 0x40;
/// Stack frame of `reset_tables`.
const RESET_FRAME: u32 = 0x10;
/// Retail setup routine called once at the start of `reset_tables`.
const SETUP_A: u32 = 0x12_EB00;
/// Retail routine called once, then polled until it returns 0.
const SETUP_B: u32 = 0x12_DC80;
/// Retail setup routine called after `SETUP_B`.
const SETUP_C: u32 = 0x12_E3B8;
/// Quadwords zeroed at the start of the state block.
const CLEARED_QUADWORDS: u32 = 4;
/// Word zeroed at table+0x40, after the quadwords.
const WORD_40: u32 = 0x40;
/// Stride of the slot records cleared by `reset_tables`.
const SLOT_STRIDE: u32 = 0x70;
/// Word at the start of each slot record.
const SLOT_WORD: u32 = 0x70;
/// Byte flag inside each slot record.
const SLOT_BYTE: u32 = 0x74;
/// End of the slot range (`table + 0xd20`), exclusive.
const SLOT_END: u32 = 0xd20;

/// Sets general register `n` to the 32-bit value `v`, sign-extended as a MIPS 32-bit op leaves it.
fn set_gpr(c: &mut Cpu, n: usize, v: u32) {
    c.s(n, v as i32 as i64 as u64);
}

/// Views a 128-bit general register as a quadword (lane 0 is the low word).
fn reg_v(r: u128) -> V {
    V([r as u32, (r >> 32) as u32, (r >> 64) as u32, (r >> 96) as u32])
}

/// Zeroes the 16-byte quadword at `addr` (retail `sq zero`).
fn clear_quad(c: &mut Cpu, addr: u32) {
    for k in 0..4 {
        c.w32(addr + 4 * k, 0);
    }
}

/// 0x27e7e8 run_entry_callbacks() -> void: calls each non-null callback of the record array, record address in a0.
pub fn run_entry_callbacks(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::with_stack(c, WALK_FRAME, |c, frame| {
        let (s0, s1, s2, ra) = (c.r[16], c.r[17], c.r[18], c.r[31]);
        c.wv(frame, reg_v(s0));
        c.wv(frame + 0x10, reg_v(s1));
        c.wv(frame + 0x20, reg_v(s2));
        c.wv(frame + 0x30, reg_v(ra));
        set_gpr(c, 3, TABLE);
        let count = c.r32(TABLE + COUNT_OFF) as i32;
        if count <= 0 {
            set_gpr(c, 2, count as u32);
            return;
        }
        let mut index: i32 = 0;
        loop {
            let records = c.r32(TABLE + RECORDS_OFF);
            let record = records.wrapping_add((index as u32).wrapping_mul(RECORD_SIZE));
            set_gpr(c, 4, record);
            let callback = c.r32(record + CALLBACK_OFF);
            if callback != 0 {
                set_gpr(c, 2, callback);
                h::call_keep(c, callback, &[record], &[]);
            }
            index += 1;
            let count = c.r32(TABLE + COUNT_OFF) as i32;
            set_gpr(c, 2, (index < count) as u32);
            if index >= count {
                break;
            }
        }
    });
    true
}

/// 0x27e868 reset_tables() -> void: runs the setup calls, zeroes the head of the state block, clears the slot records.
pub fn reset_tables(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::with_stack(c, RESET_FRAME, |c, frame| {
        let ra = c.r[31];
        c.wv(frame, reg_v(ra));
        h::call_keep(c, SETUP_A, &[], &[]);
        h::call_keep(c, SETUP_B, &[], &[]);
        h::call_keep(c, SETUP_C, &[], &[]);
        while h::call_keep(c, SETUP_B, &[], &[]).0 != 0 {}
        let mut quad = TABLE;
        let mut remaining = CLEARED_QUADWORDS as i32 - 1;
        loop {
            clear_quad(c, quad);
            remaining -= 1;
            quad += 0x10;
            if remaining < 0 {
                break;
            }
        }
        c.w32(TABLE + WORD_40, 0);
        let slot_end = TABLE + SLOT_END;
        c.w32(TABLE + SLOT_WORD, 0);
        let mut slot = TABLE;
        loop {
            c.w8(slot + SLOT_BYTE, 0);
            slot += SLOT_STRIDE;
            if slot >= slot_end {
                break;
            }
            c.w32(slot + SLOT_WORD, 0);
        }
        set_gpr(c, 2, 0);
        set_gpr(c, 3, slot);
        set_gpr(c, 4, slot_end);
    });
    true
}
