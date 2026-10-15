//! Limb pulse: scales the hero and its held items by a pulse-table value each frame, plus the small timer
//! state machine and one-time init that drive it. Globals live in the state block at 0x13f350 (gp-relative
//! timer flags for the state machine).
use crate::h;
use crate::motion;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Global state block (lui 0x14, addiu -0xcb0).
const STATE: u32 = 0x0013_F350;
/// Pulse step counter (halfword) in the state block.
const PULSE_STEP: u32 = STATE + 0x22DC;
/// Hero object pointer (word) in the state block.
const HERO: u32 = STATE + 0x2080;
/// Hold flag (byte): nonzero disables the pulse.
const HOLD_FLAG: u32 = STATE + 0x20A4;
/// Once-init flag (halfword) in the state block.
const INIT_FLAG: u32 = STATE + 0x1EB2;
/// Pulse table base (lui 0x18, addiu -0x6728); indexed by the step counter times 4.
const PULSE_TABLE: u32 = 0x0017_D8D8;
/// Sentinel value (-1.0) of the pulse table that ends the pulse.
const END_MARK: F = F(0xBF80_0000);
/// Scratch slot used by the timer state machine's slew (a float in the data segment).
const SLEW_VALUE: u32 = 0x0015_F3FC;
/// Phase words are gp-relative: the active flag, the phase, and the slew step.
const GP_ACTIVE: u32 = 0x75A0;
const GP_PHASE: u32 = 0x759C;
const GP_STEP: u32 = 0x75A4;
/// Number of outer (block) and inner (slot) iterations of the pulse loop.
const BLOCKS: u32 = 7;
const SLOTS: u32 = 3;
/// Stride of one block of hero limb records.
const BLOCK_STRIDE: u32 = 0x50;
/// Reference vector (0x13f420) that the pulse offsets each limb against.
const REF_VEC: u32 = 0x0013_F420;
/// Bits of 1.0f.
const ONE_BITS: u32 = 0x3F80_0000;

/// 0x1fab00 limb_pulse(): advances the pulse step and scales hero and held-item vectors by the table value; at the
/// end sentinel it resets the step, and with the hold flag set it does nothing.
pub fn limb_pulse(c: &mut Cpu) -> bool {
    if c.r16(PULSE_STEP) as i16 == 0 {
        return true;
    }
    let next = (c.r16(PULSE_STEP) as u16).wrapping_add(1);
    c.w16(PULSE_STEP, next);
    let table = PULSE_TABLE.wrapping_add(((next as i16 as i32) * 4) as u32);
    let scale = c.rf(table);
    if scale == END_MARK {
        c.w16(PULSE_STEP, 0);
        return true;
    }
    let hero = c.r32(HERO);
    h::vec_scale(c, hero + 0xC0, hero + 0xC0, scale);
    let hero = c.r32(HERO);
    h::vec_scale(c, hero + 0xD0, hero + 0xD0, scale);
    if c.r8(HOLD_FLAG) != 0 {
        return true;
    }
    let one = F(ONE_BITS);
    for block in 0..BLOCKS {
        for slot in 0..SLOTS {
            let rec = STATE + block * BLOCK_STRIDE;
            let item = if slot == 1 { c.r32(rec + 0x1094) } else { c.r32(rec + 0x1090) };
            // The first block scales by 1.0 when the pulse is above one; the others use the table value.
            let k = if block == 0 && one < scale { one } else { scale };
            if item == 0 {
                continue;
            }
            let base = item + 0x10;
            h::vec_scale(c, item + 0xC0, item + 0xC0, k);
            h::vec_scale(c, item + 0xD0, item + 0xD0, k);
            let sp = c.g(29) as u32;
            h::vec_sub(c, sp, base, REF_VEC);
            h::run(c, crate::hero_space::set_up_component, &[sp, sp], &[F::ZERO]);
            h::vec_sub(c, base, base, sp);
            h::vec_scale(c, sp, sp, scale);
            h::vec_add(c, base, base, sp);
        }
    }
    true
}

/// 0x1facf0 pulse_timer_step(): advances the gp-relative pulse timer one frame. Phase 0 slews the value up to 1.0
/// and moves to phase 1 when it arrives; phase 1 moves to phase 2; phase 2 slews back to 0.0 and clears the
/// active flag on arrival.
pub fn pulse_timer_step(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    let active = gp.wrapping_sub(GP_ACTIVE);
    let phase_addr = gp.wrapping_sub(GP_PHASE);
    let step_addr = gp.wrapping_sub(GP_STEP);
    if c.r32(active) == 0 {
        c.ret(0);
        return true;
    }
    let phase = c.r32(phase_addr);
    match phase {
        0 => {
            let step = c.rf(step_addr);
            let (_, f0) = h::run(c, motion::slew_toward, &[SLEW_VALUE], &[F::ONE, step]);
            if c.rf(SLEW_VALUE) == F::ONE {
                c.w32(phase_addr, 1);
            }
            c.ret(1);
            c.fret(f0);
        }
        1 => {
            c.w32(phase_addr, 2);
            c.ret(2);
        }
        2 => {
            let step = c.rf(step_addr);
            let (_, f0) = h::run(c, motion::slew_toward, &[SLEW_VALUE], &[F::ZERO, step]);
            if c.rf(SLEW_VALUE) == F::ZERO {
                c.w32(active, 0);
            }
            c.ret(2);
            c.fret(f0);
        }
        _ => c.ret(2),
    }
    true
}

/// 0x1fada8 pulse_init_once(): on the first call (init halfword zero) marks it done and calls 0x1ebc48 with the
/// hero; later calls just return 0x140000.
pub fn pulse_init_once(c: &mut Cpu) -> bool {
    if c.r16(INIT_FLAG) as i16 != 0 {
        c.ret(0x0014_0000);
        return true;
    }
    c.w16(INIT_FLAG, 1);
    let hero = c.r32(HERO);
    let (v0, f0) = h::call(c, 0x001E_BC48, &[0x001F_ADE8, hero], &[]);
    c.ret(v0);
    c.fret(f0);
    true
}
