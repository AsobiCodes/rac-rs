//! Small GS packet and list helpers: RGBAQ clear colour, tag-pair append, list-slot clear, flag word sweep.

use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const AT: usize = 1;

/// Address of the frame-clear RGBAQ sprite word.
const CLEAR_RGBAQ_ADDR: u32 = 0x152078;
/// Pointer slot of the GS packet write cursor.
const PKT_CURSOR: u32 = 0x161140;
/// Packet tag words written by the append helper.
const TAG_A: u32 = 0x3000_0015;
const TAG_B: u32 = 0x5000_0015;
/// Table pointer at 0x15f640; the 0x1516d0 table is compared at +0x54 and clamped at +0x5a.
const TABLE: u32 = 0x1516d0;
/// List head (the 0x180000 bank, offset -0x6af0) and the 0x160000 bank pointer at offset -0x9c0.
const LIST_HEAD: u32 = 0x179510;
const LIST_BANK: u32 = 0x180000;
const TABLE_PTR: u32 = 0x15f640;
/// Start of the flag-word table swept by `clear_flag_bits`.
const FLAG_TABLE: u32 = 0x141968;
const FLAG_COUNT: u32 = 148;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

fn w64(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a + 4, (v >> 32) as u32);
}

/// 0x1f4000 set_clear_rgbaq(r, g, b): packs (a0 | a1<<8 | a2<<16 | 0x8000<<16) into the frame-clear RGBAQ word
/// at 0x152078 -> (): stores the 64-bit value and leaves it in a1.
pub fn set_clear_rgbaq(c: &mut Cpu) -> bool {
    let a0 = c.g(A0);
    let a1 = c.g(A1);
    let a2 = c.g(A2);
    let v = a0 | (a1 << 8) | (a2 << 16) | (0x8000u64 << 16);
    w64(c, CLEAR_RGBAQ_ADDR, v);
    c.s(V1, 0x8000_0000);
    c.s(A1, v);
    c.s(V0, 0x150000);
    true
}

/// 0x1f40e8 append_tag_pair() -> (): if the packet cursor at 0x161140 is set, writes two GS tag words and a zero
/// word through it and advances the cursor by 0x10; returns v0 = 0x30000000 when the cursor is empty.
pub fn append_tag_pair(c: &mut Cpu) -> bool {
    let p = c.r32(PKT_CURSOR);
    if p == 0 {
        set_gpr(c, V1, 0);
        set_gpr(c, V0, 0x3000_0000);
        return true;
    }
    c.w32(p, TAG_A);
    set_gpr(c, A0, 0x152040);
    set_gpr(c, A1, TAG_B);
    let p = c.r32(PKT_CURSOR);
    c.w32(p + 4, c.g(A0) as u32);
    let p = c.r32(PKT_CURSOR);
    set_gpr(c, V1, p);
    c.w32(p + 8, 0);
    let p = c.r32(PKT_CURSOR);
    set_gpr(c, A0, p);
    c.w32(p + 0xc, TAG_B);
    let p = c.r32(PKT_CURSOR);
    let next = p.wrapping_add(0x10);
    set_gpr(c, V0, next);
    set_gpr(c, AT, 0x160000);
    c.w32(PKT_CURSOR, next);
    true
}

/// 0x1f6a48 clear_help_slot() -> (): when the list pointer at 0x179510 is set, finds the entry it names and, if the
/// entry's id matches the table at 0x1516d0, clamps the halfword at +0x5a to 5 unless it is 6 or 7; then zeroes the
/// list head and marks its two slots (+0x20, +0x24) empty.
pub fn clear_help_slot(c: &mut Cpu) -> bool {
    if c.r32(LIST_HEAD) == 0 {
        set_gpr(c, A0, LIST_HEAD);
        set_gpr(c, V0, LIST_BANK);
        set_gpr(c, A2, LIST_BANK);
        set_gpr(c, V1, 0);
        return true;
    }
    set_gpr(c, A2, LIST_BANK);
    set_gpr(c, A0, LIST_HEAD);
    let entry = c.r32(LIST_HEAD + 0x20);
    let table = c.r32(TABLE_PTR);
    set_gpr(c, A0, u32::MAX);
    let key = (entry << 4).wrapping_add(table);
    let id = c.r32(key + 8);
    set_gpr(c, A1, id);
    if id != u32::MAX {
        set_gpr(c, V1, TABLE);
        let h = c.r16(TABLE + 0x54) as i16 as i32;
        if id as i32 as i64 == (h - 0x7530) as i64 {
            let hu = c.r16(TABLE + 0x5a) as i32;
            let d = (hu - 6) as i64 as u64;
            if d >= 2 {
                c.w16(TABLE + 0x5a, 5);
            }
        }
    }
    set_gpr(c, V0, LIST_HEAD);
    c.s(V1, u64::MAX);
    c.w32(LIST_HEAD, 0);
    c.w32(LIST_HEAD + 4, 0);
    c.w32(LIST_HEAD + 0x20, u32::MAX);
    c.w32(LIST_HEAD + 0x24, u32::MAX);
    true
}

/// 0x1f6ad0 clear_flag_bits() -> (): clears bit 31 of the 148 words at 0x14196c + 8*k (every other word from
/// 0x141968+4) in place, leaving a0 = -1, a1 = 0x7fffffff.
pub fn clear_flag_bits(c: &mut Cpu) -> bool {
    let mut p = FLAG_TABLE + 4;
    let mut last = 0u32;
    for _ in 0..FLAG_COUNT {
        last = c.r32(p) & 0x7fff_ffff;
        c.w32(p, last);
        p += 8;
    }
    c.s(V0, last as u64);
    set_gpr(c, V1, p);
    c.s(A0, u64::MAX);
    c.s(A1, 0x7fff_ffff);
    true
}
