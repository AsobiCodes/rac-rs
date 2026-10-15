//! Hero animation helpers (SetAnim and the held wrench / hand mirror), Eudora overlay 0x224ef0..0x226610.
use crate::calls::*;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const GET_MODE: u32 = 0x20_0540;
const SCALE_TICKS: u32 = 0x1f_14b8;
const FLOAT_TO_INT: u32 = 0x1f_27e8;
const RANDOM_RANGE: u32 = 0x24_a458;
const ANIM_PLAY: u32 = 0x24_a148;
const ACTIVE_ADD: u32 = 0x24_1f60;
const ANIM_LOAD_TRACKS: u32 = 0x24_1140;
const MOBY_SET_MODE: u32 = 0x24_45b0;
const SPR_FLUSH: u32 = 0x11_8a80;
const HELD_EXTRA_PLAY: u32 = 0x22_5360;
const HELD_EXTRA_LOOKUP: u32 = 0x22_5300;
/// Held-object seq tables (4-byte entries: i16 hero seq, held seq a, held seq b; ends with -1) per mode.
const TABLE_MODE2: u32 = 0x17_bcf0;
const TABLE_MODE3: u32 = 0x17_bcd0;
const TABLE_GP_OFFSET: u32 = -0x7568i32 as u32;
const SPEED_TABLE_GP_OFFSET: u32 = -0x7540i32 as u32;
/// Byte flag that swaps hero seqs 0x31 and 0x32.
const SWAP_SEQ_FLAG: u32 = 0x15_edb5;
const ACTIVE_RECORDS: u32 = 0x19_6e00;
const SOUND_VOICE: u32 = 0x13_fe08;

/// Hero block fields.
const H_ACTOR: u32 = 0x2080;
const H_HELD_A: u32 = 0x1180;
const H_HELD_B: u32 = 0x1184;
const H_HELD_C: u32 = 0x118c;
const H_HELD_MODE: u32 = 0x11a4;
const H_ANIM_SPEED: u32 = 0xa90;
const H_ANIM_STEP: u32 = 0xa94;
const H_LOOP_FIRST: u32 = 0xab0;
const H_LOOP_LAST: u32 = 0xab4;
const H_LOOP_FLAG: u32 = 0xab8;
/// Actor (moby) fields.
const A_FLAGS: u32 = 0x34;
const A_SEQ: u32 = 0x53;

/// Table of held-object sequences for the current mode (0 when none).
fn held_table(c: &mut Cpu) -> u32 {
    call(c, GET_MODE, &[3], &[]);
    match c.result() {
        2 => TABLE_MODE2,
        3 => TABLE_MODE3,
        4 => (c.g(28) as u32).wrapping_add(TABLE_GP_OFFSET),
        _ => 0,
    }
}

/// First table entry whose held seq a (byte +2) is `seq`: its i16 hero seq.
fn held_seq_find(c: &mut Cpu, seq: u32) -> Option<u32> {
    let table = held_table(c);
    if table == 0 {
        return None;
    }
    let mut e = table;
    while c.r16(e) as i16 != -1 {
        if c.r8(e + 2) as u32 == seq {
            return Some(c.r16(e) as i16 as i32 as u32);
        }
        e += 4;
    }
    None
}

/// 0x224ef0 held_seq_find(seq, out) -> found: *out = the matching entry's hero seq (0 when none).
pub fn held_seq_find_hand(c: &mut Cpu) -> bool {
    preserved(c, true, false, held_seq_find_hand_body)
}

fn held_seq_find_hand_body(c: &mut Cpu) -> bool {
    let (seq, out) = (c.arg(0), c.arg(1));
    let r = held_seq_find(c, seq);
    c.w32(out, r.unwrap_or(0));
    c.ret(r.is_some() as u32);
    true
}

