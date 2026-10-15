//! Hero foot-ground probe, footstep triggers and a slot release loop. Retail globals live at 0x13f350; the hero
//! actor pointer is at +0x2080 and the heading angle at +0x98.
use crate::actor;
use crate::h;
use crate::mathx;
use crate::ps2::*;
use crate::sweep;
use crate::trig;
use ee_oracle::cpu::Cpu;

/// Retail globals block (lui 0x14, addiu -0xcb0).
const GLOBALS: u32 = 0x13_F350;
/// Game mode word; the footstep path only runs in mode 12.
const MODE_ADDR: u32 = 0x15_ED84;
/// Mode word read by hero_footsteps (offset from GLOBALS).
const STATE_2084: u32 = 0x2084;
/// Pointer to the hero actor.
const HERO_PTR: u32 = 0x2080;
/// Pointer whose halfword +0xa6 gates the first retail call.
const HOOK_PTR: u32 = 0x10E0;
/// Heading angle (float) fed to cos/sin.
const HEADING: u32 = 0x98;
/// Hold flag byte; any value blocks the probe.
const HOLD_BYTE: u32 = 0x12ED;
/// Hold flag word; any value blocks the probe.
const HOLD_WORD: u32 = 0x2FC;
/// Counter compared against the scaled thresholds in hero_footsteps.
const COUNTER_198: u32 = 0x198;
/// Word that must be zero for footstep triggers in state 3 and 4.
const BLOCK_A9C: u32 = 0xA9C;
/// Slot wanted-id table (8 words).
const SLOT_WANT: u32 = 0x2238;
/// Slot index table (8 words, -1 = empty).
const SLOT_INDEX: u32 = 0x2218;
/// Slot entry table base (0x70-byte entries, relative to 0x140000 - 0x1ab0).
const SLOT_ENTRIES: u32 = 0x13_E550;
/// Size of one slot entry.
const SLOT_SIZE: u32 = 0x70;

const F_POSE_SCALE: F = F(0x3e19_999a);
const F_HALF: F = F(0x3f00_0000);
const F_GROUND_OFFSET: F = F(0x3d23_d70a);

/// Reads a word from the globals block.
fn g32(c: &mut Cpu, off: u32) -> u32 {
    c.r32(GLOBALS + off)
}

/// Calls retail 0x204918 with one float argument; returns its v0 (non-zero when the test passes).
fn probe_hit(c: &mut Cpu, bits: u32) -> u32 {
    h::call(c, 0x204918, &[], &[F(bits)]).0
}

/// 0x1f9008 foot_ground_probe(a0, a1, a2): when a2 != 0 first runs retail 0x27e668. Then, if the game mode is 12 and
/// no hold flag is set, takes the hero position from pose_node_pos (node a1 + 0x16), nudges it by the heading's
/// cos and sin, drops it to ground height and calls retail 0x266fa8 with a1 == 0 as the flag.
pub fn foot_ground_probe(c: &mut Cpu) -> bool {
    let (a0, a1, a2) = (c.arg(0), c.arg(1), c.arg(2));
    if a2 != 0 {
        let hook = g32(c, HOOK_PTR);
        let flag = hook != 0 && c.r16(hook + 0xA6) == 0xAD;
        let hero = g32(c, HERO_PTR);
        h::call(c, 0x27E668, &[a0, a1, flag as u32, 0, hero], &[]);
    }
    if c.r32(MODE_ADDR) != 12 || c.r8(GLOBALS + HOLD_BYTE) != 0 || g32(c, HOLD_WORD) != 0 {
        return true;
    }
    let hero = g32(c, HERO_PTR);
    let heading = c.rf(GLOBALS + HEADING);
    let is_zero = (a1 == 0) as u32;
    h::with_stack(c, 0x50, |c, buf| {
        h::run(c, actor::pose_node_pos, &[hero, a1.wrapping_add(0x16), buf], &[]);
        let cos_part = h::run(c, trig::fast_cos, &[], &[heading]).1 * F_POSE_SCALE;
        let x_new = c.rf(buf) + cos_part;
        c.wf(buf, x_new);
        let sin_part = h::run(c, trig::fast_sin, &[], &[heading]).1 * F_POSE_SCALE;
        let y_new = c.rf(buf + 4) + sin_part;
        c.wf(buf + 4, y_new);
        let ground = h::run(c, sweep::ground_height, &[buf, 0], &[F_HALF]).1;
        let z_new = ground + F_GROUND_OFFSET;
        c.wf(buf + 8, z_new);
        h::call(c, 0x266FA8, &[buf, is_zero], &[heading]);
    });
    true
}

/// 0x1f9128 hero_footsteps(): per-tick footstep triggers. In mode 2 the counter at +0x198 is compared with two
/// scaled thresholds (15 and 22); the probe fires when it matches. Then, by the hero's state byte (+0x53): state 3
/// probes at 49.5 and 17.0, state 4 probes at 12.5 and 1.0, each hit calling foot_ground_probe.
pub fn hero_footsteps(c: &mut Cpu) -> bool {
    if g32(c, STATE_2084) == 2 {
        let limit = h::run(c, mathx::float_to_int_scaled, &[15], &[]).0 as i32;
        if (g32(c, COUNTER_198) as i32) < limit {
            return true;
        }
    }
    // The hold byte is loaded in the delay slot of the second mode test (0x1f9178), whatever the mode is.
    let s1 = c.r8(GLOBALS + HOLD_BYTE) as u32;
    if g32(c, STATE_2084) == 2 {
        let threshold = h::run(c, mathx::float_to_int_scaled, &[0x16], &[]).0;
        if g32(c, COUNTER_198) == threshold {
            h::run(c, foot_ground_probe, &[s1, 0, 1], &[]);
        }
    }
    let hero = g32(c, HERO_PTR);
    match c.r8(hero + 0x53) {
        3 => {
            if g32(c, BLOCK_A9C) != 0 {
                return true;
            }
            if probe_hit(c, 0x4246_0000) != 0 {
                h::run(c, foot_ground_probe, &[s1, 0, 1], &[]);
            }
            if probe_hit(c, 0x4188_0000) != 0 {
                h::run(c, foot_ground_probe, &[s1, 1, 1], &[]);
            }
        }
        4 => {
            if g32(c, BLOCK_A9C) != 0 {
                return true;
            }
            if probe_hit(c, 0x4148_0000) != 0 {
                h::run(c, foot_ground_probe, &[s1, 0, 1], &[]);
            }
            if probe_hit(c, 0x3f80_0000) != 0 {
                h::run(c, foot_ground_probe, &[s1, 1, 1], &[]);
            }
        }
        _ => {}
    }
    true
}

/// 0x1f9cb0 slot_release(): for each of the 8 slots, if its index is valid and the entry's id matches the wanted id
/// (or the hero pointer when no id is wanted) and its flag byte is set, calls retail 0x27e1a0. Then clears the index
/// to -1.
pub fn slot_release(c: &mut Cpu) -> bool {
    for i in 0..8u32 {
        let want = g32(c, SLOT_WANT + 4 * i);
        let index_addr = GLOBALS + SLOT_INDEX + 4 * i;
        let index = c.r32(index_addr) as i32;
        if index != -1 {
            let key = if want != 0 { want } else { g32(c, HERO_PTR) };
            let entry = SLOT_ENTRIES.wrapping_add((index as u32).wrapping_mul(SLOT_SIZE));
            if c.r32(entry + 0x88) == key && c.r8(entry + 0x74) != 0 {
                h::call(c, 0x27E1A0, &[], &[]);
            }
        }
        c.w32(index_addr, u32::MAX);
    }
    true
}
