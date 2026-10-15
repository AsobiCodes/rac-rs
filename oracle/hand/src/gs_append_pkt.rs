//! Packet appenders: a GS scissor packet and two DMA ref packets, each written at the current packet pointer,
//! with the new end stored to the gp slot and returned.

use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

/// Global holding the current packet write pointer (only read here).
const PKT_PTR_VAR: u32 = 0x0016_1140;
/// gp-relative slot where the appenders store the new packet end.
const GP_PKT_END: i32 = -0x5ac0;
/// Framebuffer width and height as stored words; the clamps use size - 1.
const FB_WIDTH_VAR: u32 = 0x0013_e500;
const FB_HEIGHT_VAR: u32 = 0x0013_e504;

/// Packet length of the scissor append (3 qwords).
const SCISSOR_PKT_LEN: u32 = 0x30;
/// Packet length of a DMA ref append (2 qwords).
const REF_PKT_LEN: u32 = 0x10;
/// GS SCISSOR_1 register address and the A+D register code.
const GS_SCISSOR_1: u32 = 0x40;
const GS_REG_AD: u32 = 0xe;
/// GIF tag words and the DMA ref tag id.
const GIF_TAG_HEAD: u32 = 0x1000_0002;
const GIF_TAG_SCISSOR: u32 = 0x5000_0002;
const GIF_TAG_REF: u32 = 0x5000_0003;
const GIF_TAG_MID: u32 = 0x8001;
const GIF_TAG_NREG: u32 = 0x1000_0000;
const DMA_REF_TAG: u32 = 0x3000_0003;

/// Current packet pointer.
fn pkt_ptr(c: &mut Cpu) -> u32 {
    c.r32(PKT_PTR_VAR)
}

/// Stores the new packet end to the gp slot and returns it in v0.
fn store_end(c: &mut Cpu, end: u32) {
    let gp = c.r[28] as u32;
    c.w32(gp.wrapping_add(GP_PKT_END as u32), end);
    c.ret(end);
}

/// Sets a 64-bit register the way the retail 64-bit ops leave it (sign-extended into the upper half).
fn set64(c: &mut Cpu, n: usize, v: u64) {
    c.r[n] = v as i64 as i128 as u128;
}

/// Lower bound: a value below zero becomes zero.
fn clamp_low(v: u64) -> u64 {
    if (v as i64) < 0 { 0 } else { v }
}

/// Upper bound: a value above `max` becomes `max` (signed compare, as retail's slt/movn).
fn clamp_high(v: u64, max: u64) -> u64 {
    if (max as i64) < (v as i64) { max } else { v }
}

/// Appends the 2-qword DMA ref to `addr` plus its GIF header at the packet pointer.
fn append_dma_ref(c: &mut Cpu, addr: u32) -> bool {
    let pkt = pkt_ptr(c);
    c.w32(pkt, DMA_REF_TAG);
    c.w32(pkt + 4, addr);
    c.w32(pkt + 8, 0);
    c.w32(pkt + 0xc, GIF_TAG_REF);
    set64(c, 2, 0); // v0 is set by store_end below
    set64(c, 3, pkt as u64); // v1
    set64(c, 4, GIF_TAG_REF as u64); // a0
    set64(c, 5, pkt as u64); // a1
    store_end(c, pkt + REF_PKT_LEN);
    true
}

/// 0x291488 gs_scissor_pkt(left, right, top, bottom) -> end: append a 3-qword GIF packet that sets GS SCISSOR_1
/// to the rect clamped to the framebuffer (left/top at least 0, right/bottom at most size-1); returns the end.
pub fn gs_scissor_pkt(c: &mut Cpu) -> bool {
    let pkt = pkt_ptr(c);
    let left = c.r[4] as u64;
    let right = c.r[5] as u64;
    let top = c.r[6] as u64;
    let bottom = c.r[7] as u64;
    let max_x = (c.r32(FB_WIDTH_VAR) as i32).wrapping_sub(1) as i64 as u64;
    let max_y = (c.r32(FB_HEIGHT_VAR) as i32).wrapping_sub(1) as i64 as u64;

    let x0 = clamp_low(left);
    let x1 = clamp_high(right, max_x);
    let y0 = clamp_low(top);
    let y1 = clamp_high(bottom, max_y);
    let scissor = x0 | (x1 << 16) | (y0 << 32) | (y1 << 48);

    c.w32(pkt, GIF_TAG_HEAD);
    c.w32(pkt + 4, 0);
    c.w32(pkt + 8, 0);
    c.w32(pkt + 0xc, GIF_TAG_SCISSOR);
    c.w32(pkt + 0x10, GIF_TAG_MID);
    c.w32(pkt + 0x14, GIF_TAG_NREG);
    c.w32(pkt + 0x18, GS_REG_AD);
    c.w32(pkt + 0x1c, 0);
    c.w32(pkt + 0x20, scissor as u32);
    c.w32(pkt + 0x24, (scissor >> 32) as u32);
    c.w32(pkt + 0x28, GS_SCISSOR_1);
    c.w32(pkt + 0x2c, 0);

    // Scratch registers as retail leaves them.
    set64(c, 2, 0); // v0, set by store_end
    set64(c, 3, pkt as u64); // v1
    set64(c, 4, pkt as u64); // a0
    set64(c, 5, scissor); // a1
    set64(c, 6, GS_SCISSOR_1 as u64); // a2
    set64(c, 8, y0 << 32); // t0
    set64(c, 9, x1 << 16); // t1
    set64(c, 10, pkt as u64); // t2
    set64(c, 11, max_y); // t3
    set64(c, 12, GIF_TAG_SCISSOR as u64); // t4
    set64(c, 13, GIF_TAG_MID as u64); // t5
    set64(c, 14, GIF_TAG_NREG as u64); // t6
    set64(c, 15, GS_REG_AD as u64); // t7
    store_end(c, pkt + SCISSOR_PKT_LEN);
    true
}

/// 0x2915b0 dma_ref_1c3000() -> end: append a DMA ref to 0x1c3000 plus its GIF header; returns the new end.
pub fn dma_ref_1c3000(c: &mut Cpu) -> bool {
    append_dma_ref(c, 0x001c_3000)
}

/// 0x291670 dma_ref_1c25f0() -> end: append a DMA ref to 0x1c25f0 plus its GIF header; returns the new end.
pub fn dma_ref_1c25f0(c: &mut Cpu) -> bool {
    append_dma_ref(c, 0x001c_25f0)
}
