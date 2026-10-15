//! Angle approach step and small level-glue helpers: `approach_angle` turns an angle toward a target by at most a
//! signed step, `forward_word` calls a retail routine with a boot word, `level_word` reads a loader word, and
//! `claim_slot_try` claims a table slot in the overlay state block.
use crate::h;
use crate::motion;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

const ZERO: F = F(0);

/// Retail 0x121e40 (called by `forward_word`).
const FORWARD_CALL: u32 = 0x0012_1e40;
/// Boot word read by `forward_word` (0x160000 - 0x1148).
const FORWARD_ARG: u32 = 0x0015_eeb8;
/// Loader word read by `level_word` (0x170000 + 0x3154).
const LEVEL_WORD: u32 = 0x0017_3154;
/// High half of the loader word address used by `level_word`.
const LEVEL_HI: u32 = 0x0017_0000;

/// Overlay state block (0x180000 - 0x6af0): loader word at +0, slot pointer at +0x24, slot argument at +0x28.
const STATE: u32 = 0x0017_9510;
const SLOT_PTR: u32 = STATE + 0x24;
const SLOT_ARG: u32 = STATE + 0x28;
/// Gate block (0x1516d0): busy word at +0x50, gate value at +0x1c.
const GATE_BUSY: u32 = 0x0015_1720;
const GATE_VAL: u32 = 0x0015_16ec;
/// Slot table (0x141968), 8 bytes per entry, halfword at +0 (0xffff = empty).
const TABLE: u32 = 0x0014_1968;
const CLAIM_CALL: u32 = 0x001f_7c08;
const NONE: u32 = 0xffff_ffff;
const EMPTY: u32 = 0xffff;

/// Writes a general register with the sign-extended 32-bit value `v` (as the 32-bit EE ops leave it).
fn put(c: &mut Cpu, reg: usize, v: u32) {
    c.r[reg] = v as i32 as i128 as u128;
}

/// 0x1f2aa8 approach_angle(a, target, step) -> a moved toward target by at most step, wrapped into [-pi, pi).
/// The step takes the sign of the wrapped difference (target - a); the move is clamped to its magnitude.
pub fn approach_angle(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let (a, target, step) = (c.farg(0), c.farg(1), c.farg(2));
    let old = [c.f[20], c.f[21], c.f[22]];
    let (r, x) = h::frame(c, 0x30, &[(31, 0x0, h::SQ)], |c, sp| {
        c.w32(sp + 0x10, old[0]);
        c.w32(sp + 0x18, old[1]);
        c.w32(sp + 0x20, old[2]);
        let (_, diff) = h::run(c, vecmath::angle_sub, &[], &[target, a]);
        let signed = if ZERO < diff { step } else { -step };
        let (_, mag) = h::run(c, motion::fabs, &[], &[diff]);
        let x = if mag < signed {
            mag
        } else if signed < -mag {
            -mag
        } else {
            signed
        };
        let (_, r) = h::run(c, vecmath::angle_add, &[], &[a, x]);
        (r, x)
    });
    c.f[12] = a.0;
    c.f[13] = x.0;
    c.fret(r);
    true
}

/// 0x1f4028 forward_word() -> retail 0x121e40(word at 0x15eeb8): returns what that routine returns (v0, f0).
pub fn forward_word(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let arg = c.r32(FORWARD_ARG);
    h::frame(c, 0x10, &[(31, 0x0, h::SQ)], |c, _sp| {
        h::call_keep(c, FORWARD_CALL, &[arg], &[]);
    });
    true
}

/// 0x1f4830 level_word() -> the loader word at 0x173154 (v0); v1 is left at 0x170000 as the lui leaves it.
pub fn level_word(c: &mut Cpu) -> bool {
    put(c, 3, LEVEL_HI);
    let v = c.r32(LEVEL_WORD);
    c.ret(v);
    true
}

/// Result of `claim_body` on a path that returns: final v0, v1, a0, a1. `None`: it called 0x1f7c08.
type Exit = Option<(u32, u32, u32, u32)>;

/// The body of `claim_slot_try` (inside its frame).
fn claim_body(c: &mut Cpu, a0: u32, a1: u32) -> Exit {
    let loader = c.r32(STATE);
    if loader != 0 {
        return Some((0, loader, a0, a1));
    }
    let slot = c.r32(SLOT_PTR);
    if slot != NONE {
        return Some((0, 0, slot, a1));
    }
    let busy = c.r32(GATE_BUSY);
    if busy != 0 {
        return Some((0, busy, NONE, a1));
    }
    if c.r32(GATE_VAL) != NONE {
        return Some((0, 0, NONE, a1));
    }
    let entry = TABLE.wrapping_add(a1 << 3);
    if c.r16(entry) as u32 == EMPTY {
        return Some((0, entry, a0, EMPTY));
    }
    c.w32(SLOT_ARG, a1);
    c.w32(SLOT_PTR, a0);
    put(c, 2, TABLE);
    put(c, 3, entry);
    put(c, 4, a0);
    put(c, 5, EMPTY);
    put(c, 6, STATE);
    put(c, 7, a1);
    put(c, 8, a0);
    h::call_keep(c, CLAIM_CALL, &[], &[]);
    None
}

/// 0x1f69b0 claim_slot_try(a0, a1) -> 1 when the slot table entry a1 was claimed through 0x1f7c08 (a0), else 0.
/// Declines nothing once the guard passes; the loader word, gate words and table are read from RAM at call time.
pub fn claim_slot_try(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let (a0, a1) = (c.arg(0), c.arg(1));
    match h::frame(c, 0x10, &[(31, 0x0, h::SQ)], |c, _sp| claim_body(c, a0, a1)) {
        Some((v0, v1, x0, x1)) => {
            put(c, 2, v0);
            put(c, 3, v1);
            put(c, 4, x0);
            put(c, 5, x1);
            put(c, 6, STATE);
            put(c, 7, a1);
            put(c, 8, a0);
        }
        None => put(c, 2, 1),
    }
    true
}
