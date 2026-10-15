//! GIF packet writer for one untextured GS rectangle (rec/l4_05): appends a three-quadword header and two
//! packed vertex quadwords to the packet cursor, then publishes the new cursor.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Global write cursor for the current GS packet (lui 0x16 + 0x1140).
const PACKET_CURSOR: u32 = 0x0016_1140;
/// Table of signed viewport offsets read by the writer (lui 0x14 + -0x1b00): x at +0x10, y at +0x14.
const OFFSET_TABLE: u32 = 0x0013_e500;
/// Gp-relative slot that receives the packet end pointer (gp - 0x5ac0).
const GP_PACKET_END: i32 = -0x5ac0;
/// Header word of the first quadword (GIF tag, 0x10000003).
const TAG_LO: u32 = 0x1000_0003;
/// Header word of the second half of the first quadword (0x50000003).
const TAG_HI: u32 = 0x5000_0003;
/// Packed register list of the first data quadword: 0x8800 in the high word, 1 in the low word.
const REG_LIST_LO: u64 = (0x8800u64 << 47) | 1;
/// Packed register list of the second data quadword.
const REG_LIST_HI: u64 = 0x4410;
/// A+D register for the vertex quadwords (0x46 = PRIM).
const PRIM_REG: u64 = 0x46;
/// Mask that sets the fixed bits (0xffff_f000) of each packed vertex word.
const VERTEX_MASK: u64 = ((0xFFFFu64 << 16) | 0xF000) << 24;

/// Sign-extends a 32-bit result the way a MIPS 64-bit register holds it after an addu/addiu/sll.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Packs one vertex word: x in the low half, y shifted up 16 bits, plus the fixed mask.
fn pack_vertex(x: u32, y: u32, ox: u32, oy: u32, mask: u64) -> u64 {
    let xx = sx(x.wrapping_add(ox).wrapping_sub(8));
    let yy = sx(y.wrapping_add(oy).wrapping_sub(8)) << 16;
    xx | yy | mask
}

/// Same as `pack_vertex` for the scaled path: coordinates are shifted left 4 bits (1/16 px) before the offset.
fn pack_vertex_scaled(x: u32, y: u32, ox: u32, oy: u32, mask: u64) -> u64 {
    let xx = sx(x.wrapping_shl(4).wrapping_add(ox).wrapping_sub(0x10));
    let yy = sx(y.wrapping_shl(4).wrapping_add(oy).wrapping_sub(0x10)) << 16;
    xx | yy | mask
}

/// Writes a doubleword (SD) as two words.
fn sd(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a + 4, (v >> 32) as u32);
}

/// 0x22fbf0 gif_rect_pkt(x0, y0, x1, y1, rgba, flag) -> packet end: appends one GS rectangle to the GIF packet
/// (flag != 0 uses pixel coords, otherwise 1/16 px coords) and stores the new end in gp-0x5ac0.
pub fn gif_rect_pkt(c: &mut Cpu) -> bool {
    let x0 = c.arg(0);
    let y0 = c.arg(1);
    let x1 = c.arg(2);
    let y1 = c.arg(3);
    let rgba = c.g(8);
    let flag = c.g(9);
    let p = c.r32(PACKET_CURSOR);

    c.w32(p, TAG_LO);
    c.w32(p + 4, 0);
    c.w32(p + 8, 0);
    c.w32(p + 0xc, TAG_HI);
    sd(c, p + 0x10, REG_LIST_LO);
    sd(c, p + 0x18, REG_LIST_HI);
    sd(c, p + 0x20, PRIM_REG);
    sd(c, p + 0x28, rgba);
    c.w32(PACKET_CURSOR, p + 0x10);

    let ox = c.r32(OFFSET_TABLE + 0x10);
    let oy = c.r32(OFFSET_TABLE + 0x14);
    let (v0, v1) = if flag != 0 {
        (pack_vertex(x0, y0, ox, oy, VERTEX_MASK), pack_vertex(x1, y1, ox, oy, VERTEX_MASK))
    } else {
        (
            pack_vertex_scaled(x0, y0, ox, oy, VERTEX_MASK),
            pack_vertex_scaled(x1, y1, ox, oy, VERTEX_MASK),
        )
    };
    sd(c, p + 0x30, v0);
    sd(c, p + 0x38, v1);

    let end = p + 0x40;
    c.ret(end);
    c.w32(gp_at(c, GP_PACKET_END), end);
    true
}

/// Gp-relative address.
fn gp_at(c: &Cpu, off: i32) -> u32 {
    (c.g(28) as u32).wrapping_add(off as u32)
}
