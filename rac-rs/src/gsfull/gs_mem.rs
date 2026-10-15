//! GS local memory (4 MB): swizzled pixel addresses for PSMCT32/24/16(S), PSMT8, PSMT4 and the 8H/4HL/4HH
//! views, pixel read/write and colour expansion. Same tables and arithmetic as the gsfull prototype
//! (<dev>/scratch_gsfull/gsfull/src/gs.rs), which copied them from ee-oracle's src/gs.rs.
//! Note: PSMCT16S uses the PSMCT16 layout here (prototype semantics).

pub const VRAM_SIZE: usize = 4 << 20;

pub struct Luts {
    pub l32: Box<[u32; 2048]>,
    pub l16: Box<[u32; 4096]>,
    pub l8: Box<[u32; 8192]>,
    pub l4: Box<[u32; 16384]>,
}

pub fn luts() -> &'static Luts {
    static L: std::sync::OnceLock<Luts> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        fn t<const N: usize>(w: u32, f: fn(u32, u32, u32, u32) -> usize, bw: u32) -> Box<[u32; N]> {
            let v: Vec<u32> = (0..N as u32).map(|i| f(0, bw, i % w, i / w) as u32).collect();
            v.into_boxed_slice().try_into().unwrap()
        }
        Luts { l32: t(64, r32, 1), l16: t(64, r16, 1), l8: t(128, r8, 2), l4: t(128, r4, 2) }
    })
}

/// Byte address of a PSMCT32 pixel (bp in 256-byte blocks, bw in 64-pixel units).
#[inline(always)]
pub fn a32(l: &Luts, bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 32).wrapping_mul(bw).wrapping_add(x / 64);
    (bp.wrapping_mul(256).wrapping_add(page.wrapping_mul(8192)).wrapping_add(l.l32[(((y & 31) << 6) | (x & 63)) as usize]) & 0x3f_ffff) as usize
}
#[inline(always)]
pub fn a16(l: &Luts, bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 64).wrapping_mul(bw).wrapping_add(x / 64);
    (bp.wrapping_mul(256).wrapping_add(page.wrapping_mul(8192)).wrapping_add(l.l16[(((y & 63) << 6) | (x & 63)) as usize]) & 0x3f_ffff) as usize
}
#[inline(always)]
pub fn a8(l: &Luts, bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 64).wrapping_mul((bw / 2).max(1)).wrapping_add(x / 128);
    (bp.wrapping_mul(256).wrapping_add(page.wrapping_mul(8192)).wrapping_add(l.l8[(((y & 63) << 7) | (x & 127)) as usize]) & 0x3f_ffff) as usize
}
/// Nibble address of a PSMT4 pixel.
#[inline(always)]
pub fn a4(l: &Luts, bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 128).wrapping_mul((bw / 2).max(1)).wrapping_add(x / 128);
    (bp.wrapping_mul(512).wrapping_add(page.wrapping_mul(16384)).wrapping_add(l.l4[(((y & 127) << 7) | (x & 127)) as usize]) & 0x7f_ffff) as usize
}

/// Read one pixel. `m` must point at VRAM_SIZE bytes.
#[inline(always)]
pub unsafe fn rd_px(m: *const u8, l: &Luts, psm: u32, bp: u32, bw: u32, x: u32, y: u32) -> u32 {
    let r32 = |a: usize| (m.add(a) as *const u32).read_unaligned();
    match psm {
        0x00 => r32(a32(l, bp, bw, x, y)),
        0x01 => r32(a32(l, bp, bw, x, y)) & 0xff_ffff,
        0x02 | 0x0a => (m.add(a16(l, bp, bw, x, y)) as *const u16).read_unaligned() as u32,
        0x13 => *m.add(a8(l, bp, bw, x, y)) as u32,
        0x14 => {
            let n = a4(l, bp, bw, x, y);
            (*m.add(n >> 1) >> ((n & 1) * 4)) as u32 & 0xf
        }
        0x1b => *m.add(a32(l, bp, bw, x, y) + 3) as u32,
        0x24 => *m.add(a32(l, bp, bw, x, y) + 3) as u32 & 0xf,
        0x2c => (*m.add(a32(l, bp, bw, x, y) + 3) >> 4) as u32,
        _ => 0,
    }
}

