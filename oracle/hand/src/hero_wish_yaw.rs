//! Hero stick reader and wished-yaw helpers (retail overlay code at 0x204918..0x204d9c).
//! The hero state block is at 0x13f350 (retail s0 = 0x140000 - 0xcb0).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Hero state block base (retail s0 after `addiu -0xcb0`).
const CTX: u32 = 0x13_F350;
/// Anim clock: current frame T and per-tick step dT.
const FRAME_T: u32 = CTX + 0xaa8;
const FRAME_DT: u32 = CTX + 0xaac;
/// Facing yaw (used as the idle heading and as the mode 4 offset).
const FACING: u32 = CTX + 0x98;
/// Wished yaw output.
const YAW: u32 = CTX + 0x180;
/// Stick magnitude (also the 0x13f4e0 output slot of the stick reader).
const MAG: u32 = CTX + 0x190;
/// Stick X and Y as read from the pad (0x141070 / 0x141074).
const STICK_X: u32 = CTX + 0x1d20;
const STICK_Y: u32 = CTX + 0x1d24;
/// Byte flag: non-zero selects the retail idle heading routine.
const IDLE_FROM_RETAIL: u32 = CTX + 0x20b3;
/// Hero state word (0x3f and 0x73 select the magnitude clamps).
const STATE: u32 = CTX + 0x2084;
/// Scratch vector and matrix output (retail s0 = 0x13f4c0).
const OUT: u32 = 0x13_F4C0;
/// Pointer to the matrix block at OUT + 0x1f10, plus 0xc0.
const OUT_MATRIX_PTR: u32 = OUT + 0x1f10;
const OUT_MATRIX_OFF: u32 = 0xc0;
/// Camera matrix used for the non-idle stick vector.
const CAM_MATRIX: u32 = 0x16_70D0;
/// Byte flag: non-zero keeps the stick X sign in the rotated vector.
const FLIP_FLAG: u32 = 0x15_EDB4;
/// Retail stick reader that writes the wished yaw and magnitude (0x204970).
const STICK_WISH_YAW: u32 = 0x20_4970;
/// Retail idle heading routine (0x226410).
const RETAIL_IDLE_YAW: u32 = 0x22_6410;
/// Retail callees of the stick reader: called as retail code so their VU0 writes (VF/ACC) match exactly.
const LENGTH_XY: u32 = 0x1f_19a0;
const VEC_DIV_SCALAR: u32 = 0x1f_1910;
const FAST_ATAN2: u32 = 0x1f_1e48;
const ANGLE_ADD: u32 = 0x1f_2680;
const VEC3_MUL_MAT3: u32 = 0x1f_1c68;
const FLOAT_TO_INT: u32 = 0x1f_27e8;
const INT_TO_FLOAT: u32 = 0x1f_27d8;
/// Float offset used for modes other than 4.
const YAW_OFFSET: u32 = 0x16_6ED8;
/// Unit magnitude constant.
const UNIT: u32 = 0x15_ED6C;
/// Table of three magnitude breakpoints (0x17bec0, 0x17bec4, 0x17bed0).
const MAG_TABLE: u32 = 0x17_BEB8;

const PI: F = F(0x4049_0fdb);
const HALF: F = F(0x3f00_0000);
const HALF_PI: F = F(0x3fc9_0fdb);
const QUARTER_PI_ISH: F = F(0x3f49_0fd8);
const SNAP2_SCALE: F = F(0x3f22_f987);
const SNAP3_SCALE: F = F(0x3fa2_f987);
const STICK_HI: F = F(0x3f33_3333);
const STICK_LO: F = F(0x3df5_c28f);
const K_3_5: F = F(0x4060_0000);
const K_0_8: F = F(0x3f4c_cccd);
const K_2_5: F = F(0x4020_0000);

/// `sq $reg, addr` of a whole GPR: four little-endian words, lane 0 at the lowest address.
fn save_q(c: &mut Cpu, addr: u32, reg: usize) {
    let v = c.r[reg];
    for i in 0..4u32 {
        c.w32(addr + 4 * i, (v >> (32 * i)) as u32);
    }
}

/// 0x204918 hero_frame_crossed(x) -> 1 if x < T and T - x <= dT on the hero anim clock, else 0.
pub fn hero_frame_crossed(c: &mut Cpu) -> bool {
    let x = c.farg(0);
    let t = c.rf(FRAME_T);
    let crossed = x < t && t - x <= c.rf(FRAME_DT);
    c.ret(crossed as u32);
    true
}

