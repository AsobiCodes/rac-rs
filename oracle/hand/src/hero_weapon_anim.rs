//! Hero weapon and held-animation helpers (retail 0x21cbd8, 0x224e48, 0x2256e0): start the hold-fire idle on the active
//! slot, start the two held-record animations, and load the tracks of the hero anim record.

use crate::glove_slot;
use crate::h;
use crate::hero_anim;
use crate::mathx;
use crate::objects;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Base of the hero global block (retail `s0` / `v0` after `lui 0x14`, 0x140000 - 0xcb0).
const BLOCK: u32 = 0x0013_F350;
/// Hero anim record pointer (0x140000 + 0x13d0).
const RECORD_PTR: u32 = 0x0014_13D0;
/// Retail anim-track loader (0x241140), kept retail so its register leftovers match.
const ANIM_LOAD_TRACKS: u32 = 0x0024_1140;
/// Enable flags for the hold-fire idle, and the halfword gate that must be zero.
const IDLE_FLAG_A: u32 = 0x20a8;
const IDLE_FLAG_B: u32 = 0x20aa;
const IDLE_GATE: u32 = 0x22c8;
/// Slot id the active slot value must match, and the anim word the idle stores.
const IDLE_SLOT_ID: u32 = 0x2298;
const IDLE_ANIM_OUT: u32 = 0x2294;
/// Flag that, when set, latches a value into `IDLE_LATCH`.
const IDLE_FLAG_D08: u32 = 0xd08;
const IDLE_LATCH: u32 = 0xd14;
/// Table of 0x4c-byte slot records (retail 0x179bc0 = 0x180000 - 0x6440) and the anim word inside each.
const SLOT_TABLE: u32 = 0x0017_9BC0;
const SLOT_STRIDE: u32 = 0x4c;
/// Word the slot call loads into v1 (block + 0x10b4), used as the record stride when the slot matches.
const SLOT_STRIDE_WORD: u32 = 0x10B4;
const SLOT_ANIM_OFF: u32 = 0x24;
/// Scale fed to `float_to_int_scaled` for the idle, and for the held pair.
const IDLE_SCALE: u32 = 11;
const PAIR_SCALE: u32 = 7;
/// Held-record pointers in the block, and the sequence flag byte inside a held record.
const PAIR_A: u32 = 0x1180;
const PAIR_B: u32 = 0x1184;
const PAIR_SEQ_FLAG: u32 = 0x53;

/// 0x21cbd8 hero_weapon_idle() -> v0: hold-fire idle; starts the hero anim of the active slot (v0 = 1 when started)
pub fn hero_weapon_idle(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    frame_ra_s0_s1(c, 0x30, |c| {
        let v = idle_body(c);
        c.ret(v);
    });
    true
}

/// Frame of 0x30 bytes with s0, s1 and ra saved at 0x0, 0x10 and 0x20 (retail prologue).
fn frame_ra_s0_s1(c: &mut Cpu, size: u32, body: impl FnOnce(&mut Cpu)) {
    h::frame(c, size, &[(16, 0x0, h::SQ), (17, 0x10, h::SQ), (31, 0x20, h::SQ)], |c, _| body(c));
}

fn idle_body(c: &mut Cpu) -> u32 {
    if c.r8(BLOCK + IDLE_FLAG_A) == 0 || c.r8(BLOCK + IDLE_FLAG_B) == 0 || c.r16(BLOCK + IDLE_GATE) != 0 {
        return 0;
    }
    let slot = h::run(c, glove_slot::slot_value_b_fn, &[0], &[]).0;
    if slot != c.r32(BLOCK + IDLE_SLOT_ID) {
        // Retail sets v1 = 0x4c before the call, and the callee sees it.
        c.s(3, SLOT_STRIDE as u64);
        h::call_keep(c, 0x201740, &[slot], &[]);
        return 0;
    }
    // Retail: the slot call's own `lw v1, 0x10b4(a0)` leaves v1 = that word, and the mult below uses v1 as the stride.
    let stride = c.r32(BLOCK + SLOT_STRIDE_WORD);
    let record = SLOT_TABLE.wrapping_add(slot.wrapping_mul(stride));
    let seq = c.r32(record + SLOT_ANIM_OFF);
    if h::call_keep(c, 0x215538, &[0, 0], &[]).0 == 0 {
        return 0;
    }
    let scaled = h::run(c, mathx::float_to_int_scaled, &[IDLE_SCALE], &[]).0;
    h::run(c, hero_anim::hero_set_anim, &[seq, 0], &[F::from_int(scaled as i32)]);
    if c.r32(BLOCK + IDLE_FLAG_D08) != 0 {
        c.w32(BLOCK + IDLE_LATCH, 1);
    }
    c.w32(BLOCK + IDLE_ANIM_OUT, seq);
    1
}

/// 0x224e48 held_anim_pair() -> v0: starts the anims of both held records (0 if the pick fails, 1 if a flag is off)
pub fn held_anim_pair(c: &mut Cpu) -> bool {
    h::frame(
        c,
        0x50,
        &[(16, 0x10, h::SQ), (17, 0x20, h::SQ), (18, 0x30, h::SQ), (31, 0x40, h::SQ)],
        |c, sp| {
            let v = pair_body(c, sp);
            c.ret(v);
        },
    );
    true
}

fn pair_body(c: &mut Cpu, sp: u32) -> u32 {
    let s1 = c.r32(BLOCK + PAIR_A);
    let s2 = c.r32(BLOCK + PAIR_B);
    if s1 == 0 {
        return BLOCK;
    }
    if s2 == 0 {
        // Retail's delay slot of `beqz s2` is `addiu v0, zero, 1`.
        return 1;
    }
    if c.r8(s1 + PAIR_SEQ_FLAG) != 1 {
        return 1;
    }
    if h::run(c, hero_anim::held_seq_pick_hand, &[0, sp, sp + 4], &[]).0 == 0 {
        return 0;
    }
    let anim_a = c.r32(sp);
    let scale_a = h::run(c, mathx::float_to_int_scaled, &[PAIR_SCALE], &[]).0;
    h::run(c, objects::anim_start, &[s1, anim_a, 0, scale_a], &[]);
    let anim_b = c.r32(sp + 4);
    let scale_b = h::run(c, mathx::float_to_int_scaled, &[PAIR_SCALE], &[]).0;
    h::run(c, objects::anim_start, &[s2, anim_b, 0, scale_b], &[]).0
}

/// 0x2256e0 anim_track_setup(a0, a1): stores a0 and a1 (+1 for the count) in the hero anim record, clears its
/// word at +0x54, then loads the record's tracks
pub fn anim_track_setup(c: &mut Cpu) -> bool {
    let (a0, a1) = (c.arg(0), c.arg(1));
    h::frame(c, 0x10, &[(31, 0x0, h::SQ)], |c, _| {
        let rec = c.r32(RECORD_PTR);
        c.w8(rec + 0x51, a1.wrapping_add(1) as u8);
        c.w8(rec + 0x53, a0 as u8);
        c.w8(rec + 0x50, a1 as u8);
        c.w8(rec + 0x52, a0 as u8);
        c.w32(rec + 0x54, 0);
        h::call_keep(c, ANIM_LOAD_TRACKS, &[rec, a1, a0], &[]);
    });
    true
}
