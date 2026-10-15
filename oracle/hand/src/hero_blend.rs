//! Hero blend-speed update: picks the blend speed (record +0xa90) from the speed mode or the gait value, and when
//! the sequence index changes restarts the hero animation with the new blend.
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Hero record base (lui 0x140000 - 0xcb0).
const HERO: u32 = 0x0013_F350;
/// Gait threshold table, 16-byte entries (lui 0x180000 - 0x4148).
const TABLE: u32 = 0x0017_BEB8;
/// Float multiplier applied to every table threshold (lui 0x160000 - 0x1294).
const TABLE_SCALE: u32 = 0x0015_ED6C;
/// gp-relative table of per-sequence records (gp - 0x7558).
const GP_RECORD_TABLE: u32 = 0x7558;

const MODE: u32 = 0x20A4;
const SPEED_IN: u32 = 0x160;
const GAIT: u32 = 0x194;
const ANIM_LIMIT: u32 = 0x1A0;
const HALT: u32 = 0x3BC;
const SEQ_INDEX: u32 = 0x2088;
const RECORD_PTR: u32 = 0x2080;
const SPEED_OUT: u32 = 0xA90;
/// Sequence-index limit: index is clamped to 0..=1 and the hero animation changes when it moves.
const MAX_INDEX: i32 = 1;
/// Retail float_to_int_scaled (jal 0x1f14b8) and hero_set_anim (jal 0x2254f8): run as retail code so scratch residues match.
const FLOAT_TO_INT: u32 = 0x001F_14B8;
const HERO_SET_ANIM: u32 = 0x0022_54F8;
/// Retail calls float_to_int_scaled with these arguments.
const SCALE_ARG_LIMIT: u32 = 4;
const SCALE_ARG_REC: u32 = 8;

const F_54: F = F(0x4258_0000);
const F_90: F = F(0x42B4_0000);
const F_0_5: F = F(0x3F00_0000);
const F_0_6: F = F(0x3F19_999A);
const F_0_7: F = F(0x3F33_3333);
const F_14: F = F(0x4160_0000);
const F_157: F = F(0x431D_0000);
const F_4: F = F(0x4080_0000);
const F_2_2: F = F(0x400C_CCCD);

/// Table entry address for sequence index `idx` (16 bytes per entry) relative to `base`.
fn entry(base: u32, idx: i32) -> u32 {
    base.wrapping_add((idx as u32).wrapping_shl(4))
}

/// MIPS signed divide: returns (quotient, remainder) with the R3000/EE result for a zero divisor.
fn div_lo_hi(n: i32, d: i32) -> (i32, i32) {
    if d == 0 {
        let lo = if n < 0 { 1 } else { -1 };
        (lo, n)
    } else {
        (n.wrapping_div(d), n.wrapping_rem(d))
    }
}

/// Speed in the fixed modes: max(gait-independent input * scale, floor).
fn speed_from_input(c: &mut Cpu, scale: F, floor: F) {
    let x = c.rf(HERO + SPEED_IN) * scale;
    let v = if x < floor { floor } else { x };
    c.wf(HERO + SPEED_OUT, v);
}

/// Tail for sequence index 0 or 1: max(gait * scale, 0.6), then capped at `cap`.
/// Retail leaves f0 = the uncapped lo, f1 = cap, f2 = 0.6 and at = cap bits.
fn tail_speed(c: &mut Cpu, scale: F, cap: F) {
    let x = c.rf(HERO + GAIT) * scale;
    let lo = if x < F_0_6 { F_0_6 } else { x };
    let v = if cap < lo { cap } else { lo };
    c.wf(HERO + SPEED_OUT, v);
    c.f[0] = lo.0;
    c.f[1] = cap.0;
    c.f[2] = F_0_6.0;
    c.s(1, cap.0 as u64);
}

/// Speed tail taken whenever the sequence does not change: indices 0 and 1 use their own scale and cap.
/// Every exit joins retail 0x21c01c: v0 = 1, v1 = sequence index, a0 = hero record.
fn finish_tail(c: &mut Cpu) {
    let index = c.r32(HERO + SEQ_INDEX);
    match index {
        0 => tail_speed(c, F_157, F_4),
        1 => tail_speed(c, F_14, F_2_2),
        _ => {}
    }
    c.s(2, 1);
    c.s(3, index as u64);
    c.s(4, HERO as u64);
}

/// The retail guard before a gait step: the scaled int must be below the record limit and the halt flag clear.
fn may_step(c: &mut Cpu) -> bool {
    let (v0, _) = h::call_keep(c, FLOAT_TO_INT, &[SCALE_ARG_LIMIT], &[]);
    let limit = c.r32(HERO + ANIM_LIMIT) as i32;
    (v0 as i32) < limit && c.r16(HERO + HALT) as i16 == 0
}

