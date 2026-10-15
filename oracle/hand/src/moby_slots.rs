//! Moby slot setup and teardown (retail 0x2c3ff8, 0x2c4130, 0x2cd988).
use crate::actor;
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

const V0: usize = 2;
const A0: usize = 4;
/// Retail slot setup routine, called once per slot with (parent, slot, next slot, table entry).
const SLOT_SETUP: u32 = 0x002c_3ee0;
/// Retail moby free routine (kept retail so its leftover scratch registers match).
const FREE_MOBY: u32 = 0x0024_10e8;
/// Byte offset of the actor pointer in the object at `actor + ACTOR_INFO`.
const ACTOR_INFO: u32 = 0x78;
/// Offset of the child moby pointer inside the info block.
const INFO_CHILD: u32 = 0xf8;
/// (first slot index, gp-relative offset of the table entry) for each of the 14 setup calls; the second slot index
/// is always the first plus one.
const SLOT_CALLS: [(u32, i32); 14] = [
    (0x00, -0x53c0),
    (0x02, -0x53bc),
    (0x04, -0x53b8),
    (0x05, -0x53b4),
    (0x06, -0x53b0),
    (0x08, -0x53b8),
    (0x09, -0x53b4),
    (0x0a, -0x53b0),
    (0x0c, -0x53ac),
    (0x0e, -0x53ac),
    (0x10, -0x53a8),
    (0x14, -0x53a8),
    (0x12, -0x53a4),
    (0x16, -0x53a4),
];

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x2c3ff8 slot_setup_all(parent) -> void: runs the 14 retail slot setup calls for the parent actor.
pub fn slot_setup_all(c: &mut Cpu) -> bool {
    let parent = c.g(A0) as u32;
    for (slot, off) in SLOT_CALLS {
        let table_entry = c.r32((c.g(28) as u32).wrapping_add(off as u32));
        h::call_keep(c, SLOT_SETUP, &[parent, slot, slot + 1, table_entry], &[]);
    }
    true
}

/// 0x2c4130 actor_release(actor) -> void: frees the child moby of the actor's info block if any, sets up the slots,
/// then frees the actor itself.
pub fn actor_release(c: &mut Cpu) -> bool {
    let actor = c.g(A0) as u32;
    let info = c.r32(actor.wrapping_add(ACTOR_INFO));
    set_gpr(c, V0, info);
    let child = c.r32(info.wrapping_add(INFO_CHILD));
    if child != 0 {
        h::call_keep(c, FREE_MOBY, &[child], &[]);
    }
    h::run_keep(c, slot_setup_all, &[actor], &[]);
    h::call_keep(c, FREE_MOBY, &[actor], &[]);
    true
}

/// 0x2cd988 free_moby_entry(moby) -> void: frees the moby in a0 (tail call of free_moby).
pub fn free_moby_entry(c: &mut Cpu) -> bool {
    let moby = c.g(A0) as u32;
    h::run_keep(c, actor::free_moby, &[moby], &[]);
    true
}
