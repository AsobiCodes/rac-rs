//! Hero ledge probe (0x21aaf0) and the pad-combo follow-up that fires when the input history matches (0x21bcb8).
use crate::collision;
use crate::h::{self, with_stack};
use crate::particles;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Hero state block (0x140000 - 0xcb0), addressed through s0/s1 in retail.
const HERO: u32 = 0x13_F350;
/// Hero state id; 0x6a means the hero is inactive.
const STATE: u32 = HERO + 0x2084;
const STATE_INACTIVE: u32 = 0x6a;
/// Hero state id that gates the probe off in some places.
const STATE_HANG: u32 = 0x12;
/// Sub-mode of the hero (0x11 is the ledge-approach mode).
const MODE: u32 = HERO + 0x208c;
/// Sub-state checked for mode 0x11.
const SUB: u32 = HERO + 0x2088;
/// Byte gate: the probe only runs while this is non-zero.
const ARMED: u32 = HERO + 0x12e4;
/// Halfword: surface class of the last probe hit.
const HIT_CLASS: u32 = HERO + 0x12e0;
/// Halfword flag checked in several modes.
const FLAG_41E: u32 = HERO + 0x41e;
/// Word: probe length argument of the sweep.
const PROBE_LEN: u32 = HERO + 0x2080;
/// Float: speed, height, radius, drop distance and timer of the hero.
const SPEED: u32 = HERO + 0x108;
const HEIGHT: u32 = HERO + 0x2f0;
const RADIUS: u32 = HERO + 0x88;
const DROP: u32 = HERO + 0xe8;
const TIMER: u32 = HERO + 0x22a4;
/// Float: probe reach used by the retail sweep, read from the global table.
const PROBE_REACH: u32 = 0x17_3F68;
/// Word at gp - 0x7654: probe option bits.
const GP_OPTIONS: u32 = 0x7654;
/// Word: hero collision flag block, read by the pad-combo follow-up.
const COMBO_FLAGS: u32 = 0x14_0000 - 0x3520;

/// Retail leaves the last constant it loaded in $at (r1); the checker compares it, so mirror those loads.
fn load_at(c: &mut Cpu, bits: u32) {
    c.s(1, bits as i32 as i64 as u64);
}

/// Calls retail code at `addr`. Unlike `h::call` the registers are left as the callee leaves them, which is
/// what retail does; the checker compares that residue. Returns v0 and f0.
fn raw_call(c: &mut Cpu, addr: u32, ints: &[u32], floats: &[F]) -> (u32, F) {
    for (i, &a) in ints.iter().enumerate() {
        c.set_arg(i, a);
    }
    for (i, &x) in floats.iter().enumerate() {
        c.set_farg(i, x);
    }
    c.call(addr);
    (c.result(), c.fresult())
}

/// Stack bytes reserved by the ledge probe frame.
const PROBE_FRAME: u32 = 0x70;
/// Stack bytes reserved by the combo frame.
const COMBO_FRAME: u32 = 0x40;
/// Pad button mask (bit 6) scanned in the input history.
const COMBO_MASK: u32 = 0x40;
/// Sweep radius/extent passed to 0x258f80.
const COMBO_EXTENT: u32 = 0x1f000;

const F_0_4: F = F(0x3ecc_cccd);
const F_0_07: F = F(0x3d8f_5c29);
const F_0_2: F = F(0x3e4c_cccd);
const F_0_45: F = F(0x3ee6_6666);
const F_0_8: F = F(0x3f4c_cccd);
const F_0_7: F = F(0x3f33_3333);
const F_0_01: F = F(0x3c23_d70a);
const F_4: F = F(0x4080_0000);
const F_16: F = F(0x4180_0000);
const F_1_3: F = F(0x3fa6_6666);

/// 0x21aaf0 edge_probe_step() -> 1 when the ledge probe hit a surface and the grab event was raised, else 0.
pub fn edge_probe_step(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let r = with_stack(c, PROBE_FRAME, |c, buf| edge_probe(c, buf));
    c.ret(r);
    true
}

