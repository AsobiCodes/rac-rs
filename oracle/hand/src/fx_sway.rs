//! Sway/tint effect update for the overlay at 0x1fa750..0x1fab00 (rec/l4_02). Retail state block is the
//! `lui 0x14; addiu -0xcb0` base; float constants come from the `lui 0x16; -0x1294` literal pool.
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Effect state block base (retail `$s4`/`$s1`).
const STATE: u32 = 0x0014_0000 - 0x0cb0;
/// Nonzero: the packed tint word is decayed each frame.
const ENABLE: u32 = STATE + 0x22f4;
/// Packed word: top byte kept, three 8-bit levels (low, mid, high) decayed by 7 and clamped at 0.
const PACKED: u32 = STATE + 0x22f8;
/// Mode word; 0x7f, 0x6b, 0x76, 0x80, 0x82, 0xf, 0x11 select the paths below.
const MODE: u32 = STATE + 0x2084;
/// Object pointer handed to the helpers.
const OBJ: u32 = STATE + 0x2080;
/// Counter compared against thresholds.
const COUNTER: u32 = STATE + 0x198;
/// Signed index into the flag table at `TABLE`.
const INDEX: u32 = STATE + 0x10b8;
/// Table of one-byte flags indexed by `INDEX`.
const TABLE: u32 = 0x0014_0000 - 0x1ae0;
/// Pointer passed to the chained helper 0x1fa3d8.
const CHAIN: u32 = STATE + 0x1090;
/// Pointer to a 16-byte cell that receives the sway offset.
const CELL: u32 = STATE + 0x86c;
/// Halfword flag that suppresses the sway update.
const HALF_FLAG: u32 = STATE + 0x8bc;
/// Literal-pool float (time scale), `lwc1 0x160000-0x1294`.
const SCALE_ADDR: u32 = 0x0016_0000 - 0x1294;
/// Literal-pool word (counter threshold table), `lw 0x160000-0x127c`.
const SELECT_ADDR: u32 = 0x0016_0000 - 0x127c;
/// Matrix (3x4 floats) inside the state block.
const MATRIX: u32 = STATE + 0x00c0;
/// Constant vector (x,y,z,w) table at 0x180000-0x5840.
const VEC_TABLE: u32 = 0x0018_0000 - 0x5840;

/// Level `(packed >> shift) & 0xff`, minus 7, clamped to 0.
fn level(packed: u32, shift: u32) -> i32 {
    ((((packed >> shift) & 0xff) as i32) - 7).max(0)
}

/// 0x1fa750 fx_tint_update(): decays the packed tint word when enabled, then tests the counter for the current
/// mode and, when due, calls the retail helpers 0x242be8/0x242bb8/0x1fa370; finally runs 0x1fa3d8 on a flagged
/// table entry.
pub fn fx_tint_update(c: &mut Cpu) -> bool {
    if c.r32(ENABLE) != 0 {
        let packed = c.r32(PACKED);
        let (s3, s1, s0) = (level(packed, 0), level(packed, 8), level(packed, 16));
        let obj = c.r32(OBJ);
        h::call(c, 0x242bb8, &[obj, s3 as u32, s1 as u32, s0 as u32], &[]);
        let top = packed >> 24;
        let decayed = (top << 24) | ((s0 as u32) << 16) | ((s1 as u32) << 8) | s3 as u32;
        c.w32(PACKED, decayed);
    }

    let mode = c.r32(MODE);
    let mut due = false;
    // Retail loads `-0x7e7c($gp)` into v1 in the delay slot of the 0x82 test, so that load is the selector
    // unless the 0x80/0x82 path reloads it from the literal pool.
    let mut sel = c.r32((c.g(28) as u32).wrapping_sub(0x7e7c));
    if mode == 0x80 || mode == 0x82 {
        let v = h::scale_int(c, 30);
        due = (c.r32(COUNTER) as i32) < v;
        sel = c.r32(SELECT_ADDR);
    }
    if sel == 0xf || sel == 0x11 {
        if c.r32(MODE) == 0x76 {
            let v = h::scale_int(c, 20);
            if (c.r32(COUNTER) as i32) < v {
                due = true;
            }
        }
    }

    if due {
        let sp = c.g(29) as u32;
        let obj = c.r32(OBJ);
        h::call(c, 0x242be8, &[obj, sp, sp + 4, sp + 8], &[]);
        // Split the counter into a (value mod 4) remainder: 3 or more gives (0x90, 0x90, 0xf0), else zeros.
        let v1 = c.r32(COUNTER) as i32;
        let t = if v1 >= 0 { v1 } else { v1 + 3 };
        let rem = v1 - ((t >> 2) << 2);
        let (a, d) = if rem < 3 { (0, 0) } else { (0x90, 0xf0) };
        c.w32(sp, a);
        c.w32(sp + 4, a);
        c.w32(sp + 8, d);
        let obj = c.r32(OBJ);
        h::call(c, 0x242bb8, &[obj, a, a, d], &[]);
        h::call(c, 0x1fa370, &[], &[]);
    }

    let index = c.r32(INDEX) as i32;
    if index >= 0 && c.r8(TABLE + index as u32) != 0 {
        let chain = c.r32(CHAIN);
        if chain != 0 {
            let obj = c.r32(OBJ);
            h::call(c, 0x1fa3d8, &[chain, obj], &[]);
        }
    }
    true
}

