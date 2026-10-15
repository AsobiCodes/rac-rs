//! Camera record selection and camera spring helpers (Eudora overlay 0x1dd000 area).
use crate::actor;
use crate::ps2::*;
use crate::vecmath::wrap_once;
use ee_oracle::cpu::Cpu;

/// Per-kind camera handler table: 0x14-byte records (+4 accept test, +0xc enter, +0x10 leave).
const KIND_TABLE: u32 = 0x1dd080;
/// Deferred-callback list: count word and the function pointer array.
const CALLBACK_COUNT: u32 = 0x15ef4c;
const CALLBACK_LIST: u32 = 0x16f0f0;
/// Pointer to the current camera record.
const CURRENT_RECORD: u32 = 0x166f00;
/// Camera records (`RECORD_COUNT + 1` of `RECORD_SIZE` bytes) with a parallel list of used flags.
const RECORDS: u32 = 0x167250;
const RECORD_USED: u32 = 0x169990;
const RECORD_SIZE: u32 = 0xa0;
const RECORD_COUNT: u32 = 0x2f;
/// Camera globals block; +0xc4 holds the camera object handle.
const CAM_BLOCK: u32 = 0x166f10;
/// Pointer to a table of 0x20-byte entries; word +0x1c of each points at a name record.
const ENTRY_TABLE_PTR: u32 = 0x15ef50;
const STATE_BLOCK: u32 = 0x13f350;

/// Word `off` of the handler record for `rec`'s kind (+0x8c).
fn kind_hook(c: &mut Cpu, rec: u32, off: u32) -> u32 {
    let kind = c.r16(rec + 0x8c) as i16 as i32;
    c.r32(KIND_TABLE.wrapping_add((kind * 0x14) as u32) + off)
}

fn entry_name(c: &mut Cpu, rec: u32) -> u32 {
    let ent = c.r32(ENTRY_TABLE_PTR).wrapping_add((c.r16(rec + 0x84) as i16 as i32 as u32) << 5);
    c.r32(ent + 0x1c)
}

/// Caller-saved registers before a function that calls retail code; retail leaves callee garbage there, so the
/// rewrite puts the entry values back and sets only its real result.
pub struct Scratch([u128; 32], [u32; 32]);

pub fn save(c: &Cpu) -> Scratch {
    Scratch(c.r, c.f)
}

pub fn restore(c: &mut Cpu, sv: &Scratch) {
    for i in (1..16).chain([24, 25]) {
        c.r[i] = sv.0[i];
    }
    c.f[..20].copy_from_slice(&sv.1[..20]);
}

fn spring_step(c: &mut Cpu, p: u32, d: F, k: F, damp: F, limit: F) {
    let cur = c.rf(p);
    let x = cur + (k * d - damp * cur);
    c.wf(p, x);
    if !(limit == F::ZERO) {
        if limit < x {
            c.wf(p, limit);
        } else if x < -limit {
            c.wf(p, -limit);
        }
    }
    // The velocity may not exceed the distance itself.
    let m = d.abs();
    if m < c.rf(p) {
        c.wf(p, m);
    } else if c.rf(p) < -m {
        c.wf(p, -m);
    }
}

/// 0x1dd9f0 run_deferred_callbacks(): calls every queued callback in order (count re-read each time), clears it.
pub fn run_deferred_callbacks(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let sv = save(c);
    let mut i = 0;
    if c.r32(CALLBACK_COUNT) as i32 > 0 {
        loop {
            let f = c.r32(CALLBACK_LIST + 4 * i);
            i += 1;
            c.call(f);
            if (i as i32) >= c.r32(CALLBACK_COUNT) as i32 {
                break;
            }
        }
    }
    c.w32(CALLBACK_COUNT, 0);
    restore(c, &sv);
    true
}

/// spring_follow on plain values: steps the velocity at `p` toward b - a and returns a + vel.
pub fn spring(c: &mut Cpu, p: u32, a: F, b: F, k: F, damp: F, limit: F) -> F {
    spring_step(c, p, b - a, k, damp, limit);
    a + c.rf(p)
}

/// 0x1ddaa8 spring_follow(ptr vel, a, b, k, damp, limit) -> a + vel: the velocity chases b - a.
pub fn spring_follow(c: &mut Cpu) -> bool {
    let p = c.arg(0);
    let (a, b, k, damp, limit) = (c.farg(0), c.farg(1), c.farg(2), c.farg(3), c.farg(4));
    spring_step(c, p, b - a, k, damp, limit);
    let r = a + c.rf(p);
    c.fret(r);
    true
}

/// angle_spring_follow on plain values.
pub fn angle_spring(c: &mut Cpu, p: u32, a: F, b: F, k: F, damp: F, limit: F) -> F {
    spring_step(c, p, wrap_once(b - a), k, damp, limit);
    wrap_once(a + c.rf(p))
}

/// 0x1ddb90 angle_spring_follow(ptr vel, a, b, k, damp, limit) -> angle_add(a, vel): spring_follow on angles.
pub fn angle_spring_follow(c: &mut Cpu) -> bool {
    let p = c.arg(0);
    let (a, b, k, damp, limit) = (c.farg(0), c.farg(1), c.farg(2), c.farg(3), c.farg(4));
    spring_step(c, p, wrap_once(b - a), k, damp, limit);
    let r = wrap_once(a + c.rf(p));
    c.fret(r);
    true
}

