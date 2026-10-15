//! Hero rig update (mode-switched layout and sequence calls over the record at 0x179510) and the hero idle
//! sequence selector over the Veldin hero record at 0x13f350, plus a sequence identity helper.
use crate::glove_slot;
use crate::h;
use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

/// Rig state record: 0x180000 - 0x6af0.
const RIG: u32 = 0x0018_0000 - 0x6af0;
/// Byte gate for the rig update, read gp-relative: gp - 0x7de3.
const GATE_GP_OFF: u32 = 0x7de3;
/// Pointer word (at 0x15f640) to the 0x10-byte entry table used by the mode 4-6 sequence.
const ENTRY_TABLE_PTR: u32 = 0x0016_0000 - 0x09c0;

/// Retail helper that places the rig rectangle (args a0..a3, t0).
const LAYOUT_HELPER: u32 = 0x001e_d6c0;
/// Retail helper called in rig mode 2.
const MODE2_HELPER: u32 = 0x001f_7770;
/// Retail helper returning a scale word (called with a0 = 4).
const SCALE_HELPER: u32 = 0x001e_bb00;
/// Retail helper taking two packed 64-bit stack values.
const PACK_HELPER: u32 = 0x001e_caf8;
/// Retail sequence setup helper (modes 4-6).
const SEQ_SETUP_HELPER: u32 = 0x001e_eab8;
/// Retail sequence submit helper (modes 4-6).
const SEQ_SUBMIT_HELPER: u32 = 0x001e_ea40;

/// Veldin hero record: 0x140000 - 0xcb0.
const HERO: u32 = 0x0014_0000 - 0x0cb0;
/// Hero mode byte.
const HERO_MODE_OFF: u32 = 0x20a4;
/// Hero state word (1 selects the fixed 0x54 sequence).
const HERO_STATE_OFF: u32 = 0x22a8;
/// Hero health-like byte; nonzero is returned directly.
const HERO_HEALTH_OFF: u32 = 0x12e2;
/// Hero flag bytes checked before the glove lookup.
const HERO_FLAG_A_OFF: u32 = 0x20a8;
const HERO_FLAG_B_OFF: u32 = 0x20aa;
/// Hero halfword that must be zero for the glove lookup.
const HERO_HALF_OFF: u32 = 0x22c8;

/// Glove record table: 0x180000 - 0x6440, 0x4c-byte records, value at +0x24.
const GLOVE_TABLE: u32 = 0x0018_0000 - 0x6440;
const GLOVE_STRIDE: u32 = 0x4c;
const GLOVE_VALUE_OFF: u32 = 0x24;

/// Sequence ids returned by the idle selector.
const SEQ_FIXED_A: u32 = 0x54;
const SEQ_FIXED_B: u32 = 0x6e;

/// Adjust-and-shift used by the rig rounding (round toward zero for negative values).
fn adj_shift(x: i32) -> i32 {
    (if x >= 0 { x } else { x + 7 }) >> 3
}

/// Sign-extension high word of a 32-bit value (as a 64-bit register would store it).
fn sext_hi(x: u32) -> u32 {
    if (x as i32) < 0 { 0xffff_ffff } else { 0 }
}

fn rig(c: &mut Cpu, off: u32) -> u32 {
    c.r32(RIG + off)
}

fn set_rig(c: &mut Cpu, off: u32, v: u32) {
    c.w32(RIG + off, v);
}

fn layout(c: &mut Cpu, a0: u32, a1: u32, a2: u32, a3: u32) {
    h::call(c, LAYOUT_HELPER, &[a0, a1, a2, a3, 0x60], &[]);
}

/// Writes two 64-bit values to the stack: `lo` at sp+0 and `hi_val` at sp+8.
fn store_stack_pair(c: &mut Cpu, lo: u32, hi_val: u32) {
    let sp = c.g(29) as u32;
    c.w32(sp, lo);
    c.w32(sp + 4, sext_hi(lo));
    c.w32(sp + 8, hi_val);
    c.w32(sp + 12, sext_hi(hi_val));
}

/// Rig mode 1: layout with the stored offset v = (w4 << 2) + 8.
fn rig_mode1(c: &mut Cpu) {
    let v = (rig(c, 4) << 2).wrapping_add(8);
    let a2 = rig(c, 0x10);
    let a0 = rig(c, 0x14);
    set_rig(c, 0x18, v);
    set_rig(c, 0x1c, v);
    layout(c, a0.wrapping_sub(v), a0.wrapping_add(v), a2.wrapping_sub(v), a2.wrapping_add(v));
}

/// Rig mode 3: scaled layout, then the scale helper and a packed submit.
fn rig_mode3(c: &mut Cpu) {
    let w4 = rig(c, 4);
    let a3 = (rig(c, 8).wrapping_sub(0x20) as i32).wrapping_mul(w4 as i32);
    let a1 = (rig(c, 0xc).wrapping_sub(0x20) as i32).wrapping_mul(w4 as i32);
    let v1 = adj_shift(a3) + 0x20;
    let v0 = adj_shift(a1) + 0x20;
    let a0 = rig(c, 0x14);
    let a2 = rig(c, 0x10);
    set_rig(c, 0x18, v1 as u32);
    set_rig(c, 0x1c, v0 as u32);
    layout(c, a0.wrapping_sub(v0 as u32), a0.wrapping_add(v0 as u32), a2.wrapping_sub(v1 as u32), a2.wrapping_add(v1 as u32));

    let s0_raw = (8u32.wrapping_sub(w4) << 4) as i32;
    let s0 = if s0_raw >= 0 { s0_raw as u32 } else { 0 };
    let r = h::call(c, SCALE_HELPER, &[4], &[]).0;
    let packed = (s0 << 24) | 0x0080_8080;
    store_stack_pair(c, packed, r);
    let a0 = rig(c, 0x10).wrapping_sub(0x20);
    let a1 = rig(c, 0x14).wrapping_sub(0x20);
    h::call(c, PACK_HELPER, &[a0, a1, 0x40, 0x40, 0, 0, 0x40, 0x40], &[]);
}