/// Write one pixel. `m` must point at VRAM_SIZE bytes.
#[inline(always)]
pub unsafe fn wr_px(m: *mut u8, l: &Luts, psm: u32, bp: u32, bw: u32, x: u32, y: u32, v: u32) {
    match psm {
        0x00 => (m.add(a32(l, bp, bw, x, y)) as *mut u32).write_unaligned(v),
        0x01 => {
            let p = m.add(a32(l, bp, bw, x, y));
            *p = v as u8;
            *p.add(1) = (v >> 8) as u8;
            *p.add(2) = (v >> 16) as u8;
        }
        0x02 | 0x0a => (m.add(a16(l, bp, bw, x, y)) as *mut u16).write_unaligned(v as u16),
        0x13 => *m.add(a8(l, bp, bw, x, y)) = v as u8,
        0x14 => {
            let n = a4(l, bp, bw, x, y);
            let b = m.add(n >> 1);
            *b = if n & 1 != 0 { (*b & 0x0f) | ((v as u8) << 4) } else { (*b & 0xf0) | (v as u8 & 0xf) };
        }
        0x1b => *m.add(a32(l, bp, bw, x, y) + 3) = v as u8,
        0x24 => {
            let b = m.add(a32(l, bp, bw, x, y) + 3);
            *b = (*b & 0xf0) | (v as u8 & 0xf);
        }
        0x2c => {
            let b = m.add(a32(l, bp, bw, x, y) + 3);
            *b = (*b & 0x0f) | ((v as u8) << 4);
        }
        _ => {}
    }
}

pub fn bpp(psm: u32) -> u32 {
    match psm {
        0x00 => 32,
        0x01 => 24,
        0x02 | 0x0a => 16,
        0x13 | 0x1b => 8,
        _ => 4,
    }
}

/// Colour of a raw pixel value as RGBA8 (TEXA rules for 24- and 16-bit).
#[inline(always)]
pub fn unpack_col(psm: u32, v: u32, texa: u64) -> [u8; 4] {
    match psm {
        0x02 | 0x0a => {
            let c = |s: u32| (((v >> s) & 0x1f) << 3) as u8;
            let rgb0 = v & 0x7fff == 0;
            let a = if v & 0x8000 != 0 { (texa >> 32) & 0xff } else if texa & 0x8000 != 0 && rgb0 { 0 } else { texa & 0xff };
            [c(0), c(5), c(10), a as u8]
        }
        0x01 => {
            let a = if texa & 0x8000 != 0 && v & 0xff_ffff == 0 { 0 } else { texa & 0xff };
            [v as u8, (v >> 8) as u8, (v >> 16) as u8, a as u8]
        }
        _ => [v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8],
    }
}

/// Block range [lo, hi) (256-byte blocks) touched by a w x h rectangle at (x, y).
pub fn span(psm: u32, bp: u32, bw: u32, x: u32, y: u32, w: u32, h: u32) -> (u32, u32) {
    let l = luts();
    let (sw, sh): (u32, u32) = match psm {
        0x00 | 0x01 | 0x1b | 0x24 | 0x2c => (8, 8),
        0x02 | 0x0a => (16, 8),
        0x13 => (16, 16),
        0x14 => (32, 16),
        _ => return (0, u32::MAX),
    };
    let f = |xx: u32, yy: u32| -> usize {
        match psm {
            0x00 | 0x01 | 0x1b | 0x24 | 0x2c => a32(l, bp, bw, xx, yy),
            0x02 | 0x0a => a16(l, bp, bw, xx, yy),
            0x13 => a8(l, bp, bw, xx, yy),
            _ => a4(l, bp, bw, xx, yy) / 2,
        }
    };
    let (mut lo, mut hi) = (u32::MAX, 0);
    for xx in (x..x + w).step_by(sw as usize).chain(std::iter::once(x + w - 1)) {
        for yy in (y..y + h).step_by(sh as usize).chain(std::iter::once(y + h - 1)) {
            let b = (f(xx, yy) / 256) as u32;
            lo = lo.min(b);
            hi = hi.max(b + 1);
        }
    }
    (lo, hi)
}

/// Position inside a host->local transfer (TRXREG rectangle), in pixels.
#[derive(Default, Clone, Copy, Debug, PartialEq)]
pub struct Trx {
    pub x: u32,
    pub y: u32,
}

/// Pixels one qword of IMAGE data carries for a destination format (CT24 drops the last 8 bits).
fn px_per_qword(psm: u32) -> u64 {
    (128 / bpp(psm)) as u64
}

