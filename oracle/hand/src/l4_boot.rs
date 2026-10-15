//! L4 boot-step helpers (retail 0x1ec228..0x1ed9a8): the frame tick, the packed word submit, and two gp flag writes.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Sign-extended 32-bit register write (what lw/addiu/lui leave in a 64-bit GPR).
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Frame counter word, incremented each tick (lui 0x16; lw -0xc08).
const FRAME_COUNTER: u32 = 0x0015_f3f8;
/// Gp-relative word that receives the incremented frame counter (gp-0x7808).
const GP_FRAME_STORE: u32 = 0x7808;
/// Gp-relative flag word set by the two flag helpers (gp-0x77a4).
const GP_FLAG: u32 = 0x77a4;
/// Word read as the first argument of the frame-tick submit (lui 0x16; lw -0x117c).
const SUBMIT_ARG_WORD: u32 = 0x0015_ee84;
/// Word read as the third argument of the frame-tick submit (lui 0x14; lw -0x1afc), shifted left 11.
const SUBMIT_SHIFT_WORD: u32 = 0x0013_e504;
/// Pointer to the packet write cursor (lui 0x16; lw 0x1140).
const PACKET_CURSOR: u32 = 0x0016_1140;

/// 0x1ec228 frame_tick(a0) -> void: advances the frame counter, then runs the per-frame submit.
pub fn frame_tick(c: &mut Cpu) -> bool {
    let a0 = c.arg(0) as u32;
    let gp = c.g(28) as u32;
    h::frame(c, 0x20, &[(16, 0x0, h::SQ), (31, 0x10, h::SQ)], |c, _sp| {
        h::call_keep(c, 0x0029_11f8, &[1], &[]);
        h::call_keep(c, 0x0012_2298, &[0], &[]);
        let v1 = c.r32(FRAME_COUNTER).wrapping_add(1);
        c.w32(gp.wrapping_sub(GP_FRAME_STORE), v1);
        h::call_keep(c, 0x0029_1018, &[], &[]);
        let a1 = c.r32(SUBMIT_ARG_WORD);
        let a2 = c.r32(SUBMIT_SHIFT_WORD) << 11;
        h::call_keep(c, 0x0029_1738, &[a0, a1, a2], &[]);
    });
    true
}

/// 0x1ec8b8 pack_submit(a0, a1, a2, a3) -> void: packs four byte lanes into one word, submits it, and appends
/// the two-quadword packet to the write cursor.
pub fn pack_submit(c: &mut Cpu) -> bool {
    let packed = c.g(4) | (c.g(5) << 8) | (c.g(6) << 16) | (c.g(7) << 24);
    c.s(5, packed);
    h::frame(c, 0x10, &[(31, 0x0, h::SQ)], |c, _sp| {
        h::call_keep(c, 0x0029_13c8, &[1], &[]);
    });
    let tag_lo: u32 = 0x3000_0014;
    let tag_hi: u32 = 0x5000_0014;
    let cursor = c.r32(PACKET_CURSOR);
    c.w32(cursor, tag_lo);
    let v1: u32 = 0x0013_cc90;
    let cursor = c.r32(PACKET_CURSOR);
    c.w32(cursor.wrapping_add(4), v1);
    let cursor = c.r32(PACKET_CURSOR);
    c.w32(cursor.wrapping_add(8), 0);
    let cursor = c.r32(PACKET_CURSOR);
    c.w32(cursor.wrapping_add(0xc), tag_hi);
    let next = c.r32(PACKET_CURSOR).wrapping_add(0x10);
    c.w32(PACKET_CURSOR, next);
    set32(c, 2, next);
    set32(c, 3, next.wrapping_sub(0x10));
    set32(c, 4, tag_hi);
    set32(c, 5, next.wrapping_sub(0x10));
    set32(c, 1, 0x0016_0000);
    true
}

/// 0x1ed990 set_flag_one() -> 1: sets the gp flag word to 1 and returns 1.
pub fn set_flag_one(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    set32(c, 2, 1);
    c.w32(gp.wrapping_sub(GP_FLAG), 1);
    true
}

/// 0x1ed9a0 clear_flag() -> void: clears the gp flag word.
pub fn clear_flag(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    c.w32(gp.wrapping_sub(GP_FLAG), 0);
    true
}
