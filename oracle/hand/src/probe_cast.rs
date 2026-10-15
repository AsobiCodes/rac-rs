//! Hand rewrite of the sphere-probe cast setup at 0x24c510.

use crate::h::{self, SQ};

use crate::ps2::*;
use ee_oracle::cpu::Cpu;



/// Stack frame size of the retail function (`addiu sp, sp, -0xe0`).
const FRAME: u32 = 0xe0;
/// Register saves (reg, sp offset, width) of the retail prologue: ra and s0-s6.
const SAVES: [(usize, u32, u8); 8] = [
    (31, 0xb0, SQ),
    (22, 0xa0, SQ),
    (21, 0x90, SQ),
    (20, 0x80, SQ),
    (19, 0x70, SQ),
    (18, 0x60, SQ),
    (17, 0x50, SQ),
    (16, 0x40, SQ),
];
/// Object flags halfword; bit 0x100 means the direction is the stored vector at +0xc0.
const OBJ_FLAGS: u32 = 0x34;
const FLAG_VECTOR: u16 = 0x100;
/// Object yaw float: its cos and sin give the direction when the flag is clear.
const OBJ_ANGLE: u32 = 0x48;
/// Object stored direction vector (16 bytes).
const OBJ_VECTOR: u32 = 0xc0;
/// Object halfword passed on to the retail cast.
const OBJ_SHORT: u32 = 0xa6;
/// Retail cast routine called last (its v0/f0 are the result).
const CAST_FN: u32 = 0x1e50d0;
/// Retail sine and cosine (VU micro calls), run as retail so the VU and at state match.
const FAST_COS: u32 = 0x1f1d80;
const FAST_SIN: u32 = 0x1f1d98;
/// Retail vector scale (VU), run as retail.
const VEC_SCALE: u32 = 0x1f1898;
/// Retail probe_init (0x24c4e8), run as retail.
const PROBE_INIT: u32 = 0x24c4e8;
/// Float 1.0, stored as the w lane of the direction.
const ONE: F = F(0x3f80_0000);
/// Probe length float stored at sp+0xc.
const PROBE_LENGTH: F = F(0x45af_df66);

/// 0x24c510 probe_cast(obj, a1, a2, a3, t0, t1, f12, f13, f14): builds a unit-w direction at sp (the object's stored
/// vector at +0xc0 when flag 0x100 is set, else (cos, sin, 0) of its yaw), scales it by f14, initialises the probe at
/// sp+0x10 from f13 and runs the retail cast 0x1e50d0 with its flags; the cast's v0/f0 are the result.
pub fn probe_cast(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let a1 = c.arg(1);
    let a2 = c.arg(2);
    let a3 = c.arg(3);
    let t0 = c.arg(4);
    let t1 = c.arg(5);
    let f12 = c.farg(0);
    let f13 = c.farg(1);
    let f14 = c.farg(2);
    h::frame(c, FRAME, &SAVES, |c, sp| {
        c.wf(sp + 0xd0, f12);
        c.wf(sp + 0xc8, f13);
        c.wf(sp + 0xc0, f14);
        let dir = sp;
        if c.r16(obj + OBJ_FLAGS) & FLAG_VECTOR != 0 {
            let v = c.rv(obj + OBJ_VECTOR);
            c.wv(dir, v);
        } else {
            let angle = c.rf(obj + OBJ_ANGLE);
            let (_, cos) = h::call_keep(c, FAST_COS, &[], &[angle]);
            let (_, sin) = h::call_keep(c, FAST_SIN, &[], &[angle]);
            c.wf(dir, cos);
            c.wf(dir + 4, sin);
            c.w32(dir + 8, 0);
        }
        h::call_keep(c, VEC_SCALE, &[dir, dir], &[f14]);
        c.wf(dir + 8, ONE);
        c.wf(dir + 0xc, PROBE_LENGTH);
        let probe = dir + 0x10;
        h::call_keep(c, PROBE_INIT, &[probe, obj, a2, dir], &[f13]);
        c.f[0] = ONE.0;
        c.f[1] = PROBE_LENGTH.0;
        c.s(1, PROBE_LENGTH.0 as i32 as i64 as u64);
        c.w8(dir + 0x28, a3 as u8);
        c.w8(dir + 0x29, t0 as u8);
        let short = c.r16(obj + OBJ_SHORT);
        c.w16(dir + 0x2a, short);
        h::call_keep(c, CAST_FN, &[a1, t1, obj, probe], &[f12]);
    });
    true
}
