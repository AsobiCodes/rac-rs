//! Camera blend-completion, class-index lookup and neighbour-order helpers (Eudora overlay 0x1df000-0x1e9c00 area).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Blend state block: record at +0, flag byte at +2, pose slots at +0x10 (settle) and +0x70 (step).
const BLEND_STATE: u32 = 0x166ff0;
/// Pose settle / step callbacks (retail).
const POSE_SETTLE: u32 = 0x1de960;
const POSE_STEP: u32 = 0x1deb40;
/// Saved pose destination: 4 quadwords, the last one at POSE_DEST - 0x210.
const POSE_DEST: u32 = 0x1670d0;
const POSE_DEST_LAST: u32 = 0x166ec0;
const POSE_DEST_B: u32 = 0x1670e0;
const POSE_DEST_C: u32 = 0x1670f0;
/// Owner word: when non-zero the pose copy is skipped.
const OWNER_WORD: u32 = 0x16c16c;
/// Upper half of the address the blend code loads its owner pointer from (lui value).
const OWNER_HI: u32 = 0x170000;

/// Camera class table: 0x14-byte records whose first word is the class id; -1 ends the table.
const CLASS_TABLE: u32 = 0x1dd080;
/// Upper half of the table address as left in a2 by the retail lui.
const CLASS_TABLE_HI: u32 = 0x1e0000;
const NONE_ID: u32 = u32::MAX;
const RECORD_SIZE: u32 = 0x14;

/// Neighbour-order threshold: the float argument is compared with 0.5 (bits 0x3f000000).
const HALF_BITS: u32 = 0x3f00_0000;
/// Neighbour probe (retail, returns non-zero when a neighbour was found).
const NEIGHBOUR_PROBE: u32 = 0x1e9ae0;

/// Copies the 0x30-byte pose at `pose` into the saved slots, leaving registers as retail does.
fn copy_pose(c: &mut Cpu, pose: u32) {
    let q0 = c.rv(pose);
    c.wv(POSE_DEST, q0);
    let q1 = c.rv(pose.wrapping_add(0x10));
    c.wv(POSE_DEST_B, q1);
    let q2 = c.rv(pose.wrapping_add(0x20));
    c.wv(POSE_DEST_C, q2);
    let q3 = c.rv(pose.wrapping_add(0x30));
    c.wv(POSE_DEST_LAST, q3);
    c.r[2] = c.mem.rd128(pose.wrapping_add(0x30));
    c.s(3, POSE_DEST_LAST as u64);
    c.s(4, pose.wrapping_add(0x30) as u64);
    c.s(5, POSE_DEST_B as u64);
    c.s(6, POSE_DEST_C as u64);
}

/// 0x1df118 cam_blend_done(pose) -> void: when the blend state is complete, copies the pose at a0 into the saved
/// slots (unless the owner word is set) and clears the blend flag and record.
pub fn cam_blend_done(c: &mut Cpu) -> bool {
    let pose = c.arg(0);
    let saves = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (31, 0x20, h::SQ)];
    h::frame(c, 0x30, &saves, |c, _sp| {
        c.s(16, pose as u64);
        c.s(17, BLEND_STATE as u64);
        let busy = c.r8(BLEND_STATE + 2) != 0;
        let done = if busy {
            h::call_keep(c, POSE_STEP, &[pose, BLEND_STATE + 0x70], &[]).0
        } else {
            h::call_keep(c, POSE_SETTLE, &[pose, BLEND_STATE + 0x10], &[]).0
        };
        c.s(2, OWNER_HI as u64);
        if done == 0 {
            return;
        }
        let owner = c.r32(OWNER_WORD);
        if owner != 0 {
            c.s(3, owner as i32 as i64 as u64);
        } else {
            copy_pose(c, pose);
        }
        c.w8(BLEND_STATE + 2, 0);
        c.w16(BLEND_STATE, 0);
    });
    true
}

/// 0x1dfa98 camera_class_index(class) -> index: position of the first class-table record whose id is `class` or
/// -1 (entry 0 is checked too; the table ends at -1).
pub fn camera_class_index(c: &mut Cpu) -> bool {
    let class = c.arg(0);
    let first = c.r32(CLASS_TABLE);
    c.s(3, first as i32 as i64 as u64);
    c.s(6, CLASS_TABLE_HI as u64);
    let mut idx = 0u32;
    if first != NONE_ID && first != class {
        c.s(6, u64::MAX);
        let mut rec = CLASS_TABLE + RECORD_SIZE;
        loop {
            idx += 1;
            let id = c.r32(rec);
            if id == NONE_ID {
                c.s(3, rec as u64);
                break;
            }
            if id == class {
                c.s(3, (rec + RECORD_SIZE) as u64);
                break;
            }
            rec += RECORD_SIZE;
        }
    }
    c.s(5, idx as u64);
    c.ret(idx);
    true
}

/// 0x1e9bb8 occ_nearer_order(a0..a3, t0, t1, f12) -> result: probes the neighbour pair in the order chosen by the
/// float threshold (below 0.5: (a0,a1,a2) first; else (a3,t0,t1) first) and falls back to the other pair.
pub fn occ_nearer_order(c: &mut Cpu) -> bool {
    let first_pair = [c.arg(0), c.arg(1), c.arg(2)];
    let second_pair = [c.arg(3), c.arg(4), c.arg(5)];
    let near = c.farg(0) < F(HALF_BITS);
    let saves = [
        (16, 0x00, h::SQ),
        (17, 0x10, h::SQ),
        (18, 0x20, h::SQ),
        (19, 0x30, h::SQ),
        (20, 0x40, h::SQ),
        (21, 0x50, h::SQ),
        (31, 0x60, h::SQ),
    ];
    h::frame(c, 0x70, &saves, |c, _sp| {
        c.f[0] = HALF_BITS;
        let (a, b) = if near { (first_pair, second_pair) } else { (second_pair, first_pair) };
        let v = h::call_keep(c, NEIGHBOUR_PROBE, &a, &[]).0;
        if v == 0 {
            h::call_keep(c, NEIGHBOUR_PROBE, &b, &[]);
        }
    });
    true
}
