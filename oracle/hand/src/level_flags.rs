//! Level-1 object flag latches: two byte flags set when a tracked object matches a fixed kind/sub-kind/mode.

use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Register numbers (MIPS GPR indices) the retail code leaves behind.
const V0: usize = 2;
const V1: usize = 3;
const A1: usize = 5;

/// Level word in RAM; the latches only run while it holds `LEVEL_WANTED`.
const LEVEL_WORD: u32 = 0x0015_ed84;
const LEVEL_WANTED: u32 = 1;
/// Byte flags written by the two phases (set to the level word's value).
const FLAG_A: u32 = 0x0013_d394;
const FLAG_B: u32 = 0x0013_d395;
/// Object fields: kind (halfword), sub-kind (halfword), mode (byte).
const OFF_KIND: u32 = 0xb2;
const OFF_SUB: u32 = 0xa6;
const OFF_MODE: u32 = 0x20;
/// Expected values for phase one.
const PHASE1_KIND: i32 = 0x34;
const PHASE1_SUB: i32 = 0x118;
const PHASE1_MODE: i32 = 5;
/// Expected values for phase two.
const PHASE2_KIND: i32 = 0x35;
const PHASE2_SUB: i32 = 0x118;
const PHASE2_MODE: i32 = 5;
/// Value the retail code leaves in v0 after its last test (`lui v0, 0x14`).
const UPPER_14: i32 = 0x0014_0000;

/// Reads a field the way retail does: `lh` (sign-extended) for the halfwords, `lbu` for the mode byte.
fn field(c: &mut Cpu, obj: u32, off: u32, half: bool) -> i32 {
    if half {
        c.r16(obj + off) as i16 as i32
    } else {
        c.r8(obj + off) as i32
    }
}

/// Phase one: kind == 0x34, sub == 0x118, mode == 5. Returns (v0, v1, whether the flag store runs).
fn phase_one(c: &mut Cpu, obj: u32) -> (i32, i32, bool) {
    let kind = field(c, obj, OFF_KIND, true);
    if kind != PHASE1_KIND {
        return (1, kind, false);
    }
    let sub = field(c, obj, OFF_SUB, true);
    if sub != PHASE1_SUB {
        return (1, sub, false);
    }
    let mode = field(c, obj, OFF_MODE, false);
    if mode != PHASE1_MODE {
        return (1, mode, false);
    }
    (1, mode, true)
}

/// Phase two: kind == 0x35, sub == 0x118, mode == 5. Returns (v0, v1, whether the flag store runs).
fn phase_two(c: &mut Cpu, obj: u32) -> (i32, i32, bool) {
    let kind = field(c, obj, OFF_KIND, true);
    if kind != PHASE2_KIND {
        return (PHASE2_SUB, kind, false);
    }
    let sub = field(c, obj, OFF_SUB, true);
    if sub != PHASE2_SUB {
        return (PHASE2_MODE, sub, false);
    }
    let mode = field(c, obj, OFF_MODE, false);
    if mode != PHASE2_MODE {
        return (UPPER_14, mode, false);
    }
    (UPPER_14, mode, true)
}

/// 0x2bf5c8 level_flags_update(obj) -> none: sets the two byte flags when obj matches phase one / phase two.
pub fn update(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let level = c.r32(LEVEL_WORD);
    if level != LEVEL_WANTED {
        // The level test fails: retail leaves v0 = 0x34 and v1 unchanged.
        c.s(V0, PHASE1_KIND as i64 as u64);
        c.s(A1, level as u64);
        return true;
    }
    let (_, _, store_a) = phase_one(c, obj);
    if store_a {
        c.w8(FLAG_A, level as u8);
    }
    let (v0, v1, store_b) = phase_two(c, obj);
    if store_b {
        c.w8(FLAG_B, level as u8);
    }
    c.s(V0, v0 as i64 as u64);
    c.s(V1, v1 as i64 as u64);
    c.s(A1, level as u64);
    true
}

/// 0x2bf658 mode_dispatch(obj) -> none: per-frame mode state machine (not rewritten; retail runs).
pub fn mode_dispatch(_c: &mut Cpu) -> bool {
    false
}
