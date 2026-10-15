//! Checks on the object attached to the current level slot. `attached_obj` reads the attached-object pointer from the
//! level word at 0x173f58 and calls `objects::attached_value` on it; both checks below return 1 only when that pointer
//! is set and a field of the object has its flag.
use crate::h;
use crate::objects;
use ee_oracle::cpu::Cpu;
use crate::ps2::*;

/// Retail word holding the attached-object argument (lui 0x17 + 0x3f58).
const ATTACHED_SLOT: u32 = 0x0017_3f58;
/// Byte offset in the attached object of the flag tested by `attached_byte9_is_one`.
const OBJ_BYTE_FLAG: u32 = 9;
/// Halfword offset in the attached object of the bits tested by `attached_bit1_set`.
const OBJ_HALF_FLAGS: u32 = 0x1e;
/// Bit of the halfword at `OBJ_HALF_FLAGS` that must be set.
const FLAG_BIT: u32 = 2;

/// 0x2998c0 attached_byte9_is_one() -> v0: 1 if an attached object exists and its byte at +9 equals 1, else 0.
pub fn attached_byte9_is_one(c: &mut Cpu) -> bool {
    let frame = (c.r[29] as u32).wrapping_sub(0x20);
    // The frame saves s0 and ra before the call, as retail does.
    c.mem.wr128(frame, c.r[16]);
    c.mem.wr128(frame + 0x10, c.r[31]);
    let obj = c.r32(ATTACHED_SLOT);
    c.r[4] = obj as i32 as i128 as u128;
    let mut result = 0;
    if obj != 0 {
        let (attached, _) = h::run_keep(c, objects::attached_value, &[obj], &[]);
        if attached != 0 {
            result = u32::from(c.r8(attached + OBJ_BYTE_FLAG) == 1);
        }
    }
    c.ret(result);
    true
}

/// 0x299910 attached_bit1_set() -> v0: 1 if an attached object exists and bit 1 of its halfword at +0x1e is set, else 0.
pub fn attached_bit1_set(c: &mut Cpu) -> bool {
    let frame = (c.r[29] as u32).wrapping_sub(0x10);
    // The frame saves ra before the call, as retail does.
    c.mem.wr128(frame, c.r[31]);
    let obj = c.r32(ATTACHED_SLOT);
    c.r[4] = obj as i32 as i128 as u128;
    let (attached, _) = h::run_keep(c, objects::attached_value, &[obj], &[]);
    let mut result = 0;
    if attached != 0 {
        // Retail leaves the masked halfword in v1 on this path.
        let flags = u32::from(c.r16(attached + OBJ_HALF_FLAGS)) & FLAG_BIT;
        c.r[3] = flags as u128;
        result = u32::from(flags != 0);
    }
    c.ret(result);
    true
}
