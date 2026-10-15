//! Walks the linked record list at the head pointer (word at 0x186e20) and a slot table of record pointers.

use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Word holding the head pointer of the record list.
const LIST_HEAD: u32 = 0x0018_6e20;
/// Bytes per list record.
const REC_STRIDE: u32 = 0x28;
/// Record field offsets.
const OFF_FLAGS: u32 = 0x10;
const OFF_VALUE: u32 = 0x12;
const OFF_KIND: u32 = 0x24;
const OFF_OTHER: u32 = 0x26;
/// Record kind that is marked (kind 2).
const KIND_MARK: i16 = 2;
/// Record kind that is never processed (kind 0).
const KIND_NONE: i16 = 0;
/// Flag bit that excludes a record from processing.
const FLAG_HIDDEN: u16 = 2;
/// Flag bit that, together with a marked kind, excludes a record.
const FLAG_ALT: u16 = 1;
/// Value stored by the wide variant for a marked record.
const WIDE_MARK_VALUE: u32 = 0x523e;
/// Bit set in the mask word when every unhidden record was marked.
const MASK_TOP_BIT: u32 = 0x8000_0000;

/// Slot table: one record-list pointer per slot, at 0x197400.
const SLOT_TABLE: u32 = 0x0019_7400;
/// Scale applied to the converted slot value (0.0625).
const SLOT_SCALE: F = F(0x3d80_0000);
/// Sub-index limit: sub-indices at or above this give 0.0.
const SUB_LIMIT: u32 = 0xff;

/// 0x240438 walk_records(first, mask, sums, wide) -> count: walks the record list; each processed record writes its
/// value to `first` (or a marked record's tag when `wide`), sets its bit in `mask`, and adds two fields into `sums`.
/// Returns the number of processed records; `mask` gets its top bit when no hidden record was skipped.
pub fn walk_records(c: &mut Cpu) -> bool {
    let first_out = c.arg(0);
    let mask_out = c.arg(1);
    let sums_arg = c.arg(2);
    let wide = c.arg(3) != 0;
    c.w32(first_out, 0);
    if mask_out != 0 {
        c.w32(mask_out, 0);
    }
    if sums_arg != 0 {
        c.w32(sums_arg, u32::MAX);
    }
    let mut node = c.r32(LIST_HEAD);
    if node == 0 {
        c.ret(0);
        return true;
    }
    let mut cursor = first_out;
    let mut sums_cursor = sums_arg;
    let mut count: u32 = 0;
    let mut mask_ok = true;
    while (c.r16(node) as i16) != 0 {
        let kind = c.r16(node.wrapping_add(OFF_KIND)) as i16;
        let flags = c.r16(node.wrapping_add(OFF_FLAGS));
        if kind != KIND_MARK && flags & FLAG_HIDDEN == 0 {
            mask_ok = false;
        }
        let skip = (flags & FLAG_HIDDEN) != 0
            || kind == KIND_NONE
            || ((flags & FLAG_ALT) != 0 && kind == KIND_MARK);
        if !skip {
            let head = c.r16(node) as i16 as i32 as u32;
            c.w32(cursor, head);
            if wide {
                let tag = if kind == KIND_MARK {
                    WIDE_MARK_VALUE
                } else {
                    let off = (c.r16(node.wrapping_add(OFF_OTHER)) as i16 as i32 as u32).wrapping_shl(1);
                    c.r16(node.wrapping_add(off).wrapping_add(0x14)) as i16 as i32 as u32
                };
                c.w32(cursor, tag);
            }
            if mask_out != 0 && kind == KIND_MARK {
                let bits = c.r32(mask_out) | (1u32 << (count & 31));
                c.w32(mask_out, bits);
            }
            if sums_cursor != 0 {
                let a = c.r16(node.wrapping_add(OFF_VALUE)) as i16 as i32;
                let b = c.r16(node.wrapping_add(OFF_OTHER)) as i16 as i32;
                c.w32(sums_cursor, a.wrapping_add(b) as u32);
                sums_cursor = sums_cursor.wrapping_add(4);
            }
            cursor = cursor.wrapping_add(4);
            count += 1;
        }
        node = node.wrapping_add(REC_STRIDE);
    }
    if mask_out != 0 && mask_ok {
        let bits = c.r32(mask_out) | MASK_TOP_BIT;
        c.w32(mask_out, bits);
    }
    c.ret(count);
    true
}

/// 0x2414a0 slot_scaled(slot, sub) -> f0: for sub < 255, the record's field (found through the slot table) converted
/// to float and scaled by 1/16; otherwise 0.0. Stores ra in the 16-byte frame below sp, as retail does.
pub fn slot_scaled(c: &mut Cpu) -> bool {
    let frame = (c.g(29) as u32).wrapping_sub(0x10);
    let ra = c.r[31];
    c.wv(frame, V([ra as u32, (ra >> 32) as u32, (ra >> 64) as u32, (ra >> 96) as u32]));
    let slot = c.arg(0);
    let sub = c.arg(1);
    if sub >= SUB_LIMIT {
        c.fret(F(0));
        return true;
    }
    let slot_ptr = c.r32(SLOT_TABLE.wrapping_add(slot.wrapping_shl(2)));
    let rec = c.r32(slot_ptr.wrapping_add(sub.wrapping_shl(2)).wrapping_add(0x48));
    let last = (c.r8(rec.wrapping_add(0x10)) as u32).wrapping_sub(1);
    let entry = rec.wrapping_add(last.wrapping_shl(2));
    let field_ptr = c.r32(entry.wrapping_add(0x1c));
    let value = c.r16(field_ptr.wrapping_add(4)) as i16 as i32;
    c.fret(F::from_int(value) * SLOT_SCALE);
    true
}
