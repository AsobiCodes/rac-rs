//! DMA tag chain append (retail 0x286218..0x286350): writes a NEXT tag (0x2000_0000) at the block the tag pointer names,
//! advances the chain write pointer by one qword and links the old block to the new end. The slow path (table word
//! +0x14 set) calls the count helpers and may spill the camera arguments.

use crate::camswitch;
use crate::h;
use ee_oracle::cpu::Cpu;
use crate::ps2::*;

const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const S0: usize = 16;
const S1: usize = 17;
const GP: usize = 28;
const SP: usize = 29;
const RA: usize = 31;

/// Bytes reserved below sp for the saved s0, s1 and ra quadwords.
const FRAME: u32 = 0x30;
/// Word holding the current tag block address.
const TAG_BLOCK_PTR: u32 = 0x0016_0f58;
/// Word holding the chain write pointer (advanced by one qword per call).
const WRITE_PTR: u32 = 0x0016_1140;
/// Word compared against the count from 0x2915b0; a larger count is also stored at gp-0x5c9c.
const COUNT_LIMIT: u32 = 0x0016_0f64;
/// Table at 0x16a0f0: word +0x14 selects the slow path, word +0x18 selects the gp-slot target.
const TABLE: u32 = 0x0016_a0f0;
const TABLE_SLOW: u32 = TABLE + 0x14;
const TABLE_GP: u32 = TABLE + 0x18;
/// gp-relative words: the default write target, the argument of 0x287c98, the count store.
const GP_TARGET: i32 = -0x5ac0;
const GP_ARG: i32 = -0x7d8c;
const GP_COUNT: i32 = -0x5c9c;
/// Retail helpers with no Rust (0x1dd878 is camswitch::spill_cam_args).
const CALL_ARG: u32 = 0x0028_7c98;
const CALL_COUNT: u32 = 0x0029_15b0;
/// Tag word written at each block (DMA NEXT tag id, zero qwc).
const TAG_NEXT: u32 = 0x2000_0000;
/// Bytes the write pointer advances per call.
const WRITE_STEP: u32 = 0x10;
/// Count above which the camera arguments are spilled; spill argument is 0x1e0000 - 0x4ef8.
const COUNT_CAP: i64 = 0x0040_0000;
const SPILL_ARG: u32 = 0x001d_b108;
/// Value of `at` after `lui $at, 0x16`.
const HI_16: u32 = 0x0016_0000;

fn set(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

fn gp_word(c: &Cpu, off: i32) -> u32 {
    (c.g(GP) as u32).wrapping_add(off as u32)
}

fn save_quad(c: &mut Cpu, addr: u32, reg: usize) {
    let r = c.r[reg];
    c.mem.wr64(addr, r as u64);
    c.mem.wr64(addr + 8, (r >> 64) as u64);
}

/// 0x286218 dma_tag_link() -> (): appends a NEXT tag at the tag block, moves the chain write pointer forward by one
/// qword and links the new end into the block list; the target words depend on the table flags at 0x16a0f0.
pub fn dma_tag_link(c: &mut Cpu) -> bool {
    let sp_entry = c.g(SP);
    let s0_entry = c.g(S0);
    let s1_entry = c.g(S1);
    let sp = (sp_entry as u32).wrapping_sub(FRAME);
    set(c, SP, sp);
    save_quad(c, sp + 0x10, S1);
    save_quad(c, sp + 0x20, RA);
    save_quad(c, sp, S0);

    // Tag block gets the NEXT tag; the write pointer moves on and the old block links to it.
    let tag_block = c.r32(TAG_BLOCK_PTR);
    set(c, A1, tag_block);
    let old_write = c.r32(WRITE_PTR);
    set(c, S1, old_write);
    set(c, A2, TAG_NEXT);
    set(c, A3, TABLE);
    let new_write = old_write.wrapping_add(WRITE_STEP);
    set(c, V1, new_write);
    set(c, AT, HI_16);
    c.w32(WRITE_PTR, new_write);
    c.w32(tag_block, TAG_NEXT);

    { let v = c.r32(WRITE_PTR); set(c, V0, v); }
    { let v = c.r32(TAG_BLOCK_PTR); set(c, A0, v); }
    let link_blk = c.g(A0) as u32;
    let link_val = c.g(V0) as u32;
    c.w32(link_blk + 4, link_val);
    let blk = c.r32(TAG_BLOCK_PTR);
    set(c, V1, blk);
    c.w32(blk + 8, 0);
    set(c, V0, blk);
    c.w32(blk + 0xc, 0);

    // Pick the store target: the gp word, or the write pointer on the slow path.
    let flag_gp = c.r32(TABLE_GP);
    set(c, V1, flag_gp);
    let gp_target = c.r32(gp_word(c, GP_TARGET));
    set(c, V1, gp_target);
    let target = if flag_gp == 0 {
        set(c, A1, TAG_NEXT);
        gp_target
    } else {
        let flag_slow = c.r32(TABLE_SLOW);
        set(c, V0, flag_slow);
        set(c, A1, TAG_NEXT);
        if flag_slow == 0 {
            gp_target
        } else {
            let arg = c.r32(gp_word(c, GP_ARG));
            set(c, A0, arg);
            h::call_keep(c, CALL_ARG, &[arg, TAG_NEXT], &[]);
            let count = c.g(V0);
            c.s(S0, count);
            h::call_keep(c, CALL_COUNT, &[], &[]);
            let over = COUNT_CAP < c.g(S0) as i64;
            set(c, V0, over as u32);
            set(c, A0, 0x001e_0000);
            if over {
                set(c, A0, SPILL_ARG);
                h::run_keep(c, camswitch::spill_cam_args, &[SPILL_ARG], &[]);
            }
            { let v = c.r32(COUNT_LIMIT); set(c, V0, v); }
            let raise = (c.g(V0) as i64) < (c.g(S0) as i64);
            set(c, V0, raise as u32);
            if raise {
                let count_now = c.g(S0) as u32;
                let addr = gp_word(c, GP_COUNT);
                c.w32(addr, count_now);
            }
            { let v = c.r32(WRITE_PTR); set(c, V1, v); }
            set(c, A1, TAG_NEXT);
            c.r32(WRITE_PTR)
        }
    };
    c.w32(target, TAG_NEXT);

    // Restore s0, then link the write pointer block and advance the write pointer.
    c.s(S0, s0_entry);
    { let v = c.r32(TAG_BLOCK_PTR); set(c, V0, v); }
    { let v = c.r32(WRITE_PTR); set(c, A0, v); }
    let p_plus = c.g(V0).wrapping_add(WRITE_STEP as u64) as u32;
    set(c, V0, p_plus);
    let q = c.g(A0) as u32;
    c.w32(q + 4, p_plus);
    { let v = c.r32(WRITE_PTR); set(c, V1, v); }
    let v1_now = c.g(V1) as u32;
    c.w32(v1_now + 8, 0);
    let q2 = c.r32(WRITE_PTR);
    set(c, A0, q2);
    c.w32(q2 + 0xc, 0);
    let q3 = c.r32(WRITE_PTR).wrapping_add(WRITE_STEP);
    set(c, V0, q3);
    set(c, AT, HI_16);
    c.w32(WRITE_PTR, q3);

    // The old write block: tag, end link and cleared words.
    c.w32(old_write, TAG_NEXT);
    { let v = c.r32(WRITE_PTR); set(c, V0, v); }
    c.w32(old_write + 0xc, 0);
    c.w32(old_write + 4, c.g(V0) as u32);
    c.w32(old_write + 8, 0);

    c.s(S1, s1_entry);
    c.s(SP, sp_entry);
    true
}
