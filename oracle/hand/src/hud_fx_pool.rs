//! Three small retail routines of the HUD and effects code: a gated HUD slot close, a 30-entry ring sample pick, and a
//! flash-particle spawn that fills a pool record from its arguments and a float scale.

use crate::h;
use crate::ps2::*;
use crate::{hud_slot_ops, mathx, talk_pad_scan};
use ee_oracle::cpu::Cpu;

/// Retail word that must equal the argument for the gate to open (lui 0x16, -0xa8c).
const GATE_WORD: u32 = 0x0015_f574;
/// Retail word holding the slot handle that the gate closes (lui 0x16, +0x1f04).
const SLOT_WORD: u32 = 0x0016_1f04;
/// Retail word cleared before the close (lui 0x16, -0xa90).
const CLEAR_WORD: u32 = 0x0015_f570;
/// gp-relative offset (negative) of the word cleared before the close.
const GP_CLEAR_OFF: u32 = 0x768c;
/// Upper half of the retail data page used through `at` (lui 0x16).
const PAGE_16: u32 = 0x0016_0000;
/// Slot handle value meaning "no slot open".
const SLOT_CLOSED: u32 = u32::MAX;

/// 0x256890 hud_slot_gate(a0 key) -> v0: 1 if the gate word equals a0 and a slot is open (the slot is closed), else 0.
pub fn hud_slot_gate(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let frame = (c.r[29] as u32).wrapping_sub(0x20);
    // The frame saves s0 and ra before anything else, as retail does.
    c.mem.wr128(frame, c.r[16]);
    c.mem.wr128(frame + 0x10, c.r[31]);
    let key = c.arg(0);
    if c.r32(GATE_WORD) != key {
        c.ret(0);
        return true;
    }
    let slot = c.r32(SLOT_WORD);
    c.r[4] = slot as i32 as i128 as u128;
    if slot == SLOT_CLOSED {
        c.ret(0);
        return true;
    }
    c.r[1] = PAGE_16 as u128;
    let gp = c.r[28] as u32;
    c.w32(CLEAR_WORD, 0);
    c.w32(gp.wrapping_sub(GP_CLEAR_OFF), 0);
    let _ = h::run_keep(c, hud_slot_ops::hud_slot_close, &[slot], &[]);
    c.r[1] = PAGE_16 as u128;
    c.w32(SLOT_WORD, SLOT_CLOSED);
    c.ret(1);
    true
}

/// Base of the ring-sample table in retail data (lui 0x14, -0x36c0).
const RING_BASE: u32 = 0x0013_c940;
/// Word in the table: the lower bound compared against a0.
const RING_MIN: u32 = 0x190;
/// Signed halfword in the table: the upper end of the span.
const RING_HALF: u32 = 0x18e;
/// Float table, indexed by the ring position (4 bytes per entry).
const RING_FLOATS: u32 = 0x2d0;
/// Ring length: positions wrap modulo this.
const RING_LEN: i32 = 30;

/// 0x259000 ring_sample_pick(a0 value) -> v0 = 4 * position, f0 = ring float: wraps (min(a0, word) and half) into a
/// 30-slot ring and reads the float at that slot.
pub fn ring_sample_pick(c: &mut Cpu) -> bool {
    let limit = c.arg(0) as i32;
    let word = c.r32(RING_BASE + RING_MIN) as i32;
    let half = c.r16(RING_BASE + RING_HALF) as i16 as i32;
    // slt a1, word, limit; movz keeps word when word < limit, else takes limit: a signed minimum.
    let below = (word < limit) as i32;
    let lower = if below != 0 { word } else { limit };
    let span = half.wrapping_sub(lower).wrapping_add(RING_LEN);
    // div: LO = quotient, HI = remainder, both truncated toward zero as on MIPS.
    let quotient = span / RING_LEN;
    let rem = span % RING_LEN;
    let scaled = rem.wrapping_mul(4);
    let slot = RING_BASE.wrapping_add(scaled as u32);
    c.lo = c.lo & !(u64::MAX as u128) | (quotient as i64 as u64) as u128;
    c.hi = c.hi & !(u64::MAX as u128) | (rem as i64 as u64) as u128;
    c.r[5] = below as u32 as u128;
    c.r[3] = span as u32 as u128;
    c.r[6] = slot as u128;
    c.r[7] = RING_LEN as u32 as u128;
    c.r[2] = scaled as u32 as u128;
    let value = c.rf(slot.wrapping_add(RING_FLOATS));
    c.fret(value);
    true
}

