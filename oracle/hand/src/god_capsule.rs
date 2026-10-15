//! GoD capsule creation: allocates a moby, fills its pos/vel/parent block and scales its radius.

use crate::actor;
use crate::h::run_keep;
use crate::ps2::*;
use ee_oracle::cpu::Cpu;

/// Moby class id of a GoD capsule.
const CAPSULE_CLASS: u32 = 0xe6;
/// Radius scale factor (0.01 as f32 bits) applied to the capsule's size.
const SCALE_BITS: u32 = 0x3c23_d70a;
/// Base of the game-state block that holds the capsule flags.
const FLAG_BASE: u32 = 0x13_f350;
/// Flag byte that forces the extra render bits when set.
const FLAG_A: u32 = 0x20a5;
/// Second flag byte that forces the extra render bits when set.
const FLAG_B: u32 = 0x20af;
/// Render bits OR-ed into the moby's halfword at +0x34.
const RENDER_BITS: u16 = 0x41;

/// 0x2bc870 god_capsule_create(owner, src_pos, src_vel) -> moby or 0: allocates a capsule moby and fills it
pub fn god_capsule_create(c: &mut Cpu) -> bool {
    let (owner, src_pos, src_vel) = (c.arg(0), c.arg(1), c.arg(2));
    let (moby, _) = run_keep(c, actor::alloc_moby, &[CAPSULE_CLASS], &[]);
    c.s(6, moby as u64); // a2
    if moby == 0 {
        c.ret(0);
        return true;
    }
    c.w16(moby + 0x32, 0xff);
    c.w8(moby + 0x30, 0xff);
    c.w8(moby + 0x31, 1);
    c.w8(moby + 0x20, 0);
    let block = c.r32(moby + 0x78);
    c.s(5, (moby + 0x10) as u64); // a1
    let pos = c.rv(src_pos);
    c.wv(moby + 0x10, pos);
    let vel = c.rv(src_vel);
    c.wv(block, vel);
    c.w32(block + 0x30, owner);
    c.w16(block + 0x38, 0);
    c.w32(block + 0x34, 0);
    c.s(4, FLAG_BASE as u64); // a0
    c.s(3, block as u64); // v1
    c.s(1, SCALE_BITS as u64); // at
    c.f[1] = F(SCALE_BITS).0;
    let body = c.r32(moby + 0x24);
    let size = c.rf(body + 0x24);
    let scaled = size * F(SCALE_BITS);
    c.f[0] = scaled.0;
    c.wf(moby + 0x2c, scaled);
    let flag_a = c.r8(FLAG_BASE + FLAG_A);
    let flag_b = c.r8(FLAG_BASE + FLAG_B);
    if flag_a != 0 || flag_b != 0 {
        let bits = c.r16(moby + 0x34) | RENDER_BITS;
        c.w16(moby + 0x34, bits);
    }
    c.ret(moby);
    true
}
