//! Per-frame effects pass (retail 0x271020): does nothing while the object's state word is below 4; otherwise resets
//! the fx frame, logs both sample rings, blits two sprite text rows with the same packed header and tags the packets.

use crate::h;
use crate::ps2::*;
use crate::{fxtex, sample_ring, sprite_text};
use ee_oracle::cpu::Cpu;

/// Bytes of the retail stack frame.
const FRAME: u32 = 0x40;
/// Frame slots for the saved s0, s1 and ra (retail `sq` offsets).
const SAVE_S0: u32 = 0x10;
const SAVE_S1: u32 = 0x20;
const SAVE_RA: u32 = 0x30;
/// Object fields: the state word gates the pass, the two ring pointers feed the sample logs.
const STATE_WORD: u32 = 0x44;
const RING_A: u32 = 0x48;
const RING_B: u32 = 0x4c;
/// The pass runs only when the state word is at least this (signed compare).
const STATE_MIN: i32 = 4;
/// Packed header word stored in the two stack argument slots (retail's s0 = 0x80808080).
const HEADER: u64 = 0x8080_8080;
/// Stack slots of the two stack arguments of sprite_blit_int (sp+0 header, sp+8 ring result).
const ARG_HEADER: u32 = 0x00;
const ARG_RING: u32 = 0x08;
/// Sprite blit arguments: the first call draws at origin 0, the second at 0x100 (same size both times).
const SIZE: u32 = 0x100;
const ORIGIN_FIRST: u32 = 0;
const ORIGIN_SECOND: u32 = 0x100;
/// Retail fx_pkt_tags (its Rust hand is not used here: the retail leftovers are what the caller sees).
const FX_PKT_TAGS: u32 = 0x001e_b9e0;
/// Return value when the pass runs.
const RESULT_RAN: u32 = 8;

/// 0x271020 fx_frame_pass(obj) -> v0: skips when obj.state < 4 (returns 0); else resets the fx frame, logs both
/// sample rings, blits two sprite rows and tags the packets (returns 8)
pub fn fx_frame_pass(c: &mut Cpu) -> bool {
    if h::unwind_pending(c) {
        return false;
    }
    let obj = c.arg(0);
    let saves = [(16, SAVE_S0, h::SQ), (17, SAVE_S1, h::SQ), (31, SAVE_RA, h::SQ)];
    h::frame(c, FRAME, &saves, |c, sp| {
        if (c.r32(obj.wrapping_add(STATE_WORD)) as i32) < STATE_MIN {
            c.ret(0);
            return;
        }
        h::run_keep(c, fxtex::fx_frame_reset, &[0], &[]);
        // Retail's delay slots load a0 from the object before each ring_push call.
        let ring_a = c.r32(obj.wrapping_add(RING_A));
        h::run_keep(c, sample_ring::ring_push, &[ring_a], &[]);
        c.mem.wr64(sp + ARG_HEADER, HEADER);
        c.mem.wr64(sp + ARG_RING, c.r[2] as u64);
        h::run_keep(
            c,
            sprite_text::sprite_blit_int,
            &[ORIGIN_FIRST, 0, SIZE, SIZE, 0, 0, SIZE, SIZE],
            &[],
        );

        let ring_b = c.r32(obj.wrapping_add(RING_B));
        h::run_keep(c, sample_ring::ring_push, &[ring_b], &[]);
        c.mem.wr64(sp + ARG_RING, c.r[2] as u64);
        c.mem.wr64(sp + ARG_HEADER, HEADER);
        h::run_keep(
            c,
            sprite_text::sprite_blit_int,
            &[ORIGIN_SECOND, 0, SIZE, SIZE, 0, 0, SIZE, SIZE],
            &[],
        );

        // fx_pkt_tags takes the registers the second blit left (no new arguments).
        h::call_keep(c, FX_PKT_TAGS, &[], &[]);
        c.ret(RESULT_RAN);
    });
    true
}
