//! Per-object slot jitter: frees the object when its body is missing, mismatched or in a dead state (slot_release),
//! else spins it by a random factor, offsets a random vector by the pose node position, rescales it and stores the
//! spin and the heading.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Offset of the body pointer inside the object.
const OBJ_BODY: u32 = 0x30;
/// Offset of the object id word (compared with the body's halfword id).
const OBJ_ID: u32 = 0x38;
/// Offset of the object's 32-bit id word inside the object (read as the pose node argument).
const OBJ_ARG: u32 = 0x34;
/// Offset of the 16-byte vector inside the object (copied to the frame, then offset and rescaled).
const OBJ_VEC: u32 = 0x10;
/// Offset of the float scale inside the object.
const OBJ_SCALE: u32 = 0x3c;
/// Offset of the float spin input inside the object (written back unchanged).
const OBJ_SPIN_IN: u32 = 0x2c;
/// Offset of the float spin output inside the object.
const OBJ_SPIN_OUT: u32 = 0x1c;
/// Offset of the heading vector inside the object (written from the offset vector).
const OBJ_HEADING: u32 = 0x20;
/// Body offsets: the halfword id (compared with the object id) and the state byte.
const BODY_ID: u32 = 0xa6;
const BODY_STATE: u32 = 0x20;
/// Body states that free the object.
const STATE_DEAD_A: u8 = 0xfe;
const STATE_DEAD_B: u8 = 0xfd;
/// Float constants: rand range 0.5 .. 1.0, jitter range -0.021 .. 0.021 (bits), length threshold 0.0625.
const SPIN_LO: u32 = 0x3f00_0000;
const SPIN_HI: u32 = 0x3f80_0000;
const JITTER_LO: u32 = 0xbcac_0831;
const JITTER_HI: u32 = 0x3cac_0831;
const LEN_LIMIT: u32 = 0x3d80_0000;

/// 0x264ab0 slot_jitter_update(obj) -> none: frees a stale object, else spins it, offsets its vector and rescales it.
pub fn slot_jitter_update(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let saves = [
        (16, 0x10, h::SQ),
        (17, 0x20, h::SQ),
        (18, 0x30, h::SQ),
        (19, 0x40, h::SQ),
        (31, 0x50, h::SQ),
    ];
    h::frame(c, 0x80, &saves, |c, sp| {
        let body = c.r32(obj + OBJ_BODY);
        let release = body == 0
            || (c.r16(body + BODY_ID) as i16 as i32 as u32) != c.r32(obj + OBJ_ID)
            || matches!(c.r8(body + BODY_STATE), STATE_DEAD_A | STATE_DEAD_B);
        if release {
            h::call_keep(c, 0x259b88, &[obj], &[]);
            return;
        }

        let spin = h::call_keep(c, 0x24a4b0, &[], &[F(SPIN_LO), F(SPIN_HI)]).1;
        let scale = c.rf(obj + OBJ_SCALE);
        let spin_in = c.rf(obj + OBJ_SPIN_IN);
        let spin_out = scale * spin;

        for i in 0..4 {
            let word = c.r32(obj + OBJ_VEC + 4 * i);
            c.w32(sp + 4 * i, word);
        }
        let body_now = c.r32(obj + OBJ_BODY);
        let arg = c.r32(obj + OBJ_ARG);
        h::call_keep(c, 0x242090, &[body_now, arg, obj + OBJ_VEC], &[]);
        h::call_keep(c, 0x1f1840, &[sp, sp, obj + OBJ_VEC], &[]);

        let jitter = h::call_keep(c, 0x24a4b0, &[], &[F(JITTER_LO), F(JITTER_HI)]).1;
        let len = h::call_keep(c, 0x1f1970, &[sp], &[]).1;
        let scaled = if len.get() < F(LEN_LIMIT).get() {
            jitter + F(LEN_LIMIT)
        } else {
            let again = h::call_keep(c, 0x1f1970, &[sp], &[]).1;
            again + jitter
        };
        h::call_keep(c, 0x1f1a98, &[sp, sp], &[scaled]);
        h::call_keep(c, 0x1f1810, &[obj + OBJ_HEADING, obj + OBJ_VEC, sp], &[]);
        c.wf(obj + OBJ_SPIN_IN, spin_in);
        c.wf(obj + OBJ_SPIN_OUT, spin_out);
    });
    true
}
