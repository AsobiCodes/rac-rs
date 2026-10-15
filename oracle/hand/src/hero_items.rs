//! Hero item slots: the per-frame slot loop (seven 0x50-byte records in the state block, each pointing at an item
//! object), the frame update that calls it, and the small actor-kind test used by the slot code.
use crate::glow;
use crate::h;
use crate::objects;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Global state block (lui 0x14, addiu -0xcb0).
const STATE: u32 = 0x0013_F350;
/// Item slot records: count, stride, and the per-record fields.
const SLOT_COUNT: u32 = 7;
const SLOT_STRIDE: u32 = 0x50;
/// Item object pointer (word) of a slot record.
const SLOT_OBJECT: u32 = 0x1090;
/// Slot mode (word): 2 = equipped tick, 3 = draw tick.
const SLOT_MODE: u32 = 0x10B4;
/// Tick counter for mode 2 (word).
const SLOT_TICKS: u32 = 0x10B0;
/// Draw tick counter for mode 3 (byte).
const SLOT_DRAW_TICKS: u32 = 0x10AB;
/// Item object fields: flags byte, anim-lock byte, hook pointer.
const OBJ_FLAGS: u32 = 0x20;
const OBJ_ANIM_LOCK: u32 = 0x53;
const OBJ_STATE_BYTE: u32 = 0x70;
const OBJ_HOOK: u32 = 0x74;
/// Object flags that skip the hook call.
const FLAG_SKIP_A: u8 = 0xFE;
const FLAG_SKIP_B: u8 = 0xFD;
/// Slot index whose hook is never called.
const LAST_SLOT_NO_HOOK: u32 = 6;
/// Table of 0xB0-byte entries that are reset when the first slot is in mode 2.
const RESET_TABLE_END: u32 = STATE + 0x210;
const RESET_TABLE_KEY: u32 = 0x1990;
const RESET_TABLE_FLAG: u32 = 0x1992;
const RESET_TABLE_ITEM: u32 = STATE + 0x18F0;
const RESET_ENTRY_STRIDE: u32 = 0xB0;
const RESET_VALUE: u16 = 5;
/// Global word (gp-relative offset -0x7808) read by the mode 3 path.
const GP_WORD_OFFSET: u32 = 0x7808;
/// Global word at 0x15F3F8 read by the mode 3 path for slot 0.
const SLOT_ZERO_WORD: u32 = 0x0015_F3F8;
/// Frame state words: a hold word, a kind word, and an actor word.
const STATE_HOLD: u32 = 0x0000_0308;
const STATE_KIND: u32 = 0x0000_208C;
const STATE_ACTOR: u32 = 0x0000_2084;
const FLAG_A: u32 = 0x20AC;
const FLAG_B: u32 = 0x20A7;
const FLAG_C: u32 = 0x20AE;
/// Kind values that set FLAG_C directly; the actor value that also does.
const KIND_LO: u32 = 0x15;
const KIND_HI: u32 = 0x16;
const KIND_THREE: u32 = 3;
const ACTOR_KIND: u32 = 0x12;
/// Actor kind (halfword at +0xA6) matched by `actor_kind_is_47`.
const ACTOR_KIND_47: i16 = 0x47;
const ACTOR_KIND_OFFSET: u32 = 0xA6;

/// Retail routines called by the slot code and the frame update.
const RETAIL_SLOT_EMPTY: u32 = 0x0020_2F48;
const RETAIL_SLOT_ADD: u32 = 0x0020_2D50;
const RETAIL_SLOT_RESET: u32 = 0x001F_8210;
const RETAIL_PREDICATE: u32 = 0x0020_0610;
const RETAIL_FRAME_A: u32 = 0x0020_1B28;
const RETAIL_FRAME_B: u32 = 0x0020_2628;
const RETAIL_FRAME_C: u32 = 0x001F_B158;

/// Mode 2: bumps the tick counter, calls the empty-slot hook, resets the table on slot 0, and starts the
/// item's animation when its state bit 2 is set and the anim lock is clear.
fn equipped_tick(c: &mut Cpu, slot_index: u32, obj: u32, slot: u32) {
    let ticks = c.r32(slot + SLOT_TICKS).wrapping_add(1);
    c.w32(slot + SLOT_TICKS, ticks);
    h::call(c, RETAIL_SLOT_EMPTY, &[slot_index], &[]);
    if slot_index == 0 {
        reset_table(c);
    }
    if c.r8(obj + OBJ_STATE_BYTE) & 2 != 0 && c.r8(obj + OBJ_ANIM_LOCK) == 0 {
        h::run(c, objects::anim_start, &[obj, 1, 0, 2], &[]);
    }
}

