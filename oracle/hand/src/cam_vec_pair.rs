//! Retail functions 0x2ed268 and 0x2ed2f0: a call passthrough and the camera vector setup it sits beside.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// The 128 bits of a GPR as a vector, for sq / lq.
fn q_to_v(q: u128) -> V {
    V(std::array::from_fn(|i| (q >> (32 * i)) as u32))
}

/// Frame size of 0x2ed2f0 (addiu sp, -0x50).
const FRAME: u32 = 0x50;
/// Camera state block: lui 0x14 + addiu -0xcb0.
const CAM_BLOCK: u32 = 0x13_f350;
/// Second table pointer: lui 0x14 + addiu -0xc30.
const CAM_TABLE: u32 = 0x13_f3d0;
/// Mode byte inside the camera block (0x20a4 past CAM_BLOCK).
const MODE_OFF: u32 = 0x20a4;
/// Length word inside the camera block (0x2080 past CAM_BLOCK).
const LEN_OFF: u32 = 0x2080;
/// Flag byte at 0x15edb3 (lui 0x16 + lbu -0x124d).
const FLAG_ADDR: u32 = 0x15_edb3;
/// Retail callee that copies / sets a vector length (hand: vu_vec::vec_set_length).
const VEC_LEN_TARGET: u32 = 0x1f_1a98;
/// Plain copy helper called twice at the end of 0x2ed2f0.
const PAIR_COPY: u32 = 0x1f_1810;
/// Frame slot offsets (sq s0 / s1 / ra).
const SLOT_S0: u32 = 0x20;
const SLOT_S1: u32 = 0x30;
const SLOT_RA: u32 = 0x40;
/// Scratch vector on the stack, passed to vec_scale and PAIR_COPY.
const SCRATCH_VEC: u32 = 0x10;

/// 0x2ed268 passthrough_2ed038() -> void: saves ra in a 16-byte frame, calls 0x2ed038, restores ra and returns.
pub fn passthrough_2ed038(c: &mut Cpu) -> bool {
    let sp = (c.g(29) as u32).wrapping_sub(0x10);
    let ra = c.r[31];
    c.wv(sp, q_to_v(ra));
    let _ = h::call_keep(c, 0x2ed038, &[], &[]);
    true
}

/// 0x2ed2f0 camera_vec_setup(a0 = out) -> void: sets a unit-length stack vector from the camera length word, scales
/// it by a mode-dependent factor, then writes two packed copies to out+0x30 and CAM_TABLE.
pub fn camera_vec_setup(c: &mut Cpu) -> bool {
    let out = c.arg(0);
    let frame = (c.g(29) as u32).wrapping_sub(FRAME);
    let s0_val = c.r[16];
    let s1_val = c.r[17];
    let ra_val = c.r[31];
    c.wv(frame.wrapping_add(SLOT_S0), q_to_v(s0_val));
    c.wv(frame.wrapping_add(SLOT_S1), q_to_v(s1_val));
    c.wv(frame.wrapping_add(SLOT_RA), q_to_v(ra_val));

    // Length = camera length word + 0xe0, written as a vector of length 1.0 at the frame base.
    let len_word = c.r32(CAM_BLOCK.wrapping_add(LEN_OFF));
    let len_arg = len_word.wrapping_add(0xe0);
    let _ = h::run_keep(c, crate::vu_vec::vec_set_length, &[frame, len_arg], &[F(0x3f80_0000)]);

    // Scaled copy of the scratch vector; the factor depends on the mode byte.
    let scratch = frame.wrapping_add(SCRATCH_VEC);
    let mode = c.r8(CAM_BLOCK.wrapping_add(MODE_OFF));
    let factor = match mode {
        2 => F(0x4118_0000), // 9.5
        1 => {
            if c.r8(FLAG_ADDR) != 0 {
                F(0x3f99_999a) // 1.2
            } else {
                F(0x3f66_6666) // 0.9
            }
        }
        _ => F(0x3fcc_cccd), // 1.6
    };
    let _ = h::run_keep(c, crate::vecmath::vec_scale, &[scratch, frame], &[factor]);

    // Every path leaves a2 = scratch; the two PAIR_COPY calls below use it.
    let s1 = out.wrapping_add(0x30);
    let _ = h::call_keep(c, PAIR_COPY, &[s1, CAM_TABLE, scratch], &[]);
    let _ = h::call_keep(c, PAIR_COPY, &[s1, CAM_TABLE.wrapping_add(0xc0), s1], &[]);
    true
}
