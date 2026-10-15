//! GS sprite packet emitter and glyph-advance text width (retail 0x1ecaf8, 0x1ed9a8, 0x1ed9f8).
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Per-viewport 12.4 offsets: X at +0x10 and Y at +0x14 of this table.
const VIEW_OFFSETS: u32 = 0x13_e500;
/// Global pointer to the next free byte of the GS packet buffer.
const PACKET_PTR: u32 = 0x16_1140;
/// 16-byte GIF tag template copied into the second quadword of each packet.
const GIF_TEMPLATE: u32 = 0x16_0880;
const GIF_TAG_WORD: u32 = 0x1000_0007;
const GIF_AD_WORD: u32 = 0x5000_0007;
/// GS register address word written after the first data word.
const GS_REG_ADDR: u64 = 0x154;
/// Constant OR-ed into every position doubleword (bits 36..55 set).
const POS_TAG: u64 = ((0xffff_u64 << 16) | 0xf000) << 24;
/// Glyph table for the font at 0x1c3250: four bytes per character, advance is the signed byte at +3.
const FONT_TABLE: u32 = 0x1c_3250;
/// Size of one sprite packet in bytes.
const PACKET_SIZE: u32 = 0x80;

/// Quadword from a 128-bit register value.
fn quad(q: u128) -> V {
    V(std::array::from_fn(|i| (q >> (32 * i)) as u32))
}

/// Sign-extends a 32-bit result to the 64-bit register value.
fn sext(v: u32) -> u64 {
    v as i32 as i64 as u64
}

fn write64(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a.wrapping_add(4), (v >> 32) as u32);
}

fn read64(c: &mut Cpu, a: u32) -> u64 {
    (c.r32(a) as u64) | ((c.r32(a.wrapping_add(4)) as u64) << 32)
}

/// 12.4 screen coordinate: pixels * 16 + view offset - half a pixel.
fn screen12(v: u32, offset: u32) -> u32 {
    (v << 4).wrapping_add(offset).wrapping_sub(8)
}

/// Position doubleword: x in the low half, y shifted to bit 16, plus the constant tag.
fn position_word(x: u32, y: u32) -> u64 {
    sext(x) | (sext(y) << 16) | POS_TAG
}

