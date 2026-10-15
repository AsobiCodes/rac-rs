//! Bolt pickup prompts: the aridia flower gate, the timed text popup, the forced prompt slot and the spare-gold clamp.

use crate::cue_tables;
use crate::h::{self, SQ};
use crate::hud_bolts;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Position the flower gate measures distance against (a fixed RAM point, read at call time).
const FLOWER_POINT: u32 = 0x13_F3D0;
/// Distance (7.0) under which the flower gate asks for the bolts.
const FLOWER_GATE: F = F(0x40e0_0000);
/// Retail function that spawns the bolt drop (actor, 4, 7, count, -1).
const BOLT_SPAWN: u32 = 0x25_3330;
/// Popup timer: ticks of -1 mean PAL ticks for 180 frames; the text id goes through 0x1f6b80, then 0x1165b8 shows it.
const TICKS_SENTINEL: i32 = -1;
const POPUP_PAL_TICKS: i32 = 180;
const POPUP_TEXT_TABLE: u32 = 0x17_9218;
const POPUP_TICKS_SLOT: u32 = 0x15_F620;
const POPUP_SHOW: u32 = 0x11_65b8;
/// Prompt slot globals written by the forced prompt.
const PROMPT_SLOT: u32 = 0x15_F574;
const PROMPT_KIND: u32 = 0x15_F570;
const PROMPT_TEXT: u32 = 0x16_1F08;
const PROMPT_KIND_FORCED: u32 = 2;
const PROMPT_RESULT_DONE: u32 = 3;
/// Request counter at gp-0x7690 (kept while the same owner holds the slot).
const REQUEST_COUNTER_GP: i32 = -0x7690;
/// Retail helper that fills the bolt name (the callee's own name is a hand).
const NAME_FETCH: u32 = 0x1f_6b80;
/// Retail helper that runs once before the gold count (its result is unused).
const GOLD_PRE: u32 = 0x25_6938;
/// Gold clamp: the result is limited to 0..=40 (0x28).
const GOLD_MAX: i32 = 0x28;

/// 0x255ff0 flower_near_bolt_drop(actor): spawns the bolt drop with count 2 when the XY distance to the point is
/// under 7.0 and 0 otherwise (range 4..7); returns the spawner's result.
pub fn flower_near_bolt_drop(c: &mut Cpu) -> bool {
    let actor = c.arg(0);
    h::frame(c, 0x30, &[(16, 0x0, SQ), (17, 0x10, SQ), (31, 0x20, SQ)], |c, _sp| {
        let dist = h::dist_xy(c, actor.wrapping_add(0x10), FLOWER_POINT);
        let count = if dist < FLOWER_GATE { 2 } else { 0 };
        let r = h::call(c, BOLT_SPAWN, &[actor, 4, 7, count, TICKS_SENTINEL as u32], &[]);
        c.ret(r.0);
    });
    true
}

/// 0x256060 timed_text_popup(id, ticks): shows text `id` for `ticks` frames (-1 means 180 PAL ticks); the ticks
/// are kept in a global and the popup call returns its result.
pub fn timed_text_popup(c: &mut Cpu) -> bool {
    let (id, ticks_arg) = (c.arg(0), c.arg(1));
    h::frame(c, 0x40, &[(16, 0x0, SQ), (17, 0x10, SQ), (18, 0x20, SQ), (31, 0x30, SQ)], |c, _sp| {
        let ticks = if ticks_arg as i32 == TICKS_SENTINEL {
            h::scale_int(c, POPUP_PAL_TICKS)
        } else {
            ticks_arg as i32
        };
        let name = h::call(c, NAME_FETCH, &[id], &[]).0;
        let r = h::call(c, POPUP_SHOW, &[POPUP_TEXT_TABLE, name], &[]);
        c.w32(POPUP_TICKS_SLOT, ticks as u32);
        c.ret(r.0);
    });
    true
}

/// 0x256820 force_prompt_slot(slot, text): when the request is refused (0 from `request_slot`), forces the prompt box
/// owner slot and text (a nonzero text also sets the bolt name) and returns 3; otherwise returns the request's v0.
pub fn force_prompt_slot(c: &mut Cpu) -> bool {
    let (slot, text) = (c.arg(0), c.arg(1));
    h::frame(c, 0x30, &[(16, 0x0, SQ), (17, 0x10, SQ), (31, 0x20, SQ)], |c, _sp| {
        let request = request_slot(c, slot, text);
        if request != 0 {
            c.ret(request);
            return;
        }
        if text != 0 {
            let name = h::call(c, NAME_FETCH, &[text], &[]).0;
            h::run(c, hud_bolts::hud_name_set, &[name], &[]);
        }
        c.w32(PROMPT_SLOT, slot);
        c.w32(PROMPT_KIND, PROMPT_KIND_FORCED);
        c.w32(PROMPT_TEXT, text);
        c.ret(PROMPT_RESULT_DONE);
    });
    true
}

/// gp-relative address: gp + off.
fn gp(c: &Cpu, off: i32) -> u32 {
    (c.g(28) as u32).wrapping_add(off as u32)
}

/// 0x256778 fx_request(owner, arg) -> v0, as retail leaves it: 0 when another owner holds the slot (nothing written),
/// else 1 when the slot was free (owner, counter 2 and the text are stored) or 2 when `owner` already held it.
/// A non-zero `arg` is announced through 0x1f6b80 and the hud name setter first.
fn request_slot(c: &mut Cpu, owner: u32, arg: u32) -> u32 {
    let cur = c.r32(PROMPT_SLOT);
    if cur != owner && cur != 0 {
        return 0;
    }
    if arg != 0 {
        let name = h::call(c, NAME_FETCH, &[arg], &[]).0;
        h::run(c, hud_bolts::hud_name_set, &[name], &[]);
    }
    if cur == owner {
        c.w32(PROMPT_TEXT, arg);
        c.w32(gp(c, REQUEST_COUNTER_GP), 2);
        return 2;
    }
    c.w32(PROMPT_SLOT, owner);
    c.w32(PROMPT_KIND, 2);
    c.w32(PROMPT_TEXT, arg);
    1
}

/// 0x2568f0 spare_gold_bolts() -> i32: runs the retail pre-step, then the gold count n gives clamp(n - 4*n, 0, 40)
/// (the value is `-3n` clamped to 0..=40 as retail's slt/movz/movn chain computes it).
pub fn spare_gold_bolts(c: &mut Cpu) -> bool {
    h::frame(c, 0x20, &[(31, 0x10, SQ), (16, 0x0, SQ)], |c, _sp| {
        h::call_keep(c, GOLD_PRE, &[], &[]);
        let n = h::run_keep(c, cue_tables::gold_bolts_spent, &[], &[]).0 as i32;
        let mut s0 = n.wrapping_sub(n << 2);
        let v1 = (-1 < s0) as i32;
        if v1 == 0 {
            s0 = 0;
        }
        let a0 = (s0 < 0x29) as i32;
        let v0 = if a0 != 0 { s0 } else { GOLD_MAX };
        c.s(3, v1 as i64 as u64);
        c.s(4, a0 as i64 as u64);
        c.ret(v0 as u32);
    });
    true
}
