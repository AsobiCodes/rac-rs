//! Proximity walk over the global actor list (retail 0x299df0).

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Head pointer of the global actor list (lw from 0x160000 - 0x9c).
const LIST_HEAD: u32 = 0x15_FF64;
/// Base of the retail data table; the word at +0x2080 is passed to the spawn helper (0x140000 - 0xcb0).
const TABLE: u32 = 0x13_F350;
const TABLE_SPAWN_ARG: u32 = 0x2080;
/// Node kinds that end the walk.
const KIND_END_A: u8 = 0xfe;
const KIND_END_B: u8 = 0xfd;
/// Node target type (halfword at ptr + 0x46) that gets the random spawn.
const TYPE_TARGET: i16 = 0x13;
/// Distance limit (2.7) below which a node counts as near.
const NEAR_LIMIT: F = F(0x402c_cccd);
/// Degrees to radians (pi / 180).
const DEG_TO_RAD: F = F(0x3c8e_fa35);
/// Random angle range: [-30, 30).
const RAND_LO: F = F(0xc1f0_0000);
const RAND_HI: F = F(0x41f0_0000);
const ZERO: F = F(0);

const NODE_NEXT: u32 = 0x28;
const NODE_KIND: u32 = 0x20;
const NODE_PTR: u32 = 0x24;
const PTR_TYPE: u32 = 0x46;

/// Retail helpers.
const SPAWN_RANDOM: u32 = 0x298d40;
const HIT_NODE: u32 = 0x2b85d0;

/// Saved registers of the retail prologue (s0-s5, ra) at their frame offsets.
const SAVES: [(usize, u32, u8); 7] = [
    (16, 0x00, h::SQ),
    (17, 0x10, h::SQ),
    (18, 0x20, h::SQ),
    (19, 0x30, h::SQ),
    (20, 0x40, h::SQ),
    (21, 0x50, h::SQ),
    (31, 0x60, h::SQ),
];

/// Frame offsets where retail stores f20 and f21 (swc1).
const SAVE_F20: u32 = 0x70;
const SAVE_F21: u32 = 0x78;

/// Distance from a node to the target actor (VU0 dist3 of their positions), as f0.
fn near_dist(c: &mut Cpu, node: u32, target: u32) -> F {
    h::run(c, crate::vu_vec::dist3, &[node + 0x10, target + 0x10], &[]).1
}

/// True when the node's class is one of the special classes (path::is_special_class).
fn special_class(c: &mut Cpu, node: u32) -> bool {
    h::run(c, crate::path::is_special_class, &[node], &[]).0 != 0
}

/// 0x299df0 proximity_walk(target) -> void: walks the global actor list from its head until a node of kind 0xfe or
/// 0xfd. A node of type 0x13 within 2.7 of the target gets a random-angle spawn (0x298d40), which ends the walk when
/// it returns nonzero. Other nodes that are special-class and within 2.7 get 0x2b85d0, which ends the walk when it
/// returns nonzero. Every other node is skipped.
pub fn proximity_walk(c: &mut Cpu) -> bool {
    if crate::h::unwind_pending(c) {
        return false;
    }
    let target = c.arg(0);
    let head = c.r32(LIST_HEAD);
    if head == 0 {
        return true;
    }
    h::frame(c, 0x80, &SAVES, |c, sp| {
        c.w32(sp + SAVE_F20, c.f[20] as u32);
        c.w32(sp + SAVE_F21, c.f[21] as u32);
        let mut node = head;
        while node != 0 {
            let kind = c.r8(node + NODE_KIND);
            if kind == KIND_END_A || kind == KIND_END_B {
                break;
            }
            let ptr = c.r32(node + NODE_PTR);
            let typed = ptr != 0 && c.r16(ptr + PTR_TYPE) as i16 == TYPE_TARGET;
            if typed {
                if near_dist(c, node, target) < NEAR_LIMIT {
                    let angle = h::run(c, crate::rand::rand_float, &[], &[RAND_LO, RAND_HI]).1 * DEG_TO_RAD;
                    let spawn_arg = c.r32(TABLE + TABLE_SPAWN_ARG);
                    if h::call(c, SPAWN_RANDOM, &[node, spawn_arg], &[angle, ZERO]).0 != 0 {
                        break;
                    }
                }
            } else if special_class(c, node) && near_dist(c, node, target) < NEAR_LIMIT {
                if h::call(c, HIT_NODE, &[node], &[]).0 != 0 {
                    break;
                }
            }
            // Retail reads the link (0x28) after the calls above (0x299f10), so a callee that relinks is seen.
            node = c.r32(node + NODE_NEXT);
        }
    });
    true
}
