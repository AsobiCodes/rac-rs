//! Hud slot table reset: clears the 13 slot records (stride 0x90) and allocates the two work buffers once.
use crate::h;
use crate::hud_slots;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail frame save list for the prologue `addiu sp,-0x60` (s0..s4 at 0x0..0x40, ra at 0x50).
const SAVES: [(usize, u32, u8); 6] = [
    (31, 0x50, h::SQ),
    (20, 0x40, h::SQ),
    (19, 0x30, h::SQ),
    (18, 0x20, h::SQ),
    (17, 0x10, h::SQ),
    (16, 0x00, h::SQ),
];
/// Two words cleared at entry (retail `lui 0x18` + `-0x1a28`).
const COUNTER: u32 = 0x0017_E5D8;
/// First slot record (retail 0x17dd50 + 0x24).
const SLOT_BASE: u32 = 0x0017_DD74;
const SLOT_COUNT: u32 = 13;
const SLOT_STRIDE: u32 = 0x90;
/// Value stored at slot record offset -4.
const SLOT_FLAG_HI: u32 = 0x1000;
const SLOT_STATE_NONE: u32 = 0xffff_ffff;
const SLOT_LIMIT: u32 = 0xffff_fffa;
/// Slot state word stored at record offset 0x40.
/// Work-buffer pointer slots (retail `lui 0x16` + negative offsets).
const BUF_MAIN: u32 = 0x0015_FA28;
const BUF_AUX: u32 = 0x0015_FA34;
const BUF_END: u32 = 0x0015_FA30;
/// Parameter block passed to both allocations (retail 0x15f728).
const ALLOC_PARAMS: u32 = 0x0015_F728;
const ALLOC: u32 = 0x0022_8730;
const MAIN_SIZE: u32 = 0x2800;
const AUX_SIZE: u32 = 0x1400;
/// Gp-relative word that keeps the main buffer pointer (retail `sw v0,-0x71d4(gp)`).
const GP_SAVE: u32 = 0x71d4;
/// Byte written into the main buffer at offset 0x20 (retail `sb v0,0x20(v1)` with v0 = 0xff).
const MARK_BYTE: u8 = 0xff;
const MARK_OFFSET: u32 = 0x20;

/// 0x227fd8 hud_slot_table_init(): resets the slot counter and the 13 slot records, then allocates the work buffers
/// on first use and clears the main one.
pub fn hud_slot_table_init(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x60, &SAVES, |c, _sp| {
        c.w32(COUNTER, 0);
        c.w32(COUNTER + 4, 0);
        for i in 0..SLOT_COUNT {
            let rec = SLOT_BASE + i * SLOT_STRIDE;
            c.w32(rec + 0x40, SLOT_STATE_NONE);
            c.w32(rec - 4, SLOT_FLAG_HI);
            let args = [i, 0xffff, 0, 0, 0, 0, 1];
            h::run(c, hud_slots::hud_slot_request, &args, &[]);
            c.w32(rec + 0x58, 0);
            c.w32(rec + 0x48, SLOT_LIMIT);
            c.w32(rec - 0x20, 0);
            c.w32(rec, 0);
        }
        if c.r32(BUF_MAIN) == 0 {
            let main = h::call(c, ALLOC, &[MAIN_SIZE, 0, ALLOC_PARAMS, 0x115], &[]).0;
            c.w32(BUF_MAIN, main);
            let aux = h::call(c, ALLOC, &[AUX_SIZE, 0, ALLOC_PARAMS, 0x116], &[]).0;
            c.w32(BUF_AUX, aux);
        }
        let main = c.r32(BUF_MAIN);
        let end = main.wrapping_add(MAIN_SIZE);
        c.w32(BUF_END, end);
        let gp = c.g(28) as u32;
        c.w32(gp.wrapping_sub(GP_SAVE), main);
        h::run(c, mathx::zero_qwords, &[main, MAIN_SIZE], &[]);
        let mark_base = c.r32(BUF_MAIN);
        c.w8(mark_base.wrapping_add(MARK_OFFSET), MARK_BYTE);
        c.s(3, mark_base as i32 as i64 as u64);
        c.ret(MARK_BYTE as u32);
    });
    true
}