/// Rig modes 4-6: layout from the 8-byte offsets, then the sequence setup and submit.
fn rig_mode456(c: &mut Cpu, mode: u32) {
    let w8 = rig(c, 8);
    let w12 = rig(c, 0xc);
    let w16 = rig(c, 0x10);
    let w20 = rig(c, 0x14);
    set_rig(c, 0x18, w8);
    set_rig(c, 0x1c, w12);
    layout(c, w20.wrapping_sub(w12), w20.wrapping_add(w12), w16.wrapping_sub(w8), w16.wrapping_add(w8));

    let w4 = rig(c, 4);
    let mut s2: u32 = 0x80ff_a888;
    if mode == 4 {
        s2 = (w4 << 29) | 0x00ff_a888;
    } else if mode == 6 {
        s2 = (4u32.wrapping_sub(w4) << 29) | 0x00ff_a888;
    }
    let table = c.r32(ENTRY_TABLE_PTR);
    let index = rig(c, 0x20);
    let entry = c.r32(table.wrapping_add(index << 4));
    let sp = c.g(29) as u32;
    c.w32(sp, 3);
    let buf = sp + 0x10;
    let w20_now = rig(c, 0x14);
    h::call(c, SEQ_SETUP_HELPER, &[buf, 0xf0, 0x1e0, 0x2c, 0x1d4, 0x100, w20_now, 0x10], &[]);
    h::call(c, SEQ_SUBMIT_HELPER, &[buf, s2, entry, u32::MAX], &[]);
}

/// Rig mode 7: inverse layout with the 12-scaled offset.
fn rig_mode7(c: &mut Cpu) {
    let w4 = rig(c, 4) as i32;
    let a0 = rig(c, 0x1c) as i32;
    let a2 = rig(c, 0x18) as i32;
    let a3 = (a0 - 8).wrapping_mul(w4);
    let a1 = (a2 - 8).wrapping_mul(w4);
    let t0 = (8 - w4).wrapping_mul(12);
    let t5 = rig(c, 0x10);
    let t4 = rig(c, 0x14);
    let a0 = a0 - adj_shift(a3);
    let a2 = a2 - adj_shift(a1);
    let (a0, a2) = (a0 as u32, a2 as u32);
    h::call(
        c,
        LAYOUT_HELPER,
        &[t4.wrapping_sub(a0), t4.wrapping_add(a0), t5.wrapping_sub(a2), t5.wrapping_add(a2), t0 as u32],
        &[],
    );
}

/// 0x1f7858 rig_update(): mode-switched rig layout and sequence submit for the rig record at 0x179510
pub fn rig_update(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    let gate = c.r8(gp.wrapping_sub(GATE_GP_OFF)) as u32;
    let mode = rig(c, 0);
    if mode == 0 || rig(c, 0x30) == 0 || gate == 0 {
        return true;
    }
    match mode {
        1 => rig_mode1(c),
        2 => {
            h::call(c, MODE2_HELPER, &[], &[]);
        }
        3 => rig_mode3(c),
        4..=6 => rig_mode456(c, mode),
        7 => rig_mode7(c),
        _ => {}
    }
    true
}

/// Idle sequence id from the hero record (0 = no sequence).
fn idle_seq(c: &mut Cpu, gate: u32) -> u32 {
    let mode = c.r8(HERO + HERO_MODE_OFF) as u32;
    let limit = if mode < 3 { 1 } else { 3 };
    if mode == 2 || mode == limit {
        return 0;
    }
    if c.r32(HERO + HERO_STATE_OFF) == 1 {
        return SEQ_FIXED_A;
    }
    // The delay slot of the health branch sets 0x6e, and the gate branch's delay slot clears the result.
    let health = c.r8(HERO + HERO_HEALTH_OFF) as u32;
    if health != 0 {
        return SEQ_FIXED_B;
    }
    if gate == 0 {
        return 0;
    }
    if c.r8(HERO + HERO_FLAG_A_OFF) == 0
        || c.r8(HERO + HERO_FLAG_B_OFF) == 0
        || c.r16(HERO + HERO_HALF_OFF) != 0
    {
        return 0;
    }
    let slot = h::run(c, glove_slot::slot_value_b_fn, &[0], &[]).0;
    c.r32(GLOVE_TABLE.wrapping_add(slot.wrapping_mul(GLOVE_STRIDE)).wrapping_add(GLOVE_VALUE_OFF))
}

/// 0x1f80a8 hero_idle_seq(gate) -> v0: idle sequence id from the Veldin hero record (0 = none)
pub fn hero_idle_seq(c: &mut Cpu) -> bool {
    let gate = c.arg(0);
    let v = idle_seq(c, gate);
    c.ret(v);
    true
}

/// 0x1f8160 seq_identity(x) -> v0: returns its argument
pub fn seq_identity(c: &mut Cpu) -> bool {
    let x = c.arg(0);
    c.ret(x);
    true
}
