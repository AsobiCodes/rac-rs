//! GS packet cursor and register setup for the level-4 overlay (rec/l4_01): packet writer, GS privileged register
//! init, and the per-frame gate that may submit a packet.
use crate::ps2::*;
use ee_oracle::cpu::{w4, Cpu};

/// Global write cursor for the current GS packet (lui 0x16 + 0x1140).
const PACKET_CURSOR: u32 = 0x0016_1140;
/// Packed-value block read by the packet writer (a3 = 0x16cb40; fields at +0x230, +0x234, +0x238).
const PACKED_BLOCK: u32 = 0x0016_cb40;
/// Retail routine that submits the finished packet.
const SUBMIT_PACKET: u32 = 0x0029_13c8;
/// Gp-relative slot that receives the packet end pointer (gp - 0x5ac0).
const GP_PACKET_END: i32 = -0x5ac0;
/// Gp-relative word written by the gate (gp - 0x780c).
const GP_GATE_MODE: i32 = -0x780c;
/// Word read by the gate: non-zero means "already done" (0x16 + -0xa48).
const GATE_DONE: u32 = 0x0015_f5b8;
/// Byte at 0x141401 that disables the gate when set.
const GATE_DISABLE: u32 = 0x0014_1401;
/// Word that holds the current mode; 3 selects the second setting (0x16 + -0xa5c).
const GATE_MODE_WORD: u32 = 0x0015_f5a4;
/// Word written with 0x7f when the mode is 3 (0x16 + -0xc0c).
const GATE_RESULT_WORD: u32 = 0x0015_f3f4;
/// Retail routine the gate calls at the end.
const GATE_CALLEE: u32 = 0x001e_ae20;
/// Return address of the packet submit call (inside the packet writer).
const SUBMIT_RETURN: u32 = 0x001e_ad98;

/// Reads a word and sign-extends it the way LW leaves it in a 64-bit register.
fn lw(c: &mut Cpu, a: u32) -> u64 {
    c.r32(a) as i32 as i64 as u64
}

/// Reads a doubleword (LD) from two words.
fn ld(c: &mut Cpu, a: u32) -> u64 {
    c.r32(a) as u64 | (c.r32(a + 4) as u64) << 32
}

/// Writes a doubleword (SD) as two words.
fn sd(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a + 4, (v >> 32) as u32);
}

/// Gp-relative address.
fn gp_at(c: &Cpu, off: i32) -> u32 {
    (c.g(28) as u32).wrapping_add(off as u32)
}

/// Saves ra into the 16-byte frame below sp (sq $ra, 0($sp) after addiu sp, -0x10); returns the old sp.
fn push_ra(c: &mut Cpu) -> u32 {
    let sp = c.g(29) as u32;
    c.wv(sp.wrapping_sub(0x10), V(w4(c.r[31])));
    sp
}

/// Calls retail with the frame pushed (sp = frame, ra = return address), then restores sp and ra.
fn call_in_frame(c: &mut Cpu, sp: u32, ra: u32, addr: u32) {
    let saved_ra = c.r[31];
    c.s(29, sp.wrapping_sub(0x10) as i32 as i64 as u64);
    c.s(31, ra as u64);
    c.call(addr);
    c.s(29, sp as i32 as i64 as u64);
    c.r[31] = saved_ra;
}

