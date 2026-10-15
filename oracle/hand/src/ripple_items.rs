//! Hero ripple tick, the item-fraction table and the item-use gate. All three read the hero-state block at
//! `BLOCK` (retail `lui 0x14` base, `addiu -0xcb0`).
use crate::h;
use crate::ps2::*;
use crate::rand;
use ee_oracle::cpu::Cpu;

/// Hero-state block base (`lui v1, 0x14; addiu s0, v1, -0xcb0`).
const BLOCK: u32 = 0x14_0000 - 0xcb0;
/// Ripple countdown (halfword) in the block.
const RIPPLE_COUNTER: u32 = 0x904;
/// Ripple base position floats: x at +0x80, y at +0x84, z at +0x2f0.
const RIPPLE_X: u32 = 0x80;
const RIPPLE_Y: u32 = 0x84;
const RIPPLE_Z: u32 = 0x2f0;
/// Jitter range for x and y: -0.05 .. 0.05.
const JITTER_LO: F = F(0xbd4c_cccd);
const JITTER_HI: F = F(0x3d4c_cccd);
/// Jitter range for the third random draw: 0.1 .. 0.6.
const Z_LO: F = F(0x3ecc_cccd);
const Z_HI: F = F(0x3f19_999a);
/// Range passed to the retail fill helper (1312.0).
const RANGE: F = F(0x45a4_1000);
/// Retail helper that fills the ripple record: (record at sp, block+0x2f0, -1, range, random f12).
const RIPPLE_FILL: u32 = 0x2639c0;
/// Retail callee of the item-use gate (its v0 is the blocked flag).
const GATE_CALL: u32 = 0x227078;
/// Offsets of the item-fraction table inputs in the block.
const ITEM_CAP_STORED: u32 = 0x21ac;
const ITEM_COUNT: u32 = 0x21a8;
const ITEM_TABLE: u32 = 0x2128;
/// Offsets read by the item-use gate.
const GATE_BYTE_A: u32 = 0x20a4;
const GATE_MODE: u32 = 0x208c;
const GATE_BYTE_B: u32 = 0x12e4;
/// Global flag word the gate checks (`lui v0, 0x16; lw -0xa5c(v0)`).
const GATE_FLAG_WORD: u32 = 0x16_0000 - 0xa5c;

/// 0x1fc5e8 ripple_tick(owner, limit): advances the halfword countdown at block+0x904; when it passes `limit` it
/// draws a new countdown (0..owner-range) and fills a jittered ripple record on the stack for 0x2639c0.
pub fn ripple_tick(c: &mut Cpu) -> bool {
    let owner = c.arg(0);
    let limit = c.arg(1) as i32;
    let sp = (c.g(29) as u32).wrapping_sub(0x50);
    c.mem.wr128(sp + 0x10, c.r[16]);
    c.mem.wr128(sp + 0x20, c.r[17]);
    c.mem.wr128(sp + 0x30, c.r[31]);
    c.w32(sp + 0x40, c.f[20] as u32);
    c.w32(sp + 0x48, c.f[21] as u32);
    let counter = c.r16(BLOCK + RIPPLE_COUNTER) as u32 + 1;
    c.w16(BLOCK + RIPPLE_COUNTER, counter as u16);
    let v1 = counter as u16 as i16 as i32;
    c.ret(counter);
    c.s(3, v1 as i64 as u64);
    let fire = limit < v1;
    c.set_arg(1, fire as u32);
    if !fire {
        return true;
    }
    let (_, r1) = h::run_keep(c, rand::rand_float, &[], &[JITTER_LO, JITTER_HI]);
    let x = c.rf(BLOCK + RIPPLE_X) + r1;
    c.wf(sp, x);
    let (_, r2) = h::run_keep(c, rand::rand_float, &[], &[JITTER_LO, JITTER_HI]);
    let y = c.rf(BLOCK + RIPPLE_Y) + r2;
    c.wf(sp + 4, y);
    let z = c.rf(BLOCK + RIPPLE_Z);
    c.wf(sp + 8, z);
    let (_, r3) = h::run_keep(c, rand::rand_float, &[], &[Z_LO, Z_HI]);
    h::call_keep(c, RIPPLE_FILL, &[sp, BLOCK + RIPPLE_Z, u32::MAX], &[r3, RANGE]);
    let (n, _) = h::run_keep(c, rand::rand_below, &[owner], &[]);
    c.w16(BLOCK + RIPPLE_COUNTER, n as u16);
    true
}

/// 0x200700 item_frac(index) -> f0: clamps the index to 0..=31 and to the block word at +0x21ac, then reads the float
/// of the 32-entry table at block+0x2128 at the wrapped row (pure lookup, no writes).
pub fn item_frac(c: &mut Cpu) -> bool {
    let n = c.arg(0) as i32;
    let cap = if n < 0x20 { n } else { 0x1f };
    let stored = c.r32(BLOCK + ITEM_CAP_STORED) as i32;
    let cap = if stored < cap { stored } else { cap };
    let t = (c.r32(BLOCK + ITEM_COUNT) as i32).wrapping_sub(cap);
    let v1 = t.wrapping_add(0x1f);
    let pos = (v1 > -1) as i32;
    let q = if pos != 0 { v1 } else { t.wrapping_add(0x3e) };
    let qa = (q >> 5) << 5;
    let off = v1.wrapping_sub(qa);
    let addr = BLOCK.wrapping_add((off << 2) as u32);
    let f = c.rf(addr + ITEM_TABLE);
    c.set_arg(0, pos as u32);
    c.set_arg(1, cap as u32);
    c.set_arg(2, addr);
    c.set_arg(3, stored as u32);
    c.ret(qa as u32);
    c.s(3, (off << 2) as i64 as u64);
    c.fret(f);
    true
}

/// 0x202ed8 item_use_allowed() -> v0: 1 when item use is allowed: retail 0x227078 returns 0, the flag word at
/// 0x15f5a4 is 0, the byte at block+0x20a4 is 0, and the mode word at block+0x208c is 0..=2 or 4 with the byte at
/// block+0x12e4 zero.
pub fn item_use_allowed(c: &mut Cpu) -> bool {
    let sp = (c.g(29) as u32).wrapping_sub(0x10);
    c.mem.wr128(sp, c.r[31]);
    let (blocked, _) = h::call_keep(c, GATE_CALL, &[], &[]);
    if blocked != 0 || c.r32(GATE_FLAG_WORD) != 0 {
        c.ret(0);
        return true;
    }
    c.set_arg(0, BLOCK);
    let byte_a = c.r8(BLOCK + GATE_BYTE_A);
    if byte_a != 0 {
        c.s(3, byte_a as u64);
        c.ret(0);
        return true;
    }
    let mode = c.r32(BLOCK + GATE_MODE);
    c.s(3, mode as i32 as i64 as u64);
    let ok = if mode < 3 || mode == 4 {
        (c.r8(BLOCK + GATE_BYTE_B) == 0) as u32
    } else {
        0
    };
    c.ret(ok);
    true
}
