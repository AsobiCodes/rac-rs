//! Exact fast path of the CPU GS capture for untextured sprites drawn into a 32-bit VRAM target (PC speed-up, no retail
//! counterpart; RAC_GSCAP_FASTFILL=0 turns it off).
//!
//! The menus' render targets are cleared and faded with big untextured sprites (512 x 127 clears, 32 x 511 blended strips). Every
//! pixel of such a sprite has the same fragment colour, so what `View::put_h` computes is a pure function of the 32-bit word that
//! was in VRAM: this file runs that function (the same formulas, in the same order, on the same f32 values) over raw rows of VRAM
//! instead of through `rd_px` / `unpack_col` / `wr_px` per pixel. The result is byte for byte what `put_h` leaves.
//!
//! The fast path is taken only for: untextured sprite, VRAM target (not the overlay), no Z buffer (`St::zb`), frame format
//! PSMCT32, a full 4 MB local memory. Everything else, and the whole prim when the path is off, goes through `put_h`. The alpha test
//! (ATST / AFAIL) is not a condition: `View::shade` evaluates it once for the prim, as before. `put_h` has no DATE / DATM test, so
//! the path has none either; the other states `put_h` reads are all handled here: FBMSK, ABE / ALPHA (A, B, C, D, FIX), PABE,
//! COLCLAMP, FBA, TEXA (not read for PSMCT32), the scissor (the rect passed in is already clipped), partial pages at the edges.
//! Dither is not modelled by `put_h` (it ignores DTHE).

use super::{par_rows, Pt, Px, St, View};
#[cfg(test)]
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

const VRAM_BYTES: usize = 4 << 20;
/// Pixels of a sprite from which its rows run on the capture's pool (one pass of these fills costs about a nanosecond a pixel, so
/// waking the pool pays only for big ones).
const FAST_PAR_PX: i64 = 1 << 17;
/// Pixels from which a blend with a constant coefficient builds its three 256-entry channel tables.
const LUT_MIN_PX: i64 = 2048;

fn enabled() -> bool {
    #[cfg(test)]
    if OFF.load(Ordering::Relaxed) {
        return false;
    }
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var("RAC_GSCAP_FASTFILL").map_or(true, |v| v != "0"))
}

/// Turns every fast path off while set (the unit tests run the same prim both ways and compare the VRAM bytes).
#[cfg(test)]
pub(super) static OFF: AtomicBool = AtomicBool::new(false);

/// A PSMCT32 buffer: block base of the frame (`FBP * 32`) and its width in pages.
#[derive(Clone, Copy)]
struct Geo {
    bp: u32,
    bw: u32,
}

impl Geo {
    /// Byte offset of the page that holds (x, y): `Gs::a32` without the in-page part (which is below 8 KB and the page start is a
    /// multiple of 8 KB, so the 4 MB wrap of `a32` applies to the page start alone).
    fn page(&self, x: u32, y: u32) -> usize {
        let page = (y / 32) * self.bw + x / 64;
        ((self.bp * 256 + page * 8192) & ((1 << 22) - 1)) as usize
    }
}

/// `View::put_h`'s blend of one colour channel (`cs` the fragment's, `cd` the destination's) for the ALPHA register `al` and the
/// coefficient `cf`, then the byte the 32-bit write stores.
#[inline(always)]
fn blend_ch(al: u64, cs: f32, cd: f32, cf: f32, clamp: bool) -> u32 {
    let pick = |s: u64, cs: f32, cd: f32| match s & 3 {
        0 => cs,
        1 => cd,
        _ => 0.0,
    };
    let v = (pick(al, cs, cd) - pick(al >> 2, cs, cd)) * cf / 128.0 + pick(al >> 6, cs, cd);
    let o = if clamp { v.clamp(0.0, 255.0) } else { (v as i32 & 0xff) as f32 };
    o.clamp(0.0, 255.0) as u32
}

/// One prim's constants, decoded once.
struct Fill {
    msk: u32,
    wa: bool,
    fba: bool,
    /// Alpha byte, shifted into place, of a fragment whose alpha is written (FBA applied).
    a_c: u32,
    /// Colour bytes of an unblended fragment.
    col: u32,
    blend: bool,
    al: u64,
    clamp: bool,
    c: [f32; 4],
    /// The blend coefficient when it is the fragment alpha or FIX; None when it is the destination alpha (C = 1).
    cf: Option<f32>,
}

impl Fill {
    fn new(st: &St, v: &View, c: &[f32; 4], wa: bool) -> Fill {
        let pt = Pt::new(st, v);
        let pabe = pt.pabe && c[3] < 128.0;
        let b8 = |f: f32| f.clamp(0.0, 255.0) as u32;
        let fba = st.fba & 1 != 0;
        Fill {
            msk: (st.frame >> 32) as u32,
            wa,
            fba,
            a_c: b8(if fba { c[3].max(128.0) } else { c[3] }) << 24,
            col: b8(c[0]) | b8(c[1]) << 8 | b8(c[2]) << 16,
            blend: pt.abe && !pabe,
            al: pt.al,
            clamp: v.colclamp & 1 != 0,
            c: *c,
            cf: match (pt.al >> 4) & 3 {
                0 => Some(c[3]),
                1 => None,
                _ => Some(pt.fix),
            },
        }
    }