/// A random table entry whose hero seq equals `seq` (retail passes it through the identity fn 0x1f8160): its held seqs (a, b).
fn held_seq_pick(c: &mut Cpu, seq: u32) -> Option<(u32, u32)> {
    let seq = seq as i32;
    let table = held_table(c);
    let mut matches = Vec::new();
    if table != 0 {
        let mut i = 0;
        while c.r16(table + 4 * i) as i16 != -1 {
            if c.r16(table + 4 * i) as i16 as i32 == seq {
                matches.push(i);
            }
            i += 1;
        }
    }
    if matches.is_empty() {
        return None;
    }
    call(c, RANDOM_RANGE, &[0, matches.len() as u32 - 1], &[]);
    let e = table + 4 * matches[c.result() as usize];
    Some((c.r8(e + 2) as u32, c.r8(e + 3) as u32))
}

/// 0x225138 held_seq_pick(_, out_a, out_b) -> found: random matching entry's two held seqs (else *out_a = 1).
pub fn held_seq_pick_hand(c: &mut Cpu) -> bool {
    preserved(c, true, false, held_seq_pick_hand_body)
}

fn held_seq_pick_hand_body(c: &mut Cpu) -> bool {
    let (seq, out_a, out_b) = (c.arg(0), c.arg(1), c.arg(2));
    match held_seq_pick(c, seq) {
        Some((a, b)) => {
            c.w32(out_a, a);
            c.w32(out_b, b);
            c.ret(1);
        }
        None => {
            c.w32(out_a, 1);
            c.ret(0);
        }
    }
    true
}

/// 0x224fb8 hero_held_mirror(seq, blend, f12 speed): mirrors the hero seq onto the held wrench and hand mobys.
pub fn hero_held_mirror(c: &mut Cpu) -> bool {
    preserved(c, false, false, hero_held_mirror_body)
}

fn hero_held_mirror_body(c: &mut Cpu) -> bool {
    let (seq, blend, speed) = (c.arg(0), c.arg(1), c.farg(0));
    let (held_a, held_b) = (c.r32(HERO + H_HELD_A), c.r32(HERO + H_HELD_B));
    if c.r32(HERO + H_HELD_MODE) != 2 || held_a == 0 || held_b == 0 {
        return true;
    }
    if let Some((a, b)) = held_seq_pick(c, seq) {
        if seq != 0 {
            if c.r8(held_a + A_SEQ) as u32 != a {
                let ticks = speed.to_int() as u32;
                call(c, ANIM_PLAY, &[held_a, a, blend, ticks], &[]);
                call(c, ANIM_PLAY, &[held_b, b, blend, ticks], &[]);
            }
            return true;
        }
    }
    if c.r32(HERO + 0x208c) == 0 {
        let held_seq = c.r8(held_a + A_SEQ) as u32;
        if held_seq_find(c, held_seq) == Some(0) {
            return true;
        }
    }
    call(c, SCALE_TICKS, &[7], &[]);
    let mut ticks = c.result();
    if c.r32(HERO + 0x2090) == 8 {
        call(c, GET_MODE, &[3], &[]);
        if c.result() == 2 {
            call(c, SCALE_TICKS, &[0x13], &[]);
            ticks = c.result();
        }
    }
    for moby in [held_a, held_b] {
        if c.r8(moby + A_SEQ) != 1 {
            call(c, ANIM_PLAY, &[moby, 1, 0, ticks], &[]);
        }
    }
    true
}

/// 0x225268 held_speed_copy(): per tick, when the hero seq has a held-object table entry (RNG draw), the two held
/// mobys take the hero animation speed.
pub fn held_speed_copy(c: &mut Cpu) -> bool {
    preserved(c, false, false, held_speed_copy_body)
}

