//! VIF/GIF packet emitters that append to the DMA packet cursor at 0x161140 (retail 0x2912d0, 0x291380, 0x2913c8).
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;
const GP: usize = 28;
const SP: usize = 29;
const S0: usize = 16;
const RA: usize = 31;

/// Packet cursor cell: the address of the next free packet byte.
const PACKET_CURSOR: u32 = 0x0016_1140;
/// gp-relative cell that receives the packet end after each emit (retail `sw v0, -0x5ac0($gp)`).
const GP_END_CELL: i32 = -0x5ac0;
/// Stack frame size of emit_unpack_packet.
const FRAME: u32 = 0x20;
/// DMA CNT tag word with qwc 0 (high nibble 0x1 = CNT).
const DMA_CNT_TAG: u32 = 0x1000_0000;
/// VIF UNPACK V4-32 command in the top byte.
const VIF_UNPACK_V4_32: u32 = 0x6c00_0000;
/// VIF word that sets the unpack destination/mode for the unpack packet (retail 0x01000404).
const VIF_UNPACK_MODE: u32 = 0x0100_0404;
/// GIF tag header for a single-loop A+D packet (retail 0x10000002).
const GIF_TAG_HEAD: u32 = 0x1000_0002;
/// GIF tag second word: A+D register mode, two registers (retail 0x50000002).
const GIF_TAG_REGS_AD: u32 = 0x5000_0002;
/// GIF tag flags word (retail 0x8001).
const GIF_TAG_FLAGS: u32 = 0x8001;
/// GIF register count for the tag (retail 0xe).
const GIF_REG_COUNT: u32 = 0xe;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// A 128-bit register as a quadword (lane 0 is the low 32 bits).
fn quad(r: u128) -> V {
    V([r as u32, (r >> 32) as u32, (r >> 64) as u32, (r >> 96) as u32])
}

fn packet_cursor(c: &mut Cpu) -> u32 {
    c.r32(PACKET_CURSOR)
}

/// Stores the new packet end into the gp-relative cell.
fn set_gp_end(c: &mut Cpu, end: u32) {
    let gp = c.g(GP) as u32;
    c.w32(gp.wrapping_add(GP_END_CELL as u32), end);
}

/// 0x2912d0 emit_unpack_packet(word a0, src a1, qwords a2) -> v0: appends a VIF UNPACK V4-32 header
/// (word a0 | qwords << 16), copies the qwords from src into the packet, advances the cursor by qwords * 16.
pub fn emit_unpack_packet(c: &mut Cpu) -> bool {
    let word = c.arg(0);
    let src = c.arg(1);
    let qwords = c.arg(2);
    let bytes = qwords << 4;
    let frame = (c.g(SP) as u32).wrapping_sub(FRAME);
    // The retail prologue spills s0 and ra to the new frame; the epilogue reloads them unchanged.
    c.wv(frame, quad(c.r[S0]));
    c.wv(frame.wrapping_add(0x10), quad(c.r[RA]));

    let p = packet_cursor(c);
    c.w32(p, DMA_CNT_TAG | qwords);
    c.w32(p + 4, 0);
    c.w32(p + 8, VIF_UNPACK_MODE);
    c.w32(p + 0xc, word | (qwords << 16) | VIF_UNPACK_V4_32);
    let dst = p + 0x10;
    set_gp_end(c, dst);

    // Register state at the jal: a0 = dst, a2 = bytes, v0 = dst, v1 = cursor, a3 = unpack mode.
    set_gpr(c, V0, dst);
    set_gpr(c, V1, p);
    set_gpr(c, A3, VIF_UNPACK_MODE);
    h::run_keep(c, mathx::copy_qwords, &[dst, src, bytes], &[]);

    let end = packet_cursor(c).wrapping_add(bytes);
    c.w32(PACKET_CURSOR, end);
    set_gpr(c, V0, end);
    true
}

/// 0x291380 append_dma_tag(word a0) -> v0: appends a 16-byte DMA CNT tag (qwc 0) followed by the VIF word a0.
pub fn append_dma_tag(c: &mut Cpu) -> bool {
    let word = c.arg(0);
    let p = packet_cursor(c);
    c.w32(p, DMA_CNT_TAG);
    c.w32(p + 4, 0);
    c.w32(p + 8, 0);
    c.w32(p + 0xc, word);
    let end = p + 0x10;
    set_gp_end(c, end);
    set_gpr(c, A1, p);
    set_gpr(c, V1, p);
    set_gpr(c, V0, end);
    true
}

/// 0x2913c8 gs_set_reg(reg a0, val a1 as u64) -> v0: emits one A+D GIF packet that writes `val` to GS register `reg`,
/// then advances the packet cursor by 0x30.
pub fn gs_set_reg(c: &mut Cpu) -> bool {
    let reg = c.arg(0);
    let val = c.g(A1);
    let p = packet_cursor(c);
    c.w32(p, GIF_TAG_HEAD);
    c.w32(p + 4, 0);
    c.w32(p + 8, 0);
    c.w32(p + 0xc, GIF_TAG_REGS_AD);
    c.w32(p + 0x10, GIF_TAG_FLAGS);
    c.w32(p + 0x14, DMA_CNT_TAG);
    c.w32(p + 0x18, GIF_REG_COUNT);
    c.w32(p + 0x1c, 0);
    c.w32(p + 0x20, val as u32);
    c.w32(p + 0x24, (val >> 32) as u32);
    c.w32(p + 0x28, reg);
    c.w32(p + 0x2c, 0);
    let end = p + 0x30;
    set_gp_end(c, end);
    set_gpr(c, A0, p);
    set_gpr(c, V1, p);
    set_gpr(c, V0, end);
    set_gpr(c, A2, GIF_TAG_REGS_AD);
    set_gpr(c, A3, GIF_TAG_FLAGS);
    set_gpr(c, T0, DMA_CNT_TAG);
    set_gpr(c, T1, GIF_REG_COUNT);
    true
}
