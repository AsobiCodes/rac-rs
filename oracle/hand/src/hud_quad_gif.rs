//! HUD quad packet emission and the eight-slot orbit ring update (retail 0x22e9a8, 0x230ab0, 0x230de8).
use crate::h;
use crate::ps2::*;
use crate::trig;
use crate::vecmath;
use ee_oracle::cpu::Cpu;

/// Quad parameter table (0x17e5d8): word at +0x20 is the entry base, +0x24 the record base, +0xc the sprite word.
const QUAD_TABLE: u32 = 0x0018_0000 - 0x1a28;
/// Corner table (0x13e500): words at +0x10 and +0x14 are the per-axis quad offsets.
const CORNER_TABLE: u32 = 0x0014_0000 - 0x1b00;
/// Packet write pointer global (0x161140).
const PACKET_PTR: u32 = 0x0016_1140;
/// Retail block emitter that appends the quad block (0x22e400).
const BLOCK_EMIT: u32 = 0x0022_e400;
/// Retail GS register write (0x2913c8).
const GS_REG_WRITE: u32 = 0x0029_13c8;
/// Mode global read by gs_mode_setup (0x15ed84) and level global (0x15fe88).
const MODE_GLOBAL: u32 = 0x0016_0000 - 0x127c;
const LEVEL_GLOBAL: u32 = 0x0016_0000 - 0x1178;
/// Orbit phase angle (f32 at 0x166ed8).
const PHASE_ADDR: u32 = 0x0016_6ed8;
/// Orbit output block (0x17ffc0): sin/cos pair and offsets written each update.
const ORBIT_OUT: u32 = 0x0018_0000 - 0x40;
/// Slot records (0x30 bytes each, kind word at -0x10) and their source positions (0x20 bytes each).
const SLOT_REC0: u32 = 0x0018_05e0;
const SLOT_SRC0: u32 = 0x0018_03d0;

const TAG_HEAD: u32 = 0x1000_0007;
const TAG_TAIL: u32 = 0x5000_0007;
const TAG_DATA: u64 = 0xb400_0000_0000_8001;
const TAG_REGS: u64 = (0xa6a6_a6a6u64 << 11) | 0x106;
/// Colour word (0x007f7f7f) OR-ed into the packed high bits of the first quad header.
const COLOUR_MASK: u64 = 0x007f_7f7f;
const SCALE_K: F = F(0x3f5d_b22d);
const NEAR_LIMIT: F = F(0x4100_0000);

/// Sign-extends a 32-bit MIPS result to 64 bits.
fn sx(x: u32) -> u64 {
    x as i32 as i64 as u64
}

/// Writes a 64-bit doubleword (sd).
fn put64(c: &mut Cpu, addr: u32, v: u64) {
    c.w32(addr, v as u32);
    c.w32(addr + 4, (v >> 32) as u32);
}

/// Writes a saved register as a quadword (sq), used for the frame that the retail prologue builds.
fn save_q(c: &mut Cpu, addr: u32, reg: usize) {
    let v = c.r[reg];
    for k in 0..4 {
        c.w32(addr + 4 * k, (v >> (32 * k)) as u32);
    }
}