    /// The alpha byte (in place) the pixel ends with, `d` the destination word.
    #[inline(always)]
    fn alpha(&self, d: u32) -> u32 {
        if self.wa {
            self.a_c
        } else if self.fba {
            (d >> 24).max(128) << 24
        } else {
            d & 0xff00_0000
        }
    }

    /// The frame mask: masked bits keep the destination.
    #[inline(always)]
    fn merge(&self, v: u32, d: u32) -> u32 {
        (v & !self.msk) | (d & self.msk)
    }

    /// The colour bytes of the blend of the fragment with `d`, `cf` the coefficient.
    #[inline(always)]
    fn blend_col(&self, d: u32, cf: f32) -> u32 {
        let dc = [(d & 0xff) as f32, ((d >> 8) & 0xff) as f32, ((d >> 16) & 0xff) as f32];
        let ch = |k: usize| blend_ch(self.al, self.c[k], dc[k], cf, self.clamp);
        ch(0) | ch(1) << 8 | ch(2) << 16
    }

    fn tables(&self, cf: f32) -> Box<[[u8; 256]; 3]> {
        let mut t = Box::new([[0u8; 256]; 3]);
        for (k, row) in t.iter_mut().enumerate() {
            for (i, e) in row.iter_mut().enumerate() {
                *e = blend_ch(self.al, self.c[k], i as f32, cf, self.clamp) as u8;
            }
        }
        t
    }
}

/// The write path of a PSMCT32 VRAM target for fragments that differ per pixel (textured sprites, triangles): `View::put_h` with the
/// frame decode, the format switch, the unpack to floats and the bounds asserts done once per prim, on raw words.
pub(super) struct VramPx {
    g: Geo,
    lut: &'static [u32],
    msk: u32,
    fba: bool,
    abe: bool,
    pabe: bool,
    al: u64,
    fix: f32,
    clamp: bool,
}

impl VramPx {
    /// None when the prim takes the generic `put_h` (the conditions of `sprite`).
    pub(super) fn new(st: &St, v: &View) -> Option<VramPx> {
        let frame = st.frame;
        let (bp, bw, psm) = (((frame & 0x1ff) * 32) as u32, ((frame >> 16) & 0x3f) as u32, ((frame >> 24) & 0x3f) as u32);
        if !enabled() || st.ov || st.zb.is_some() || psm != 0 || v.vram_len < VRAM_BYTES {
            return None;
        }
        let pt = Pt::new(st, v);
        Some(VramPx { g: Geo { bp, bw }, lut: &super::super::luts().0, msk: (frame >> 32) as u32, fba: st.fba & 1 != 0, abe: pt.abe, pabe: pt.pabe, al: pt.al, fix: pt.fix, clamp: v.colclamp & 1 != 0 })
    }

    /// `View::put_h` of the shaded fragment (`c`, `wa`) at (x, y), x and y >= 0.
    #[inline(always)]
    pub(super) fn put(&self, v: &View, x: i32, y: i32, c: &[f32; 4], wa: bool) {
        let (xu, yu) = (x as u32, y as u32);
        let a = self.g.page(xu, yu) + self.lut[((yu & 31) * 64 + (xu & 63)) as usize] as usize;
        debug_assert!(a + 4 <= v.vram_len);
        // SAFETY: as `View::fast_row`.
        let p = unsafe { v.vram.add(a) as *mut u32 };
        let dst = u32::from_le(unsafe { p.read_unaligned() });
        let b8 = |f: f32| f.clamp(0.0, 255.0) as u32;
        let (r, g, b) = if self.abe && !(self.pabe && c[3] < 128.0) {
            let cf = match (self.al >> 4) & 3 {
                0 => c[3],
                1 => (dst >> 24) as f32,
                _ => self.fix,
            };
            let ch = |k: usize, sh: u32| blend_ch(self.al, c[k], ((dst >> sh) & 0xff) as f32, cf, self.clamp);
            (ch(0, 0), ch(1, 8), ch(2, 16))
        } else {
            (b8(c[0]), b8(c[1]), b8(c[2]))
        };
        let a = if wa { c[3] } else { (dst >> 24) as f32 };
        let a = if self.fba { a.max(128.0) } else { a };
        let w = r | g << 8 | b << 16 | b8(a) << 24;
        // SAFETY: as above.
        unsafe { p.write_unaligned(((w & !self.msk) | (dst & self.msk)).to_le()) };
    }
}

