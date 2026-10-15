//! TFrag texture passes. `tfrag_texture_chain` closes the tfrag DMA chain (end tag at the cursor, a count check that
//! may spill camera args) and `tfrag_texture_patch` walks the per-slot residency list and patches the GS record words.
use crate::camswitch;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const AT: usize = 1;
const SP: usize = 29;
const GP: usize = 28;
const RA: usize = 31;
const S0: usize = 16;
const S1: usize = 17;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;
const T2: usize = 10;

/// Retail word (0x160efc) holding the DMA tag write pointer.
const TAG_PTR_SLOT: u32 = 0x0016_0efc;
/// Retail word (0x161140) holding the chain cursor.
const CURSOR: u32 = 0x0016_1140;
/// Retail word (0x160f04) compared against the chain count.
const LIMIT: u32 = 0x0016_0f04;
/// Retail close-flag block (a3 at entry, 0x16a0f0).
const CLOSE_TAG: u32 = 0x0016_a0f0;
/// Retail DMA end tag word.
const DMA_END: u32 = 0x2000_0000;
/// Camera spill argument block passed to `camswitch::spill_cam_args`.
const SPILL_ARG: u32 = 0x001d_b0f0;
/// Retail lui value (0x1e0000) that the spill call's delay slot completes.
const SPILL_HI: u32 = 0x001e_0000;
/// Retail callees of the chain close.
const CHAIN_OPEN: u32 = 0x0028_57b0;
const CHAIN_FLUSH: u32 = 0x0028_5920;
const CHAIN_COUNT: u32 = 0x0029_15b0;
/// Retail `gp`-relative slots: close flag, flush argument, count store.
const GP_FLAG: u32 = 0x5ac0;
const GP_ARG: u32 = 0x7d8c;
const GP_COUNT: u32 = 0x5cfc;
/// Limit value (0x400000) compared against the count.
const COUNT_LIMIT: i32 = 0x0040_0000;

/// Retail residency list (0x1c5080) and the GS record table (0x1c4d80, indexed by a byte of each record).
const LIST: u32 = 0x001c_5080;
const PATCH_TABLE: u32 = 0x001c_4d80;
/// Mask keeping the upper bits of a GS word (sign-extended 0xffffc000).
const PATCH_MASK: u32 = 0xffff_c000;

