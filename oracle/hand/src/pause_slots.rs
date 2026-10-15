//! Pause-menu slot bookkeeping (retail 0x2771a8, 0x277208, 0x277248).

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Stack frame size of `reset_pause_state` (retail `addiu sp,sp,-0x20`).
const RESET_FRAME: u32 = 0x20;
/// Halfword pause flag (`lh` at 0x1516d8).
const PAUSE_FLAG: u32 = 0x0015_16d8;
/// Pause state block (0x1c0000 - 0x6210).
const STATE: u32 = 0x001b_9df0;
/// Pending-callback byte in the state block (+0xcb).
const STATE_PENDING: u32 = STATE + 0xcb;
/// Word cleared at the end of `reset_pause_state` (0x1c0000 - 0x5d74).
const RESET_WORD_A: u32 = 0x001b_a28c;
/// Word cleared at the end of `reset_pause_state` (0x160000 + 0x390).
const RESET_WORD_B: u32 = 0x0016_0390;
/// Routine run once when the pending byte is set.
const PAUSE_CALLBACK: u32 = 0x0025_8080;
/// Records of (byte-table index, slot index), 8 bytes each, read by `clear_pending_slots`.
const RECORD_TABLE: u32 = 0x001b_9bd8;
/// Per-record byte table base (`lui 0x19` + 0x7cc0).
const KIND_BYTES: u32 = 0x0019_7cc0;
/// Pointer table indexed by the record byte (`lui 0x19` + 0x7400).
const KIND_PTRS: u32 = 0x0019_7400;
/// Offset of the word cleared inside each slot.
const SLOT_FLAG_OFF: u32 = 0x48;
/// Start index word in the state block.
const START_OFF: u32 = 0xa8;
/// Count word in the state block.
const COUNT_OFF: u32 = 0xac;
/// Constant `lui 0x1c` value used for the state block high half.
const HI_1C: u32 = 0x001c_0000;

/// Sets a general register to a 32-bit result, sign-extended as the EE's 32-bit ops do.
fn set(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x2771a8 reset_pause_state() -> void: if the pause flag is set and the pending byte is non-zero, clears the byte
/// after one call to the retail routine; then clears two state words.
pub fn reset_pause_state(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, RESET_FRAME, &[(31, 0x10, h::SQ), (16, 0x0, h::SQ)], |c, _sp| {
        let flag = c.r16(PAUSE_FLAG) as i16;
        set(c, 3, flag as i32 as u32);
        if flag != 0 {
            let pending = c.r8(STATE_PENDING);
            set(c, 3, pending as u32);
            if pending != 0 {
                set(c, 2, HI_1C);
                h::call_keep(c, PAUSE_CALLBACK, &[], &[]);
                c.w8(STATE_PENDING, 0);
            }
        }
        set(c, 2, HI_1C);
        c.w32(RESET_WORD_A, 0);
        set(c, 2, 0);
        set(c, 1, 0x0016_0000);
        c.w32(RESET_WORD_B, 0);
    });
    true
}

/// 0x277208 make_half_rect(dst a0, src a1) -> void: writes a 16-byte record from the source's two i32 fields at
/// +0x20 and +0x24, halved (arithmetic shift), plus a fixed 0x10 word at +0x10.
pub fn make_half_rect(c: &mut Cpu) -> bool {
    let dst = c.arg(0);
    let src = c.arg(1);
    c.w16(dst, 0);
    let lo_y = c.r16(src + 0x24);
    c.w16(dst + 2, lo_y);
    c.w16(dst + 4, 0);
    let lo_x = c.r16(src + 0x20);
    c.w16(dst + 6, lo_x);
    let x = (c.r32(src + 0x20) as i32) >> 1;
    c.w16(dst + 8, x as u16);
    set(c, 6, 0x10);
    c.w16(dst + 0x10, 0x10);
    c.w16(dst + 0x12, 0);
    let y = (c.r32(src + 0x24) as i32) >> 1;
    c.w16(dst + 0xa, y as u16);
    set(c, 2, x as u32);
    set(c, 3, y as u32);
    true
}

/// 0x277248 clear_pending_slots() -> void: for each record from `start` to `start + count`, zeroes the word at +0x48
/// of the slot named by the record's byte and pointer, then sets count to 0.
pub fn clear_pending_slots(c: &mut Cpu) -> bool {
    set(c, 7, STATE);
    set(c, 10, HI_1C);
    let start = c.r32(STATE + START_OFF);
    set(c, 6, start);
    let end = start.wrapping_add(c.r32(STATE + COUNT_OFF));
    let mut v1: u32 = 0x0019_0000;
    if (start as i32) < (end as i32) {
        set(c, 9, KIND_BYTES);
        set(c, 8, KIND_PTRS);
        let mut i = start;
        let mut a0: u32 = 0x0019_0000;
        let mut a1 = (start << 3).wrapping_add(RECORD_TABLE);
        loop {
            let rec_byte_ptr = c.r32(a1);
            let sub = c.r32(a1 + 4);
            i = i.wrapping_add(1);
            let kind = c.r8(rec_byte_ptr.wrapping_add(KIND_BYTES)) as u32;
            let slot_base = c.r32(KIND_PTRS.wrapping_add(kind << 2));
            a0 = sub << 2;
            c.w32(slot_base.wrapping_add(a0).wrapping_add(SLOT_FLAG_OFF), 0);
            v1 = 0;
            a1 = a1.wrapping_add(8);
            let limit = c.r32(STATE + START_OFF).wrapping_add(c.r32(STATE + COUNT_OFF));
            if (i as i32) >= (limit as i32) {
                break;
            }
            v1 = 1;
        }
        set(c, 4, a0);
        set(c, 5, a1);
        set(c, 6, i);
    }
    set(c, 3, v1);
    set(c, 2, STATE);
    c.w32(STATE + COUNT_OFF, 0);
    true
}
