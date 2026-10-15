//! Help-chain cancel state machine (0x1f6928 .. 0x1f6c2c) and the save-record packer (0x23f3e8).

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// MIPS register numbers used here.
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;

/// High half of the data base (`lui 0x18`); the help block sits at `HI - 0x6af0`.
const HI: u32 = 0x0018_0000;
/// Help state block.
const HELP: u32 = 0x0017_9510;
/// State word: index into `STATE_TABLE`, 0..8; 8 is the armed-chain state.
const HELP_STATE: u32 = HELP;
/// Countdown word, used by state 4.
const HELP_TIMER: u32 = HELP + 0x4;
/// Word written with -1 by state 0.
const HELP_ARMED: u32 = HELP + 0x24;
/// Halfword: help chain flag (set by `help_graceful_cancel`, cleared by `help_chain_clear`).
const HELP_CHAIN_FLAG: u32 = HELP + 0x38;
/// Word: scaled countdown (result of `float_to_int_scaled(60)`).
const HELP_CHAIN_SCALE: u32 = HELP + 0x3c;
/// Jump table of the state switch: one retail code address per state.
const STATE_TABLE: u32 = 0x001d_9870;
/// What `lui 0x1e` leaves in v0 on the out-of-range path (high half of the table base).
const TABLE_HI: u32 = 0x001e_0000;
/// Argument of `float_to_int_scaled` for the chain countdown.
const CHAIN_SCALE_ARG: u32 = 0x3c;

/// Save-record lists (`HI + 0x46c0`, `HI + 0x49c0`) and the number of chunk blocks packed after the header.
const SAVE_LIST_A: u32 = 0x0018_46c0;
const SAVE_LIST_B: u32 = 0x0018_49c0;
const SAVE_BLOCKS: u32 = 20;
/// Retail `chunk_emit(dst, index, list) -> size`; called as retail so the VU registers it leaves match.
const CHUNK_EMIT: u32 = 0x0023_f5b0;

fn set(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x1f6928 help_state_step() -> v0: one step of the help cancel state machine, dispatched on the state word
/// (states 0..7 through the jump table; 8 and up return with v0 = 0x1e0000 and change nothing).
pub fn help_state_step(c: &mut Cpu) -> bool {
    let state = c.r32(HELP_STATE);
    // states 1-3, 5-7: their jump-table targets are not exercised by any run; retail runs them
    if matches!(state, 1 | 2 | 3 | 5 | 6 | 7) {
        return false;
    }
    set(c, A2, HI);
    if state >= 8 {
        set(c, V0, TABLE_HI);
        set(c, V1, state);
        return true;
    }
    let slot = STATE_TABLE + state * 4;
    set(c, V0, STATE_TABLE);
    set(c, V1, slot);
    let target = c.r32(slot);
    set(c, A0, target);
    match state {
        // armed: mark the block -1 and return -1
        0 => {
            set(c, V1, HELP);
            set(c, V0, u32::MAX);
            c.w32(HELP_ARMED, u32::MAX);
        }
        // states 1-3 move to 7, state 5 moves to 6; both reset the countdown
        1..=3 | 5 => {
            let next = if state == 5 { 6 } else { 7 };
            set(c, V0, next);
            set(c, V1, HELP);
            c.w32(HELP_STATE, next);
            c.w32(HELP_TIMER, 0);
        }
        // countdown: the timer word becomes 4 - timer, the state moves to 6
        4 => {
            set(c, A1, HELP);
            set(c, V1, 4);
            let timer = c.r32(HELP_TIMER);
            set(c, A0, timer);
            set(c, V0, 6);
            c.w32(HELP_STATE, 6);
            let left = 4u32.wrapping_sub(timer);
            set(c, V1, left);
            c.w32(HELP_TIMER, left);
        }
        // states 6 and 7 change nothing (v0 keeps the table address)
        _ => {}
    }
    true
}

/// 0x1f6c20 help_chain_clear(): clears the help chain flag halfword; v0 = 0x180000.
pub fn help_chain_clear(c: &mut Cpu) -> bool {
    c.w16(HELP_CHAIN_FLAG, 0);
    set(c, V0, HI);
    true
}

/// 0x1f6bc0 help_graceful_cancel(): starts the help chain once: if the chain flag is clear, sets it, stores the
/// scaled countdown (`float_to_int_scaled(60)`), runs `help_state_step`, and arms state 8 when the state is 0.
pub fn help_graceful_cancel(c: &mut Cpu) -> bool {
    // the start path runs help_state_step, which declines states 1-3, 5-7: decline here too, before any write
    if c.r16(HELP_CHAIN_FLAG) == 0 && matches!(c.r32(HELP_STATE), 1 | 2 | 3 | 5 | 6 | 7) {
        return false;
    }
    h::frame(c, 0x30, &[(17, 0x10, h::SQ), (16, 0x0, h::SQ), (31, 0x20, h::SQ)], |c, _sp| {
        let flag = c.r16(HELP_CHAIN_FLAG) as i16 as i32 as u32;
        set(c, V0, flag);
        if flag != 0 {
            return;
        }
        c.w16(HELP_CHAIN_FLAG, 1);
        let (scale, _) = h::run_keep(c, crate::mathx::float_to_int_scaled, &[CHAIN_SCALE_ARG], &[]);
        c.w32(HELP_CHAIN_SCALE, scale);
        help_state_step(c);
        let state = c.r32(HELP_STATE);
        if state == 0 {
            set(c, V0, 8);
            c.w32(HELP_STATE, 8);
        } else {
            set(c, V0, state);
        }
    });
    true
}

/// 0x23f3e8 save_record_pack(dst) -> 0: writes two record-list sizes at dst, packs the first chunk block and then
/// 20 numbered blocks after an 8-byte header, advancing dst by each block's size; leaves dst + total in a0.
pub fn save_record_pack(c: &mut Cpu) -> bool {
    let dst = c.g(A0) as u32;
    h::frame(
        c,
        0x60,
        &[
            (16, 0x0, h::SQ),
            (17, 0x10, h::SQ),
            (18, 0x20, h::SQ),
            (19, 0x30, h::SQ),
            (20, 0x40, h::SQ),
            (31, 0x50, h::SQ),
        ],
        |c, _sp| {
            let (size_a, _) = h::run_keep(c, crate::row_records::record_list_bytes, &[SAVE_LIST_A], &[]);
            c.w32(dst, size_a);
            let (size_b, _) = h::run_keep(c, crate::row_records::record_list_bytes, &[SAVE_LIST_B], &[]);
            c.w32(dst + 4, size_b);
            let mut out = dst + 8;
            let (n, _) = h::call_keep(c, CHUNK_EMIT, &[out, 0, SAVE_LIST_A], &[]);
            out = out.wrapping_add(n);
            for block in 0..SAVE_BLOCKS {
                let (n, _) = h::call_keep(c, CHUNK_EMIT, &[out, block, SAVE_LIST_B], &[]);
                out = out.wrapping_add(n);
            }
            set(c, A0, out);
            set(c, V0, 0);
        },
    );
    true
}
