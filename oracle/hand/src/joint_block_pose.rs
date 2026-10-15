//! Hero joint-block pose pass: 0x1f81b8 joint_state_slot, 0x1f8210 hero_joint_block_pose and
//! 0x1f8590 hero_joint_blocks_pose (31 blocks of 0xb0 bytes starting at 0x17a780).
use crate::actor;
use crate::geom;
use crate::h;
use crate::motion;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Mode byte: when nonzero, the kind filter only skips kind 0.
const MODE_FLAG: u32 = 0x0014_13f4;
/// Joint-state table word for kinds 0, 2, 3 and 4.
const STATE_SLOT_A: u32 = 0x0014_13d0;
/// Joint-state table word for kind 1.
const STATE_SLOT_B: u32 = 0x0014_04d4;
/// Joint-state table word for kind 5.
const STATE_SLOT_C: u32 = 0x0014_03e0;
/// Byte flag: when nonzero, the settle path negates the offsets 0x60, 0x68, 0x74 and uses `K0`.
const FLAG2: u32 = 0x0016_0000 - 0x124b;
/// Float constant used as the spring time scale.
const K0_ADDR: u32 = 0x0016_0000 - 0x129c;
/// Retail helper called to finish a block (its meaning is not yet named).
const RETAIL_CLEAR: u32 = 0x001f_17f8;
/// First joint block (0x180000 - 0x5880).
const BLOCK_BASE: u32 = 0x0018_0000 - 0x5880;
/// Span of the block range (31 blocks at 0xb0 stride).
const BLOCK_SPAN: u32 = 0x1550;
/// Size of one joint block.
const BLOCK_STRIDE: u32 = 0xb0;

const ZERO: F = F(0);
const ONE: F = F(0x3f80_0000);
/// Rest threshold for offsets 0x70, 0x74, 0x78 (0x3b449ba6).
const EPS_POS: F = F(0x3b44_9ba6);
/// Rest threshold for offsets 0x40, 0x44, 0x48 (0x3ba3d70a).
const EPS_ANG: F = F(0x3ba3_d70a);

fn fv(x: F) -> f32 {
    x.get()
}

/// 0x1f81b8 joint_state_slot(state) -> u32: picks a joint-state table word for the kind (0 if none).
pub fn joint_state_slot_hand(c: &mut Cpu) -> bool {
    let state = c.arg(0) as i32;
    let v = joint_state_slot(c, state);
    c.ret(v);
    true
}

fn joint_state_slot(c: &mut Cpu, state: i32) -> u32 {
    match state {
        0 | 2 | 3 | 4 => c.r32(STATE_SLOT_A),
        1 => c.r32(STATE_SLOT_B),
        5 => c.r32(STATE_SLOT_C),
        _ => 0,
    }
}

/// 0x1f8210 hero_joint_block_pose(blk): one joint block pose step (settles the spring targets while
/// moving, otherwise only re-attaches or detaches the layer).
pub fn hero_joint_block_pose(_c: &mut Cpu) -> bool {
    // Declined: the moving-block settle path does not match retail (cause not found), so retail runs it.
    false
}

/// 0x1f8590 hero_joint_blocks_pose(): runs the block pose step over all 31 hero joint blocks.
pub fn hero_joint_blocks_pose(_c: &mut Cpu) -> bool {
    // Declined: checker BAD 174/300 (block 2 offset 0x10, euler_to_quat output); retail runs it.
    false
}

fn pose_block(c: &mut Cpu, blk: u32) {
    let kind = c.r16(blk.wrapping_add(0xa2)) as i16 as i32;
    let slot = joint_state_slot(c, kind);
    if slot == 0 {
        return;
    }
    if c.r8(MODE_FLAG) != 0 {
        if kind == 0 {
            return;
        }
    } else if matches!(kind, 2 | 3 | 4) {
        return;
    }
    match at_rest(c, blk) {
        None => {
            if c.r8(blk.wrapping_add(1)) != 0 {
                h::run(c, actor::layer_detach, &[slot, blk], &[]);
            }
            finish(c, blk);
        }
        Some(f0) => settle(c, blk, slot, f0),
    }
}

