//! Moby orientation and expiry tick (`orient_cull`), and the claim of a free word in a moby's 15-entry list
//! (`claim_slot`).
use crate::actor;
use crate::h;
use crate::mathx;
use crate::vecmath;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Retail `lui at, 0x4049; ori at, 0x0fdb`: pi as a single, 0x40490fdb.
const PI: F = F(0x4049_0fdb);
/// Overlay float constant read by `orient_cull` (`lui at, 0x16; lwc1 -0x1294(at)`); read from RAM at call time.
const TILT_SCALE: u32 = 0x15ed6c;

/// Moby fields: the record pointer at +0x78, the three angles at +0x40/+0x44/+0x48, the target at +0x2c, the count
/// byte at +0x23.
const OBJ_REC: u32 = 0x78;
const OBJ_TARGET: u32 = 0x2c;
const OBJ_ANGLE: u32 = 0x40;
const OBJ_COUNT: u32 = 0x23;

/// Record fields: a word at +0x14, a float at +0x18, halfwords at +0x1c and +0x1e.
const REC_WORD: u32 = 0x14;
const REC_SCALE: u32 = 0x18;
const REC_LEN: u32 = 0x1c;
const REC_RATE: u32 = 0x1e;

/// Limit of the list walk in `claim_slot` (a2 must stay below 15).
const SLOT_LIMIT: u32 = 15;

/// Sets a GPR the way a 32-bit MIPS result does: sign-extended into the 64-bit register.
fn set32(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// MIPS signed `div` as the EE does it: a zero divisor gives LO = 1 for a negative dividend, else -1.
fn div32(n: i32, d: i32) -> i32 {
    if d == 0 {
        if n < 0 { 1 } else { -1 }
    } else {
        n.wrapping_div(d)
    }
}

/// 0x29edb0 orient_cull(moby): aims the moby's angles at +0x40..+0x48 (each `angle_add` with the record's tilt
/// scale), stores the record's ratio at +0x2c, then frees the moby when the record test at +0x1c fires; otherwise
/// stores a scaled count into +0x23 when the record's length is under half its word.
pub fn orient_cull(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let rec = c.r32(obj + OBJ_REC);
    // retail's test (0x1f1530) counts the record's length down: it is nonzero (the free path) when the length is 1 or
    // less, and then the moby is freed by retail's free_moby, whose scratch leftovers this hand does not reproduce.
    // That path declines before any write, so retail runs the whole function. Otherwise the test writes nothing and
    // leaves at = length - 1.
    let len0 = c.r16(rec + REC_LEN) as i16 as i32;
    if len0 <= 1 {
        return false;
    }
    h::frame(c, 0x40, &[(16, 0x0, h::SQ), (17, 0x10, h::SQ), (31, 0x20, h::SQ)], |c, sp| {
        // retail saves f21 and f20 in the frame and reloads them on exit; they are never changed here
        let (f20, f21) = (c.f[20], c.f[21]);
        c.w32(sp + 0x30, f20);
        c.w32(sp + 0x38, f21);

        // the ratio: (float(word - len) * scale) / float(word). Retail's `mov.s f20, f0` sits in the delay slot of the
        // first conversion, so it takes that first result before the second conversion runs.
        let word = c.r32(rec + REC_WORD);
        let len = c.r16(rec + REC_LEN) as i16 as i32;
        let (_, head) = h::run_keep(c, mathx::int_to_float, &[word.wrapping_sub(len as u32)], &[]);
        let (_, base) = h::run_keep(c, mathx::int_to_float, &[word], &[]);
        let scale = c.rf(rec + REC_SCALE);
        let ratio = head * scale / base;
        c.wf(obj + OBJ_TARGET, ratio);

        // three angle blends toward the tilt scale (times pi)
        let tilt = c.rf(TILT_SCALE) * PI;
        let old = c.rf(obj + OBJ_ANGLE);
        let (_, a) = h::run_keep(c, vecmath::angle_add, &[], &[old, tilt]);
        c.wf(obj + OBJ_ANGLE, a);
        let old = c.rf(obj + OBJ_ANGLE + 4);
        let (_, b) = h::run_keep(c, vecmath::angle_add, &[], &[old, tilt]);
        c.wf(obj + OBJ_ANGLE + 4, b);
        let old = c.rf(obj + OBJ_ANGLE + 8);
        let (_, d) = h::run_keep(c, vecmath::angle_add, &[], &[old, tilt]);
        c.wf(obj + OBJ_ANGLE + 8, d);

        // count: len < (word + sign) / 2 gives count = len * rate / half, stored as a byte
        // the test's delay slot stores the counted-down length (at = length - 1) before the test returns 0
        c.w16(rec + REC_LEN, (len0 - 1) as u16);
        let word = c.r32(rec + REC_WORD);
        let len = c.r16(rec + REC_LEN) as i16 as i32;
        let sum = word.wrapping_add(word >> 31);
        let half = (sum as i32) >> 1;
        let (mut v0, mut v1) = (sum, 0u32);
        if len < half {
            v1 = 1;
            let rate = c.r16(rec + REC_RATE) as i16 as i32;
            let count = div32(len.wrapping_mul(rate), half);
            c.w8(obj + OBJ_COUNT, count as u8);
            v0 = count as u32;
        }
        set32(c, 2, v0);
        set32(c, 3, v1);
        set32(c, 4, half as u32);
        set32(c, 5, len as u32);
        // the test left at = the counted-down length
        set32(c, 1, len as u32);
    });
    true
}

/// 0x2aab40 claim_slot(moby, value): stores `value` in the first empty word of the 15-entry list at the moby's
/// record (+0x78) plus 4; the first word is taken when the list is empty. Leaves the walk index in a2 as retail does.
pub fn claim_slot(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let value = c.arg(1);
    let rec = c.r32(obj + OBJ_REC);
    let base = rec.wrapping_add(4);
    let first = c.r32(base);
    if first == 0 {
        c.w32(base, value);
        set32(c, 2, 0);
        set32(c, 6, 0);
        set32(c, 7, rec);
        return true;
    }
    let mut a2: u32 = 1;
    let mut v0;
    let mut v1 = 0u32;
    let mut a0 = 0u32;
    loop {
        v0 = (a2 < SLOT_LIMIT) as u32;
        if a2 >= SLOT_LIMIT {
            break;
        }
        v0 = a2 << 2;
        v1 = base.wrapping_add(v0);
        a0 = c.r32(v1);
        a2 += 1;
        if a0 != 0 {
            continue;
        }
        c.w32(v1, value);
        break;
    }
    set32(c, 2, v0);
    set32(c, 3, v1);
    set32(c, 4, a0);
    set32(c, 6, a2);
    set32(c, 7, rec);
    true
}
