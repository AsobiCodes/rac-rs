//! R&C2 HUD overlay mode of the software GS (`overlay.rs` drives it): while rac-rs draws the 3D world natively, the GS draws
//! only the HUD (and the effect programs `overlay::progs` still runs) into a cleared, see-through buffer; the window lays
//! that over the native picture. Off (`GsOvl::on` false) for every R&C1 field. Child of `gs.rs` (Gs's private fields).
use super::super::gs_mem::{a32, luts, span};
use super::super::gs_raster::Ctx;
use super::Gs;

/// Overlay state of a field (Gs::ovl).
#[derive(Default)]
pub(in crate::gsfull) struct GsOvl {
    /// HUD overlay field (overlay::raster): on, the shown size, the buffer the HUD draws into this field (FBP blocks, FBW)
    /// and last field's.
    pub(in crate::gsfull) on: bool,
    size: (u32, u32),
    target: Option<(u32, u32)>,
    /// The part of the overlay buffer cleared this field (w, h): the HUD context's SCISSOR within the shown size.
    clip: (u32, u32),
    last: Option<u32>,
    /// The segment being fed is an effect program's (overlay::raster): its overlay pixels are depth-tested (Ctx::ozfx).
    fx: bool,
    /// The prim being drawn is a world layer (`world_layer`): drawn into the overlay like an effect.
    layer: bool,
    /// The GPU capture draws this field's HUD (`set_hud_skip`; gscap/rc2.rs): the HUD prims are not drawn here, only the effects.
    pub(in crate::gsfull) skip_hud: bool,
    /// Nearest effect Z per overlay pixel (1024 x 1024; allocated by the first overlay field).
    z: Vec<u32>,
}

impl Gs {
    /// Path, VU1 program and HUD flag of the data fed next (overlay fields: the overlay context depends on the HUD flag).
    pub fn set_seg(&mut self, path: u8, prog: u8, hud: bool) {
        if hud != self.hud {
            self.ctx_dirty = true;
        }
        (self.path, self.prog, self.hud) = (path, prog, hud);
    }

    /// Starts a field in HUD overlay mode (shown size w x h) or not. The game draws the HUD into its back buffer (Aranos:
    /// FBP 0x1e00 blocks) and copies that to the shown one at the next field's start [C: rac2 shots RAC2_FBLOG field 2540],
    /// so the overlay is the buffer of the first HUD draw, cleared just before it; last field's buffer gets no scene draws.
    pub fn set_overlay(&mut self, on: bool, size: (u32, u32)) {
        if on != self.ovl.on {
            self.ctx_dirty = true;
        }
        self.ovl.last = if on { self.ovl.target.map(|o| o.0) } else { None };
        (self.ovl.on, self.ovl.size, self.ovl.target) = (on, size, None);
    }

    /// The segment fed next is an effect program's (its overlay pixels are depth-tested against the native picture).
    pub fn set_fx(&mut self, fx: bool) {
        if fx != self.ovl.fx {
            self.ctx_dirty = true;
        }
        self.ovl.fx = fx;
    }

    /// The HUD of the overlay fields fed next is drawn by the GPU capture at the window's resolution (gscap/rc2.rs): this GS then
    /// draws only the effect programs' and the world layers' overlay pixels (nothing is drawn twice).
    pub fn set_hud_skip(&mut self, on: bool) {
        self.ovl.skip_hud = on;
    }

    /// The overlay's depth for the w x h shown rectangle: per pixel the view depth (metres) of the nearest effect drawn
    /// there, 0 where the HUD drew or nothing did (always in front). GS Z = 2^24 Q - 480 with Q = 1 / (32 depth) for
    /// every R&C2 VU1 program [C: level 1 field 2700 vertices of programs 2-11, RAC2_ZQLOG], so depth = 524288 / (Z + 480).
    pub fn overlay_depth(&mut self, w: u32, h: u32) -> Vec<f32> {
        self.finish();
        let mut out = vec![0f32; (w * h) as usize];
        if self.ovl.z.is_empty() {
            return out;
        }
        for y in 0..h.min(1024) {
            for x in 0..w.min(1024) {
                let z = self.ovl.z[(y * 1024 + x) as usize];
                if z != 0 && z != u32::MAX {
                    out[(y * w + x) as usize] = 524288.0 / (z as f32 + 480.0);
                }
            }
        }
        out
    }

    /// The buffer (FBP blocks, FBW) the HUD went into this field (None: no HUD draw).
    pub fn overlay_target(&self) -> Option<(u32, u32)> {
        self.ovl.target
    }

    /// The cleared part of the overlay buffer (w, h); outside it the buffer holds other data, not HUD.
    pub fn overlay_clip(&self) -> (u32, u32) {
        self.ovl.clip
    }

