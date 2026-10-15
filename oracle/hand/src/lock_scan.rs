//! Hero lock-on check from the L4_17 record: picks the lock target, then rejects it when another class 0x154 object
//! stands closer to the hero than the 3.0 limit or closer to the target than the hero is.
use crate::h::{self, SQ};
use crate::ps2::*;
use crate::targets;
use ee_oracle::cpu::Cpu;

/// Head of the object list (`lui 0x16` + `lw -0x9c`); each object links to the next at +0x28.
const OBJECT_LIST_HEAD: u32 = 0x0015_FF64;
/// Frame size of the retail function (`addiu sp, sp, -0xc0`).
const FRAME: u32 = 0xc0;
/// Object class (`lh +0xa6`) that blocks a lock.
const CLASS_BLOCKER: u16 = 0x154;
/// Near limit for a blocker, 3.0 (`lui 0x4040`).
const NEAR_LIMIT: F = F(0x4040_0000);

/// 0x2c17c0 lock_scan(player) -> 1 when the lock target is clear, 0 when a blocker is near: the lock range is
/// doubled when the hero's state word or flags at +0x136 are set, then `pick_lock_target` fills a stack aim block
/// whose target position is compared with every class 0x154 object.
pub fn lock_scan(c: &mut Cpu) -> bool {
    let player = c.arg(0);
    let saves = [
        (16, 0x50, SQ),
        (17, 0x60, SQ),
        (18, 0x70, SQ),
        (19, 0x80, SQ),
        (20, 0x90, SQ),
        (31, 0xa0, SQ),
    ];
    let verdict = h::frame(c, FRAME, &saves, |c, sp| {
        // Retail stores the float saves (f21 at 0xb8, f20 at 0xb0) in the same frame.
        let (f20, f21) = (c.f[20] as u32, c.f[21] as u32);
        c.w32(sp + 0xb8, f21);
        c.w32(sp + 0xb0, f20);

        let state = c.r32(player + 0x78);
        let flagged = c.r32(state + 0x38) != 0 || c.r16(state + 0x136) != 0;
        let base_range = c.rf(state + 0x140);
        let range = if flagged { base_range + base_range } else { base_range };
        h::run(c, targets::pick_lock_target, &[player, sp], &[range]);

        let hero_pos = player + 0x10;
        let target_dist = h::dist_xy(c, hero_pos, sp);
        let mut obj = c.r32(OBJECT_LIST_HEAD);
        while obj != 0 {
            if obj != player && c.r16(obj + 0xa6) == CLASS_BLOCKER {
                if h::dist_xy(c, hero_pos, obj + 0x10) < NEAR_LIMIT {
                    return 0;
                }
                if h::dist_xy(c, obj + 0x10, sp) < target_dist {
                    return 0;
                }
            }
            obj = c.r32(obj + 0x28);
        }
        1
    });
    c.ret(verdict);
    true
}
