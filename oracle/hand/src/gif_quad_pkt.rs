//! GIF packet writer for one two-vertex GS quad (rec/l4_05): appends a DMA tag, a GIF header and two packed
//! vertex quadwords at the packet cursor, then stores the new end in gp-0x5ac0.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Global write cursor for the current GS packet (lui 0x16 + 0x1140).
const PACKET_CURSOR: u32 = 0x0016_1140;
/// Table of signed viewport offsets read by the writer (lui 0x14 + -0x1b00): x at +0x10, y at +0x14.
const OFFSET_TABLE: u32 = 0x0013_e500;
/// Gp-relative slot that receives the packet end pointer (gp - 0x5ac0).
const GP_PACKET_END: i32 = -0x5ac0;
/// Low word of the first DMA tag (REF tag, QWC 3).
const TAG_LO: u32 = 0x1000_0003;
/// Word at +0xc of the DMA tag quadword (0x50000003).
const TAG_HI: u32 = 0x5000_0003;
/// GIF A+D register list of the first data quadword (0x8800 in bits 47..62, 1 in the low bits).
const REG_LIST_LO: u64 = (0x8800u64 << 47) | 1;
/// GIF A+D register list of the second data quadword.
const REG_LIST_HI: u64 = 0x4410;
/// Register descriptor of the third data quadword.
const REG_DESC: u64 = 0x41;
/// Fixed bits (0xffff_f000) of each packed vertex word.
const VERTEX_MASK: u64 = ((0xFFFFu64 << 16) | 0xF000) << 24;
/// Left in the table base register (lui 0x14 + -0x1b00) and in a2/a0 by the scaled path.
const TABLE_BASE: u64 = 0x0013_e500;
/// Value of `at` after the cursor load (lui 0x16).
const AT_VALUE: u64 = 0x0016_0000;

// MIPS register numbers.
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
const T4: usize = 12;
const T5: usize = 13;
const T6: usize = 14;
const T7: usize = 15;
const T8: usize = 24;
const T9: usize = 25;

/// Sign-extends a 32-bit result the way a MIPS 64-bit register holds it after an addu/addiu/sll.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Writes a doubleword (SD) as two words.
fn sd(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a.wrapping_add(4), (v >> 32) as u32);
}

/// Gp-relative address.
fn gp_at(c: &Cpu, off: i32) -> u32 {
    (c.g(28) as u32).wrapping_add(off as u32)
}

/// Packs the x word (low half) and the y word (shifted up 16) of one vertex, without the fixed mask.
/// `scaled` multiplies the coordinates by 16 first and uses a 0x10 bias instead of 8.
fn vertex_parts(x: u64, y: u64, ox: u32, oy: u32, scaled: bool) -> (u64, u64) {
    let (x, y, bias) = if scaled {
        ((x as u32) << 4, (y as u32) << 4, 0x10)
    } else {
        (x as u32, y as u32, 8)
    };
    let xx = sx(x.wrapping_add(ox).wrapping_sub(bias));
    let yy = sx(y.wrapping_add(oy).wrapping_sub(bias)) << 16;
    (xx, yy)
}

/// 0x22fa68 gif_quad_pkt(x0,y0,x1,y1,rgba,flag) -> end: appends a GS quad packet (flag 0: 1/16 px coords).
pub fn gif_quad_pkt(c: &mut Cpu) -> bool {
    let a0 = c.g(A0);
    let a1 = c.g(A1);
    let a2 = c.g(A2);
    let a3 = c.g(A3);
    let rgba = c.g(T0);
    let flag = c.g(T1);
    let p = c.r32(PACKET_CURSOR);

    c.w32(p, TAG_LO);
    c.w32(p.wrapping_add(4), 0);
    c.w32(p.wrapping_add(8), 0);
    c.w32(p.wrapping_add(0xc), TAG_HI);
    sd(c, p.wrapping_add(0x10), REG_LIST_LO);
    sd(c, p.wrapping_add(0x18), REG_LIST_HI);
    sd(c, p.wrapping_add(0x20), REG_DESC);
    sd(c, p.wrapping_add(0x28), rgba);
    c.w32(PACKET_CURSOR, p.wrapping_add(0x10));

    let ox = c.r32(OFFSET_TABLE + 0x10);
    let oy = c.r32(OFFSET_TABLE + 0x14);
    let scaled = flag == 0;
    let (x0, y0) = vertex_parts(a0, a1, ox, oy, scaled);
    let (x1, y1) = vertex_parts(a2, a3, ox, oy, scaled);
    let word0 = x0 | y0 | VERTEX_MASK;
    let word1 = x1 | y1 | VERTEX_MASK;
    sd(c, p.wrapping_add(0x30), word0);
    sd(c, p.wrapping_add(0x38), word1);

    // Registers the retail code leaves behind.
    let end = p.wrapping_add(0x40);
    c.s(T2, sx(p));
    c.s(T3, REG_DESC);
    c.s(T4, REG_LIST_LO);
    c.s(T5, REG_LIST_HI);
    c.s(AT, AT_VALUE);
    c.s(T6, a1);
    c.s(T7, a0);
    c.s(T8, a3);
    c.s(T9, a2);
    if scaled {
        c.s(V1, sx(oy));
        c.s(A0, word1);
        c.s(A1, y1);
        c.s(A2, TABLE_BASE);
        c.s(A3, sx(oy));
        c.s(T0, sx(ox));
        c.s(T1, VERTEX_MASK);
    } else {
        c.s(V1, y1);
        c.s(A0, TABLE_BASE);
        c.s(A1, VERTEX_MASK);
    }
    c.ret(end);
    c.w32(gp_at(c, GP_PACKET_END), end);
    true
}
