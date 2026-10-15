//! Path table helpers: segment lengths for a closed path, and the path point nearest a position.
use crate::h;
use crate::ps2::*;
use crate::vu_vec;
use ee_oracle::cpu::Cpu;

/// Path table in RAM: entry for index n is the path pointer at PATH_TABLE + 4 * n.
const PATH_TABLE: u32 = 0x001b_05b0;
/// Path header: point count (word 0). Points follow at +0x10, 16 bytes each (x, y, z, length).
const PATH_POINTS: u32 = 0x10;
const POINT_SIZE: u32 = 0x10;
/// Field offsets relative to a point's base (PATH_POINTS + 16 * index): z at +0x8, segment length at +0xc.
const POINT_Z: u32 = 0x08;
const POINT_LEN: u32 = 0x0c;
/// Owner record fields: flag byte, counter word, selected path pointer, path table index.
const OWNER_FLAG: u32 = 0x05;
const OWNER_COUNTER: u32 = 0x08;
const OWNER_PATH: u32 = 0x10;
const OWNER_INDEX: u32 = 0x14;

/// Float constants with their exact bits.
const HALF: F = F(0x3f00_0000);
/// Initial "best so far" value of path_nearest (about 4e10).
const NEAR_INIT: F = F(0x51ba_43b7);

/// 0x268428 path_init_lengths(this): clears the owner flag, selects the path from the table by this.index, and
/// stores each point's distance to the next point (wrapping at the end) in its length field, nudging the z field by
/// +0.5 then -0.5 as retail does.
pub fn path_init_lengths(c: &mut Cpu) -> bool {
    let this = c.arg(0);
    c.w8(this + OWNER_FLAG, 0);
    let slot = PATH_TABLE + (c.r32(this + OWNER_INDEX) << 2);
    c.w32(this + OWNER_COUNTER, 0);
    let path = c.r32(slot);
    c.w32(this + OWNER_PATH, path);
    let count = c.r32(path) as i32;
    if count <= 0 {
        // retail's blez exit: a1 = 0 (delay slot), v0 = count, v1 = path, a0 = table base
        c.set_arg(1, 0);
        c.r[2] = count as u32 as u128;
        c.r[3] = path as u128;
        c.set_arg(0, PATH_TABLE);
        return true;
    }
    let count = count as u32;
    let mut last_z = F(0);
    let mut point = path;
    for j in 0..count {
        let next = (j + 1) % count;
        point = path + PATH_POINTS + j * POINT_SIZE;
        let there = path + PATH_POINTS + next * POINT_SIZE;
        let dist = dist3(c, point, there);
        c.wf(point + POINT_LEN, dist);
        let z = c.rf(point + POINT_Z) + HALF;
        last_z = z - HALF;
        c.wf(point + POINT_Z, last_z);
    }
    // Registers retail's last iteration leaves behind: a1 = count, v0 = 0 (slt), v1 = path, f0 = z.

    c.set_arg(1, count);
    c.r[2] = 0;
    c.r[3] = path as u128;
    c.fret(last_z);
    true
}

/// 0x268528 path_nearest(pos, path, target) -> index: index of the path point whose distance to pos is nearest to
/// target. Retail keeps the point's distance (not the gap) as the running minimum, and returns 0 for an empty path.
pub fn path_nearest(c: &mut Cpu) -> bool {
    let pos = c.arg(0);
    let path = c.arg(1);
    let target = c.farg(0);
    let count = c.r32(path) as i32;
    let mut best = 0u32;
    let mut best_dist = NEAR_INIT;
    for i in 0..count.max(0) as u32 {
        let point = path + PATH_POINTS + i * POINT_SIZE;
        let dist = dist3(c, point, pos);
        let gap = h::fabs(dist - target);
        if gap < best_dist {
            best_dist = dist;
            best = i;
        }
    }
    c.ret(best);
    true
}

/// 0x1f19e8 dist3(a, b) through the vector unit helper; keeps the callee's VU and scratch registers as retail does.
fn dist3(c: &mut Cpu, a: u32, b: u32) -> F {
    h::run_keep(c, vu_vec::dist3, &[a, b], &[]).1
}
