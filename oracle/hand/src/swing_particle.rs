//! Hand-rewritten retail functions: the swing-camera pair call (0x231318) and the particle texture loader (0x231b88).

use crate::h::{self, SQ};

use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail callees of 0x231318, called in order with the same a0.
const SWING_FIRST: u32 = 0x231348;
const SWING_SECOND: u32 = 0x230f68;
/// Retail mem_copy(dst, src, n) and log2_floor(n), called as retail does so their scratch registers match.
const MEM_COPY: u32 = 0x1f1628;
const LOG2_FLOOR: u32 = 0x1f1590;

/// Destination of the particle definition blob (copied with mem_copy).
const PARTICLE_BLOB_DST: u32 = 0x1b2380;
/// Scratch array of rebased blob offsets, one word per header entry.
const PARTICLE_OFFSETS: u32 = 0x1b2180;
/// Particle texture record table: 8-byte slots, pal/pix words and side/log2 words.
const PARTICLE_RECORDS: u32 = 0x1b1b80;
/// Gp-relative counter (0x160100 is the same word as gp-0x6b00 in the retail build).
const COUNTER_ABS: u32 = 0x160100;
const COUNTER_GP_OFF: u32 = 0xffff_9500;
/// High half of the blob-copy base (lui 0x1b).
const HI_1B: u32 = 0x1b_0000;

/// Saved-register frame of 0x231b88: s0-s5 and ra at fixed offsets.
const PARTICLE_FRAME: u32 = 0x70;

/// Sets a MIPS register to a 32-bit value, sign-extended like the EE 32-bit ops.
fn set(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// Address of the gp-relative counter word.
fn counter_addr(c: &Cpu) -> u32 {
    (c.g(28) as u32).wrapping_add(COUNTER_GP_OFF)
}

/// 0x231318 swing_cam_pair(a0) -> void: calls 0x231348(a0) then 0x230f68(a0) and returns with a0 unchanged.
pub fn swing_cam_pair(c: &mut Cpu) -> bool {
    let a0 = c.arg(0) as u32;
    h::frame(c, 0x20, &[(16, 0x00, SQ), (31, 0x10, SQ)], |c, _sp| {
        set(c, 16, a0);
        let _ = h::call_keep(c, SWING_FIRST, &[a0], &[]);
        let _ = h::call_keep(c, SWING_SECOND, &[a0], &[]);
    });
    true
}

/// 0x231b88 particle_tex_install(a0 hdr, a1 base, a2 recs, a3 n) -> void: copies the blob at hdr+hdr.blob_off to
/// 0x1b2380 (mem_copy), then writes n four-word records from recs into the 0x1b1b80 table.
pub fn particle_tex_install(c: &mut Cpu) -> bool {
    let hdr = c.arg(0) as u32;
    let base = c.arg(1) as u32;
    let recs = c.arg(2) as u32;
    let n = c.arg(3) as i32;
    let count = c.r32(hdr) as i32;
    let blob_off = c.r32(hdr.wrapping_add(8));
    let blob_len = c.r32(hdr.wrapping_add(0xc));
    let entries = hdr.wrapping_add(0x10);
    let saves = [
        (16, 0x00, SQ),
        (17, 0x10, SQ),
        (18, 0x20, SQ),
        (19, 0x30, SQ),
        (20, 0x40, SQ),
        (21, 0x50, SQ),
        (31, 0x60, SQ),
    ];
    h::frame(c, PARTICLE_FRAME, &saves, |c, _sp| {
        let blob_src = hdr.wrapping_add(blob_off);
        if count > 0 {
            let t1 = blob_off.wrapping_sub(PARTICLE_BLOB_DST);
            let mut last = 0u32;
            for i in 0..count as u32 {
                let w = c.r32(entries.wrapping_add(4 * i));
                let v = w.wrapping_sub(t1);
                last = v;
                let stored = if w != 0 { v } else { PARTICLE_BLOB_DST };
                c.w32(PARTICLE_OFFSETS.wrapping_add(4 * i), stored);
            }
            set(c, 2, last);
            set(c, 3, 0);
            set(c, 8, PARTICLE_BLOB_DST);
            set(c, 9, t1);
            set(c, 7, entries.wrapping_add((4 * count) as u32));
        } else {
            set(c, 3, count as u32);
            set(c, 7, entries);
        }
        set(c, 10, blob_off);
        set(c, 11, HI_1B);
        set(c, 4, PARTICLE_BLOB_DST);
        set(c, 5, blob_src);
        set(c, 6, blob_len);
        let _ = h::call_keep(c, MEM_COPY, &[PARTICLE_BLOB_DST, blob_src, blob_len], &[]);

        if n > 0 {
            let gp_counter = counter_addr(c);
            c.w32(gp_counter, 0);
            let mut rec = recs;
            let mut last_g = 0u32;
            loop {
                let a = c.r32(rec);
                let b = c.r32(rec.wrapping_add(4));
                let s = c.r32(rec.wrapping_add(8));
                let d = c.r32(rec.wrapping_add(12));
                let g = c.r32(COUNTER_ABS);
                let slot = PARTICLE_RECORDS.wrapping_add(g.wrapping_mul(8));
                c.w32(slot, (base.wrapping_add(a) << 4).wrapping_add(b));
                let log2 = h::call_keep(c, LOG2_FLOOR, &[d], &[]).0 as u32;
                let g2 = c.r32(COUNTER_ABS);
                let slot2 = PARTICLE_RECORDS.wrapping_add(4).wrapping_add(g2.wrapping_mul(8));
                c.w32(slot2, (base.wrapping_add(s) << 4).wrapping_add(log2));
                last_g = g2;
                let next = g2.wrapping_add(1);
                c.w32(gp_counter, next);
                rec = rec.wrapping_add(16);
                if (next as i32) >= n {
                    break;
                }
            }
            set(c, 2, PARTICLE_RECORDS.wrapping_add(4).wrapping_add(last_g.wrapping_mul(8)));
            set(c, 3, last_g.wrapping_add(1));
            set(c, 4, 0);
        }
    });
    true
}
