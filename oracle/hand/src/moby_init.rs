//! Moby block setup and the busy-flag wait helper that sits next to it (retail 0x240120..0x2410e8).

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

// MIPS GPR numbers.
const GPR_AT: usize = 1;
const GPR_V0: usize = 2;
const GPR_V1: usize = 3;
const GPR_A0: usize = 4;
const GPR_A1: usize = 5;
const GPR_A2: usize = 6;
const GPR_A3: usize = 7;
const GPR_T0: usize = 8;
const GPR_T1: usize = 9;
const GPR_T2: usize = 10;
const GPR_T3: usize = 11;
const GPR_T6: usize = 14;
const GPR_S0: usize = 16;
const GPR_S1: usize = 17;
const GPR_SP: usize = 29;
const GPR_RA: usize = 31;

/// Busy bit in a block's first word; the wait helper spins while it is set.
const BLOCK_BUSY: u32 = 0x100;
/// IEEE single 1.0.
const ONE_BITS: u32 = 0x3f80_0000;
/// Byte table indexed by the caller's index argument (a1); the byte read from it selects the table rows below.
const INDEX_BYTE_TABLE: u32 = 0x0019_7cc0;
/// Word table, 4-byte rows indexed by the byte at block+0x22 (the anim-entry pointer row).
const ENTRY_TABLE: u32 = 0x0019_7780;
/// Word table, 4-byte rows indexed by the same byte (the anim-entry table row).
const ANIM_TABLE: u32 = 0x0019_7400;
/// Word holding the base the block offset is measured from (retail: lw at 0x160000 - 0xa8).
const BASE_SLOT: u32 = 0x0015_ff58;
/// Value of t0 after retail's dsll chain: 0x80808000 << 23.
const T0_VALUE: u64 = 0x8080_8000u64 << 23;

/// Writes a 128-bit register to memory the way SQ does: low dword, high dword, then sign-extension words.
fn save_quad(c: &mut Cpu, addr: u32, reg: usize) {
    let v = c.g(reg);
    let hi = if (v as i64) < 0 { u32::MAX } else { 0 };
    c.w32(addr, v as u32);
    c.w32(addr + 4, (v >> 32) as u32);
    c.w32(addr + 8, hi);
    c.w32(addr + 12, hi);
}

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// Sets the registers the tail of moby_init leaves behind.
fn set_tail_regs(c: &mut Cpu, v0: u32, v1: u32, a0: u32, a1: u32) {
    set_gpr(c, GPR_V0, v0);
    set_gpr(c, GPR_V1, v1);
    set_gpr(c, GPR_A0, a0);
    set_gpr(c, GPR_A1, a1);
    set_gpr(c, GPR_A2, 0xff);
    set_gpr(c, GPR_A3, 0x80);
    c.s(GPR_T0, T0_VALUE);
    set_gpr(c, GPR_T1, 0x7f80);
    set_gpr(c, GPR_T2, 0x7f);
}

/// Sets the three registers the post-call tail of moby_init rewrites.
fn set_vr(c: &mut Cpu, v0: u32, v1: u32, a0: u32) {
    set_gpr(c, GPR_V0, v0);
    set_gpr(c, GPR_V1, v1);
    set_gpr(c, GPR_A0, a0);
}

/// 0x240120 kick_block(blk = v1, t3, t6): stores the control words of a block and toggles t6's bit 0x2000.
pub fn kick_block(c: &mut Cpu) -> bool {
    let blk = c.g(GPR_V1) as u32;
    let t6 = c.g(GPR_T6);
    let t3 = c.g(GPR_T3) as u32;
    c.w32(blk + 0x80, t6 as u32);
    c.w32(blk + 0x20, 0x200);
    c.w32(blk + 0x10, t3);
    c.w32(blk, 0x100);
    c.s(GPR_T6, t6 ^ 0x2000);
    set_gpr(c, GPR_A2, 0x100);
    true
}

/// 0x240140 wait_block_idle(blk = v1): spins until the block's busy bit clears; leaves a2 = 0.
pub fn wait_block_idle(c: &mut Cpu) -> bool {
    let blk = c.g(GPR_V1) as u32;
    while c.r32(blk) & BLOCK_BUSY != 0 {}
    set_gpr(c, GPR_A2, 0);
    true
}

