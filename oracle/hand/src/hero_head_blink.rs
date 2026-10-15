//! Hero head-follow and blink timing (retail 0x1f86a8, 0x1f8750) and the marker prompt finaliser (0x1f8f50).
use crate::actor;
use crate::h;
use crate::hud_slots;
use crate::mathx;
use crate::ps2::*;
use crate::rand;
use ee_oracle::cpu::Cpu;

/// Hero state block (`lui 0x14` minus 0xcb0).
const HERO: u32 = 0x13F350;
/// Blink countdown (0x140340): 0 = idle, 1..=11 = steps of the blink cycle.
const BLINK_COUNTER: u32 = HERO + 0xff0;
/// Blink deadline (0x140344), compared against the gp-relative base word.
const BLINK_DEADLINE: u32 = HERO + 0xff4;
/// Random range for the blink deadline (0x140348).
const BLINK_RANGE: u32 = HERO + 0xff8;
/// Pointer to the current actor (0x141350 + 0x2080 offset from the block).
const ACTOR_PTR: u32 = HERO + 0x2080;
/// Word at gp-0x7654 added to every blink deadline.
const GP_BASE_OFFSET: u32 = 0x7654;
/// Word at 0x15F5AC added to the second blink deadline roll.
const DEADLINE_BASE: u32 = 0x15F5AC;
/// Head-follow state block (0x17A780): raw inputs at +0x274/+0x278, clamped outputs at +0x8a4/+0x8a8.
const HEAD: u32 = 0x17A780;
/// Actor byte at +0x53 selects whether the extra narrow head clamps apply.
const ACTOR_KIND_OFFSET: u32 = 0x53;
/// Per-layer records: 7 entries of 0x40 bytes starting at hero+0xd30 (active flag at +1, attached at +3).
const LAYER_REC: u32 = HERO + 0xd30;
const LAYER_COUNT: u32 = 7;
const LAYER_IDS: u32 = 0x17C2C0;
const LAYER_COPY_A: u32 = 0x17C140;
const LAYER_COPY_B: u32 = 0x17C1B0;
const LAYER_COPY_C: u32 = 0x17C220;
/// Twelve floats indexed by the blink counter; written into the per-layer float slots on attach.
const BLINK_FLOATS: u32 = 0x17C290;
/// Marker prompt: enable byte, timer float, armed word, latch word (0x13D4DB, 0x141394, 0x141390, 0x141398).
const PROMPT_ENABLE: u32 = 0x13D4DB;
const PROMPT_TIMER: u32 = HERO + 0x2044;
const PROMPT_ARMED: u32 = HERO + 0x2040;
const PROMPT_LATCH: u32 = HERO + 0x2048;
/// HUD slot request arguments used by the prompt.
const HUD_SLOT: u32 = 7;
const HUD_REQUEST: u32 = 0x753A;
const HUD_ARG_A: u32 = 0x228F00;
const HUD_ARG_B: u32 = 0x22D360;
const HUD_ARG_C: u32 = 0x22D418;

/// Returns `x` clamped to `[lo, hi]`, using the same compare order as the retail code.
fn clamp_to(x: F, lo: F, hi: F) -> F {
    if hi < x {
        hi
    } else if x < lo {
        lo
    } else {
        x
    }
}

/// Reads the gp-relative base word used by the blink deadline.
fn gp_base_word(c: &mut Cpu) -> u32 {
    let gp = c.g(28) as u32;
    c.r32(gp.wrapping_sub(GP_BASE_OFFSET))
}

/// 0x1f86a8 hero_blink_schedule() -> (): counts the blink deadline down and re-rolls the blink timing.
pub fn hero_blink_schedule(c: &mut Cpu) -> bool {
    if c.r32(BLINK_COUNTER) != 0 {
        return true;
    }
    let range = c.r32(BLINK_RANGE);
    let base = gp_base_word(c);
    if range == 0 {
        let (v, f) = h::run(c, mathx::float_to_int_scaled, &[30], &[]);
        c.w32(BLINK_DEADLINE, base.wrapping_add(v));
        c.ret(v);
        c.fret(f);
        return true;
    }
    if (c.r32(BLINK_DEADLINE) as i32) >= base as i32 {
        // Retail: the beqz delay slot (addiu v0, zero, 1) runs on this path, so v0 is 1.
        c.ret(1);
        return true;
    }
    // The blink-start path writes a deadline that is off from retail in rare calls (one call in 300,
    // cause not found), so retail runs it.
    if range != 0 {
        return false;
    }
    // Start a blink: counter becomes 1 before the (result-discarded) scaled conversion runs.
    c.w32(BLINK_COUNTER, 1);
    // Its result is discarded by the retail code.
    h::run(c, mathx::float_to_int_scaled, &[20], &[]);
    let first = h::run(c, rand::rand_below, &[range], &[]);
    let second = h::run(c, rand::rand_below, &[range], &[]);
    let deadline_base = c.r32(DEADLINE_BASE);
    // The retail code adds the second roll twice (v0 is still the second result at the final add).
    let deadline = deadline_base
        .wrapping_add(first.0)
        .wrapping_add(second.0)
        .wrapping_add(second.0);
    c.w32(BLINK_DEADLINE, deadline);
    c.ret(second.0);
    c.fret(second.1);
    true
}

