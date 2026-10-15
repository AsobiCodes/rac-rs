//! Selector-driven field patch over the object list at 0x1c7180 (Eudora overlay tables).
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// 0x1c7180: list of object indices, one word each; a negative word ends the list.
const INDEX_LIST: u32 = 0x001c_7180;
/// 0x1c5880: object pointer table, indexed by list index (4 bytes each).
const OBJECT_TABLE: u32 = 0x001c_5880;
/// 0x1c6b80: selector table of (halfword, halfword) pairs, 4 bytes per selector.
const SELECTOR_TABLE: u32 = 0x001c_6b80;
/// 0x1c8380: source copied over the index list (0x200 bytes).
const INDEX_LIST_SRC: u32 = 0x001c_8380;
/// 0x1c7f80: source copied over the selector table (0x400 bytes).
const SELECTOR_TABLE_SRC: u32 = 0x001c_7f80;
/// 0x1c7380: first object-list argument of the retail pair call.
const PAIR_ARG_A: u32 = 0x001c_7380;
/// 0x1c8580: second object-list argument of the retail pair call.
const PAIR_ARG_B: u32 = 0x001c_8580;
/// Retail callee taking one object-list pointer.
const PAIR_CALLEE: u32 = 0x0028_7f48;

/// Object fields: entry count (byte), entry array pointer (word).
const OBJ_ENTRY_COUNT: u32 = 0x23;
const OBJ_ENTRY_PTR: u32 = 0x2c;
/// Entry fields: selector index (byte), two patched words at +0 and +0x20, entry stride 0x50.
const ENTRY_SELECTOR: u32 = 0x33;
const ENTRY_WORD_A: u32 = 0x00;
const ENTRY_WORD_B: u32 = 0x20;
const ENTRY_STRIDE: u32 = 0x50;
/// Patched word keeps its top 18 bits and takes the sign-extended halfword in its low bits.
const KEEP_HIGH_BITS: u32 = 0xffff_c000;
const RA: usize = 31;

/// Writes the selected halfword into the low bits of one entry word, keeping the top 18 bits, when it is nonzero.
fn patch_word(c: &mut Cpu, addr: u32, value: u32) {
    if value != 0 {
        let word = c.r32(addr);
        c.w32(addr, (word & KEEP_HIGH_BITS) | value);
    }
}

/// Patches every entry of one object: each entry's selector picks a (lo, hi) halfword pair from the selector table.
fn patch_object(c: &mut Cpu, obj: u32) {
    let mut entry = c.r32(obj + OBJ_ENTRY_PTR);
    let mut i = 0;
    while i < c.r8(obj + OBJ_ENTRY_COUNT) as u32 {
        let pair = SELECTOR_TABLE + 4 * c.r8(entry + ENTRY_SELECTOR) as u32;
        let lo = c.r16(pair) as i16 as i32 as u32;
        patch_word(c, entry + ENTRY_WORD_A, lo);
        let hi = c.r16(pair + 2) as i16 as i32 as u32;
        patch_word(c, entry + ENTRY_WORD_B, hi);
        entry = entry.wrapping_add(ENTRY_STRIDE);
        i += 1;
    }
}

/// 0x286358 patch_entries(): patches the entries of every object listed at 0x1c7180 until the negative terminator.
pub fn patch_entries(c: &mut Cpu) -> bool {
    let mut slot = INDEX_LIST;
    let mut index = c.r32(slot) as i32;
    while index >= 0 {
        let obj = c.r32(OBJECT_TABLE + 4 * index as u32);
        if c.r8(obj + OBJ_ENTRY_COUNT) != 0 {
            patch_object(c, obj);
        }
        slot = slot.wrapping_add(4);
        index = c.r32(slot) as i32;
    }
    true
}

/// 0x286418 reload_patch_tables(): patch pass, reload both tables from their sources, patch pass again.
pub fn reload_patch_tables(c: &mut Cpu) -> bool {
    h::frame(c, 0x10, &[(RA, 0x00, h::SQ)], |c, _sp| {
        h::run(c, patch_entries, &[], &[]);
        h::run(c, mathx::copy_qwords, &[INDEX_LIST, INDEX_LIST_SRC, 0x200], &[]);
        h::run(c, mathx::copy_qwords, &[SELECTOR_TABLE, SELECTOR_TABLE_SRC, 0x400], &[]);
        h::run(c, patch_entries, &[], &[]);
    });
    true
}

/// 0x286470 call_pair(): calls the retail routine 0x287f48 on the two object lists 0x1c7380 and 0x1c8580.
pub fn call_pair(c: &mut Cpu) -> bool {
    h::frame(c, 0x10, &[(RA, 0x00, h::SQ)], |c, _sp| {
        h::call(c, PAIR_CALLEE, &[PAIR_ARG_A], &[]);
        h::call(c, PAIR_CALLEE, &[PAIR_ARG_B], &[]);
    });
    true
}