/// Resets every reset-table entry whose key is not -1 (sets its flag to 5 and calls the retail slot reset).
fn reset_table(c: &mut Cpu) {
    let mut entry = STATE;
    let mut item = RESET_TABLE_ITEM;
    while entry < RESET_TABLE_END {
        if c.r16(entry + RESET_TABLE_KEY) as i16 != -1 {
            c.w16(entry + RESET_TABLE_FLAG, RESET_VALUE);
            h::call(c, RETAIL_SLOT_RESET, &[item], &[]);
        }
        entry += RESET_ENTRY_STRIDE;
        item += RESET_ENTRY_STRIDE;
    }
}

/// Mode 3: bumps the draw tick byte and calls the retail slot add with the global word plus 2 (slot 0 uses its own
/// word, and slot 1 or a flagged item always uses the global word).
fn draw_tick(c: &mut Cpu, slot_index: u32, obj: u32, slot: u32) {
    let draw_ticks = c.r8(slot + SLOT_DRAW_TICKS).wrapping_add(1);
    c.w8(slot + SLOT_DRAW_TICKS, draw_ticks);
    let gp = c.g(28) as u32;
    let global = c.r32(gp.wrapping_sub(GP_WORD_OFFSET)).wrapping_add(2);
    if c.r8(obj + OBJ_STATE_BYTE) & 2 != 0 || slot_index == 1 {
        h::call(c, RETAIL_SLOT_ADD, &[slot_index, global], &[]);
    } else if slot_index == 0 {
        let zero_word = c.r32(SLOT_ZERO_WORD).wrapping_add(2);
        h::call(c, RETAIL_SLOT_ADD, &[0, zero_word], &[]);
    }
}

/// 0x2037f0 hero_item_slots_tick() -> void: per-frame loop over the seven item slots; each slot with an item runs its
/// mode handler, then (unless the item flags say skip, or it is the last slot) calls the item's hook
pub fn hero_item_slots_tick(c: &mut Cpu) -> bool {
    for slot_index in 0..SLOT_COUNT {
        let slot = STATE + slot_index * SLOT_STRIDE;
        let obj = c.r32(slot + SLOT_OBJECT);
        if obj == 0 {
            h::call(c, RETAIL_SLOT_EMPTY, &[slot_index], &[]);
            continue;
        }
        match c.r32(slot + SLOT_MODE) {
            2 => equipped_tick(c, slot_index, obj, slot),
            3 => draw_tick(c, slot_index, obj, slot),
            _ => {}
        }
        let flags = c.r8(obj + OBJ_FLAGS);
        if flags == FLAG_SKIP_A || flags == FLAG_SKIP_B || slot_index == LAST_SLOT_NO_HOOK {
            continue;
        }
        let hook = c.r32(obj + OBJ_HOOK);
        if hook != 0 {
            h::call(c, hook, &[obj], &[]);
        }
    }
    c.ret(0);
    true
}

/// 0x2039d0 hero_frame_update() -> void: sets the hero flag words, runs the frame helpers and the item slot tick
pub fn hero_frame_update(c: &mut Cpu) -> bool {
    let mut flag = c.r16(STATE + STATE_HOLD) as i16 == 1;
    if !flag {
        let kind = c.r32(STATE + STATE_KIND);
        if kind.wrapping_sub(KIND_LO) < 2 {
            flag = true;
        } else {
            flag = h::call(c, RETAIL_PREDICATE, &[], &[]).0 != 0;
        }
    }
    if flag {
        c.w8(STATE + FLAG_A, 1);
        c.w8(STATE + FLAG_B, 1);
    }
    if h::call(c, RETAIL_PREDICATE, &[], &[]).0 != 0 {
        c.w8(STATE + FLAG_C, 1);
    } else {
        let kind = c.r32(STATE + STATE_KIND);
        let ready = matches!(kind, KIND_LO | KIND_HI | KIND_THREE) || c.r32(STATE + STATE_ACTOR) == ACTOR_KIND;
        c.w8(STATE + FLAG_C, ready as u8);
    }
    h::call(c, RETAIL_FRAME_A, &[], &[]);
    h::call(c, RETAIL_FRAME_B, &[], &[]);
    h::run(c, glow::glow_pulse_update, &[], &[]);
    h::call(c, RETAIL_FRAME_C, &[], &[]);
    hero_item_slots_tick(c);
    c.ret(0);
    true
}

/// 0x204408 actor_kind_is_47(obj) -> u32: 1 when obj is non-null and its kind halfword (+0xA6) is 0x47, else 0
pub fn actor_kind_is_47(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let is_47 = obj != 0 && c.r16(obj.wrapping_add(ACTOR_KIND_OFFSET)) as i16 == ACTOR_KIND_47;
    c.ret(is_47 as u32);
    true
}