/// Gait path: walks the threshold table to a new sequence index, then restarts the hero sequence if it changed.
fn blend_by_gait(c: &mut Cpu) {
    if !may_step(c) {
        return finish_tail(c);
    }
    let gait = c.rf(HERO + GAIT);
    let index = c.r32(HERO + SEQ_INDEX) as i32;
    let k = c.rf(TABLE_SCALE);
    let above = gait < c.rf(entry(TABLE, index)) * k;
    if !above && !(c.rf(entry(TABLE, index) + 4) * k < gait) {
        return finish_tail(c);
    }
    if !may_step(c) {
        return finish_tail(c);
    }
    // Retail `lw $t0, 0x2088(hero)` in the delay slot of the branch on `above`: t0 = the old index.
    c.s(8, index as u32 as u64);

    let mut idx = index;
    if above {
        // Walk down while the gait still exceeds the lower threshold of the entry.
        loop {
            idx -= 1;
            c.w32(HERO + SEQ_INDEX, idx as u32);
            if !(gait < c.rf(entry(TABLE, idx)) * k) {
                break;
            }
        }
    } else {
        // Walk up while the upper threshold of the next entry is still below the gait.
        idx = index + 1;
        while c.rf(entry(TABLE + 4, idx)) * k < gait {
            idx += 1;
        }
        c.w32(HERO + SEQ_INDEX, idx as u32);
    }

    // Clamp the index to 0..=MAX_INDEX (negative becomes 0; anything above becomes MAX_INDEX).
    let mut new = c.r32(HERO + SEQ_INDEX) as i32;
    if new < 0 {
        new = 0;
        c.w32(HERO + SEQ_INDEX, 0);
    }
    if new > MAX_INDEX {
        new = MAX_INDEX;
        c.w32(HERO + SEQ_INDEX, MAX_INDEX as u32);
    }
    if new == index {
        return finish_tail(c);
    }
    restart_sequence(c, index, new);
    finish_tail(c);
}

/// Starts the hero sequence for the new index with the blend derived from the per-sequence records.
fn restart_sequence(c: &mut Cpu, old: i32, new: i32) {
    let record = c.r32(HERO + RECORD_PTR);
    let table = c.r32(record + 0x24).wrapping_add(0x48);
    let seq = (new + 3) as u32;
    let entry_new = c.r32(table.wrapping_add(seq << 2));
    let entry_old = c.r32(table.wrapping_add(((old + 3) as u32) << 2));
    let gp = c.g(28) as u32;
    let slot = gp.wrapping_sub(GP_RECORD_TABLE)
        .wrapping_add((new as u32) << 2)
        .wrapping_add((old as u32) << 3);
    let slot_val = c.r32(slot) as i32;
    let rate = c.r8(record + 0x51) as i32;
    let a1 = c.r8(entry_new + 0x10) as i32;
    let v1 = c.r8(entry_old + 0x10) as i32;
    let (q, _) = div_lo_hi(rate.wrapping_mul(a1), v1);
    let frame = q.wrapping_add(slot_val);
    let (quot, first_frame) = div_lo_hi(frame, a1);
    // Retail scratch before its calls: t0 = entry_new, t1 = entry_old, v0 = frame, v1 = entry_old byte,
    // a1 = entry_new byte, a2 = slot address, a3 = slot value, HI = frame % a1, LO = frame / a1.
    c.s(8, entry_new as u64);
    c.s(9, entry_old as u64);
    c.s(2, frame as i64 as u64);
    c.s(3, v1 as i64 as u64);
    c.s(5, a1 as i64 as u64);
    c.s(6, slot as u64);
    c.s(7, slot_val as i64 as u64);
    c.set_lh(quot as i64 as u64, first_frame as i64 as u64);
    let (blend_int, _) = h::call_keep(c, FLOAT_TO_INT, &[SCALE_ARG_REC], &[]);
    h::call_keep(
        c,
        HERO_SET_ANIM,
        &[seq, first_frame as u32],
        &[F::from_int(blend_int as i32)],
    );
}

/// 0x21bd58 hero_blend_update() -> none: sets the hero blend speed (record +0xa90) from the speed mode or gait value,
/// and restarts the hero sequence when the gait sequence index changes.
pub fn hero_blend_update(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    match c.r8(HERO + MODE) {
        3 => speed_from_input(c, F_54, F_0_7),
        1 => speed_from_input(c, F_90, F_0_5),
        _ => blend_by_gait(c),
    }
    true
}
