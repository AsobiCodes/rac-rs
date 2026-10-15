//! Camera parameter block helpers (retail 0x1dd8b0..0x1dd924): clear a 5-word block, and copy a 5-word camera
//! parameter block whose first word is an angle wrapped through the angle add.

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Saved-register slots of the retail frame (s0 at 0, s1 at 0x10, ra at 0x20).
const S0: usize = 16;
const S1: usize = 17;
const RA: usize = 31;
const FRAME_SIZE: u32 = 0x30;
const OFF_S0: u32 = 0x0;
const OFF_S1: u32 = 0x10;
const OFF_RA: u32 = 0x20;

/// Byte offset of the last word of the 5-word parameter block.
const PARAM_WORDS: u32 = 5;

/// 0x1dd8b0 cam_params_zero(block) -> clears the five words at block+0..+0x10.
pub fn cam_params_zero(c: &mut Cpu) -> bool {
    let block = c.arg(0);
    for i in 0..PARAM_WORDS {
        c.w32(block + 4 * i, 0);
    }
    true
}

/// 0x1dd8c8 cam_params_copy(dst, src, angle) -> dst word 0 = angle_add(src word 0, angle), then words 1..4 of src
/// copied to dst unchanged.
pub fn cam_params_copy(c: &mut Cpu) -> bool {
    let dst = c.arg(0);
    let src = c.arg(1);
    let angle = c.farg(0);
    h::frame(
        c,
        FRAME_SIZE,
        &[(RA, OFF_RA, h::SQ), (S1, OFF_S1, h::SQ), (S0, OFF_S0, h::SQ)],
        |c, _sp| {
            c.r[S1] = src as i32 as i128 as u128;
            c.r[S0] = dst as i32 as i128 as u128;
            let word0 = c.rf(src);
            c.f[13] = angle.0;
            c.f[12] = word0.0;
            let (_, first) = h::run(c, crate::vecmath::angle_add, &[], &[word0, angle]);
            c.wf(dst, first);
            // Words 1..4 pass through f0 / f1 as retail's lwc1 pairs leave them: f0 = word 3, f1 = word 4.
            let mut words = [F(0); PARAM_WORDS as usize];
            for i in 1..PARAM_WORDS {
                let w = c.rf(src + 4 * i);
                c.wf(dst + 4 * i, w);
                words[i as usize] = w;
            }
            c.fret(words[3]);
            c.f[1] = words[4].0;
        },
    );
    true
}

/// 0x1e9490 empty_hook_tail() -> returns at once (retail is `jr ra` with a nop in the delay slot).
pub fn empty_hook_tail(_c: &mut Cpu) -> bool {
    true
}