    /// Clears the shown rectangle (FBP blocks, FBW, DBX, DBY, w, h) to black, fully see-through (top byte 255).
    fn overlay_clear(&mut self, (fbp, fbw, x0, y0, w, h): (u32, u32, u32, u32, u32, u32)) {
        let (lo, hi) = span(0, fbp, fbw, x0, y0, w, h);
        self.flush(lo, hi);
        self.finish();
        let l = luts();
        for y in y0..y0 + h {
            for x in x0..x0 + w {
                let a = a32(l, fbp, fbw, x, y);
                self.vram[a..a + 4].copy_from_slice(&0xff00_0000u32.to_le_bytes());
                if let Some(z) = self.ovl.z.get_mut((y * 1024 + x) as usize).filter(|_| x < 1024) {
                    *z = 0;
                }
            }
        }
    }

    /// make_ctx in an overlay field: the first HUD draw picks (and clears) the overlay buffer; draws into it get Ctx::ovl.
    pub(super) fn overlay_ctx(&mut self, cx: &mut Ctx, fbp: u32, fbw: u32, fpsm: u32, sc: u64) {
        let hud = self.hud || self.ovl.layer;
        if hud && fpsm <= 1 && self.ovl.target.is_none() {
            self.ovl.target = Some((fbp, fbw));
            // Only the drawing area (SCISSOR): the game keeps CLUTs right after its 512 x 448 HUD buffer, which a clear of
            // the 512-line shown size wiped [C: Aranos FBP 0x1e00, Ratchet's T8H CLUTs at CBP 0x2c00..0x2cac; Ratchet
            // black in the window].
            let (w, h) = self.ovl.size;
            let (w, h) = (w.min(((sc >> 16) & 0x7ff) as u32 + 1), h.min(((sc >> 48) & 0x7ff) as u32 + 1));
            self.ovl.clip = (w, h);
            if std::env::var_os("RAC2_OVLLOG").is_some() {
                eprintln!("overlay: fbp {fbp:#x} fbw {fbw} shown {:?} scissor {sc:#x} clear {w}x{h}", self.ovl.size);
            }
            self.overlay_clear((fbp, fbw, 0, 0, w, h));
        }
        cx.ovl = hud && fpsm <= 1 && self.ovl.target.map(|o| o.0) == Some(fbp);
        if cx.ovl {
            if self.ovl.z.is_empty() {
                self.ovl.z = vec![0; 1024 * 1024];
            }
            (cx.oz, cx.ozfx) = (self.ovl.z.as_mut_ptr() as usize, self.ovl.fx || self.ovl.layer);
        }
    }

    /// draw in an overlay field: true = skip this prim. A world layer (`world_layer`) is drawn into the overlay as an effect;
    /// the rest of what is drawn before the HUD (the sky, the world clears) is not needed.
    pub(super) fn overlay_draw(&mut self, at: u64) -> bool {
        let layer = self.world_layer(at);
        if layer != self.ovl.layer {
            self.ovl.layer = layer;
            self.ctx_dirty = true;
        }
        !layer && self.overlay_skip(at)
    }

    /// One of the game's own world layers that the native picture does not draw: a prim of the 3D part (not HUD) that the EE
    /// sends itself (not a VU1 program's output: VIF DIRECT or GIF), tests depth against the world drawn before it (ZTE, ZTST
    /// GEQUAL / GREATER) and draws a colour of its own. Level 6's lit tower windows: white quads (TEST 0x5360b, ALPHA 0x44,
    /// TEX0 0x800646a59930728c) laid over the tie bodies by VIF DIRECT, still drawn with every VU1 program skipped [C: level 6
    /// field 3000, emulated picture; RAC_GSPIX 174,107]. Not the shadow passes (TEST 0x51001: alpha test NEVER, sampling the
    /// frame they draw into, TEX0 TBP 0x1e00 [C: same field, RAC_GSPIX 335,425]) or a draw with RGB masked off: as overlay
    /// pixels they came out as black blobs.
    fn world_layer(&self, at: u64) -> bool {
        let c = ((at >> 9) & 1) as usize;
        let (t, frame) = (self.r[0x47 + c], self.r[0x4c + c]);
        let own_frame = at & 0x10 != 0 && (self.r[0x06 + c] & 0x3fff) == (frame & 0x1ff) * 32;
        !self.hud && self.path != 1 && t & 0x10000 != 0 && (t >> 17) & 3 >= 2 && !(t & 1 != 0 && (t >> 1) & 7 == 0) && !own_frame && (frame >> 32) & 0xff_ffff != 0xff_ffff
    }

    /// What is drawn into the shown buffer before the HUD is not needed, and a HUD-part draw that samples the HUD buffer
    /// itself is a screen effect over the 3D picture (Aranos: a full-screen sprite reading FBP 0x1e00), which the native
    /// picture does not have yet [P].
    fn overlay_skip(&self, at: u64) -> bool {
        let c = ((at >> 9) & 1) as usize;
        if let Some(fb) = self.ovl.last.filter(|_| !self.hud) {
            if (self.r[0x4c + c] & 0x1ff) as u32 * 32 == fb {
                return true;
            }
        }
        self.hud && at & 0x10 != 0 && (self.r[0x06 + c] & 0x3fff) as u32 == (self.r[0x4c + c] & 0x1ff) as u32 * 32
    }
}
