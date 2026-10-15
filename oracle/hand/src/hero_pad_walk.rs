//! Hero pad-walk helpers (retail 0x226388..0x227f3c): the flag clear, the pad-walk start, the hero-state query
//! and the per-frame pad-walk tick. Game-state table at 0x13f350; hero state word at 0x1413d4.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const SP: usize = 29;
const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const S0: usize = 16;
const S1: usize = 17;
const RA: usize = 31;

/// Base of the game-state table (retail `lui 0x14` plus `addiu -0xcb0`).
const STATE_BASE: u32 = 0x0013_f350;
/// Pointer to the first flag object, at base+0x2080.
const OBJ_A_PTR: u32 = STATE_BASE + 0x2080;
/// Pointer to the second flag object (0 when unset), at base+0x1620.
const OBJ_B_PTR: u32 = STATE_BASE + 0x1620;
/// Halfword flags inside a flag object; bit 0 is cleared.
const OBJ_FLAGS: u32 = 0x34;
/// Hero state word (base+0x2084 = 0x1413d4).
const HERO_STATE: u32 = STATE_BASE + 0x2084;
/// 16-byte pad position block.
const PAD_POS: u32 = 0x0014_0990;
/// Float and mode words written by pad_walk_begin.
const PAD_F: u32 = 0x164c;
const PAD_MODE: u32 = 0x1650;
/// Float at base+0x88 (probed height) and at base+0x98 (walk value).
const PAD_PROBE_F: u32 = 0x88;
const PAD_WALK_F: u32 = 0x98;
/// Hero states that start a pad walk (0x65..=0x67).
const WALK_STATE_FIRST: u32 = 0x65;
const WALK_STATE_COUNT: u32 = 3;
/// Retail routine started for a non-walk hero state.
const PAD_WALK_START: u32 = 0x0021_5538;
/// Retail helpers used by the tick.
const VEC_ADD: u32 = 0x001f_1810;
/// Threshold 0.3 and half a unit.
const PAD_THRESHOLD: u32 = 0x3e99_999a;
const HALF: u32 = 0x3f00_0000;
/// -0.75 (vector scale).
const NEG_THREE_QUARTERS: u32 = 0xbf40_0000;
/// Offset of the third word of the working vector on the stack.
const VEC_Z: u32 = 8;

fn set_gpr(c: &mut Cpu, n: usize, v: u32) {
    c.r[n] = v as i32 as i128 as u128;
}

fn write_quad(c: &mut Cpu, a: u32, v: u128) {
    for i in 0..4u32 {
        c.w32(a + 4 * i, (v >> (32 * i)) as u32);
    }
}

fn read_quad(c: &mut Cpu, a: u32) -> u128 {
    let mut v = 0u128;
    for i in 0..4u32 {
        v |= (c.r32(a + 4 * i) as u128) << (32 * i);
    }
    v
}

/// 0x226388 clear_pad_flags(): clears bit 0 of the halfword at +0x34 of the object at base+0x2080, then of the one at
/// base+0x1620 when that pointer is set. Returns v0 = the new flags of the second object, or the table base.
pub fn clear_pad_flags(c: &mut Cpu) -> bool {
    let a = c.r32(OBJ_A_PTR);
    let flags = c.r16(a + OBJ_FLAGS) & 0xfffe;
    c.w16(a + OBJ_FLAGS, flags);
    let b = c.r32(OBJ_B_PTR);
    set_gpr(c, A0, a);
    set_gpr(c, V1, b);
    if b != 0 {
        let v = c.r16(b + OBJ_FLAGS) & 0xfffe;
        c.w16(b + OBJ_FLAGS, v);
        c.ret(v as u32);
    } else {
        c.ret(STATE_BASE);
    }
    true
}

/// 0x227028 pad_walk_begin(src, mode, f): copies the 16-byte block at a0 to the pad position block, stores f and a1
/// in the table, and unless the hero state is 0x65..0x67 runs the retail start routine 0x215538(0x65, 1).
pub fn pad_walk_begin(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let frame = (c.g(SP) as u32).wrapping_sub(0x10);
    let ra = c.r[RA];
    write_quad(c, frame, ra);
    let src = c.rv(c.arg(0));
    c.wv(PAD_POS, src);
    c.wf(STATE_BASE + PAD_F, c.farg(0));
    c.w32(STATE_BASE + PAD_MODE, c.arg(1));
    let state = c.r32(HERO_STATE);
    set_gpr(c, V1, STATE_BASE);
    if state.wrapping_sub(WALK_STATE_FIRST) < WALK_STATE_COUNT {
        c.ret(1);
    } else {
        let _ = h::call_keep(c, PAD_WALK_START, &[0x65, 1], &[]);
    }
    true
}

/// 0x227078 pad_walk_status() -> v0: hero state 0x65 or 0x66 gives 1, 0x67 gives 2, other states 0. v1 is 1 when
/// the state is below 0x65.
pub fn pad_walk_status(c: &mut Cpu) -> bool {
    let state = c.r32(HERO_STATE);
    let s = state as i32;
    set_gpr(c, A0, state);
    let (v0, v1) = if s < 0x65 {
        (0, 1)
    } else if s < 0x67 {
        (1, 0)
    } else if s == 0x67 {
        (2, 0)
    } else {
        (0, 0)
    };
    set_gpr(c, V1, v1);
    c.ret(v0);
    true
}

/// 0x227e80 pad_walk_tick(): takes the base vector scaled by -0.75 plus the one at base+0x80, raises its third word by
/// 0.5 and replaces it with the ground height there. When the probed height at base+0x88 is within 0.3 of it, starts
/// pad_walk_begin(base+0x80, 1, base+0x98) and copies the result block back to base+0x80.
pub fn pad_walk_tick(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let outer_sp = c.r[SP];
    let sp = (c.g(SP) as u32).wrapping_sub(0x40);
    // The callees run on the new frame, as after retail's `addiu sp, -0x40`.
    set_gpr(c, SP, sp);
    let s0 = STATE_BASE;
    let s1 = STATE_BASE + 0x80;
    let saved_s0 = c.r[S0];
    let saved_s1 = c.r[S1];
    let saved_ra = c.r[RA];
    write_quad(c, sp + 0x10, saved_s0);
    write_quad(c, sp + 0x20, saved_s1);
    write_quad(c, sp + 0x30, saved_ra);

    h::run_keep(c, crate::vecmath::vec_scale, &[sp, s0], &[F(NEG_THREE_QUARTERS)]);
    let _ = h::call_keep(c, VEC_ADD, &[sp, sp, s1], &[]);

    let raised = c.rf(sp + VEC_Z) + F(HALF);
    c.wf(sp + VEC_Z, raised);
    h::run_keep(c, crate::sweep::ground_height, &[sp, 0], &[F(HALF)]);
    let ground = c.fresult();
    c.wf(sp + VEC_Z, ground);

    let diff = c.rf(STATE_BASE + PAD_PROBE_F) - ground;
    h::run_keep(c, crate::motion::fabs, &[], &[diff]);
    c.f[1] = PAD_THRESHOLD;
    set_gpr(c, 1, PAD_THRESHOLD);
    let dist = c.fresult();
    if !(F(PAD_THRESHOLD) < dist) {
        let walk = c.rf(STATE_BASE + PAD_WALK_F);
        h::run_keep(c, pad_walk_begin, &[s1, 1], &[walk]);
        let quad = read_quad(c, sp);
        c.r[V0] = quad;
        let block = c.rv(sp);
        c.wv(s1, block);
    }
    c.r[SP] = outer_sp;
    true
}
