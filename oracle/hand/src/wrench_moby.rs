//! Wrench aim and spawn helpers (retail 0x299f48 and 0x29b4e0).

use crate::h;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// 105 degrees in radians (0x3fea927f).
const WRENCH_LIMIT: F = F(0x3fea_927f);
/// Scale floor, 0.7 (0x3f333333).
const SCALE_BASE: F = F(0x3f33_3333);
/// Scale span, 0.3 (0x3e99999a).
const SCALE_SPAN: F = F(0x3e99_999a);
/// Base the retail code loads with lui (0x140000); the hero yaw float sits at offset -0xc18 from it.
const HERO_YAW_BASE: u32 = 0x14_0000;
const HERO_YAW_ADDR: u32 = HERO_YAW_BASE - 0xc18;
/// Retail helpers called by the aim push.
const FAST_ATAN2: u32 = 0x1f_1e48;
const ANGLE_DIST: u32 = 0x1f_2788;
const LENGTH3: u32 = 0x1f_1970;
const VEC_SET_LENGTH: u32 = 0x1f_1a98;
/// Moby class id passed to alloc_moby by the wrench spawn.
const WRENCH_MOBY_CLASS: u32 = 0x593;
/// Byte offsets inside the spawned moby.
const MOBY_QUAD: u32 = 0x10;
const MOBY_FLAGS: u32 = 0x20;
const MOBY_COLOUR: u32 = 0x30;
const MOBY_OWNER: u32 = 0x24;
const MOBY_SCALE: u32 = 0x2c;
const MOBY_SUB: u32 = 0x78;

/// 0x299f48 wrench_aim_push(v) -> (): scales the xyz vector at a0 to |v| * (0.7 + 0.3 * (105deg - d) / 105deg),
/// where d = angle_dist(hero yaw, atan2(y, x)) capped at 105 degrees.
pub fn wrench_aim_push(c: &mut Cpu) -> bool {
    let vec = c.arg(0);
    let saved_f20 = c.f[20];
    let saved_f21 = c.f[21];
    h::frame(c, 0x30, &[(16, 0x00, h::SQ), (31, 0x10, h::SQ)], |c, sp| {
        c.wf(sp + 0x20, F(saved_f20 as u32));
        c.wf(sp + 0x28, F(saved_f21 as u32));
        let x = c.rf(vec);
        let y = c.rf(vec + 4);
        let heading = h::call_keep(c, FAST_ATAN2, &[], &[x, y]).1;
        // retail: lui v0,0x14 (the base of the hero yaw load), then the load in the delay slot
        c.s(2, u64::from(HERO_YAW_BASE));
        let hero_yaw = c.rf(HERO_YAW_ADDR);
        let mut diff = h::call_keep(c, ANGLE_DIST, &[], &[hero_yaw, heading]).1;
        if WRENCH_LIMIT < diff {
            diff = WRENCH_LIMIT;
        }
        let len = h::call_keep(c, LENGTH3, &[vec], &[]).1;
        let fraction = (WRENCH_LIMIT - diff) / WRENCH_LIMIT;
        // retail leaves these constants in f1 / f2 and at before the last call
        c.f[1] = SCALE_SPAN.0;
        c.f[2] = SCALE_BASE.0;
        c.s(1, u64::from(SCALE_BASE.0));
        let scale = fraction * SCALE_SPAN + SCALE_BASE;
        h::call_keep(c, VEC_SET_LENGTH, &[vec, vec], &[len * scale]);
    });
    true
}

/// 0x29b4e0 wrench_spawn(src, flag, x, y, z) -> moby: allocates a wrench moby (class 0x593), copies the quad at src
/// into it, stores the flag and the three floats and returns the moby, or 0 when none is free.
pub fn wrench_spawn(c: &mut Cpu) -> bool {
    let src = c.arg(1);
    let flag = c.arg(2);
    let x = c.farg(0);
    let y = c.farg(1);
    let z = c.farg(2);
    let saved = [c.f[20], c.f[21], c.f[22]];
    let saves = [(16, 0x00, h::SQ), (17, 0x10, h::SQ), (18, 0x20, h::SQ), (31, 0x30, h::SQ)];
    let moby = h::frame(c, 0x60, &saves, |c, sp| {
        c.wf(sp + 0x40, F(saved[0] as u32));
        c.wf(sp + 0x48, F(saved[1] as u32));
        c.wf(sp + 0x50, F(saved[2] as u32));
        let moby = h::run(c, crate::actor::alloc_moby, &[WRENCH_MOBY_CLASS], &[]).0;
        if moby == 0 {
            return 0;
        }
        let quad = c.rv(src);
        c.wv(moby + MOBY_QUAD, quad);
        h::run(c, crate::packed_vu::pack_word_high, &[moby, 0x80, 0x80, 0x70], &[]);
        let sub = c.r32(moby + MOBY_SUB);
        c.wf(sub, y);
        let twice_x = x + x;
        c.wf(moby + MOBY_SCALE, twice_x);
        let owner = c.r32(moby + MOBY_OWNER);
        let k = c.rf(owner + MOBY_OWNER);
        c.wf(moby + MOBY_SCALE, twice_x * k);
        c.wf(sub + 0xc, z);
        c.w32(sub + 8, flag);
        c.w32(sub + 4, flag);
        c.w8(moby + MOBY_COLOUR, 0xff);
        c.w16(moby + MOBY_COLOUR + 2, 0xff);
        c.w8(moby + MOBY_COLOUR + 1, 1);
        c.w8(moby + MOBY_FLAGS, 0);
        moby
    });
    c.ret(moby);
    true
}
