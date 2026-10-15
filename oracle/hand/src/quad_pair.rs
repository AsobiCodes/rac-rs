//! Two-sprite GIF packets: `quad_pair` appends one packet for a table entry to the GS buffer, after `index_pass`
//! (the slot-table pass it calls) has updated the records. Retail EE 0x22e620 and 0x22e400.
use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

/// Table block: +0x20 pointer to the index entries (two u16: list-1 index, list-2 index), +0x24 pointer to the
/// list-2 records, +0x28 pointer to the list-1 records, +0xc word copied into every packet quad.
const BLOCK: u32 = 0x17_e5d8;
const OFF_INDEX: u32 = 0x20;
const OFF_LIST2: u32 = 0x24;
const OFF_LIST1: u32 = 0x28;
const OFF_QUAD_WORD: u32 = 0xc;
/// Slot table of 16-byte entries, addressed by the slot counter.
const SLOT_TABLE: u32 = 0x16_ce80;
const SLOT_COUNT: u32 = 0x15_f418;
const SLOT_LIMIT: i32 = 0x40;
/// Time words: TIME_LO feeds the list-1 count and the accumulator, TIME_HI feeds the tail's width test.
const TIME_LO: u32 = 0x15_ee74;
const TIME_HI: u32 = 0x15_ee8c;
/// gp-relative word that accumulates the list-2 value.
const GP_ACCUM: i32 = -0x7d8c;
const FULL_WEIGHT: u16 = 0x3ff0;
/// Pointer to the next free GS packet slot.
const GS_PTR: u32 = 0x16_1140;
/// Viewport offsets: +0x10 added to the x terms, +0x14 added to the y terms.
const VIEW: u32 = 0x13_e500;
const GIF_TAG_LO: u32 = 0x1000_0005;
const GIF_TAG_HI: u32 = 0x5000_0005;
const SPRITE_TAG: u64 = (0xe800u64 << 47) | 0x8001;
const SPRITE_REG_PAIR: u64 = 0x0535_3106;
const TEXTURE_WORD: u64 = 0x156;
const COLOR_MASK: u64 = 0x007f_7f7f;

/// Sign-extends a 32-bit register value to 64 bits, as the MIPS 32-bit ops leave it.
fn sext(v: u32) -> u64 {
    v as i32 as i64 as u64
}

fn put64(c: &mut Cpu, addr: u32, v: u64) {
    c.w32(addr, v as u32);
    c.w32(addr + 4, (v >> 32) as u32);
}

fn gp_accum(c: &mut Cpu) -> u32 {
    (c.g(28) as u32).wrapping_add(GP_ACCUM as u32)
}

/// List-1 record with a zero count: writes a slot from the record's first word.
fn emit_slot(c: &mut Cpu, p: u32) {
    let n = c.r32(SLOT_COUNT);
    let slot = SLOT_TABLE + (n << 4);
    let word = c.r32(p);
    c.w32(slot, word);
    c.w16(slot + 4, 0);
    c.w16(slot + 6, FULL_WEIGHT);
    c.w32(slot + 8, word);
    c.w8(slot + 0xc, 5);
    c.w8(slot + 0xd, 5);
    c.w16(slot + 0xe, FULL_WEIGHT);
}

/// List-1 record whose count is zero: the count becomes the time word's high half, the accumulator is set from
/// the time word, and a slot is recorded while the slot counter is below the limit. Returns true when it wrote one.
fn time_slot(c: &mut Cpu, p: u32) -> bool {
    let t = c.r32(TIME_LO) as i32;
    c.w16(p + 4, (t >> 8) as u16);
    let accum = gp_accum(c);
    c.w32(accum, t.wrapping_add(0x400) as u32);
    let n = c.r32(SLOT_COUNT);
    if (n as i32) >= SLOT_LIMIT {
        return false;
    }
    let slot = SLOT_TABLE + (n << 4);
    let word = c.r32(p);
    let count = c.r16(p + 4);
    c.w32(slot, word);
    c.w16(slot + 4, 0);
    c.w16(slot + 6, count);
    true
}

/// List-2 record whose count is zero: the count becomes the accumulator's high half, the accumulator grows by a
/// power of two chosen by the larger of the two weight bytes, and a slot is recorded below the limit.
fn accum_slot(c: &mut Cpu, q: u32) -> bool {
    let accum = gp_accum(c);
    let g = c.r32(accum);
    c.w16(q + 4, ((g as i32) >> 8) as u16);
    let b6 = c.r8(q + 6);
    let b7 = c.r8(q + 7);
    let m = b6.max(b7) as u32;
    c.w32(accum, g.wrapping_add(1u32 << ((m << 1) & 31)));
    let n = c.r32(SLOT_COUNT);
    if (n as i32) >= SLOT_LIMIT {
        return false;
    }
    let slot = SLOT_TABLE + (n << 4);
    let word = c.r32(q);
    let count = c.r16(q + 4);
    c.w32(slot + 8, word);
    c.w8(slot + 0xc, b6);
    c.w8(slot + 0xd, b7);
    c.w16(slot + 0xe, count);
    true
}

