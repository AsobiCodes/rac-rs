//! Object state ticks (retail 0x2e4cc8..0x2e4d64 and 0x2e72d0..0x2e73c8). A state byte at object +0x20 moves
//! 0 -> 1 -> 2; the mode word at 0x15f5a4 gates a table call, and the link step calls the record linker first.

use crate::h;
use crate::ps2::*;
use crate::{actor, bolt_prompts, objects, voice_slot};
use ee_oracle::cpu::Cpu;

/// Object fields: state byte at +0x20, timer byte at +0x30.
const OBJ_STATE: u32 = 0x20;
const OBJ_TIMER: u32 = 0x30;
/// Timer value a fresh object gets when it leaves state 0.
const TIMER_START: u32 = 0xff;
/// Mode word (lw from 0x15f5a4) that must equal MODE_WANTED for the table call.
const MODE_WORD: u32 = 0x0015_f5a4;
const MODE_WANTED: u32 = 2;
/// Table selector word at 0x16c990; the table call runs only while it is below 2 (unsigned).
const TABLE_SELECTOR: u32 = 0x0016_c990;
const TABLE_SELECT_LIMIT: u32 = 2;
/// Table base at 0x16c960 (0x170000 - 0x36a0), its upper half 0x170000, and the entry displacement.
const TABLE_BASE: u32 = 0x0016_c960;
const TABLE_HI: u32 = 0x0017_0000;
const TABLE_ENTRY_OFF: u32 = 0x178;
/// Retail callee of the table dispatch (0x255c78).
const TABLE_CALL: u32 = 0x0025_5c78;
/// Link step: a1 = 0x10000, a2 = 0 for objects::link_record.
const LINK_ARG1: u32 = 0x0001_0000;
/// Record field the link result's float (f1) is read from.
const LINK_RESULT_FLOAT: u32 = 0x2c;
/// Retail callees of the state-2 path.
const CUE_ARG1: u32 = 0;
const PROMPT_CALL: u32 = 0x0025_6150;
const PROMPT_ARG1: u32 = 0x72c;
const PROMPT_ARG2: u32 = 1;
const PROMPT_ARG_T2: u32 = 2;
const RELEASE_CALL: u32 = 0x0025_6640;
const RELEASE_ARG1: u32 = 0x72b;
/// Fixed state values.
const STATE_FRESH: u32 = 0;
const STATE_SPAWNED: u32 = 1;
const STATE_LINKED: u32 = 2;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// The table dispatch of slot_state_tick (state 1): calls table entry `idx` while the mode word is MODE_WANTED.
fn table_dispatch(c: &mut Cpu) {
    let mode = c.r32(MODE_WORD);
    set_gpr(c, 4, mode);
    set_gpr(c, 2, MODE_WANTED);
    // The `lui v0` in the delay slot of the mode test runs whether or not the branch is taken.
    set_gpr(c, 2, TABLE_HI);
    if mode != MODE_WANTED {
        return;
    }
    set_gpr(c, 5, TABLE_HI);
    let selector = c.r32(TABLE_SELECTOR);
    set_gpr(c, 2, selector);
    let below = selector < TABLE_SELECT_LIMIT;
    set_gpr(c, 3, below as u32);
    if !below {
        return;
    }
    // Index 3 when the selector is 0, else the mode (2).
    let idx = if selector == 0 { 3 } else { MODE_WANTED };
    set_gpr(c, 3, idx << 2);
    let entry_addr = TABLE_BASE + (idx << 2);
    set_gpr(c, 2, entry_addr);
    let entry = c.r32(entry_addr + TABLE_ENTRY_OFF);
    h::call_keep(c, TABLE_CALL, &[entry], &[]);
}

/// 0x2e4cc8 slot_state_tick(obj) -> void: state 0 becomes state 1 with timer 0xff; state 1 runs the mode-gated table
/// call; other states do nothing.
pub fn slot_state_tick(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    h::frame(c, 0x10, &[(31, 0x00, h::SQ)], |c, _sp| {
        let state = c.r8(obj + OBJ_STATE) as u32;
        set_gpr(c, 2, 1);
        set_gpr(c, 3, state);
        match state {
            STATE_FRESH => {
                c.w8(obj + OBJ_STATE, STATE_SPAWNED as u8);
                c.w8(obj + OBJ_TIMER, TIMER_START as u8);
                set_gpr(c, 3, TIMER_START);
            }
            STATE_SPAWNED => table_dispatch(c),
            _ => {}
        }
    });
    true
}

/// 0x2e72d0 link_state_tick(obj) -> void: links the object record, then advances a state-0/1 object (state 1 moves
/// to state 2 when the link result's float is positive); state 2 runs the cue, prompt and release calls.
pub fn link_state_tick(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    h::frame(c, 0x30, &[(16, 0x00, h::SQ), (17, 0x10, h::SQ), (31, 0x20, h::SQ)], |c, _sp| {
        set_gpr(c, 16, obj);
        let link = h::run_keep(c, objects::link_record, &[obj, LINK_ARG1, 0], &[]);
        let link_v0 = link.0 as u32;
        set_gpr(c, 17, 0);
        let state = c.r8(obj + OBJ_STATE) as u32;
        set_gpr(c, 3, state);
        set_gpr(c, 4, STATE_SPAWNED);
        match state {
            STATE_SPAWNED => {
                let mut advance = false;
                if link_v0 != 0 {
                    let f1 = F(c.r32(link_v0 + LINK_RESULT_FLOAT));
                    c.f[0] = F(0).0;
                    c.f[1] = f1.0;
                    advance = F(0) < f1;
                    set_gpr(c, 17, advance as u32);
                }
                set_gpr(c, 2, STATE_LINKED);
                if advance {
                    c.w8(obj + OBJ_STATE, STATE_LINKED as u8);
                }
            }
            STATE_FRESH => {
                set_gpr(c, 2, 1);
                c.w8(obj + OBJ_STATE, STATE_SPAWNED as u8);
            }
            STATE_LINKED => {
                set_gpr(c, 2, STATE_LINKED);
                h::run_keep(c, voice_slot::moby_cue, &[CUE_ARG1, CUE_ARG1, obj], &[]);
                h::run_keep(c, bolt_prompts::flower_near_bolt_drop, &[obj], &[]);
                h::call_keep(
                    c,
                    PROMPT_CALL,
                    &[obj, PROMPT_ARG1, PROMPT_ARG2, PROMPT_ARG1, 1, 1, PROMPT_ARG_T2],
                    &[],
                );
                h::call_keep(c, RELEASE_CALL, &[obj, RELEASE_ARG1], &[]);
                h::run_keep(c, actor::free_moby, &[obj], &[]);
            }
            _ => set_gpr(c, 2, STATE_LINKED),
        }
    });
    true
}