fn held_speed_copy_body(c: &mut Cpu) -> bool {
    if c.r8(HERO + 0x20a4) != 0 || c.r32(HERO + H_HELD_MODE) != 2 {
        return true;
    }
    let actor = c.r32(HERO + H_ACTOR);
    let seq = c.r8(actor + A_SEQ) as u32;
    if held_seq_pick(c, seq).is_none() {
        return true;
    }
    let speed = c.rf(HERO + H_ANIM_SPEED);
    for held in [HERO + H_HELD_A, HERO + H_HELD_B] {
        let m = c.r32(held);
        if m != 0 {
            c.wf(m + 0x58, speed);
        }
    }
    true
}

/// 0x2252e8 held_extra_clear(): clears the two extra held-object pointers.
pub fn held_extra_clear(c: &mut Cpu) -> bool {
    c.w32(HERO + 0x227c, 0);
    c.w32(HERO + 0x2278, 0);
    true
}

/// 0x225408 held_extra_speed(): the extra held moby takes the hero animation speed when its lookup succeeds.
pub fn held_extra_speed(c: &mut Cpu) -> bool {
    preserved(c, false, false, held_extra_speed_body)
}

fn held_extra_speed_body(c: &mut Cpu) -> bool {
    if c.r32(HERO + 0x2278) == 0 || c.r32(HERO + 0x227c) == 0 {
        return true;
    }
    let actor = c.r32(HERO + H_ACTOR);
    let seq = c.r8(actor + A_SEQ) as u32;
    let ok = with_frame(c, 0x30, |c, buf| {
        call(c, HELD_EXTRA_LOOKUP, &[seq, buf], &[]);
        c.result()
    });
    if ok != 0 {
        let speed = c.rf(HERO + H_ANIM_SPEED);
        let extra = c.r32(HERO + 0x2278);
        c.wf(extra + 0x58, speed);
    }
    true
}

/// 0x225460 held_extra_sync(): the extra held moby copies the hero's blend step, time and current frame index.
pub fn held_extra_sync(c: &mut Cpu) -> bool {
    preserved(c, false, false, held_extra_sync_body)
}

fn held_extra_sync_body(c: &mut Cpu) -> bool {
    let extra = c.r32(HERO + 0x2278);
    if extra == 0 {
        return true;
    }
    let step = c.rf(HERO + H_ANIM_STEP);
    c.wf(extra + 0x5c, step);
    let table = c.r32(extra + 0x24) + 0x48;
    let pa = table + 4 * c.r8(extra + 0x52) as u32;
    let pb = table + 4 * c.r8(extra + 0x53) as u32;
    let actor = c.r32(HERO + H_ACTOR);
    let time = c.rf(actor + 0x54);
    c.wf(extra + 0x54, time);
    let frame = c.r8(actor + 0x51);
    c.w8(extra + 0x50, frame);
    c.w8(extra + 0x51, frame);
    let ta = c.r32(pa);
    let a = c.r32(ta + 4 * frame as u32 + 0x1c);
    c.w32(extra + 0x68, a);
    let tb = c.r32(pb);
    let b = c.r32(tb + 4 * frame as u32 + 0x1c);
    c.w32(extra + 0x6c, b);
    true
}

/// 0x2254f8 hero_set_anim(seq, first_frame, f12 blend): starts a hero sequence: snapshots the pose into the active
/// record, sets blend/speed state, clears the loop range and mirrors onto the held objects.
pub fn hero_set_anim(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    preserved(c, false, false, hero_set_anim_body)
}

