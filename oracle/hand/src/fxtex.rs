//! Level fx texture slots: the per-frame reset of the slot table, the packet tag writer, and the lazy GS TEX0 upload
//! of fx texture records.
use crate::h::call;
use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

/// Packet-buffer write pointer.
const GIF_PTR: u32 = 0x16_1140;
/// Packet pointers saved by the frame reset (A) and by the tag writer (B).
const PKT_SAVE_A: u32 = 0x15_F410;
const PKT_SAVE_B: u32 = 0x15_F414;
/// Number of fx records cleared by the frame reset.
const RESET_COUNT: u32 = 0x15_F41C;
/// VRAM cursor source (read by the reset), and the cursor itself (advanced by each upload).
const VRAM_SRC: u32 = 0x15_EE78;
const VRAM_CURSOR: u32 = 0x15_EE74;
/// Threshold (shifted right by 8) that the slot stamps are compared with.
const VRAM_THRESH: u32 = 0x15_EE8C;
/// Number of logged uploads, and the base address that log entries point into.
const UPLOAD_COUNT: u32 = 0x15_F418;
const UPLOAD_BASE: u32 = 0x15_F420;
/// Flag word: when zero, the tag writer uses the gp-relative pointer instead of the flush routines.
const FLUSH_FLAG: u32 = 0x16_A11C;
const FLUSH_FN_A: u32 = 0x23_FCE0;
const FLUSH_FN_B: u32 = 0x29_15B0;
/// gp-relative slots.
const GP_RESET_WORD: i64 = -0x77e8;
const GP_PTR_SAVE: i64 = -0x5ac0;
/// fx texture records (16 bytes each), indexed by slot.
const RECORDS: u32 = 0x16_D280;
/// Upload log (16 bytes each), indexed by upload number.
const UPLOAD_LOG: u32 = 0x16_CE80;
/// Slot list: header pointer at +0x18; entry bases at +0x24 (stamp pass) and +0x28 (clear pass).
const SLOT_LIST: u32 = 0x17_E5D8;
/// Word written at the start of each packet pair.
const DMA_TAG: u32 = 0x2000_0000;
/// Constant bits of the GS TEX0 word: 0x0130_0000, bit 34 and bit 63.
const TEX0_CONST: u64 = 0x0130_0000;
const TEX0_BIT34: u64 = 1 << 34;
const TEX0_BIT63: u64 = 1 << 63;
/// Uploads at or above this count are not logged.
const LOG_LIMIT: i32 = 0x40;

fn gp_addr(c: &Cpu, off: i64) -> u32 {
    (c.g(28) as i64).wrapping_add(off) as u32
}

fn r64(c: &mut Cpu, a: u32) -> u64 {
    let lo = c.r32(a) as u64;
    let hi = c.r32(a + 4) as u64;
    lo | (hi << 32)
}

fn w64(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a + 4, (v >> 32) as u32);
}

/// 0x1eb8c8 fx_frame_reset(skip): zeroes the fx record table, rewinds the packet pointer and copies the VRAM cursor
/// source; unless `skip` is set, it also clears the stamp of every slot at or past the threshold and clears the
/// slot list's clear-pass entries.
pub fn fx_frame_reset(c: &mut Cpu) -> bool {
    let skip = c.arg(0);
    let gif = c.r32(GIF_PTR);
    let src = c.r32(VRAM_SRC);
    c.w32(PKT_SAVE_A, gif);
    c.w32(GIF_PTR, gif + 0x10);
    c.w32(VRAM_CURSOR, src);
    let records = c.r32(RESET_COUNT) as i32;
    for i in 0..records.max(0) as u32 {
        let rec = RECORDS + i * 16;
        w64(c, rec, 0);
    }
    let reset_word = gp_addr(c, GP_RESET_WORD);
    c.w32(reset_word, 0);
    if skip != 0 {
        return true;
    }
    let thresh = (c.r32(VRAM_THRESH) as i32) >> 8;
    let header = c.r32(SLOT_LIST + 0x18);
    let stamp_count = c.r32(header + 0x44) as i32;
    for i in 0..stamp_count.max(0) as u32 {
        let base = c.r32(SLOT_LIST + 0x24);
        let entry = base + i * 8;
        if c.r16(entry + 4) as i32 >= thresh {
            c.w16(entry + 4, 0);
        }
    }
    let header = c.r32(SLOT_LIST + 0x18);
    let clear_count = c.r32(header + 0x24) as i32;
    for i in 0..clear_count.max(0) as u32 {
        let base = c.r32(SLOT_LIST + 0x28);
        c.w16(base + i * 8 + 4, 0);
    }
    true
}

