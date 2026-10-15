//! Pixel pipeline of the GS-full software GS: triangles, sprites, lines and points into one screen
//! rectangle (a tile, or the whole screen for serial prims).
//!
//! Every pixel is computed with the same arithmetic as the gsfull prototype renderer
//! (<dev>/scratch_gsfull/gsfull/src/gs.rs): attributes are evaluated per pixel from their plane
//! equations (no incremental stepping), so the result does not depend on how the screen is split into
//! tiles. Z is tested first ("early Z"): in the prototype nothing before the z test has a side effect, so
//! testing it before evaluating colours and texturing is exact. The frame buffer pixel is only read when
//! the result depends on it (blending, DATE, FBMSK, alpha-test RGB_ONLY). Ordering between prims is the job
//! of `gs_queue`.
use super::gs::Vtx;
use super::gs_mem::{a32, a8, rd_px, unpack_col, wr_px, Luts};

/// Tile size (pixels) of the parallel rasteriser; tiles are aligned to whole blocks of every format.
pub const TS: i32 = 16;

/// Everything a prim needs from the GS registers, decoded once per register change (`Ctx::derive` fills
/// the decoded copies of TEX0 / CLAMP / TEST / ALPHA / FOGCOL fields).
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub struct Ctx {
    pub at: u64,
    pub tme: bool,
    pub fst: bool,
    pub iip: bool,
    pub fge: bool,
    pub abe: bool,
    pub bilinear: bool,
    pub tex0: u64,
    pub clamp: u64,
    pub alpha: u64,
    pub test: u64,
    pub tbp: u32,
    pub tbw: u32,
    pub tpsm: u32,
    pub tw: u32,
    pub th: u32,
    pub fpsm: u32,
    pub fbp: u32,
    pub fbw: u32,
    pub fbmsk: u32,
    pub fba: bool,
    pub zte: bool,
    pub ztst: u32,
    pub zwrite: bool,
    pub zmask: u32,
    pub pabe: bool,
    pub colclamp: bool,
    pub fogcol: u64,
    pub texa: u64,
    pub ofx: i32,
    pub ofy: i32,
    pub sc: [i32; 4],
    pub clut_off: u32,
    /// CLUT version for paletted textures (0 otherwise); mapped to a snapshot when queued.
    pub clut_ver: u64,
    pub clut: u32,
    pub zplane: u32,
    /// TEX1, MIPTBP1, MIPTBP2 of the prim's context (GS mip level choice, `mip_level`).
    pub tex1: u64,
    pub mip1: u64,
    pub mip2: u64,
    // Decoded by `derive`.
    pub(super) wms: u8,
    pub(super) wmt: u8,
    pub(super) minu: u32,
    pub(super) maxu: u32,
    pub(super) minv: u32,
    pub(super) maxv: u32,
    pub(super) tfx: u8,
    pub(super) tcc: bool,
    pub(super) ate: bool,
    pub(super) atst: u8,
    pub(super) aref: i32,
    pub(super) afail: u8,
    pub(super) date: bool,
    pub(super) datm: u32,
    pub(super) ba: u8,
    pub(super) bb: u8,
    pub(super) bc: u8,
    pub(super) bd: u8,
    pub(super) fix: i32,
    pub(super) fogc: [i32; 3],
    pub(super) need_dst: bool,
    /// The texel depends on LOD: MXL > 0 with a mipmap MMIN, or MMIN's filter differs from MMAG's.
    pub(super) lodsel: bool,
}

impl Ctx {
    pub fn paletted(&self) -> bool {
        self.tme && matches!(self.tpsm, 0x13 | 0x1b | 0x14 | 0x24 | 0x2c)
    }

    /// Fill the decoded fields from the register copies.
    pub fn derive(&mut self) {
        let cl = self.clamp;
        self.wms = (cl & 3) as u8;
        self.wmt = ((cl >> 2) & 3) as u8;
        self.minu = ((cl >> 4) & 0x3ff) as u32;
        self.maxu = ((cl >> 14) & 0x3ff) as u32;
        self.minv = ((cl >> 24) & 0x3ff) as u32;
        self.maxv = ((cl >> 34) & 0x3ff) as u32;
        self.tfx = ((self.tex0 >> 35) & 3) as u8;
        self.tcc = self.tex0 & (1 << 34) != 0;
        let t = self.test;
        self.ate = t & 1 != 0;
        self.atst = ((t >> 1) & 7) as u8;
        self.aref = ((t >> 4) & 0xff) as i32;
        self.afail = ((t >> 12) & 3) as u8;
        self.date = t & 0x4000 != 0;
        self.datm = ((t >> 15) & 1) as u32;
        let al = self.alpha;
        self.ba = (al & 3) as u8;
        self.bb = ((al >> 2) & 3) as u8;
        self.bc = ((al >> 4) & 3) as u8;
        self.bd = ((al >> 6) & 3) as u8;
        self.fix = ((al >> 32) & 0xff) as i32;
        self.fogc = [(self.fogcol & 0xff) as i32, ((self.fogcol >> 8) & 0xff) as i32, ((self.fogcol >> 16) & 0xff) as i32];
        self.need_dst = self.abe || self.date || self.fbmsk != 0 || (self.ate && self.afail == 3);
        let mmin = ((self.tex1 >> 6) & 7) as u8;
        let mxl = ((self.tex1 >> 2) & 7) as u8;
        self.lodsel = self.tme && ((mxl > 0 && (2..=5).contains(&mmin)) || matches!(mmin, 1 | 4 | 5) != self.bilinear);
    }

