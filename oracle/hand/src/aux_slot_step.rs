//! Per-object auxiliary slot update: frees the object when its state byte is 0 or 0xF0-class, otherwise syncs the
//! link flag, updates the visibility bit from a threshold, advances the aux offset and re-randomises the scale.

use crate::h;
use crate::ps2::*;
use crate::{actor, layer_bake, mathx, moby_anim_bsphere, path, rand, vecmath};
use ee_oracle::cpu::Cpu;

/// Offset of the aux block pointer inside the object.
const OBJ_AUX: u32 = 0x78;
/// Offset of the body pointer inside the aux block.
const AUX_BODY: u32 = 0x8;
/// Offset of the state byte inside the body.
const BODY_STATE: u32 = 0x20;
/// Offset of the 16-bit flag word inside the object and the body.
const FLAGS: u32 = 0x34;
/// Offset of the aux offset word (advanced by `shift_entry_offsets`) inside the aux block.
const AUX_OFFSET: u32 = 0x4;
/// Offset of the float scale inside the aux block.
const AUX_SCALE: u32 = 0xc;
/// Offset of the pointer to the layer data inside the object.
const OBJ_LAYER: u32 = 0x24;
/// Offset of the float heading inside the object.
const OBJ_HEADING: u32 = 0x44;
/// Offset of the float spin scale written inside the object.
const OBJ_SPIN: u32 = 0x2c;

/// Body state byte of a dead object: its class nibble is 0xF0 (or the byte is zero).
const STATE_DEAD: u8 = 0xf0;
/// Flag bits of the object: bit 0 mirrors the body flag; FLAG_NEAR (0x41) also sets bit 6 when the threshold is passed.
const FLAG_MIRROR: u16 = 0x0001;
const FLAG_NEAR: u16 = 0x0041;
/// Integer argument of `float_to_int_scaled`.
const SCALE_INPUT: u32 = 25;
/// Word the scaled value is compared against (signed), at 0x140400.
const LIMIT_WORD: u32 = 0x0014_0400;
/// Offset window: the aux offset is kept within (-0xc0 .. 0x1001), wrapped by 0x10c0 or 0xf40 otherwise.
const OFFSET_STEP: u32 = 0xc0;
const OFFSET_LIMIT: i32 = 0x1001;
const OFFSET_WRAP_HIGH: u32 = 0x10c0;
const OFFSET_WRAP_LOW: u32 = 0xf40;
/// Float constants: 0.5 and 1.0.
const HALF: u32 = 0x3f00_0000;
const ONE: u32 = 0x3f80_0000;

/// 0x2ad7a8 aux_slot_update(obj) -> none: frees a dead object, else syncs flags, advances the aux offset, rolls a
/// random scale and updates the heading and bounding sphere.
pub fn aux_slot_update(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let aux = c.r32(obj + OBJ_AUX);
    let body = c.r32(aux + AUX_BODY);
    let state = c.r8(body + BODY_STATE);
    if state == 0 || state & STATE_DEAD == STATE_DEAD {
        h::run_keep(c, actor::free_moby, &[obj], &[]);
        return true;
    }
    if body != 0 {
        let body_flag = c.r16(body + FLAGS) & FLAG_MIRROR;
        let own = c.r16(obj + FLAGS);
        if own & FLAG_MIRROR != body_flag {
            let cleared = own & !FLAG_MIRROR;
            c.w16(obj + FLAGS, cleared);
            let body_now = c.r32(aux + AUX_BODY);
            let flag = c.r16(body_now + FLAGS) & FLAG_MIRROR;
            c.w16(obj + FLAGS, cleared | flag);
        }
    }

    let scaled = h::run_keep(c, mathx::float_to_int_scaled, &[SCALE_INPUT], &[]).0;
    let limit = c.r32(LIMIT_WORD) as i32;
    if limit < scaled as i32 {
        let flags = c.r16(obj + FLAGS) | FLAG_NEAR;
        c.w16(obj + FLAGS, flags);
    }

    let base = c.r32(aux + AUX_OFFSET);
    let stepped = base.wrapping_sub(OFFSET_STEP);
    let (offset, shift_by) = if (stepped as i32) >= OFFSET_LIMIT {
        (base.wrapping_sub(OFFSET_WRAP_HIGH), 0u32.wrapping_sub(OFFSET_WRAP_HIGH))
    } else if (stepped as i32) < 0 {
        (base.wrapping_add(OFFSET_WRAP_LOW), OFFSET_WRAP_LOW)
    } else {
        (stepped, 0u32.wrapping_sub(OFFSET_STEP))
    };
    c.w32(aux + AUX_OFFSET, offset);
    let layer = c.r32(obj + OBJ_LAYER);
    h::run_keep(c, layer_bake::shift_entry_offsets, &[layer, 0, shift_by], &[]);

    let spin = h::run_keep(c, rand::rand_float, &[], &[F(HALF), F(ONE)]).1;
    let layer = c.r32(obj + OBJ_LAYER);
    let scale = c.rf(aux + AUX_SCALE);
    let layer_scale = c.rf(layer + 0x24);
    c.wf(obj + OBJ_SPIN, layer_scale * scale * spin);

    let heading = c.rf(aux + AUX_SCALE);
    let wrapped = h::run_keep(c, path::wrap_angle_pi, &[], &[heading]).1;
    let current = c.rf(obj + OBJ_HEADING);
    let sum = h::run_keep(c, vecmath::angle_add, &[], &[current, wrapped]).1;
    c.wf(obj + OBJ_HEADING, sum);
    h::run_keep(c, moby_anim_bsphere::bsphere_stored_rows, &[obj], &[]);
    true
}