/// 0x1ecaf8 sprite_blit_int(x, y, w, h, u, v, du, dv, s0_arg, s1_arg): appends one 0x80-byte GS textured sprite
/// (GIF tag, then the two corner positions and texture coords in 12.4 fixed point) at the packet pointer.
pub fn sprite_blit_int(c: &mut Cpu) -> bool {
    let (x, y, w, h) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let (u, v, du, dv) = (c.arg(4), c.arg(5), c.arg(6), c.arg(7));
    let sp = c.g(29) as u32;
    // Prologue spills s0/s1 to the new frame; the 9th and 10th args sit at the caller's stack pointer.
    let (saved_s0, saved_s1) = (c.r[16], c.r[17]);
    c.wv(sp.wrapping_sub(0x20), quad(saved_s0));
    c.wv(sp.wrapping_sub(0x10), quad(saved_s1));
    let arg9 = read64(c, sp);
    let arg10 = read64(c, sp.wrapping_add(8));

    let off_x = c.r32(VIEW_OFFSETS + 0x10);
    let off_y = c.r32(VIEW_OFFSETS + 0x14);
    let (x0, y0) = (screen12(x, off_x), screen12(y, off_y));
    let (x1, y1) = (screen12(x.wrapping_add(w), off_x), screen12(y.wrapping_add(h), off_y));

    // Texture coords: u in the low field, v in the field shifted by 20.
    let tu0 = u << 4;
    let tu1 = u.wrapping_add(du) << 4;
    let tv0 = v << 20;
    let tv1 = v.wrapping_add(dv) << 20;
    let uv00 = tv0.wrapping_add(tu0);
    let uv10 = tv0.wrapping_add(tu1);
    let uv01 = tv1.wrapping_add(tu0);
    let uv11 = tv1.wrapping_add(tu1);

    let pos00 = position_word(x0, y0);
    let pos10 = position_word(x1, y0);
    let pos01 = position_word(x0, y1);
    let pos11 = position_word(x1, y1);

    let packet = c.r32(PACKET_PTR);
    c.w32(packet, GIF_TAG_WORD);
    c.w32(packet.wrapping_add(4), 0);
    c.w32(packet.wrapping_add(8), 0);
    c.w32(packet.wrapping_add(0xc), GIF_AD_WORD);
    let template = c.rv(GIF_TEMPLATE);
    c.wv(packet.wrapping_add(0x10), template);
    let words: [(u32, u64); 12] = [
        (0x20, arg10),
        (0x28, GS_REG_ADDR),
        (0x30, arg9),
        (0x38, sext(uv00)),
        (0x40, pos00),
        (0x48, sext(uv10)),
        (0x50, pos10),
        (0x58, sext(uv01)),
        (0x60, pos01),
        (0x68, sext(uv11)),
        (0x70, pos11),
        (0x78, 0),
    ];
    for (off, val) in words {
        write64(c, packet.wrapping_add(off), val);
    }
    let end = packet.wrapping_add(PACKET_SIZE);
    c.w32(PACKET_PTR, end);

    // Scratch registers as the retail code leaves them. The template lq left its high doubleword in v0.
    let template_hi = ((template.0[3] as u64) << 32) | template.0[2] as u64;
    c.r[2] = ((template_hi as u128) << 64) | sext(end) as u128;
    c.s(3, sext(packet.wrapping_add(0x20)));
    c.s(15, sext(off_y));
    c.s(4, pos00);
    c.s(5, GS_REG_ADDR);
    c.s(6, pos10);
    c.s(7, pos01);
    c.s(8, sext(tu0));
    c.s(9, sext(uv00));
    c.s(10, sext(uv10));
    c.s(11, sext(uv01));
    c.s(12, pos11);
    c.s(13, sext(packet));
    c.s(14, sext(uv11));
    true
}

/// Sums the glyph advances of up to n characters of the NUL-terminated string s, using the table at `table`.
/// Leaves the retail scratch registers (a0, a3, t0, t1, v1) as the loop leaves them. Returns the width.
fn width_n(c: &mut Cpu, s: u32, n: u32, table: u32) -> u32 {
    if n == 0 || c.r8(s) == 0 {
        c.s(8, 0);
        c.s(9, 0);
        return 0;
    }
    let mut sum: u32 = 0;
    let mut count: u32 = 0;
    let mut ch = c.r8(s) as u32;
    let (mut adv, mut v1, mut scan): (u32, u32, u32);
    loop {
        count += 1;
        scan = s.wrapping_add(count);
        adv = c.r8(table.wrapping_add(ch * 4).wrapping_add(3)) as i8 as i32 as u32;
        v1 = sum.wrapping_add(adv);
        sum = v1;
        if count == n {
            break;
        }
        ch = c.r8(scan) as u32;
        if ch == 0 {
            count += 1;
            break;
        }
    }
    c.s(8, sext(sum));
    c.s(9, sext(count));
    c.s(4, sext(adv));
    c.s(3, sext(v1));
    c.s(7, sext(scan));
    sum
}

/// 0x1ed9a8 text_width_n(str, n, table) -> width: sum of the glyph advances of the first n chars of str.
pub fn text_width_n(c: &mut Cpu) -> bool {
    let (s, n, table) = (c.arg(0), c.arg(1), c.arg(2));
    let width = width_n(c, s, n, table);
    c.ret(width);
    true
}

/// 0x1ed9f8 text_width_1c3250(str, n) -> width: text_width_n over the font table at 0x1c3250.
pub fn text_width_1c3250(c: &mut Cpu) -> bool {
    // The retail wrapper saves ra in a 16-byte frame below sp and restores it on return.
    let frame = (c.g(29) as u32).wrapping_sub(0x10);
    let ra = c.r[31];
    c.wv(frame, quad(ra));
    c.s(6, sext(FONT_TABLE));
    let (s, n) = (c.arg(0), c.arg(1));
    let width = width_n(c, s, n, FONT_TABLE);
    c.ret(width);
    true
}
