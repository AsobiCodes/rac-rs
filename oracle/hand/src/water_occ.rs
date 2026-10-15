//! Water-view fog swap and occlusion-grid helpers (Veldin/Novalis twins, retail addresses in each doc line).
use crate::mathx;
use crate::ps2::Ee;
use ee_oracle::cpu::Cpu;

const REG_AT: usize = 1;
const REG_V0: usize = 2;
const REG_V1: usize = 3;
const REG_A0: usize = 4;
const REG_A1: usize = 5;
const REG_A2: usize = 6;
const REG_A3: usize = 7;
const REG_T0: usize = 8;
const REG_T1: usize = 9;
const REG_GP: usize = 28;
const REG_SP: usize = 29;

/// Upper half of the 0x160000 data page (lui 0x16).
const PAGE: u32 = 0x0016_0000;
/// Camera water flag (page + 0x7114).
const WATER_FLAG: u32 = PAGE + 0x7114;
/// Fog block when the water flag is set (page + 0x1184..).
const FOG_WATER: u32 = PAGE + 0x1184;
/// Fog block when the water flag is clear (0x15f444..).
const FOG_DRY: u32 = 0x015F_444;
/// Clamp/far word written for the water case (page + 0xfc).
const CLAMP_SLOT: u32 = PAGE + 0xfc;
/// Value stored to the clamp slot (500/64 m).
const CLAMP_VALUE: u32 = 0x001F_4000;
/// Word cleared at the end of the swap (page - 0xba8).
const SWAP_DONE: u32 = PAGE - 0xba8;
/// Renderer state block the swap writes into (0x16cb40).
const VIEW: u32 = 0x016C_B40;
/// Retail routine the swap calls after writing the block.
const SWAP_CALL: u32 = 0x001E_A1E8;

/// Occlusion grid pointer (page - 0xa20).
const GRID_PTR: u32 = PAGE - 0xa20;
/// Occlusion mode word (0x16c174).
const OCC_MODE: u32 = 0x0016_C174;
/// Occlusion mask buffer cleared with -1 when the mode is 0 (0x173e00).
const OCC_MASK: u32 = 0x0017_3E00;
/// Retail routine run when the occlusion mode is 2.
const OCC_UPDATE: u32 = 0x001E_9C70;

fn put(c: &mut Cpu, reg: usize, v: u32) {
    c.s(reg, v as i32 as i64 as u64);
}

fn byte(c: &mut Cpu, addr: u32) -> u32 {
    c.r8(addr) as u32
}

/// Saves ra in the frame slot the retail prologue (`sq $ra, 0($sp)`) writes; the epilogue reloads it.
fn save_ra(c: &mut Cpu) -> u128 {
    let sp = (c.g(REG_SP) as u32).wrapping_sub(0x10);
    let ra = c.r[31];
    for lane in 0..4 {
        c.w32(sp.wrapping_add(4 * lane), (ra >> (32 * lane)) as u32);
    }
    ra
}

/// 0x1e99d8 water_view_swap(): end-of-frame fog/far-clip swap. With the camera water flag set it copies the fog block
/// at 0x161184 (floats at 0x161188..0x161194) into the renderer block and sets the clamp word; otherwise it copies
/// the dry block at 0x15f444 and sets the clamp word to 0x1f4000. Then runs 0x1ea1e8 and clears 0x15f458.
pub fn water_view_swap(c: &mut Cpu) -> bool {
    let ra = save_ra(c);
    let gp = c.g(REG_GP) as u32;
    if c.r32(WATER_FLAG) != 0 {
        let (v1, a1, a0) = (byte(c, FOG_WATER), byte(c, FOG_WATER + 1), byte(c, FOG_WATER + 2));
        let f1 = c.r32(PAGE + 0x1188);
        let f2 = c.r32(PAGE + 0x118c);
        let f3 = c.r32(PAGE + 0x1190);
        let f0 = c.r32(PAGE + 0x1194);
        c.w32(VIEW + 0x230, v1);
        c.w32(VIEW + 0x22c, f0);
        c.w32(VIEW + 0x234, a1);
        c.w32(VIEW + 0x238, a0);
        c.w32(VIEW + 0x218, f1);
        c.w32(VIEW + 0x21c, f2);
        c.w32(VIEW + 0x228, f3);
        c.w32(gp.wrapping_sub(0x6b04), 0x0004_0000);
        put(c, REG_V1, v1);
        put(c, REG_A0, a0);
        put(c, REG_A1, a1);
        put(c, REG_A2, 0x0004_0000);
        c.f[1] = f1;
        c.f[2] = f2;
        c.f[3] = f3;
        c.f[0] = f0;
    } else {
        let (a0, a2, a1) = (byte(c, FOG_DRY), byte(c, FOG_DRY + 1), byte(c, FOG_DRY + 2));
        let f3 = c.r32(FOG_DRY + 4);
        let f1 = c.r32(FOG_DRY + 8);
        let f2 = c.r32(FOG_DRY + 0xc);
        let f0 = c.r32(FOG_DRY + 0x10);
        c.w32(VIEW + 0x230, a0);
        c.w32(VIEW + 0x22c, f0);
        c.w32(VIEW + 0x234, a2);
        c.w32(VIEW + 0x238, a1);
        c.w32(VIEW + 0x218, f3);
        c.w32(VIEW + 0x21c, f1);
        c.w32(VIEW + 0x228, f2);
        c.w32(CLAMP_SLOT, CLAMP_VALUE);
        put(c, REG_V1, CLAMP_VALUE);
        put(c, REG_A0, a0);
        put(c, REG_A1, a1);
        put(c, REG_A2, a2);
        c.f[1] = f1;
        c.f[2] = f2;
        c.f[3] = f3;
        c.f[0] = f0;
    }
    put(c, REG_V0, VIEW);
    c.call(SWAP_CALL);
    put(c, REG_AT, PAGE);
    c.w32(SWAP_DONE, 0);
    c.r[31] = ra;
    true
}