    /// Base pointer and width of mip level `n` (1..=6): MIPTBP1 / MIPTBP2, or with TEX1 MTBA the levels packed after
    /// level 0 (each level's block size, width halved).
    pub fn level_tbp(&self, n: u32) -> (u32, u32) {
        if self.tex1 & 0x200 != 0 {
            let bpp = match self.tpsm {
                0x00 | 0x1b | 0x24 | 0x2c => 32,
                0x01 => 24,
                0x02 | 0x0a => 16,
                0x13 => 8,
                _ => 4,
            };
            let (mut bp, mut bw) = (self.tbp, self.tbw);
            for k in 0..n {
                let (w, h) = ((self.tw >> k).max(1), (self.th >> k).max(1));
                bp += (w * h * bpp / 8).div_ceil(256);
                bw = (bw / 2).max(1);
            }
            (bp, bw)
        } else {
            let (m, k) = if n <= 3 { (self.mip1, n - 1) } else { (self.mip2, n - 4) };
            let w = m >> (20 * k);
            ((w & 0x3fff) as u32, ((w >> 14) & 0x3f) as u32)
        }
    }

    /// GS level choice: LOD = (log2(1 / |Q|) << L) + K (LCM 0) or K (LCM 1); LOD <= 0 magnifies (level 0, MMAG filter),
    /// else MMIN: *_MIPMAP_NEAREST rounds LOD, *_MIPMAP_LINEAR blends floor(LOD) and the next level by its fraction (8 bits);
    /// levels capped at MXL. Returns (level, next level, fraction 0..=255, bilinear).
    #[inline(always)]
    pub fn mip_level(&self, q: f32) -> (u32, u32, u32, bool) {
        let t1 = self.tex1;
        let k = (((t1 >> 32) & 0xfff) as i32) << 20 >> 20;
        let k = k as f32 / 16.0;
        let lod = if t1 & 1 != 0 { k } else { -q.abs().log2() * (1u32 << ((t1 >> 19) & 3)) as f32 + k };
        if !(lod > 0.0) && !lod.is_nan() {
            return (0, 0, 0, self.bilinear);
        }
        let mmin = ((t1 >> 6) & 7) as u32;
        let mxl = ((t1 >> 2) & 7) as u32;
        let lin = matches!(mmin, 1 | 4 | 5);
        let lod = if lod.is_nan() { 0.0 } else { lod.min(16.0) };
        match mmin {
            2 | 4 => (((lod + 0.5) as u32).min(mxl), 0, 0, lin),
            3 | 5 => {
                let l0 = (lod as u32).min(mxl);
                let fr = if l0 >= mxl { 0 } else { ((lod - lod.floor()) * 256.0) as u32 };
                (l0, (l0 + 1).min(mxl), fr.min(255), lin)
            }
            _ => (0, 0, 0, lin),
        }
    }
}

#[derive(Clone, Copy)]
pub struct Prim {
    pub v: [Vtx; 3],
    pub ctx: u32,
    pub ty: u8,
    /// Conservative pixel bounding box x0, y0, x1, y1 (inclusive, inside 0..1023).
    pub bb: [i16; 4],
}

#[derive(Default, Clone, Copy, Debug)]
pub struct Acc {
    pub cover: u64,
    pub zfail: u64,
    pub written: u64,
}

impl Acc {
    pub fn add(&mut self, o: &Acc) {
        self.cover += o.cover;
        self.zfail += o.zfail;
        self.written += o.written;
    }
}

#[derive(Clone, Copy)]
pub struct Mem {
    pub vram: *mut u8,
    pub zp: *const *mut u32,
}
unsafe impl Send for Mem {}
unsafe impl Sync for Mem {}

/// Draw the part of prim `p` that falls into `rect` (x0, y0, x1, y1 inclusive, inside 0..1023).
///
/// # Safety
/// `m` must point at the 4 MB VRAM and the z plane table; no other thread may touch the memory this
/// prim writes inside `rect` or the texture it reads.
pub unsafe fn raster_prim(p: &Prim, cx: &Ctx, clut: &[[u8; 4]; 512], m: &Mem, l: &Luts, rect: [i32; 4], acc: &mut Acc) {
    let zp = *m.zp.add(cx.zplane as usize);
    let d = Dst { cx, clut, l, vram: m.vram, zp };
    match p.ty {
        6 => sprite(&d, p.v[0], p.v[1], rect, acc),
        3..=5 => tri(&d, &p.v, rect, acc),
        1 | 2 => line(&d, p.v[0], p.v[1], rect, acc),
        0 => {
            let v = &p.v[0];
            let x = ((v.x as i32 - cx.ofx) + 8) >> 4;
            let y = ((v.y as i32 - cx.ofy) + 8) >> 4;
            if x >= rect[0] && x <= rect[2] && y >= rect[1] && y <= rect[3] && x >= 0 && y >= 0 && x < 1024 && y < 1024 {
                let (u, vv) = uv_of(cx, v);
                pixel(&d, x, y, v.z, v.rgba.map(|c| c as f32), u, vv, v.f as f32, acc);
            }
        }
        _ => {}
    }
}

struct Dst<'a> {
    cx: &'a Ctx,
    clut: &'a [[u8; 4]; 512],
    l: &'a Luts,
    vram: *mut u8,
    zp: *mut u32,
}

/// Texture coordinates of one vertex in texels (prototype `Px::of`).
#[inline(always)]
fn uv_of(cx: &Ctx, v: &Vtx) -> (f32, f32) {
    if cx.fst {
        (v.u as f32 / 16.0, v.v as f32 / 16.0)
    } else {
        let q = if v.q != 0.0 { v.q } else { 1.0 };
        (v.s / q * cx.tw as f32, v.t / q * cx.th as f32)
    }
}

