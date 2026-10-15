//! Hero "up axis" vector helpers (Eudora overlay 0x226610..0x226ef8). The hero block's gravity mode (byte 0x20b3)
//! selects the frame: 0 = world z up, 1 = up axis at +0x270, 2 = the current gravity frame (matrix at +0x40).
use crate::calls::*;
use crate::ps2::*;
use crate::trig;
use ee_oracle::cpu::Cpu;

const VEC_ADD: u32 = 0x1f_1810;
const VEC_SUB: u32 = 0x1f_1840;
const VEC_SCALE: u32 = 0x1f_1898;
const DOT3: u32 = 0x1f_1930;
const LENGTH_XY: u32 = 0x1f_19a0;
const SET_LENGTH: u32 = 0x1f_1a98;
const SET_LENGTH_XY: u32 = 0x1f_1ae8;
const MUL_MAT4: u32 = 0x1f_1c90;
const MAT_FROM_ANGLES: u32 = 0x1f_2008;
const VEC_INIT: u32 = 0x1f_17f8;
const MAT3_TRANSPOSE: u32 = 0x1f_2290;
const ATAN2: u32 = 0x1f_1e48;
const FLOAT_TO_INT: u32 = 0x1f_27e8;
const INT_TO_FLOAT: u32 = 0x1f_27d8;
const FAST_COS: u32 = 0x1f_1d80;
const FAST_SIN: u32 = 0x1f_1d98;

const H_ACTOR: u32 = 0x2080;
const H_STATE: u32 = 0x2084;
const H_UP_MODE: u32 = 0x20b3;
/// Up axis vectors in the hero block, and the matrix rotating gravity-frame vectors to world.
const H_UP_AXIS: u32 = 0x270;
const H_UP_AXIS_2: u32 = 0x290;
const H_FRAME_MATRIX: u32 = 0x40;
const A_YAW: u32 = 0x48;
const A_MATRIX: u32 = 0x40;

fn up_mode(c: &mut Cpu) -> u32 {
    c.r8(HERO + H_UP_MODE) as u32
}

fn rd_f(c: &mut Cpu) -> F {
    c.fresult()
}

/// Retail fast_cos / fast_sin run the VU microprogram: calling them as retail does leaves VF and ACC as they are.
fn retail_cos(c: &mut Cpu, x: F) -> F {
    call(c, FAST_COS, &[], &[x]);
    rd_f(c)
}

fn retail_sin(c: &mut Cpu, x: F) -> F {
    call(c, FAST_SIN, &[], &[x]);
    rd_f(c)
}

/// dst = src moved along the up axis by -k.
fn up_offset(c: &mut Cpu, dst: u32, src: u32, k: F) {
    match up_mode(c) {
        0 => {
            let z = c.rf(src + 8) - k;
            c.wf(dst + 8, z);
        }
        1 => with_frame(c, 0x40, |c, tmp| {
            call(c, SET_LENGTH, &[tmp, HERO + H_UP_AXIS], &[-k]);
            call(c, VEC_ADD, &[dst, src, tmp], &[]);
        }),
        2 => with_frame(c, 0x40, |c, tmp| {
            if c.r32(HERO + H_STATE) == 0x3e && c.r16(HERO + 0x30e) == 0 {
                call(c, SET_LENGTH, &[tmp, HERO + H_UP_AXIS], &[-k]);
            } else {
                call(c, SET_LENGTH, &[tmp, HERO + H_UP_AXIS_2], &[k]);
            }
            call(c, VEC_ADD, &[dst, src, tmp], &[]);
        }),
        _ => {}
    }
}

/// 0x226610 up_offset(dst, src, f12 k): dst = src moved along the hero up axis by -k (mode 0: z - k).
pub fn up_offset_hand(c: &mut Cpu) -> bool {
    preserved(c, false, false, up_offset_hand_body)
}

fn up_offset_hand_body(c: &mut Cpu) -> bool {
    let (dst, src, k) = (c.arg(0), c.arg(1), c.farg(0));
    up_offset(c, dst, src, k);
    true
}

/// 0x226828 up_offset_neg(dst, src, f12 k): up_offset(dst, src, -k) (mode 0: z + k).
pub fn up_offset_neg(c: &mut Cpu) -> bool {
    preserved(c, false, false, up_offset_neg_body)
}

