//! GIF packet writer for a box of four vertices (rec/l4_12): appends a DMA tag, a GIF header template and six
//! data quadwords (colour, four packed xy vertices, and float pairs) at the packet cursor, then moves the cursor.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Global write cursor for the current GS packet (lui 0x16 + 0x1140).
const PACKET_CURSOR: u32 = 0x0016_1140;
/// Table of signed viewport offsets (lui 0x14 + -0x1b00): x at +0x10, y at +0x14.
const OFFSET_TABLE: u32 = 0x0013_e500;
/// Quadword template copied to the GIF header slot after the DMA tag (lui 0x16 + 0x890).
const HEADER_TEMPLATE: u32 = 0x0016_0890;
/// Low word of the DMA tag (REF tag, QWC 7).
const TAG_LO: u32 = 0x1000_0007;
/// High word of the DMA tag quadword (at +0xc).
const TAG_HI: u32 = 0x5000_0007;
/// Register descriptor word of the first data quadword.
const REG_DESC: u64 = 0x54;
/// Float 1.0, stored in the high word of the second data quadword.
const ONE_BITS: u64 = 0x3f80_0000;
/// Fixed bits of each packed vertex word (0xffff_f000 shifted left 24).
const VERTEX_MASK: u64 = ((0xFFFFu64 << 16) | 0xF000) << 24;
/// Bias subtracted from each scaled coordinate (half a pixel in 1/16 units).
const BIAS: u32 = 8;
/// Value the retail code leaves in `at` (lui 0x16).
const AT_VALUE: u64 = 0x0016_0000;
/// Size of the packet after the tag: the header quadword and six data quadwords.
const PACKET_SPAN: u32 = 0x80;
/// Size of the stack frame the retail prologue reserves.
const FRAME: u32 = 0x20;

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
const SP: usize = 29;

/// Sign-extends a 32-bit result the way a MIPS 64-bit register holds it.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Two 32-bit words packed into one doubleword (`lo` in bits 0..31).
fn pair(lo: u32, hi: u32) -> u64 {
    lo as u64 | ((hi as u64) << 32)
}

/// Writes a doubleword (SD) as two words.
fn sd(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a.wrapping_add(4), (v >> 32) as u32);
}

/// Packs one vertex: x in the low word (sign-extended), y shifted up 16, plus the fixed mask.
fn vertex_word(x: u32, y: u32) -> u64 {
    sx(x) | (sx(y) << 16) | VERTEX_MASK
}

/// 0x282d20 box_quad_pkt(x, y, dx, dy, color, t0_word, f12..f15) -> none: appends a four-vertex GS packet
/// whose corners are (x, y) and (x+dx, y+dy) in 1/16 px (minus half a pixel), with the float args stored as
/// raw words in the packet and on the stack frame below the caller's sp.
pub fn box_quad_pkt(c: &mut Cpu) -> bool {
    let x = c.g(A0) as u32;
    let y = c.g(A1) as u32;
    let dx = c.g(A2) as u32;
    let dy = c.g(A3) as u32;
    let color = c.g(T1);
    let t0_lo = c.g(T0) as u32;
    let f = [c.farg(0).0, c.farg(1).0, c.farg(2).0, c.farg(3).0];
    let (f12, f13, f14, f15) = (f[0], f[1], f[2], f[3]);
    let ox = c.r32(OFFSET_TABLE + 0x10);
    let oy = c.r32(OFFSET_TABLE + 0x14);

    // Scaled corner coordinates: (v << 4) + offset - bias, all in 32-bit arithmetic.
    let sx_of = |v: u32| (v << 4).wrapping_add(ox).wrapping_sub(BIAS);
    let sy_of = |v: u32| (v << 4).wrapping_add(oy).wrapping_sub(BIAS);
    let x0 = sx_of(x);
    let x1 = sx_of(x.wrapping_add(dx));
    let y0 = sy_of(y);
    let y1 = sy_of(y.wrapping_add(dy));
    // Vertex order in the packet: (x0,y0), (x1,y0), (x0,y1), (x1,y1).
    let verts = [vertex_word(x0, y0), vertex_word(x1, y0), vertex_word(x0, y1), vertex_word(x1, y1)];

    // Stack frame below the caller's sp: the float args are spilled as raw words.
    let sp = c.g(SP) as u32;
    let frame = sp.wrapping_sub(FRAME);
    let spill = [f12, f14, f13, f14, f12, f15, f13, f15];
    for (i, w) in spill.iter().enumerate() {
        c.w32(frame.wrapping_add(4 * i as u32), *w);
    }

    // DMA tag, then the GIF header quadword copied from the template.
    let p = c.r32(PACKET_CURSOR);
    c.w32(p, TAG_LO);
    c.w32(p.wrapping_add(4), 0);
    c.w32(p.wrapping_add(8), 0);
    c.w32(p.wrapping_add(0xc), TAG_HI);
    for i in 0..4 {
        let w = c.r32(HEADER_TEMPLATE + 4 * i);
        c.w32(p.wrapping_add(0x10 + 4 * i), w);
    }

    // Six data quadwords at p+0x20.
    let data = p.wrapping_add(0x20);
    sd(c, data, color);
    sd(c, data.wrapping_add(0x08), REG_DESC);
    sd(c, data.wrapping_add(0x10), pair(t0_lo, ONE_BITS as u32));
    sd(c, data.wrapping_add(0x18), pair(f12, f14));
    sd(c, data.wrapping_add(0x20), verts[0]);
    sd(c, data.wrapping_add(0x28), pair(f13, f14));
    sd(c, data.wrapping_add(0x30), verts[1]);
    sd(c, data.wrapping_add(0x38), pair(f12, f15));
    sd(c, data.wrapping_add(0x40), verts[2]);
    sd(c, data.wrapping_add(0x48), pair(f13, f15));
    sd(c, data.wrapping_add(0x50), verts[3]);
    sd(c, data.wrapping_add(0x58), 0);
    c.w32(PACKET_CURSOR, p.wrapping_add(PACKET_SPAN));

    // Registers the retail code leaves behind.
    c.s(AT, AT_VALUE);
    c.s(V0, sx(p.wrapping_add(PACKET_SPAN)));
    c.s(V1, pair(f12, f15));
    c.s(A0, verts[2]);
    c.s(A1, verts[1]);
    c.s(A2, verts[3]);
    c.s(A3, sx(y1) << 16);
    c.s(T0, VERTEX_MASK);
    c.s(T2, sx(data));
    c.s(T3, sx(p));
    c.s(T4, HEADER_TEMPLATE as u64);
    c.s(T5, sx(oy));
    true
}
