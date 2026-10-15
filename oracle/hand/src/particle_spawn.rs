//! Particle record writer: allocates a record, fills it from two source vectors and a rate.
use crate::h;
use ee_oracle::cpu::Cpu;
use crate::mathx;
use crate::ps2::*;

/// Retail allocator (a0 = 8 bytes requested), returns the record pointer in v0.
const ALLOC_ADDR: u32 = 0x0025_9a38;
/// Pointer slot in the EE data area (lui 0x1b, lw 0x21a0): points at the kind byte.
const KIND_PTR_SLOT: u32 = 0x001b_21a0;
/// 4.0f, the argument to float_to_int that yields the record's packed offset.
const FOUR_F: F = F(0x4080_0000);
/// Offset subtracted from the converted value before it is stored as a byte.
const OFFSET_BIAS: u32 = 0x60;
/// Byte stored at record+3.
const RECORD_FLAG: u8 = 0x48;
/// Stack frame size: the function starts with addiu $sp, $sp, -0x80.
const FRAME_SIZE: u32 = 0x80;

// Record layout (byte offsets from the allocated pointer).
const REC_KIND: u32 = 0x02;
const REC_FLAG: u32 = 0x03;
const REC_COUNT: u32 = 0x0a;
const REC_TAG_BYTE: u32 = 0x09;
const REC_TAG: u32 = 0x04;
const REC_POS: u32 = 0x10;
const REC_VEL: u32 = 0x20;
const REC_EXTRA: u32 = 0x34;
const REC_TAG_WORD: u32 = 0x30;
const REC_COUNT_WORD: u32 = 0x38;
const REC_RATE: u32 = 0x3c;

/// 0x25c648 particle_spawn(pos, vel, tag, extra, count, scale) -> none: allocates a record, copies the two
/// 16-byte source vectors into it, stores tag/extra/count and rate = scale / (float)count; does nothing if count is 0.
pub fn particle_spawn(c: &mut Cpu) -> bool {
    let pos_src = c.arg(0) & !0xf;
    let vel_src = c.arg(1) & !0xf;
    let tag = c.arg(2);
    let extra = c.arg(3);
    let count = c.arg(4);
    let scale = c.farg(0);

    // Prologue: the retail frame spills s0-s5, ra and the old f20 to the stack, and those slots stay in RAM.
    let sp = (c.r[29] as u32).wrapping_sub(FRAME_SIZE);
    let old_f20 = c.f[20];
    c.mem.wr128(sp + 0x60, c.r[31]);
    c.mem.wr128(sp + 0x50, c.r[21]);
    c.mem.wr128(sp + 0x40, c.r[20]);
    c.mem.wr128(sp + 0x30, c.r[19]);
    c.mem.wr128(sp + 0x20, c.r[18]);
    c.mem.wr128(sp + 0x10, c.r[17]);
    c.mem.wr128(sp, c.r[16]);
    c.wf(sp + 0x70, F(old_f20));

    if count == 0 {
        return true;
    }
    let (rec, _) = h::call_keep(c, ALLOC_ADDR, &[8], &[]);
    if rec == 0 {
        return true;
    }

    let pos = c.mem.rd128(pos_src);
    c.mem.wr128(rec + REC_POS, pos);
    c.w32(rec + REC_TAG, tag);

    // at = 4.0f; f12 = 4.0f; packed offset = float_to_int(4.0) - 0x60.
    c.r[1] = FOUR_F.0 as u128;
    let (packed, _) = h::run_keep(c, mathx::float_to_int, &[], &[FOUR_F]);
    // Retail cvt.w.s leaves the integer result in f12 as well.
    c.f[12] = packed;
    c.w8(rec + REC_TAG_BYTE, packed.wrapping_sub(OFFSET_BIAS) as u8);
    c.w8(rec + REC_FLAG, RECORD_FLAG);
    c.w8(rec + 1, 0);
    c.w32(rec + 0x0c, 0);
    c.w8(rec + 0x08, 0);
    let kind_ptr = c.r32(KIND_PTR_SLOT);
    let kind = c.r8(kind_ptr);
    c.w8(rec + REC_KIND, kind);

    let vel = c.mem.rd128(vel_src);
    c.mem.wr128(rec + REC_VEL, vel);
    c.w16(rec + REC_COUNT, count as u16);

    // v0 holds the velocity quadword and v1 the kind byte when int_to_float runs.
    c.r[2] = vel;
    c.r[3] = kind as u128;
    let (_, count_f) = h::run_keep(c, mathx::int_to_float, &[count], &[]);
    let rate = scale / count_f;

    c.w32(rec + REC_EXTRA, extra);
    c.w32(rec + REC_TAG_WORD, tag);
    c.w32(rec + REC_COUNT_WORD, count);
    c.wf(rec + REC_RATE, rate);
    c.fret(rate);
    true
}
