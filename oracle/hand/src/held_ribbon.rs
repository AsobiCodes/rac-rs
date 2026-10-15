//! Held-item ribbon spawn: allocates a type-0x37 particle, fills its header and joint offset from the owner, and
//! copies the owner's pose-node point into it.
use crate::actor;
use crate::h;
use crate::mathx;
use crate::ps2::*;
use crate::talk_pad_scan;
use ee_oracle::cpu::Cpu;

/// Particle type passed to the allocator (0x259a38 mode 0).
const PARTICLE_TYPE: u32 = 0x37;
/// Stack frame of the retail routine.
const FRAME_SIZE: u32 = 0x70;
/// Float 0.4 converted to int for the header byte 9 (retail `mtc1 0x3ecccccd`).
const OFFSET_SCALE: F = F(0x3ecc_cccd);
/// Offset added to the float-to-int result for byte 9.
const BYTE9_BASE: u32 = 0x20;
/// Value of particle byte 1.
const BYTE1: u8 = 3;
/// Value of particle byte 3.
const BYTE3: u8 = 0x48;
/// Address of the pointer whose byte is copied to particle byte 2 (`lw 0x225c` off 0x1b0000).
const TABLE_PTR: u32 = 0x1b_225c;
/// Offset of the owner's 16-bit field read into particle +0x38 (signed).
const OWNER_FIELD: u32 = 0xa6;

/// 0x2649b0 held_ribbon_spawn(owner, node, tag, id, f12, f13) -> particle or 0: allocates a type-0x37 joint ribbon
/// particle for the owner. Header bytes and `id` are stored, the pose-node point of `node` is copied into it and
/// the two float arguments go into its floats at +0x1c/+0x2c and +0x3c.
pub fn held_ribbon_spawn(c: &mut Cpu) -> bool {
    let owner = c.arg(0);
    let node = c.arg(1);
    let tag = c.arg(2);
    let id = c.arg(3);
    let f_a = c.farg(0);
    let f_b = c.farg(1);
    let saves = [
        (16, 0x00, h::SQ),
        (17, 0x10, h::SQ),
        (18, 0x20, h::SQ),
        (19, 0x30, h::SQ),
        (20, 0x40, h::SQ),
        (31, 0x50, h::SQ),
    ];
    h::frame(c, FRAME_SIZE, &saves, |c, sp| {
        c.wf(sp + 0x60, f_a);
        c.wf(sp + 0x68, f_b);
        let p = h::run_keep(c, talk_pad_scan::particle_alloc_mode0, &[PARTICLE_TYPE], &[]).0;
        if p != 0 {
            fill(c, p, Args { owner, node, tag, id, f_a, f_b });
        }
        c.ret(p);
    });
    true
}

/// The arguments of `held_ribbon_spawn`, as the fill step uses them.
struct Args {
    owner: u32,
    node: u32,
    tag: u32,
    id: u32,
    f_a: F,
    f_b: F,
}

/// Fills the header and the position of the new particle `p`.
fn fill(c: &mut Cpu, p: u32, a: Args) {
    c.w32(p + 4, a.tag);
    let out = p + 0x10;
    c.w32(p + 0xc, a.id);
    let (scaled, _) = h::run_keep(c, mathx::float_to_int, &[], &[OFFSET_SCALE]);
    c.w8(p + 9, (scaled + BYTE9_BASE) as u8);
    c.w8(p + 1, BYTE1);
    c.w8(p + 3, BYTE3);
    let table = c.r32(TABLE_PTR);
    let byte2 = c.r8(table);
    c.w8(p + 2, byte2);
    h::run_keep(c, actor::pose_node_pos, &[a.owner, a.node, out], &[]);
    let point = c.rv(out);
    c.wv(p + 0x20, point);
    c.wf(p + 0x2c, a.f_b);
    c.wf(p + 0x1c, a.f_a);
    c.w32(p + 0x30, a.owner);
    let field = c.r16(a.owner + OWNER_FIELD) as i16 as i32 as u32;
    c.wf(p + 0x3c, a.f_a);
    c.w32(p + 0x38, field);
    // Retail leaves the loaded halfword in v1.
    c.s(3, field as i32 as i64 as u64);
    c.w32(p + 0x34, a.node);
}