/// 0x1eb9e0 fx_pkt_tags(): writes the DMA tag words of the packet pair opened by `fx_frame_reset`, flushing through
/// the two retail routines first when the flush flag is set.
pub fn fx_pkt_tags(c: &mut Cpu) -> bool {
    let p = c.r32(GIF_PTR);
    c.w32(PKT_SAVE_B, p);
    let a = c.r32(PKT_SAVE_A);
    c.w32(GIF_PTR, p + 0x10);
    c.w32(a, DMA_TAG);
    let a = c.r32(PKT_SAVE_A);
    let gif = c.r32(GIF_PTR);
    c.w32(a + 4, gif);
    c.w32(a + 8, 0);
    c.w32(a + 0xc, 0);
    let tag_at = if c.r32(FLUSH_FLAG) != 0 {
        call(c, FLUSH_FN_A, &[], &[]);
        call(c, FLUSH_FN_B, &[], &[]);
        c.r32(GIF_PTR)
    } else {
        c.r32(gp_addr(c, GP_PTR_SAVE))
    };
    c.w32(tag_at, DMA_TAG);
    let a = c.r32(PKT_SAVE_A);
    let gif = c.r32(GIF_PTR);
    c.w32(gif + 4, a + 0x10);
    let gif = c.r32(GIF_PTR);
    c.w32(gif + 8, 0);
    c.w32(gif + 0xc, 0);
    let b = c.r32(PKT_SAVE_B);
    let gif = c.r32(GIF_PTR) + 0x10;
    c.w32(GIF_PTR, gif);
    c.w32(b, DMA_TAG);
    let gif = c.r32(GIF_PTR);
    c.w32(b + 4, gif);
    c.w32(b + 8, 0);
    c.w32(b + 0xc, 0);
    true
}

/// 0x1ebb00 fx_tex_upload(slot) -> u64: returns the GS TEX0 word of fx texture record `slot`. On first use it builds
/// the word from the record's fields, advances the VRAM cursor, stores the word, and logs the upload.
pub fn fx_tex_upload(c: &mut Cpu) -> bool {
    let rec = RECORDS + (c.arg(0) << 4);
    let cached = r64(c, rec);
    if cached != 0 {
        c.s(2, cached);
        return true;
    }
    let field_c = c.r16(rec + 0xc);
    let cursor = c.r32(VRAM_CURSOR) as i32;
    let cursor_hi = cursor >> 8;
    let cursor_next = cursor.wrapping_add(0x400);
    let cursor_next_hi = cursor_next >> 8;
    let shift = (field_c as i16 as i32 - 6).max(0) as u32;
    let bit_c = (1u32 << (shift & 31)) as i32 as i64 as u64;
    let low_field = (((field_c as u64) << 48) as i64 >> 22) as u64;
    let mut word = (cursor_next_hi as i64 as u64) | (bit_c << 14) | low_field | TEX0_CONST;

    let field_e = c.r16(rec + 0xe) as i16 as i32;
    let field_e_u = c.r16(rec + 0xe);
    let high_field = (((field_e_u as u64) << 48) as i64 >> 18) as u64;
    let hi_word = ((cursor_hi as i64 as u64) << 37) | TEX0_BIT34;
    let shift2 = (field_c as i16 as i32).wrapping_add(field_e) as u32;
    let bit_2 = (1u32 << (shift2 & 31)) as i32;
    word |= high_field;
    word |= hi_word;
    word |= TEX0_BIT63;
    let cursor_after = cursor_next.wrapping_add(bit_2);
    c.w32(VRAM_CURSOR, cursor_after as u32);

    let uploads = c.r32(UPLOAD_COUNT) as i32;
    w64(c, rec, word);
    if uploads < LOG_LIMIT {
        let field_a = c.r16(rec + 0xa) as u32;
        let base = c.r32(UPLOAD_BASE);
        let entry = UPLOAD_LOG + ((uploads as u32) << 4);
        let field_8 = c.r16(rec + 8) as u32;
        c.w32(entry, base.wrapping_add(field_a << 4));
        c.w16(entry + 4, 0);
        c.w16(entry + 6, cursor_hi as u16);
        c.w32(entry + 8, base.wrapping_add(field_8 << 4));
        let byte_c = c.r8(rec + 0xc);
        let byte_e = c.r8(rec + 0xe);
        c.w8(entry + 0xc, byte_c);
        c.w8(entry + 0xd, byte_e);
        c.w16(entry + 0xe, cursor_next_hi as u16);
        c.w32(UPLOAD_COUNT, (uploads + 1) as u32);
    }
    c.s(2, word);
    true
}
