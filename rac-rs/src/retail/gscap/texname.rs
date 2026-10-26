//! [PC] DEV (lane tex2): the RAC_TEXNAMES hook of the GS capture. Every prim that samples a texture hands its texture registers to
//! `texpack::discover`, which names the texture the way PCSX2 would (`texpack::gsname`) and logs it. Zero cost when the variable is
//! unset (`Gs::kick` calls `note_texture`, which returns at once). CPU capture path only: with the GPU backend the local memory lives on
//! the GPU, so discovery runs set RAC_GSCAP_GPU=0.

use super::{Gs, Vtx};
use crate::texpack::discover::{self, Draw, Origin};
use crate::texpack::gsname::has_palette;
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// Bumped by every upload and local copy: a texture named before may have changed.
static EPOCH: AtomicU32 = AtomicU32::new(0);

/// Local memory is about to change (host upload, local -> local copy).
pub(super) fn bump() {
    EPOCH.fetch_add(1, Relaxed);
}

impl Gs {
    /// One prim, before the `no_draw` test (the front end's prims count too: capture runs `no_draw` while GS-full draws them).
    pub(super) fn note_texture(&mut self, vs: &[Vtx]) {
        if discover::on().is_none() {
            return;
        }
        let at = self.attrs();
        if at & 0x10 == 0 {
            return;
        }
        if self.gpu.is_some() {
            static ONCE: std::sync::Once = std::sync::Once::new();
            ONCE.call_once(|| eprintln!("texnames: the GS capture runs on the GPU, its local memory is not readable: set RAC_GSCAP_GPU=0 (GS capture draws skipped)"));
            return;
        }
        let c = ((at >> 9) & 1) as usize;
        let (tex0, tex1, clamp) = (self.r[0x06 + c], self.r[0x14 + c], self.r[0x08 + c]);
        let (psm, tbp, tbw) = (((tex0 >> 20) & 0x3f) as u32, (tex0 & 0x3fff) as u32, ((tex0 >> 14) & 0x3f) as u32);
        // A texture inside the main frame buffer is a render target (screen effects): PCSX2 never hashes those.
        if let Some(f) = self.main_fbp {
            if tbp >= f * 32 && tbp < self.main_end.max(f * 32 + 1) {
                return;
            }
        }
        let (x0, x1) = vs.iter().fold((u16::MAX, 0u16), |(a, b), v| (a.min(v.x), b.max(v.x)));
        let (y0, y1) = vs.iter().fold((u16::MAX, 0u16), |(a, b), v| (a.min(v.y), b.max(v.y)));
        let d = Draw { tex0, clamp, texa: self.r[0x3b], texclut: self.r[0x1c], tex1, mip1: 0, mip2: 0, w: u32::from(x1.saturating_sub(x0)) / 16, h: u32::from(y1.saturating_sub(y0)) / 16, field: self.fields as u32 };
        let Some(p) = discover::probe(Origin::Cap, EPOCH.load(Relaxed), d) else { return };
        let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
        let (lo, hi) = Self::span(psm, tbp, tbw, 0, 0, tw, th);
        self.flush(lo, hi);
        if has_palette(psm) {
            let cbp = ((tex0 >> 37) & 0x3fff) as u32;
            self.flush(cbp, cbp + 4);
        }
        discover::record(p, &self.vram);
    }
}