/// Writes a 32-bit result to a register, sign-extended to 64 bits as MIPS 32-bit ops do.
fn set(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x284878 tfrag_texture_chain(): closes the tfrag DMA chain with the end tag at the cursor, runs the count-limited
/// camera-arg spill and writes the chain record words. Preserves s0, s1, ra and sp.
pub fn tfrag_texture_chain(c: &mut Cpu) -> bool {
    let sp = (c.g(SP) as u32).wrapping_sub(0x30);
    let gp = c.g(GP) as u32;
    set(c, SP, sp);
    c.mem.wr128(sp.wrapping_add(0x10), c.r[S1]);
    c.mem.wr128(sp.wrapping_add(0x20), c.r[RA]);
    c.mem.wr128(sp, c.r[S0]);

    let tag_ptr = c.r32(TAG_PTR_SLOT);
    set(c, A1, tag_ptr);
    set(c, A2, DMA_END);
    let cursor = c.r32(CURSOR);
    set(c, S1, cursor);
    set(c, A3, CLOSE_TAG);
    set(c, V1, cursor.wrapping_add(0x10));
    c.w32(CURSOR, cursor.wrapping_add(0x10));
    c.w32(tag_ptr, DMA_END);

    let cur = c.r32(CURSOR);
    set(c, V0, cur);
    let ptr = c.r32(TAG_PTR_SLOT);
    set(c, A0, ptr);
    c.w32(ptr.wrapping_add(4), cur);
    let ptr = c.r32(TAG_PTR_SLOT);
    set(c, V1, ptr);
    c.w32(ptr.wrapping_add(8), 0);
    let ptr = c.r32(TAG_PTR_SLOT);
    set(c, V0, ptr);
    c.w32(ptr.wrapping_add(0xc), 0);

    let flag = c.r32(CLOSE_TAG + 0x10);
    set(c, V1, flag);
    // Delay slot of the flag branch: runs on both paths.
    let spill = c.r32(gp.wrapping_sub(GP_FLAG));
    set(c, V1, spill);
    let mut v0 = c.g(V0) as u32;
    if flag != 0 {
        v0 = c.r32(CLOSE_TAG + 0xc);
        set(c, V0, v0);
    }
    set(c, A1, DMA_END);

    if flag != 0 && v0 != 0 {
        let args = [c.g(A0) as u32, c.g(A1) as u32, c.g(A2) as u32, c.g(A3) as u32];
        h::call_keep(c, CHAIN_OPEN, &args, &[]);
        let a0v = c.r32(gp.wrapping_sub(GP_ARG));
        set(c, A0, a0v);
        let args = [c.g(A0) as u32, c.g(A1) as u32, c.g(A2) as u32, c.g(A3) as u32];
        h::call_keep(c, CHAIN_FLUSH, &args, &[]);
        let flushed = c.g(V0) as u32;
        set(c, S0, flushed);
        let args = [c.g(A0) as u32, c.g(A1) as u32, c.g(A2) as u32, c.g(A3) as u32];
        h::call_keep(c, CHAIN_COUNT, &args, &[]);
        let count = c.g(S0) as u32 as i32;
        let over = COUNT_LIMIT < count;
        set(c, V0, over as u32);
        set(c, A0, SPILL_HI);
        if over {
            set(c, A0, SPILL_ARG);
            h::run_keep(c, camswitch::spill_cam_args, &[SPILL_ARG], &[]);
        }
        let limit = c.r32(LIMIT) as i32;
        set(c, V0, limit as u32);
        let limit_lt = limit < (c.g(S0) as u32 as i32);
        set(c, V0, limit_lt as u32);
        if limit_lt {
            let s0v = c.g(S0) as u32;
            c.w32(gp.wrapping_sub(GP_COUNT), s0v);
        }
        let cur2 = c.r32(CURSOR);
        set(c, V1, cur2);
        set(c, A1, DMA_END);
    }

    let v1_now = c.g(V1) as u32;
    c.w32(v1_now, DMA_END);
    c.r[RA] = c.mem.rd128(sp.wrapping_add(0x20));
    c.r[S0] = c.mem.rd128(sp);

    let slot = c.r32(TAG_PTR_SLOT);
    set(c, V0, slot);
    let cur3 = c.r32(CURSOR);
    set(c, A0, cur3);
    set(c, V0, slot.wrapping_add(0x10));
    c.w32(cur3.wrapping_add(4), slot.wrapping_add(0x10));
    let cur4 = c.r32(CURSOR);
    set(c, V1, cur4);
    c.w32(cur4.wrapping_add(8), 0);
    let cur5 = c.r32(CURSOR);
    set(c, A0, cur5);
    c.w32(cur5.wrapping_add(0xc), 0);
    let cur6 = c.r32(CURSOR);
    let cur6n = cur6.wrapping_add(0x10);
    set(c, V0, cur6n);
    c.w32(CURSOR, cur6n);
    let s1v = c.g(S1) as u32;
    c.w32(s1v, DMA_END);
    let cur7 = c.r32(CURSOR);
    set(c, V0, cur7);
    c.w32(s1v.wrapping_add(0xc), 0);
    c.w32(s1v.wrapping_add(4), cur7);
    c.w32(s1v.wrapping_add(8), 0);

    // The retail `lui $at, 0x16` before the last cursor store leaves at = 0x160000.
    set(c, AT, 0x0016_0000);
    c.r[S1] = c.mem.rd128(sp.wrapping_add(0x10));
    set(c, SP, sp.wrapping_add(0x30));
    true
}

/// 0x2849c0 tfrag_texture_patch(): walks the residency list at 0x1c5080 (pairs of head pointer and count) and, for each
/// record of `count` GS entries, ORs the table halfwords into the low and TBP words (keeping the 0xffffc000 mask).
pub fn tfrag_texture_patch(c: &mut Cpu) -> bool {
    set(c, V0, LIST & 0xffff_0000);
    let head = c.r32(LIST);
    set(c, V1, head);
    set(c, A0, LIST);
    if head == 0 {
        return true;
    }
    set(c, T2, LIST & 0xffff_0000);
    let mut slot = LIST;
    loop {
        let count = c.r32(slot + 4);
        set(c, V0, count);
        set(c, T0, slot + 8);
        let ptr = c.r32(slot);
        set(c, A0, ptr);
        if (count as i32) > 0 {
            set(c, A3, count);
            c.s(A2, 0xffff_ffff_ffff_c000);
            set(c, T1, PATCH_TABLE);
            let mut e = ptr;
            let mut left = count;
            loop {
                let idx = c.r8(e + 0x23) as u32;
                let scaled = idx << 2;
                set(c, V0, scaled);
                let entry = PATCH_TABLE.wrapping_add(scaled);
                set(c, A1, entry);
                let lo = c.r16(entry) as i16 as i32 as u32;
                if lo != 0 {
                    let w = c.r32(e);
                    let nv = (w & PATCH_MASK) | lo;
                    set(c, V0, nv);
                    c.w32(e, nv);
                }
                let hi = c.r16(entry + 2) as i16 as i32 as u32;
                set(c, V1, hi);
                if hi != 0 {
                    let w = c.r32(e + 0x30);
                    let nv = (w & PATCH_MASK) | hi;
                    set(c, V0, nv);
                    c.w32(e + 0x30, nv);
                }
                left -= 1;
                set(c, A3, left);
                e = e.wrapping_add(0x50);
                set(c, A0, e);
                if left == 0 {
                    break;
                }
            }
        }
        set(c, A0, slot + 8);
        let next = c.r32(slot + 8);
        set(c, V0, next);
        if next == 0 {
            break;
        }
        slot += 8;
    }
    true
}
