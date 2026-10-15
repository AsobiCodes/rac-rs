//! Pause-menu slot pass: the per-slot dispatch loop (declined) and the slot-pointer clear.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail slot helper (0x275dc0): takes a slot pointer, returns 0 in v0.
const SLOT_HELPER: u32 = 0x0027_5dc0;
/// Object offsets of the two slot pointers cleared by `pause_slot_clear`.
const SLOT_A: u32 = 0x44;
const SLOT_B: u32 = 0x48;

/// 0x26e680 pause_slot_pass(a0 = obj) -> v0 4: walks 24 slots, dispatching through a jump table at 0x1dae0.
/// Declined: the table entries in RAM point at code outside the case blocks, so the jump is not modelled.
pub fn pause_slot_pass(_c: &mut Cpu) -> bool {
    false
}

/// 0x26e7e0 pause_slot_clear(a0 = obj) -> v0 0: clears the slot pointers at obj+0x44 and obj+0x48 through 0x275dc0.
pub fn pause_slot_clear(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let first_in = c.r32(obj.wrapping_add(SLOT_A));
    let (first, _) = h::call_keep(c, SLOT_HELPER, &[first_in], &[]);
    c.w32(obj.wrapping_add(SLOT_A), first);
    let (second, _) = h::call_keep(c, SLOT_HELPER, &[first], &[]);
    c.w32(obj.wrapping_add(SLOT_B), second);
    c.ret(0);
    true
}