#[inline(always)]
unsafe fn sprite(d: &Dst, a: Vtx, b: Vtx, rect: [i32; 4], acc: &mut Acc) {
    let cx = d.cx;
    let (ax, ay) = (a.x as i32 - cx.ofx, a.y as i32 - cx.ofy);
    let (bx, by) = (b.x as i32 - cx.ofx, b.y as i32 - cx.ofy);
    let (lx, hx) = (ax.min(bx), ax.max(bx));
    let (ly, hy) = (ay.min(by), ay.max(by));
    let ix0 = ((lx + 15) >> 4).max(cx.sc[0]).max(rect[0]).max(0);
    let ix1 = (((hx + 15) >> 4) - 1).min(cx.sc[1]).min(rect[2]).min(1023);
    let iy0 = ((ly + 15) >> 4).max(cx.sc[2]).max(rect[1]).max(0);
    let iy1 = (((hy + 15) >> 4) - 1).min(cx.sc[3]).min(rect[3]).min(1023);
    if ix0 > ix1 || iy0 > iy1 {
        return;
    }
    let (ua, va) = uv_of(cx, &a);
    let (ub, vb) = uv_of(cx, &b);
    let (fx0, fx1) = (ax as f32 / 16.0, bx as f32 / 16.0);
    let (fy0, fy1) = (ay as f32 / 16.0, by as f32 / 16.0);
    let rgba = [b.rgba[0] as f32, b.rgba[1] as f32, b.rgba[2] as f32, b.rgba[3] as f32];
    let f = b.f as f32;
    let z = b.z;
    let mut us = [0f32; TS as usize];
    let short = (ix1 - ix0 + 1) as usize <= us.len();
    if short {
        for x in ix0..=ix1 {
            let tx = if fx1 != fx0 { (x as f32 - fx0) / (fx1 - fx0) } else { 0.0 };
            us[(x - ix0) as usize] = ua + (ub - ua) * tx;
        }
    }
    for y in iy0..=iy1 {
        let ty = if fy1 != fy0 { (y as f32 - fy0) / (fy1 - fy0) } else { 0.0 };
        let v = va + (vb - va) * ty;
        for x in ix0..=ix1 {
            acc.cover += 1;
            let pos = (y as usize) * 1024 + x as usize;
            if !zpass(d, pos, z) {
                acc.zfail += 1;
                continue;
            }
            let u = if short {
                us[(x - ix0) as usize]
            } else {
                let tx = if fx1 != fx0 { (x as f32 - fx0) / (fx1 - fx0) } else { 0.0 };
                ua + (ub - ua) * tx
            };
            shade(d, x as u32, y as u32, pos, z, rgba, u, v, f, 1.0, acc);
        }
    }
}

/// If sprite `p` is an in-place pass over the frame buffer (texel (x, y) is frame pixel (x, y)) whose pixel
/// (x, y) reads only texels in {x-1, x} x {y-1, y, y+1}, return its pixel rectangle (x0, y0, x1, y1). Such
/// passes can run as row pipelines (`gs_queue::Exec::pipe`): a row may follow the row above it with a lag of
/// two pixels and every pixel sees the values the serial raster order gives it.
pub fn sprite_inplace(cx: &Ctx, p: &Prim) -> Option<[i32; 4]> {
    if p.ty != 6 || !cx.tme || !cx.fst || cx.fpsm != 0 || cx.tpsm != 0 || cx.tbp != cx.fbp || cx.tbw != cx.fbw || cx.wms != 1 || cx.wmt != 1 || cx.fbw == 0 {
        return None;
    }
    let (a, b) = (&p.v[0], &p.v[1]);
    let (ax, ay) = (a.x as i32 - cx.ofx, a.y as i32 - cx.ofy);
    let (bx, by) = (b.x as i32 - cx.ofx, b.y as i32 - cx.ofy);
    let (lx, hx) = (ax.min(bx), ax.max(bx));
    let (ly, hy) = (ay.min(by), ay.max(by));
    let ix0 = ((lx + 15) >> 4).max(cx.sc[0]).max(0);
    let ix1 = (((hx + 15) >> 4) - 1).min(cx.sc[1]).min(1023);
    let iy0 = ((ly + 15) >> 4).max(cx.sc[2]).max(0);
    let iy1 = (((hy + 15) >> 4) - 1).min(cx.sc[3]).min(1023);
    if ix0 > ix1 || iy0 > iy1 {
        return None;
    }
    let (ua, va) = uv_of(cx, a);
    let (ub, vb) = uv_of(cx, b);
    let (fx0, fx1) = (ax as f32 / 16.0, bx as f32 / 16.0);
    let (fy0, fy1) = (ay as f32 / 16.0, by as f32 / 16.0);
    // The texels read for one coordinate (two for bilinear, one for nearest, clamped) must lie in
    // [pos - 1, pos + fwd].
    let ok = |c: f32, pos: i32, size: u32, fwd: i32| -> bool {
        let (t0, t1) = if cx.bilinear {
            let x0 = ffloor(c - 0.5) as i32;
            (clampi(x0, 0, size as i32 - 1), clampi(x0 + 1, 0, size as i32 - 1))
        } else {
            let t = clampi(ffloor(c) as i32, 0, size as i32 - 1);
            (t, t)
        };
        t0 >= pos - 1 && t0 <= pos + fwd && t1 >= pos - 1 && t1 <= pos + fwd
    };
    for x in ix0..=ix1 {
        let tx = if fx1 != fx0 { (x as f32 - fx0) / (fx1 - fx0) } else { 0.0 };
        if !ok(ua + (ub - ua) * tx, x, cx.tw, 0) {
            return None;
        }
    }
    for y in iy0..=iy1 {
        let ty = if fy1 != fy0 { (y as f32 - fy0) / (fy1 - fy0) } else { 0.0 };
        if !ok(va + (vb - va) * ty, y, cx.th, 1) {
            return None;
        }
    }
    Some([ix0, iy0, ix1, iy1])
}

