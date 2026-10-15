//! Hero blend pose: the end-of-frame callback list push, the hero blend encode and the mode-0 blend source copy.
//! All three work on the hero block at 0x166ff0 (blend rows at +0x50..+0xd0, mode flag at +2).
use crate::h;
use crate::ps2::*;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Queued end-of-camera-frame callback count (signed word, at most 16).
const FRAME_LIST_COUNT: u32 = 0x0015_ef4c;
/// The 16-slot end-of-camera-frame callback list.
const FRAME_LIST: u32 = 0x0016_90f0;
/// Hero block: blend rows at +0x50..+0xd0, mode flag byte at +2.
const HERO_BLOCK: u32 = 0x0016_6ff0;
/// Pose table base; the word at +0x2080 holds the pointer to the three pose vectors.
const POSE_BASE: u32 = 0x0013_f350;
/// Retail blend routine that `hero_blend_encode` calls.
const BLEND_POSE: u32 = 0x0001_de398;
/// Stack frame size of `hero_blend_encode`.
const FRAME_SIZE: u32 = 0x90;
/// Float 1.0 (the vector length passed to vec_set_length).
const ONE: F = F(0x3f80_0000);

/// Sets a GPR to a 32-bit value, sign-extended as lw / addiu leave it.
fn set32(c: &mut Cpu, r: usize, v: u32) {
    c.s(r, v as i32 as i64 as u64);
}

/// 0x1dd988 frame_list_push(item): queue `item` in the 16-slot end-of-camera-frame list unless it is already there
/// or the list is full (the count word is then left alone).
pub fn frame_list_push(c: &mut Cpu) -> bool {
    let item = c.arg(0);
    let count = c.r32(FRAME_LIST_COUNT) as i32;
    // Scratch registers as the retail body leaves them: a2 = count, a3 = item, t0 = list segment (0x170000).
    set32(c, 6, count as u32);
    set32(c, 7, item);
    if count >= 16 {
        // The retail blez/beqz pair skips the t0 load and the v1/a1 setup here; a0 = count (delay slot of beqz).
        set32(c, 4, count as u32);
        set32(c, 2, 0);
        return true;
    }
    // Fall-through path: the blez delay slot loads t0 = 0x170000 before the scan.
    set32(c, 8, 0x0017_0000);
    set32(c, 4, count as u32);
    // Scan the queued entries; a match returns with v1 = index + 1 and a1 = the matching slot.
    // The retail body loads a1 = list base only when count > 0 (the blez skips that addiu).
    let mut slot = if count > 0 { FRAME_LIST } else { c.arg(1) };
    for i in 0..count {
        let v = c.r32(slot);
        if v == item {
            set32(c, 2, v);
            set32(c, 3, (i + 1) as u32);
            set32(c, 5, slot);
            return true;
        }
        slot = slot.wrapping_add(4);
    }
    // Not queued: append at index count, then bump the count (a1 ends at the slot after the scan).
    let at_slot = FRAME_LIST.wrapping_add((count as u32).wrapping_mul(4));
    c.w32(at_slot, item);
    c.w32(FRAME_LIST_COUNT, (count + 1) as u32);
    set32(c, 1, 0x0016_0000);
    set32(c, 2, FRAME_LIST);
    set32(c, 3, at_slot);
    set32(c, 4, (count + 1) as u32);
    set32(c, 5, slot);
    true
}

/// 0x1de578 hero_blend_encode(): normalises the three pose vectors to unit length into the stack, copies the two
/// outer ones into the hero block (+0x90, +0xa0), runs the retail blend 0x1de398 on them and copies its result
/// (+0xd0) to +0xb0.
pub fn hero_blend_encode(c: &mut Cpu) -> bool {
    let block = HERO_BLOCK;
    let s2 = POSE_BASE;
    let saves = [(16, 0x30, h::SQ), (17, 0x40, h::SQ), (18, 0x50, h::SQ), (19, 0x60, h::SQ), (31, 0x70, h::SQ)];
    h::frame(c, FRAME_SIZE, &saves, |c, sp| {
        // The retail body spills f20 at +0x80 and then uses it as the constant 1.0; it restores f20 on exit,
        // so f20 itself is left alone here and the constant goes straight into f12.
        let f20 = c.f[20];
        c.w32(sp.wrapping_add(0x80), f20);
        set32(c, 18, s2);
        let out = [sp, sp.wrapping_add(0x10), sp.wrapping_add(0x20)];
        set32(c, 19, out[1]);
        set32(c, 17, out[2]);
        for (k, dst) in out.iter().enumerate() {
            let pose = c.r32(s2.wrapping_add(0x2080));
            let src = pose.wrapping_add(0xc0 + 0x10 * k as u32);
            h::run_keep(c, vecmath::vec_set_length, &[*dst, src], &[ONE]);
        }
        // Outer pose vectors go to the block: +0x90 from sp+0, +0xa0 from sp+0x20.
        let q0 = c.mem.rd128(sp);
        c.r[2] = q0;
        c.mem.wr128(block + 0x90, q0);
        set32(c, 3, block + 0x90);
        let q2 = c.mem.rd128(out[2]);
        c.r[2] = q2;
        c.mem.wr128(block + 0xa0, q2);
        set32(c, 4, block + 0xa0);
        // Retail blend: a0 = block+0x70, a1 = block+0xc0, a2 = s2+0x80, a3 = sp, t0 = sp+0x10, t1 = sp+0x20.
        h::call_keep(
            c,
            BLEND_POSE,
            &[block + 0x70, block + 0xc0, s2.wrapping_add(0x80), sp, out[1], out[2]],
            &[],
        );
        // Copy the blend result (+0xd0) to +0xb0 (s0 is advanced by 0xb0 first, as retail does).
        set32(c, 3, block + 0xd0);
        let q = c.mem.rd128(block + 0xd0);
        c.r[2] = q;
        c.s(16, (block + 0xb0) as u64);
        c.mem.wr128(block + 0xb0, q);
    });
    true
}

/// 0x1de6d0 blend_mode0_from_mode2(): when the hero mode flag is set, copies the blend row at +0xc0 to +0x50 and
/// +0xd0 to +0x60 (the mode-0 blend source); otherwise does nothing.
pub fn blend_mode0_from_mode2(c: &mut Cpu) -> bool {
    let block = HERO_BLOCK;
    set32(c, 2, 0x0016_0000);
    set32(c, 6, block);
    let flag = c.r8(block + 2);
    set32(c, 3, flag as u32);
    set32(c, 4, block + 0x50);
    if flag == 0 {
        return true;
    }
    set32(c, 3, block + 0xc0);
    let q0 = c.mem.rd128(block + 0xc0);
    c.r[2] = q0;
    c.mem.wr128(block + 0x50, q0);
    set32(c, 5, block + 0xd0);
    set32(c, 3, block + 0x60);
    let q1 = c.mem.rd128(block + 0xd0);
    c.r[2] = q1;
    c.mem.wr128(block + 0x60, q1);
    true
}
