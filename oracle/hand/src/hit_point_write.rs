//! Hit ring point writer: `hit_write_point` appends a point record (position and direction vectors, the owner pointer,
//! two words and a distance float) to the 64-slot hit ring at 0x178200, and sets the record's moving flag from the
//! length of the direction vector.
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Ring table; its word at +0x14 is the next slot index.
const RING_TABLE: u32 = 0x0017_3f40;
const RING_INDEX: u32 = RING_TABLE + 0x14;
/// Base of the hit ring: 64 slots of 64 bytes.
const RING_BASE: u32 = 0x0017_8200;
const RING_MASK: u32 = 0x3f;
const SLOT_SHIFT: u32 = 6;
/// Object byte holding the object's current slot index (0xff = none).
const OBJ_SLOT: u32 = 0xa4;
const NO_SLOT: u32 = 0xff;

/// Slot fields.
const SLOT_POS: u32 = 0x00;
const SLOT_DIR: u32 = 0x10;
const SLOT_ARG1: u32 = 0x20;
const SLOT_ARG2: u32 = 0x24;
const SLOT_KEY: u32 = 0x2c;
const SLOT_MOVING: u32 = 0x30;
const SLOT_OWNER: u32 = 0x34;
const SLOT_ZERO: u32 = 0x38;

/// Direction length above which the point counts as moving (1e-4, bits 0x38d1b717).
const MOVE_EPSILON: F = F(0x38d1_b717);

fn set_gpr(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

/// 0x24c788 hit_write_point(obj, a1, a2, dist, pos, dir) -> void: writes a point record for obj into the hit ring
/// (skipped when obj's current slot is its own and nearer than dist), with the moving flag set when |dir| > 1e-4.
pub fn hit_write_point(c: &mut Cpu) -> bool {
    let obj = c.arg(0);
    let (a1, a2) = (c.arg(1), c.arg(2));
    let dist = c.farg(0);
    let pos_ptr = c.arg(3);
    let dir_ptr = c.arg(4);

    let slot = c.r8(obj + OBJ_SLOT) as u32;
    if slot != NO_SLOT {
        let cur = RING_BASE.wrapping_add(slot << SLOT_SHIFT);
        let owner = c.r32(cur + SLOT_OWNER);
        if owner == obj {
            let slot_key = c.rf(cur + SLOT_KEY);
            if dist < slot_key {
                // Retail returns here: v0 = the slot, v1 = its owner, a0 = ring base, f0 = its key.
                c.f[0] = slot_key.0;
                set_gpr(c, 2, cur);
                set_gpr(c, 3, owner);
                set_gpr(c, 4, RING_BASE);
                return true;
            }
        }
    }

    let idx = c.r32(RING_INDEX);
    let entry = RING_BASE.wrapping_add(idx << SLOT_SHIFT);
    c.w32(entry + SLOT_ARG1, a1);
    c.w32(entry + SLOT_ARG2, a2);
    c.wf(entry + SLOT_KEY, dist);
    c.w32(entry + SLOT_OWNER, obj);
    c.w32(entry + SLOT_ZERO, 0);
    let (_, len) = h::run(c, vecmath::length3, &[dir_ptr], &[]);
    let moving = MOVE_EPSILON < len;
    c.w32(entry + SLOT_MOVING, moving as u32);
    let pos = c.rv(pos_ptr);
    c.wv(entry + SLOT_POS, pos);
    let dir = c.rv(dir_ptr);
    c.wv(entry + SLOT_DIR, dir);
    c.w8(obj + OBJ_SLOT, idx as u8);
    let next = (idx + 1) & RING_MASK;
    c.w32(RING_INDEX, next);

    // Registers as retail leaves them at return.
    c.f[0] = len.0;
    c.f[1] = MOVE_EPSILON.0;
    set_gpr(c, 1, 0x38d1_b717);
    set_gpr(c, 2, RING_TABLE);
    set_gpr(c, 3, entry + SLOT_DIR);
    set_gpr(c, 4, next);
    true
}