impl View {
    /// Applies `f` (destination word -> new word) to the pixels `xs..xe` of row `y` of a PSMCT32 buffer.
    #[inline(always)]
    fn fast_row<F: Fn(u32) -> u32>(&self, g: Geo, y: u32, xs: u32, xe: u32, f: &F) {
        let lut = &super::super::luts().0;
        let row = &lut[(y % 32) as usize * 64..][..64];
        let mut x = xs;
        while x < xe {
            let end = ((x / 64 + 1) * 64).min(xe);
            let page = g.page(x, y);
            for xx in x..end {
                let a = page + row[(xx & 63) as usize] as usize;
                debug_assert!(a + 4 <= self.vram_len);
                // SAFETY: the page start is below 4 MB and a multiple of 8 KB and the in-page offset is a word offset below
                // 8 KB (`in_page_offsets_stay_in_the_page` checks the table), so a + 4 <= 4 MB <= vram_len (checked by
                // `sprite`); rows of one prim share no bytes (`par_ok`, or serial).
                unsafe {
                    let p = self.vram.add(a) as *mut u32;
                    p.write_unaligned(f(u32::from_le(p.read_unaligned())).to_le());
                }
            }
            x = end;
        }
    }

    /// Writes the word `w` to the whole rect: whole pages as one 8 KB fill, the rest row by row.
    fn fast_fill_const(&self, g: Geo, r: (u32, u32, u32, u32), w: u32) {
        let (x0, x1, y0, y1) = r;
        let wle = w.to_le();
        let mut y = y0;
        while y < y1 {
            let band_end = ((y / 32 + 1) * 32).min(y1);
            let full_band = y % 32 == 0 && band_end - y == 32;
            let mut x = x0;
            while x < x1 {
                let xe = ((x / 64 + 1) * 64).min(x1);
                if full_band && x % 64 == 0 && xe - x == 64 {
                    let page = g.page(x, y);
                    debug_assert!(page + 8192 <= self.vram_len);
                    // SAFETY: the page lies inside local memory (page start below 4 MB, multiple of 8 KB).
                    unsafe {
                        let p = self.vram.add(page) as *mut u32;
                        for i in 0..2048 {
                            p.add(i).write_unaligned(wle);
                        }
                    }
                } else {
                    for yy in y..band_end {
                        self.fast_row(g, yy, x, xe, &|_| w);
                    }
                }
                x = xe;
            }
            y = band_end;
        }
    }

    fn fast_run<F: Fn(u32) -> u32 + Sync>(&self, g: Geo, r: (u32, u32, u32, u32), par: bool, f: F) {
        let row = |y: i32| self.fast_row(g, y as u32, r.0, r.1, &f);
        if par {
            par_rows(r.2 as i32..r.3 as i32, row);
        } else {
            (r.2 as i32..r.3 as i32).for_each(row);
        }
    }
}

/// Draws an untextured sprite (`rect` = x0, x1, y0, y1 of the pixels it covers, x1 / y1 exclusive, already clipped to the
/// scissor; `pb` its last vertex, `par` whether its rows may run on the pool) when the fast path applies; false = not drawn, the
/// caller draws it through `put_h`.
pub(super) fn sprite(v: &View, st: &St, pb: &Px, rect: (i32, i32, i32, i32), par: bool) -> bool {
    let frame = st.frame;
    let (bp, bw, psm) = (((frame & 0x1ff) * 32) as u32, ((frame >> 16) & 0x3f) as u32, ((frame >> 24) & 0x3f) as u32);
    if !enabled() || st.ov || st.zb.is_some() || st.at & 0x10 != 0 || psm != 0 || v.vram_len < VRAM_BYTES {
        return false;
    }
    let Some((c, wa)) = v.shade(st, pb) else { return true };
    let (x0, x1, y0, y1) = (rect.0.max(0), rect.1, rect.2.max(0), rect.3);
    if x1 <= x0 || y1 <= y0 {
        return true;
    }
    let fill = Fill::new(st, v, &c, wa);
    let g = Geo { bp, bw };
    let r = (x0 as u32, x1 as u32, y0 as u32, y1 as u32);
    let area = (x1 - x0) as i64 * (y1 - y0) as i64;
    let par = par && area >= FAST_PAR_PX;
    if !fill.blend {
        if fill.msk == 0 && fill.wa {
            v.fast_fill_const(g, r, fill.col | fill.a_c);
        } else {
            v.fast_run(g, r, par, |d| fill.merge(fill.col | fill.alpha(d), d));
        }
    } else if let (Some(cf), true) = (fill.cf, area >= LUT_MIN_PX) {
        let t = fill.tables(cf);
        v.fast_run(g, r, par, |d| fill.merge(t[0][(d & 0xff) as usize] as u32 | (t[1][((d >> 8) & 0xff) as usize] as u32) << 8 | (t[2][((d >> 16) & 0xff) as usize] as u32) << 16 | fill.alpha(d), d));
    } else {
        v.fast_run(g, r, par, |d| fill.merge(fill.blend_col(d, fill.cf.unwrap_or((d >> 24) as f32)) | fill.alpha(d), d));
    }
    true
}