#[inline(always)]
fn plane(vv: &[Vtx; 3], ea: &[i64; 3], eb: &[i64; 3], ec: &[i64; 3], inv: f64, f: impl Fn(&Vtx) -> f64) -> (f64, f64, f64) {
    let (a0, a1, a2) = (f(&vv[0]), f(&vv[1]), f(&vv[2]));
    let gx = (a0 * ea[0] as f64 + a1 * ea[1] as f64 + a2 * ea[2] as f64) * inv * 16.0;
    let gy = (a0 * eb[0] as f64 + a1 * eb[1] as f64 + a2 * eb[2] as f64) * inv * 16.0;
    let c0 = (a0 * ec[0] as f64 + a1 * ec[1] as f64 + a2 * ec[2] as f64) * inv;
    (c0, gx, gy)
}

#[inline(always)]
unsafe fn tri(d: &Dst, v: &[Vtx; 3], rect: [i32; 4], acc: &mut Acc) {
    let cx = d.cx;
    let mut vv = *v;
    let pt = |t: &Vtx| ((t.x as i64) - cx.ofx as i64, (t.y as i64) - cx.ofy as i64);
    let (p0, mut p1, mut p2) = (pt(&vv[0]), pt(&vv[1]), pt(&vv[2]));
    let mut area = (p1.0 - p0.0) * (p2.1 - p0.1) - (p1.1 - p0.1) * (p2.0 - p0.0);
    if area == 0 {
        return;
    }
    if area < 0 {
        std::mem::swap(&mut p1, &mut p2);
        vv.swap(1, 2);
        area = -area;
    }
    let minx = ((p0.0.min(p1.0).min(p2.0) + 15) >> 4).max(cx.sc[0] as i64).max(rect[0] as i64).max(0);
    let maxx = (p0.0.max(p1.0).max(p2.0) >> 4).min(cx.sc[1] as i64).min(rect[2] as i64).min(1023);
    let miny = ((p0.1.min(p1.1).min(p2.1) + 15) >> 4).max(cx.sc[2] as i64).max(rect[1] as i64).max(0);
    let maxy = (p0.1.max(p1.1).max(p2.1) >> 4).min(cx.sc[3] as i64).min(rect[3] as i64).min(1023);
    if minx > maxx || miny > maxy {
        return;
    }
    let edges = [(p1, p2), (p2, p0), (p0, p1)];
    let tl = |a: (i64, i64), b: (i64, i64)| {
        let (dx, dy) = (b.0 - a.0, b.1 - a.1);
        dy < 0 || (dy == 0 && dx > 0)
    };
    let bias = [tl(edges[0].0, edges[0].1) as i64 - 1, tl(edges[1].0, edges[1].1) as i64 - 1, tl(edges[2].0, edges[2].1) as i64 - 1];
    let ea = [-(edges[0].1 .1 - edges[0].0 .1), -(edges[1].1 .1 - edges[1].0 .1), -(edges[2].1 .1 - edges[2].0 .1)];
    let eb = [edges[0].1 .0 - edges[0].0 .0, edges[1].1 .0 - edges[1].0 .0, edges[2].1 .0 - edges[2].0 .0];
    let ec = [
        -(edges[0].1 .0 - edges[0].0 .0) * edges[0].0 .1 + (edges[0].1 .1 - edges[0].0 .1) * edges[0].0 .0,
        -(edges[1].1 .0 - edges[1].0 .0) * edges[1].0 .1 + (edges[1].1 .1 - edges[1].0 .1) * edges[1].0 .0,
        -(edges[2].1 .0 - edges[2].0 .0) * edges[2].0 .1 + (edges[2].1 .1 - edges[2].0 .1) * edges[2].0 .0,
    ];
    // Trivial reject: if one edge function is negative at all four corners of the pixel rectangle, no pixel of it
    // is inside (exact: the edge function is linear), so skip the plane setup.
    for i in 0..3 {
        let at = |x: i64, y: i64| ea[i] * (x << 4) + eb[i] * (y << 4) + ec[i] + bias[i];
        if at(minx, miny) < 0 && at(maxx, miny) < 0 && at(minx, maxy) < 0 && at(maxx, maxy) < 0 {
            return;
        }
    }
    let inv = 1.0f64 / area as f64;
    let z = plane(&vv, &ea, &eb, &ec, inv, |t| t.z as f64);
    let iip = cx.iip;
    let zero = (0.0, 0.0, 0.0);
    let mut cols = [zero; 4];
    if iip {
        for (k, c) in cols.iter_mut().enumerate() {
            *c = plane(&vv, &ea, &eb, &ec, inv, |t| t.rgba[k] as f64);
        }
    }
    let fog = if cx.fge { plane(&vv, &ea, &eb, &ec, inv, |t| t.f as f64) } else { zero };
    let (su, sv, sq) = if !cx.tme {
        (zero, zero, zero)
    } else if cx.fst {
        (plane(&vv, &ea, &eb, &ec, inv, |t| t.u as f64 / 16.0), plane(&vv, &ea, &eb, &ec, inv, |t| t.v as f64 / 16.0), (1.0, 0.0, 0.0))
    } else {
        (plane(&vv, &ea, &eb, &ec, inv, |t| t.s as f64), plane(&vv, &ea, &eb, &ec, inv, |t| t.t as f64), plane(&vv, &ea, &eb, &ec, inv, |t| t.q as f64))
    };
    let flat = [vv[2].rgba[0] as f32, vv[2].rgba[1] as f32, vv[2].rgba[2] as f32, vv[2].rgba[3] as f32];
    let (tw, th) = (cx.tw as f64, cx.th as f64);
    for y in miny..=maxy {
        let py = y << 4;
        let mut e = [ea[0] * (minx << 4) + eb[0] * py + ec[0], ea[1] * (minx << 4) + eb[1] * py + ec[1], ea[2] * (minx << 4) + eb[2] * py + ec[2]];
        let fy = y as f64;
        // Row terms gy*fy: the prototype evaluates (c0 + gx*fx) + gy*fy, so hoisting gy*fy is exact.
        let zy = z.2 * fy;
        let cy = [cols[0].2 * fy, cols[1].2 * fy, cols[2].2 * fy, cols[3].2 * fy];
        let fogy = fog.2 * fy;
        let (suy, svy, sqy) = (su.2 * fy, sv.2 * fy, sq.2 * fy);
        let row = (y as usize) * 1024;
        let mut inside_seen = false;
        for x in minx..=maxx {
            if e[0] + bias[0] >= 0 && e[1] + bias[1] >= 0 && e[2] + bias[2] >= 0 {
                inside_seen = true;
                acc.cover += 1;
                let fx = x as f64;
                let zz = z.0 + z.1 * fx + zy;
                let zi = clampf(zz) as u32;
                let pos = row + x as usize;
                if !zpass(d, pos, zi) {
                    acc.zfail += 1;
                } else {
                    let f = if cx.fge { (fog.0 + fog.1 * fx + fogy) as f32 } else { 0.0 };
                    let rgba = if iip {
                        [
                            (cols[0].0 + cols[0].1 * fx + cy[0]) as f32,
                            (cols[1].0 + cols[1].1 * fx + cy[1]) as f32,
                            (cols[2].0 + cols[2].1 * fx + cy[2]) as f32,
                            (cols[3].0 + cols[3].1 * fx + cy[3]) as f32,
                        ]
                    } else {
                        flat
                    };
                    let (mut u, mut vt, mut pq) = (0.0f32, 0.0f32, 1.0f32);
                    if cx.tme {
                        let s = su.0 + su.1 * fx + suy;
                        let t = sv.0 + sv.1 * fx + svy;
                        if cx.fst {
                            u = s as f32;
                            vt = t as f32;
                        } else {
                            let q = sq.0 + sq.1 * fx + sqy;
                            let rq = if q != 0.0 { 1.0 / q } else { 1.0 };
                            pq = q as f32;
                            u = (s * rq * tw) as f32;
                            vt = (t * rq * th) as f32;
                        }
                    }
                    shade(d, x as u32, y as u32, pos, zi, rgba, u, vt, f, pq, acc);
                }
            } else if inside_seen {
                break;
            }
            e[0] += ea[0] << 4;
            e[1] += ea[1] << 4;
            e[2] += ea[2] << 4;
        }
    }
}

