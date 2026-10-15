//! Homing-ray particle spawn, particle type 0x4e (retail 0x267d20): allocates a slot and fills its header, its
//! position/velocity block and the optional attached-object value.
use crate::h;
use crate::mathx;
use crate::objects;
use crate::ps2::*;
use crate::rand;
use crate::talk_pad_scan;
use ee_oracle::cpu::Cpu;

/// Particle type passed to the slot allocator.
const PARTICLE_TYPE_HOMING_RAY: u32 = 0x4e;
/// Kind byte stored at slot+3.
const RAY_KIND: u8 = 0x48;
/// Float 4.0, passed to float_to_int.
const FOUR: u32 = 0x4080_0000;
/// Float scale applied to both float arguments (0x484d1400).
const SCALE: u32 = 0x484d_1400;
/// Global holding a pointer to a byte table (0x1b22b8, overlay data: read at call time).
const TABLE_PTR: u32 = 0x1b_22b8;
/// Offset added to the float_to_int result for the byte at slot+9.
const BYTE9_BIAS: u32 = 0x20;
/// Random range for the kind byte when the mode is neither 0 nor 1.
const RAND_RANGE: u32 = 255;
/// Kind value used for mode 1.
const MODE1_KIND: u8 = 0x20;
/// Offset of the second record (slot + 0x20) inside the slot.
const SECOND: u32 = 0x20;
/// Offset of the attached-object float inside the object.
const ATTACHED_OFFSET: u32 = 0x10;

/// 0x267d20 spawn_ray_4e(src, life, colour, mode, side, vel, target, fx, fy) -> slot or 0: allocates a type-0x4e
/// slot (0 when none is free) and fills its header from the 16-byte record at src, the scaled floats fx and fy,
/// and the velocity vector at vel. The attached-object value is read from target when it is non-null.
pub fn spawn_ray_4e(c: &mut Cpu) -> bool {
    let src = c.arg(0);
    let life = c.arg(1);
    let colour = c.arg(2);
    let mode = c.arg(3);
    let side = c.arg(4);
    let vel = c.arg(5);
    let target = c.arg(6);
    let fx = c.farg(0);
    let fy = c.farg(1);

    let args = [PARTICLE_TYPE_HOMING_RAY, c.arg(1), c.arg(2), c.arg(3)];
    let (slot, _) = h::run(c, talk_pad_scan::particle_alloc_mode0, &args, &[]);
    if slot == 0 {
        c.ret(0);
        return true;
    }
    let p = slot;

    // Header: the 16-byte source record goes to slot+0x10.
    for i in 0..4 {
        let word = c.r32(src + i * 4);
        c.w32(p + 0x10 + i * 4, word);
    }
    c.w32(p + 4, colour);
    c.w8(p + 3, RAY_KIND);
    let (cell, _) = h::run(c, mathx::float_to_int, &[], &[F(FOUR)]);
    c.w8(p + 1, 0);
    let table = c.r32(TABLE_PTR);
    let tag = c.r8(table);
    c.w8(p + 2, tag);
    c.w8(p + 9, (cell.wrapping_add(BYTE9_BIAS)) as u8);
    c.w16(p + 0xa, life as u16);
    let scaled_x = fx * F(SCALE);
    c.wf(p + 0xc, scaled_x);

    let kind = match mode {
        0 => 0,
        1 => MODE1_KIND,
        _ => h::run(c, rand::rand_below, &[RAND_RANGE], &[]).0 as u8,
    };
    c.w8(p + 8, kind);

    // Second record at slot+0x20: the motion block.
    let s1 = p + SECOND;
    let scaled_y = fy * F(SCALE);
    c.w16(s1 + 8, life as u16);
    c.wf(s1, scaled_x);
    c.w8(s1 + 0xa, side as u8);
    c.wf(s1 + 4, scaled_y);
    c.w8(s1 + 0xb, ((colour as i32) >> 24) as u8);
    c.w32(s1 + 0x18, target);
    c.w32(s1 + 0x1c, 0);
    if target != 0 {
        let (attached, _) = h::run(c, objects::attached_value, &[target], &[]);
        if attached != 0 {
            let value = c.rf(attached + ATTACHED_OFFSET);
            c.wf(s1 + 0x1c, value);
        }
    }
    let vx = c.rf(vel);
    let vy = c.rf(vel + 4);
    let vz = c.rf(vel + 8);
    c.wf(s1 + 0xc, vx);
    c.wf(s1 + 0x10, vy);
    c.wf(s1 + 0x14, vz);
    c.ret(p);
    true
}
