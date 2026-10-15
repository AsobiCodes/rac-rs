//! Template text expander (retail 0x23cb70): copies a record's text to an output buffer and replaces its first `%`
//! escape with the formatter's output (a value for `%b`, a plain form for any other escape).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Word at this address is the base of the 0x28-byte record array (record index in a0).
const RECORD_BASE_PTR: u32 = 0x18_42f0 + 0x20;
/// The value path reads its record table from this pointer (retail a3), not from RECORD_BASE_PTR.
const VALUE_BASE_PTR: u32 = 0x16_42f0 + 0x20;
const RECORD_SIZE: u32 = 0x28;
/// Record fields: text id (signed halfword at +0xa) and value index (signed halfword at +0xc).
const REC_TEXT_ID: u32 = 0xa;
const REC_VALUE_INDEX: u32 = 0xc;
/// 24-byte value entries; the word at +0 of an entry is the value handed to the formatter.
const VALUE_TABLE: u32 = 0x1c_4130;
const VALUE_ENTRY_SIZE: u32 = 0x18;
/// Returns the NUL-terminated template for a text id.
const TEXT_LOOKUP: u32 = 0x1f_6b80;
/// Formatter: writes a formatted string to dst from a format string and one value.
const FORMATTER: u32 = 0x11_6248;
/// Format strings used for the `%b` escape (value) and for every other escape (plain).
const FMT_VALUE: u32 = 0x15_fdb8;
const FMT_PLAIN: u32 = 0x15_fdc0;
/// Value the retail code leaves in a1 (0x180000) once the text lookup has returned.
const TEXT_BASE_HI: u32 = 0x18_0000;
/// Stack buffer the formatter writes into (retail frame size).
const FRAME_SIZE: u32 = 0x60;
const ESCAPE: u8 = b'%';
const CONV_VALUE: u8 = b'b';

/// Sign-extends a 32-bit result to the 64-bit register value.
fn sext(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// 0x23cb70 expand_text(index, out) -> none: copies the template text of record `index` to `out` (NUL-terminated),
/// replacing its first `%` escape with the formatter output; the rest of the template is copied as is.
pub fn expand_text(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let index = c.arg(0);
    let out = c.arg(1);
    let base = c.r32(RECORD_BASE_PTR);
    let rec = base.wrapping_add(index.wrapping_mul(RECORD_SIZE));
    c.s(3, sext(rec));
    let text_id = c.r16(rec + REC_TEXT_ID) as i16 as i32 as u32;
    let (text, _) = h::call_keep(c, TEXT_LOOKUP, &[text_id, out], &[]);
    c.s(5, sext(TEXT_BASE_HI));
    if text == 0 {
        return true;
    }
    h::with_stack(c, FRAME_SIZE, |c, buf| expand(c, buf, index, base, rec, text, out));
    true
}

/// Body of `expand_text` inside the retail frame; `buf` is the frame's formatter buffer.
fn expand(c: &mut Cpu, buf: u32, index: u32, base: u32, rec: u32, text: u32, out: u32) {
    let mut src = text;
    let mut dst = out;
    // Literal text up to the NUL terminator or the first escape.
    let mut ch = c.r8(src);
    // The retail loop leaves the first character in a0 (and a non-empty copy leaves the escape value 0x25 there)
    // and the last character loaded in v1.
    c.s(4, if ch == 0 { 0 } else { ESCAPE as u64 });
    c.s(3, ch as u64);
    while ch != 0 && ch != ESCAPE {
        c.w8(dst, ch);
        dst += 1;
        src += 1;
        ch = c.r8(src);
        c.s(3, ch as u64);
    }
    if ch == 0 {
        c.w8(dst, 0);
        c.s(2, 0);
        return;
    }
    // Escape: the character after `%` selects the formatter call.
    src += 1;
    let conv = c.r8(src);
    c.s(3, CONV_VALUE as u64);
    if conv == CONV_VALUE {
        let vbase = c.r32(VALUE_BASE_PTR);
        let vrec = vbase.wrapping_add(index.wrapping_mul(RECORD_SIZE));
        let entry = (c.r16(vrec + REC_VALUE_INDEX) as i16 as i32 as u32)
            .wrapping_mul(VALUE_ENTRY_SIZE)
            .wrapping_add(VALUE_TABLE);
        let value = c.r32(entry);
        c.s(2, sext(entry));
        c.s(3, sext(vrec));
        c.s(7, sext(vbase));
        c.s(8, sext(VALUE_ENTRY_SIZE));
        h::call_keep(c, FORMATTER, &[buf, FMT_VALUE, value], &[]);
    } else {
        let value = c.arg(2);
        c.s(2, conv as u64);
        h::call_keep(c, FORMATTER, &[buf, FMT_PLAIN, value], &[]);
    }
    // The formatted text goes out, then the template after the conversion character.
    src += 1;
    let mut p = buf;
    let mut b = c.r8(p);
    while b != 0 {
        c.w8(dst, b);
        p += 1;
        dst += 1;
        b = c.r8(p);
    }
    let mut ch = c.r8(src);
    while ch != 0 {
        c.w8(dst, ch);
        src += 1;
        dst += 1;
        ch = c.r8(src);
    }
    c.w8(dst, 0);
    c.s(2, 0);
}
