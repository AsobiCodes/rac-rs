//! Moby spawn/init (0x24a290) and a polynomial curve evaluation (0x24a6e8) of the Eudora overlay.
use crate::actor;
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// 0.025 as f32 bits: a moby timer above this counts as running (timer at +0x54).
const TIMER_THRESHOLD: F = F(0x3ccc_cccd);
/// 1.0 as f32 bits.
const ONE: F = F(0x3f80_0000);
/// Slot record table: 16-byte records indexed by the active slot number.
const SLOT_TABLE: u32 = 0x196e00;
/// Moby field offsets.
const OFF_TIMER: u32 = 0x54;
const OFF_FLAG_A: u32 = 0x60;
const OFF_FLAG_B: u32 = 0x64;
const OFF_SLOT_SRC: u32 = 0xf0;
const OFF_PREV_SLOT: u32 = 0x52;
const OFF_SLOT: u32 = 0x50;
const OFF_NEXT_SLOT: u32 = 0x51;
const OFF_SAVED_SLOT: u32 = 0xa5;
const OFF_KIND: u32 = 0x53;
const OFF_SCALE: u32 = 0x58;
const OFF_RATE: u32 = 0x5c;
const OFF_FLAGS_70: u32 = 0x70;
const OFF_TABLE: u32 = 0x24;
const OFF_CLASS: u32 = 0x7c;
/// Flag bits of the spawn request (t0).
const REQ_SLOT_LOW: u32 = 1;
const REQ_SLOT_HIGH: u32 = 2;
const REQ_KEEP: u32 = 4;
/// Slot number bits set by the request flags.
const SLOT_BIT_A: u32 = 0x100;
const SLOT_BIT_B: u32 = 0x200;
/// Retail callees.
const SPAWN_LINK: u32 = 0x2445b0;
const INT_TO_FLOAT: u32 = 0x1f27d8;

/// 0x24a290 spawn_init(obj, kind, mode, count, flags): activates moby `obj` (a0) in a slot when it is running or
/// flagged, copies its slot record, then loads its animation tracks and scale from `count`.
pub fn spawn_init(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let kind = c.arg(1);
    let mode = c.arg(2) as u8;
    let count = c.arg(3);
    let flags = c.arg(4);
    let saves = [
        (31, 0x60, h::SQ),
        (16, 0x00, h::SQ),
        (17, 0x10, h::SQ),
        (18, 0x20, h::SQ),
        (19, 0x30, h::SQ),
        (20, 0x40, h::SQ),
        (21, 0x50, h::SQ),
    ];
    h::frame(c, 0x80, &saves, |c, sp| {
        c.w32(sp + 0x70, c.f[20]);
        let timer = c.rf(obj + OFF_TIMER);
        let active = TIMER_THRESHOLD < timer
            || c.r32(obj + OFF_FLAG_A) != 0
            || c.r32(obj + OFF_FLAG_B) != 0
            || flags & REQ_KEEP != 0;
        if active {
            let slot = h::run(c, actor::active_add, &[obj], &[]).0 as i32;
            if slot >= 0 {
                let slot = slot as u32;
                let mut a1 = if flags & REQ_SLOT_LOW != 0 { slot | SLOT_BIT_A } else { slot };
                if flags & REQ_SLOT_HIGH != 0 {
                    a1 |= SLOT_BIT_B;
                }
                h::call(c, SPAWN_LINK, &[obj, a1], &[]);
                let rec = SLOT_TABLE.wrapping_add(slot << 4);
                let src = c.rv(obj + OFF_SLOT_SRC);
                c.wv(rec, src);
                let prev = c.r8(obj + OFF_PREV_SLOT);
                if prev != 0xff {
                    c.w8(obj + OFF_SAVED_SLOT, prev);
                }
                c.w8(obj + OFF_PREV_SLOT, 0xff);
                c.w8(obj + OFF_SLOT, slot as u8);
            }
        }
        c.w8(obj + OFF_NEXT_SLOT, mode);
        c.w8(obj + OFF_KIND, kind as u8);
        h::run(c, actor::anim_load_tracks, &[obj], &[]);
        c.wf(obj + OFF_SCALE, ONE);
        let (_, scale) = h::run_keep(c, mathx::int_to_float, &[count], &[]);
        let rate = ONE / scale;
        let flags70 = c.r8(obj + OFF_FLAGS_70) & 0xfd;
        c.w8(obj + OFF_FLAGS_70, flags70);
        c.w32(obj + OFF_TIMER, 0);
        let entry = c.r32(obj + OFF_TABLE).wrapping_add(kind << 2);
        let info = c.r32(entry + 0x48);
        let class = c.r8(info + 0x11);
        c.w8(obj + OFF_CLASS, class);
        c.wf(obj + OFF_RATE, rate);
        c.set_arg(0, entry);
        c.ret(info);
        true
    })
}

/// 0x24a6e8 curve_eval(a, b, c, d, t) -> f0: cubic-style blend of a..d at parameter t (f12..f16).
pub fn curve_eval(c: &mut Cpu) -> bool {
    let a = c.farg(0);
    let b = c.farg(1);
    let k = c.farg(2);
    let d = c.farg(3);
    let t = c.farg(4);
    let ab = a - b;
    let dk = d - k;
    let t2 = t * t;
    let ka = k - a;
    let dk_ab = dk - ab;
    let t3 = t2 * t;
    let ka_t = ka * t;
    let rest = ab - dk_ab;
    let mid = dk_ab * t3;
    let tail = rest * t2;
    let sum = mid + tail;
    let sum = sum + ka_t;
    let res = sum + b;
    c.f[1] = tail.0;
    c.f[2] = t3.0;
    c.f[3] = t2.0;
    c.f[14] = ka_t.0;
    c.fret(res);
    true
}
