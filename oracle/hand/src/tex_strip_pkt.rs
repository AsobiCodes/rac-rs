//! Textured-strip packet writer: at the packet cursor it writes a 0x09 tag block and a GIF tag with a PRIM word
//! and four vertex records (read from three pointers), then advances the cursor.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Register indices (MIPS numbering).
const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;
const T2: usize = 10;
const T3: usize = 11;
const GP: usize = 28;

/// Global packet cursor (0x161140).
const CURSOR_PTR: u32 = 0x0016_1140;
/// Upper half of the cursor address, left in `at` by retail's store.
const HI_BASE: u32 = 0x0016_0000;
/// Sixteen-byte template copied to the head of the packet (0x160920).
const TEMPLATE: u32 = 0x0016_0920;
/// gp-relative slot that receives the advanced cursor.
const CURSOR_SLOT_GP: i32 = -0x5ac0;
/// Words of the leading 0x09 tag block (low word and high word).
const TAG_LO: u32 = 0x1000_0009;
const TAG_HI: u32 = 0x5000_0009;
/// PRIM register value of the textured tristrip.
const PRIM_TEX_STRIP: u64 = 0x154;

/// Loads a 32-bit word and sign-extends it to 64 bits, as `lw` does.
fn lw(c: &mut Cpu, a: u32) -> u64 {
    c.r32(a) as i32 as i64 as u64
}

/// Loads a 64-bit doubleword, as `ld` does.
fn ld(c: &mut Cpu, a: u32) -> u64 {
    let lo = c.r32(a) as u64;
    let hi = c.r32(a.wrapping_add(4)) as u64;
    lo | (hi << 32)
}

/// Stores a 64-bit doubleword, as `sd` does.
fn sd(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a.wrapping_add(4), (v >> 32) as u32);
}

/// 0x1ed5b8 emit_textured_strip(a0 verts, a1 verts, a2 verts, a3 prim-data, t0 flag): appends a 0x09 tag block, a GIF
/// tag, the PRIM word and four vertex records to the packet at the cursor, then sets the cursor to strip + 0x80.
pub fn emit_textured_strip(c: &mut Cpu) -> bool {
    let a0 = c.g(A0) as u32;
    let a1 = c.g(A1) as u32;
    let a2 = c.g(A2) as u32;
    let a3 = c.g(A3);
    let t0 = c.g(T0);
    let gp = c.g(GP) as u32;

    // Tag block at the cursor: low word, two zero words, high word.
    let p = c.r32(CURSOR_PTR);
    c.w32(p, TAG_LO);
    c.w32(p + 4, 0);
    c.w32(p + 8, 0);
    c.w32(p + 0xc, TAG_HI);

    // Copy the template to p + 0x10 and advance the cursor by 0x10.
    let block = p.wrapping_add(0x10);
    let template = c.rv(TEMPLATE);
    c.wv(block, template);
    c.w32(CURSOR_PTR, block);

    // GIF tag at p + 0x20: NLOOP 5 when the flag is set, else zero.
    let strip = p.wrapping_add(0x20);
    sd(c, strip, if t0 != 0 { 5 } else { 0 });

    // PRIM word, then four vertex records of three doublewords each.
    sd(c, strip + 8, a3);
    sd(c, strip + 0x10, PRIM_TEX_STRIP);
    let mut last = 0;
    for v in 0..4u32 {
        let dst = strip + 0x18 + 0x18 * v;
        let w2 = lw(c, a2 + 4 * v);
        sd(c, dst, w2);
        let w1 = lw(c, a1 + 4 * v);
        sd(c, dst + 8, w1);
        last = ld(c, a0 + 8 * v);
        sd(c, dst + 0x10, last);
    }
    sd(c, strip + 0x78, 0);

    // The slot (the cursor word) was set to strip at the GIF tag, so the final read gives strip + 0x80; retail
    // leaves that in v0 and stores it back.
    let next = strip.wrapping_add(0x80);
    c.w32(gp.wrapping_add(CURSOR_SLOT_GP as u32), next);

    c.s(AT, HI_BASE as u64);
    c.s(V0, next as i32 as i64 as u64);
    c.s(V1, last);
    c.s(T1, strip as i32 as i64 as u64);
    c.s(T2, TEMPLATE as u64);
    c.s(T3, p as i32 as i64 as u64);
    true
}