/// Frame size of the spawn routine.
const SPAWN_FRAME: u32 = 0x80;
/// Pool kind requested from `particle_alloc_mode0`.
const POOL_KIND_FLASH: u32 = 5;
/// Colour/flag base word: 0x7f in the top byte (lui 0x7f00).
const FLAG_BASE: u32 = 0x7f00_0000;
/// Float 4.0, the divisor of the mode argument conversion (lui 0x4080).
const FOUR_BITS: u32 = 0x4080_0000;
/// Upper half of the page holding the scale-table pointer (lui 0x1b).
const SCALE_PAGE: u32 = 0x001b_0000;
/// Offset of the pointer to the scale byte (0x1b2194).
const SCALE_PTR: u32 = 0x2194;
/// Constant subtracted from the converted life (0x60) and the constant stored as the record tag (0x48).
const LIFE_BIAS: u32 = 0x60;
const TAG_BYTE: u8 = 0x48;

/// 0x25bcf0 particle_flash_spawn(a0 src record, a1 kind byte, a2 flag, a3 mode, t0 count, f12 end size, f13 start
/// size): when t0 is nonzero and a pool slot is free, fills the slot from the 16-byte record at a0 and the arguments.
pub fn particle_flash_spawn(c: &mut Cpu) -> bool {
    let sp = (c.r[29] as u32).wrapping_sub(SPAWN_FRAME);
    // The frame saves the callee-saved registers, ra and f20/f21 before anything else, as retail does.
    c.mem.wr128(sp + 0x50, c.r[21]);
    c.mem.wr128(sp + 0x40, c.r[20]);
    c.mem.wr128(sp + 0x30, c.r[19]);
    c.mem.wr128(sp + 0x20, c.r[18]);
    c.mem.wr128(sp, c.r[16]);
    c.mem.wr128(sp + 0x10, c.r[17]);
    c.mem.wr128(sp + 0x60, c.r[31]);
    c.w32(sp + 0x78, c.f[21]);
    c.w32(sp + 0x70, c.f[20]);

    let src = c.arg(0);
    let kind = c.arg(1) & 0xff;
    let flag = c.arg(2) & 0xff;
    let mode = c.arg(3) & 0xff;
    let count = c.arg(4);
    let end_size = c.farg(0);
    let start_size = c.farg(1);
    if count == 0 {
        return true;
    }
    let (slot, _) = h::run_keep(c, talk_pad_scan::particle_alloc_mode0, &[POOL_KIND_FLASH], &[]);
    if slot == 0 {
        return true;
    }
    let record = c.mem.rd128(src);
    c.mem.wr128(slot + 0x10, record);
    let high = (flag << 8) | FLAG_BASE;
    let packed = (mode << 0x10) | high | kind;
    c.w32(slot + 4, packed);
    c.r[1] = FOUR_BITS as u128;
    c.r[2] = packed as u128;
    c.r[3] = high as u128;
    c.r[4] = FLAG_BASE as u128;
    let (rounded, _) = h::run_keep(c, mathx::float_to_int, &[], &[F(FOUR_BITS)]);
    // retail's cvt.w.s converts f12 in place: f12 keeps the integer bits
    c.f[12] = rounded;
    let life = rounded.wrapping_sub(LIFE_BIAS);
    c.w8(slot + 9, life as u8);
    c.w8(slot + 3, TAG_BYTE);
    c.r[4] = count as u128;
    c.w32(slot + 0xc, start_size.0);
    c.w8(slot + 1, 0);
    c.w8(slot + 8, 0);
    c.r[5] = SCALE_PAGE as u128;
    let scale_ptr = c.r32(SCALE_PAGE + SCALE_PTR);
    let scale_byte = c.r8(scale_ptr);
    c.w32(slot + 0x20, count);
    c.w8(slot + 2, scale_byte);
    c.r[2] = scale_ptr as u128;
    c.r[3] = scale_byte as u128;
    let (_, scale) = h::run_keep(c, mathx::int_to_float, &[count], &[]);
    let ratio = end_size / scale;
    c.w32(slot + 0x24, ratio.0);
    c.fret(ratio);
    true
}
