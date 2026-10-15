//! Voice slot helpers: `voice_pick_slot` (0x27e668) picks an index from a 2-D table and claims a voice slot,
//! `voice_set_level` (0x27e738) and `voice_set_aux` (0x27e758) store one word into a voice slot, and
//! `voice_find_entry` (0x27e778) looks a key up in an 8-byte-entry table.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const SP: usize = 29;
const S0: usize = 16;
const S1: usize = 17;
const RA: usize = 31;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;

/// Voice slot array (0x140000 - 0x1ab0), 0x70 bytes per slot.
const SLOTS: u32 = 0x0013_E550;
const SLOT_SIZE: u32 = 0x70;
/// Slot word written by `voice_set_level` (+0x80) and `voice_set_aux` (+0x84).
const LEVEL_OFF: u32 = 0x80;
const AUX_OFF: u32 = 0x84;
/// Slot claim routine called by `voice_pick_slot` (retail 0x27e1f8).
const CLAIM: u32 = 0x0027_E1F8;
/// Flags argument of the claim call.
const CLAIM_FLAGS: u32 = 0x400;
/// Word holding the table pointer offset used by `voice_pick_slot` (0x160000 - 0x127c), and the offset added to it
/// (0x1c0000 - 0x26e0).
const TABLE_PTR_WORD: u32 = 0x0015_ED84;
const TABLE_OFF: u32 = 0x001B_D920;
/// Weight word added to the index (0x160000 - 0xaac).
const WEIGHT_WORD: u32 = 0x0015_F554;
/// Limit word the index is compared against (0x160000 - 0xa30).
const LIMIT_WORD: u32 = 0x0015_F5D0;
/// Base word of the claim call's first argument (0x160000 - 0xa2c).
const CLAIM_BASE_WORD: u32 = 0x0015_F5D4;
/// Key table of `voice_find_entry` (0x1e0000 - 0x2e80): a header entry, then 8-byte entries.
const KEY_TABLE: u32 = 0x001D_D180;
const KEY_TABLE_HI: u32 = 0x001E_0000;
const NONE: u32 = 0xffff_ffff;

/// Writes a general register with the sign-extended 32-bit value `v`, as the 32-bit EE ops leave it.
fn put(c: &mut Cpu, reg: usize, v: u32) {
    c.r[reg] = v as i32 as i128 as u128;
}

/// Stores a 128-bit register as `sq` does: four words, lowest lane first.
fn save_quad(c: &mut Cpu, addr: u32, reg: usize) {
    let v = c.r[reg];
    for i in 0..4u32 {
        c.w32(addr + 4 * i, (v >> (32 * i)) as u32);
    }
}

/// 0x27e668 voice_pick_slot(a, b, c, weight, key) -> slot index, or -1 when the sum of the table byte, a*4, b*2, c and
/// the weight reaches the limit. Otherwise claims a voice slot through 0x27e1f8 and records the index (halfword at
/// +0x7e) and key (word at +0x88) in it; the result is the claim's return value.
pub fn voice_pick_slot(c: &mut Cpu) -> bool {
    let (a, b, cc, d, key) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3), c.arg(4));
    let sp = (c.r[SP] as u32).wrapping_sub(0x30);
    save_quad(c, sp, S0);
    save_quad(c, sp + 0x10, S1);
    save_quad(c, sp + 0x20, RA);
    put(c, T1, d);
    let table = c.r32(TABLE_PTR_WORD).wrapping_add(TABLE_OFF);
    let byte = c.r8(table) as u32;
    let weight = c.r32(WEIGHT_WORD);
    let idx = byte.wrapping_add(a << 2).wrapping_add(b << 1).wrapping_add(cc).wrapping_add(weight);
    let limit = c.r32(LIMIT_WORD);
    if (idx as i32) >= (limit as i32) {
        put(c, A0, a << 2);
        put(c, A1, b << 1);
        put(c, A2, cc);
        put(c, A3, weight);
        put(c, V1, 0);
        c.ret(NONE);
        return true;
    }
    let row = c.r32(CLAIM_BASE_WORD).wrapping_add(idx << 5);
    let (ret, _) = h::call_keep(c, CLAIM, &[row, d, key, 0, CLAIM_FLAGS], &[]);
    let slot_index = ret as i32;
    put(c, A0, ret);
    if slot_index < 0 {
        put(c, V1, SLOT_SIZE);
    } else {
        let slot = SLOTS.wrapping_add(ret.wrapping_mul(SLOT_SIZE));
        put(c, V1, slot);
        c.w16(slot + 0x7e, idx as u16);
        c.w32(slot + 0x88, key);
    }
    c.ret(ret);
    true
}

/// Stores `val` at word `off` of voice slot `slot`, and returns 1.
fn store_slot_word(c: &mut Cpu, off: u32) -> bool {
    let (slot, val) = (c.arg(0), c.arg(1));
    let addr = SLOTS.wrapping_add(slot.wrapping_mul(SLOT_SIZE));
    put(c, V1, SLOTS);
    put(c, A0, addr);
    c.w32(addr + off, val);
    c.ret(1);
    true
}

/// 0x27e738 voice_set_level(slot, level) -> 1: writes the level word (+0x80) of voice slot `slot`.
pub fn voice_set_level(c: &mut Cpu) -> bool {
    store_slot_word(c, LEVEL_OFF)
}

/// 0x27e758 voice_set_aux(slot, value) -> 1: writes the word at +0x84 of voice slot `slot`.
pub fn voice_set_aux(c: &mut Cpu) -> bool {
    store_slot_word(c, AUX_OFF)
}

/// 0x27e778 voice_find_entry(rec) -> entry address: the key is the signed halfword at rec+0. The search starts at
/// the second entry (the first is a header whose key word is compared too: -1 or the key ends it at index 0). It
/// returns the index of the matching entry (or of the -1 terminator, counted as one past the last compared entry) as
/// a halfword at rec+2, and the entry's value word at rec+4.
pub fn voice_find_entry(c: &mut Cpu) -> bool {
    let rec = c.arg(0);
    put(c, T0, KEY_TABLE_HI);
    put(c, A2, rec);
    let head = c.r32(KEY_TABLE);
    let mut idx: u32 = 0;
    if head != NONE {
        let key = c.r16(rec) as i16 as i32 as u32;
        put(c, A0, key);
        if head != key {
            put(c, A3, NONE);
            let mut p = KEY_TABLE + 8;
            loop {
                let w = c.r32(p);
                if w == NONE {
                    idx += 1;
                    break;
                }
                idx += 1;
                p += 8;
                if w == key {
                    break;
                }
            }
        }
    }
    let entry = KEY_TABLE.wrapping_add(idx << 3);
    put(c, A0, idx << 3);
    put(c, A1, idx);
    c.w16(rec + 2, idx as u16);
    let value = c.r32(entry + 4);
    put(c, V1, value);
    c.w32(rec + 4, value);
    c.ret(entry);
    true
}
