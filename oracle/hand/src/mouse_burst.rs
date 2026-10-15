//! Mouse-burst particle spawner: allocates a type 0x35 particle and fills its position, speed and z acceleration.
use crate::ps2::*;
use crate::{mathx, rand};
use ee_oracle::cpu::Cpu;

/// Particle type allocated by the burst spawner.
const PARTICLE_TYPE: u32 = 0x35;
/// Retail particle allocator: a0 = type, v0 = particle or 0.
const ALLOC_PARTICLE: u32 = 0x259a38;
/// Pointer to the byte table whose first byte is stored as the particle's +2 field.
const ROW_TABLE_PTR: u32 = 0x1b_2254;
/// Constant added to the particle's height (+0x18, read after the position copy), 0.05.
const HEIGHT_STEP: F = F(0x3d4c_cccd);
/// Input of the timer conversion: float_to_int(0.4) is stored plus 0x20 in the particle's +9 field.
const TIMER_INPUT: F = F(0x3ecc_cccd);
/// Scale applied to the float inputs before they are stored (+0xc and the burst copy at +0x24).
const SPEED_SCALE: F = F(0x484d_1400);
/// Byte stored at +8 for mode 1.
const MODE1_KIND: u8 = 0x20;
/// Upper bound passed to the random byte draw for modes other than 0 and 1.
const RAND_LIMIT: u32 = 0xff;

/// 0x264438 spawn_mouse_burst(pos, id, z_bits, mode, tag, vel, scale, spread, z_accel) -> particle or 0.
pub fn spawn_mouse_burst(c: &mut Cpu) -> bool {
    clean(c, spawn_mouse_burst_body, true)
}

/// Runs `body` and puts every register back as it was on entry, keeping only v0 when the function returns a value.
fn clean(c: &mut Cpu, body: fn(&mut Cpu) -> bool, returns_v0: bool) -> bool {
    let (r, f) = (c.r, c.f);
    let ok = body(c);
    let v0 = c.r[2];
    c.r = r;
    c.f = f;
    if ok && returns_v0 {
        c.r[2] = v0;
    }
    ok
}

/// v0 = `p` with the upper 64 bits of `hi_from`, as `move v0, s0` after an `lq v0` leaves them.
fn ret_with_hi(c: &mut Cpu, p: u32, hi_from: V) {
    let hi = (hi_from.0[2] as u128 | (hi_from.0[3] as u128) << 32) << 64;
    c.r[2] = hi | (p as i32 as i64 as u64) as u128;
}

/// The body of `spawn_mouse_burst`.
fn spawn_mouse_burst_body(c: &mut Cpu) -> bool {
    let (pos, id, z_bits, mode) = (c.arg(0), c.arg(1) as u16, c.arg(2), c.arg(3));
    let (tag, vel) = (c.arg(4) as u8, c.arg(5));
    let (scale, spread, z_accel) = (c.farg(0), c.farg(1), c.farg(2));

    c.set_arg(0, PARTICLE_TYPE);
    c.call(ALLOC_PARTICLE);
    let p = c.result();
    if p == 0 {
        c.ret(0);
        return true;
    }
    // The 16-byte position copy covers +0x10..+0x1f, so the height read below sees the copied z.
    let v = c.rv(pos);
    c.wv(p + 0x10, v);
    c.w32(p + 4, z_bits);
    c.w16(p + 0xa, id);
    c.w8(p + 3, 0x48);
    let height = c.rf(p + 0x18) + HEIGHT_STEP;
    c.wf(p + 0x18, height);
    let timer = timer_field(c);
    c.w8(p + 9, timer);
    c.w8(p + 1, 0);
    let row = c.r32(ROW_TABLE_PTR);
    let row_byte = c.r8(row);
    c.w8(p + 2, row_byte);
    c.wf(p + 0xc, scale * SPEED_SCALE);
    let kind = kind_for_mode(c, mode);
    c.w8(p + 8, kind);

    // Burst block at +0x20: copies of the scale and spread, the id and tag, z acceleration and the velocity vector.
    let b = p + 0x20;
    let life = c.rf(p + 0xc);
    c.wf(b, life);
    c.wf(b + 4, spread * SPEED_SCALE);
    c.w16(b + 8, id);
    c.w8(b + 0xa, tag);
    c.w8(b + 0xb, (z_bits >> 24) as u8);
    c.wf(b + 0x18, z_accel);
    for i in 0..3 {
        let x = c.rf(vel + 4 * i);
        c.wf(b + 0xc + 4 * i, x);
    }
    ret_with_hi(c, p, v);
    true
}

/// The +9 field: float_to_int(0.4) plus 0x20.
fn timer_field(c: &mut Cpu) -> u8 {
    let (int_part, _) = crate::h::run(c, mathx::float_to_int, &[], &[TIMER_INPUT]);
    (int_part as u8).wrapping_add(0x20)
}

/// The +8 field: 0 for mode 0, 0x20 for mode 1, otherwise a random byte below 0xff.
fn kind_for_mode(c: &mut Cpu, mode: u32) -> u8 {
    match mode {
        0 => 0,
        1 => MODE1_KIND,
        _ => crate::h::run(c, rand::rand_below, &[RAND_LIMIT], &[]).0 as u8,
    }
}
