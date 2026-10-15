//! Aim-turn step over the EE state block at 0x13f4c0, and the slot-table pair store at 0x13f350.
use ee_oracle::cpu::Cpu;
use crate::h::{self, SQ};
use crate::ps2::{Ee, F};
use crate::{glove_slot, motion, vu_geom, vu_vec};

/// State block of the aim-turn code: 0x140000 - 0xb40.
const BLOCK: u32 = 0x0014_0000 - 0x0b40;
/// Retail helper that finishes the step (not rewritten).
const FINISH: u32 = 0x0020_4da0;
/// Retail hero yaw turn; its scratch registers after the call are kept as the retail callee leaves them.
const HERO_YAW_TURN: u32 = 0x0020_4ed8;
/// Retail allocator called by the pair store (not rewritten).
const ALLOC: u32 = 0x0027_e470;
/// Slot table base: 0x140000 - 0xcb0.
const SLOT_TABLE: u32 = 0x0014_0000 - 0x0cb0;
/// Pair-A table offset from the slot table (word per index).
const PAIR_A: u32 = 0x2218;
/// Pair-B table offset from the slot table (word per index).
const PAIR_B: u32 = 0x2238;
/// Pair-A entry meaning "not yet allocated".
const UNSET: u32 = 0xffff_ffff;
/// Slot value that selects the first scale pair.
const SLOT_MATCH: u32 = 0x10;
/// Address of the float constant at gp - 0x7e9c is read through gp.
const GP_FLOAT_OFF: u32 = 0x7e9c;
/// Float constant base (lui 0x16) minus 0x1294.
const CONST_BASE: u32 = 0x0016_0000;
const CONST_OFF: u32 = 0x1294;

/// 0x204e20 aim_turn_step(ctx, x, y, z) -> void: maps the direction (x, y, z) through the matrix at 0x13f4c0,
/// takes its yaw, springs the angle state toward it (stored at state+0x18), then calls the finisher at 0x204da0.
pub fn aim_turn_step(c: &mut Cpu) -> bool {
    let ctx = c.arg(0);
    let (f22, f23, f21) = (c.farg(0), c.farg(1), c.farg(2));
    let saves = [(16, 0x20, SQ), (17, 0x30, SQ), (31, 0x40, SQ)];
    h::frame(c, 0x70, &saves, |c, sp| {
        c.wf(sp + 0x68, F(c.f[23]));
        c.wf(sp + 0x60, F(c.f[22]));
        c.wf(sp + 0x58, F(c.f[21]));
        c.wf(sp + 0x50, F(c.f[20]));
        h::run(c, vu_vec::vec4_mul_mat4, &[sp, BLOCK, BLOCK - 0x130], &[]);
        let (ax, ay) = (c.rf(sp), c.rf(sp + 4));
        let angle = h::run(c, vu_geom::fast_atan2, &[], &[ax, ay]).1;
        c.wf(sp + 0x10, F(0));
        let turn = h::run_keep(c, motion::turn_spring_angle, &[sp + 0x10, BLOCK + 0x14, ctx], &[angle, f22, f23, f21]);
        c.wf(BLOCK + 0x18, turn.1);
        let f14 = c.rf(sp + 0x10);
        let a0 = c.arg(0);
        h::call_keep(c, FINISH, &[a0], &[F(0), F(0), f14]);
    });
    true
}

/// 0x204f40 yaw_turn_by_slot_b() -> void: reads slot value B; picks one of two scale pairs (by whether it is 16) and
/// calls hero_yaw_turn with the scaled components.
pub fn yaw_turn_by_slot_b(c: &mut Cpu) -> bool {
    let gp = c.g(28) as u32;
    h::frame(c, 0x10, &[(31, 0, SQ)], |c, _sp| {
        let slot = h::run(c, glove_slot::slot_value_b_fn, &[0], &[]).0;
        let f2 = c.rf(gp.wrapping_sub(GP_FLOAT_OFF));
        let (k1, k0) = if slot == SLOT_MATCH {
            (0x3bc4_9ba6, 0x415f_66f3)
        } else {
            (0x3b03_126f, 0x40df_66f3)
        };
        let f1 = F(k1);
        let f3 = F(0x3d8f_5c29);
        let f4 = c.rf(CONST_BASE - CONST_OFF);
        let f0 = F(k0);
        c.f[0] = f0.0;
        c.f[1] = f1.0;
        c.f[2] = f2.0;
        c.f[3] = f3.0;
        c.f[4] = f4.0;
        c.s(1, SLOT_MATCH as u64);
        h::call_keep(c, HERO_YAW_TURN, &[0], &[f2 * f1, f2 * f3, f4 * f0]);
    });
    true
}

/// 0x209f88 set_slot_pair(index, value, ptr) -> void: allocates pair A[index] through 0x27e470 when it is unset, then
/// stores value into pair B[index] of the slot table at 0x13f350.
pub fn set_slot_pair(c: &mut Cpu) -> bool {
    let (index, value, ptr) = (c.arg(0), c.arg(1), c.arg(2));
    let saves = [(16, 0x00, SQ), (17, 0x10, SQ), (18, 0x20, SQ), (19, 0x30, SQ), (31, 0x40, SQ)];
    h::frame(c, 0x50, &saves, |c, _sp| {
        let s2 = index.wrapping_shl(2);
        let s3 = SLOT_TABLE;
        let s0 = s3.wrapping_add(PAIR_A).wrapping_add(s2);
        c.s(3, u64::MAX);
        if c.r32(s0) == UNSET {
            let v = h::call_keep(c, ALLOC, &[ptr, 4, value], &[]).0;
            c.w32(s0, v);
        }
        let base = s3.wrapping_add(s2);
        c.s(2, base as i32 as i64 as u64);
        c.w32(base.wrapping_add(PAIR_B), value);
    });
    true
}
