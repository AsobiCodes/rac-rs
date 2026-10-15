//! Retail routines from the L4_11 record: the save-tier slot setter (0x2778e8), the registry record restore loop
//! (0x277e60) and its boot wrapper (0x277f18).
use crate::cue_tables;
use crate::h;
use crate::mathx;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const V1: usize = 3;
const A0: usize = 4;
const A1: usize = 5;
const A2: usize = 6;
const A3: usize = 7;
const T0: usize = 8;
const T1: usize = 9;
const T2: usize = 10;
const T3: usize = 11;
const T4: usize = 12;
const S0: usize = 16;
const S1: usize = 17;
const S2: usize = 18;
const S3: usize = 19;
const S4: usize = 20;
const SP: usize = 29;
const RA: usize = 31;

/// High half of the tier addresses (`lui 0x1b`); also the value retail leaves in t4.
const TIER_HI: u32 = 0x001b_0000;
/// Save-tier halfword table (`TIER_HI + 0x6a00`).
const TIER_TABLE: u32 = TIER_HI + 0x6a00;
/// Slot-id word block (`TIER_HI + 0x6a78`).
const SLOT_WORDS: u32 = TIER_HI + 0x6a78;
/// Flag word read by the setter (`lui 0x16` + `lw -0x1170`); when nonzero the halfword at +0x60 is cleared.
const CLEAR_FLAG: u32 = 0x0016_0000 - 0x1170;
/// Registry base the boot wrapper passes (`lui 0x1c` + `addiu -0x4c80`).
const REGISTRY_BASE: u32 = 0x001c_0000 - 0x4c80;
/// 0x277e18: retail lookup of the first actor with a uid pair (returns the actor or 0 in v0).
const FIND_ACTOR: u32 = 0x0027_7e18;

fn gpr(c: &Cpu, reg: usize) -> u32 {
    c.r[reg] as u32
}

