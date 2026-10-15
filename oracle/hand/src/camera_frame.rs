//! Per-frame camera update (Eudora overlay 0x1df000 area): shake, ground probe, mode flags.
use crate::camobj::{camera_select, restore, save};
use crate::mathx::{tick_down, wrap_angle};
use crate::ps2::*;
use crate::trig::vu_cos;
use crate::vecmath::{cross3, mat3_copy_w1, set_length};
use ee_oracle::cpu::Cpu;

/// Pointer to the current camera record.
const CURRENT_RECORD: u32 = 0x166f00;
/// Camera globals block: +0x140 position, +0x180 current record, +0x270 mode, +0x394 probe flag, +0x398 frame count.
const CAM_STATE: u32 = 0x166d80;
const PLAYER: u32 = 0x13f350;

fn with_stack<R>(c: &mut Cpu, size: u32, f: impl FnOnce(&mut Cpu, u32) -> R) -> R {
    let sp = c.g(29) as u32;
    let buf = sp - size;
    c.s(29, buf as i32 as i64 as u64);
    let r = f(c, buf);
    c.s(29, sp as i32 as i64 as u64);
    r
}

/// 0x1df1c8 camera_shake_step(shake, alt): counts the shake timer (+8) down, stores the damped amplitude (+4) and
/// adds a vector of that length (along 0x1670f0, or 0x1670d0 when `alt`) to the camera offset 0x166ec0.
pub fn camera_shake_step(c: &mut Cpu) -> bool {
    let (s, alt) = (c.arg(0), c.arg(1));
    let sv = save(c);
    let rec = c.r32(CURRENT_RECORD);
    if rec != 0 && c.r16(rec + 0x86) as i16 == 6 {
        c.w32(s + 0xc, 0);
        c.w32(s + 8, 0);
    } else if c.r32(s + 8) == 0 {
        c.w32(s + 0xc, 0);
    } else {
        let t = c.r32(s + 8) as i32;
        if (c.r32(s + 0xc) as i32) < t {
            c.w32(s + 0xc, t as u32);
        }
        if let (Some(n), _) = tick_down(t) {
            c.w32(s + 8, n as u32);
        }
        let phase = F::from_int(c.r32(s + 8) as i32);
        let ratio = phase / F::from_int(c.r32(s + 0xc) as i32);
        let cos = vu_cos(wrap_angle(phase + phase));
        let len = c.rf(s) * cos * ratio * ratio;
        c.wf(s + 4, len);
        let dir = c.rv(if alt != 0 { 0x1670d0 } else { 0x1670f0 });
        let off = set_length(dir, len);
        let acc = c.rv(0x166ec0);
        c.wv(0x166ec0, acc.with(acc + off, XYZ));
    }
    restore(c, &sv);
    true
}

/// 0x1df658 camera_clip_check(): probes up to 6 times below the camera position (collision ray, the hit z lowered
/// 0.01 each time) and sets the flag at CAM_STATE + 0x394 when the ground is close enough.
pub fn camera_clip_check(c: &mut Cpu) -> bool {
    let sv = save(c);
    let rec = c.r32(CURRENT_RECORD);
    let gp = c.g(28) as u32;
    if c.r16(rec + 0x86) as i16 == 6 || c.r32(gp.wrapping_sub(0x765c)) != 0 {
        c.w32(CAM_STATE + 0x394, 0);
    } else {
        with_stack(c, 0x20, |c, buf| clip_probe(c, buf));
    }
    restore(c, &sv);
    true
}

fn clip_probe(c: &mut Cpu, buf: u32) {
    const HIT: u32 = 0x173f60;
    let (a, b) = (buf, buf + 0x10);
    let pos = c.rv(CAM_STATE + 0x140);
    c.wv(a, pos);
    c.wv(b, pos);
    let (za, zb) = (c.rf(a + 8) + F(0x3f40_0000), c.rf(b + 8) - F(0x3f40_0000));
    c.wf(a + 8, za);
    c.wf(b + 8, zb);
    for _ in 0..6 {
        c.set_arg(0, a);
        c.set_arg(1, b);
        c.set_arg(2, 0x12);
        c.set_arg(3, 0);
        c.set_arg(4, 0);
        c.call(0x1e24d8);
        if c.result() == 0 {
            return;
        }
        c.call(0x1e5e40);
        if c.result() == 0 {
            c.set_arg(0, HIT);
            c.set_arg(1, 0);
            c.call(0x24ca18);
            let h = c.fresult() + F(0x3d23_d70a);
            let near = c.rf(CAM_STATE + 0x148) < h;
            c.w32(CAM_STATE + 0x394, near as u32);
            return;
        }
        let hit = c.rv(HIT);
        c.wv(a, hit);
        let z = c.rf(a + 8) - F(0x3c23_d70a);
        c.wf(a + 8, z);
    }
}

