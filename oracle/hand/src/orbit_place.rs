//! Object placement from its rig table: writes the object's position from a table entry, rotated by its angle, and a
//! small setup sequence of two retail calls.
use crate::h;
use crate::ps2::*;
use crate::trig;
use ee_oracle::cpu::Cpu;

/// Frame of 0x26ef20 (saves s0 and ra, then the float registers f20..f23 are stored to the frame).
const FRAME: u32 = 0x40;
const SAVES: [(usize, u32, u8); 2] = [(16, 0x00, h::SQ), (31, 0x10, h::SQ)];
/// Offsets of the saved f20..f23 in the frame.
const FSAVE_OFF: [(usize, u32); 4] = [(20, 0x20), (21, 0x28), (22, 0x30), (23, 0x38)];

/// Object fields: rig pointer, angle, and the position x / y / z.
const OBJ_RIG: u32 = 0x78;
const OBJ_ANGLE: u32 = 0x48;
const OBJ_X: u32 = 0x10;
const OBJ_Y: u32 = 0x14;
const OBJ_Z: u32 = 0x18;

/// Rig header fields: a float at +0x38 (the angle) and a flags word at +0x30 (bit 0 selects the table column).
const RIG_ANGLE: u32 = 0x38;
const RIG_FLAGS: u32 = 0x30;
/// Rig fields: the object's table index at +0xc.
const RIG_INDEX: u32 = 0xc;

/// Offering table of per-index entries (32 bytes each), at 0x1c4588.
const ENTRY_TABLE: u32 = 0x001c_4588;
const ENTRY_SHIFT: u32 = 5;
/// Base table of per-object offsets at 0x166d80 (a3 = 0x160000 is its high half).
const BASE_HI: u32 = 0x0016_0000;
const BASE_TABLE: u32 = 0x0016_6d80;
const BASE_X: u32 = 0x140;
const BASE_Y: u32 = 0x144;
const BASE_Z: u32 = 0x148;

/// Entry fields: x (flag clear) or x (flag set) at +4 / +0, y at +8, z at +0xc, and the rotated pair at +0x18 / +0x1c.
const ENTRY_X_SET: u32 = 0;
const ENTRY_X_CLEAR: u32 = 4;
const ENTRY_Y: u32 = 8;
const ENTRY_Z: u32 = 0xc;
const ENTRY_ROT_A: u32 = 0x18;
const ENTRY_ROT_B: u32 = 0x1c;

/// Setup sequence of 0x26fbb8: two calls to 0x2913c8 and one to 0x239700, returning 8.
const SETUP_CALL: u32 = 0x0029_13c8;
const SETUP_FINISH: u32 = 0x0023_9700;
const SETUP_RESULT: u32 = 8;

/// 0x26ef20 orbit_place(obj) -> void: copies the object's table entry (x, y, z from the base table plus the entry's
/// offsets) into its position, then turns the entry's (A, B) pair by the object's angle (cos, sin) and adds it to the
/// x and y position.
pub fn orbit_place(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    h::frame(c, FRAME, &SAVES, |c, sp| {
        for (reg, off) in FSAVE_OFF {
            c.wf(sp + off, F(c.f[reg] as u32));
        }
        place(c, obj);
    });
    true
}

/// Body of orbit_place inside its frame.
fn place(c: &mut Cpu, obj: u32) {
    let rig = c.r32(obj + OBJ_RIG);
    let header = c.r32(rig);
    let index = c.r32(rig + RIG_INDEX);
    let angle = c.rf(header + RIG_ANGLE);
    c.wf(obj + OBJ_ANGLE, angle);
    let flags = c.r32(header + RIG_FLAGS);

    let shifted = index << ENTRY_SHIFT;
    let entry = ENTRY_TABLE.wrapping_add(shifted);
    let x_off = if flags & 1 != 0 { ENTRY_X_SET } else { ENTRY_X_CLEAR };
    let base_x = c.rf(BASE_TABLE + BASE_X);
    let x = base_x + c.rf(entry + x_off);
    c.wf(obj + OBJ_X, x);

    let base_y = c.rf(BASE_TABLE + BASE_Y);
    let y = base_y + c.rf(entry + ENTRY_Y);
    c.wf(obj + OBJ_Y, y);

    let base_z = c.rf(BASE_TABLE + BASE_Z);
    let z = base_z + c.rf(entry + ENTRY_Z);
    c.wf(obj + OBJ_Z, z);

    let rot_a = c.rf(entry + ENTRY_ROT_A);
    let rot_b = c.rf(entry + ENTRY_ROT_B);

    // Retail's scratch registers at the first call: a0 = base table, a1 = v0 = v1 = entry, a2 = index << 5, a3 = 0x160000.
    c.s(4, BASE_TABLE as u64);
    c.s(5, entry as u64);
    c.s(6, shifted as u64);
    c.s(7, BASE_HI as u64);
    c.s(2, entry as u64);
    c.s(3, entry as u64);
    let (_, cos) = h::run_keep(c, trig::fast_cos, &[], &[angle]);
    let (_, sin) = h::run_keep(c, trig::fast_sin, &[], &[angle]);
    let (cos, sin) = (F(cos.0), F(sin.0));

    let a_sin = rot_a * sin;
    let a_cos = rot_a * cos;
    let b_cos = rot_b * cos;
    let b_sin = -rot_b * sin;
    let dy = b_cos + a_sin;
    let dx = b_sin + a_cos;
    let y_new = c.rf(obj + OBJ_Y) + dy;
    let x_new = c.rf(obj + OBJ_X) + dx;
    c.wf(obj + OBJ_Y, y_new);
    c.wf(obj + OBJ_X, x_new);

    // Leave retail's scratch values: f0 = new y, f1 = new x, f2 = rot_a * sin.
    c.f[0] = y_new.0;
    c.f[1] = x_new.0;
    c.f[2] = a_sin.0;
}

/// 0x26fbb8 orbit_setup() -> int: calls 0x2913c8(0x42, 0x44), 0x2913c8(0x47, 0xb) and 0x239700(), returns 8.
pub fn orbit_setup(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    h::frame(c, 0x10, &[(31, 0x00, h::SQ)], |c, _sp| {
        h::call_keep(c, SETUP_CALL, &[0x42, 0x44], &[]);
        h::call_keep(c, SETUP_CALL, &[0x47, 0x0b], &[]);
        h::call_keep(c, SETUP_FINISH, &[], &[]);
    });
    c.ret(SETUP_RESULT);
    true
}
