//! Look-point spring step: a gated copy of the mode byte, a spring-scaled blend between two gp-held points, and a
//! call that applies the resulting point to the look point at +0x90.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// 0x1f1530 (gate: its nonzero result enables the mode copy at +0xc5).
const GATE_CALL: u32 = 0x1f1530;
/// 0x2e88e8 (spring helper, args: obj, arg, out buffer, mode).
const SPRING_CALL: u32 = 0x2e88e8;
/// 0x2e8828 (applies the point (x, y, 0) to the look point; args: obj, look block, out buffer).
const LOOK_POINT_CALL: u32 = 0x2e8828;

/// gp-relative float pair A (x, y) of the lower corner of the blend range.
const GP_LO_X: i32 = -0x4de0;
const GP_LO_Y: i32 = -0x4ddc;
/// gp-relative float pair C (x, y) of the upper corner of the blend range.
const GP_HI_X: i32 = -0x4de8;
const GP_HI_Y: i32 = -0x4de4;

/// Look-block offsets (relative to the block at obj.+0x70 + 0x40).
const MODE: u32 = 0xc4;
const MODE_COPY: u32 = 0xc5;
const SCALE_INT: u32 = 0xc6;
const SCALE_FLOAT: u32 = 0xc8;
const LOOK_POINT: u32 = 0x90;
const GATE_OFFSET: u32 = 0x106;
const BLOCK_OFFSET: u32 = 0x40;
const OBJ_BLOCK_PTR: u32 = 0x70;

/// Modes that blend the point between the two gp corners by the spring factor.
const BLEND_MODES: [u8; 5] = [1, 3, 5, 6, 7];

/// Reads a gp-relative float constant.
fn gp_float(c: &mut Cpu, off: i32) -> F {
    let addr = (c.g(28) as u32).wrapping_add(off as u32);
    c.rf(addr)
}

/// 0x2e89a0 look_point_spring(obj, arg): when the gate passes, copies the mode byte to +0xc5 and reads the spring
/// scale (int at +0xc6 times float at +0xc8). It then runs the spring helper, picks a point between the two gp corners
/// (blended by the scale for the blend modes, else the upper corner) and applies it to the look point at +0x90.
pub fn look_point_spring(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    let arg = c.arg(1);
    let block = c.r32(obj + OBJ_BLOCK_PTR).wrapping_add(BLOCK_OFFSET);
    let gate_arg = c.r32(obj + OBJ_BLOCK_PTR).wrapping_add(GATE_OFFSET);
    let frame_sizes = 0x60;
    let saves = [(16, 0x10, h::SQ), (17, 0x20, h::SQ), (18, 0x30, h::SQ), (31, 0x40, h::SQ)];
    h::frame(c, frame_sizes, &saves, |c, sp| {
        c.w32(sp + 0x50, c.f[20]);
        let (gate, _) = h::call_keep(c, GATE_CALL, &[gate_arg], &[]);
        if gate != 0 {
            let mode = c.r8(block + MODE);
            c.w8(block + MODE_COPY, mode);
        }
        let raw = c.r16(block + SCALE_INT) as i16 as i32;
        let scale = F::from_int(raw) * c.rf(block + SCALE_FLOAT);
        let mode = c.r8(block + MODE);
        h::call_keep(c, SPRING_CALL, &[obj, arg, sp, mode as u32], &[]);
        let mode = c.r8(block + MODE);
        // retail's `lbu v1` (the mode) runs before the look-point call, which leaves v1 as it returns
        c.s(3, mode as u64);
        let (x, y) = if BLEND_MODES.contains(&mode) {
            let (lo_x, lo_y) = (gp_float(c, GP_LO_X), gp_float(c, GP_LO_Y));
            let (hi_x, hi_y) = (gp_float(c, GP_HI_X), gp_float(c, GP_HI_Y));
            (lo_x + (hi_x - lo_x) * scale, lo_y + (hi_y - lo_y) * scale)
        } else {
            (gp_float(c, GP_HI_X), gp_float(c, GP_HI_Y))
        };
        h::call_keep(c, LOOK_POINT_CALL, &[obj, block + LOOK_POINT, sp], &[x, y, F(0)]);
        // epilogue `lwc1 $f20, 0x50($sp)`
        c.f[20] = c.r32(sp + 0x50);
    });
    true
}
