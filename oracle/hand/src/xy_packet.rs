//! Draw-packet builder (Eudora overlay): writes a 64-byte GIF packet at the packet pointer (a tag, three A+D
//! register pairs and two XYZ2 vertex words) and advances the pointer by 0x40. Vertex coordinates are either raw
//! (t2 != 0) or scaled by 16 as 12.4 fixed point (t2 == 0), then offset by the table words.
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Packet-buffer write pointer (a RAM word that holds the address of the next free packet).
const PACKET_PTR: u32 = 0x16_1140;
/// Offset table: word +0x10 is the y offset (applied to the low lane), word +0x14 the x offset (high lane).
const OFFSET_TABLE: u32 = 0x13_E500;
const OFFSET_LO: u32 = 0x10;
const OFFSET_HI: u32 = 0x14;
/// Packet header words: the GIF tag's two halves and the A+D register addresses.
const TAG_LO: u32 = 0x1000_0003;
const TAG_HI: u32 = 0x5000_0003;
const TAG_LAYOUT: u64 = 0x8800;
const TAG_LAYOUT_SHIFT: u32 = 47;
const REG_A: u64 = 0x4410;
const REG_B: u64 = 0x46;
/// Bytes the packet pointer advances by.
const PACKET_SIZE: u32 = 0x40;
/// Vertex bias: one pixel back in raw units, and in 12.4 units when scaled.
const BIAS_RAW: u32 = 8;
const BIAS_SCALED: u32 = 0x10;
/// Scale of the 12.4 fixed-point path.
const SCALE_SHIFT: u32 = 4;
/// Shift that puts a vertex's high lane in bits 16..31.
const HI_LANE_SHIFT: u32 = 16;
/// Scratch-register numbers used by the routine.
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
const S0: usize = 16;
const T8: usize = 24;
const T9: usize = 25;
const SP: usize = 29;

/// Sign-extends a 32-bit word to the 64-bit register value.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Stores a 64-bit doubleword little-endian.
fn put64(c: &mut Cpu, addr: u32, v: u64) {
    c.w32(addr, v as u32);
    c.w32(addr + 4, (v >> 32) as u32);
}

/// One XYZ2 word: the low lane (sign-extended) and the high lane shifted to bits 16..31, OR'd with the shared bits.
fn vertex(lo: u32, hi: u32, shared: u64) -> u64 {
    sx(lo) | (sx(hi) << HI_LANE_SHIFT) | shared
}

/// 0x22fd78 gif_xy_packet(a0..a3 coords, t0 tail word, t1 flags, t2 scaled-flag): writes the 64-byte packet at the
/// packet pointer and advances the pointer past it. Coordinates are scaled by 16 when t2 == 0, raw otherwise.
pub fn gif_xy_packet(c: &mut Cpu) -> bool {
    let p = c.r32(PACKET_PTR);
    let save_at = (c.g(SP) as u32).wrapping_sub(16);
    let s0_quad = c.r[S0];
    c.wv(save_at, V(std::array::from_fn(|i| (s0_quad >> (32 * i)) as u32)));

    // Retail copies the coordinate args into t7/t8/t9 and a2 into s0 (s0 is restored from the stack afterwards).
    let (a0_in, a1_in, a2_in, a3_in) = (c.g(A0), c.g(A1), c.g(A2), c.g(A3));
    let t7 = a1_in as u32;
    let t8 = a0_in as u32;
    let t9 = a3_in as u32;
    let s0 = a2_in as u32;
    let t1 = c.g(T1);
    let t2 = c.g(T2);
    let tail = c.g(T0);
    let shared = t1 << 32;
    let off_lo = c.r32(OFFSET_TABLE + OFFSET_LO);
    let off_hi = c.r32(OFFSET_TABLE + OFFSET_HI);

    c.w32(p, TAG_LO);
    c.w32(p + 4, 0);
    c.w32(p + 8, 0);
    c.w32(p + 0xc, TAG_HI);
    let tag_word = (TAG_LAYOUT << TAG_LAYOUT_SHIFT) | 1;
    put64(c, p + 0x10, tag_word);
    put64(c, p + 0x18, REG_A);
    put64(c, p + 0x20, REG_B);
    put64(c, p + 0x28, tail);
    c.w32(PACKET_PTR, p.wrapping_add(0x10));

    let (bias, first, second) = if t2 != 0 {
        (BIAS_RAW, (t7, t8), (t9, s0))
    } else {
        // 12.4 fixed point: scale the coordinates before the offset.
        let scale = |v: u32| v << SCALE_SHIFT;
        (BIAS_SCALED, (scale(t7), scale(t8)), (scale(t9), scale(s0)))
    };
    let vx = |x: u32| x.wrapping_add(off_hi).wrapping_sub(bias);
    let vy = |y: u32| y.wrapping_add(off_lo).wrapping_sub(bias);
    let v0_word = vertex(vy(first.1), vx(first.0), shared);
    let v1_word = vertex(vy(second.1), vx(second.0), shared);
    put64(c, p + 0x30, v0_word);
    put64(c, p + 0x38, v1_word);

    let next = p.wrapping_add(PACKET_SIZE);
    c.w32(PACKET_PTR, next);

    // Scratch registers as retail leaves them.
    c.s(T7, a1_in);
    c.s(T8, a0_in);
    c.s(T9, a3_in);
    c.s(V0, sx(next));
    c.s(AT, 0x16_0000);
    c.s(T3, sx(p));
    c.s(T4, REG_B);
    c.s(T5, tag_word);
    c.s(T6, REG_A);
    let second_hi = sx(vx(second.0)) << HI_LANE_SHIFT;
    if t2 != 0 {
        c.s(V1, second_hi);
        c.s(A0, sx(OFFSET_TABLE));
        c.s(A1, shared);
    } else {
        c.s(V1, sx(off_hi));
        c.s(T1, shared);
        c.s(A0, v1_word);
        c.s(A1, second_hi);
        c.s(A2, sx(OFFSET_TABLE));
        c.s(A3, sx(off_hi));
        c.s(T0, sx(off_lo));
    }
    true
}
