//! Predicates on the glove slot value (via `glove_slot::slot_value_b_fn`) and a hero state gate.
use crate::glove_slot;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Bytes the retail prologue reserves on the stack for the saved return address.
const FRAME: u32 = 0x10;
/// Base of the global block that the hero gate reads (`lui 0x14` + `addiu -0xcb0`).
const GATE_BASE: u32 = 0x0013_F350;
/// Group word read by `hero_skip_gate` (base + 0x208c).
const GROUP_ADDR: u32 = GATE_BASE + 0x208C;
/// State word read by `hero_skip_gate` (base + 0x2084).
const STATE_ADDR: u32 = GATE_BASE + 0x2084;
/// Group values that always skip (the range 0x11..=0x12).
const GROUP_FIRST: u32 = 0x11;
/// Hero state values that skip.
const SKIP_STATES: [u32; 4] = [0x6A, 0x82, 0x76, 0x75];

/// Calls the glove slot helper the way the retail prologue does: stores `ra` in the 16-byte frame, then calls it.
/// Returns the helper's v0.
fn glove_slot_value(c: &mut Cpu) -> u32 {
    let frame = (c.g(29) as u32).wrapping_sub(FRAME);
    let ra = c.r[31];
    c.mem.wr128(frame & !15, ra);
    h::run(c, glove_slot::slot_value_b_fn, &[], &[]).0
}

/// 0x2005c0 slot_b_in_5_7(a0) -> v0: 1 when the glove slot value is 5, 6 or 7, else 0.
pub fn slot_b_in_5_7(c: &mut Cpu) -> bool {
    let v = glove_slot_value(c);
    c.ret((v.wrapping_sub(5) < 3) as u32);
    true
}

/// 0x2005e8 slot_b_in_1c_1d(a0) -> v0: 1 when the glove slot value is 0x1c or 0x1d, else 0.
pub fn slot_b_in_1c_1d(c: &mut Cpu) -> bool {
    let v = glove_slot_value(c);
    c.ret((v.wrapping_sub(0x1C) < 2) as u32);
    true
}

/// 0x200610 hero_skip_gate() -> v0: 1 when the hero group is 0x11/0x12 or the state is 0x6a/0x82/0x76/0x75.
/// Leaves v1 at the global block base in the group case, else at the loaded state word (retail side effect).
pub fn hero_skip_gate(c: &mut Cpu) -> bool {
    let group = c.r32(GROUP_ADDR);
    if group.wrapping_sub(GROUP_FIRST) < 2 {
        c.s(3, GATE_BASE as u64);
        c.ret(1);
        return true;
    }
    let state = c.r32(STATE_ADDR);
    c.s(3, state as i32 as i64 as u64);
    c.ret(SKIP_STATES.contains(&state) as u32);
    true
}
