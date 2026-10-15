//! Slot attach helpers. Both look up a base pointer, attach a slot through the retail
//! routine 0x27e1f8 (size 0x400), and record the slot and a value in a 0x70-byte record.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const ATTACH_FN: u32 = 0x0027_e1f8;
const ATTACH_FLAGS: u32 = 0x400;
const SLOT_STRIDE_SHIFT: u32 = 5;

const RECORD_BASE: u32 = 0x0013_e550;
const RECORD_STRIDE: i32 = 0x70;
const RECORD_SLOT_OFF: u32 = 0x7e;
const RECORD_VALUE_OFF: u32 = 0x88;

// keyed variant: key byte at KEY_INDEX_BASE + a3 selects a word pointer in KEY_TABLE
const KEY_INDEX_BASE: u32 = 0x0019_7cc0;
const KEY_TABLE: u32 = 0x0019_7400;
const ENTRY_LIMIT_OFF: u32 = 0x0d;
const ENTRY_BASE_OFF: u32 = 0x28;

// bounded variant: slot must be below the word at COUNT_WORD; base is the word at BASE_WORD
const COUNT_WORD: u32 = 0x0015_f554;
const BASE_WORD: u32 = 0x0015_f5d4;

const V1: usize = 3;
const A0: usize = 4;
const A3: usize = 7;
const T0: usize = 8;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Attaches `slot` under `base` through 0x27e1f8 and records slot and value when it succeeds.
/// Returns the attach result (negative when it failed, then nothing is recorded).
fn attach_and_record(c: &mut Cpu, base: u32, slot: u32, value: u32) -> u32 {
    let a1 = c.arg(1);
    let entry = base.wrapping_add(slot << SLOT_STRIDE_SHIFT);
    let (ret, _) = h::call(c, ATTACH_FN, &[entry, a1, value, 0, ATTACH_FLAGS], &[]);
    set_gpr(c, A3, 0);
    set_gpr(c, T0, ATTACH_FLAGS);
    set_gpr(c, A0, ret);
    if (ret as i32) >= 0 {
        let rec = RECORD_BASE.wrapping_add((ret as i32).wrapping_mul(RECORD_STRIDE) as u32);
        c.w16(rec + RECORD_SLOT_OFF, slot as u16);
        c.w32(rec + RECORD_VALUE_OFF, value);
        set_gpr(c, V1, rec);
    }
    ret
}

/// 0x27e518 slot_attach_keyed(slot, _, value, key_off) -> slot record index or -1: the key byte at
/// 0x197cc0 + key_off picks a table entry; a slot below the entry's limit is attached under its base.
pub fn slot_attach_keyed(c: &mut Cpu) -> bool {
    let slot = c.arg(0);
    let value = c.arg(2);
    let key_addr = c.arg(3).wrapping_add(KEY_INDEX_BASE);
    set_gpr(c, A3, key_addr);
    let key = c.r8(key_addr) as u32;
    let entry = c.r32(KEY_TABLE.wrapping_add(key * 4));
    set_gpr(c, V1, entry);
    if entry == 0 {
        c.ret(u32::MAX);
        return true;
    }
    let limit = c.r8(entry + ENTRY_LIMIT_OFF) as i32;
    if (slot as i32) >= limit {
        c.ret(u32::MAX);
        return true;
    }
    let base = c.r32(entry + ENTRY_BASE_OFF);
    let ret = attach_and_record(c, base, slot, value);
    c.ret(ret);
    true
}

/// 0x27e5c8 slot_attach_bounded(slot, _, value) -> slot record index or -1: a slot below the count
/// word 0x15f554 is attached under the base word 0x15f5d4.
pub fn slot_attach_bounded(c: &mut Cpu) -> bool {
    let slot = c.arg(0);
    let value = c.arg(2);
    let count = c.r32(COUNT_WORD) as i32;
    if (slot as i32) >= count {
        c.ret(u32::MAX);
        return true;
    }
    let base = c.r32(BASE_WORD);
    let ret = attach_and_record(c, base, slot, value);
    c.ret(ret);
    true
}
