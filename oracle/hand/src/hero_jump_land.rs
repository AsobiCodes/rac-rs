//! Hero landing-end and jump-request checks (retail 0x21c4e8 and 0x21c7a0). Both read the hero record at 0x13f350,
//! then either start a hero animation or return a yes/no flag.
use crate::glove_slot;
use crate::h;
use crate::hero_anim;
use crate::particles;
use crate::path;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Hero record base (0x140000 - 0xcb0). The OFF_ constants below are offsets from it.
const HERO: u32 = 0x0013_f350;
/// Global flags word (0x140000 - 0x3520). Bits 0xa gate the landing and jump paths.
const FLAGS_WORD: u32 = 0x0013_cae0;
/// Global flag byte (0x140000 - 0x2b3e).
const GLOBAL_FLAG: u32 = 0x0013_d4c2;
/// Word holding the landing animation mode (0x160000 - 0x944).
const LANDING_MODE: u32 = 0x0015_f6bc;
/// Float scaling the clamp length (0x160000 - 0x1294).
const CLAMP_SCALE: u32 = 0x0015_ed6c;
/// Vector clamped by the landing-stop path (0x140000 - 0xb60).
const CLAMP_VEC: u32 = 0x0013_f4a0;
/// Frame constant read from gp (gp - 0x7e94).
const GP_FLOAT_OFF: u32 = 0x7e94;

const OFF_164: u32 = 0x164;
const OFF_168: u32 = 0x168;
const OFF_198: u32 = 0x198;
const OFF_1B8: u32 = 0x1b8;
const OFF_1C4: u32 = 0x1c4;
const OFF_2084: u32 = 0x2084;
const OFF_2088: u32 = 0x2088;
const OFF_2284: u32 = 0x2284;
const OFF_229C: u32 = 0x229c;
const OFF_22D8: u32 = 0x22d8;
const OFF_248: u32 = 0x248;
const OFF_250: u32 = 0x250;
const OFF_254: u32 = 0x254;
const OFF_12E2: u32 = 0x12e2;
const OFF_20A9: u32 = 0x20a9;
const OFF_3BC: u32 = 0x3bc;
const OFF_418: u32 = 0x418;
const OFF_A98: u32 = 0xa98;

const HALF: F = F(0x3f00_0000);
const ONE: F = F(0x3f80_0000);
const POINT_THREE: F = F(0x3e99_999a);
const POINT_SEVEN: F = F(0x3f33_3333);
const POINT_NINE: F = F(0x3f66_6666);
const THREE: F = F(0x4040_0000);
const TWO_AND_HALF: F = F(0x4020_0000);
const FIVE: F = F(0x40a0_0000);
const NEG_ONE: F = F(0xbf80_0000);
const NEG_TWO: F = F(0xc000_0000);
const QUARTER_PI: F = F(0x3f49_0fdb);

/// Reads the gp-relative frame constant used by both checks.
fn gp_float(c: &mut Cpu) -> F {
    let gp = c.g(28) as u32;
    c.rf(gp.wrapping_sub(GP_FLOAT_OFF))
}

/// 0x215538(a0, a1): retail helper, returns v0.
fn helper_215538(c: &mut Cpu, a0: u32, a1: u32) -> u32 {
    h::call(c, 0x215538, &[a0, a1], &[]).0
}

/// 0x1f80a8(0): retail helper, returns v0.
fn helper_1f80a8(c: &mut Cpu) -> u32 {
    h::call(c, 0x1f80a8, &[0], &[]).0
}

/// Starts hero animation `anim` in `mode` at `speed` via retail hero_set_anim (0x2254f8);
/// keeps its scratch registers and returns its v0.
fn set_anim(c: &mut Cpu, anim: u32, mode: u32, speed: F) -> u32 {
    h::call_keep(c, 0x2254f8, &[anim, mode], &[speed]).0
}

/// Glove slot lookup (0x200540) for slot `index`; returns v0.
fn slot_value_b(c: &mut Cpu, index: u32) -> u32 {
    h::run(c, glove_slot::slot_value_b_fn, &[index], &[]).0
}

/// Shared tail of the jump paths: 0x215538(a0, 1), then the caller returns 1.
fn call_then_one(c: &mut Cpu, a0: u32) -> u32 {
    helper_215538(c, a0, 1);
    1
}

/// 0x21c4e8 landing_end(): hero landing end. Starts the landing animation and returns v0.
pub fn landing_end(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let v0 = landing_body(c);
    c.ret(v0);
    true
}

fn landing_body(c: &mut Cpu) -> u32 {
    c.w32(HERO + OFF_2284, 0);
    if c.r8(HERO + OFF_12E2) != 0 && gp_float(c) * HALF < c.rf(HERO + OFF_164) {
        return helper_215538(c, 0x2f, 1);
    }
    h::call(c, 0x204970, &[0], &[ONE]);
    if (c.r32(FLAGS_WORD) & 0xa) != 0 {
        helper_215538(c, 4, 0);
        set_anim(c, 0xd, 2, NEG_TWO);
        return h::run(c, hero_anim::hero_loop_range, &[3, 0x12], &[]).0;
    }
    if POINT_THREE < c.rf(HERO + OFF_229C) && c.r32(HERO + OFF_1C4) == 0 {
        if c.r8(HERO + OFF_12E2) != 0 || c.r8(HERO + OFF_20A9) != 0 {
            return helper_215538(c, 2, 1);
        }
        if helper_215538(c, 2, 0) == 0 {
            return 0;
        }
        return landing_motion(c);
    }
    landing_tail(c)
}

