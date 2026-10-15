//! Trail particle constructor: allocates a type-26 particle and fills it from its owner, a node index and a size.
use crate::actor;
use crate::h;
use crate::ps2::*;
use crate::talk_pad_scan;
use ee_oracle::cpu::Cpu;

/// Particle type passed to the allocator (0x259a38 mode 0).
const PARTICLE_TYPE: u32 = 0x1a;
/// Stack frame of the retail routine.
const FRAME_SIZE: u32 = 0x90;
/// Retail RNG draw (0x1160d8); its result is not used here, only the generator state advances.
const RAND_DRAW: u32 = 0x1160_d8;
/// Constant vector the node offset is taken from (0x166ec0).
const ANCHOR: u32 = 0x166_ec0;
/// Length the node offset is scaled to (0.4).
const OFFSET_LEN: F = F(0x3ecc_cccd);
/// Address of the pointer whose byte is copied to the particle's byte 2.
const TABLE_PTR: u32 = 0x1b_21e8;
/// High half of the table address, left in a0 by retail (`lui a0, 0x1b`).
const TABLE_HI: u32 = 0x1b_0000;
/// Byte 3 of the particle.
const BYTE3: u8 = 0x48;
/// Byte 9 of the particle: the float-to-int of 4.0 (4) plus 0x70.
const BYTE9: u8 = 4 + 0x70;
/// Node index value meaning "copy the owner's position instead of a pose node".
const NO_NODE: i32 = -1;

/// 0x25fd40 trail_spawn(owner, id, timer, node, size) -> particle or 0: allocates a trail particle that follows the
/// owner. The position is the owner's (`node` -1) or the pose node's point offset toward the anchor; `id` and `timer`
/// go into its header and `size` into its float at +0xc.
pub fn trail_spawn(c: &mut Cpu) -> bool {
    let owner = c.arg(0);
    let id = c.arg(1);
    let timer = c.arg(2);
    let node = c.arg(3);
    let size = c.farg(0);
    let saved_f20 = F(c.f[20] as u32);
    let frame = [
        (16, 0x20, h::SQ),
        (17, 0x30, h::SQ),
        (18, 0x40, h::SQ),
        (19, 0x50, h::SQ),
        (20, 0x60, h::SQ),
        (31, 0x70, h::SQ),
    ];
    h::frame(c, FRAME_SIZE, &frame, |c, sp| {
        c.wf(sp + 0x80, saved_f20);
        let p = h::run(c, talk_pad_scan::particle_alloc_mode0, &[PARTICLE_TYPE], &[]).0;
        if p != 0 {
            fill_particle(c, p, owner, id, timer, node, size, sp);
        }
        c.ret(p);
    });
    true
}

/// Writes the header of the particle `p` and its position.
fn fill_particle(c: &mut Cpu, p: u32, owner: u32, id: u32, timer: u32, node: u32, size: F, sp: u32) {
    c.w32(p + 0x20, owner);
    c.w16(p + 0x24, timer as u16);
    c.w16(p + 0x26, node as u16);
    c.w32(p + 0x28, timer);
    c.w32(p + 0x2c, ((id as i32) >> 24) as u32);
    c.w32(p + 4, id);
    let index = node as i16 as i32;
    if index == NO_NODE {
        let v = c.rv(owner + 0x10);
        c.wv(p + 0x10, v);
    } else {
        h::run(c, actor::pose_node_pos, &[owner, index as u32, sp], &[]);
        let offset = sp + 0x10;
        h::vec_sub(c, offset, ANCHOR, sp);
        h::vec_set_length(c, offset, offset, OFFSET_LEN);
        h::vec_add(c, p + 0x10, sp, offset);
    }
    // Retail stores the low byte of the draw in the delay slot of the float-to-int call: the draw's result.
    let (draw, _) = h::call_keep(c, RAND_DRAW, &[], &[]);
    c.w8(p + 8, draw as u8);
    c.w8(p + 9, BYTE9);
    c.w8(p + 3, BYTE3);
    c.wf(p + 0xc, size);
    c.w8(p + 1, 0);
    let table = c.r32(TABLE_PTR);
    let byte = c.r8(table);
    c.w8(p + 2, byte);
    c.s(4, TABLE_HI as u64);
    c.s(3, byte as u64);
}
