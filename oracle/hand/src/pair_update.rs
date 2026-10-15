//! Per-object pair update: adds a step record's position and scale into the object, adds its byte count, and rebuilds
//! the object's tag word (+4) from the step flag, a pending test on the half-word slot, and a float-to-int conversion.
//! Retail callees run as retail code (scratch residues kept); the scratch registers the retail tail leaves are set here.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Step record, 0x20 bytes into the object.
const STEP: u32 = 0x20;
/// Tag word (+4 of the object; the step's +4 is its flag).
const TAG: u32 = 0x4;
/// Byte counter.
const COUNT: u32 = 0x8;
/// Signed half-word slot value.
const HALF: u32 = 0xA;
/// Float scale (the step's scale is at the same offset in the step record).
const SCALE: u32 = 0xC;
/// Low-bits word (step +0x14), merged into the tag word.
const BITS: u32 = 0x14;
/// Float position (object +0x18); the step adds its float at +0x10 to it.
const POS: u32 = 0x18;
const STEP_POS: u32 = 0x10;
/// Retail callees: pending test on the half-word slot, float_to_int_scaled, float_to_int, slot_release.
const PENDING: u32 = 0x001F_1530;
const FLOAT_TO_INT_SCALED: u32 = 0x001F_14B8;
const FLOAT_TO_INT: u32 = 0x001F_27E8;
const SLOT_RELEASE: u32 = 0x0025_9B88;
/// Tag word high byte set on the pending path.
const TAG_HIGH: u32 = 0x7F00_0000;
/// Added to the converted value on the ratio-96 path.
const TAG_BIAS: u32 = 0x20;
/// Scale arguments passed to float_to_int_scaled.
const SCALE_ARG_A: i32 = 30;
const SCALE_ARG_B: i32 = 10;
/// Float constants 96.0 and 127.0 (also the bit patterns retail loads into at).
const F_96: F = F(0x42C0_0000);
const F_127: F = F(0x42FE_0000);
/// Stack frame size of the retail function.
const FRAME: u32 = 0x40;
/// EE GPR numbers of the scratch registers the tail writes.
const AT: usize = 1;
const V0: usize = 2;
const V1: usize = 3;

/// Sign-extends a 32-bit value to the 64-bit register form.
fn sx(v: u32) -> u64 {
    v as i32 as i64 as u64
}

/// Float register n (the checker keeps f regs as bit patterns).
fn getf(c: &Cpu, n: usize) -> F {
    F(c.f[n])
}

/// Sets float register n to x.
fn setf(c: &mut Cpu, n: usize, x: F) {
    c.f[n] = x.0;
}

/// Retail pending test (0x1f1530) on the object's half-word slot; nonzero while pending.
fn pending(c: &mut Cpu, obj: u32) -> bool {
    h::call_keep(c, PENDING, &[obj.wrapping_add(HALF)], &[]).0 != 0
}

/// Retail float_to_int_scaled (0x1f14b8) with argument n; returns its v0.
fn scaled(c: &mut Cpu, n: i32) -> i32 {
    h::call_keep(c, FLOAT_TO_INT_SCALED, &[n as u32], &[]).0 as i32
}

/// Retail float_to_int (0x1f27e8) of x (f12); returns its v0.
fn to_int(c: &mut Cpu, x: F) -> u32 {
    h::call_keep(c, FLOAT_TO_INT, &[], &[x]).0 as u32
}

/// Stores the tag word from a converted value (value << 24 over the low bits) and leaves the retail tail's scratch
/// registers: v1 = the low bits, v0 = the tag.
fn tail(c: &mut Cpu, obj: u32, step: u32, value: u32) {
    let bits = c.r32(step.wrapping_add(BITS));
    let tag = (value << 24) | bits;
    c.w32(obj + TAG, tag);
    c.s(V1, sx(bits));
    c.s(V0, sx(tag));
}

/// 0x25dbb0 pair_update(obj) -> void: adds the step's position and scale into the object, adds the step's byte count,
/// then sets the object's tag word from the step flag, the pending test and the scaled conversions (or releases the
/// object's slot when the flag is set and the slot is no longer pending).
pub fn pair_update(c: &mut Cpu) -> bool {
    let obj = c.arg(0) as u32;
    let step = obj.wrapping_add(STEP);
    h::with_stack(c, FRAME, |c, buf| {
        // Prologue saves: s0 at +0x0, s1 at +0x10, s2 at +0x20, ra at +0x30.
        let saved = [(0x0, c.r[16]), (0x10, c.r[17]), (0x20, c.r[18]), (0x30, c.r[31])];
        for (off, v) in saved {
            c.mem.wr128(buf + off, v);
        }
        // Byte count: v1 = the step's count, v0 = the sum.
        let step_count = c.r8(step + COUNT);
        let sum = c.r8(obj + COUNT) as u32 + step_count as u32;
        c.w8(obj + COUNT, sum as u8);
        c.s(V1, step_count as u64);
        c.s(V0, sum as u64);
        // Position and scale: f1 = new position, f2 = new scale, f0 = the step's scale.
        let step_scale = c.rf(step + SCALE);
        let pos = c.rf(obj + POS) + c.rf(step + STEP_POS);
        let scale = c.rf(obj + SCALE) * step_scale;
        c.wf(obj + POS, pos);
        c.wf(obj + SCALE, scale);
        setf(c, 0, step_scale);
        setf(c, 1, pos);
        setf(c, 2, scale);
        let flag = c.r32(step + TAG);
        c.s(V0, sx(flag));
        if flag == 0 {
            if pending(c, obj) {
                c.w32(step + TAG, 1);
                let r = scaled(c, SCALE_ARG_A);
                c.w16(obj + HALF, r as u16);
                let bits = c.r32(step + BITS);
                let tag = bits | TAG_HIGH;
                c.w32(obj + TAG, tag);
                c.s(V1, sx(TAG_HIGH));
                c.s(V0, sx(tag));
            } else {
                let s0 = scaled(c, SCALE_ARG_B);
                let d = scaled(c, SCALE_ARG_B);
                setf(c, 1, F::from_int(d));
                c.s(AT, sx(F_96.0));
                let half = c.r16(obj + HALF) as i16 as i32;
                c.s(V0, sx(half as u32));
                setf(c, 0, F_96 / getf(c, 1));
                let x = F::from_int(s0.wrapping_sub(half)) * getf(c, 0);
                let t = to_int(c, x).wrapping_add(TAG_BIAS);
                tail(c, obj, step, t);
            }
        } else if pending(c, obj) {
            h::call_keep(c, SLOT_RELEASE, &[obj], &[]);
        } else {
            let r = scaled(c, SCALE_ARG_A);
            setf(c, 1, F::from_int(r));
            c.s(AT, sx(F_127.0));
            let half = c.r16(obj + HALF) as i16 as i32;
            c.s(V0, sx(half as u32));
            setf(c, 0, F::from_int(half));
            let x = getf(c, 0) * (F_127 / getf(c, 1));
            let t = to_int(c, x);
            tail(c, obj, step, t);
        }
    });
    true
}
