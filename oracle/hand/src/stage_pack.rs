//! Staging-buffer pack step: a per-stage enable word chooses between a bit-pack of the buffer
//! and a zero fill, and a pack result is folded into a per-level maximum slot.

use crate::h::{call_keep, run_keep, unwind_pending};
use crate::{mat4_rle, mathx, stage_dispatch};
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Staging state block (v0 in retail); the fields used are at +0x0c, +0x14 and +0x28.
const STATE_BASE: u32 = 0x18_42F0;
const STATE_A3_OFF: u32 = 0x0C;
const STATE_A2_OFF: u32 = 0x14;
const STATE_ENABLE_OFF: u32 = 0x28;
/// Pack-enable word: non-zero packs, zero fills. Read by both entries.
const PACK_ENABLE: u32 = 0x18_4318;
/// v0 value left by the `lui 0x18` in stage_pack_entry.
const OVERLAY_HI: u32 = 0x18_0000;
/// Byte count used by both the fill and the pack.
const BUF_BYTES: u32 = 0x800;
/// Value rle_bits_pack returns when it cannot pack.
const PACK_FAILED: u32 = 0xFFFF_FFFF;
/// Retail helper run on a failed pack, and the level word it is followed by.
const FAIL_HELPER: u32 = 0x23_C920;
const LEVEL_WORD: u32 = 0x15_ED84;
/// Gp-relative word holding the slot index used after a successful pack.
const GP_INDEX_OFF: i32 = -0x7E7C;
/// Per-index signed maximum table: slot = table + index * 4.
const SLOT_TABLE: u32 = 0x13_D560;
/// Retail callees of stage_pack_entry.
const FLAG_BRANCH_TARGET: u32 = 0x23_C518;
const FLAG_FALLBACK_TARGET: u32 = 0x23_C748;

/// Sets GPR `r` to a 32-bit value, sign-extended as the MIPS 32-bit ops leave it.
fn set_gpr(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x23c3f8 stage_pack_or_fill(a0): runs the stage hook, then if the enable word is zero fills the
/// 0x800-byte buffer at a0 with zeros. Otherwise packs it with rle_bits_pack and, unless the pack
/// failed (-1, which first runs 0x23c920), raises the slot picked by the gp index to the pack result
/// when the slot is smaller (signed). v0 is left as the last compare or callee wrote it.
pub fn stage_pack_or_fill(c: &mut Cpu) -> bool {
    if unwind_pending(c) {
        return false;
    }
    let buf = c.arg(0);
    run_keep(c, stage_dispatch::nop_hook, &[], &[]);
    set_gpr(c, 2, STATE_BASE);
    let enable = c.r32(PACK_ENABLE);
    set_gpr(c, 3, enable);
    if enable == 0 {
        run_keep(c, mathx::fill_words, &[buf, 0, BUF_BYTES], &[]);
        return true;
    }
    let pack_a3 = c.r32(STATE_BASE + STATE_A3_OFF);
    set_gpr(c, 7, pack_a3);
    let pack_in = c.r32(STATE_BASE + STATE_A2_OFF);
    let (packed, _) = run_keep(c, mat4_rle::rle_bits_pack, &[buf, BUF_BYTES, pack_in], &[]);

    let gp_index = c.r32((c.g(28) as u32).wrapping_add(GP_INDEX_OFF as u32));
    let index = if packed == PACK_FAILED {
        call_keep(c, FAIL_HELPER, &[buf], &[]);
        c.r32(LEVEL_WORD)
    } else {
        gp_index
    };
    let slot = SLOT_TABLE.wrapping_add(index << 2);
    set_gpr(c, 3, SLOT_TABLE);
    set_gpr(c, 4, slot);
    let old = c.r32(slot);
    let raise = (old as i32) < (packed as i32);
    set_gpr(c, 2, raise as u32);
    if raise {
        c.w32(slot, packed);
    }
    true
}

/// 0x23c4a0 stage_pack_entry(a0, a1, a2): runs the stage hook, then when the enable word is set
/// calls 0x23c518(a0, a1, a2) if bit 0 of the byte at a1 is set, else 0x23c748(a0, a1).
pub fn stage_pack_entry(c: &mut Cpu) -> bool {
    if unwind_pending(c) {
        return false;
    }
    let a0 = c.arg(0);
    let a1 = c.arg(1);
    let a2 = c.arg(2);
    run_keep(c, stage_dispatch::nop_hook, &[], &[]);
    let enable = c.r32(PACK_ENABLE);
    set_gpr(c, 2, OVERLAY_HI);
    set_gpr(c, 3, enable);
    if enable == 0 {
        return true;
    }
    let flag = (c.r8(a1) as u32) & 1;
    set_gpr(c, 2, flag);
    set_gpr(c, 4, a0);
    set_gpr(c, 5, a1);
    if flag != 0 {
        set_gpr(c, 6, a2);
        call_keep(c, FLAG_BRANCH_TARGET, &[a0, a1, a2], &[]);
    } else {
        call_keep(c, FLAG_FALLBACK_TARGET, &[a0, a1], &[]);
    }
    true
}
