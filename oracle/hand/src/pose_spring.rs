//! Pose-copy frame builder for the hero (0x1fc370) and two spring-constant stores (0x1fd288, 0x1fd2a0).
use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Compare mask for packed quadwords: the low 48 bits (`-1 >> 16`).
const LOW48: u64 = 0x0000_ffff_ffff_ffff;
/// Quadword written for skipped slots: `0xfffe << 47`.
const SKIP_TAG: u64 = 0xfffe << 47;
/// Value a packed joint quadword is compared against to decide that it is an identity joint.
const IDENTITY_TAG: u64 = ((0x8000u64 << 16) | 0x8000) << 13 | 0x1000;
/// Float 32768.0 (scale of the first joint list).
const SCALE_32K: u32 = 0x4700_0000;
/// Float 4096.0 (scale of the second joint list).
const SCALE_4K: u32 = 0x4580_0000;
/// Float 1.0.
const ONE: u32 = 0x3f80_0000;
/// Scratchpad base of the joint matrices, one list per pass (base + index * 64).
const SCRATCH_PASS1: u32 = 0x7000_0000;
const SCRATCH_PASS2: u32 = 0x7000_0010;
const SCRATCH_PASS3: u32 = 0x7000_0020;
/// Retail helper called once at the start, and the global used when the context argument is zero.
const RETAIL_HELPER: u32 = 0x0024_7420;
const CONTEXT_GLOBAL: u32 = 0x0014_13d0;
/// Spring-constant block at 0x180000 - 0x5880.
const SPRING_BLOCK: u32 = 0x0018_0000 - 0x5880;

/// Callee-saved registers and their frame offsets (s0..s7, fp, ra).
const SAVED: [(usize, u32); 10] = [
    (16, 0x10),
    (17, 0x20),
    (18, 0x30),
    (19, 0x40),
    (20, 0x50),
    (21, 0x60),
    (22, 0x70),
    (23, 0x80),
    (30, 0x90),
    (31, 0xa0),
];

fn quad_to_v(q: u128) -> V {
    V(std::array::from_fn(|i| (q >> (32 * i)) as u32))
}

fn store64(c: &mut Cpu, addr: u32, v: u64) {
    c.w32(addr, v as u32);
    c.w32(addr.wrapping_add(4), (v >> 32) as u32);
}

fn lanes_to_u128(l: [u32; 4]) -> u128 {
    (0..4).fold(0u128, |acc, i| acc | (l[i] as u128) << (32 * i))
}

/// Packs `(vector at addr) * scale` truncated to four clamped signed halfwords (same as vec4_to_i16x4).
/// Also leaves the vector unit's registers as the retail routine does: at = clamped lanes, v0 = packed,
/// v1 = 0xffff8000 splat, vf1 = truncated lanes, vf2 = scale.
fn pack_vec(c: &mut Cpu, addr: u32, scale: u32) -> u64 {
    let raw = (c.rv(addr) * F(scale)).ftoi(0);
    let mut clamped = [0u32; 4];
    let mut packed = 0u64;
    for i in 0..4 {
        let h = (raw.0[i] as i32).clamp(-0x8000, 0x7fff);
        clamped[i] = h as u32;
        packed |= ((h as u16) as u64) << (16 * i);
    }
    c.r[1] = lanes_to_u128(clamped);
    c.r[2] = packed as u128;
    c.r[3] = lanes_to_u128([0xffff_8000; 4]);
    c.vf[1] = raw.0;
    c.vf[2] = [scale, 0, 0, 0x7fff];
    packed
}

/// Joint index `idx` of the list at `list` (words after the header), or None at the negative terminator.
fn joint_entry(c: &mut Cpu, list: u32, idx: u32) -> Option<u32> {
    let e = c.r32(list.wrapping_add(4 + 4 * idx));
    ((e as i32) >= 0).then_some(e)
}

/// Scratchpad address of the matrix/vector for joint `e` in a pass whose base is `base`.
fn joint_addr(base: u32, e: u32) -> u32 {
    base.wrapping_add(e << 6)
}

