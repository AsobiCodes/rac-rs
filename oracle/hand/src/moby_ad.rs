//! Moby class relocation and the texture A+D packet writer: 0x232648 writes the four GS A+D words of one texture block,
//! and 0x232c08 relocates a moby class block by its own address, remaps its byte runs, and patches every AD block
//! through the packet writer.
use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

/// Base of the 16-byte slots; slot `n` receives the qword copied from the class table for index byte `n`.
const SLOT_BASE: u32 = 0x19_AA80;
/// Index bytes: one per class slot, read at `slot + SLOT_INDEX_TABLE`.
const SLOT_INDEX_TABLE: u32 = 0x19_7CC0;
/// Word whose arithmetic `>> 8` is the signed position scale added to every packed offset.
const POS_SCALE: u32 = 0x15_EE8C;
/// Two 16-byte data blocks for the `mode < -1` packets (`mode == -3` picks the second).
const DATA_A: u32 = 0x18_28C0;
const DATA_B: u32 = 0x18_28D8;
/// Retail sub-patch used when no remap table is given.
const SUBBLOCK_PATCH: u32 = 0x23_28A0;
/// Byte that ends a remap run.
const RUN_END: u8 = 0xff;

/// Sign-extends a 32-bit register value to 64 bits.
fn ext(v: i32) -> u64 {
    v as i64 as u64
}

/// The packed scale fields are clamped to at least 1.
fn clamp1(v: i32) -> i32 {
    if v > 0 { v } else { 1 }
}

/// 0x1f1590 log2_floor(n): index of the top set bit of `n` (or of `!n` when negative); -1 for zero.
fn log2_floor(n: i32) -> i32 {
    let w = n as u32;
    let t = if w >> 31 != 0 { !w } else { w };
    30u32.wrapping_sub(t.leading_zeros() - 1) as i32
}

fn w64(c: &mut Cpu, a: u32, v: u64) {
    c.w32(a, v as u32);
    c.w32(a.wrapping_add(4), (v >> 32) as u32);
}

fn r64(c: &mut Cpu, a: u32) -> u64 {
    let lo = c.r32(a) as u64;
    let hi = c.r32(a.wrapping_add(4)) as u64;
    lo | (hi << 32)
}

/// 0x232648 moby_ad_patch_tex(out, blk, a2, a3, t0, t1, t2) -> none: writes the four GS A+D words at `out` (+0x00,
/// +0x10, +0x20, +0x30) for one texture block, from the packed scales and offsets stored in `blk`.
pub fn moby_ad_patch_tex(c: &mut Cpu) -> bool {
    let (out, blk, a2, a3) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let (sp0, fp, mode) = (c.arg(4), c.arg(5), c.arg(6));
    patch_tex(c, out, blk, a2, a3, sp0, fp, mode);
    true
}

/// Packet writer behind `moby_ad_patch_tex`; `mode` picks one of three packet layouts (>= 0, < -1, == -1).
fn patch_tex(c: &mut Cpu, out: u32, blk: u32, a2: u32, a3: u32, sp0: u32, fp: u32, mode: u32) {
    let s3 = mode as i32;
    let s5 = a3 as i32;
    let s6 = a2 as i32;
    let ws = c.r16(blk + 4) as i16 as i32;
    let s4 = clamp1(ws >> 7);
    let s2 = clamp1(ws >> 6);
    let l6 = log2_floor(c.r16(blk + 6) as i16 as i32);
    let scale = (c.r32(POS_SCALE) as i32) >> 8;
    let t4 = c.r16(blk + 0xe) as i16 as i32 + scale;
    let t3 = c.r16(blk + 0xa) as i16 as i32 + scale;
    let t0 = c.r16(blk + 0xc) as i16 as i32 + scale;
    let h8 = c.r16(blk + 8) as i16 as i32;

    if s3 >= 0 {
        let q0 = (ext(h8 - 1) << 2) | (ext(s5) << 6) | 0x20 | (ext(s6) << 32);
        let q1 = (ext(sp0 as i32) | (ext(fp as i32) << 2)) | (ext(s3) << 24);
        let q2 = (ext(s2) << 14)
            | (ext(l6) << 26)
            | 0x1300_0000
            | (ext(l6) << 30)
            | (ext(t3) << 37)
            | (1 << 34)
            | (1 << 63);
        let q3 = (ext(s4) << 14) | (ext(t0) << 20) | (ext(t4) << 40) | (1 << 34) | (1 << 54);
        w64(c, out, q0);
        w64(c, out + 0x10, q1);
        w64(c, out + 0x20, q2);
        w64(c, out + 0x30, q3);
    } else if s3 < -1 {
        let data = if s3 == -3 { DATA_B } else { DATA_A };
        w64(c, out, (ext(s5) << 6) | 0x20 | (ext(s6) << 32));
        w64(c, out + 0x10, 5);
        let d0 = r64(c, data);
        let d1 = r64(c, data + 0x10);
        w64(c, out + 0x20, d0);
        w64(c, out + 0x30, d1);
    } else {
        w64(c, out, (ext(s5) << 6) | (ext(s6) << 32) | 0x20);
        w64(c, out + 0x10, 5);
        w64(c, out + 0x20, (1u64 << 63) | (0x9980u64 << 19) | 0x7ffb);
        w64(c, out + 0x30, 0);
    }
}