/// 0x22e9a8 quad_packet_emit(idx, x, y, z, t0, t1) -> void: appends a texture tag pair and a quad block for table
/// entry `idx` at (x, y, z) to the packet buffer, then advances the packet write pointer.
pub fn quad_packet_emit(c: &mut Cpu) -> bool {
    let sp = (c.g(29) as u32).wrapping_sub(0xa0);
    let frame = [(0x00, 16), (0x10, 17), (0x20, 18), (0x30, 19), (0x40, 20), (0x50, 21), (0x60, 22), (0x70, 23), (0x80, 30), (0x90, 31)];
    for (off, reg) in frame {
        save_q(c, sp + off, reg);
    }
    let saved: Vec<(usize, u64)> = frame.iter().map(|&(_, reg)| (reg, c.r[reg] as u64)).collect();
    let idx = c.arg(0);
    let (a1, a2, a3) = (c.arg(1), c.arg(2), c.arg(3));
    let t0 = c.arg(4);
    let t1 = c.g(9);

    // Entry lookup: halfword at record+2 (scaled by 8) selects the 8-byte entry; its bytes 6 and 7 are shifts.
    let rec = idx.wrapping_shl(2).wrapping_add(c.r32(QUAD_TABLE + 0x20));
    let scaled = (c.r16(rec + 2) as i16 as i32) << 3;
    let entry = c.r32(QUAD_TABLE + 0x24).wrapping_add(scaled as u32);
    let mask_a = 1u32.wrapping_shl(c.r8(entry + 6) as u32);
    let mask_b = 1u32.wrapping_shl(c.r8(entry + 7) as u32);

    // Tag pair and the two pre-filled data quadwords at the packet cursor.
    let p = c.r32(PACKET_PTR);
    c.w32(p, TAG_HEAD);
    c.w32(p + 4, 0);
    c.w32(p + 8, 0);
    c.w32(p + 0xc, TAG_TAIL);
    let s2 = p + 0x10;
    c.w32(PACKET_PTR, s2);
    put64(c, s2, TAG_DATA);
    put64(c, s2 + 8, TAG_REGS);

    // The retail emitter reads the saved registers the prologue-side code has set up: s0..s7 and fp.
    c.s(16, t1);
    c.s(17, sx(a1));
    c.s(18, sx(s2));
    c.s(19, sx(t0));
    c.s(20, sx(mask_a));
    c.s(21, sx(a2));
    c.s(22, sx(QUAD_TABLE));
    c.s(23, sx(mask_b));
    c.s(30, sx(a3));

    // Retail block emitter; its full 64-bit return value (v0) is used below.
    // h::call would drop the upper word of v0, so call directly and keep the full 64-bit result.
    let entry_regs = c.r;
    for (i, a) in [idx, a1, a2, a3].into_iter().enumerate() {
        c.set_arg(i, a);
    }
    c.call(BLOCK_EMIT);
    let v0 = c.r[2] as u64;
    c.r = entry_regs;

    let s0 = (t1 << 24) | COLOUR_MASK;
    put64(c, s2 + 0x20, s0);
    put64(c, s2 + 0x28, sx(mask_a << 4));
    put64(c, s2 + 0x18, 0x154);
    put64(c, s2 + 0x10, v0);

    // Four quad vertex headers, each packing x/y/z offsets from the corner table and the sprite word.
    let ct14 = c.r32(CORNER_TABLE + 0x14);
    let ct10 = c.r32(CORNER_TABLE + 0x10);
    let sprite = c.r32(QUAD_TABLE + 0xc);
    let s5 = a2 << 4;
    let s3 = (a2.wrapping_add(t0)) << 4;
    let a1s = a1 << 4;
    let s7 = mask_b << 20;
    let s4 = (mask_a << 4).wrapping_add(s7);
    let s1 = (a1.wrapping_add(a3)) << 4;

    let v1_30 = s3.wrapping_add(ct14).wrapping_sub(8);
    let v0_30 = a1s.wrapping_add(ct10).wrapping_sub(8);
    put64(c, s2 + 0x38, sx(s4));
    put64(c, s2 + 0x30, sx(v0_30) | (sx(v1_30) << 16) | (sx(sprite) << 32));

    let v0_40 = s5.wrapping_add(ct14).wrapping_sub(8);
    let a1_40 = a1s.wrapping_add(ct10).wrapping_sub(8);
    put64(c, s2 + 0x48, 0);
    put64(c, s2 + 0x40, sx(a1_40) | (sx(v0_40) << 16) | (sx(sprite) << 32));
    let a1_final = sx(a1_40) | (sx(v0_40) << 16) | (sx(sprite) << 32);

    let s3_50 = s3.wrapping_add(ct14).wrapping_sub(8);
    let v0_50 = s1.wrapping_add(ct10).wrapping_sub(8);
    put64(c, s2 + 0x58, sx(s7));
    put64(c, s2 + 0x50, sx(v0_50) | (sx(s3_50) << 16) | (sx(sprite) << 32));

    let s5_60 = s5.wrapping_add(ct14).wrapping_sub(8);
    let s1_60 = s1.wrapping_add(ct10).wrapping_sub(8);
    put64(c, s2 + 0x68, 0);
    put64(c, s2 + 0x60, sx(s1_60) | (sx(s5_60) << 16) | (sx(sprite) << 32));

    // Cursor advances by 0x70 past the block start; register state matches the retail epilogue.
    let ptr = c.r32(PACKET_PTR).wrapping_add(0x70);
    c.w32(PACKET_PTR, ptr);
    c.s(1, 0x0016_0000);
    c.s(2, sx(ptr));
    c.s(3, sx(ct14));
    c.s(4, sx(ct10));
    c.s(5, a1_final);
    c.s(6, CORNER_TABLE as u64);
    // Epilogue restores the callee-saved registers from the frame.
    for (reg, v) in saved {
        c.s(reg, v);
    }
    true
}