/// 0x240eb0 moby_init(m = a0, idx = a1): clears the 256-byte moby block, fills its timing, pose and flag
/// fields from the tables selected by the index byte, then copies in the anim entry's fields when one exists.
pub fn moby_init(c: &mut Cpu) -> bool {
    let m = c.arg(0);
    let idx = c.arg(1);
    let frame = (c.g(GPR_SP) as u32).wrapping_sub(0x30);
    save_quad(c, frame, GPR_S0);
    save_quad(c, frame + 0x10, GPR_S1);
    save_quad(c, frame + 0x20, GPR_RA);
    h::run(c, crate::mathx::fill_words, &[m, 0, 0x100], &[]);

    let b = c.r8(idx.wrapping_add(INDEX_BYTE_TABLE)) as u32;
    c.w32(m + 0x38, T0_VALUE as u32);
    c.w32(m + 0x3c, (T0_VALUE >> 32) as u32);
    c.w8(m + 0x23, 0x80);
    c.w8(m + 0x22, b as u8);
    c.w8(m + 0xa4, 0xff);
    c.w8(m + 0x21, 0xff);
    c.w8(m + 0x71, 0xff);
    c.w8(m + 0x72, 0xff);
    c.w16(m + 0xa6, idx as u16);
    c.w16(m + 0x36, 0x7f80);

    let base = c.r32(BASE_SLOT);
    let delta = (m.wrapping_sub(base) as i32) >> 8;
    c.w32(m + 0xac, delta as u32);
    c.w32(m + 0xa8, (delta << 16) as u32);

    c.w8(m + 0x7d, 0xff);
    c.w8(m + 0xa1, 0x7f);
    c.w8(m + 0xa3, 0x80);
    c.w8(m + 0x7e, 0);
    c.w8(m + 0x7c, 0xff);
    c.w8(m + 0xa0, 0x7f);
    c.w8(m + 0xa2, 0x80);

    // Row of the entry table selected by b; a zero row marks the block as having no entry.
    let entry = ENTRY_TABLE + (b & 0xff) * 4;
    let entry_word = c.r32(entry);
    c.w32(m + 0x74, entry_word);
    if entry_word == 0 {
        let flags = c.r16(m + 0x34) as u32 | 2;
        c.w16(m + 0x34, flags as u16);
    }

    let v1 = ANIM_TABLE + (b << 2);
    let a1 = c.r32(v1);
    if a1 == 0 {
        let flags = c.r16(m + 0x34) as u32 | 5;
        c.w32(m + 0x94, 0);
        c.w32(m + 0x24, 0);
        c.w16(m + 0x34, flags as u16);
        set_tail_regs(c, flags, v1, entry, 0);
        return true;
    }

    c.w32(m + 0x24, a1);
    let mut a0 = c.r16(m + 0x34) as u32;
    let row_e = c.r8(a1 + 0xe) as u32;
    c.w8(m + 0x72, row_e as u8);
    a0 |= c.r16(a1 + 0x44) as u32;
    c.w16(m + 0x34, a0 as u16);
    let x10 = c.r32(a1 + 0x10);
    c.w32(m + 0x94, x10);
    c.w32(m + 0x5c, ONE_BITS);
    let f24 = c.r32(a1 + 0x24);
    c.w32(m + 0x2c, f24);
    c.f[0] = f24;
    c.w32(m + 0x58, ONE_BITS);
    c.f[1] = ONE_BITS;

    let x40 = c.r32(a1 + 0x40);
    if x40 != 0 {
        c.w16(m + 0x34, (a0 | 0x10) as u16);
        c.w32(m + 0x90, x40);
    }
    set_gpr(c, GPR_AT, ONE_BITS);

    if c.r8(a1 + 0xf) != 0 {
        let flags = c.r16(m + 0x34) as u32 | 0x400;
        c.w8(m + 0x7f, 0x18);
        c.w32(m + 0x84, 0);
        c.w16(m + 0x34, flags as u16);
        c.w32(m + 0x88, 0);
        c.w8(m + 0xbd, 0);
    }
    if c.r8(a1 + 6) != 0 {
        c.w8(m + 0x73, 0x18);
    }

    let anim = c.r32(a1 + 0x48);
    if anim == 0 {
        set_tail_regs(c, a1, 0, a0, a1);
        return true;
    }

    // The callee's register effects (a1..a3, t0..t2, $at) reach the caller unchanged; only v0, v1, a0 are rewritten.
    h::run(c, crate::actor::anim_load_tracks, &[m], &[]);
    let s = c.r32(m + 0x24);
    let anim2 = c.r32(s + 0x48);
    if c.r8(anim2 + 0x10) >= 2 {
        let flags = c.r16(m + 0x34) & 0xfffd;
        c.w16(m + 0x34, flags as u16);
    }
    let s2 = c.r32(m + 0x24);
    let kind = c.r8(s2 + 0xc) as u32;
    if kind != 1 {
        set_vr(c, kind, 1, s2);
        return true;
    }
    let anim3 = c.r32(s2 + 0x48);
    if c.r8(anim3 + 0x10) >= 2 {
        set_vr(c, 0, anim3, s2);
        return true;
    }
    c.w32(m + 0x58, 0);
    let sign = c.r8(anim3 + 0x11) as i8 as i32;
    let mut v0 = anim3;
    if sign < 0 {
        v0 = c.r16(m + 0x34) as u32 | 0x40;
        c.w16(m + 0x34, v0 as u16);
    }
    set_vr(c, v0, sign as u32, s2);
    true
}