#[inline(always)]
unsafe fn line(d: &Dst, a: Vtx, b: Vtx, rect: [i32; 4], acc: &mut Acc) {
    let cx = d.cx;
    let (x0, y0) = ((a.x as i32 - cx.ofx) as f32 / 16.0, (a.y as i32 - cx.ofy) as f32 / 16.0);
    let (x1, y1) = ((b.x as i32 - cx.ofx) as f32 / 16.0, (b.y as i32 - cx.ofy) as f32 / 16.0);
    let n = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
    let pa = a.rgba.map(|c| c as f32);
    let pb = b.rgba.map(|c| c as f32);
    let (u, v) = uv_of(cx, &b);
    let f = b.f as f32;
    for i in 0..n {
        let t = i as f32 / n as f32;
        let mut c = pb;
        if cx.at & 8 != 0 {
            for k in 0..4 {
                c[k] = pa[k] + (pb[k] - pa[k]) * t;
            }
        }
        let (x, y) = ((x0 + (x1 - x0) * t).round() as i32, (y0 + (y1 - y0) * t).round() as i32);
        if x >= cx.sc[0] && x <= cx.sc[1] && y >= cx.sc[2] && y <= cx.sc[3] && x >= rect[0] && x <= rect[2] && y >= rect[1] && y <= rect[3] && x >= 0 && y >= 0 && x < 1024 && y < 1024 {
            pixel(d, x, y, b.z, c, u, v, f, acc);
        }
    }
}

/// One pixel at (x, y) inside 0..1023 (points and lines).
#[allow(clippy::too_many_arguments)]
#[inline(always)]
unsafe fn pixel(d: &Dst, x: i32, y: i32, z: u32, rgba: [f32; 4], pu: f32, pv: f32, pf: f32, acc: &mut Acc) {
    acc.cover += 1;
    let pos = (y as usize) * 1024 + x as usize;
    if !zpass(d, pos, z) {
        acc.zfail += 1;
        return;
    }
    shade(d, x as u32, y as u32, pos, z, rgba, pu, pv, pf, 1.0, acc);
}

/// Z test (prototype order: it has no side effects, so it can run first).
#[inline(always)]
unsafe fn zpass(d: &Dst, pos: usize, z: u32) -> bool {
    let cx = d.cx;
    if !cx.zte {
        return true;
    }
    let zm = cx.zmask;
    let (zs, zd) = (z & zm, *d.zp.add(pos) & zm);
    match cx.ztst {
        0 => false,
        1 => true,
        2 => zs >= zd,
        _ => zs > zd,
    }
}