/// 0x1fa950 fx_sway_matrix(): in mode 0x7f and then mode 0x6b (when the half flag is clear) advances the sway
/// angles through 0x254f80, then rotates a seed vector by the object matrix and adds it into the sway cell.
pub fn fx_sway_matrix(c: &mut Cpu) -> bool {
    let scale = c.rf(SCALE_ADDR);
    if c.r32(MODE) == 0x7f {
        let obj = c.r32(OBJ);
        let args = [obj, STATE + 0x06a8, STATE + 0x06ac];
        h::call(c, 0x254f80, &args, &[F(0x3e99_999a), scale * F(0x40df_66f3)]);
    }
    if c.r32(MODE) != 0x6b || (c.r16(HALF_FLAG) as i16) != 0 {
        return true;
    }
    let obj = c.r32(OBJ);
    let args = [obj, STATE + 0x06a8, STATE + 0x06ac];
    h::call(c, 0x254f80, &args, &[F(0x3f8c_cccd), scale * F(0x40a7_8d36)]);

    let cell = c.r32(CELL);
    if cell == 0 {
        return true;
    }
    // Copy the 16-byte object seed into the cell, then build the local seed vector (0, 0, 0.21, 0).
    let src = c.r32(OBJ) + 0x10;
    let dst = cell + 0x10;
    let quad = c.rv(src);
    c.wv(dst, quad);
    let sp = c.g(29) as u32;
    c.w32(sp, 0);
    c.w32(sp + 4, 0);
    c.w32(sp + 8, 0x3e57_0a3d);
    c.w32(sp + 12, 0);
    let obj = c.r32(OBJ);
    h::run(c, vecmath::vec3_mul_mat3, &[sp, sp, obj + 0x00c0], &[]);
    let mat = sp + 0x10;
    h::call(c, 0x1f2008, &[mat, VEC_TABLE], &[]);
    h::run(c, vecmath::vec4_mul_mat4, &[sp, sp, mat], &[]);
    let cell = c.r32(CELL);
    let dst = cell + 0x10;
    h::run(c, vecmath::vec_add, &[dst, dst, sp], &[]);
    true
}

/// 0x1faa90 fx_sway_yaw(): in mode 0x7f advances the yaw/pitch pair through 0x255000 with angle scales derived from
/// the time-scale float.
pub fn fx_sway_yaw(c: &mut Cpu) -> bool {
    if c.r32(MODE) != 0x7f {
        return true;
    }
    let scale = c.rf(SCALE_ADDR);
    let obj = c.r32(OBJ);
    let f13 = scale * F(0x3fc9_0fdb);
    let f14 = scale * F(0x4006_0a92);
    h::call(c, 0x255000, &[obj, STATE + 0x06b0, STATE + 0x06b4], &[F(0x3e32_b8c2), f13, f14]);
    true
}