/// The slot pass for table entry `idx`: returns the packed 64-bit value that retail leaves in v0.
fn pass(c: &mut Cpu, idx: u32) -> u64 {
    let entry = c.r32(BLOCK + OFF_INDEX).wrapping_add(idx << 2);
    let h0 = c.r16(entry) as i16 as i32;
    let h1 = c.r16(entry + 2) as i16 as i32;
    let p = c.r32(BLOCK + OFF_LIST1).wrapping_add((h0 << 3) as u32);
    let q = c.r32(BLOCK + OFF_LIST2).wrapping_add((h1 << 3) as u32);
    let mut slot_used = false;
    if c.r16(p + 4) == 0 || c.r16(q + 4) == 0 {
        emit_slot(c, p);
    }
    if c.r16(p + 4) == 0 {
        slot_used |= time_slot(c, p);
    }
    if c.r16(q + 4) == 0 {
        slot_used |= accum_slot(c, q);
    }
    if slot_used {
        let n = c.r32(SLOT_COUNT);
        c.w32(SLOT_COUNT, n.wrapping_add(1));
    }
    let b6 = c.r8(q + 6) as u32;
    let t_hi = c.r32(TIME_HI) as i32;
    let shift = b6.saturating_sub(6) & 31;
    let bit = ((1i32 << shift) as i64 as u64) << 14;
    let qf = c.r16(q + 4);
    let pf = c.r16(p + 4);
    let width: u64 = if (qf as i64) < ((t_hi >> 8) as i64) { 27 } else { 19 };
    let b7 = c.r8(q + 7) as u64;
    (qf as u64)
        | bit
        | (width << 20)
        | ((b6 as u64) << 26)
        | (b7 << 30)
        | ((pf as u64) << 37)
        | (1u64 << 34)
        | (1u64 << 63)
}

/// 0x22e400 index_pass(idx) -> v0: slot-table pass for table entry idx; returns the packed 64-bit value.
pub fn index_pass(c: &mut Cpu) -> bool {
    let idx = c.arg(0);
    let v = pass(c, idx);
    c.s(2, v);
    true
}

/// 0x22e620 quad_pair(idx, x, y, z, t0, t1) -> ptr: appends a GIF packet of two textured sprites for entry idx to
/// the GS buffer and returns the new buffer pointer.
pub fn quad_pair(c: &mut Cpu) -> bool {
    let idx = c.arg(0);
    let x = c.arg(1);
    let y = c.arg(2);
    let z = c.arg(3);
    let dx = c.arg(4);
    let dy = c.arg(5);
    let entry = c.r32(BLOCK + OFF_INDEX).wrapping_add(idx << 2);
    let h1 = c.r16(entry + 2) as i16 as i32;
    let rec = c.r32(BLOCK + OFF_LIST2).wrapping_add((h1 << 3) as u32);
    let b6 = c.r8(rec + 6) as u32;
    let b7 = c.r8(rec + 7) as u32;
    let s5 = 1u32 << (b6 & 31);
    let s4 = 1u32 << (b7 & 31);
    let gs = c.r32(GS_PTR);
    c.w32(gs, GIF_TAG_LO);
    c.w32(gs + 4, 0);
    c.w32(gs + 8, 0);
    c.w32(gs + 0xc, GIF_TAG_HI);
    let base = gs + 0x10;
    put64(c, base, SPRITE_TAG);
    put64(c, base + 8, SPRITE_REG_PAIR);
    c.w32(GS_PTR, base);
    let packed = pass(c, idx);
    put64(c, base + 0x10, packed);
    put64(c, base + 0x18, TEXTURE_WORD);
    put64(c, base + 0x20, ((dy as i32 as i64 as u64) << 24) | COLOR_MASK);
    put64(c, base + 0x28, 0);
    let view_x = c.r32(VIEW + 0x10);
    let view_y = c.r32(VIEW + 0x14);
    let quad_word = c.r32(BLOCK + OFF_QUAD_WORD) as u64;
    let quad = |px: u32, py: u32| -> u64 {
        let lo = sext((px << 4).wrapping_add(view_x).wrapping_sub(8));
        let hi = sext((py << 4).wrapping_add(view_y).wrapping_sub(8)) << 16;
        lo | hi | (quad_word << 32)
    };
    let first = quad(x, y);
    let second = quad(x.wrapping_add(z), y.wrapping_add(dx));
    put64(c, base + 0x30, first);
    put64(c, base + 0x38, sext((s4 << 20).wrapping_add(s5 << 4)));
    put64(c, base + 0x40, second);
    put64(c, base + 0x48, 0);
    let end = base + 0x50;
    c.w32(GS_PTR, end);
    c.ret(end);
    true
}
