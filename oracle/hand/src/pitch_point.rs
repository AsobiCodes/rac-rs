//! Pitch angle of a point lifted off the hero, measured from an object's forward axis in the vertical plane.
use crate::camera;
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Camera globals: the up vector (16 bytes) that the hero position is lifted along.
const CAM_UP: u32 = 0x166f30;
/// pi/2 as a single-precision float.
const HALF_PI: F = F(0x3fc9_0fdb);
/// Stack frame size of the retail function.
const FRAME: u32 = 0xa0;

/// 0x2e77f0 follow_point_pitch_angle(obj, h) -> signed pitch of hero + h*up as seen from obj: the angle between obj's
/// forward axis and the point's offset in the plane spanned by forward and up (negative when the point lies below
/// obj's up axis).
pub fn follow_point_pitch_angle(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let lift = c.farg(0);
    let saves = [
        (16, 0x40, h::SQ),
        (17, 0x50, h::SQ),
        (18, 0x60, h::SQ),
        (19, 0x70, h::SQ),
        (31, 0x80, h::SQ),
    ];
    let angle = h::frame(c, FRAME, &saves, |c, sp| {
        let hero = sp;
        let point = sp + 0x10;
        let offset = sp + 0x20;
        let along = sp + 0x30;
        let forward = obj + 0x10;
        let up = obj + 0x20;
        let origin = obj + 0x30;
        // point = hero position + lift * up
        h::run(c, camera::follow_hero_pos, &[obj, hero], &[]);
        h::vec_scale(c, point, CAM_UP, lift);
        h::vec_add(c, point, point, hero);
        // offset = point - origin; keep only the part perpendicular to forward
        h::vec_sub(c, offset, point, origin);
        let depth = h::dot3(c, offset, forward);
        h::vec_scale(c, along, forward, depth);
        h::vec_sub(c, offset, offset, along);
        // magnitude: pi/2 - acos(cos) where cos = offset . obj[0] / |offset|
        let cos_num = h::dot3(c, offset, obj);
        let len = h::length3(c, offset);
        if len == F::ZERO {
            return F::ZERO;
        }
        let acos = h::run(c, mathx::acos_approx, &[], &[cos_num / len]).1;
        let magnitude = HALF_PI - acos;
        let side = h::dot3(c, offset, up);
        if side < F::ZERO { -magnitude } else { magnitude }
    });
    c.fret(angle);
    true
}
