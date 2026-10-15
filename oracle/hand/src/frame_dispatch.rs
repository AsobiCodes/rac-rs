//! Per-frame hero update dispatch: a gate on the hero state, a mode selector (0x1f9ea8, still retail), and the
//! long chain of hero, glove, limb and effect updates in the frame tick (0x1fa178).
use crate::collision;
use crate::feet_ik;
use crate::fx_sway;
use crate::glove_aim;
use crate::glove_gate;
use crate::h;
use crate::hero_hand;
use crate::hero_items;
use crate::hero_state;
use crate::limb_pulse;
use crate::ps2::*;
use crate::spheres;
use ee_oracle::cpu::Cpu;

/// Base of the global block the hero code reads (`lui 0x14` + `addiu -0xcb0`).
const BASE: u32 = 0x0013_F350;
/// Start of the 0x140000 page that the gate reads through (`lui 0x14`).
const PAGE_14: u32 = 0x0014_0000;
/// Word the gate reads at 0x160000 - 0xa54 (signed; positive means active).
const ACTIVE_WORD: u32 = 0x0015_F5AC;
/// Halfword and table tests in the gate.
const HALF_GATE_A: u32 = 0x0014_1D28;
const HALF_GATE_B: u32 = 0x0014_1BE8;
const HALF_GATE_C: u32 = 0x0014_18F0;
/// Byte table indexed by the scan index (`0x140000 - 0x2b40`).
const KEY_BYTES: u32 = 0x0013_D4C0;
/// Word table of 37 entries with a stride of 0x4c, starting at `0x180000 - 0x6440 + 8`.
const WORD_TABLE: u32 = 0x0017_9BC8;
/// Byte flag that selects the slow path in the frame tick (`base + 0x20a4`).
const FLAG_20A4: u32 = BASE + 0x20A4;
/// Float limits for the range test: 2.0 and 1023.0.
const LOW_LIMIT: F = F(0x4000_0000);
const HIGH_LIMIT: F = F(0x447F_8000);

/// Calls a retail function with no arguments and ignores its result.
fn retail(c: &mut Cpu, addr: u32) {
    h::call(c, addr, &[], &[]);
}

/// Calls a hand function with no arguments.
fn hand(c: &mut Cpu, f: fn(&mut Cpu) -> bool) {
    h::run(c, f, &[], &[]);
}

/// 0x1f9da0 dispatch_gate() -> (): when the hero is not blocked, runs the skip gate and the scan of the
/// 37-entry key table, and raises the alert 0x1f69b0 when at least nine keys match.
pub fn dispatch_gate(c: &mut Cpu) -> bool {
    // Retail: when HALF_GATE_A is set, `bnez` jumps past the skip gate and the active-word load straight to the
    // `blez` with v0 still holding the delay-slot load from gp-0x7654. The active test then uses that word.
    let active = if c.r16(HALF_GATE_A) != 0 {
        c.r32((c.g(28) as u32).wrapping_add(-0x7654i32 as u32)) as i32
    } else {
        if c.r8(PAGE_14 - 0x2B3C) != 0 {
            let skip = h::run(c, glove_gate::hero_skip_gate, &[], &[]).0;
            if skip != 0 {
                h::call(c, 0x001F_69B0, &[0x4E2E, 0x78], &[]);
            }
        }
        c.r32(ACTIVE_WORD) as i32
    };
    if active <= 0 || active & 0x7F != 0 {
        return true;
    }
    if c.r16(HALF_GATE_B) != 0 || c.r16(HALF_GATE_C) != 0 {
        return true;
    }
    let mut matches = 0u32;
    for index in 0..0x25u32 {
        let word = c.r32(WORD_TABLE + 0x4C * index);
        // Entries 8 and 0x18 are never counted.
        if word == 0 && index != 8 && index != 0x18 && c.r8(KEY_BYTES + index) != 0 {
            matches += 1;
        }
    }
    if matches >= 9 {
        h::call(c, 0x001F_69B0, &[0x4E26, 0x50], &[]);
    }
    true
}

/// 0x1f9ea8 mode_select() -> (): mode selector, still retail; declined.
pub fn mode_select(_c: &mut Cpu) -> bool {
    false
}

/// 0x1fa178 frame_dispatch() -> (): when the hero's two float values are inside (2.0, 1023.0), handles the
/// quick path; otherwise runs the full chain of hero, glove, limb and effect updates.
pub fn frame_dispatch(c: &mut Cpu) -> bool {
    // Declined: the full chain still leaves RAM bytes that differ from retail on s07/s08 (unexplained), so retail runs it.
    let _ = c;
    return false;
    #[allow(unreachable_code)]
    let f1 = c.rf(BASE + 0x80);
    let f0 = c.rf(BASE + 0x84);
    if f1 < LOW_LIMIT || f0 < LOW_LIMIT || HIGH_LIMIT < f1 || HIGH_LIMIT < f0 {
        let n = h::scale_int(c, 16);
        h::call(c, 0x001E_C0A0, &[n as u32], &[]);
        c.w8(BASE + 0x20B1, 1);
        return true;
    }
    c.w16(BASE + 0x1EB2, 0);
    let flag = c.r8(FLAG_20A4);
    c.w16(BASE + 0x1EB0, 0);
    if flag != 0 {
        retail(c, 0x001F_9358);
        return true;
    }
    for addr in [0x0022_57B0, 0x0020_4718, 0x001F_EA08, 0x001F_EF78, 0x0020_0000, 0x0020_03D8, 0x0021_CCB0] {
        retail(c, addr);
    }
    if c.r8(FLAG_20A4) != 0 {
        return true;
    }
    retail(c, 0x0020_A050);
    hand(c, feet_ik::foot_ik_blocks);
    hand(c, glove_aim::glove_aim_block);
    retail(c, 0x001F_D5E8);
    retail(c, 0x001F_DD40);
    hand(c, glove_aim::aim_jitter_tick);
    retail(c, 0x001F_8750);
    retail(c, 0x001F_8A80);
    retail(c, 0x0020_8128);
    retail(c, 0x001F_8590);
    hand(c, hero_state::hero_pose_sync);
    h::run(c, hero_hand::hand_pose_chain, &[0], &[]);
    h::run(c, hero_items::hero_frame_update, &[0], &[]);
    retail(c, 0x0020_1AF8);
    hand(c, limb_pulse::pulse_timer_step);
    hand(c, limb_pulse::limb_pulse);
    hand(c, dispatch_gate);
    retail(c, 0x001F_8F50);
    retail(c, 0x001F_9128);
    h::call(c, 0x001F_9EA8, &[], &[]);
    let g2080 = c.r32(BASE + 0x2080);
    h::run(c, collision::empty_hook, &[g2080], &[]);
    if c.r32(BASE + 0x208C) != 0x16 && c.r32(BASE + 0x2084) != 0x32 {
        h::call(c, 0x0023_AA38, &[BASE + 0x80], &[]);
    }
    let g2080 = c.r32(BASE + 0x2080);
    h::run(c, spheres::zone_tint, &[g2080], &[]);
    hand(c, fx_sway::fx_tint_update);
    hand(c, hero_hand::hero_hand_pose);
    true
}
