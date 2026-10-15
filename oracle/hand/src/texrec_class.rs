//! Level fx texture record table builder, and the moby class registration that fills the class tables (byte, halfword
//! and word entries per class id, the core's word, then the class setup calls).
use crate::h::{self, SQ};
use crate::mathx;
use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

/// Stack frame of both functions; the saved registers (s0-s3, ra) sit at the same offsets in each.
const FRAME: u32 = 0x50;
const SAVES: [(usize, u32, u8); 5] = [(16, 0x00, SQ), (17, 0x10, SQ), (18, 0x20, SQ), (19, 0x30, SQ), (31, 0x40, SQ)];

/// Fx texture record count (word); the builder restarts it at zero and indexes records with it.
const FX_COUNT: u32 = 0x15_F41C;
/// Gp-relative offset of the running fx record count.
const FX_COUNT_GP: u32 = 0x77E4;
/// Fx texture record table: 16-byte records.
const FX_TABLE: u32 = 0x16_D280;

/// Moby class count (word); its low byte and value index the class tables.
const CLASS_COUNT: u32 = 0x15_FF40;
/// Class tables: byte per class (at id + base), halfword per count, word per count, and the core-link word table.
const CLASS_BYTES: u32 = 0x19_7CC0;
const CLASS_HALF: u32 = 0x19_7B00;
const CLASS_WORDS: u32 = 0x19_7400;
const CLASS_LINKS: u32 = 0x19_A380;
/// Gp-relative offset of the class count.
const CLASS_COUNT_GP: u32 = 0x6CC0;
/// Retail callees: the class id setup and the class registration.
const CLASS_SETUP: u32 = 0x25_6A38;
const CLASS_REGISTER: u32 = 0x23_2C08;
/// Offset of the core's link word.
const CORE_LINK: u32 = 0x2C;

/// Sets a general register to a 32-bit value, sign-extended as the MIPS 32-bit ops leave it.
fn set(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x231cc0 fx_tex_table_build(recs, n): clears the record count, then for each of n 16-byte records (x, y, w, h)
/// stores x>>4 and y>>4 as halfwords, log2_floor(w) and log2_floor(h) after them, zeroes the record's first 8 bytes and
/// advances the count (also kept at gp-0x77e4).
pub fn fx_tex_table_build(c: &mut Cpu) -> bool {
    let recs = c.arg(0);
    let n = c.arg(1) as i32;
    c.w32(FX_COUNT, 0);
    let gp = c.g(28) as u32;
    let mut last: Option<(u32, u32, u32)> = None;
    h::frame(c, FRAME, &SAVES, |c, _sp| {
        if n <= 0 {
            return;
        }
        let mut p = recs;
        loop {
            let cnt = c.r32(FX_COUNT);
            let x = (c.r32(p) as i32 >> 4) as u32;
            let y = (c.r32(p + 4) as i32 >> 4) as u32;
            let w = c.r32(p + 8);
            let height = c.r32(p + 12);
            p = p.wrapping_add(16);
            let rec = FX_TABLE.wrapping_add(cnt << 4);
            c.w16(rec + 0xA, x as u16);
            c.w16(rec + 8, y as u16);
            let log_w = h::run(c, mathx::log2_floor, &[w], &[]).0 as u32;
            c.w16(rec + 0xC, log_w as u16);
            let log_h = h::run(c, mathx::log2_floor, &[height], &[]).0 as u32;
            c.w16(rec + 0xE, log_h as u16);
            c.w32(rec, 0);
            c.w32(rec + 4, 0);
            let next = cnt.wrapping_add(1);
            c.w32(gp.wrapping_sub(FX_COUNT_GP), next);
            last = Some((log_h, rec, next));
            if (next as i32) >= n {
                break;
            }
        }
    });
    set(c, 1, 0x16_0000);
    if let Some((v0, rec, next)) = last {
        c.ret(v0);
        set(c, 3, rec);
        set(c, 4, next);
        set(c, 5, 0);
    }
    true
}

/// 0x232f10 moby_class_register(core, tex, remap, id) -> count: stores the id's byte, the count's halfword and the core
/// word in the class tables; with a core it also writes the core's link word, calls the class setup and the class
/// registration; bumps the class count (also kept at gp-0x6cc0) and returns it.
pub fn moby_class_register(c: &mut Cpu) -> bool {
    let core = c.arg(0);
    let tex = c.arg(1);
    let remap = c.arg(2);
    let id = c.arg(3);
    let gp = c.g(28) as u32;
    h::frame(c, FRAME, &SAVES, |c, _sp| {
        let count = c.r32(CLASS_COUNT);
        c.w8(id.wrapping_add(CLASS_BYTES), (count & 0xFF) as u8);
        c.w16(CLASS_HALF.wrapping_add(count << 1), id as u16);
        c.w32(CLASS_WORDS.wrapping_add(count << 2), core);
        if core != 0 {
            let link = c.r32(core.wrapping_add(CORE_LINK));
            c.w32(CLASS_LINKS.wrapping_add(count << 2), link);
            h::call_keep(c, CLASS_SETUP, &[id], &[]);
            let next = c.r32(CLASS_COUNT).wrapping_add(1);
            c.w32(gp.wrapping_sub(CLASS_COUNT_GP), next);
            h::call_keep(c, CLASS_REGISTER, &[core, tex, remap, id], &[]);
        } else {
            h::call_keep(c, CLASS_SETUP, &[id], &[]);
            let next = c.r32(CLASS_COUNT).wrapping_add(1);
            c.w32(gp.wrapping_sub(CLASS_COUNT_GP), next);
            c.ret(next);
        }
    });
    true
}