fn set(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// `sq reg, 0(addr)`: stores the whole quadword register.
fn save(c: &mut Cpu, addr: u32, reg: usize) {
    let v = c.r[reg];
    for i in 0..4 {
        c.w32(addr.wrapping_add(4 * i), (v >> (32 * i)) as u32);
    }
}

/// Runs a retail-or-hand callee with `jal` register semantics and returns its v0.
fn call_v0(c: &mut Cpu, f: fn(&mut Cpu) -> bool) -> u32 {
    let _ = h::run_keep(c, f, &[], &[]);
    gpr(c, V0)
}

/// 0x2778e8 save_tier_slots() -> (): sets the save-tier halfwords at 0x1b6a00 from the two shift counts and the gold
/// count (2 or 3, or 10 for gold), then the four slot-id words at 0x1b6a78 (0x4fd3..0x4fdb), and clears the halfword
/// at 0x1b6a60 when the flag word at 0x15ee90 is nonzero.
pub fn save_tier_slots(c: &mut Cpu) -> bool {
    let frame = gpr(c, SP).wrapping_sub(0x30);
    save(c, frame.wrapping_add(0x10), S1);
    save(c, frame, S0);
    save(c, frame.wrapping_add(0x20), RA);
    let shifts1 = call_v0(c, cue_tables::shift_count);
    let big1 = (shifts1 as i32) >= 15;
    let shifts2 = call_v0(c, cue_tables::shift_count);
    let big2 = (shifts2 as i32) >= 30;
    let gold = call_v0(c, cue_tables::gold_bolts_spent);
    let big3 = (gold as i32) >= 10;

    c.w16(TIER_TABLE + 0x3e, if big1 { 3 } else { 2 });
    c.w16(TIER_TABLE + 0x4a, if big2 { 3 } else { 2 });
    c.w16(TIER_TABLE + 0x56, if big3 { 10 } else { 2 });
    c.w16(TIER_TABLE + 0x62, if big3 { 3 } else { 2 });

    let v0_val: u32 = if big3 { 0x4fd8 } else { 0x4fdb };
    let a0_val: u32 = if big3 { 0x4fd7 } else { 0x4fdb };
    let a1_val: u32 = if big2 { 0x4fd4 } else { 0x4fda };
    let a2_val: u32 = if big1 { 0x4fd3 } else { 0x4fd9 };
    let flag = c.r32(CLEAR_FLAG);
    c.w32(SLOT_WORDS + 0x14, a2_val);
    c.w32(SLOT_WORDS + 0x18, a1_val);
    c.w32(SLOT_WORDS + 0x1c, a0_val);
    c.w32(SLOT_WORDS + 0x20, v0_val);
    if flag != 0 {
        c.w16(TIER_TABLE + 0x60, 0);
    }

    set(c, V0, if flag != 0 { TIER_TABLE } else { v0_val });
    set(c, V1, SLOT_WORDS);
    set(c, A0, a0_val);
    set(c, A1, a1_val);
    set(c, A2, a2_val);
    set(c, A3, flag);
    set(c, T0, 0x4fd4);
    set(c, T1, 0x4fd7);
    set(c, T2, 0x4fd3);
    set(c, T3, big3 as u32);
    set(c, T4, TIER_HI);
    true
}

/// 0x277e60 restore_registry_records(base) -> (): base holds a count word, then 16-byte records (uid pair at +0xc/+0xe,
/// flag at +0xa, size at +0x8, offset word at +0, data at +4). Each record finds its actor (0x277e18), skipping
/// missing ones, and copies its data (mem_copy) to the actor plus the offset, or to the actor's +0x78 block plus the
/// offset when the flag is not 1.
pub fn restore_registry_records(c: &mut Cpu) -> bool {
    let base = gpr(c, A0);
    let frame = gpr(c, SP).wrapping_sub(0x60);
    save(c, frame.wrapping_add(0x30), S3);
    save(c, frame.wrapping_add(0x20), S2);
    save(c, frame.wrapping_add(0x50), RA);
    save(c, frame.wrapping_add(0x40), S4);
    save(c, frame.wrapping_add(0x10), S1);
    save(c, frame, S0);
    let count = c.r32(base);
    if (count as i32) <= 0 {
        set(c, V0, count);
        return true;
    }
    let mut i: u32 = 0;
    loop {
        let rec = base.wrapping_add(4 + 16 * i);
        let uid_a = c.r16(rec + 0xe) as i16 as i32 as u32;
        let uid_b = c.r16(rec + 0xc) as i16 as i32 as u32;
        set(c, A0, uid_a);
        set(c, A1, uid_b);
        let _ = h::call_keep(c, FIND_ACTOR, &[uid_a, uid_b], &[]);
        let actor = gpr(c, V0);
        set(c, A0, actor);
        if actor != 0 {
            let flag = c.r16(rec + 0xa) as i16 as i32;
            let offset = c.r32(rec);
            set(c, V0, offset);
            let dst = if flag != 1 {
                let block = c.r32(actor + 0x78);
                set(c, V1, block);
                offset.wrapping_add(block)
            } else {
                offset.wrapping_add(actor)
            };
            let n = c.r16(rec + 8) as u32;
            set(c, A0, dst);
            set(c, A2, n);
            set(c, A1, rec + 4);
            let _ = h::run_keep(c, mathx::mem_copy, &[dst, rec + 4, n], &[]);
        }
        // Retail reloads the count word from memory each iteration (the copy may alias it) and leaves the signed loop
        // test (i + 1 < count) in v0.
        i = i.wrapping_add(1);
        let count = c.r32(base);
        let more = (i as i32) < (count as i32);
        set(c, V0, more as u32);
        if !more {
            break;
        }
    }
    true
}

/// 0x277f18 restore_registry_boot() -> (): restores the registry records at 0x1bb380 (0x277e60).
pub fn restore_registry_boot(c: &mut Cpu) -> bool {
    let sp = gpr(c, SP).wrapping_sub(0x10);
    save(c, sp, RA);
    set(c, SP, sp);
    set(c, A0, REGISTRY_BASE);
    restore_registry_records(c);
    set(c, SP, sp.wrapping_add(0x10));
    true
}