/// Rasterized fields so far (stamped on the GSPIX probe lines: the field a write belongs to).
pub static PROBE_FIELD: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// `i32::clamp` for lo <= hi, always inlined (the std one is an out-of-line call in this big function).
#[inline(always)]
/// Debug probe: `RAC_GSPIX=x,y` logs every write to that framebuffer pixel (vertex colour, texel, GS regs, before / after).
fn gspix_probe() -> Option<(u32, u32)> {
    static P: std::sync::OnceLock<Option<(u32, u32)>> = std::sync::OnceLock::new();
    *P.get_or_init(|| {
        let v = std::env::var("RAC_GSPIX").ok()?;
        let (a, b) = v.split_once(',')?;
        Some((a.trim().parse().ok()?, b.trim().parse().ok()?))
    })
}

/// PROBE (fx21): RAC_GSPIX_COLS=x:y0-y1;x:y0-y1 prints GSPIX lines for those pixel columns.
fn gspix_cols() -> &'static [(u32, u32, u32)] {
    static P: std::sync::OnceLock<Vec<(u32, u32, u32)>> = std::sync::OnceLock::new();
    P.get_or_init(|| {
        let v = std::env::var("RAC_GSPIX_COLS").unwrap_or_default();
        v.split(';')
            .filter_map(|c| {
                let (x, r) = c.split_once(':')?;
                let (a, b) = r.split_once('-')?;
                Some((x.trim().parse().ok()?, a.trim().parse().ok()?, b.trim().parse().ok()?))
            })
            .collect()
    })
}

fn clampi(v: i32, lo: i32, hi: i32) -> i32 {
    v.max(lo).min(hi)
}

/// `f64::clamp(0, u32::MAX)` (NaN stays NaN, as in std), always inlined.
#[inline(always)]
fn clampf(v: f64) -> f64 {
    let mut x = v;
    if x < 0.0 {
        x = 0.0;
    }
    if x > 4294967295.0 {
        x = 4294967295.0;
    }
    x
}

/// `f32::floor` without a libm call (the x86-64 baseline has no SSE4.1 round): exact for every input,
/// including NaN, infinities and values past 2^23 (already integers).
#[inline(always)]
fn ffloor(x: f32) -> f32 {
    if x.abs() < 8388608.0 {
        let t = (x as i32) as f32;
        if t > x {
            t - 1.0
        } else {
            t
        }
    } else {
        x
    }
}

#[inline(always)]
fn wrap(m: u8, c: i32, size: u32, lo: u32, hi: u32) -> u32 {
    match m {
        0 => (c as u32) & (size - 1),
        1 => clampi(c, 0, size as i32 - 1) as u32,
        2 => c.max(lo as i32).min(hi as i32).max(0) as u32,
        _ => ((c as u32) & lo) | hi,
    }
}

/// Texel colour (RGBA bytes as a little-endian u32) at wrapped texture coordinates.
#[inline(always)]
unsafe fn texel_xy(d: &Dst, x: u32, y: u32) -> u32 {
    let cx = d.cx;
    match cx.tpsm {
        0x13 => u32::from_le_bytes(d.clut[*d.vram.add(a8(d.l, cx.tbp, cx.tbw, x, y)) as usize]),
        0x00 => (d.vram.add(a32(d.l, cx.tbp, cx.tbw, x, y)) as *const u32).read_unaligned(),
        psm => {
            let raw = rd_px(d.vram, d.l, psm, cx.tbp, cx.tbw, x, y);
            u32::from_le_bytes(match psm {
                0x1b => d.clut[raw as usize & 0xff],
                0x14 | 0x24 | 0x2c => d.clut[(raw as usize & 0xf) + cx.clut_off as usize],
                _ => unpack_col(psm, raw, cx.texa),
            })
        }
    }
}

#[inline(always)]
unsafe fn texel(d: &Dst, u: i32, v: i32) -> [u8; 4] {
    let cx = d.cx;
    let x = wrap(cx.wms, u, cx.tw, cx.minu, cx.maxu);
    let y = wrap(cx.wmt, v, cx.th, cx.minv, cx.maxv);
    texel_xy(d, x, y).to_le_bytes()
}

/// The 2x2 texels (x0, y0), (x0 + 1, y0), (x0, y0 + 1), (x0 + 1, y0 + 1) as `texel` returns them: the four
/// wrapped coordinates are computed once and, for 8-bit and 32-bit textures, so is the page part of the address.
#[inline(always)]
unsafe fn texel4(d: &Dst, x0: i32, y0: i32) -> [u32; 4] {
    let cx = d.cx;
    let xa = wrap(cx.wms, x0, cx.tw, cx.minu, cx.maxu);
    let xb = wrap(cx.wms, x0.wrapping_add(1), cx.tw, cx.minu, cx.maxu);
    let ya = wrap(cx.wmt, y0, cx.th, cx.minv, cx.maxv);
    let yb = wrap(cx.wmt, y0.wrapping_add(1), cx.th, cx.minv, cx.maxv);
    let bp = cx.tbp.wrapping_mul(256);
    match cx.tpsm {
        0x13 => {
            let pw = (cx.tbw / 2).max(1);
            let l8 = &d.l.l8;
            let at = |x: u32, y: u32| -> usize {
                let page = (y / 64).wrapping_mul(pw).wrapping_add(x / 128);
                (bp.wrapping_add(page.wrapping_mul(8192)).wrapping_add(l8[(((y & 63) << 7) | (x & 127)) as usize]) & 0x3f_ffff) as usize
            };
            let px = |a: usize| u32::from_le_bytes(d.clut[*d.vram.add(a) as usize]);
            [px(at(xa, ya)), px(at(xb, ya)), px(at(xa, yb)), px(at(xb, yb))]
        }
        0x00 => {
            let l32 = &d.l.l32;
            let at = |x: u32, y: u32| -> usize {
                let page = (y / 32).wrapping_mul(cx.tbw).wrapping_add(x / 64);
                (bp.wrapping_add(page.wrapping_mul(8192)).wrapping_add(l32[(((y & 31) << 6) | (x & 63)) as usize]) & 0x3f_ffff) as usize
            };
            let px = |a: usize| (d.vram.add(a) as *const u32).read_unaligned();
            [px(at(xa, ya)), px(at(xb, ya)), px(at(xa, yb)), px(at(xb, yb))]
        }
        _ => [texel_xy(d, xa, ya), texel_xy(d, xb, ya), texel_xy(d, xa, yb), texel_xy(d, xb, yb)],
    }
}

