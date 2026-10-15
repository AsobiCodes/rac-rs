//! GIF packet assembly (retail 0x1f0430) and GS register writes (retail 0x1f1718).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// gp - 0x5ac0 holds the packet write pointer.
const PTR_SLOT_OFS: u32 = 0x5ac0;
/// Five-quadword packet header template.
const HEAD_TPL: u32 = 0x001c_40a0;
/// Quadword templates written ahead of the three data blocks.
const BLOCK_A_TPL: u32 = 0x0016_0e40;
const BLOCK_B_TPL: u32 = 0x0016_0e60;
const BLOCK_C_TPL: u32 = 0x0016_0e80;
/// Tag words closing a packet; the first is OR-ed with the low bits of the flag argument.
const TAG_HI: u32 = 0x1500_0000;
const TAG_LO: u32 = 0x1100_0000;
/// GS register window and the status word read by `gs_regs_write`.
const GS_BASE: u32 = 0x1000_d000;
const GS_STATUS: u32 = 0x2010_0000;

/// MIPS register numbers used for the final register state.
const REG_AT: usize = 1;
const REG_V0: usize = 2;
const REG_V1: usize = 3;
const REG_A0: usize = 4;
const REG_T0: usize = 8;
const REG_T2: usize = 10;
const REG_T3: usize = 11;
const REG_T5: usize = 13;
const REG_T7: usize = 15;
const REG_T9: usize = 25;
const REG_GP: usize = 28;

/// Sign-extends a 32-bit value to the 64-bit register form.
fn sext(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// lq/sq: copies one quadword and returns the next destination address.
fn copy_quad(c: &mut Cpu, dst: u32, src: u32) -> u32 {
    let v = c.rv(src & !0xf);
    c.wv(dst & !0xf, v);
    dst.wrapping_add(0x10)
}

/// lw / or / sw on one word: stores (sign-extended word | mask) and returns that register value.
fn or_word(c: &mut Cpu, addr: u32, mask: u64) -> u64 {
    let v = sext(c.r32(addr)) | mask;
    c.w32(addr, v as u32);
    v
}

/// sd: stores a doubleword as two words.
fn store_dword(c: &mut Cpu, addr: u32, v: u64) {
    c.w32(addr, v as u32);
    c.w32(addr.wrapping_add(4), (v >> 32) as u32);
}

/// Stores zero words until the address is 16-byte aligned.
fn pad_to_quad(c: &mut Cpu, mut at: u32) -> u32 {
    while at & 0xf != 0 {
        c.w32(at, 0);
        at = at.wrapping_add(4);
    }
    at
}

/// 0x1f0430 build_gif_packet(count, triples, words, dwords, flag) -> appends one GIF packet (header template, three
/// data blocks of `count` entries of 8, 4 and 12 bytes, tail tags) to the buffer at the write pointer kept in
/// gp-0x5ac0, updates the pointer and the packet's word count.
pub fn build_gif_packet(c: &mut Cpu) -> bool {
    let count = c.arg(0);
    let triples = c.arg(1);
    let words = c.arg(2);
    let dwords = c.arg(3);
    let a0 = c.g(REG_A0);
    let t0 = ((c.arg(4) & 1) << 1) + 0xc;
    let t7 = sext(count << 16);
    let slot = (c.g(REG_GP) as u32).wrapping_sub(PTR_SLOT_OFS);
    let head = c.r32(slot);
    let mut at = head;

    // Header: five template quadwords, then the count mixed into the word at +0x10.
    let mut src = HEAD_TPL;
    for _ in 0..5 {
        at = copy_quad(c, at, src);
        src += 0x10;
    }
    or_word(c, at.wrapping_sub(0x40), a0);

    // Block A: `count` doublewords from `dwords`, framed by a template quadword.
    at = copy_quad(c, at, BLOCK_A_TPL);
    or_word(c, at.wrapping_sub(4), t7);
    let mut src = dwords;
    for _ in 0..count {
        let v = c.r32(src) as u64 | ((c.r32(src.wrapping_add(4)) as u64) << 32);
        store_dword(c, at, v);
        src = src.wrapping_add(8);
        at = at.wrapping_add(8);
    }
    at = pad_to_quad(c, at);

    // Block B: `count` words from `words`.
    at = copy_quad(c, at, BLOCK_B_TPL);
    or_word(c, at.wrapping_sub(4), t7);
    let mut src = words;
    for _ in 0..count {
        let w = c.r32(src);
        c.w32(at, w);
        src = src.wrapping_add(4);
        at = at.wrapping_add(4);
    }
    at = pad_to_quad(c, at);

    // Block C: `count` triples of words from `triples`.
    let t9 = at;
    // v0 keeps the high doubleword of the last quadword loaded into it.
    let tpl_c = c.rv(BLOCK_C_TPL & !0xf).0;
    let v0_hi = tpl_c[2] as u64 | ((tpl_c[3] as u64) << 32);
    at = copy_quad(c, at, BLOCK_C_TPL);
    or_word(c, at.wrapping_sub(4), t7);
    let mut src = triples;
    for _ in 0..count {
        for k in 0..3u32 {
            let w = c.r32(src.wrapping_add(4 * k));
            c.w32(at.wrapping_add(4 * k), w);
        }
        src = src.wrapping_add(12);
        at = at.wrapping_add(12);
    }
    let t3 = triples.wrapping_add(count.wrapping_mul(12));
    at = pad_to_quad(c, at);

    // Tail: two tag words and a zero doubleword, then the packet's word count in its header.
    c.w32(at, TAG_HI | t0);
    c.w32(at.wrapping_add(4), TAG_LO);
    store_dword(c, at.wrapping_add(8), 0);
    at = at.wrapping_add(0x10);
    let qwords = (at.wrapping_sub(head) >> 4).wrapping_sub(1);
    let v1 = or_word(c, head, sext(qwords));

    // Publish the new write pointer.
    c.w32(slot, at);

    let slot_addr = sext((c.g(REG_GP) as u32).wrapping_sub(PTR_SLOT_OFS));
    c.r[REG_V0] = ((v0_hi as u128) << 64) | slot_addr as u128;
    c.s(REG_V1, v1);
    c.s(REG_AT, sext(at));
    c.s(REG_T2, sext(at));
    c.s(REG_T3, sext(t3));
    c.s(REG_T0, t0 as u64);
    c.s(REG_T5, 0x7000_0000);
    c.s(REG_T7, t7);
    c.s(REG_T9, sext(t9));
    true
}

/// 0x1f1718 gs_regs_write(a0, a1, a2) -> stores a0, a1, a2 into GS registers at 0x1000d010, 0x1000d080,
/// 0x1000d020, writes 0x100 to 0x1000d000, and returns v0 = 0x100, v1 = (status word at 0x20100000) | 1.
pub fn gs_regs_write(c: &mut Cpu) -> bool {
    let (a0, a1, a2) = (c.arg(0), c.arg(1), c.arg(2));
    c.w32(GS_BASE + 0x80, a1);
    c.w32(GS_BASE + 0x10, a0);
    c.w32(GS_BASE + 0x20, a2);
    c.w32(GS_BASE, 0x100);
    let status = sext(c.r32(GS_STATUS)) | 1;
    c.ret(0x100);
    c.s(REG_V1, status);
    c.s(REG_AT, GS_BASE as u64);
    true
}