/// Probe body. `buf` is the 0x70-byte scratch frame (retail sp).
fn edge_probe(c: &mut Cpu, buf: u32) -> u32 {
    c.s(4, 0x6a);
    if c.r32(STATE) == STATE_INACTIVE {
        return 0;
    }
    let mut grab = false;
    if c.r8(ARMED) != 0 {
        match c.r32(MODE) {
            0x11 => grab = ledge_approach(c),
            0x12 | 3 => {}
            _ => grab = sweep_grab(c),
        }
    }
    load_at(c, F_0_07.0);
    // Hang-state drop check: the grab flag is what matters; the constant load is what retail leaves in $at.
    if c.r32(STATE) == STATE_HANG && c.r16(FLAG_41E) as i16 != 0 {
        load_at(c, F_0_7.0);
        if c.rf(RADIUS) < c.rf(HEIGHT) - F_0_7 {
            grab = true;
        }
    }
    let mode = c.r32(MODE);
    let gp_options = c.r32((c.g(28) as u32).wrapping_sub(GP_OPTIONS));
    let skip_probe = mode.wrapping_sub(0x11) < 2
        || mode == 7
        || mode == 0x14
        || c.r32(STATE) == STATE_HANG
        || gp_options & 0xf != 0;
    if skip_probe {
        return finish(c, grab);
    }
    let reach = if gp_options & 0x3f != 0 { F_4 } else { F_16 };
    let base = buf + 0x10;
    load_at(c, F_1_3.0);
    raw_call(c, 0x205760, &[buf, base, 0], &[reach, F_1_3]);
    let len = c.r32(PROBE_LEN);
    let (hit, _) = raw_call(c, 0x1e24d8, &[buf, base, 2, len, 0], &[]);
    if hit == 0 {
        return finish(c, grab);
    }
    let (class, _) = h::run(c, collision::hit_surface_class, &[], &[]);
    if class & 0xffff != 0 {
        return finish(c, grab);
    }
    c.w16(HIT_CLASS, class as u16);
    load_at(c, F_0_01.0);
    let aim = c.rf(PROBE_REACH) - F_0_01;
    c.wf(buf + 8, aim);
    let len = c.r32(PROBE_LEN);
    let (hit2, _) = raw_call(c, 0x1e24d8, &[buf, base, 2, len, 0], &[]);
    if hit2 == 0 {
        raw_call(c, 0x215538, &[0x34, 1], &[]);
        return 1;
    }
    finish(c, grab)
}

/// Retail tail of the probe: when the grab flag is set, raise the event and reset the state.
fn finish(c: &mut Cpu, grab: bool) -> u32 {
    c.s(4, 0x37);
    if grab {
        raw_call(c, 0x215538, &[0x37, 1], &[]);
        raw_call(c, 0x209f28, &[3, 0], &[]);
    }
    grab as u32
}

/// Mode 0x11 test: the hero is moving, is above the ledge band and the drop is close enough.
fn ledge_approach(c: &mut Cpu) -> bool {
    c.r32(SUB) != 1 && F::ZERO < c.rf(SPEED) && c.rf(HEIGHT) - F_0_4 < c.rf(RADIUS)
}

/// Modes other than 0x11, 0x12 and 3: the drop-distance sweep decides the grab flag.
fn sweep_grab(c: &mut Cpu) -> bool {
    let drop = c.rf(DROP).abs();
    let mut grab = false;
    let mut band = F_0_2;
    if band < drop + F_0_07 {
        band = drop + F_0_07;
    }
    let gap = c.rf(HEIGHT) - (c.rf(RADIUS) + F_0_45);
    if gap.abs() < band {
        let mode = c.r32(MODE);
        let flag_clear = c.r16(FLAG_41E) as i16 == 0;
        let skip = mode == 4 && flag_clear;
        if !skip && F_0_8 < c.rf(TIMER) && c.rf(SPEED) < F::ZERO {
            grab = true;
        }
    }
    // Retail reloads 0.8 here whether or not the sweep above ran.
    if c.rf(RADIUS) < c.rf(HEIGHT) - F_0_8 {
        let mode = c.r32(MODE);
        if mode != 4 || c.r16(FLAG_41E) as i16 != 0 {
            grab = true;
        }
    }
    grab
}

/// 0x21bcb8 pad_combo_fire(frames, flag) -> 1 when the combo fired and the cue was raised, else 0.
/// Scans the input history for the button mask; on a hit with the armed flags set, plays the cue.
pub fn combo_input_fire(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let frames = c.arg(0);
    let flag = c.arg(1);
    let r = with_stack(c, COMBO_FRAME, |c, buf| {
        let (hit, _) = h::run(c, particles::input_history_scan, &[COMBO_MASK, frames, buf], &[]);
        if hit == 0 {
            return 0;
        }
        let (n, _) = raw_call(c, 0x214ed8, &[COMBO_MASK, frames, buf], &[]);
        c.s(3, 0x14_0000);
        if c.r32(COMBO_FLAGS) & 0xa != 0 && (n as i32) > 0 {
            let (sent, _) = raw_call(c, 0x258f80, &[COMBO_MASK, COMBO_EXTENT, frames.wrapping_add(n)], &[]);
            if sent != 0 {
                raw_call(c, 0x215538, &[0xb, 1], &[]);
                return 1;
            }
        }
        if flag != 0 {
            return 0;
        }
        raw_call(c, 0x215538, &[7, 1], &[]);
        1
    });
    c.ret(r);
    true
}