/// 0x1fc370 build_pose_copy_frame(list, out, src, extra, ctx): builds one animation frame at `out` from the joint
/// list (quantised rot/trans, then identity-joint runs) and writes the 0x10-byte frame header.
pub fn build_pose_copy_frame(c: &mut Cpu) -> bool {
    let list = c.arg(0);
    let out = c.arg(1);
    let src = c.arg(2);
    let extra = c.arg(3);
    let ctx_arg = c.arg(4);
    let frame = (c.g(29) as u32).wrapping_sub(0xc0);

    // Prologue: save the callee-saved registers, f20 and src into the frame, as the retail prologue does.
    for (reg, off) in SAVED {
        let q = c.r[reg];
        c.wv(frame.wrapping_add(off), quad_to_v(q));
    }
    c.wf(frame.wrapping_add(0xb0), F(c.f[20]));
    c.w32(frame, src);

    let ctx = if ctx_arg != 0 { ctx_arg } else { c.r32(CONTEXT_GLOBAL) };
    let _ = h::call(c, RETAIL_HELPER, &[ctx], &[]);

    store64(c, out.wrapping_add(0x10), SKIP_TAG);
    let mut dst = out.wrapping_add(0x18);

    // Pass 1: one quadword per listed joint (scale 32768), counting entries from 1.
    let mut count: u32 = 1;
    let mut idx = 0;
    while let Some(e) = joint_entry(c, list, idx) {
        count += 1;
        let v = pack_vec(c, joint_addr(SCRATCH_PASS1, e), SCALE_32K);
        store64(c, dst, v);
        dst = dst.wrapping_add(8);
        idx += 1;
    }
    // `extra` skip quadwords.
    if (extra as i32) > 0 {
        for _ in 0..extra {
            store64(c, dst, SKIP_TAG);
            dst = dst.wrapping_add(8);
            count += 1;
        }
    }
    let first_entry = c.r32(list.wrapping_add(4)) as i32;
    let count_first_shifted = count << 3;
    c.w32(frame.wrapping_add(4), count_first_shifted);

    // Pass 2: identity-free joints (scale 4096), tagged with their slot number in halfword 3.
    let mut tags: u32 = 0;
    let mut slot: u32 = 1;
    if first_entry >= 0 {
        let mut idx = 0;
        while let Some(e) = joint_entry(c, list, idx) {
            let v = pack_vec(c, joint_addr(SCRATCH_PASS2, e), SCALE_4K);
            if (v & LOW48) != IDENTITY_TAG {
                store64(c, dst, v);
                tags += 1;
                c.w16(dst.wrapping_add(6), (slot as u16) | 0x8000);
                dst = dst.wrapping_add(8);
            }
            slot += 1;
            idx += 1;
        }
    }
    count += tags;
    c.w32(frame.wrapping_add(8), count);
    let count_mid = count;

    // Pass 3: joints that differ from the reference transform (scale 1.0), tagged with their index.
    let mut changed: u32 = 0;
    if first_entry >= 0 {
        let mut n: u32 = 1;
        let mut idx = 0;
        while let Some(e) = joint_entry(c, list, idx) {
            let a = pack_vec(c, joint_addr(SCRATCH_PASS3, e), ONE) & LOW48;
            let base = c.r32(src.wrapping_add(0x18));
            let b = pack_vec(c, base.wrapping_add(n << 4), ONE) & LOW48;
            if a != b {
                store64(c, dst, a);
                changed += 1;
                c.w16(dst.wrapping_add(6), n as u16);
                dst = dst.wrapping_add(8);
            }
            n += 1;
            idx += 1;
        }
    }

    // Pad to a 16-byte boundary, then write the header.
    if dst & 0xf != 0 {
        store64(c, dst, 0);
        dst = dst.wrapping_add(8);
    }
    let total = ((dst.wrapping_sub(0x10).wrapping_sub(out)) as i32 >> 4) as u32;
    c.w16(out.wrapping_add(0xe), changed as u16);
    c.w16(out.wrapping_add(0xa), tags as u16);
    c.w16(out.wrapping_add(8), count_first_shifted as u16);
    c.w16(out.wrapping_add(6), total as u16);
    c.w16(out.wrapping_add(0xc), (count_mid << 3) as u16);
    c.w16(out, 0);
    c.w16(out.wrapping_add(2), 0);
    c.w16(out.wrapping_add(4), 0);

    // Leave the registers retail leaves behind.
    // Leave the registers retail leaves behind.
    c.ret(total);
    c.s(3, (count_mid << 3) as i32 as i64 as u64);
    c.s(4, count_first_shifted as i32 as i64 as u64);
    if (extra as i32) > 0 {
        c.s(7, 0);
    }
    if first_entry >= 0 {
        c.set_farg(0, F(ONE));
    }
    true
}

/// 0x1fd288 set_spring_pair_a(k, c): stores f12 and f13 into the spring block at +0xa4 and +0xa8.
pub fn set_spring_pair_a(c: &mut Cpu) -> bool {
    let (k, damp) = (c.farg(0), c.farg(1));
    c.wf(SPRING_BLOCK.wrapping_add(0xa4), k);
    c.wf(SPRING_BLOCK.wrapping_add(0xa8), damp);
    c.ret(SPRING_BLOCK);
    true
}

/// 0x1fd2a0 set_spring_pair_b(k, c): stores f12 and f13 into the spring block at +0x154 and +0x158.
pub fn set_spring_pair_b(c: &mut Cpu) -> bool {
    let (k, damp) = (c.farg(0), c.farg(1));
    c.wf(SPRING_BLOCK.wrapping_add(0x154), k);
    c.wf(SPRING_BLOCK.wrapping_add(0x158), damp);
    c.ret(SPRING_BLOCK);
    true
}
