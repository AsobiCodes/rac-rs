//! [PC] Texture-pack replacements on the CPU capture (lane v041_hudcpu): the table a replaced texture is sampled from, the view of it
//! the pixel pipeline reads, and the per-prim hook that selects it.
//!
//! The GPU capture samples the pack's image (`gpu_repl.rs`, gpu.wgsl `repl_box`). The CPU overlay is 512 pixels wide, so here the image
//! is boiled down once to the size of the named rectangle in GS texels (`box_table`: the same 4 x 4 grid of samples per texel as
//! `repl_box`) and `View::texel_slow` returns that table's texel instead of decoding the GS texture: the pack's colours and art show,
//! its extra resolution does not (the overlay has no room for it). Which prims get a table is decided in `texrepl_cpu.rs` (naming, the
//! pack's slot); this file has no pack logic. With no textures mod `repl_cpu_prim` returns at its first line and `View::rp` stays empty.

use super::{St, Vtx};
use crate::texpack::hudrepl;
use crate::retail::gscap::Gs;
use std::sync::Arc;

/// A replacement boiled down to the named rectangle: `w` x `h` texels of (r, g, b, a), rgb 0..255 and alpha on the GS scale (128 = opaque).
pub(in crate::retail::gscap) struct CpuRepl {
    pub w: u32,
    pub h: u32,
    /// Unique per table made by a capture (the prim memo keys on it: an address could be reused after an eviction).
    pub serial: u64,
    pub px: Vec<[u8; 4]>,
}

/// The table of one prim and the origin of its rectangle in the GS texture.
#[derive(Clone)]
pub(in crate::retail::gscap) struct CpuSel {
    pub tab: Arc<CpuRepl>,
    pub x: u32,
    pub y: u32,
}

/// The table a `View` reads (raw, so `View` stays `Copy`; the `Arc` lives in `ReplState::cpu_cur` for the whole prim).
#[derive(Clone, Copy)]
pub(super) struct RpView {
    tab: *const [u8; 4],
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

impl RpView {
    pub(super) const NONE: RpView = RpView { tab: std::ptr::null(), x: 0, y: 0, w: 0, h: 0 };

    pub(in crate::retail::gscap) fn new(s: Option<&CpuSel>) -> RpView {
        match s {
            Some(s) if !s.tab.px.is_empty() => RpView { tab: s.tab.px.as_ptr(), x: s.x as i32, y: s.y as i32, w: s.tab.w as i32, h: s.tab.h as i32 },
            _ => RpView::NONE,
        }
    }

    #[inline(always)]
    pub(super) fn on(&self) -> bool {
        !self.tab.is_null()
    }

    /// The texel at GS texel (x, y) of the texture, kept inside the rectangle like `repl_box`; alpha on the GS scale.
    pub(super) fn texel(&self, x: u32, y: u32) -> [f32; 4] {
        let bx = (x as i32 - self.x).clamp(0, self.w - 1);
        let by = (y as i32 - self.y).clamp(0, self.h - 1);
        // SAFETY: `on()` (a table of w * h entries, alive for the prim) and the coordinates are clamped inside it.
        let t = unsafe { *self.tab.add((by * self.w + bx) as usize) };
        [t[0] as f32, t[1] as f32, t[2] as f32, t[3] as f32]
    }
}

/// The image (`iw` x `ih` RGBA8, alpha 0..255) boiled down to `rw` x `rh` texels exactly as gpu.wgsl `repl_box` does for each texel: the
/// mean of a 4 x 4 grid of image pixels (colour weighted by alpha where the alpha sum is over 0.5), rgb rounded, alpha scaled to 128.
pub(in crate::retail::gscap) fn box_table(rgba: &[u8], iw: u32, ih: u32, rw: u32, rh: u32) -> Vec<[u8; 4]> {
    let (kx, ky) = (iw as f32 / rw as f32, ih as f32 / rh as f32);
    // The image column / row of each of the four sample positions of every texel column / row.
    let axis = |n: u32, k: f32, max: u32| -> Vec<[u32; 4]> {
        (0..n).map(|b| std::array::from_fn(|j| (((b as f32 + (j as f32 + 0.5) * 0.25) * k).floor() as i64).clamp(0, max as i64 - 1) as u32)).collect()
    };
    let (xs, ys) = (axis(rw, kx, iw), axis(rh, ky, ih));
    let mut out = Vec::with_capacity((rw * rh) as usize);
    for yi in &ys {
        for xi in &xs {
            let (mut plain, mut wrgb, mut asum) = ([0f32; 3], [0f32; 3], 0f32);
            for &py in yi {
                for &px in xi {
                    let o = ((py * iw + px) * 4) as usize;
                    let a = rgba[o + 3] as f32;
                    for k in 0..3 {
                        let c = rgba[o + k] as f32;
                        plain[k] += c;
                        wrgb[k] += c * a;
                    }
                    asum += a;
                }
            }
            let rgb: [f32; 3] = if asum > 0.5 { wrgb.map(|v| v / asum) } else { plain.map(|v| v / 16.0) };
            let r = |v: f32| (v + 0.5).floor().clamp(0.0, 255.0) as u8;
            out.push([r(rgb[0]), r(rgb[1]), r(rgb[2]), r(asum / 16.0 * (128.0 / 255.0))]);
        }
    }
    out
}

impl Gs {
    /// Called by `raster` for every prim of the CPU capture: selects the replacement table the prim samples (`ReplState::cpu_cur`, read by
    /// `view`) or, for a prim that draws into local memory, marks the blocks it wrote (a texture there is a render target, never named).
    pub(super) fn repl_cpu_prim(&mut self, st: &St, ty: u32, vs: &[Vtx]) {
        if !hudrepl::enabled() {
            return;
        }
        if st.ov {
            self.repl.cpu_cur = self.repl_cpu_for(st.ov, st.at, st.tex0, st.clamp);
            return;
        }
        self.repl.cpu_cur = None;
        let n = match ty {
            6 | 1 | 2 => 2,
            3..=5 => 3,
            0 => 1,
            _ => return,
        };
        let (mut x0, mut x1, mut y0, mut y1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for v in &vs[..n.min(vs.len())] {
            let (x, y) = (v.x as f32 / 16.0 - st.ofx, v.y as f32 / 16.0 - st.ofy);
            (x0, x1, y0, y1) = (x0.min(x), x1.max(x), y0.min(y), y1.max(y));
        }
        let (ix0, ix1) = ((x0.floor() as i32 - 1).max(st.sc[0]).max(0), (x1.ceil() as i32 + 1).min(st.sc[1]));
        let (iy0, iy1) = ((y0.floor() as i32 - 1).max(st.sc[2]).max(0), (y1.ceil() as i32 + 1).min(st.sc[3]));
        if ix1 < ix0 || iy1 < iy0 {
            return;
        }
        let frame = st.frame;
        let (lo, hi) = Self::span(((frame >> 24) & 0x3f) as u32, ((frame & 0x1ff) * 32) as u32, ((frame >> 16) & 0x3f) as u32, ix0 as u32, iy0 as u32, (ix1 - ix0 + 1) as u32, (iy1 - iy0 + 1) as u32);
        if hi != u32::MAX && lo < hi {
            self.repl_drawn_once(lo, hi);
        }
    }
}

#[cfg(test)]
#[path = "cpu_repl_tests.rs"]
mod tests;