fn up_offset_neg_body(c: &mut Cpu) -> bool {
    let (dst, src, k) = (c.arg(0), c.arg(1), c.farg(0));
    up_offset(c, dst, src, -k);
    true
}

/// 0x226708 polar_to_world(out, f12 radius, f13 angle, f14 z): out = (cos a * r, sin a * r, z) * hero matrix + offset.
pub fn polar_to_world(c: &mut Cpu) -> bool {
    preserved(c, false, false, polar_to_world_body)
}

fn polar_to_world_body(c: &mut Cpu) -> bool {
    let (out, r, ang, z) = (c.arg(0), c.farg(0), c.farg(1), c.farg(2));
    let x = trig::vu_cos(ang) * r;
    c.wf(out, x);
    let y = trig::vu_sin(ang) * r;
    c.wf(out + 8, z);
    c.wf(out + 4, y);
    call(c, MUL_MAT4, &[out, out, HERO], &[]);
    call(c, VEC_ADD, &[out, out, HERO + 0x80], &[]);
    true
}

/// 0x2267a0 point_from_offset(out, f12 x, f13 y, f14 z): out = (x, y, z) * hero matrix + hero feet position.
pub fn point_from_offset(c: &mut Cpu) -> bool {
    preserved(c, false, false, point_from_offset_body)
}

fn point_from_offset_body(c: &mut Cpu) -> bool {
    let (out, x, y, z) = (c.arg(0), c.farg(0), c.farg(1), c.farg(2));
    call(c, VEC_INIT, &[out], &[]);
    c.wf(out, x);
    c.wf(out + 4, y);
    c.wf(out + 8, z);
    call(c, MUL_MAT4, &[out, out, HERO], &[]);
    call(c, VEC_ADD, &[out, out, HERO + 0x80], &[]);
    true
}

/// 0x226848 point_ahead(out, base, f12 r): out = base + r along the hero yaw (mode 0) or the gravity frame.
pub fn point_ahead(c: &mut Cpu) -> bool {
    preserved(c, false, false, point_ahead_body)
}

fn point_ahead_body(c: &mut Cpu) -> bool {
    let (out, base, r) = (c.arg(0), c.arg(1), c.farg(0));
    let mode = up_mode(c);
    if mode == 0 {
        let v = c.rv(base);
        c.wv(out, v);
        let actor = c.r32(HERO + H_ACTOR);
        let yaw = c.rf(actor + A_YAW);
        let x = c.rf(out) + retail_cos(c, yaw) * r;
        c.wf(out, x);
        let actor = c.r32(HERO + H_ACTOR);
        let yaw = c.rf(actor + A_YAW);
        let y = c.rf(out + 4) + retail_sin(c, yaw) * r;
        c.wf(out + 4, y);
    } else if mode < 3 {
        with_frame(c, 0xa0, |c, m| {
            let tmp = m + 0x40;
            let x = retail_cos(c, F::ZERO) * r;
            c.wf(tmp, x);
            let y = retail_sin(c, F::ZERO) * r;
            c.wf(tmp + 8, F::ZERO);
            c.wf(tmp + 4, y);
            let actor = c.r32(HERO + H_ACTOR);
            call(c, MAT_FROM_ANGLES, &[m, actor + A_MATRIX], &[]);
            call(c, MUL_MAT4, &[tmp, tmp, m], &[]);
            call(c, VEC_ADD, &[out, base, tmp], &[]);
        });
    }
    true
}

/// 0x226950 point_up(out, base, f12 h): out = base raised by h along the up axis (mode 0: z + h).
pub fn point_up(c: &mut Cpu) -> bool {
    preserved(c, false, false, point_up_body)
}

fn point_up_body(c: &mut Cpu) -> bool {
    let (out, base, h) = (c.arg(0), c.arg(1), c.farg(0));
    let mode = up_mode(c);
    if mode == 0 {
        let v = c.rv(base);
        c.wv(out, v);
        let z = c.rf(out + 8) + h;
        c.wf(out + 8, z);
    } else if mode < 3 {
        with_frame(c, 0xb0, |c, m| {
            let tmp = m + 0x40;
            call(c, VEC_INIT, &[tmp], &[]);
            c.wf(tmp + 8, h);
            let actor = c.r32(HERO + H_ACTOR);
            call(c, MAT_FROM_ANGLES, &[m, actor + A_MATRIX], &[]);
            call(c, MUL_MAT4, &[tmp, tmp, m], &[]);
            call(c, VEC_ADD, &[out, base, tmp], &[]);
        });
    }
    true
}

