//! Wallop feedback scan, ledge mode cue and held-moby release (retail 0x20a4a8, 0x21a778, 0x21c0e8).
use crate::actor;
use crate::h;
use crate::hero_space;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Global block (retail `lui 0x14` plus `-0xcb0`).
const GLOBALS: u32 = 0x13_f350;
/// One-time setup flag (set after the first 0x27e470 call).
const SETUP_FLAG: u32 = GLOBALS + 0xa64;
/// Held moby pointer, and the two moby slots released with it.
const HELD_MOBY: u32 = GLOBALS + 0x1614;
const MOBY_SLOT_A: u32 = GLOBALS + 0x1180;
const MOBY_SLOT_B: u32 = GLOBALS + 0x1184;
/// Ledge mode byte (0x1413f4).
const LEDGE_MODE: u32 = 0x14_13f4;
/// Victim table of pointers (0x178000); each victim's class is read through +0x24 and +0x46.
const VICTIM_TABLE: u32 = 0x17_8000;

/// Class types that get the feedback spark.
const CLASS_A: i32 = 5;
const CLASS_B: i32 = 9;
/// Offset from a victim to its quadword position, and from its sub-record to the class halfword.
const VICTIM_POS: u32 = 0x10;
const VICTIM_SUB: u32 = 0x24;
const SUB_CLASS: u32 = 0x46;
/// Offset from a moby to its flag halfword, and the bits cleared there (0x41).
const MOBY_FLAGS: u32 = 0x34;
const FLAG_CLEAR_MASK: u32 = 0xffbe;

/// The float 0.5 (f12 for up_offset_neg).
const HALF: u32 = 0x3f00_0000;

/// Retail routines called by these hands.
const SETUP_CALL: u32 = 0x27_e470;
const CUE_CALL: u32 = 0x21_5538;
const QUAD_CALL: u32 = 0x29_9950;

/// Class type of a victim: 0 when the victim or its sub-record is missing, else the signed halfword at +0x46.
fn victim_class(c: &mut Cpu, victim: u32) -> i32 {
    if victim == 0 {
        return 0;
    }
    let sub = c.r32(victim + VICTIM_SUB);
    if sub == 0 {
        return 0;
    }
    c.r16(sub + SUB_CLASS) as i16 as i32
}

/// 0x20a4a8 wallop_feedback_scan(a0 source, a1 count): on the first call runs 0x27e470 once; then for each of the
/// first `count` victims of class 5 or 9, copies the quad at +0x10 to a stack buffer, moves it up by 0.5
/// (hero_space::up_offset_neg) and passes the buffer to 0x299950.
pub fn wallop_feedback_scan(c: &mut Cpu) -> bool {
    let (source, count) = (c.arg(0), c.arg(1) as i32);
    let saves = [(16, 0x10, h::SQ), (17, 0x20, h::SQ), (18, 0x30, h::SQ), (19, 0x40, h::SQ), (31, 0x50, h::SQ)];
    h::frame(c, 0x60, &saves, |c, sp| {
        if count == 0 {
            return;
        }
        if c.r32(SETUP_FLAG) == 0 {
            h::call_keep(c, SETUP_CALL, &[1, 0, source], &[]);
            c.w32(SETUP_FLAG, 1);
            c.s(3, 1);
        }
        if count <= 0 {
            return;
        }
        for i in 0..count as u32 {
            let victim = c.r32(VICTIM_TABLE + 4 * i);
            let class = victim_class(c, victim);
            c.s(2, class as i64 as u64);
            if class == CLASS_A || class == CLASS_B {
                let quad = c.mem.rd128(victim + VICTIM_POS);
                c.mem.wr128(sp, quad);
                // Retail's lq/addiu leave v0 = the quad and v1 = its address (the callees keep what they leave).
                c.r[2] = quad;
                c.s(3, (victim + VICTIM_POS) as u64);
                h::run_keep(c, hero_space::up_offset_neg, &[sp, sp], &[F(HALF)]);
                h::call_keep(c, QUAD_CALL, &[sp, 4], &[]);
            }
        }
    });
    true
}

/// 0x21a778 ledge_mode_cue(): with the ledge mode byte 0 to 3 plays cue 0, 0x43, 0x5a or 0x53 through 0x215538
/// (argument 1); other modes play nothing.
pub fn ledge_mode_cue(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let mode = c.r8(LEDGE_MODE) as u32;
    let cue = match mode {
        0 => Some(0),
        1 => Some(0x43),
        2 => Some(0x5a),
        3 => Some(0x53),
        _ => None,
    };
    h::frame(c, 0x10, &[(31, 0, h::SQ)], |c, _| match cue {
        Some(code) => {
            h::call_keep(c, CUE_CALL, &[code, 1], &[]);
        }
        None => {
            // Retail leaves v0 = 3, v1 = 1 and a0 = the mode byte on this path.
            c.s(2, 3);
            c.s(3, 1);
            c.s(4, mode as u64);
        }
    });
    true
}

/// 0x21c0e8 release_held_moby(): frees the held moby through 0x2410e8 when one is set, then clears the bits 0x41 of
/// the flag halfword (+0x34) of the two moby slots that are set.
pub fn release_held_moby(c: &mut Cpu) -> bool {
    let saves = [(16, 0, h::SQ), (31, 0x10, h::SQ)];
    h::frame(c, 0x20, &saves, |c, _| {
        let held = c.r32(HELD_MOBY);
        if held != 0 {
            h::run(c, actor::free_moby, &[held], &[]);
            c.w32(HELD_MOBY, 0);
        }
        for slot in [MOBY_SLOT_A, MOBY_SLOT_B] {
            let moby = c.r32(slot);
            if moby != 0 {
                let flags = c.r16(moby + MOBY_FLAGS) as u32 & FLAG_CLEAR_MASK;
                c.w16(moby + MOBY_FLAGS, flags as u16);
            }
        }
    });
    true
}
