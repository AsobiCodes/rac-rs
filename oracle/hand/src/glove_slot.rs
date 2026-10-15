//! Per-slot lookups over the 0x50-byte record table at 0x13f350 (EE RAM).
use ee_oracle::cpu::Cpu;
use crate::ps2::Ee;

/// Base of the slot record table: 0x140000 - 0xcb0.
const SLOT_TABLE: u32 = 0x0014_0000 - 0x0cb0;
/// Size of one slot record.
const SLOT_STRIDE: u32 = 0x50;
/// Record field: slot state word.
const STATE_OFF: u32 = 0x10b4;
/// State value meaning the slot is active.
const STATE_ACTIVE: u32 = 2;
/// Record field read when the slot is active (first value).
const VALUE_A_OFF: u32 = 0x1090;
/// Record field read when the slot is active (second value).
const VALUE_B_OFF: u32 = 0x10b8;
/// Returned by the second lookup when the slot is not active.
const INACTIVE: u32 = 0xffff_ffff;

/// Glove item ids (slot item is 10/17/20/25 for gloves).
const GLOVE_IDS: [u32; 4] = [0x11, 0x0a, 0x14, 0x19];

fn slot_record(index: u32) -> u32 {
    SLOT_TABLE.wrapping_add(index.wrapping_mul(SLOT_STRIDE))
}

fn slot_value_a(c: &mut Cpu, index: u32) -> u32 {
    let rec = slot_record(index);
    if c.r32(rec + STATE_OFF) == STATE_ACTIVE { c.r32(rec + VALUE_A_OFF) } else { 0 }
}

fn slot_value_b(c: &mut Cpu, index: u32) -> u32 {
    let rec = slot_record(index);
    if c.r32(rec + STATE_OFF) == STATE_ACTIVE { c.r32(rec + VALUE_B_OFF) } else { INACTIVE }
}

/// 0x200508 slot_value_a(index) -> v0: record value A if the slot is active, else 0
pub fn slot_value_a_fn(c: &mut Cpu) -> bool {
    let index = c.arg(0);
    let v = slot_value_a(c, index);
    c.ret(v);
    true
}

/// 0x200540 slot_value_b(index) -> v0: record value B if the slot is active, else -1
pub fn slot_value_b_fn(c: &mut Cpu) -> bool {
    let index = c.arg(0);
    let v = slot_value_b(c, index);
    c.ret(v);
    true
}

/// 0x200578 item_is_glove(index) -> v0: 1 if the slot's item id is a glove id, else 0
pub fn item_is_glove(c: &mut Cpu) -> bool {
    let index = c.arg(0);
    let id = slot_value_b(c, index);
    let is_glove = GLOVE_IDS.contains(&id);
    c.ret(is_glove as u32);
    true
}