/// 0x204970 hero_stick_wish_yaw(mode, scale) -> writes the wished yaw (0x13f4d0) and magnitude (0x13f4e0).
/// Mode 1 dead-zones the stick; modes 2 and 3 snap the heading to 4 or 8 octants; mode 4 adds facing.
pub fn hero_stick_wish_yaw(c: &mut Cpu) -> bool {
    let mode = c.arg(0);
    let scale = c.farg(0);
    // Retail prologue: addiu $sp,-0x60, then sq $s0/$s1/$ra and swc1 $f20/$f21 into the frame.
    let entry_sp = c.g(29) as u32;
    let sp = entry_sp.wrapping_sub(0x60);
    save_q(c, sp + 0x20, 16);
    save_q(c, sp + 0x30, 17);
    save_q(c, sp + 0x40, 31);
    let (f20, f21) = (c.f[20], c.f[21]);
    c.w32(sp + 0x50, f20);
    c.w32(sp + 0x58, f21);
    let mut stick_x = c.rf(STICK_X);
    let mut stick_y = c.rf(STICK_Y);
    c.wf(sp + 4, stick_y);
    c.wf(sp, stick_x);

    if mode == 1 {
        // A large Y clears a small X; then a large X clears a small Y.
        if stick_y.abs() > STICK_HI && stick_x.abs() < STICK_LO {
            stick_x = F::ZERO;
            c.w32(sp, 0);
        }
        if stick_x.abs() > STICK_HI && stick_y.abs() < STICK_LO {
            stick_y = F::ZERO;
            c.w32(sp + 4, 0);
        }
    }

    // Normalise the stick vector when it is longer than 1.
    let (_, len) = h::call_keep(c, LENGTH_XY, &[sp], &[]);
    if F::ONE < len {
        h::call_keep(c, VEC_DIV_SCALAR, &[sp, sp], &[len]);
    }
    let ax = c.rf(sp);
    let ay = c.rf(sp + 4);

    if ax == F::ZERO && ay == F::ZERO {
        idle_heading(c, sp, ay);
        return true;
    }

    // Heading from the stick, atan2 of the negated components.
    let (_, mut heading) = h::call_keep(c, FAST_ATAN2, &[], &[-ay, -ax]);
    match mode {
        2 => heading = snap_octant(c, heading, SNAP2_SCALE, 3, HALF_PI),
        3 => heading = snap_octant(c, heading, SNAP3_SCALE, 7, QUARTER_PI_ISH),
        _ => {}
    }
    c.wf(YAW, heading);

    let offset = if mode == 4 { c.rf(FACING) } else { c.rf(YAW_OFFSET) };
    let (_, yaw) = h::call_keep(c, ANGLE_ADD, &[], &[heading, offset]);
    c.wf(YAW, yaw);

    // Rotate the stick vector by the camera matrix and store its scaled length.
    let flip_byte = c.r8(FLIP_FLAG);
    c.wf(sp, -ay);
    c.wf(sp + 4, if flip_byte != 0 { ax } else { -ax });
    c.w32(sp + 8, 0);
    c.w32(sp + 0xc, 0);
    // Retail leaves f0 = -ay, f1 = 0, f2 = -ax, f3 = ax and v0 = the flag byte at the vec3 call.
    c.s(2, flip_byte as u64);
    c.f[0] = (-ay).0;
    c.f[1] = F::ZERO.0;
    c.f[2] = (-ax).0;
    c.f[3] = ax.0;
    h::call_keep(c, VEC3_MUL_MAT3, &[OUT, sp, CAM_MATRIX], &[]);
    let (_, stick_len) = h::call_keep(c, LENGTH_XY, &[sp], &[]);
    let magnitude = scale * stick_len;
    c.wf(MAG, magnitude);
    // Retail: mul.s f0, f21 (scale), f0 (length), then swc1 f0 to the magnitude slot.
    c.f[0] = magnitude.0;
    true
}

/// Idle stick: heading is the retail routine's result or the facing yaw; the vector points along +X.
fn idle_heading(c: &mut Cpu, sp: u32, ay: F) {
    c.wf(MAG, F::ZERO);
    let heading = if c.r8(IDLE_FROM_RETAIL) != 0 {
        h::call_keep(c, RETAIL_IDLE_YAW, &[], &[ay]).1
    } else {
        c.rf(FACING)
    };
    c.wf(YAW, heading);
    for i in 0..4 {
        c.w32(sp + 0x10 + 4 * i, 0);
    }
    c.wf(sp + 0x10, F::ONE);
    // Retail: por v0,zero,zero; mtc1 1.0 into f0 (f1 is still 0 from the normalise test).
    c.s(2, 0);
    c.f[0] = F::ONE.0;
    c.f[1] = F::ZERO.0;
    let matrix = c.r32(OUT_MATRIX_PTR).wrapping_add(OUT_MATRIX_OFF);
    h::call_keep(c, VEC3_MUL_MAT3, &[OUT, sp + 0x10, matrix], &[]);
}

