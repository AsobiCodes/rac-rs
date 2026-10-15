//! Ground probe for the rec L4_13 record: clamps a height above a floor, runs the retail probe, and reads its result.
//! Also the default joint-limit table fill.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail probe routine (0x1e24d8) that the height probe calls.
const PROBE: u32 = 0x1e24_d8;
/// Upper half of the table address the probe result is read from (`lui v0, 0x17`, then `lwc1 0x3f68(v0)`).
const RESULT_HI: u32 = 0x17_0000;
const RESULT_OFF: u32 = 0x3f68;
/// Offset of the probe output (copy of the input vector) inside the local frame.
const OUT_VEC: u32 = 0x10;
/// Frame size of the retail function.
const FRAME: u32 = 0x30;
/// Frame offset where the function saves ra.
const RA_SAVE: u32 = 0x20;
/// Offset of z inside a 16-byte vector.
const Z: u32 = 8;
/// 2.5f: the height offset subtracted from z.
const OFFSET: u32 = 0x4020_0000;
/// 0.1f: the minimum clamped height.
const MIN_H: u32 = 0x3dcc_cccd;
/// 0.75f: added to the copied z before the probe.
const PROBE_LIFT: u32 = 0x3f40_0000;

/// 0x291bf8 probe_floor_height(vec*) -> f32: z - 2.5 clamped to at least 0.1 (in the frame), probe at z + 0.75 and
/// return the float at 0x173f68 when the probe hits (0.0 otherwise); v0 = 0x170000.
pub fn probe_floor_height(c: &mut Cpu) -> bool {
    let vec = c.arg(0);
    h::frame(c, FRAME, &[(31, RA_SAVE, h::SQ)], |c, sp| {
        let v = c.rv(vec);
        c.wv(sp, v);
        let z = c.rf(sp + Z) - F(OFFSET);
        c.wf(sp + Z, z);
        if z < F(MIN_H) {
            c.wf(sp + Z, F(MIN_H));
        }
        let copy = c.rv(vec);
        c.wv(sp + OUT_VEC, copy);
        let lifted = c.rf(sp + OUT_VEC + Z) + F(PROBE_LIFT);
        c.wf(sp + OUT_VEC + Z, lifted);
        let (hit, _) = h::call_keep(c, PROBE, &[sp + OUT_VEC, sp, 2, 0, 0], &[]);
        let out = if hit != 0 { c.rf(RESULT_HI + RESULT_OFF) } else { F(0) };
        c.fret(out);
        c.ret(RESULT_HI);
    });
    true
}

/// 0x291c98 joint_limits_default(_, table*) -> f32: stores -pi/16, pi/16 and -pi/2, pi/2 into the table; f0 = pi/16.
pub fn joint_limits_default(c: &mut Cpu) -> bool {
    let table = c.arg(1);
    c.w32(table + 0x10c, 0xbe49_0fdb);
    c.w32(table + 0x1c0, 0x3e49_0fdb);
    c.w32(table + 0x110, 0x3fc9_0fdb);
    c.w32(table + 0x1bc, 0xbfc9_0fdb);
    c.fret(F(0x3e49_0fdb));
    true
}
