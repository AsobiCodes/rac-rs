//! Moby slot reset: releases the moby's voice slot when that slot still belongs to the moby and is active, clears the
//! slot index in the moby's state block, resets the moby's pose and starts cue 1 on it through the voice allocator.
use crate::h;
use crate::ps2::*;
use crate::{actor, euler_basis, voice_slot};
use ee_oracle::cpu::Cpu;

/// MIPS register numbers of the saved registers the frame keeps.
const S0: usize = 16;
const S1: usize = 17;
const S2: usize = 18;
const S3: usize = 19;
const RA: usize = 31;

/// Frame size and the offsets of the saved registers in it.
const FRAME_SIZE: u32 = 0x90;
const POSE_OUT_OFF: u32 = 0x20;
const VEC_OFF: u32 = 0x30;

/// Moby field: pointer to the moby's state block.
const MOBY_STATE: u32 = 0x78;
/// State block field: index of the voice slot the moby holds (-1 when none).
const STATE_SLOT: u32 = 0x238;
const NO_SLOT: u32 = u32::MAX;

/// Voice slot table (0x13E550) with 0x70-byte records: active-state byte at +0x74, owner moby pointer at +0x88.
const SLOT_TABLE: u32 = 0x0014_0000 - 0x1ab0;
const SLOT_SIZE: u32 = 0x70;
const SLOT_ACTIVE: u32 = 0x74;
const SLOT_OWNER: u32 = 0x88;

/// Retail helpers: 0x249d38(moby, 0, -1) and 0x251010 (the cue-setup call with its stack words and float arguments).
const FN_249D38: u32 = 0x249d38;
const FN_251010: u32 = 0x251010;
/// Cue started on the moby by the reset.
const RESET_CUE: u32 = 1;

/// 0x2c1698 moby_slot_reset(moby a0) -> v0: releases the moby's voice slot if it still owns an active one, clears
/// the slot index, resets the pose, runs the cue setup and returns the new slot index from moby_cue.
pub fn moby_slot_reset(c: &mut Cpu) -> bool {
    let moby = c.arg(0);
    let saves = [(S0, 0x40, h::SQ), (S1, 0x50, h::SQ), (S2, 0x60, h::SQ), (S3, 0x70, h::SQ), (RA, 0x80, h::SQ)];
    h::frame(c, FRAME_SIZE, &saves, |c, sp| {
        let out = sp + VEC_OFF;
        h::run(c, euler_basis::zero_vec4, &[out], &[]);

        let state = c.r32(moby + MOBY_STATE);
        let slot = c.r32(state + STATE_SLOT);
        if slot != NO_SLOT {
            let record = SLOT_TABLE.wrapping_add(slot.wrapping_mul(SLOT_SIZE));
            if c.r32(record + SLOT_OWNER) == moby && c.r8(record + SLOT_ACTIVE) != 0 {
                h::run(c, voice_slot::voice_release, &[slot], &[]);
            }
        }
        c.w32(state + STATE_SLOT, NO_SLOT);

        h::call(c, FN_249D38, &[moby, 0, NO_SLOT], &[]);
        h::run(c, actor::pose_node_pos, &[moby, 5, sp + POSE_OUT_OFF], &[]);

        c.w32(sp, 1);
        c.w32(sp + 8, NO_SLOT);
        c.w32(sp + 0x10, 0);
        let floats = [F(0), F(0), F(0x4000_0000), F(0x3f80_0000), F(0x47c3_5000), F(0x3f80_0000), F(0x4170_0000)];
        h::call(c, FN_251010, &[moby, out, sp + POSE_OUT_OFF, 10, 3, 4, NO_SLOT, 1], &floats);

        let (slot_id, _) = h::run(c, voice_slot::moby_cue, &[RESET_CUE, 0, moby], &[]);
        c.ret(slot_id);
    });
    true
}