/// Returns None when the block is at rest; otherwise Some(f0) with the value the retail code leaves in f0
/// at the failing compare.
fn at_rest(c: &mut Cpu, blk: u32) -> Option<F> {
    for off in [0x60, 0x64, 0x68, 0x90, 0x94, 0x98] {
        let v = c.rf(blk.wrapping_add(off));
        if fv(v) != 0.0 {
            return Some(v);
        }
    }
    for off in [0x70, 0x74, 0x78] {
        let v = c.rf(blk.wrapping_add(off)).abs();
        if !(fv(v) < fv(EPS_POS)) {
            return Some(v);
        }
    }
    if fv(c.rf(blk.wrapping_add(0xac))) != fv(ONE) {
        return Some(ONE);
    }
    for off in [0x40, 0x44, 0x48] {
        let v = c.rf(blk.wrapping_add(off)).abs();
        if !(fv(v) < fv(EPS_ANG)) {
            return Some(v);
        }
    }
    None
}

/// Moving-block path: spring-settles the joint targets, re-attaches the layer and copies the target quad.
fn settle(c: &mut Cpu, blk: u32, slot: u32, f0_in: F) {
    let mut f0 = f0_in;
    if c.r8(FLAG2) != 0 {
        for off in [0x60, 0x68, 0x74] {
            let v = c.rf(blk.wrapping_add(off));
            c.wf(blk.wrapping_add(off), -v);
        }
        f0 = c.rf(K0_ADDR);
    }
    let k0 = c.rf(K0_ADDR);
    spring_angle(c, blk, 0x40, 0x50, 0x60, f0);
    spring_angle(c, blk, 0x44, 0x54, 0x64, k0);
    spring_angle(c, blk, 0x48, 0x58, 0x68, k0);
    spring_hand(c, blk, 0x70, 0x80, 0x90);
    spring_hand(c, blk, 0x74, 0x84, 0x94);
    spring_hand(c, blk, 0x78, 0x88, 0x98);
    if c.r8(blk.wrapping_add(1)) == 0 {
        let len = c.r16(blk.wrapping_add(0xa0)) as i16 as i32 as u32;
        h::run(c, actor::layer_attach, &[slot, len, blk], &[]);
    }
    h::run(c, geom::euler_to_quat, &[blk.wrapping_add(0x10), blk.wrapping_add(0x40)], &[]);
    let quad = c.rv(blk.wrapping_add(0x70));
    c.wv(blk.wrapping_add(0x30), quad);
    let axis = c.rf(blk.wrapping_add(0xac));
    c.wf(blk.wrapping_add(0x28), axis);
    c.wf(blk.wrapping_add(0x20), axis);
    c.wf(blk.wrapping_add(0x24), axis);
    finish(c, blk);
}

/// Turn-spring on the block's angle pair; `scale` multiplies the gains at 0xa4 and 0xa8.
fn spring_angle(c: &mut Cpu, blk: u32, a: u32, b: u32, v: u32, scale: F) {
    let gain_a = c.rf(blk.wrapping_add(0xa4)) * scale;
    let gain_b = c.rf(blk.wrapping_add(0xa8)) * scale;
    let value = c.rf(blk.wrapping_add(v));
    h::run(
        c,
        motion::turn_spring_angle,
        &[blk.wrapping_add(a), blk.wrapping_add(b), 2],
        &[value, gain_a, gain_b, ZERO],
    );
}

/// Hand spring-settle of the value at `v` toward the pair at `a`/`b`.
fn spring_hand(c: &mut Cpu, blk: u32, a: u32, b: u32, v: u32) {
    let value = c.rf(blk.wrapping_add(v));
    let gain_a = c.rf(blk.wrapping_add(0xa4));
    let gain_b = c.rf(blk.wrapping_add(0xa8));
    h::run(
        c,
        motion::spring_settle_hand,
        &[blk.wrapping_add(a), blk.wrapping_add(b)],
        &[value, gain_a, gain_b, ZERO],
    );
}

/// Common tail: calls the retail helper on the two blocks and sets the axis to 1.0.
fn finish(c: &mut Cpu, blk: u32) {
    h::call(c, RETAIL_CLEAR, &[blk.wrapping_add(0x90)], &[]);
    h::call(c, RETAIL_CLEAR, &[blk.wrapping_add(0x60)], &[]);
    c.wf(blk.wrapping_add(0xac), ONE);
}
