//! Moby hooks: a held moby's fire-button latch, a float store to the moby and its sub-object, and a pose probe that
//! feeds one retail call with fixed tuning values.
use crate::actor;
use crate::euler_basis;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Scalar registers by number (MIPS).
const V0: usize = 2;
const V1: usize = 3;
const A1: usize = 5;
const A2: usize = 6;
const A0: usize = 4;

/// Moby word: pointer to the moby's state block.
const MOBY_STATE: u32 = 0x78;
/// State block word: pointer to the sub-object (0 when none).
const SUB_OBJ: u32 = 0x424;
/// 0x2b4160: called when the state block's word at +4 is set.
const STATE_CALL: u32 = 0x2b4160;
/// 0x251010: the tuning call of `held_pose_probe`.
const TUNE_CALL: u32 = 0x25_1010;
/// Overlay data page (`lui 0x14`); its words are read from RAM at call time.
const DATA_HI: u32 = 0x14_0000;
/// Word at DATA_HI - 0x3520: first operand of the fire-button test.
const FLAG_WORD_LO: u32 = 0x3520;
/// Word at DATA_HI + 0x3f0: mask of the fire-button test.
const MASK_LO: u32 = 0x3f0;
/// Moby byte written with the fire-button flag.
const FIRE_FLAG: u32 = 0xbc;

/// Sign-extends a 32-bit value into a 64-bit register like lw / addiu do.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// 0x2b4100 held_fire_latch(moby): when the state block's word at +4 is set, calls 0x2b4160 with the moby; otherwise
/// sets moby+0xbc to 1 when (word 0x13cae0 & word 0x1403f0) is nonzero, else to 0, and returns that flag in v0.
pub fn held_fire_latch(c: &mut Cpu) -> bool {
    let moby = c.arg(0);
    h::frame(c, 0x10, &[(31, 0x0, h::SQ)], |c, _sp| {
        c.s(A2, moby as u64);
        let state = c.r32(moby.wrapping_add(MOBY_STATE));
        let sub = c.r32(state.wrapping_add(4));
        if sub != 0 {
            h::call_keep(c, STATE_CALL, &[moby], &[]);
        } else {
            c.s(A0, DATA_HI as u64);
            c.s(A1, DATA_HI as u64);
            let flag_word = c.r32(DATA_HI - FLAG_WORD_LO);
            let mask = c.r32(DATA_HI + MASK_LO);
            let on = (flag_word & mask) != 0;
            c.s(V1, sx(mask));
            // retail's `addiu v0, zero, 1` is the delay slot of the beqz: v0 is 1 on both sides
            c.s(V0, 1);
            c.w8(moby.wrapping_add(FIRE_FLAG), on as u8);
        }
    });
    true
}

/// 0x2b9d70 moby_store_f58(moby, f): stores f at moby+0x58; when the state block's sub-object is set, stores f at its
/// +0x58 too. v0 is the sub-object pointer (0 when none).
pub fn moby_store_f58(c: &mut Cpu) -> bool {
    let moby = c.arg(0);
    let f = c.farg(0);
    c.wf(moby.wrapping_add(0x58), f);
    let state = c.r32(moby.wrapping_add(MOBY_STATE));
    let sub = c.r32(state.wrapping_add(SUB_OBJ));
    c.ret(sub);
    if sub != 0 {
        c.wf(sub.wrapping_add(0x58), f);
    }
    true
}

/// 0x2b9d98 held_pose_probe(moby): zeroes a 16-byte vector, takes pose node 6 of the moby into a second one, then calls
/// 0x251010 with both, fixed ints (10, 3, 4, -1, 1), floats (0, 0, 2, 1, 25000, 1.5, 15) and stack words (1, -1, 0).
pub fn held_pose_probe(c: &mut Cpu) -> bool {
    let moby = c.arg(0);
    let saves = [(17, 0x50, h::SQ), (16, 0x40, h::SQ), (31, 0x60, h::SQ)];
    h::frame(c, 0x70, &saves, |c, sp| {
        let vec = sp.wrapping_add(0x30);
        let pos = sp.wrapping_add(0x20);
        h::run_keep(c, euler_basis::zero_vec4, &[vec], &[]);
        h::run_keep(c, actor::pose_node_pos, &[moby, 6, pos], &[]);
        c.w32(sp, 1);
        c.w32(sp.wrapping_add(8), 0xffff_ffff);
        c.w32(sp.wrapping_add(0x10), 0);
        let ints = [moby, vec, pos, 10, 3, 4, 0xffff_ffff, 1];
        let floats = [F(0), F(0), F(0x4000_0000), F(0x3f80_0000), F(0x47c3_5000), F(0x3fc0_0000), F(0x4170_0000)];
        h::call_keep(c, TUNE_CALL, &ints, &floats);
    });
    true
}
