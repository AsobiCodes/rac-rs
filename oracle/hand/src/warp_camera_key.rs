//! Camera key warp: copies the current camera key and rebuilds the camera basis from it.

use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Camera table pointer block (retail `v1` after `lui 0x17; addiu -0x36a0`).
const CAM_TABLE: u32 = 0x0016_C960;
/// Table word holding the current key block pointer (base).
const KEY_BASE_OFF: u32 = 0x54;
/// Table word holding the current key index; the key block is 32 bytes per index.
const KEY_INDEX_OFF: u32 = 0x38;
const KEY_STRIDE_SHIFT: u32 = 5;
/// Slot that receives the key's float (retail `swc1 -0x3410($a0)` with `a0 = 0x170000`).
const KEY_FLOAT_SLOT: u32 = 0x0016_CBF0;
const LUI_17: u32 = 0x0017_0000;
/// Camera state block (retail `s2 = 0x166ec0`).
const STATE: u32 = 0x0016_6EC0;
/// Basis rows are written relative to `STATE - 0x140`.
const BASIS_OUT: u32 = STATE - 0x140;
/// Byte flag (retail `0x15edb4`): when set, the basis cross product is taken.
const CROSS_FLAG: u32 = 0x0015_EDB4;
/// Frame size of this function's stack frame.
const FRAME: u32 = 0x90;

const FN_1EA1E8: u32 = 0x001e_a1e8;
const FN_125218: u32 = 0x0012_5218;
const FN_125360: u32 = 0x0012_5360;
const FN_125408: u32 = 0x0012_5408;
const FN_1252B8: u32 = 0x0012_52b8;
/// Return address of the jal to 0x1252b8 (the delay slot after it is 0x27edf4).
const RET_AFTER_1252B8: u32 = 0x0027_edf0;

/// Stores a whole 128-bit register to the stack like `sq`.
fn store_q(c: &mut Cpu, addr: u32, v: u128) {
    for i in 0..4u32 {
        c.w32(addr.wrapping_add(4 * i), (v >> (32 * i)) as u32);
    }
}

/// Loads a float from memory (`lwc1`).
fn load_f(c: &mut Cpu, addr: u32) -> F {
    F(c.r32(addr))
}

/// 0x27ed60 warp_camera_key() -> u8: copies the current key block into the camera state, writes the key's three
/// basis rows (negated, from the stack buffer) and crosses two of them when the flag byte is set; returns the key's
/// byte 0xc.
pub fn warp_camera_key(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let sp = (c.r[29] as u32).wrapping_sub(FRAME);
    store_q(c, sp + 0x40, c.r[16]);
    store_q(c, sp + 0x50, c.r[17]);
    store_q(c, sp + 0x60, c.r[18]);
    store_q(c, sp + 0x70, c.r[19]);
    store_q(c, sp + 0x80, c.r[31]);

    let key_base = c.r32(CAM_TABLE + KEY_BASE_OFF);
    let key_index = c.r32(CAM_TABLE + KEY_INDEX_OFF);
    let key = key_base.wrapping_add(key_index << KEY_STRIDE_SHIFT);
    let key_pos = key.wrapping_add(0x10);
    let key_flag = c.r8(key + 0xc) as u32;

    // The key float goes to its slot before the first helper runs (delay slot of the jal).
    c.f[0] = load_f(c, key_pos + 0xc).0;
    c.w32(KEY_FLOAT_SLOT, c.f[0]);
    h::call_keep(c, FN_1EA1E8, &[LUI_17], &[]);

    // The key block is copied into the camera state, then the helpers fill the stack buffer from it.
    let quad = c.rv(key);
    c.wv(STATE, quad);
    h::call_keep(c, FN_125218, &[sp], &[]);
    let f12 = load_f(c, key + 0x10);
    h::call_keep(c, FN_125360, &[sp, sp], &[f12]);
    let f12 = load_f(c, key_pos + 4);
    h::call_keep(c, FN_125408, &[sp, sp], &[f12]);
    let f12 = load_f(c, key_pos + 8);
    h::call_keep(c, FN_1252B8, &[sp, sp], &[f12]);
    // The callee copies its return address into a2 (`move $a2, $ra`) and restores ra from it; the nested call
    // returns with the wrong ra, so a2 gets retail's return address (the jal at 0x27edec) here.
    c.r[6] = (c.r[6] & !(u32::MAX as u128)) | RET_AFTER_1252B8 as u128;

    // Basis rows: the stack buffer's rows, negated where retail negates them.
    let x0 = load_f(c, sp);
    let y0 = load_f(c, sp + 4);
    let z0 = load_f(c, sp + 8);
    let x1 = load_f(c, sp + 0x10);
    let y1 = load_f(c, sp + 0x14);
    let z1 = load_f(c, sp + 0x18);
    let x2 = load_f(c, sp + 0x20);
    let y2 = load_f(c, sp + 0x24);
    let z2 = load_f(c, sp + 0x28);
    let (nx0, ny0, nz0) = (-x0, -y0, -z0);
    let (nx2, ny2, nz2) = (-x2, -y2, -z2);

    c.f[0] = z1.0;
    c.f[1] = nz0.0;
    c.f[2] = nx0.0;
    c.f[3] = ny0.0;
    c.f[4] = nz2.0;
    c.f[5] = nx2.0;
    c.f[6] = ny2.0;
    c.f[7] = x1.0;
    c.f[8] = y1.0;
    c.w32(BASIS_OUT + 0x378, z1.0);
    c.w32(BASIS_OUT + 0x350, nx2.0);
    c.w32(BASIS_OUT + 0x360, nx0.0);
    c.w32(BASIS_OUT + 0x370, x1.0);
    c.w32(BASIS_OUT + 0x354, ny2.0);
    c.w32(BASIS_OUT + 0x364, ny0.0);
    c.w32(BASIS_OUT + 0x374, y1.0);
    c.w32(BASIS_OUT + 0x358, nz2.0);
    c.w32(BASIS_OUT + 0x368, nz0.0);

    c.r[3] = c.r8(CROSS_FLAG) as u128;
    if c.r[3] as u32 != 0 {
        h::run_keep(
            c,
            vecmath::cross3,
            &[STATE + 0x220, STATE + 0x230, STATE + 0x210],
            &[],
        );
    }
    c.ret(key_flag);
    true
}