/// Texel at texel coordinates (pu, pv) of the context's texture, point or bilinear.
#[inline(always)]
unsafe fn sample(d: &Dst, pu: f32, pv: f32, bilinear: bool) -> [u8; 4] {
    if bilinear {
        let (u, v) = (pu - 0.5, pv - 0.5);
        let (x0, y0) = (ffloor(u), ffloor(v));
        let (fx, fy) = (((u - x0) * 16.0) as i32, ((v - y0) * 16.0) as i32);
        let (x0, y0) = (x0 as i32, y0 as i32);
        // fx, fy are in 0..=15: (u - floor(u)) is in [0, 1) (NaN and infinities give 0).
        bilerp(texel4(d, x0, y0), fx as u32 & 15, fy as u32 & 15)
    } else {
        texel(d, ffloor(pu) as i32, ffloor(pv) as i32)
    }
}

/// Texel of mip level `n`: that level's TBP / TBW (`Ctx::level_tbp`), size TW >> n x TH >> n, REGION_CLAMP bounds >> n,
/// coordinates scaled by 2^-n; same CLUT.
#[inline(always)]
unsafe fn sample_level(d: &Dst, n: u32, pu: f32, pv: f32, bilinear: bool) -> [u8; 4] {
    if n == 0 {
        return sample(d, pu, pv, bilinear);
    }
    let mut c = *d.cx;
    let (bp, bw) = c.level_tbp(n);
    c.tbp = bp;
    c.tbw = bw;
    c.tw = (c.tw >> n).max(1);
    c.th = (c.th >> n).max(1);
    if c.wms == 2 {
        c.minu >>= n;
        c.maxu >>= n;
    }
    if c.wmt == 2 {
        c.minv >>= n;
        c.maxv >>= n;
    }
    let s = 1.0 / (1u32 << n) as f32;
    let dl = Dst { cx: &c, clut: d.clut, l: d.l, vram: d.vram, zp: d.zp };
    sample(&dl, pu * s, pv * s, bilinear)
}

/// RGBA bytes of a u32 as four 16-bit lanes of a u64.
#[inline(always)]
fn lanes(t: u32) -> u64 {
    let v = t as u64;
    let v = (v | (v << 16)) & 0x0000_ffff_0000_ffff;
    (v | (v << 8)) & 0x00ff_00ff_00ff_00ff
}

/// Bilinear blend of the four texels with 4-bit weights fx, fy in 0..=15, per channel
/// `((t00 * (16 - fx) + t10 * fx) * (16 - fy) + (t01 * (16 - fx) + t11 * fx) * fy) >> 8`, all channels at once
/// (every lane stays below 2^16: the weighted sum is at most 255 * 256).
#[inline(always)]
fn bilerp(t: [u32; 4], fx: u32, fy: u32) -> [u8; 4] {
    let (w0, w1) = (16 - fx as u64, fx as u64);
    let a = lanes(t[0]) * w0 + lanes(t[1]) * w1;
    let b = lanes(t[2]) * w0 + lanes(t[3]) * w1;
    let r = (a * (16 - fy as u64) + b * fy as u64) >> 8 & 0x00ff_00ff_00ff_00ff;
    let v = (r | (r >> 8)) & 0x0000_ffff_0000_ffff;
    let v = (v | (v >> 16)) as u32;
    v.to_le_bytes()
}

