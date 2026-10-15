//! Weighted random pick over the five-slot table at 0x179990 (used by the glove/slot selector).
use crate::ps2::*;
use crate::{glove_slot, h, mathx, rand};
use ee_oracle::cpu::Cpu;

/// Global state block (`lui 0x14` then `-0xcb0`).
const STATE: u32 = 0x0013_F350;
/// Slot table: five entries of `SLOT_SIZE` bytes.
const TABLE: u32 = 0x0017_9990;
const SLOT_SIZE: u32 = 0x70;
const SLOT_COUNT: u32 = 5;
/// Word that must be zero for the mode-1 path (`0x180000 - 0x6450`).
const BLOCK_WORD: u32 = 0x0017_9BB0;
/// Word that must equal 12 for slot 3 (`0x160000 - 0x127c`).
const PHASE_WORD: u32 = 0x0015_ED84;
/// Retail routine that scores a candidate (0x1e24d8).
const SCORE_FN: u32 = 0x001e_24d8;

const OFF_GATE: u32 = 0x1e8;
const OFF_MODE: u32 = 0x22a8;
const OFF_FLAGS: u32 = 0xa98;
const OFF_QUAD: u32 = 0xd0;
const OFF_2080: u32 = 0x2080;
const OFF_2FC: u32 = 0x2fc;
const OFF_2094: u32 = 0x2094;
const OFF_198: u32 = 0x198;

/// Slot field offsets.
const SLOT_BUSY: u32 = 0x60;
const SLOT_WEIGHT: u32 = 0x58;
const SLOT_KEY: u32 = 0x50;
const SLOT_FLAG: u32 = 0x44;

/// Stack frame size of the retail routine; its scratch lists sit at the post-prologue sp.
const FRAME: u32 = 0x130;

const ONE: F = F(0x3f80_0000);
const EIGHT: F = F(0x4100_0000);
const SIXTY: F = F(0x4270_0000);
/// Threshold for the mode-1 random gate (about 0.33).
const MODE1_THRESHOLD: F = F(0x3ea8_f5c3);

fn done(c: &mut Cpu, v: u32) -> bool {
    c.ret(v);
    true
}

fn slot_value(c: &mut Cpu) -> u32 {
    h::run(c, glove_slot::slot_value_b_fn, &[0], &[]).0
}

/// 1 / (float)(int)(scale * weight * 60) for table slot `idx`.
fn inv_weight(c: &mut Cpu, idx: u32) -> F {
    let weight = F(c.r32(TABLE + idx * SLOT_SIZE + SLOT_WEIGHT));
    let (_, scaled) = h::run(c, mathx::scale_float, &[], &[weight]);
    let steps = (scaled * SIXTY).to_int();
    ONE / F::from_int(steps)
}

/// Copies the 16-byte quadword at `src` to `dst`.
fn copy_quad(c: &mut Cpu, dst: u32, src: u32) {
    for w in 0..4 {
        let v = c.r32(src + 4 * w);
        c.w32(dst + 4 * w, v);
    }
}

/// Decides whether slot `i` is recorded as a candidate.
fn eligible(c: &mut Cpu, i: u32, frame: u32) -> bool {
    let e = TABLE + i * SLOT_SIZE;
    if c.r32(e + SLOT_BUSY) != 0 {
        return false;
    }
    if F(c.r32(e + SLOT_WEIGHT)) == F(0) {
        return false;
    }
    let v = slot_value(c) as i32;
    let key = c.r32(e + SLOT_KEY) as i32;
    if key != -1 && key != v {
        return false;
    }
    if c.r32(e + SLOT_FLAG) != 0 && c.r32(STATE + OFF_2FC) != 0 {
        return false;
    }
    if i == 3 {
        if c.r32(PHASE_WORD) != 12 {
            return false;
        }
        if slot_value(c) == 0x17 {
            return false;
        }
        copy_quad(c, frame + 0x60, STATE + OFF_QUAD);
        copy_quad(c, frame + 0x50, frame + 0x60);
        let adjusted = F(c.r32(frame + 0x58)) + EIGHT;
        c.w32(frame + 0x58, adjusted.0);
        let args = [frame + 0x60, frame + 0x50, 2, c.r32(STATE + OFF_2080), 0];
        let (score, _) = h::call(c, SCORE_FN, &args, &[]);
        if score != 0 {
            return false;
        }
    }
    if i != 4 {
        return true;
    }
    if c.r32(STATE + OFF_2094) != 1 {
        return false;
    }
    let (n, _) = h::run(c, mathx::float_to_int_scaled, &[50], &[]);
    if (n as i32) < c.r32(STATE + OFF_198) as i32 {
        return false;
    }
    (c.r32(STATE + OFF_MODE) as i32) < 2
}

/// 0x21c150 weighted_pick(out: *mut i32) -> bool: picks one table slot at random, weighted by its scaled weight,
/// stores its index to *out and returns 1 (returns 0 when nothing is picked; mode 1 writes 4 and rolls a gate).
pub fn weighted_pick(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    if c.r16(STATE + OFF_GATE) as i16 != 0 {
        return done(c, 0);
    }
    if c.r32(STATE + OFF_MODE) as i32 == 1 {
        c.w32(out, 4);
        if c.r32(BLOCK_WORD) != 0 || c.r32(STATE + OFF_FLAGS) & 2 == 0 {
            return done(c, 0);
        }
        let (_, roll) = h::run(c, rand::rand_float, &[], &[F(0), ONE]);
        return done(c, (roll < MODE1_THRESHOLD) as u32);
    }

    let frame = c.g(29) as u32 - FRAME;
    let list = frame;
    let mut count = 0;
    for i in 0..SLOT_COUNT {
        if eligible(c, i, frame) {
            c.w32(list + 4 * count, i);
            count += 1;
        }
    }
    if count == 0 {
        return done(c, 0);
    }

    let mut sum = F(0);
    for k in 0..count {
        let idx = c.r32(list + 4 * k);
        sum = sum + inv_weight(c, idx);
    }
    let (_, roll) = h::run(c, rand::rand_float, &[], &[F(0), ONE]);
    if !(roll < sum) {
        return done(c, 0);
    }
    for k in 0..count {
        let idx = c.r32(list + 4 * k);
        c.w32(out, idx);
        sum = sum - inv_weight(c, idx);
        if sum < roll {
            return done(c, 1);
        }
    }
    done(c, 1)
}