/// Landing path after the sensor checks: marks the hero, picks the landing speed and starts the animation.
fn landing_motion(c: &mut Cpu) -> u32 {
    c.w32(HERO + OFF_2088, 1);
    let mut speed_steps = h::scale_int(c, 8);
    if h::scale_int(c, 5) < c.r32(HERO + OFF_418) as i32 {
        speed_steps = h::scale_int(c, 0xe);
    }
    let mode = c.r32(LANDING_MODE);
    let v0 = set_anim(c, 4, mode, F::from_int(speed_steps));
    c.w16(HERO + OFF_3BC, 1);
    v0
}

/// Tail of landing_end: a gp-scaled height test chooses between the stop path and the selector.
fn landing_tail(c: &mut Cpu) -> u32 {
    let mut low = false;
    if gp_float(c) * THREE < c.rf(HERO + OFF_168) {
        low = h::scale_int(c, 3) < c.r32(HERO + OFF_418) as i32;
    }
    if low || c.r32(HERO + OFF_2084) == 0x10 {
        return landing_stop(c);
    }
    landing_select(c)
}

/// Stop path: restarts the hero animation, then clamps the hero's aim vector to a scaled length.
fn landing_stop(c: &mut Cpu) -> u32 {
    if helper_215538(c, 3, 0) == 0 {
        return 0;
    }
    set_anim(c, 5, 0, NEG_ONE);
    let len = c.rf(CLAMP_SCALE) * FIVE;
    h::run(c, path::vec_clamp_len, &[CLAMP_VEC, 0], &[len]).0
}

/// Selector path: a scaled-height test and the 0x54 animation choice.
fn landing_select(c: &mut Cpu) -> u32 {
    let steps = h::scale_int(c, 0xc);
    let lower = steps < c.r32(HERO + OFF_418) as i32;
    if !lower && (c.r32(HERO + OFF_A98) & 2) == 0 {
        return 0;
    }
    if helper_215538(c, 0, 0) == 0 {
        return 0;
    }
    if helper_1f80a8(c) == 0x54 {
        let anim = helper_1f80a8(c);
        let speed = h::scale_int(c, 0xf);
        return set_anim(c, anim, 0, F::from_int(speed));
    }
    if (c.r32(HERO + OFF_A98) & 2) == 0 {
        return 0;
    }
    let anim = helper_1f80a8(c);
    set_anim(c, anim, 0, NEG_TWO)
}

/// 0x21c7a0 jump_request(): hero jump request. Starts a jump animation when the gates pass; returns 1 when it
/// handled the request, 0 otherwise.
pub fn jump_request(c: &mut Cpu) -> bool {
    let v0 = jump_body(c);
    c.ret(v0);
    true
}

fn jump_body(c: &mut Cpu) -> u32 {
    if c.r32(HERO + OFF_2084) == 0xe {
        return 0;
    }
    let s0 = h::call(c, 0x214ed8, &[0xe], &[]).0 as i32;
    let a2 = h::scale_int(c, 5);
    if (c.r32(FLAGS_WORD) & 0xa) != 0 && s0 > 0 {
        let hit = h::call(c, 0x258f80, &[0x40, 0x1f000, a2.wrapping_add(s0) as u32, s0 as u32], &[]).0;
        if hit != 0 {
            return call_then_one(c, 0xb);
        }
    }
    let steps = h::scale_int(c, 9);
    if h::scale_int(c, 6) >= c.r32(HERO + OFF_198) as i32 {
        return 0;
    }
    if h::run(c, particles::input_history_scan, &[0x40, steps as u32, 0], &[]).0 == 0 {
        return 0;
    }
    if slot_value_b(c, 3) == 2 {
        if let Some(v0) = jump_branch_a(c) {
            return v0;
        }
    }
    jump_branch_b(c)
}

/// Slot-2 path: returns Some(v0) when it starts the 0xa call, None to fall through to the next checks.
fn jump_branch_a(c: &mut Cpu) -> Option<u32> {
    if c.r8(GLOBAL_FLAG) == 0 || c.r16(HERO + OFF_22D8) != 0 {
        return None;
    }
    let height = if TWO_AND_HALF < c.rf(HERO + OFF_248) {
        c.rf(HERO + OFF_229C)
    } else {
        if !(c.rf(HERO + OFF_250) < QUARTER_PI) {
            return None;
        }
        c.rf(HERO + OFF_229C)
    };
    if !(POINT_SEVEN < height) {
        return None;
    }
    if gp_float(c) * POINT_NINE < c.rf(HERO + OFF_168) {
        return Some(call_then_one(c, 0xa));
    }
    None
}

/// Slot-3 path: when the gates pass, returns 1 directly or starts the 0x10 call.
fn jump_branch_b(c: &mut Cpu) -> u32 {
    if slot_value_b(c, 3) == 3 && c.r16(HERO + OFF_22D8) == 0 && POINT_SEVEN < c.rf(HERO + OFF_229C) {
        let gated = TWO_AND_HALF < c.rf(HERO + OFF_248) || c.r8(HERO + OFF_254) != 0;
        if gated && gp_float(c) * POINT_NINE < c.rf(HERO + OFF_168) {
            if c.r32(HERO + OFF_1B8) != 0 {
                return 1;
            }
            return call_then_one(c, 0x10);
        }
    }
    jump_branch_c(c)
}

/// Last slot checks: every remaining path ends in 0x215538(a0, 1) and returns 1.
fn jump_branch_c(c: &mut Cpu) -> u32 {
    if slot_value_b(c, 3) == 3 && c.r16(HERO + OFF_22D8) == 0 {
        return call_then_one(c, 0xd);
    }
    if slot_value_b(c, 3) == 2 && c.r16(HERO + OFF_22D8) == 0 && c.r8(GLOBAL_FLAG) != 0 {
        return call_then_one(c, 0xf);
    }
    call_then_one(c, 7)
}