fn hero_set_anim_body(c: &mut Cpu) -> bool {
    let (mut seq, first, blend) = (c.arg(0), c.arg(1), c.farg(0));
    let actor = c.r32(HERO + H_ACTOR);
    if c.r32(HERO + 0xa98) & 2 != 0 {
        c.wf(actor + 0x54, F::ZERO);
    }
    if c.r8(SWAP_SEQ_FLAG) != 0 {
        seq = match seq {
            0x31 => 0x32,
            0x32 => 0x31,
            s => s,
        };
    }
    call(c, ACTIVE_ADD, &[actor], &[]);
    let slot = c.result();
    if (slot as i32) >= 0 {
        call(c, MOBY_SET_MODE, &[actor, slot | 0x300], &[]);
        call(c, SPR_FLUSH, &[0], &[]);
        let rec = ACTIVE_RECORDS + (slot << 4);
        let v = c.rv(actor);
        c.wv(rec, v);
        let cur = c.r8(actor + 0x52);
        if cur != 0xff {
            c.w8(actor + 0xa5, cur);
        }
        let v = c.rv(actor + 0xf0);
        c.wv(rec, v);
        c.w8(actor + 0x52, 0xff);
        c.w8(actor + 0x50, slot as u8);
    }
    c.w8(actor + 0x51, first as u8);
    c.w8(actor + 0x53, seq as u8);
    call(c, ANIM_LOAD_TRACKS, &[actor], &[]);
    c.wf(HERO + H_ANIM_SPEED, F::ONE);
    let mut blend = blend;
    if F::ZERO < blend {
        c.wf(HERO + H_ANIM_STEP, F::ONE / blend);
        c.w32(HERO + 0xaa0, -1i32 as u32);
    } else {
        c.wf(HERO + H_ANIM_STEP, F::ONE);
        call(c, FLOAT_TO_INT, &[], &[-blend]);
        c.w32(HERO + 0xaa4, 0);
        c.w32(HERO + 0xaa0, c.result().wrapping_sub(1));
    }
    c.wf(actor + 0x54, F::ZERO);
    hero_loop_clear(c);
    let tracks = c.r32(actor + 0x24);
    let track = c.r32(tracks + 4 * seq + 0x48);
    let b = c.r8(track + 0x11);
    c.w8(actor + 0x7c, b);
    c.w32(HERO + 0xa9c, 1);
    if blend < F::ZERO {
        let idx = c.r32(HERO + 0xaa0);
        let v = c.r32((c.g(28) as u32).wrapping_add(SPEED_TABLE_GP_OFFSET).wrapping_add(4 * idx));
        blend = F::from_int(v as i32);
    }
    if c.r8(HERO + 0x20a4) == 0 {
        c.set_arg(0, seq);
        c.set_arg(1, first);
        c.set_farg(0, blend);
        hero_held_mirror(c);
    }
    held_extra_clear(c);
    if c.r32(HERO + 0x2278) != 0 {
        call(c, HELD_EXTRA_PLAY, &[seq, first], &[blend]);
    }
    true
}

/// 0x225720 hero_loop_range(from, to): loops frames from..to while the current seq has more than `to` frames.
pub fn hero_loop_range(c: &mut Cpu) -> bool {
    let (from, to) = (c.arg(0), c.arg(1));
    if (from as i32) < 0 {
        return true;
    }
    let actor = c.r32(HERO + H_ACTOR);
    let seq = c.r8(actor + A_SEQ) as u32;
    let tracks = c.r32(actor + 0x24);
    let track = c.r32(tracks + 4 * seq + 0x48);
    if (to as i32) < c.r8(track + 0x10) as i32 {
        c.w32(HERO + H_LOOP_LAST, to);
        c.w32(HERO + H_LOOP_FIRST, from);
    }
    true
}

fn hero_loop_clear(c: &mut Cpu) {
    c.w32(HERO + H_LOOP_FIRST, -1i32 as u32);
    c.w32(HERO + H_LOOP_FLAG, 0);
}

/// 0x225768 hero_loop_clear(): no loop range (first = -1, flag 0).
pub fn hero_loop_clear_hand(c: &mut Cpu) -> bool {
    hero_loop_clear(c);
    true
}

/// 0x225780 set_sound_voice(v): stores v (or the scaled tick 5 when v == -1) at 0x13fe08.
pub fn set_sound_voice(c: &mut Cpu) -> bool {
    preserved(c, false, false, set_sound_voice_body)
}