/// 0x1ddcd0 camera_obj_sync(rec): rec+0x86 == 0 creates the camera object (0x296910) once, otherwise frees it.
pub fn camera_obj_sync(c: &mut Cpu) -> bool {
    let rec = c.arg(0);
    let sv = save(c);
    let obj = c.r32(CAM_BLOCK + 0xc4);
    let mut v = obj;
    if c.r16(rec + 0x86) as i16 == 0 {
        if obj == 0 {
            c.set_arg(0, CAM_BLOCK - 0x50);
            c.call(0x296910);
            v = c.result();
            c.w32(CAM_BLOCK + 0xc4, v);
        }
    } else if obj != 0 {
        c.set_arg(0, obj);
        actor::free_moby(c);
        v = c.result();
        c.w32(CAM_BLOCK + 0xc4, 0);
    }
    restore(c, &sv);
    c.ret(v);
    true
}

/// 0x1de078 camera_record_accept(rec, cur) -> 0/1: whether `rec` may replace `cur` (kind handler first, then the
/// rules of rec's mode word at +0x74).
pub fn camera_record_accept(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let (rec, cur) = (c.arg(0), c.arg(1));
    let sv = save(c);
    let r = record_accept(c, rec, cur);
    restore(c, &sv);
    c.ret(r);
    true
}

fn record_accept(c: &mut Cpu, rec: u32, cur: u32) -> u32 {
    if c.r8(rec + 0x7c) == 0 {
        return 0;
    }
    let mode_rec = rec + 0x74;
    let thresh = c.r8(mode_rec + 8);
    let accept = kind_hook(c, rec, 4);
    if accept != 0 {
        c.set_arg(0, rec);
        c.set_arg(1, cur);
        c.call(accept);
        match c.result() as i32 {
            -1 => return 0,
            1 => return 1,
            _ => {}
        }
    }
    // cur can be displaced: none, one with a nonzero flag, or one below the threshold
    let outranks = |c: &mut Cpu| cur == 0 || c.r16(cur + 0x7e) as i16 != 0 || c.r8(cur + 0x7c) < thresh;
    match c.r32(mode_rec) {
        0 => outranks(c) as u32,
        1 | 2 => {
            if c.r8(mode_rec + 9) == 0 {
                return 0;
            }
            outranks(c) as u32
        }
        4 => {
            let name = entry_name(c, rec);
            if cur != 0 && c.r16(cur + 0x7e) as i16 == 0 && !(c.r8(cur + 0x7c) < thresh) {
                return 0;
            }
            let a1 = c.r32(name + 0xc);
            c.set_arg(0, 0x13f3d0);
            c.set_arg(1, a1);
            c.call(0x252520);
            (c.result() != 0) as u32
        }
        7 => {
            let kind = c.r16(rec + 0x86) as i16 as i32;
            if kind != c.r32(STATE_BLOCK + 0x2284) as i32 {
                return 0;
            }
            if c.r16(cur + 0x7e) as i16 == 0 && !(c.r8(cur + 0x7c) < thresh) {
                return 0;
            }
            if kind != 3 {
                return 1;
            }
            let name = entry_name(c, rec);
            let idx = c.r32(name + 0x24) as i32;
            if idx < 0 {
                return 1;
            }
            let gp = c.g(28) as u32;
            let tab = c.r32(gp.wrapping_sub(0x7514));
            let have = c.r32(tab.wrapping_add((idx as u32) << 5) + 0x10);
            if c.r32(STATE_BLOCK + 0x560) != have {
                return 0;
            }
            (c.r32(STATE_BLOCK + 0x570) == 0) as u32
        }
        _ => 0,
    }
}

/// 0x1de240 camera_leave(rec): calls the kind handler's leave hook (+0x10) when set.
pub fn camera_leave(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let rec = c.arg(0);
    let sv = save(c);
    leave_rec(c, rec);
    restore(c, &sv);
    true
}

fn leave_rec(c: &mut Cpu, rec: u32) {
    let f = kind_hook(c, rec, 0x10);
    if f != 0 {
        c.set_arg(0, rec);
        c.call(f);
    }
}

/// 0x1de288 camera_select() -> -1: leaves the current record, takes the last accepted one, runs its enter hook and
/// mirrors its position (+0x30) to +0x64.
pub fn camera_select(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let sv = save(c);
    let mut cur = c.r32(CURRENT_RECORD);
    leave_rec(c, cur);
    let mut changed = false;
    for i in 0..=RECORD_COUNT {
        let rec = RECORDS + i * RECORD_SIZE;
        if c.r32(RECORD_USED + 4 * i) == 0 || rec == cur {
            continue;
        }
        if record_accept(c, rec, cur) != 0 {
            cur = rec;
            changed = true;
        }
    }
    if changed {
        c.set_arg(0, cur);
        c.call(0x1ddd78);
    }
    let enter = kind_hook(c, cur, 0xc);
    c.set_arg(0, cur);
    camera_obj_sync(c);
    if enter != 0 {
        c.set_arg(0, cur);
        c.call(enter);
    }
    for k in 0..3 {
        let v = c.rf(cur + 0x30 + 4 * k);
        c.wf(cur + 0x64 + 4 * k, v);
    }
    run_deferred_callbacks(c);
    restore(c, &sv);
    c.ret(-1i32 as u32);
    true
}