/// 0x1e9ae0 occ_grid_cell(x, y, z) -> 128-byte visibility mask address or 0: three-level sparse lookup (x in a2,
/// y in a1, z in a0). Each level is {u16 min, u16 count, u16 table[count]}; 0 or 0xffff entries mean no mask.
pub fn occ_grid_cell(c: &mut Cpu) -> bool {
    let (mut a0, mut a1, mut a2) = (c.arg(0), c.arg(1), c.arg(2));
    let tbl = c.r32(GRID_PTR);
    let t1 = tbl.wrapping_add(c.r32(tbl));
    let mut v1 = c.r32(tbl);
    let mut a3 = tbl.wrapping_add(4);
    let mut v0 = c.r16(tbl.wrapping_add(4)) as u32;
    a2 = a2.wrapping_sub(v0);
    let mut mask = None;
    'find: {
        if (a2 as i32) < 0 {
            break 'find;
        }
        v0 = c.r16(a3.wrapping_add(2)) as u32;
        if !((a2 as i32) < (v0 as i32)) {
            break 'find;
        }
        v0 = (a2 << 1).wrapping_add(a3);
        v1 = c.r16(v0.wrapping_add(4)) as u32;
        if v1 == 0 {
            break 'find;
        }
        v0 = v1;
        a3 = tbl.wrapping_add(v0 << 2);
        v1 = c.r16(a3) as u32;
        a1 = a1.wrapping_sub(v1);
        if (a1 as i32) < 0 {
            break 'find;
        }
        v0 = c.r16(a3.wrapping_add(2)) as u32;
        if !((a1 as i32) < (v0 as i32)) {
            break 'find;
        }
        v0 = (a1 << 1).wrapping_add(a3);
        v1 = c.r16(v0.wrapping_add(4)) as u32;
        if v1 == 0 {
            break 'find;
        }
        v0 = v1;
        a3 = tbl.wrapping_add(v0 << 2);
        v1 = c.r16(a3) as u32;
        a0 = a0.wrapping_sub(v1);
        if (a0 as i32) < 0 {
            break 'find;
        }
        v0 = c.r16(a3.wrapping_add(2)) as u32;
        if !((a0 as i32) < (v0 as i32)) {
            break 'find;
        }
        v0 = (a0 << 1).wrapping_add(a3);
        a0 = 0xffff;
        v1 = c.r16(v0.wrapping_add(4)) as u32;
        if v1 == a0 {
            break 'find;
        }
        mask = Some(t1.wrapping_add(v1 << 7));
    }
    put(c, REG_T0, tbl);
    put(c, REG_T1, t1);
    put(c, REG_V1, v1);
    put(c, REG_A3, a3);
    put(c, REG_A0, a0);
    put(c, REG_A1, a1);
    put(c, REG_A2, a2);
    c.ret(mask.unwrap_or(0));
    true
}

/// 0x1ea060 occ_frame(): per-frame occlusion update. Mode 0 clears the mask buffer with -1 (fill_words); mode 2 runs
/// 0x1e9c70 (occ_update_mask); other modes leave the buffer alone (all visible).
pub fn occ_frame(c: &mut Cpu) -> bool {
    let ra = save_ra(c);
    let mode = c.r32(OCC_MODE);
    put(c, REG_V0, 0x0017_0000);
    put(c, REG_V1, mode);
    match mode {
        0 => {
            put(c, REG_A0, OCC_MASK);
            put(c, REG_A1, u32::MAX);
            put(c, REG_A2, 0x80);
            mathx::fill_words(c);
        }
        2 => {
            put(c, REG_V0, 2);
            put(c, REG_V1, 2);
            c.call(OCC_UPDATE);
        }
        other => {
            put(c, REG_V0, 2);
            put(c, REG_V1, other);
        }
    }
    c.r[31] = ra;
    true
}