/// 0x1df7a8 camera_flags_init(): derives the camera mode word (0x15ef5c, old value kept at 0x15ef60) and the
/// state word (gp-0x7ca8) from the player state.
pub fn camera_flags_init(c: &mut Cpu) -> bool {
    const MODE: u32 = 0x15ef5c;
    let gp = c.g(28) as u32;
    c.w32(gp.wrapping_sub(0x7ca4), 0x14);
    let kind = c.r32(PLAYER + 0x208c);
    if kind == 0x11 || kind == 0x12 || c.r32(PLAYER + 0x2084) == 0x73 {
        c.w32(MODE, 0x34);
    }
    if c.r32(PLAYER + 0x208c) != 0x11 && c.rf(PLAYER + 0x2f0) < c.rf(0x166ec8) {
        c.w32(MODE, 0x14);
    }
    let mode = c.r32(MODE);
    c.w32(0x15ef60, mode);
    c.w32(MODE, mode | 0x80);
    let flags = gp.wrapping_sub(0x7ca8);
    c.w32(flags, 0xb4);
    let off_a = 0x166f10 + 0xc0;
    let table = [(0x12e5, 0x100, 0x1b4), (0x12eb, 0xb00, 0xbb4), (0x12e6, 0x300, 0x3b4), (0x12ec, 0xd00, 0xdb4)];
    for (flag, a, b) in table {
        if c.r8(PLAYER + flag) != 0 {
            c.w32(off_a, a);
            c.w32(flags, b);
            return true;
        }
    }
    if c.r8(PLAYER + 0x12e4) != 0 {
        c.w32(off_a, 0);
        c.w32(flags, 0xb4);
    } else {
        let v = c.r32(off_a);
        c.w32(flags, v | 0xb4);
    }
    true
}

/// 0x1df8c8 camera_boost_tick(): subtracts the timer at CAM_STATE + 0x258 from the countdown in gp-0x7804; when
/// that runs out both are cleared.
pub fn camera_boost_tick(c: &mut Cpu) -> bool {
    let t = c.rf(CAM_STATE + 0x258);
    if t == F::ZERO {
        return true;
    }
    let gp = c.g(28) as u32;
    let slot = gp.wrapping_sub(0x7804);
    let left = c.rf(slot) - t;
    c.wf(slot, left);
    if left <= F::ZERO {
        c.wf(CAM_STATE + 0x258, F::ZERO);
        c.wf(0x15f3fc, F::ZERO);
    }
    true
}

/// 0x1df910 camera_update(): the per-frame camera step (counters, record selection, position/orientation copy,
/// shake, ground probe).
pub fn camera_update(c: &mut Cpu) -> bool {
    let sv = save(c);
    if c.r32(0x15f5a4) == 5 {
        if c.r32(0x1caa80) != 0 {
            return true;
        }
        c.w16(0x166ff0, 0);
        c.w8(0x166ff2, 0);
    }
    let frame = c.r32(CAM_STATE + 0x398).wrapping_add(1);
    c.w32(CAM_STATE + 0x398, frame);
    camera_boost_tick(c);
    camera_flags_init(c);
    c.call(0x1df2d8);
    camera_select(c);
    let mode = c.r16(CAM_STATE + 0x270) as i16;
    let s1 = c.r32(CURRENT_RECORD);
    if matches!(mode, 1 | 2) {
        let a = c.r32(CAM_STATE + 0x184);
        c.set_arg(0, a);
        c.call(0x1de708);
    }
    let gate = |c: &mut Cpu| c.r32(0x16c158 + 0x14) == 0;
    let mut matrix = false;
    if mode == 3 {
        c.set_arg(0, s1);
        c.call(0x1df118);
        matrix = gate(c);
    } else if gate(c) {
        let p = c.rv(s1 + 0x30);
        c.wv(CAM_STATE + 0x140, p);
        for (dst, src) in [(0x350, 0), (0x360, 0x10), (0x370, 0x20)] {
            let v = c.rv(s1 + src);
            c.wv(CAM_STATE + dst, v);
        }
        matrix = true;
    }
    if matrix {
        with_stack(c, 0x40, |c, buf| {
            c.set_arg(0, buf);
            c.set_arg(1, 0x1670d0);
            mat3_copy_w1(c);
            c.set_arg(0, buf);
            c.set_arg(1, 0x1670d0 - 0x200);
            c.call(0x24fef0);
        });
    }
    let shake = 0x166ee0;
    c.set_arg(0, shake);
    c.set_arg(1, 0);
    camera_shake_step(c);
    c.set_arg(0, shake + 0x10);
    c.set_arg(1, 1);
    camera_shake_step(c);
    camera_clip_check(c);
    c.set_arg(0, shake - 0x20);
    c.call(0x1e0f20);
    if c.r8(0x15edb4) != 0 {
        c.set_arg(0, shake + 0x200);
        c.set_arg(1, shake + 0x210);
        c.set_arg(2, shake + 0x1f0);
        cross3(c);
    }
    restore(c, &sv);
    true
}