/// 0x230ab0 gs_mode_setup() -> void: runs the two mode setup calls (the variant depends on the mode global), then
/// writes GS registers 0x47 and 0x4e, the second taking the level global shifted down by 13 bits.
pub fn gs_mode_setup(c: &mut Cpu) -> bool {
    let sp = (c.g(29) as u32).wrapping_sub(0x10);
    save_q(c, sp, 31);
    // Callees run with their scratch registers kept as a retail jal leaves them (v0/v1/a0/a1 are read later).
    h::call_keep(c, 0x0027_c2f8, &[], &[]);
    // lw v1, MODE_GLOBAL; addiu v0, zero, 4; bne v1, v0 -> the variant call.
    let mode = c.r32(MODE_GLOBAL);
    c.s(3, sx(mode));
    c.s(2, 4);
    if mode == 4 {
        h::call_keep(c, 0x0027_bca8, &[], &[]);
    } else {
        h::call_keep(c, 0x0027_bc48, &[], &[]);
    }
    h::call_keep(c, 0x0027_c388, &[], &[]);
    h::call_keep(c, GS_REG_WRITE, &[0x47, 0x0005_360b], &[]);
    // lui v0, 0x100; a1 = (level >> 13) | v0; a0 = 0x4e. v0 is the callee's return value afterwards.
    let level = ((c.r32(LEVEL_GLOBAL) as i32) >> 13) as u32 | 0x0100_0000;
    c.s(2, sx(0x0100_0000));
    h::call_keep(c, GS_REG_WRITE, &[0x4e, level], &[]);
    true
}

/// 0x230de8 slot_ring_update() -> void: recomputes the orbit offsets from the phase angle, then for each of the
/// eight slots with a nonzero kind whose source lies beyond 8.0 copies the source over and dispatches on kind.
pub fn slot_ring_update(c: &mut Cpu) -> bool {
    let sp = (c.g(29) as u32).wrapping_sub(0x90);
    for (off, reg) in [(0x00, 16), (0x10, 17), (0x20, 18), (0x30, 19), (0x40, 20), (0x50, 21), (0x60, 31)] {
        save_q(c, sp + off, reg);
    }
    c.w32(sp + 0x70, c.f[20]);
    c.w32(sp + 0x78, c.f[21]);
    c.w32(sp + 0x80, c.f[22]);

    // Fixed offsets, then the orbit angle from the phase and -0.8.
    c.wf(ORBIT_OUT + 0x34c, F(0xbe99_999a));
    c.wf(ORBIT_OUT + 0x348, F(0x3f4c_cccd));
    c.wf(ORBIT_OUT + 0x340, F(0x3f4c_cccd));
    c.wf(ORBIT_OUT + 0x344, F(0x3f4c_cccd));
    let phase = c.rf(PHASE_ADDR);
    let angle = h::run(c, vecmath::angle_add, &[], &[phase, F(0xbf4c_cccd)]).1;
    let cos = h::run(c, trig::fast_cos, &[], &[angle]).1;
    c.wf(ORBIT_OUT + 0x350, cos * SCALE_K);
    let sin = h::run(c, trig::fast_sin, &[], &[angle]).1;
    c.wf(ORBIT_OUT + 0x354, sin * SCALE_K);
    c.wf(ORBIT_OUT + 0x358, F(0xbf00_0000));
    c.w32(ORBIT_OUT + 0x35c, 0);

    // Eight slots; kind 1 is handed to 0x230f68 and becomes 2, kind 2 goes to 0x231318.
    for i in 0..8u32 {
        let rec = SLOT_REC0 + 0x30 * i;
        let src = SLOT_SRC0 + 0x20 * i;
        let kind = c.r32(rec - 0x10);
        if kind == 0 {
            continue;
        }
        let dist = h::dist3(c, src, rec);
        if !(NEAR_LIMIT < dist) {
            continue;
        }
        for k in 0..4 {
            let word = c.r32(src + 4 * k);
            c.w32(rec + 4 * k, word);
        }
        match kind {
            1 => {
                h::call(c, 0x0023_0f68, &[i], &[]);
                c.w32(rec - 0x10, 2);
            }
            2 => {
                h::call(c, 0x0023_1318, &[i], &[]);
            }
            _ => {}
        }
    }
    c.ret(0);
    true
}