/// 0x232c08 moby_class_relocate(class, table, remap, slot) -> none: adds the class address to the pointers of a moby
/// class block, remaps its byte runs through `remap`, and patches every AD block (see `moby_ad_patch_tex`).
pub fn moby_class_relocate(c: &mut Cpu) -> bool {
    // Unverified against retail (BAD on the only checked call): retail runs it.
    let _ = c;
    false
}

/// Adds `base` to the word at `a`.
fn add_word(c: &mut Cpu, a: u32, base: u32) {
    let v = c.r32(a).wrapping_add(base);
    c.w32(a, v);
}

/// Turns the offset word at `a` into an address (`base + offset`) unless it is zero.
fn rebase_if_set(c: &mut Cpu, a: u32, base: u32) {
    let v = c.r32(a);
    if v != 0 {
        c.w32(a, base.wrapping_add(v));
    }
}

/// Remaps every byte from `start` through the next 0xff terminator (exclusive) with the table at `remap`.
fn remap_run(c: &mut Cpu, start: u32, remap: u32) {
    let mut p = start;
    loop {
        let b = c.r8(p);
        let m = c.r8(remap.wrapping_add(b as u32));
        c.w8(p, m);
        p = p.wrapping_add(1);
        if c.r8(p) == RUN_END {
            break;
        }
    }
}

fn relocate(c: &mut Cpu, p: u32, table: u32, remap: u32, slot: u32) {
    // Header bytes 4..6 hold the record counts; their sum is the number of 16-byte records at the first table.
    let recs = c.r8(p + 4) as u32 + c.r8(p + 5) as u32 + c.r8(p + 6) as u32;

    let first = c.r32(p);
    if first != 0 {
        let base = p.wrapping_add(first);
        c.w32(p, base);
        for i in 0..recs {
            let r = base.wrapping_add(16 * i);
            add_word(c, r, p);
            add_word(c, r + 8, p);
        }
    }
    rebase_if_set(c, p + 0x10, p);
    rebase_if_set(c, p + 0x14, p);
    rebase_if_set(c, p + 0x18, p);

    // A counted list of words (count first), each relocated.
    let list = c.r32(p + 0x1c);
    if list != 0 {
        let base = p.wrapping_add(list);
        c.w32(p + 0x1c, base);
        let n = c.r32(base) as i32;
        for i in 0..n.max(0) as u32 {
            add_word(c, base + 4 + 4 * i, p);
        }
    }

    // A ring of 16-byte records ending when a record's word at +0xc turns negative.
    let ring = c.r32(p + 0x20);
    if ring != 0 {
        let mut cur = p.wrapping_add(ring);
        c.w32(p + 0x20, cur);
        loop {
            add_word(c, cur + 0xc, p);
            if c.r8(cur) != RUN_END {
                remap_run(c, cur, remap);
            }
            if (c.r32(cur + 0xc) as i32) < 0 {
                break;
            }
            cur = cur.wrapping_add(0x10);
        }
    }
    rebase_if_set(c, p + 0x28, p);

    // Pointer table at +0x48 with one entry per count byte at +0xc.
    let count = c.r8(p + 0xc) as u32;
    for i in 0..count {
        let at = p + 0x48 + 4 * i;
        let v = c.r32(at);
        if v == 0 {
            continue;
        }
        let rec = p.wrapping_add(v);
        c.w32(at, rec);
        rebase_if_set(c, rec + 0x14, p);
        let k = c.r8(rec + 0x10) as u32;
        for j in 0..k {
            add_word(c, rec + 0x1c + 4 * j, p);
        }
    }

    // Copy the qword at `remap` into the slot picked by the index byte at `slot + SLOT_INDEX_TABLE`.
    let dst = SLOT_BASE + 16 * c.r8(slot.wrapping_add(SLOT_INDEX_TABLE)) as u32;
    let q = c.rv(remap);
    c.wv(dst, q);

    // Patch every AD block of the `recs` records starting at the first table.
    let mut rec = c.r32(p);
    for _ in 0..recs {
        let block = c.r32(rec);
        let packed = c.r32(rec + 4);
        let n1 = (packed as i32) >> 16;
        let lo = packed & 0xffff;
        c.w32(rec + 4, lo);
        let mut s0 = block.wrapping_add(lo.wrapping_sub(n1 as u32) << 4);
        if n1 > 0 {
            let mut s1 = 0i32;
            loop {
                let raw = c.r32(s0 + 0x20) as i32;
                let idx = if raw < 0 { raw } else { c.r8(dst.wrapping_add(raw as u32)) as i32 };
                let w0 = c.r32(s0);
                let w4 = c.r32(s0 + 4);
                let w10 = c.r32(s0 + 0x10);
                let w14 = c.r32(s0 + 0x14);
                if table != 0 {
                    let blk = table.wrapping_add((idx as u32) << 4);
                    patch_tex(c, s0, blk, w0, w4, w10, w14, idx as u32);
                } else {
                    let _ = crate::h::call(c, SUBBLOCK_PATCH, &[s0, w0, w4, w10, w14], &[]);
                }
                s1 += 4;
                s0 = s0.wrapping_add(0x40);
                if s1 >= n1 {
                    break;
                }
            }
        }
        rec = rec.wrapping_add(16);
    }
}
