//! Orb and Suck Cannon helpers from the level 4 overlay (rec/l4_19).

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const GP: usize = 28;
const SP: usize = 29;
const RA: usize = 31;

/// Global pointer offsets of the two rotation angles (f12) used by `orb_vectors_spin`.
const GP_SPIN_ANGLE: i32 = -0x5140;
const GP_SPIN_SCALE: i32 = -0x511c;

/// Shared sparkle vector table: the first vector and the matching output table.
const SPARKLE_VECTORS: u32 = 0x001d_62b0;
const SPARKLE_OUT: u32 = 0x001d_6330;
const SPARKLE_COUNT: u32 = 8;
const SPARKLE_STRIDE: u32 = 0x10;

/// Retail callee that rotates one sparkle vector into the output table.
const ROTATE_SPARKLE: u32 = 0x0025_27c8;
/// Retail `vu_vec::vec_set_length`.
const VEC_SET_LENGTH: u32 = 0x001f_1a98;
/// Stack frame of `orb_vectors_spin`.
const FRAME_SIZE: u32 = 0x40;
/// Stack frame of `orb_pickup_los`.
const FRAME_SIZE_LOS: u32 = 0x30;

/// Hero-state block: the orb pickup reads its position pointer from `BLOCK + 0x13d0`.
const BLOCK: u32 = 0x0014_0000;
const BLOCK_BASE: u32 = BLOCK - 0x0cb0;
const BLOCK_STATE: u32 = 0x208c;
const BLOCK_MODE: u32 = 0x2084;
const ORB_PTR: u32 = 0x0000_13d0;
const ORB_OFFSET: u32 = 0x10;
const LOS_CHECK: u32 = 0x001e_24d8;
const HALF: u32 = 0x3f00_0000;

/// Suck Cannon jump table (one word per state, indices 0..12).
const SUCK_TABLE: u32 = 0x001e_0000 - 0x3b80;
const SUCK_STATES: u32 = 0xd;
const TARGET_CLEAR: u32 = 0x002d_bc68;
const TARGET_MODE: u32 = 0x002d_bc70;

/// Stores a 128-bit register as SQ does (the low four address bits are ignored).
fn store_q(c: &mut Cpu, addr: u32, v: u128) {
    let a = addr & !0xf;
    for i in 0..4 {
        c.w32(a + 4 * i, (v >> (32 * i)) as u32);
    }
}

/// Loads a 128-bit value as LQ does.
fn load_q(c: &mut Cpu, addr: u32) -> u128 {
    let a = addr & !0xf;
    (0..4).fold(0u128, |acc, i| acc | (c.r32(a + 4 * i) as u128) << (32 * i))
}

/// 0x2da3c0 orb_vectors_spin(): rotates the 8 shared sparkle vectors by one step (4 degrees per call).
pub fn orb_vectors_spin(c: &mut Cpu) -> bool {
    let gp = c.g(GP) as u32;
    // Retail frame: sq s0/s1/s2/ra, the callees run with sp = frame (their own frames sit below it).
    let saves = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (18, 0x20, h::SQ), (RA, 0x30, h::SQ)];
    h::frame(c, FRAME_SIZE, &saves, |c, _sp| {
        let mut vec = SPARKLE_VECTORS;
        let mut out = SPARKLE_OUT;
        for _ in 0..SPARKLE_COUNT {
            let angle = c.rf(gp.wrapping_add(GP_SPIN_ANGLE as u32));
            h::call_keep(c, ROTATE_SPARKLE, &[vec, vec, out], &[angle]);
            out = out.wrapping_add(SPARKLE_STRIDE);
            let scale = c.rf(gp.wrapping_add(GP_SPIN_SCALE as u32));
            h::call_keep(c, VEC_SET_LENGTH, &[vec, vec], &[scale]);
            vec = vec.wrapping_add(SPARKLE_STRIDE);
        }
    });
    true
}

/// 0x2da438 orb_pickup_los(a0 = orb quad) -> bool: lifts the orb's height by 0.5 and returns 1 when the line-of-sight
/// check at 0x1e24d8 finds nothing in the way (its result is 0).
pub fn orb_pickup_los(c: &mut Cpu) -> bool {
    let half = F(HALF);
    h::frame(c, FRAME_SIZE_LOS, &[(RA, 0x20, h::SQ)], |c, frame| {
        let src = c.arg(0).wrapping_add(ORB_OFFSET);
        let q0 = load_q(c, src);
        store_q(c, frame, q0);
        let lifted = c.rf(frame + 8) + half;
        c.wf(frame + 8, lifted);

        let orb_ptr = c.r32(BLOCK + ORB_PTR).wrapping_add(ORB_OFFSET);
        let q1 = load_q(c, orb_ptr);
        store_q(c, frame + 0x10, q1);
        let check_y = c.rf(frame + 0x18) + half;
        c.wf(frame + 0x18, check_y);

        // Registers the retail call leaves for its callee: at = 0.5 (lui), v0 = quad, v1 = orb pointer, f0/f1.
        c.s(1, HALF as u64);
        c.s(3, orb_ptr as u64);
        c.r[2] = q1;
        c.f[0] = check_y.0;
        c.f[1] = half.0;
        let (result, _) = h::call_keep(c, LOS_CHECK, &[frame, frame + 0x10, 0x12, 0, 0], &[]);
        c.ret((result == 0) as u32);
    });
    true
}

/// 0x2dbc30 suck_hero_blocked() -> bool: 1 when the Suck Cannon state allows the hero group, else 0.
pub fn suck_hero_blocked(c: &mut Cpu) -> bool {
    let state = c.r32(BLOCK_BASE + BLOCK_STATE);
    c.s(6, BLOCK as u64);
    let mut a1 = 1u32;
    let mut v1 = 0u32;
    let mut a0 = state;
    if state < SUCK_STATES {
        v1 = SUCK_TABLE.wrapping_add(state * 4);
        let target = c.r32(v1);
        a0 = target;
        match target {
            TARGET_CLEAR => a1 = 0,
            TARGET_MODE => {
                v1 = BLOCK_BASE;
                if (c.r32(BLOCK_BASE + BLOCK_MODE) ^ 0x3c) != 0 {
                    a1 = 0;
                }
            }
            _ => {}
        }
    }
    c.s(4, a0 as i32 as i64 as u64);
    c.s(3, v1 as i32 as i64 as u64);
    c.ret(a1);
    true
}
