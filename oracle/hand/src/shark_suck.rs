//! Shark suck state wrappers (aridia): accept, fire and cancel set the suck object's state byte, and the slot latch
//! that feeds accept/fire through the weapon-slot lookup.

use crate::h;
use crate::ps2::*;
use crate::voice_slot;
use crate::weapon_slots;
use ee_oracle::cpu::Cpu;

const V1: usize = 3;

/// Core accept routine (wrapped by `shark_suck_accept`).
const ACCEPT_CORE: u32 = 0x002d_d5b0;
/// Core fire routine (wrapped by `shark_suck_fire`).
const FIRE_CORE: u32 = 0x002d_d7d8;
/// Core cancel routine (wrapped by `shark_suck_cancel`).
const CANCEL_CORE: u32 = 0x002d_dad8;
/// Core routine that `suck_slot_latch` calls after the slot lookup.
const SLOT_CORE: u32 = 0x002d_d8e8;

/// Suck state byte inside the suck object.
const STATE_OFF: u32 = 0x20;
/// State "armed": set when the core call succeeds, and by cancel.
const STATE_ARMED: u8 = 9;
/// State written when a core call fails while the object is still armed.
const STATE_DONE: u8 = 1;
/// Slot record halfword that selects the moby cue.
const SLOT_KIND_OFF: u32 = 0x68;
const KIND_CUE: i16 = 6;
/// Moby cue arguments: mode 1, zero, then the suck object.
const CUE_MODE: u32 = 1;

/// Calls `core(obj)`; a nonzero result arms the state byte (9), a zero result turns an armed (9) state into 1.
/// Returns the core result.
fn suck_latch(c: &mut Cpu, core: u32) -> bool {
    let obj = c.arg(0);
    let (r, _) = h::call_keep(c, core, &[obj], &[]);
    c.set_arg(0, r);
    c.ret(r);
    let state_addr = obj.wrapping_add(STATE_OFF);
    if r != 0 {
        c.w8(state_addr, STATE_ARMED);
    } else {
        let state = c.r8(state_addr);
        c.s(V1, state as u64);
        if state == STATE_ARMED {
            c.w8(state_addr, STATE_DONE);
        }
    }
    true
}

/// 0x2c3958 shark_suck_accept(obj) -> v0: accept core on obj; nonzero arms state 9, zero turns 9 into 1.
pub fn shark_suck_accept(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    suck_latch(c, ACCEPT_CORE)
}

/// 0x2c39a8 shark_suck_fire(obj) -> v0: fire core on obj; nonzero arms state 9, zero turns 9 into 1.
pub fn shark_suck_fire(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    suck_latch(c, FIRE_CORE)
}

/// 0x2c39f8 suck_slot_latch(obj, a1, a2, f12) -> v0: looks up the weapon slot, calls the slot core, updates the suck
/// state, and plays moby cue 1 when the slot kind is 6. Returns the slot core result.
pub fn suck_slot_latch(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    let a1 = c.arg(1);
    let a2 = c.arg(2);
    let f12 = c.farg(0);
    let (slot, _) = h::run_keep(c, weapon_slots::comp_lookup, &[obj, a1, a2], &[f12]);
    let (r, _) = h::call_keep(c, SLOT_CORE, &[obj, a1, a2], &[f12]);
    let state_addr = obj.wrapping_add(STATE_OFF);
    if r != 0 {
        c.w8(state_addr, STATE_ARMED);
    } else if c.r8(state_addr) == STATE_ARMED {
        c.w8(state_addr, STATE_DONE);
    }
    let kind = c.r16(slot.wrapping_add(SLOT_KIND_OFF)) as i16;
    c.s(V1, kind as i64 as u64);
    if kind == KIND_CUE {
        h::run_keep(c, voice_slot::moby_cue, &[CUE_MODE, 0, obj], &[]);
    }
    c.ret(r);
    true
}

/// 0x2c3aa8 shark_suck_cancel(obj) -> (): calls the cancel core on obj, then sets the state to 9 (armed).
pub fn shark_suck_cancel(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    h::call_keep(c, CANCEL_CORE, &[obj], &[]);
    c.w8(obj.wrapping_add(STATE_OFF), STATE_ARMED);
    c.s(V1, STATE_ARMED as u64);
    true
}
