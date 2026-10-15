//! Scene chunk table walk: compares neighbouring chunk records and latches a camera request when the next one is larger.

use crate::cam_latch;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const REG_V0: usize = 2;
const REG_V1: usize = 3;
const REG_A0: usize = 4;
const REG_A1: usize = 5;
const REG_A2: usize = 6;
const REG_A3: usize = 7;

/// 0x170000 - 0x36a0: base of the scene chunk state block.
const STATE: u32 = 0x0016_c960;
/// State word holding the camera id passed to the latch request.
const STATE_CAMERA: u32 = 0x5c;
/// State word holding the record index base (multiplied by the record stride).
const STATE_RECORD: u32 = 0x30;
/// 0x160000 - 0x1280: mode word; nonzero selects table A.
const MODE_WORD: u32 = 0x0015_ed80;
/// Byte stride of one record in the walked table.
const RECORD_STRIDE: u32 = 0x250;
/// Table base for mode A (0x137b80 + 0x2c18).
const TABLE_A: u32 = 0x0013_a798;
/// Table base for mode B (0x137b80 + 0x2afc).
const TABLE_B: u32 = 0x0013_a67c;
/// Retail routine entered by `chunk_reload` (the function that follows `chunk_step`).
const CHUNK_STEP_RETAIL: u32 = 0x0023_77c8;
/// Scene word pair summed by `chunk_reload` (0x173f08 and 0x16114c).
const SCENE_WORD: u32 = 0x0017_3f08;
const SCENE_BIAS: u32 = 0x0016_114c;
/// Offset of the per-chunk value read by `chunk_reload` from the state table.
const CHUNK_VALUE: u32 = 0x60;
/// Upper half of the scene segment, loaded into v1 by `chunk_reload`.
const SEG_HI: u32 = 0x0017_0000;

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x237710 chunk_step(index) -> 1: latches a camera request when record index+1 is larger than record index.
pub fn chunk_step(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let idx = c.arg(0);
    let camera = c.r32(STATE + STATE_CAMERA);
    let record_base = c.r32(STATE + STATE_RECORD).wrapping_mul(RECORD_STRIDE);
    let table = if c.r32(MODE_WORD) != 0 { TABLE_A } else { TABLE_B };
    let cur_addr = (idx << 2).wrapping_add(record_base).wrapping_add(table);
    let next_addr = ((idx.wrapping_add(1)) << 2).wrapping_add(record_base).wrapping_add(table);
    let cur = c.r32(cur_addr);
    let next = c.r32(next_addr);
    let diff = next.wrapping_sub(cur);

    set_gpr(c, REG_V1, cur_addr);
    set_gpr(c, REG_A0, next);
    set_gpr(c, REG_A1, cur);
    set_gpr(c, REG_A2, diff);
    set_gpr(c, REG_A3, camera);
    set_gpr(c, REG_V0, cur);

    if (diff as i32) > 0 {
        h::run_keep(c, cam_latch::cam_latch_request, &[camera, cur], &[]);
        h::run_keep(c, cam_latch::cam_latch_drain, &[0], &[]);
    }
    c.ret(1);
    true
}

/// 0x237c78 chunk_reload(index) -> sum: refreshes the chunk value, runs the chunk step and returns the scene sum.
pub fn chunk_reload(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let idx = c.arg(0);
    let addr = STATE.wrapping_add(idx << 2);
    let value = c.r32(addr + CHUNK_VALUE);
    c.w32(STATE + STATE_CAMERA, value);
    set_gpr(c, REG_V0, value);
    h::call_keep(c, CHUNK_STEP_RETAIL, &[addr], &[]);

    let bias = c.r32(SCENE_BIAS);
    let sum = c.r32(SCENE_WORD).wrapping_add(bias);
    set_gpr(c, REG_V1, SEG_HI);
    set_gpr(c, REG_A0, bias);
    c.w32(STATE + STATE_CAMERA, sum);
    c.ret(sum);
    true
}