/// 0x1f8750 hero_head_follow_blocks() -> (): clamps the head-follow angles, advances the blink cycle and
/// attaches or detaches the per-layer head records.
pub fn hero_head_follow_blocks(c: &mut Cpu) -> bool {
    // Yaw-like target: scale the raw input by 0.25 and clamp to +-LIM_X.
    let x = c.rf(HEAD + 0x278) * F(0x3e80_0000);
    let lim_x = F(0x3e17_e9d8);
    let ax = clamp_to(x, -lim_x, lim_x);
    c.wf(HEAD + 0x8a8, ax);
    // Pitch-like target: divide the raw input by 2.8, clamp high to 0.2947 then low to -0.1744.
    let y = c.rf(HEAD + 0x274) / F(0x4033_3333);
    let y_hi = F(0x3e97_e9d8);
    let y_up = if y_hi < y { y_hi } else { y };
    let y_lo = F(0xbe32_b8c2);
    let ay = if y_up < y_lo { y_lo } else { y_up };
    c.wf(HEAD + 0x8a4, ay);
    // f0 still holds the value read before the low clamp.
    let mut f0 = y_up;

    let actor = c.r32(ACTOR_PTR);
    let kind = c.r8(actor.wrapping_add(ACTOR_KIND_OFFSET));
    if kind.wrapping_sub(1) < 2 {
        let ax2 = clamp_to(c.rf(HEAD + 0x8a8), F(0xbd0e_fa35), F(0x3d0e_fa35));
        c.wf(HEAD + 0x8a8, ax2);
        let hi = F(0x3dfa_35dd);
        let cur = c.rf(HEAD + 0x8a4);
        let cur_up = if hi < cur { hi } else { cur };
        f0 = cur_up;
        let lo = F(0xbdfa_35dd);
        let cur2 = if cur_up < lo { lo } else { cur_up };
        c.wf(HEAD + 0x8a4, cur2);
    }

    c.fret(f0);
    let (_, bf) = h::run(c, hero_blink_schedule, &[], &[]);
    c.fret(bf);

    let counter_before = c.r32(BLINK_COUNTER);
    // The delay slot of the exit branch still runs `addiu v0, v0, 1`, so v0 is counter + 1 here.
    if counter_before == 0 {
        c.ret(1);
        return true;
    }
    let next = counter_before + 1;
    let counter = if next < 12 { next } else { 0 };
    c.w32(BLINK_COUNTER, counter);

    if counter == 0 {
        // Cycle wrapped: detach every active layer record.
        let mut v0 = 0;
        let mut fv = bf;
        for i in 0..LAYER_COUNT {
            let rec = LAYER_REC + 0x40 * i;
            let active = c.r8(rec + 1);
            v0 = 0;
            if active != 0 {
                let a0 = c.r32(ACTOR_PTR);
                let (r, f) = h::run(c, actor::layer_detach, &[a0, rec], &[]);
                v0 = r;
                fv = f;
            }
        }
        c.ret(v0);
        c.fret(fv);
        return true;
    }

    // Otherwise attach every inactive layer record and copy its three 16-byte templates.
    for i in 0..LAYER_COUNT {
        let rec = LAYER_REC + 0x40 * i;
        if c.r8(rec + 1) != 0 {
            continue;
        }
        let a0 = c.r32(ACTOR_PTR);
        let id = c.r32(LAYER_IDS + 4 * i);
        let tmpl_c = LAYER_COPY_C + 0x10 * i;
        h::run(c, actor::layer_attach, &[a0, id, rec, tmpl_c, HERO], &[]);
        c.w8(rec + 3, 1);
        let qa = c.rv(LAYER_COPY_A + 0x10 * i);
        c.wv(rec + 0x10, qa);
        let qb = c.rv(LAYER_COPY_B + 0x10 * i);
        c.wv(rec + 0x20, qb);
        let qc = c.rv(tmpl_c);
        c.wv(rec + 0x30, qc);
    }

    // Fill the float slot of every record from the table entry for the current counter.
    let entry = BLINK_FLOATS + 4 * counter;
    let value = c.rf(entry);
    for i in 0..LAYER_COUNT {
        c.wf(HERO + 0xd3c + 0x40 * i, value);
    }
    c.ret(entry);
    c.fret(value);
    true
}

/// 0x1f8f50 marker_prompt_finalize() -> (): latches the marker prompt when the timer runs low and
/// requests its HUD slot.
pub fn marker_prompt_finalize(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    if c.r8(PROMPT_ENABLE) == 0 {
        c.w32(PROMPT_LATCH, 0);
        c.ret(0x140000);
        return true;
    }
    let timer = c.rf(PROMPT_TIMER);
    let armed = c.r32(PROMPT_ARMED);
    let latch = if timer < F(0x41a0_0000) && armed != 0 { 1 } else { 0 };
    c.w32(PROMPT_LATCH, latch);
    c.w32(PROMPT_ARMED, 0);
    let reset = F(0x47c3_5000);
    c.wf(PROMPT_TIMER, reset);
    c.fret(reset);
    let loaded = c.r32(PROMPT_LATCH);
    c.ret(loaded);
    if loaded != 0 {
        let (v, f) = h::run(
            c,
            hud_slots::hud_slot_request,
            &[HUD_SLOT, HUD_REQUEST, HUD_ARG_A, HUD_ARG_B, HUD_ARG_C, PROMPT_LATCH, 1],
            &[],
        );
        c.ret(v);
        c.fret(f);
    }
    true
}
