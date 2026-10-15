//! Moby release and slot-table refresh helpers of the Eudora overlay (retail 0x270608..0x275e04).

use crate::actor;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail word transform (0x276768), applied to each table word it is given.
const WORD_TRANSFORM: u32 = 0x0027_6768;
/// Retail slot-byte clearing helper (0x277248), takes no arguments.
const SLOT_CLEAR: u32 = 0x0027_7248;
/// Frame counter cell (0x15f5ac). A released moby stores it at +0x38.
const FRAME_COUNTER: u32 = 0x0015_F5AC;
const MOBY_FRAME_STAMP: u32 = 0x38;
/// Offsets inside the object given to obj_pair_fixup.
const OBJ_WORD_HEAD: u32 = 0x44;
const OBJ_WORD_A: u32 = 0x48;
const OBJ_WORD_B: u32 = 0x4C;
const OBJ_WORD_TAIL_A: u32 = 0x50;
const OBJ_WORD_TAIL_B: u32 = 0x54;
/// Pointer table inside the object passed to slot_table_refresh: 24 words at +0x44.
const OBJ_TABLE: u32 = 0x44;
const OBJ_TABLE_WORDS: u32 = 24;
/// Fixed block that slot_table_refresh rewrites (retail 0x1b9df0).
const BLOCK: u32 = 0x001B_9DF0;
const BLOCK_WORD_A: u32 = 0xA0;
const BLOCK_WORD_B: u32 = 0xA4;
const BLOCK_LIST: u32 = 0xB0;
const BLOCK_LIST_WORDS: u32 = 3;
const BLOCK_FLAG_A: u32 = 0xC8;
const BLOCK_FLAG_B: u32 = 0xC9;
const BLOCK_FLAG_C: u32 = 0xCA;
const FLAG_SET: u8 = 0xFF;

fn w64(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a + 4, (v >> 32) as u32);
}

/// Shared body of retail 0x275dc0 for a non-null moby: frees it and stamps the frame counter into +0x38.
fn release_moby_entry(c: &mut Cpu, moby: u32) {
    let _ = h::run(c, actor::free_moby, &[moby], &[]);
    let stamp = c.r32(FRAME_COUNTER) as i32 as i64 as u64;
    c.s(3, stamp);
    w64(c, moby + MOBY_FRAME_STAMP, stamp);
}

/// 0x270608 obj_pair_fixup(obj): passes obj+0x48 and obj+0x4c through the word transform, clears
/// obj+0x44, obj+0x50 and obj+0x54 to -1, leaves v1 = -1, returns 0.
pub fn obj_pair_fixup(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    let a = c.r32(obj + OBJ_WORD_A);
    let (a_new, _) = h::call_keep(c, WORD_TRANSFORM, &[a], &[]);
    c.w32(obj + OBJ_WORD_A, a_new);
    let b = c.r32(obj + OBJ_WORD_B);
    let (b_new, _) = h::call_keep(c, WORD_TRANSFORM, &[b], &[]);
    c.w32(obj + OBJ_WORD_B, b_new);
    c.w32(obj + OBJ_WORD_HEAD, u32::MAX);
    c.w32(obj + OBJ_WORD_TAIL_A, u32::MAX);
    c.w32(obj + OBJ_WORD_TAIL_B, u32::MAX);
    c.s(3, u64::MAX);
    c.ret(0);
    true
}

/// 0x275dc0 release_moby(moby): if moby is non-null, frees it and stamps the frame counter into +0x38; returns 0.
pub fn release_moby(c: &mut Cpu) -> bool {
    let moby = c.arg(0);
    if moby != 0 {
        release_moby_entry(c, moby);
    }
    c.ret(0);
    true
}

/// 0x274b48 slot_table_refresh(obj): releases the 24 mobys in obj+0x44.., clears those words, rewrites the
/// fixed block at 0x1b9df0 (two words through the transform, flag bytes, a clear helper, three list words through
/// the transform), returns 0.
pub fn slot_table_refresh(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let table = c.arg(0) + OBJ_TABLE;
    for i in 0..OBJ_TABLE_WORDS {
        let slot = table + 4 * i;
        let moby = c.r32(slot);
        if moby != 0 {
            release_moby_entry(c, moby);
        }
        c.w32(slot, 0);
    }
    let a = c.r32(BLOCK + BLOCK_WORD_A);
    let (a_new, _) = h::call_keep(c, WORD_TRANSFORM, &[a], &[]);
    c.w32(BLOCK + BLOCK_WORD_A, a_new);
    let b = c.r32(BLOCK + BLOCK_WORD_B);
    let (b_new, _) = h::call_keep(c, WORD_TRANSFORM, &[b], &[]);
    c.w32(BLOCK + BLOCK_WORD_B, b_new);
    c.w8(BLOCK + BLOCK_FLAG_B, FLAG_SET);
    c.w8(BLOCK + BLOCK_FLAG_A, FLAG_SET);
    c.w8(BLOCK + BLOCK_FLAG_C, 0);
    let _ = h::call_keep(c, SLOT_CLEAR, &[], &[]);
    for i in 0..BLOCK_LIST_WORDS {
        let word = BLOCK + BLOCK_LIST + 4 * i;
        let w = c.r32(word);
        let (w_new, _) = h::call_keep(c, WORD_TRANSFORM, &[w], &[]);
        c.w32(word, w_new);
    }
    c.ret(0);
    true
}