/// 0x1eacb8 gs_packet_submit(): writes the GIF/DMA header quadwords at the packet cursor (tags 0x30000013,
/// 0x50000013, 0x3000000b, 0x5000000b and the packed 24-bit values), advances the cursor by 0x10, records the
/// new end in the gp slot, then submits the packet through retail 0x2913c8.
pub fn gs_packet_submit(c: &mut Cpu) -> bool {
    const TAG_A: u32 = 0x3000_0013;
    const TAG_B: u32 = 0x5000_0013;
    const TAG_C: u32 = 0x3000_000b;
    const TAG_D: u32 = 0x5000_000b;
    const LEN_A: u32 = 0x0014_0000 - 0x3040;
    const LEN_B: u32 = 0x0014_0000 - 0x30f0;
    const SUBMIT_TYPE: u64 = 0x3d;

    let sp = push_ra(c);
    let p = c.r32(PACKET_CURSOR);
    c.w32(p, TAG_A);
    c.w32(p + 4, LEN_A);
    c.w32(p + 8, 0);
    c.w32(p + 0xc, TAG_B);
    c.w32(PACKET_CURSOR, p + 0x10);
    c.w32(p + 0x10, TAG_C);
    c.w32(p + 0x14, LEN_B);
    c.w32(p + 0x18, 0);
    c.w32(p + 0x1c, TAG_D);

    let x230 = lw(c, PACKED_BLOCK + 0x230);
    let v1 = lw(c, PACKED_BLOCK + 0x234) << 8;
    let a2 = lw(c, PACKED_BLOCK + 0x238) << 16;
    let a1 = x230 | v1 | a2;
    let end = p + 0x20;
    c.w32(gp_at(c, GP_PACKET_END), end);

    c.s(1, 0x0016_0000);
    c.s(2, end as u64);
    c.s(3, v1);
    c.s(4, SUBMIT_TYPE);
    c.s(5, a1);
    c.s(6, a2);
    c.s(7, PACKED_BLOCK as u64);
    c.s(8, TAG_D as u64);
    c.s(9, TAG_C as u64);
    call_in_frame(c, sp, SUBMIT_RETURN, SUBMIT_PACKET);
    true
}

/// 0x1eada8 gs_init_regs(): zeroes the GS privileged registers at 0x12000000 block (0xe0, 0x00 = 0xffa1 mode) and
/// copies the 3-doubleword table at 0x151788 into the registers at 0x12000020, 0x70/0x90, 0x80/0xa0, then clears
/// 0xd0. Leaves the scratch registers as retail does.
pub fn gs_init_regs(c: &mut Cpu) -> bool {
    const GS_BASE: u32 = 0x1200_0000;
    const TABLE: u32 = 0x0015_1788;
    sd(c, GS_BASE + 0xe0, 0);
    sd(c, GS_BASE, 0xffa1);
    let t0_src = ld(c, TABLE);
    sd(c, GS_BASE + 0x20, t0_src);
    let t_mid = ld(c, TABLE + 8);
    sd(c, GS_BASE + 0x70, t_mid);
    sd(c, GS_BASE + 0x90, t_mid);
    let t_last = ld(c, TABLE + 0x10);
    sd(c, GS_BASE + 0x80, t_last);
    sd(c, GS_BASE + 0xa0, t_last);
    sd(c, GS_BASE + 0xd0, 0);

    c.s(2, (GS_BASE + 0xd0) as u64);
    c.s(3, t_last);
    c.s(4, (GS_BASE + 0xa0) as u64);
    c.s(5, (GS_BASE + 0x70) as u64);
    c.s(6, (GS_BASE + 0x80) as u64);
    c.s(7, TABLE as u64);
    c.s(8, t_mid);
    true
}

/// 0x1eb698 gs_frame_gate(): does nothing once the done word is set or the disable byte is non-zero. Otherwise
/// it sets the gate mode word, and when the current mode is 3 it stores 0x7f in the result word, then runs retail
/// 0x1eae20.
pub fn gs_frame_gate(c: &mut Cpu) -> bool {
    const MODE_WORD_VALUE: u32 = 0x0001_00ff;
    const SECOND_MODE: u64 = 3;
    const RESULT: u32 = 0x7f;

    let sp = push_ra(c);
    let done = lw(c, GATE_DONE);
    if done != 0 {
        c.s(2, done);
        return true;
    }
    c.s(2, 0x0014_0000);
    let disabled = c.r8(GATE_DISABLE) as u64;
    if disabled != 0 {
        c.s(3, disabled);
        return true;
    }
    c.s(2, 0x0001_0000 | 0xff);
    c.s(3, SECOND_MODE);
    let mode = lw(c, GATE_MODE_WORD);
    c.s(4, mode);
    c.w32(gp_at(c, GP_GATE_MODE), MODE_WORD_VALUE);
    if mode as u32 == SECOND_MODE as u32 {
        c.w32(GATE_RESULT_WORD, RESULT);
        c.s(1, 0x0016_0000);
        c.s(2, RESULT as u64);
    }
    call_in_frame(c, sp, 0x001e_b6ec, GATE_CALLEE);
    true
}
