//! [PC] DEV (lane tex2): the RAC_TEXNAMES hook of the GS-full pipe (title, front end, and every draw when the game runs with --gsfull).
//! Same job as `retail/gscap/texname.rs`: every textured prim hands its texture registers to `texpack::discover`. Local memory is behind
//! the raster queue here, so a cache miss flushes the deferred uploads and syncs the queue first (slow but exact; once per distinct
//! texture and upload epoch). Zero cost when the variable is unset.

use super::{Gs, Vtx};
use crate::texpack::discover::{self, Draw, Origin};
use std::sync::atomic::{AtomicU32, Ordering::Relaxed};

/// Bumped by every upload and local copy: a texture named before may have changed.
static EPOCH: AtomicU32 = AtomicU32::new(0);

/// Local memory is about to change (host upload, local -> local copy).
pub(super) fn bump() {
    EPOCH.fetch_add(1, Relaxed);
}

impl Gs {
    /// One prim of type `ty` (vertices `vs`), before anything else of `Gs::draw` runs.
    pub(super) fn note_texture(&mut self, ty: u32, vs: &[Vtx; 3]) {
        let at = self.attrs();
        if at & 0x10 == 0 {
            return;
        }
        let c = ((at >> 9) & 1) as usize;
        let (tex0, tex1, clamp) = (self.r[0x06 + c], self.r[0x14 + c], self.r[0x08 + c]);
        let tbp = (tex0 & 0x3fff) as u32;
        // A texture at the frame buffer is a render target (screen copy): PCSX2 never hashes those.
        if tbp == (self.r[0x4c + c] & 0x1ff) as u32 * 32 {
            return;
        }
        let nv = match ty {
            0 => 1,
            1 | 2 | 6 => 2,
            _ => 3,
        };
        let (x0, x1) = vs[..nv].iter().fold((u16::MAX, 0u16), |(a, b), v| (a.min(v.x), b.max(v.x)));
        let (y0, y1) = vs[..nv].iter().fold((u16::MAX, 0u16), |(a, b), v| (a.min(v.y), b.max(v.y)));
        // GS-full has no field counter here: the quadword count in units of 4096 is the time stamp ("first_field" column).
        let d = Draw { tex0, clamp, texa: self.r[0x3b], texclut: self.r[0x1c], tex1, mip1: self.r[0x34 + c], mip2: self.r[0x36 + c], w: u32::from(x1.saturating_sub(x0)) / 16, h: u32::from(y1.saturating_sub(y0)) / 16, field: (self.stats.qw_in >> 12) as u32 };
        let Some(p) = discover::probe(Origin::Full, EPOCH.load(Relaxed), d) else { return };
        self.flush_all();
        self.finish();
        discover::record(p, &self.vram);
    }
}
