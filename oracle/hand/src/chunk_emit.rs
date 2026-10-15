//! Chunk list emitter (retail 0x23f5b0..0x23f698): copies each list entry's data block into the chunk and writes the
//! chunk header through the retail finisher at 0x23f4f8.

use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail finisher that writes the chunk trailer and returns its value.
const CHUNK_FINISH: u32 = 0x0023_f4f8;
/// Stride of one list entry: data base, count, pad, tag.
const ENTRY_STRIDE: u32 = 0x10;
/// Offset of the first record inside the chunk header.
const RECORD_BASE: u32 = 8;

/// 0x23f5b0 chunk_emit(dst, scale, list) -> size: copies each list entry's block (base + scale * count, count bytes)
/// into 8-byte records after the header, 4-byte aligned, then writes the header and returns size + 8
pub fn chunk_emit(c: &mut Cpu) -> bool {
    let dst = c.arg(0);
    let scale = c.arg(1);
    let list = c.arg(2);
    let mut out = dst.wrapping_add(RECORD_BASE);
    let mut total: u32 = 0;
    let mut entry = list;
    if c.r32(entry) != 0 {
        loop {
            total = total.wrapping_add(RECORD_BASE);
            let tag = c.r32(entry + 8);
            let base = c.r32(entry);
            let count = c.r32(entry + 4);
            c.w32(out, tag);
            c.w32(out + 4, count);
            out = out.wrapping_add(RECORD_BASE);
            let src = base.wrapping_add(scale.wrapping_mul(count));
            h::run_keep(c, mathx::mem_copy, &[out, src, count], &[]);
            let size = c.r32(entry + 4);
            entry = entry.wrapping_add(ENTRY_STRIDE);
            out = out.wrapping_add(size).wrapping_add(3) & !3;
            total = total.wrapping_add(size).wrapping_add(3) & !3;
            if c.r32(entry) == 0 {
                break;
            }
        }
    }
    total = total.wrapping_add(RECORD_BASE);
    c.w32(out + 4, 0);
    c.w32(out, 0xffff_ffff);
    let (v0, _) = h::call_keep(c, CHUNK_FINISH, &[dst.wrapping_add(RECORD_BASE), total], &[]);
    c.w32(dst + 4, v0);
    c.w32(dst, total);
    c.ret(total.wrapping_add(RECORD_BASE));
    true
}