/// 0x226a10 project_off_up(out, v): v with its component along the up axis removed (mode 0: z = 0).
pub fn project_off_up(c: &mut Cpu) -> bool {
    preserved(c, false, false, project_off_up_body)
}

fn project_off_up_body(c: &mut Cpu) -> bool {
    let (out, v) = (c.arg(0), c.arg(1));
    match up_mode(c) {
        0 => {
            let q = c.rv(v);
            c.wv(out, q);
            c.w32(out + 8, 0);
        }
        mode @ (1 | 2) => with_frame(c, 0x50, |c, tmp| {
            let axis = if mode == 1 {
                call(c, SET_LENGTH, &[tmp, HERO + H_UP_AXIS], &[F::ONE]);
                tmp
            } else {
                HERO + H_UP_AXIS_2
            };
            call(c, DOT3, &[axis, v], &[]);
            let d = rd_f(c);
            call(c, VEC_SCALE, &[tmp, axis], &[d]);
            call(c, VEC_SUB, &[out, v, tmp], &[]);
        }),
        _ => {}
    }
    true
}

/// 0x226b10 length_off_up(v) -> f0: horizontal length of v in the gravity frame (mode 0: length_xy(v)).
pub fn length_off_up(c: &mut Cpu) -> bool {
    preserved(c, false, true, length_off_up_body)
}

fn length_off_up_body(c: &mut Cpu) -> bool {
    let v = c.arg(0);
    let r = match up_mode(c) {
        0 => {
            call(c, LENGTH_XY, &[v], &[]);
            rd_f(c)
        }
        1 | 2 => with_frame(c, 0xd0, |c, m| {
            let actor = c.r32(HERO + H_ACTOR);
            call(c, MAT_FROM_ANGLES, &[m, actor + A_MATRIX], &[]);
            call(c, MAT3_TRANSPOSE, &[m + 0x40, m], &[]);
            call(c, MUL_MAT4, &[m + 0x80, v, m + 0x40], &[]);
            call(c, LENGTH_XY, &[m + 0x80], &[]);
            rd_f(c)
        }),
        _ => F::ZERO,
    };
    c.fret(r);
    true
}

/// 0x226bb8 up_component(v) -> f0: v's component along the up axis (mode 0: v.z).
pub fn up_component(c: &mut Cpu) -> bool {
    preserved(c, false, true, up_component_body)
}

fn up_component_body(c: &mut Cpu) -> bool {
    let v = c.arg(0);
    let r = up_component_of(c, v);
    c.fret(r);
    true
}

fn up_component_of(c: &mut Cpu, v: u32) -> F {
    match up_mode(c) {
        0 => c.rf(v + 8),
        1 | 2 => with_frame(c, 0x90, |c, m| {
            let tmp = m + 0x40;
            call(c, VEC_INIT, &[tmp], &[]);
            let actor = c.r32(HERO + H_ACTOR);
            c.wf(tmp + 8, F::ONE);
            call(c, MAT_FROM_ANGLES, &[m, actor + A_MATRIX], &[]);
            call(c, MUL_MAT4, &[tmp, tmp, m], &[]);
            call(c, DOT3, &[tmp, v], &[]);
            rd_f(c)
        }),
        _ => F::ZERO,
    }
}

/// 0x226c60 set_up_component(out, in, f12 z): out = in rotated to world with z replaced by the given up value.
pub fn set_up_component(c: &mut Cpu) -> bool {
    preserved(c, false, false, set_up_component_body)
}

fn set_up_component_body(c: &mut Cpu) -> bool {
    let (out, inp, z) = (c.arg(0), c.arg(1), c.farg(0));
    match up_mode(c) {
        0 => {
            let q = c.rv(inp);
            c.wv(out, q);
            c.wf(out + 8, z);
        }
        1 | 2 => {
            call(c, MUL_MAT4, &[out, inp, HERO + H_FRAME_MATRIX], &[]);
            c.wf(out + 8, z);
            call(c, MUL_MAT4, &[out, out, HERO], &[]);
        }
        _ => {}
    }
    true
}

