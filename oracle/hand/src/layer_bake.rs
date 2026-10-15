//! Entry-offset shifting and layer-frame baking for the particle/layer records (retail overlay code).
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail `jal` target that the layer bake calls with a0 = 0 at its end.
const FLUSH_HOOK: u32 = 0x118a80;
/// Size of the layer bake's stack frame (retail `addiu sp, sp, -0x60`).
const FRAME_SIZE: u32 = 0x60;
/// Size of one layer slot record (16 bytes; `sll 3` is one 8-byte word per slot).
const SLOT_STRIDE: u32 = 16;

/// 0x241878 shift_entry_offsets(rec, dx, dy): adds dx to the u16 at +0 and dy to the u16 at +2 of every
/// 4-byte entry, over the (u8 at +4 + u8 at +5) groups listed by the 16-byte table that `rec` points to.
pub fn shift_entry_offsets(c: &mut Cpu) -> bool {
    let rec = c.arg(0);
    let (dx, dy) = (c.arg(1) as u16, c.arg(2) as u16);
    let groups = c.r8(rec + 4) as u32 + c.r8(rec + 5) as u32;
    if groups == 0 {
        return true;
    }
    let table = c.r32(rec);
    for i in 0..groups {
        let group = c.r32(table.wrapping_add(SLOT_STRIDE * i));
        let count = c.r8(group + 2) as u32;
        let mut p = group + 4;
        for _ in 0..count {
            let x = c.r16(p);
            c.w16(p, x.wrapping_add(dx));
            let y = c.r16(p + 2);
            c.w16(p + 2, y.wrapping_add(dy));
            p += 4;
        }
    }
    true
}

fn r64(c: &mut Cpu, a: u32) -> u64 {
    let lo = c.r32(a) as u64;
    let hi = c.r32(a + 4) as u64;
    lo | (hi << 32)
}

fn w64(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a + 4, (v >> 32) as u32);
}

/// 0x241d08 bake_layer_frame(obj, i1, i2, out): builds the layer frame at `out` from the class record of `obj`
/// (+0x14 / +0x18 info blocks; slot table at +0x48 of the info, indexed by i1 and i2, both in words): zeroes the
/// frame, writes its header words, copies the source quadwords, scatters the 8-byte slots by index, and sets the
/// first empty slot word to 1 before the retail flush hook runs with a0 = 0.
pub fn bake_layer_frame(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let saves = [(16usize, 0x00u32, h::SQ), (18, 0x20, h::SQ), (20, 0x40, h::SQ), (31, 0x50, h::SQ),
        (19, 0x30, h::SQ), (17, 0x10, h::SQ)];
    h::frame(c, FRAME_SIZE, &saves, |c, _sp| {
        let obj = c.arg(0);
        let i1 = c.arg(1) << 2;
        let i2 = c.arg(2) << 2;
        let out = c.arg(3);
        let info = c.r32(obj + 0x18);
        let info_b = c.r32(obj + 0x14);
        let s3 = c.r8(info + 8) as u32;
        let s0 = c.r8(info_b + 8) as u32;
        let slot_table = c.r32(info + i1 + 0x48).wrapping_add(i2);
        let src = c.r32(slot_table + 0x1c);

        // Zero the frame: (s0 + 3) * 8 bytes, rounded down to whole quadwords within 0xff0.
        let zero_len = ((s0 + 3) << 3) & 0xff0;
        h::run(c, mathx::zero_qwords, &[out, zero_len], &[]);

        let h_a = c.r16(src + 0xa) as i16 as i32;
        let h_e = c.r16(src + 0xe) as i16 as i32;
        let base_off = s0 << 3;
        c.w16(out + 8, base_off as u16);
        let half = ((s0 as i32 + h_a + h_e + 1) >> 1) as u16;
        c.w16(out + 6, half);
        let s3_bytes = s3 << 3;
        let body = src.wrapping_add(s3_bytes).wrapping_add(0x10);
        let u_a = c.r16(src + 0xa) as u32;
        c.w16(out + 0xa, u_a as u16);
        let end_slots = (u_a.wrapping_add(s0)) << 3;
        c.w16(out + 0xc, end_slots as u16);
        let u_e = c.r16(src + 0xe);
        c.w16(out + 0xe, u_e);

        let src_len = ((c.r16(src + 6) as i16 as i32) << 4).wrapping_sub(s3_bytes as i32) as u32;
        if src_len != 0 {
            h::run(c, mathx::mem_copy, &[out.wrapping_add(base_off).wrapping_add(0x10), body, src_len], &[]);
        }

        let info = c.r32(obj + 0x18);
        let mut a0 = c.r32(info + 0x1c).wrapping_add(4);
        let mut a2 = src + 0x10;
        for _ in 0..s3 {
            let idx = c.r8(a0) as u32;
            let v = r64(c, a2);
            w64(c, out.wrapping_add(idx << 3).wrapping_add(0x10), v);
            a0 = a0.wrapping_add(1);
            a2 = a2.wrapping_add(8);
        }

        let off = ((c.r16(out + 6) as i16 as i32) << 4) as u32;
        let slot = out.wrapping_add(off);
        if r64(c, slot) == 0 {
            w64(c, slot, 1);
        }
        h::call_keep(c, FLUSH_HOOK, &[0], &[]);
        true
    })
}