fn set_sound_voice_body(c: &mut Cpu) -> bool {
    let mut v = c.arg(0);
    if v == -1i32 as u32 {
        call(c, SCALE_TICKS, &[5], &[]);
        v = c.result();
    }
    c.w32(SOUND_VOICE, v);
    true
}

/// 0x226210 hero_items_reshow(): clears the hidden bit on hero and held mobys, then hides again per flags.
pub fn hero_items_reshow(c: &mut Cpu) -> bool {
    preserved(c, false, false, hero_items_reshow_body)
}

fn hero_items_reshow_body(c: &mut Cpu) -> bool {
    let unhide = |c: &mut Cpu, m: u32| {
        let f = c.r16(m + A_FLAGS);
        c.w16(m + A_FLAGS, f & 0xfffe);
    };
    let hide = |c: &mut Cpu, m: u32| {
        let f = c.r16(m + A_FLAGS);
        c.w16(m + A_FLAGS, f | 0x41);
    };
    let m = c.r32(HERO + 0xa88);
    unhide(c, m);
    for i in 0..7 {
        for off in [0x1090, 0x1094] {
            let m = c.r32(HERO + 0x50 * i + off);
            if m != 0 {
                unhide(c, m);
            }
        }
    }
    let m = c.r32(HERO + H_HELD_C);
    if m != 0 {
        unhide(c, m);
    }
    if c.r8(HERO + 0x20a4) == 1 {
        let m = c.r32(HERO + 0x1624);
        if m != 0 {
            unhide(c, m);
        }
    }
    if c.r16(HERO + 0x22d8) != 0 {
        for off in [H_HELD_A, H_HELD_B, H_HELD_C] {
            let m = c.r32(HERO + off);
            if m != 0 {
                hide(c, m);
            }
        }
    }
    let by_mode = c.r8(HERO + 0x20ae) != 0 && c.r32(HERO + 0x10b8) == 8;
    if by_mode || c.r8(HERO + 0x20af) != 0 {
        let m = c.r32(HERO + 0x1090);
        if m != 0 {
            hide(c, m);
        }
    }
    true
}

/// 0x2263c8 foot_dust_set(spread, jitter_ticks(i16), ticks, per_joint, set, clear): stores the foot-dust emitter
/// parameters and sets/clears the flag bits in 0x12ae.
pub fn foot_dust_set(c: &mut Cpu) -> bool {
    let (a0, a1, a2, a3) = (c.arg(0), c.arg(1), c.arg(2), c.arg(3));
    let (f0, f1) = (c.farg(0), c.farg(1));
    let flags = c.r16(HERO + 0x12ae) & !(a3 as u8 as u16) | (a2 as u8 as u16);
    c.wf(HERO + 0x12a4, f1);
    c.w32(HERO + 0x12a8, a0 as u16 as i16 as i32 as u32);
    c.w16(HERO + 0x12ac, a1 as u8 as u16);
    c.w16(HERO + 0x12ae, flags);
    c.wf(HERO + 0x12a0, f0);
    true
}

/// 0x226580 wrench_swing_count() -> n: 1 for the fixed states (0x3f, 0x71, 0x70), else the hero's counter at 0x308
/// (raised to 1 while the 0x30e counter is below scaled tick 4 in the idle state).
pub fn wrench_swing_count(c: &mut Cpu) -> bool {
    preserved(c, true, false, wrench_swing_count_body)
}

fn wrench_swing_count_body(c: &mut Cpu) -> bool {
    let state = c.r32(HERO + 0x2084);
    let mut n = c.r16(HERO + 0x308) as i16 as i32;
    if state == 0x3f || state == 0x71 || state == 0x70 {
        n = 1;
    } else if c.r8(HERO + 0x12e7) != 0 && c.r8(0x13_d4dc) != 0 && state == 0 {
        let cur = c.r16(HERO + 0x30e) as i16 as i32;
        call(c, SCALE_TICKS, &[4], &[]);
        if cur < c.result() as i32 {
            n = 1;
        }
    }
    c.ret(n as u32);
    true
}