/// 0x226ce0 set_length_xy_up(out, in, f12 len): horizontal (gravity frame) set-length of `in` into `out`.
pub fn set_length_xy_up(c: &mut Cpu) -> bool {
    preserved(c, false, false, set_length_xy_up_body)
}

fn set_length_xy_up_body(c: &mut Cpu) -> bool {
    let (out, inp, len) = (c.arg(0), c.arg(1), c.farg(0));
    match up_mode(c) {
        0 => call(c, SET_LENGTH_XY, &[out, inp], &[len]),
        1 | 2 => {
            call(c, MUL_MAT4, &[out, inp, HERO + H_FRAME_MATRIX], &[]);
            call(c, SET_LENGTH_XY, &[out, out], &[len]);
            call(c, MUL_MAT4, &[out, out, HERO], &[]);
        }
        _ => {}
    }
    true
}

/// 0x226d70 fall_height(pos, vel, out_ticks, f12 period) -> f0: height of pos after the vertical velocity has run
/// n whole periods (n*(n+1)/2 gravity steps); *out_ticks = n. Gravity modes 1 and 2 use the half-period estimate.
pub fn fall_height(c: &mut Cpu) -> bool {
    preserved(c, false, true, fall_height_body)
}

fn fall_height_body(c: &mut Cpu) -> bool {
    let (pos, vel, out, period) = (c.arg(0), c.arg(1), c.arg(2), c.farg(0));
    let r = match up_mode(c) {
        0 => {
            let vz = c.rf(vel + 8);
            call(c, FLOAT_TO_INT, &[], &[vz / period]);
            let n = c.result();
            call(c, INT_TO_FLOAT, &[n], &[]);
            let nf = rd_f(c);
            let tri = (n.wrapping_mul(n).wrapping_add(n) as i32) >> 1;
            call(c, INT_TO_FLOAT, &[tri as u32], &[]);
            let tf = rd_f(c);
            let h = c.rf(pos + 8) + c.rf(vel + 8) * nf;
            if out != 0 {
                c.w32(out, n);
            }
            h - tf * period
        }
        1 | 2 => {
            let up = up_component_of(c, vel);
            call(c, FLOAT_TO_INT, &[], &[up / period]);
            let n = c.result();
            if out != 0 {
                c.w32(out, n);
            }
            c.rf(pos + 8) + c.rf(vel + 8) * F(0x3f00_0000)
        }
        _ => F::ZERO,
    };
    c.fret(r);
    true
}

/// 0x226e70 pitch_of(v) -> f0: atan2(up component, horizontal length) of v in the gravity frame.
pub fn pitch_of(c: &mut Cpu) -> bool {
    pitch_of_body(c)
}

fn pitch_of_body(c: &mut Cpu) -> bool {
    let v = c.arg(0);
    let mode = up_mode(c);
    // Retail's lui/addiu/lbu leave a2 = hero block and v0 = mode; the calls below leave their own scratch values.
    c.set_arg(2, HERO);
    c.ret(mode);
    let r = match mode {
        0 => {
            call(c, LENGTH_XY, &[v], &[]);
            let len = rd_f(c);
            let z = c.rf(v + 8);
            atan2(c, z, len)
        }
        1 | 2 => with_frame(c, 0x30, |c, tmp| {
            let q = c.rv(v);
            c.wv(tmp, q);
            c.r[2] = (0..4).fold(0u128, |acc, k| acc | (c.r32(v + 4 * k) as u128) << (32 * k));
            call(c, MUL_MAT4, &[tmp, tmp, HERO + H_FRAME_MATRIX], &[]);
            call(c, LENGTH_XY, &[tmp], &[]);
            let len = rd_f(c);
            let z = c.rf(tmp + 8);
            atan2(c, z, len)
        }),
        _ => {
            c.ret(0);
            F::ZERO
        }
    };
    c.fret(r);
    true
}

fn atan2(c: &mut Cpu, a: F, b: F) -> F {
    call(c, ATAN2, &[], &[a, b]);
    rd_f(c)
}
