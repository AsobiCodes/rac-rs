//! Spawns a 0x15-byte record: copies two 16-byte quads into it and fills the header fields from the arguments.
use crate::h;

use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail allocator (0x259a38), called with the record kind; returns the record pointer or 0.
const ALLOC: u32 = 0x259a38;
/// Record kind passed to `ALLOC`.
const KIND: u32 = 0x15;
/// Retail helper (0x1160d8) whose return byte is stored at record +8.
const HELPER: u32 = 0x1160d8;
/// Retail float-to-int conversion (0x1f27e8), called with f12 = 4.0.
const FLOAT_TO_INT: u32 = 0x1f27e8;
/// Float 4.0 bits, the argument to `mathx::float_to_int`.
const FOUR_BITS: u32 = 0x4080_0000;
const FOUR: F = F(FOUR_BITS);
/// Byte stored at record +3, also the value of v1 when the helper runs.
const TYPE_BYTE: u32 = 0x48;
/// Bias subtracted from the converted scale before it is stored at record +9.
const BIAS: u32 = 0x60;
/// Base of the table whose pointer (at `TABLE_OFF`) gives the byte copied to record +2.
const TABLE_BASE: u32 = 0x1b_0000;
const TABLE_OFF: u32 = 0x21d4;
/// Retail frame size and the stack slot where it saves f20.
const FRAME: u32 = 0x90;
const SAVE_F20: u32 = 0x80;

/// Sign-extends a 32-bit value into a 64-bit register image, as `addiu`/`lui` do.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// 0x25ee50 spawn_record(a0 quad src, a1 quad tail, a2 word, a3 word, t0 kind, t1 half, f12 scale) -> record or 0:
/// allocates a 0x15-byte record and fills it; kind 0 returns 0 without allocating.
pub fn spawn_record(c: &mut Cpu) -> bool {
    let src = c.arg(0);
    let tail = c.arg(1);
    let word_a = c.arg(2);
    let word_b = c.arg(3);
    let kind = c.arg(4);
    let half = c.arg(5);
    let scale = c.farg(0);
    let saves = [
        (22, 0x60, h::SQ),
        (21, 0x50, h::SQ),
        (20, 0x40, h::SQ),
        (19, 0x30, h::SQ),
        (18, 0x20, h::SQ),
        (17, 0x10, h::SQ),
        (16, 0x00, h::SQ),
        (31, 0x70, h::SQ),
    ];
    h::frame(c, FRAME, &saves, |c, sp| {
        // The retail prologue stores the caller's f20 in the frame.
        c.wf(sp + SAVE_F20, F(c.f[20]));
        if kind == 0 {
            c.ret(0);
            return;
        }
        let (rec, _) = h::call_keep(c, ALLOC, &[KIND], &[]);
        // The delay slot of the beqz sets v1 = record + 0x10 whether or not the allocation worked.
        c.s(3, sx(rec.wrapping_add(0x10)));
        if rec == 0 {
            c.ret(0);
            return;
        }
        let head = c.rv(src);
        c.wv(rec + 0x10, head);
        c.w32(rec + 4, word_a);
        c.s(1, sx(FOUR_BITS));
        let (code, _) = h::call_keep(c, FLOAT_TO_INT, &[], &[FOUR]);
        c.w8(rec + 1, 0);
        c.wf(rec + 0xc, scale);
        c.w8(rec + 3, TYPE_BYTE as u8);
        c.w8(rec + 9, code.wrapping_sub(BIAS) as u8);
        c.s(3, sx(TYPE_BYTE));
        let (ret, _) = h::call_keep(c, HELPER, &[], &[]);
        c.w8(rec + 8, ret as u8);
        c.s(4, sx(TABLE_BASE));
        let table = c.r32(TABLE_BASE + TABLE_OFF);
        let byte = c.r8(table);
        c.s(5, byte as u64);
        c.w8(rec + 2, byte);
        c.w16(rec + 0xa, kind as u16);
        let body = rec + 0x20;
        c.s(3, sx(body));
        c.w16(body + 0x18, kind as u16);
        c.wf(body + 0x1c, scale);
        c.w32(body + 0x10, word_a);
        c.w32(body + 0x14, word_b);
        c.w16(body + 0x1a, half as u16);
        let last = c.rv(tail);
        c.wv(body, last);
        c.ret(rec);
    });
    true
}