/// Snaps `heading` to a multiple of `step` through the octant index of (heading + pi) * scale.
fn snap_octant(c: &mut Cpu, heading: F, scale: F, mask: u32, step: F) -> F {
    let k = (heading + PI) * scale + HALF;
    let (q, _) = h::call_keep(c, FLOAT_TO_INT, &[], &[k]);
    let (_, idx) = h::call_keep(c, INT_TO_FLOAT, &[q & mask], &[]);
    idx * step - PI
}

/// Retail high half of the hero block pointer (lui v0,0x14).
const HERO_HI: u32 = 0x14_0000;
/// Retail `at` after `lui $at,0x16` and `lui $at,0x4020`.
const AT_16: u32 = 0x16_0000;
const AT_2_5: u32 = 0x4020_0000;
/// Retail `at` after `lui $at,0x4060` (float 3.5).
const AT_3_5: u32 = 0x4060_0000;

/// Writes a general register as retail does for lui/addiu/lw results (sign-extended 32-bit value).
fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x204c90 hero_stick_magnitude() -> void: runs the stick reader at full scale, then clamps the magnitude
/// at 0x13f4e0 by the hero state (0x3f raises a floor, 0x73 keeps a minimum after a table lookup).
/// The scratch registers (v0/v1, a0-a2, at, f0-f3) are left as the retail code leaves them on each path.
pub fn hero_stick_magnitude(c: &mut Cpu) -> bool {
    const V0: usize = 2;
    const V1: usize = 3;
    const A0: usize = 4;
    const A1: usize = 5;
    const A2: usize = 6;
    const AT: usize = 1;
    // Retail prologue: addiu $sp,-0x10; sq $ra,0($sp). The jal to 0x204970 leaves $ra = 0x204ca8.
    let entry_sp = c.g(29) as u32;
    let entry_ra = c.g(31);
    let sp = entry_sp.wrapping_sub(0x10);
    save_q(c, sp, 31);
    c.s(29, sp as u64);
    c.s(31, 0x204ca8);
    // Retail callee: its vector-unit work (VF/ACC) must stay as retail leaves it, so run it as retail code.
    h::call_keep(c, STICK_WISH_YAW, &[0], &[F::ONE]);
    c.s(29, entry_sp as u64);
    c.s(31, entry_ra);
    // Retail: lui v0,0x14; addiu a0,zero,0x3f; addiu a1,v0,-0xcb0; move a2,v0; lw v1,0x2084(a1).
    set_gpr(c, V0, HERO_HI);
    set_gpr(c, A0, 0x3f);
    set_gpr(c, A1, CTX);
    set_gpr(c, A2, HERO_HI);
    let state = c.r32(STATE);
    set_gpr(c, V1, state);
    let unit = c.rf(UNIT);
    if state == 0x3f {
        let mag = c.rf(MAG);
        let t = unit * K_3_5 * mag;
        let floor = unit + unit;
        c.wf(MAG, if t < floor { floor } else { t });
        // Retail: f0 = t, f1 = old magnitude, f2 = 3.5, f3 = 2 * unit; at = 0x40600000 (last lui).
        set_gpr(c, AT, AT_3_5);
        c.f[0] = t.0;
        c.f[1] = mag.0;
        c.f[2] = K_3_5.0;
        c.f[3] = floor.0;
        return true;
    }
    let mag = c.rf(MAG);
    if F::ZERO < mag {
        let breakpoint = c.rf(MAG_TABLE + 0xc);
        let table = if mag < breakpoint { c.rf(MAG_TABLE + 0x8) } else { c.rf(MAG_TABLE + 0x18) };
        let value = table * unit;
        c.wf(MAG, value);
        // Retail: f0 = table * unit, f1 = unit, at = 0x160000.
        set_gpr(c, AT, AT_16);
        c.f[0] = value.0;
        c.f[1] = unit.0;
    } else {
        // Retail: f0 = 0.0 and f1 = the magnitude; `at` is left as the callee left it.
        c.f[0] = F::ZERO.0;
        c.f[1] = mag.0;
    }
    // Retail: addiu a0,a2,-0xcb0; addiu v1,zero,0x73; lw v0,0x2084(a0).
    set_gpr(c, A0, CTX);
    set_gpr(c, V1, 0x73);
    set_gpr(c, V0, state);
    if state == 0x73 {
        let scaled = c.rf(MAG) * K_0_8;
        let limit = unit * K_2_5;
        c.wf(MAG, if scaled < limit { limit } else { scaled });
        // Retail: f0 = scaled, f1 = 2.5, f2 = 0.8, f3 = unit * 2.5, at = 0x40200000.
        set_gpr(c, AT, AT_2_5);
        c.f[0] = scaled.0;
        c.f[1] = K_2_5.0;
        c.f[2] = K_0_8.0;
        c.f[3] = limit.0;
    }
    true
}
