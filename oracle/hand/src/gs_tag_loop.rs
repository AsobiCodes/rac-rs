//! Retail 0x1ec0a0: the GS tag-block emit loop (see `tag_loop`).
use crate::dma_setup;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Packet cursor pointer (the cursor the tag emitters advance).
const CURSOR_PTR: u32 = 0x0016_1140;
/// Gp-relative slot the advanced cursor is written back to.
const CURSOR_SLOT_GP: i32 = -0x5ac0;
/// Frame counter word (lui 0x16 with offset -0xc08) and its gp-relative mirror.
const COUNTER_ADDR: u32 = 0x0015_f3f8;
const COUNTER_SLOT_GP: i32 = -0x7808;
/// Packet words: the two tag headers and the GS address word.
const TAG_LO: u32 = 0x3000_0014;
const TAG_HI: u32 = 0x5000_0014;
const GS_WORD: u32 = 0x0013_cdd0;
/// Loop bound (0x80) and the GS register number passed to gs_reg_write.
const BOUND: u32 = 0x80;
const GS_REG: u32 = 1;
/// Bytes of packet written per iteration.
const PACKET_STEP: u32 = 0x10;
/// Frame size reserved below sp.
const FRAME_SIZE: u32 = 0x60;

fn gp(c: &Cpu) -> u32 {
    c.g(28) as u32
}

/// Stores the 128-bit register `reg` at `addr` (retail `sq`).
fn store_q(c: &mut Cpu, addr: u32, reg: usize) {
    let v = c.r[reg];
    c.wv(addr, V([v as u32, (v >> 32) as u32, (v >> 64) as u32, (v >> 96) as u32]));
}

/// Bumps the frame counter word and mirrors it to its gp slot (retail `lw`/`addiu`/`sw` before a call).
fn bump_counter(c: &mut Cpu) {
    let v = c.r32(COUNTER_ADDR).wrapping_add(1);
    let slot = gp(c).wrapping_add(COUNTER_SLOT_GP as u32);
    c.w32(slot, v);
}

/// Retail `jal` with a0 set to `a0`; the callee's scratch registers stay as it left them.
fn call_a0(c: &mut Cpu, addr: u32, a0: u32) {
    h::call_keep(c, addr, &[a0], &[]);
}

/// Retail `jal` with the arguments already in place (a0 is whatever the previous call left).
fn call_as_is(c: &mut Cpu, addr: u32) {
    h::call_keep(c, addr, &[], &[]);
}

/// 0x1ec0a0 tag_loop(n): emits n GS tag blocks, each followed by a GS register write (gs_reg_write, reg 1) whose
/// value is (0x80 - (n_i << 7) / (n_i + 1)) << 24, bumping the frame counter around the calls. Returns nothing.
pub fn tag_loop(c: &mut Cpu) -> bool {
    let n = c.arg(0);
    let sp = c.g(29) as u32;
    let frame = sp.wrapping_sub(FRAME_SIZE);
    c.s(29, frame as i32 as i64 as u64);
    store_q(c, frame, 16);
    store_q(c, frame + 0x10, 17);
    store_q(c, frame + 0x20, 18);
    store_q(c, frame + 0x30, 19);
    store_q(c, frame + 0x40, 20);
    store_q(c, frame + 0x50, 31);

    call_a0(c, 0x2911f8, 1);
    call_a0(c, 0x122298, 0);
    bump_counter(c);
    call_as_is(c, 0x291018);

    let mut s0 = (n as i32).wrapping_sub(1);
    while s0 >= 0 {
        h::run_keep(c, dma_setup::tag_emit_or_flush, &[], &[]);
        call_as_is(c, 0x1f40e8);
        h::call_keep(c, 0x1ec8b8, &[0, 0, 0, BOUND], &[]);
        h::run_keep(c, dma_setup::tag_emit, &[], &[]);

        let q = s0.wrapping_shl(7) / s0.wrapping_add(1);
        let value = (BOUND as i32).wrapping_sub(q) as i64;
        s0 = s0.wrapping_sub(1);
        c.s(5, value.wrapping_shl(24) as u64);
        h::call_keep(c, 0x2913c8, &[GS_REG], &[]);

        let p = c.r32(CURSOR_PTR);
        c.w32(p, TAG_LO);
        c.w32(p.wrapping_add(4), GS_WORD);
        c.w32(p.wrapping_add(8), 0);
        c.w32(p.wrapping_add(0xc), TAG_HI);
        let slot = gp(c).wrapping_add(CURSOR_SLOT_GP as u32);
        c.w32(slot, p.wrapping_add(PACKET_STEP));

        call_a0(c, 0x2911f8, 1);
        call_a0(c, 0x122298, 0);
        bump_counter(c);
        call_as_is(c, 0x2910e8);
        call_as_is(c, 0x291078);
    }

    call_a0(c, 0x2911f8, 1);
    call_a0(c, 0x122298, 0);
    bump_counter(c);
    call_as_is(c, 0x291018);
    h::run_keep(c, dma_setup::tag_emit_or_flush, &[], &[]);
    call_as_is(c, 0x1f40e8);

    c.s(29, sp as i32 as i64 as u64);
    true
}