/// Texture, fog, alpha test, DATE, z write, blend and the frame write of a pixel that passed the z test
/// (prototype `Gs::pixel`).
#[allow(clippy::too_many_arguments)]
#[inline(always)]
unsafe fn shade(d: &Dst, x: u32, y: u32, pos: usize, z: u32, rgba: [f32; 4], pu: f32, pv: f32, pf: f32, pq: f32, acc: &mut Acc) {
    let cx = d.cx;
    let probe = match gspix_probe() {
        Some((0, step)) => x % step == 0 && y % step == 0 && cx.tme && (1..0x30).contains(&(rgba[3] as i32)) && cx.alpha & 0xff == 0x44,
        p => p == Some((x, y)) || gspix_cols().iter().any(|&(cx0, y0, y1)| cx0 == x && (y0..=y1).contains(&y)),
    };
    let mut probe_t = [0u8; 4];
    let mut c = [clampi(rgba[0] as i32, 0, 255), clampi(rgba[1] as i32, 0, 255), clampi(rgba[2] as i32, 0, 255), clampi(rgba[3] as i32, 0, 255)];
    let mut probe_lv = (0u32, 0u32, 0u32);
    if cx.tme {
        let t = if cx.lodsel {
            let (l0, l1, fr, bil) = cx.mip_level(pq);
            probe_lv = (l0, l1, fr);
            let t0 = sample_level(d, l0, pu, pv, bil);
            if fr == 0 {
                t0
            } else {
                let t1 = sample_level(d, l1, pu, pv, bil);
                std::array::from_fn(|k| ((t0[k] as u32 * (256 - fr) + t1[k] as u32 * fr) >> 8) as u8)
            }
        } else {
            sample(d, pu, pv, cx.bilinear)
        };
        probe_t = t;
        match cx.tfx {
            0 => {
                for k in 0..3 {
                    c[k] = ((t[k] as i32 * c[k]) >> 7).min(255);
                }
                if cx.tcc {
                    c[3] = ((t[3] as i32 * c[3]) >> 7).min(255);
                }
            }
            1 => {
                for k in 0..3 {
                    c[k] = t[k] as i32;
                }
                if cx.tcc {
                    c[3] = t[3] as i32;
                }
            }
            tfx => {
                let a = c[3];
                for k in 0..3 {
                    c[k] = ((t[k] as i32 * c[k]) >> 7) + a;
                    c[k] = c[k].min(255);
                }
                c[3] = if cx.tcc { if tfx == 2 { (t[3] as i32 + a).min(255) } else { t[3] as i32 } } else { a };
            }
        }
    }
    if cx.fge {
        let f = pf as i32;
        for k in 0..3 {
            c[k] = (c[k] * f + cx.fogc[k] * (255 - f)) >> 8;
        }
    }
    let mut wr_fb = true;
    let mut wr_z = cx.zwrite;
    let mut wr_a = true;
    if cx.ate {
        let (a, aref) = (c[3], cx.aref);
        let pass = match cx.atst {
            0 => false,
            1 => true,
            2 => a < aref,
            3 => a <= aref,
            4 => a == aref,
            5 => a >= aref,
            6 => a > aref,
            _ => a != aref,
        };
        if !pass {
            match cx.afail {
                0 => return,
                1 => wr_z = false,
                2 => wr_fb = false,
                _ => {
                    wr_z = false;
                    wr_a = false;
                }
            }
        }
    }
    let fpsm = cx.fpsm;
    let fa = if fpsm == 0 { a32(d.l, cx.fbp, cx.fbw, x, y) } else { 0 };
    let dst = if !cx.need_dst {
        0
    } else if fpsm == 0 {
        (d.vram.add(fa) as *const u32).read_unaligned()
    } else {
        rd_px(d.vram, d.l, fpsm, cx.fbp, cx.fbw, x, y)
    };
    if cx.date {
        let abit = match fpsm {
            0x00 => dst >> 31,
            0x02 | 0x0a => (dst >> 15) & 1,
            _ => 1,
        };
        if abit != cx.datm {
            return;
        }
    }
    if wr_z {
        *d.zp.add(pos) = z & cx.zmask;
    }
    if !wr_fb {
        return;
    }
    let abe = cx.abe && !(cx.pabe && c[3] < 128);
    let mut out = [c[0], c[1], c[2]];
    if abe {
        let dc = unpack_col(fpsm, dst, cx.texa);
        let da = match fpsm {
            0x00 => (dst >> 24) as i32,
            0x02 | 0x0a => {
                if (dst >> 15) & 1 != 0 {
                    0x80
                } else {
                    0
                }
            }
            _ => 0x80,
        };
        let cf = match cx.bc {
            0 => c[3],
            1 => da,
            _ => cx.fix,
        };
        let pick = |s: u8, k: usize| match s {
            0 => c[k],
            1 => dc[k] as i32,
            _ => 0,
        };
        for k in 0..3 {
            let v = (((pick(cx.ba, k) - pick(cx.bb, k)) * cf) >> 7) + pick(cx.bd, k);
            out[k] = if cx.colclamp { clampi(v, 0, 255) } else { v & 0xff };
        }
    }
    let mut a = if wr_a { c[3] } else { 0 };
    if cx.fba {
        a |= 0x80;
    }
    let v = match fpsm {
        0x02 | 0x0a => {
            let b5 = |f: i32| (clampi(f, 0, 255) as u32) >> 3;
            b5(out[0]) | b5(out[1]) << 5 | b5(out[2]) << 10 | ((a >= 128) as u32) << 15
        }
        0x01 => (out[0] as u32) | (out[1] as u32) << 8 | (out[2] as u32) << 16,
        _ => (out[0] as u32) | (out[1] as u32) << 8 | (out[2] as u32) << 16 | (clampi(a, 0, 255) as u32) << 24,
    };
    let v = if wr_a || fpsm == 0x01 { v } else { (v & 0x00ff_ffff) | (dst & 0xff00_0000) };
    let v = (v & !cx.fbmsk) | (dst & cx.fbmsk);
    if probe {
        eprintln!("GSPIX f{} {x},{y} fbp {:#x} vtx {:?} tex {:?} tfx {} tcc {} alpha {:#x} test {:#x} tex0 {:#x} zw {wr_z} dst {dst:#010x} -> {v:#010x} clamp {:#x} tex1 {:#x} mip1 {:#x} q {pq} lv {:?} uv {pu} {pv} tbp_lv {:?}", PROBE_FIELD.load(std::sync::atomic::Ordering::Relaxed), cx.fbp, rgba.map(|f| f as i32), probe_t, cx.tfx, cx.tcc, cx.alpha, cx.test, cx.tex0, cx.clamp, cx.tex1, cx.mip1, probe_lv, if probe_lv.0 > 0 { cx.level_tbp(probe_lv.0) } else { (cx.tbp, cx.tbw) });
    }
    if fpsm == 0 {
        (d.vram.add(fa) as *mut u32).write_unaligned(v);
    } else {
        wr_px(d.vram, d.l, fpsm, cx.fbp, cx.fbw, x, y, v);
    }
    acc.written += 1;
}