/// Write one qword of host->local image data (BITBLTBUF / TRXPOS / TRXREG) at transfer position `t`.
///
/// # Safety
/// `m` must point at VRAM_SIZE bytes nobody else accesses meanwhile.
pub unsafe fn put(m: *mut u8, bb: u64, pos: u64, rr: u64, t: &mut Trx, q: u128) {
    let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
    let (dx, dy) = (((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
    let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
    if rw == 0 {
        return;
    }
    let l = luts();
    if dpsm == 0 && t.x + 4 <= rw {
        if t.y >= rh {
            return;
        }
        for k in 0..4u32 {
            let a = a32(l, dbp, dbw, dx + t.x + k, dy + t.y);
            (m.add(a) as *mut u32).write_unaligned((q >> (32 * k)) as u32);
        }
        t.x += 4;
        if t.x >= rw {
            t.x = 0;
            t.y += 1;
        }
        return;
    }
    let bpp = bpp(dpsm);
    // Whole qword inside the current row: same writes as the pixel loop below, without its bookkeeping.
    let n = 128 / bpp;
    if matches!(dpsm, 0x13 | 0x14 | 0x02 | 0x0a) && t.y < rh && t.x + n <= rw {
        let (x0, y) = (dx + t.x, dy + t.y);
        match dpsm {
            0x13 => {
                for k in 0..16u32 {
                    *m.add(a8(l, dbp, dbw, x0 + k, y)) = (q >> (8 * k)) as u8;
                }
            }
            0x14 => {
                for k in 0..32u32 {
                    let v = (q >> (4 * k)) as u8 & 0xf;
                    let nb = a4(l, dbp, dbw, x0 + k, y);
                    let b = m.add(nb >> 1);
                    *b = if nb & 1 != 0 { (*b & 0x0f) | (v << 4) } else { (*b & 0xf0) | v };
                }
            }
            _ => {
                for k in 0..8u32 {
                    (m.add(a16(l, dbp, dbw, x0 + k, y)) as *mut u16).write_unaligned((q >> (16 * k)) as u16);
                }
            }
        }
        t.x += n;
        if t.x >= rw {
            t.x = 0;
            t.y += 1;
        }
        return;
    }
    let mut bits = q;
    let mut left = 128u32;
    while left >= bpp {
        if t.y >= rh {
            return;
        }
        let v = (bits & ((1u128 << bpp) - 1)) as u32;
        bits >>= bpp;
        left -= bpp;
        wr_px(m, l, dpsm, dbp, dbw, dx + t.x, dy + t.y, v);
        t.x += 1;
        if t.x >= rw {
            t.x = 0;
            t.y += 1;
        }
    }
}

/// `put` for every qword of `data` in order (same writes and the same final `t`), with the per-qword decoding
/// hoisted and the PSMCT32 case written as 64-bit pairs.
///
/// # Safety
/// As `put`.
pub unsafe fn put_run(m: *mut u8, bb: u64, pos: u64, rr: u64, t: &mut Trx, data: &[u128]) {
    let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
    let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
    if rw == 0 || !matches!(dpsm, 0 | 0x13) {
        for &q in data {
            put(m, bb, pos, rr, t, q);
        }
        return;
    }
    if dpsm == 0x13 {
        // 16 pixels per qword; when they share a 128-pixel page column, the page part of the address is common.
        let (dx, dy) = (((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
        let l = luts();
        let base = dbp.wrapping_mul(256);
        let pw = (dbw / 2).max(1);
        for &q in data {
            if t.y >= rh {
                return;
            }
            let (x0, y) = (dx + t.x, dy + t.y);
            if t.x + 16 <= rw && (x0 & 127) <= 112 {
                let page = (y / 64).wrapping_mul(pw).wrapping_add(x0 / 128);
                let b = base.wrapping_add(page.wrapping_mul(8192));
                let row = ((y & 63) << 7) | (x0 & 127);
                for k in 0..16u32 {
                    let a = (b.wrapping_add(l.l8[(row + k) as usize]) & 0x3f_ffff) as usize;
                    *m.add(a) = (q >> (8 * k)) as u8;
                }
                t.x += 16;
                if t.x >= rw {
                    t.x = 0;
                    t.y += 1;
                }
            } else {
                put(m, bb, pos, rr, t, q);
            }
        }
        return;
    }
    let (dx, dy) = (((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
    let l = luts();
    let base = dbp.wrapping_mul(256);
    for &q in data {
        if t.y >= rh {
            // `put` returns without writing or advancing for every further qword.
            return;
        }
        if t.x + 4 <= rw {
            let (x, y) = (dx + t.x, dy + t.y);
            if x & 1 == 0 && (x & 63) <= 60 {
                // Pixels x..x+3 share a page; neighbouring even/odd pixels are neighbouring words.
                let page = (y / 32).wrapping_mul(dbw).wrapping_add(x / 64);
                let b = base.wrapping_add(page.wrapping_mul(8192));
                let row = (y & 31) << 6;
                let a0 = (b.wrapping_add(l.l32[(row | (x & 63)) as usize]) & 0x3f_ffff) as usize;
                let a1 = (b.wrapping_add(l.l32[(row | ((x & 63) + 2)) as usize]) & 0x3f_ffff) as usize;
                (m.add(a0) as *mut u64).write_unaligned(q as u64);
                (m.add(a1) as *mut u64).write_unaligned((q >> 64) as u64);
            } else {
                for k in 0..4u32 {
                    let a = a32(l, dbp, dbw, dx + t.x + k, dy + t.y);
                    (m.add(a) as *mut u32).write_unaligned((q >> (32 * k)) as u32);
                }
            }
            t.x += 4;
            if t.x >= rw {
                t.x = 0;
                t.y += 1;
            }
        } else {
            put(m, bb, pos, rr, t, q);
        }
    }
}

/// Advance a transfer position over `nq` qwords exactly as `put` does, without writing.
pub fn trx_advance(bb: u64, rr: u64, t: &mut Trx, nq: usize) {
    let dpsm = ((bb >> 56) & 0x3f) as u32;
    let (rw, rh) = ((rr & 0xfff) as u64, ((rr >> 32) & 0xfff) as u64);
    if rw == 0 {
        return;
    }
    let total = rw * rh;
    let idx = t.y as u64 * rw + t.x as u64;
    if idx >= total {
        return;
    }
    let idx = (idx + px_per_qword(dpsm) * nq as u64).min(total);
    t.x = (idx % rw) as u32;
    t.y = (idx / rw) as u32;
}

/// Local->local transfer (TRXDIR 2).
///
/// # Safety
/// As `put`.
pub unsafe fn local_copy(m: *mut u8, bb: u64, pos: u64, rr: u64) {
    let (sbp, sbw, spsm) = ((bb & 0x3fff) as u32, ((bb >> 16) & 0x3f) as u32, ((bb >> 24) & 0x3f) as u32);
    let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
    let (sx, sy, dx, dy) = ((pos & 0x7ff) as u32, ((pos >> 16) & 0x7ff) as u32, ((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
    let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
    let l = luts();
    for y in 0..rh {
        for x in 0..rw {
            let v = rd_px(m, l, spsm, sbp, sbw, sx + x, sy + y);
            wr_px(m, l, dpsm, dbp, dbw, dx + x, dy + y, v);
        }
    }
}

/// Byte offsets (from CBP * 256) of the entry pairs (2k, 2k + 1) of a CSM1 PSMCT32 256-entry CLUT.
fn clut_pairs() -> &'static [u16; 128] {
    static T: std::sync::OnceLock<[u16; 128]> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let l = luts();
        let mut t = [0u16; 128];
        for (k, o) in t.iter_mut().enumerate() {
            let pos = |i: u32| {
                let p = (i & 0xe7) | ((i & 8) << 1) | ((i & 0x10) >> 1);
                a32(l, 0, 1, p % 16, p / 16)
            };
            let (a, b) = (pos(2 * k as u32), pos(2 * k as u32 + 1));
            assert!(b == a + 4 && a % 8 == 0 && a < 1024, "CLUT pair layout");
            *o = a as u16;
        }
        t
    })
}

/// CLUT load: 16 entries at `off` (4-bit textures) or all 256 (8-bit), from CBP in format `cfmt`.
///
/// # Safety
/// `m` must point at VRAM_SIZE bytes nobody writes meanwhile.
pub unsafe fn clut_read(m: *const u8, clut: &mut [[u8; 4]; 512], cbp: u32, cfmt: u32, t4: bool, off: usize, texa: u64) {
    let l = luts();
    if !t4 && cfmt == 0 {
        // PSMCT32 entries are the stored words (`unpack_col` is the identity byte split for it). Entries 2k and
        // 2k+1 are neighbouring words (8-byte aligned) in the 1 KB block run, so they move as one u64.
        let pairs = clut_pairs();
        let base = cbp.wrapping_mul(256) as usize;
        let dst = clut.as_mut_ptr() as *mut u8;
        for (k, &o) in pairs.iter().enumerate() {
            let a = (base.wrapping_add(o as usize)) & 0x3f_ffff;
            (dst.add(k * 8) as *mut u64).write_unaligned((m.add(a) as *const u64).read_unaligned());
        }
        return;
    }
    if t4 {
        for i in 0..16u32 {
            let raw = rd_px(m, l, cfmt, cbp, 1, i % 8, i / 8);
            clut[off + i as usize] = unpack_col(cfmt, raw, texa);
        }
    } else {
        for i in 0..256u32 {
            let p = (i & 0xe7) | ((i & 8) << 1) | ((i & 0x10) >> 1);
            let raw = rd_px(m, l, cfmt, cbp, 1, p % 16, p / 16);
            clut[i as usize] = unpack_col(cfmt, raw, texa);
        }
    }
}

fn r32(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 32) * bw + x / 64;
    let blk = BT32[((y % 32) / 8) as usize][((x % 64) / 8) as usize] as u32;
    ((((bp + page * 32 + blk) * 64 + CT32[(y % 8) as usize][(x % 8) as usize] as u32) * 4) & 0x3f_ffff) as usize
}
fn r16(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 64) * bw + x / 64;
    let blk = BT16[((y % 64) / 8) as usize][((x % 64) / 16) as usize] as u32;
    let hw = CT32[(y % 8) as usize][(x % 8) as usize] as u32 * 2 + ((x >> 3) & 1);
    ((((bp + page * 32 + blk) * 128 + hw) * 2) & 0x3f_ffff) as usize
}
fn r8(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 64) * (bw / 2).max(1) + x / 128;
    let blk = BT32[((y % 64) / 16) as usize][((x % 128) / 16) as usize] as u32;
    let (xx, yy) = (x & 15, y & 15);
    let sw = (((yy + 2) >> 2) & 1) * 4;
    let py = (((yy & !3) >> 1) + (yy & 1)) & 7;
    let w = CT32[py as usize][((xx + sw) & 7) as usize] as u32;
    ((((bp + page * 32 + blk) * 64 + w) * 4 + ((xx >> 3) & 1) * 2 + ((yy >> 1) & 1)) & 0x3f_ffff) as usize
}
fn r4(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 128) * (bw / 2).max(1) + x / 128;
    let blk = BT16[((y % 128) / 16) as usize][((x % 128) / 32) as usize] as u32;
    let (xx, yy) = (x & 31, y & 15);
    let sw = (((yy + 2) >> 2) & 1) * 4;
    let py = (((yy & !3) >> 1) + (yy & 1)) & 7;
    let w = CT32[py as usize][((xx + sw) & 7) as usize] as u32;
    ((((bp + page * 32 + blk) * 64 + w) * 8 + ((xx >> 3) & 3) * 2 + ((yy >> 1) & 1)) & 0x7f_ffff) as usize
}

const BT32: [[u8; 8]; 4] = [
    [0, 1, 4, 5, 16, 17, 20, 21],
    [2, 3, 6, 7, 18, 19, 22, 23],
    [8, 9, 12, 13, 24, 25, 28, 29],
    [10, 11, 14, 15, 26, 27, 30, 31],
];
const BT16: [[u8; 4]; 8] = [
    [0, 2, 8, 10],
    [1, 3, 9, 11],
    [4, 6, 12, 14],
    [5, 7, 13, 15],
    [16, 18, 24, 26],
    [17, 19, 25, 27],
    [20, 22, 28, 30],
    [21, 23, 29, 31],
];
const CT32: [[u8; 8]; 8] = [
    [0, 1, 4, 5, 8, 9, 12, 13],
    [2, 3, 6, 7, 10, 11, 14, 15],
    [16, 17, 20, 21, 24, 25, 28, 29],
    [18, 19, 22, 23, 26, 27, 30, 31],
    [32, 33, 36, 37, 40, 41, 44, 45],
    [34, 35, 38, 39, 42, 43, 46, 47],
    [48, 49, 52, 53, 56, 57, 60, 61],
    [50, 51, 54, 55, 58, 59, 62, 63],
];
