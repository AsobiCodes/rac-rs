//! GS pixel pipeline for the captured 2D work: sprite/triangle/line raster, texture fetch (CT32/24/16, T8/T4
//! with CLUT), TFX combine, fog, alpha test and blend. Prims into the main render buffer go to the overlay
//! (premultiplied colour plus scene transmission); prims into other buffers are drawn into VRAM.

use super::{Gs, Overlay, Vtx};
use rayon::prelude::*;

pub(super) mod pmemo;
pub(super) mod gpu;

#[derive(Clone, Copy)]
struct St {
    at: u64,
    tex0: u64,
    tex1: u64,
    clamp: u64,
    alpha: u64,
    test: u64,
    frame: u64,
    fba: u64,
    ofx: f32,
    ofy: f32,
    sc: [i32; 4],
    ov: bool,
    /// The Z buffer, for the prims of P1 mode that draw into a VRAM target (see `Zb`).
    zb: Option<Zb>,
}

/// The Z buffer of a prim drawn in P1 mode (the post-game page's robot is a Z-buffered 3D render): ZBUF_n and the
/// TEST register's ZTE / ZTST, PSMZ32 / PSMZ24 only. The capture otherwise has no Z buffer (menus and HUD draw back to
/// front); the Z clears of the page's render pass are ordinary colour draws into the Z buffer's pages, so VRAM holds it.
#[derive(Clone, Copy)]
struct Zb {
    /// Block address of the buffer, the frame's width in pages, PSMZ24 (else PSMZ32), ZMSK.
    bp: u32,
    bw: u32,
    z24: bool,
    msk: bool,
    /// ZTST: 0 never, 1 always, 2 greater or equal, 3 greater (ZTE off counts as always, as on the GS).
    tst: u32,
}

impl Zb {
    fn addr(&self, x: u32, y: u32) -> usize {
        let page = (y / 32) * self.bw + x / 64;
        ((self.bp * 256 + page * 8192 + super::zlut()[((y % 32) * 64 + x % 64) as usize]) & 0x3f_ffff) as usize
    }
}

#[derive(Clone, Copy)]
struct Px {
    rgba: [f32; 4],
    s: f32,
    t: f32,
    q: f32,
    u: f32,
    v: f32,
    f: f32,
    z: f32,
}

impl Gs {
    pub(super) fn draw(&mut self, ty: u32, vs: &[Vtx]) {
        let t0 = self.prof.is_some().then(std::time::Instant::now);
        let key = if t0.is_some() {
            let at = self.attrs();
            let c = ((at >> 9) & 1) as usize;
            let tex0 = self.r[0x06 + c];
            let (w, h) = if ty == 6 && vs.len() > 1 { ((vs[1].x.abs_diff(vs[0].x) / 16), (vs[1].y.abs_diff(vs[0].y) / 16)) } else if vs.len() >= 3 && (3..=5).contains(&ty) { (vs.iter().map(|v| v.x).max().unwrap_or(0) / 16 - vs.iter().map(|v| v.x).min().unwrap_or(0) / 16, vs.iter().map(|v| v.y).max().unwrap_or(0) / 16 - vs.iter().map(|v| v.y).min().unwrap_or(0) / 16) } else { (0, 0) };
            format!("draw ty {ty} fbp {:x} fpsm {:x} tpsm {:x} bil {:x} cl {:x} tme {} abe {} tex0 {:x} sz {w}x{h}", self.r[0x4c + c] & 0x1ff, (self.r[0x4c + c] >> 24) & 0x3f, (tex0 >> 20) & 0x3f, self.r[0x14 + c] & 0x60, self.r[0x08 + c] & 0xf, (at >> 4) & 1, (at >> 6) & 1, if at & 0x10 != 0 { tex0 & 0x3fff } else { 0 })
        } else {
            String::new()
        };
        self.ras_ns = 0;
        self.draw_in(ty, vs);
        self.prof_add(&key, t0);
        if t0.is_some() && self.ras_ns > 0 {
            let r = self.ras_ns;
            self.prof_add_ns(&format!("RASTER {key}"), r);
        }
    }

    fn draw_in(&mut self, ty: u32, vs: &[Vtx]) {
        let mut at = self.attrs();
        let c = ((at >> 9) & 1) as usize;
        let frame = self.r[0x4c + c];
        let fbp = (frame & 0x1ff) as u32;
        let mut tme = at & 0x10 != 0;
        let tex0 = self.r[0x06 + c];
        let tbp = (tex0 & 0x3fff) as u32;
        let test = self.r[0x47 + c];
        let disp = (self.dispfb & 0x1ff) as u32;
        // A path-1 packet of P1 mode: only the render-to-texture targets (the display and main buffers are rac-rs's own).
        if self.p1_only && (self.full || fbp == disp || Some(fbp) == self.main_fbp) {
            return;
        }
        if self.tracing() {
            let v1 = vs.get(1).copied().unwrap_or_default();
            eprintln!(
                "gscap f{} prim ty {ty} at {at:x} fogcol {:x} f {} fbp {fbp:x} frame {frame:x} disp {disp:x} tme {} abe {} tex0 {tex0:x} alpha {:x} test {test:x} sc {:x} xyoff {:x} v0 ({},{}) {:?} st ({},{}) uv ({},{}) v1 ({},{}) {:?} st ({},{}) uv ({},{}) mscal {} main {:?} no_draw {}",
                self.fields,
                self.r[0x3d],
                vs[0].f,
                tme as u8,
                (at >> 6) & 1,
                self.r[0x42 + c],
                self.r[0x40 + c],
                self.r[0x18 + c],
                vs[0].x / 16,
                vs[0].y / 16,
                vs[0].rgba,
                vs[0].s,
                vs[0].t,
                vs[0].u / 16,
                vs[0].v / 16,
                v1.x / 16,
                v1.y / 16,
                v1.rgba,
                v1.s,
                v1.t,
                v1.u / 16,
                v1.v / 16,
                self.mscal,
                self.main_fbp,
                self.no_draw
            );
        }
        // Full mode (the front end's mode 6): the display buffer is the whole picture. Prims into any other buffer (the
        // fade's copies into the main buffer) are never sampled and are not drawn.
        if self.full && fbp != disp {
            return;
        }
        // The level-load loop (`load`, retail.rs `LOAD_ON`): retail draws its starfield into the main buffer (FBP 0x80) and
        // copies it here with textured sprites the next field. Full mode keeps no main buffer, so the copy would show stale
        // VRAM (black after a cold boot, level textures after a Quit Game). Copy-shaped sprites (textured, from a page start
        // of another buffer, the `copy` rule below) are drawn untextured in the main buffer's clear colour (black, alpha
        // 0x80: the loop's 16 clear sprites [C]) instead.
        let solid: [Vtx; 3];
        let vs = if self.full && self.load && tme && ty == 6 && tbp % 32 == 0 && tbp / 32 != disp {
            at &= !0x10;
            tme = false;
            solid = std::array::from_fn(|i| Vtx { rgba: [0, 0, 0, 0x80], ..vs.get(i).copied().unwrap_or_default() });
            &solid[..vs.len().min(3)]
        } else {
            vs
        };
        // The main render buffer is learnt from the sprites that copy it to the display buffer; the first strip
        // of each copy ends the frame.
        let copy = !self.full && fbp == disp && tme && ty == 6 && tbp % 32 == 0 && tbp / 32 != disp;
        if copy {
            self.copied = true;
            self.main_fbp = Some(tbp / 32);
            self.main_bw = ((tex0 >> 14) & 0x3f) as u32;
            if !self.last_copy {
                self.publish();
            }
        }
        self.last_copy = copy;
        if self.log {
            let k = format!(
                "prim fbp {fbp:x} m{} ty {ty} tme {} abe {} tex0 {:x} alpha {:x} test {:x} z{:x}",
                self.mscal,
                tme as u8,
                (at >> 6) & 1,
                if tme { tex0 & 0xffff_ffff_ffff } else { 0 },
                if (at >> 6) & 1 != 0 { self.r[0x42 + c] } else { 0 },
                test,
                if (test >> 16) & 7 == 5 { vs[0].z >> 16 } else { 0 }
            );
            self.stat(k);
        }
        if self.log && std::env::var("ORACLE_GSTRACE").is_ok_and(|t| u64::from_str_radix(&t, 16) == Ok(if tme { tex0 & 0xffff_ffff_ffff } else { test })) {
            eprintln!("trace ty {ty} at {at:x} frame {frame:x} xyoff {:x} sc {:x} clamp {:x} tex1 {:x} vs {vs:?}", self.r[0x18 + c], self.r[0x40 + c], self.r[0x08 + c], self.r[0x14 + c]);
        }
        if self.log && std::env::var("ORACLE_GSSEQ").is_ok() && ty == 6 && fbp == 0x70 && !tme && test == 0x30000 {
            eprintln!("seq f{} big sprite fbp {fbp:x} tme {} tex0 {:x} test {:x} alpha {:x} rgba {:?} copy {copy}", self.frames, tme as u8, tex0, test, self.r[0x42 + c], vs[1].rgba);
        }
        let ov = Some(fbp) == self.main_fbp;
        // No-present fields (0x15f5b8 != 0, Eudora OV:54400 [C]): no frame copy, and the field's draws land in the
        // display buffer itself over the last shown picture (`field_end` publishes it). Prims that sample the
        // display buffer read a picture the capture does not have.
        let dsp = self.full || (!ov && !copy && fbp == disp && !self.copied && self.main_fbp.is_some() && !(tme && tbp / 32 == disp));
        if dsp && self.dcur.is_none() {
            self.dcur = Some(if self.gpu.is_some() {
                self.gpu_ov(1, gpu::HELD, gpu::DCUR);
                Vec::new()
            } else {
                // The display picture starts as a copy of `held`, into the buffer the last field end left (no allocation).
                self.dcur_dirty = self.held_dirty;
                let mut d = std::mem::take(&mut self.dspare);
                if d.len() == self.held.len() {
                    d.copy_from_slice(&self.held);
                    d
                } else {
                    self.held.clone()
                }
            });
        }
        if dsp && std::env::var("RAC_GSDISP").is_ok() {
            eprintln!("gsdisp f{} ty {ty} tme {} abe {} alpha {:x} rgba {:?} xy ({},{})-({},{})", self.frames, tme as u8, (at >> 6) & 1, self.r[0x42 + c], vs[0].rgba, vs[0].x / 16, vs[0].y / 16, vs.get(1).map_or(0, |v| v.x / 16), vs.get(1).map_or(0, |v| v.y / 16));
        }
        if ov && self.log && std::env::var("ORACLE_GSONLY").is_ok_and(|t| u64::from_str_radix(&t, 16) != Ok(tex0 & 0xffff_ffff_ffff)) {
            return;
        }
        if ov {
            let main = self.main_fbp.unwrap() * 32;
            // The frame covers the scissored rows; pages below it are scratch render targets (menu text).
            let rows = ((self.r[0x40 + c] >> 48) & 0x7ff) as u32 / 32 + 1;
            let main_end = main + rows * ((frame >> 16) & 0x3f) as u32 * 32;
            self.main_end = main_end;
            let zte = test & 0x10000 != 0;
            let ztst = (test >> 17) & 3;
            // Prims that sample the frame or test its alpha are screen effects over a frame the capture does
            // not have.
            // Depth-tested prims belong to the 3D scene, except those at the near end of the depth range (the
            // HUD and menus pass GEQUAL with z near the maximum) [I].
            if zte && ztst >= 2 {
                let zmax: u64 = match (self.r[0x4e + c] >> 24) & 0xf {
                    0 => 0xffff_ffff,
                    1 => 0xff_ffff,
                    _ => 0xffff,
                };
                let z = vs.iter().take(match ty { 6 => 2, 3..=5 => 3, 1 | 2 => 2, _ => 1 }).map(|v| v.z as u64).min().unwrap_or(0);
                if z < zmax - zmax / 16 {
                    return;
                }
            }
            // The sky comes first (rac-rs draws its own): Eudora's before any VU1 program, Rilgar's cloud layer after
            // two; the scene's thousands follow.
            if self.mscal < super::SKY_PROGRAMS {
                return;
            }
            if test & 0x4000 != 0 || (tme && tbp >= main && tbp < main_end) {
                return;
            }
            // Colour writes masked off, or a blend that leaves the destination as it was (A = B, D = Cd): no
            // change to the overlay.
            let al = self.r[0x42 + c];
            let pabe = self.r[0x49] & 1 != 0;
            let keep = al & 3 == (al >> 2) & 3 || ((al >> 4) & 3 == 2 && (al >> 32) & 0xff == 0);
            let never = test & 0xf == 1 && (test >> 12) & 3 == 0;
            if never || (frame >> 32) & 0xff_ffff == 0xff_ffff || (at & 0x40 != 0 && !pabe && keep && (al >> 6) & 3 == 1) {
                return;
            }
        } else if !dsp && (fbp == disp || copy) {
            return;
        }
        let xy = self.r[0x18 + c];
        let sc = self.r[0x40 + c];
        if tme {
            let (lo, hi) = match self.sp_t {
                Some((k, v)) if k == tex0 => v,
                _ => {
                    let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
                    let v = Self::span(((tex0 >> 20) & 0x3f) as u32, tbp, ((tex0 >> 14) & 0x3f) as u32, 0, 0, tw, th);
                    self.sp_t = Some((tex0, v));
                    v
                }
            };
            self.flush(lo, hi);
        }
        if !ov && !dsp {
            let (x0, x1, y0, y1) = ((sc & 0x7ff) as u32, ((sc >> 16) & 0x7ff) as u32, ((sc >> 32) & 0x7ff) as u32, ((sc >> 48) & 0x7ff) as u32);
            if x1 >= x0 && y1 >= y0 {
                let (lo, hi) = match self.sp_f {
                    Some((k, v)) if k == (frame, sc) => v,
                    _ => {
                        let v = Self::span(((frame >> 24) & 0x3f) as u32, fbp * 32, ((frame >> 16) & 0x3f) as u32, x0, y0, x1 - x0 + 1, y1 - y0 + 1);
                        self.sp_f = Some(((frame, sc), v));
                        v
                    }
                };
                self.flush(lo, hi);
            }
        }
        let st = St {
            at,
            tex0,
            tex1: self.r[0x14 + c],
            clamp: self.r[0x08 + c],
            alpha: self.r[0x42 + c],
            test,
            frame,
            fba: self.r[0x4a + c],
            ofx: (xy & 0xffff) as f32 / 16.0,
            ofy: ((xy >> 32) & 0xffff) as f32 / 16.0,
            sc: [(sc & 0x7ff) as i32, ((sc >> 16) & 0x7ff) as i32, ((sc >> 32) & 0x7ff) as i32, ((sc >> 48) & 0x7ff) as i32],
            ov: ov || dsp,
            zb: {
                let zbuf = self.r[0x4e + c];
                let zsm = (zbuf >> 24) & 0xf;
                let zbp = ((zbuf & 0x1ff) * 32) as u32;
                // Not when the frame is the Z buffer itself (the Z clears are colour draws into its pages).
                (self.p1.is_some() && !(ov || dsp) && zsm <= 1 && zbp != fbp * 32).then(|| Zb {
                    bp: zbp,
                    bw: ((frame >> 16) & 0x3f) as u32,
                    z24: zsm == 1,
                    msk: (zbuf >> 32) & 1 != 0,
                    tst: if test & 0x10000 != 0 { ((test >> 17) & 3) as u32 } else { 1 },
                })
            },
        };
        self.to_disp = dsp;
        let tr = self.prof.is_some().then(std::time::Instant::now);
        self.raster(&st, ty, vs);
        if let Some(t) = tr {
            self.ras_ns = t.elapsed().as_nanos() as u64;
        }
    }

    fn raster(&mut self, st: &St, ty: u32, vs: &[Vtx]) {
        if self.gpu.is_some() {
            self.gpu_raster(st, ty, vs);
            return;
        }
        self.tcr = (0, 0, 0, 0);
        if st.ov {
            self.mark_prim(st, ty, vs);
        }
        match ty {
            6 => self.sprite(st, vs[0], vs[1]),
            3..=5 => self.tri(st, &vs[..3]),
            1 | 2 => self.line(st, vs[0], vs[1]),
            0 => {
                let p = self.px(st, &vs[0]);
                let (x, y) = (vs[0].x as f32 / 16.0 - st.ofx, vs[0].y as f32 / 16.0 - st.ofy);
                self.view().pixel(st, x.round() as i32, y.round() as i32, &p);
            }
            _ => {}
        }
    }

    /// Rows a prim of the overlay can touch (its vertices' y range, a row of margin each side), into the dirty rows of the
    /// buffer `view` draws into.
    fn mark_prim(&mut self, st: &St, ty: u32, vs: &[Vtx]) {
        let n = match ty {
            6 | 1 | 2 => 2,
            3..=5 => 3,
            0 => 1,
            _ => return,
        };
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for v in &vs[..n.min(vs.len())] {
            let y = v.y as f32 / 16.0 - st.ofy;
            lo = lo.min(y);
            hi = hi.max(y);
        }
        let d = if self.to_disp && self.dcur.is_some() { &mut self.dcur_dirty } else { &mut self.cur_dirty };
        d.mark(lo.floor() as i32 - 1, hi.ceil() as i32 + 1);
    }

    /// Ends the overlay frame: the finished overlay becomes `done`.
    fn publish(&mut self) {
        let t0 = self.prof.is_some().then(std::time::Instant::now);
        let gpu = self.gpu.is_some();
        let mut rows = self.cur.rows;
        if let Some(f) = self.main_fbp {
            rows = ((self.main_end.saturating_sub(f * 32)) / (self.main_bw.max(1) * 32) * 32).clamp(1, self.cur.h);
        }
        rows = rows.min(disp_rows());
        self.held_rows = rows;
        if gpu {
            let blank = Overlay { w: self.cur.w, h: self.cur.h, rows: self.cur.h, frozen: false, disp: false, px: Vec::new(), rgba: Vec::new(), hd: None };
            let mut o = std::mem::replace(&mut self.cur, blank);
            o.rows = rows;
            self.gpu_publish(gpu::CUR, o);
            self.done = None;
        } else {
            // CPU capture. The finished picture becomes `held` by swapping the two buffers (no copy); the old `held` buffer,
            // blanked over its dirty rows only, is the next `cur` (no allocation). The RGBA8 picture the main thread takes is
            // converted from the first `rows` rows here, blank rows being a constant.
            let (w, h) = (self.cur.w, self.cur.h);
            let (frozen, disp) = (self.cur.frozen, self.cur.disp);
            self.cur.frozen = false;
            self.cur.disp = false;
            self.cur.rows = h;
            if self.check {
                self.check_blank("cur", &self.cur.px, &self.cur_dirty, w);
                self.check_blank("held", &self.held, &self.held_dirty, w);
            }
            std::mem::swap(&mut self.cur.px, &mut self.held);
            std::mem::swap(&mut self.cur_dirty, &mut self.held_dirty);
            blank_rows(&mut self.cur.px, w, &self.cur_dirty);
            self.cur_dirty = Rows::default();
            let rgba = to_rgba(&self.held, w, rows, &self.held_dirty);
            if self.check {
                self.check_rgba(&rgba, &self.held, w, rows);
            }
            self.done = Some(Overlay { w, h, rows, frozen, disp, px: Vec::new(), rgba, hd: None });
        }
        if self.log && std::env::var("ORACLE_GSSEQ").is_ok() {
            eprintln!("seq publish {}", self.frames + 1);
        }
        self.mscal = 0;
        self.frames += 1;
        self.prof_add("publish (overlay)", t0);
    }

    /// RAC_GSCAP_CHECK: every row outside `dirty` of `buf` must be the blank pixel.
    fn check_blank(&self, what: &str, buf: &[[f32; 4]], dirty: &Rows, w: u32) {
        let bad = buf.chunks(w as usize).enumerate().filter(|(y, r)| !dirty.has(*y as i32) && r.iter().any(|p| *p != BLANK)).count();
        if bad != 0 {
            eprintln!("gscap check: {bad} rows of {what} outside the dirty rows are not blank (publish {})", self.frames);
        }
    }

    /// RAC_GSCAP_CHECK: the old conversion (every pixel of the picture normalised, then rounded) gives the same bytes.
    fn check_rgba(&self, rgba: &[[u8; 4]], pic: &[[f32; 4]], w: u32, rows: u32) {
        let n = (w * rows) as usize;
        let bad = if rgba.len() != n {
            n.max(1)
        } else {
            pic[..n]
                .iter()
                .zip(rgba)
                .filter(|(p, r)| {
                    let mut p = **p;
                    for k in 0..3 {
                        p[k] = (p[k] / 255.0).clamp(0.0, 1.0);
                    }
                    p.map(|c| (c * 255.0 + 0.5) as u8) != **r
                })
                .count()
        };
        if bad != 0 {
            eprintln!("gscap check: {bad} RGBA8 pixels differ from the whole-picture conversion (publish {})", self.frames);
        }
    }

    /// The field ended (retail.rs worker marker): a display-buffer picture drawn this field becomes `done`, and it
    /// is what the display shows until the next copy.
    pub(super) fn field_end(&mut self) {
        self.copied = false;
        self.to_disp = false;
        let Some(px) = self.dcur.take() else { return };
        let t0 = self.prof.is_some().then(std::time::Instant::now);
        if self.gpu.is_some() {
            let o = gpu::meta(self.cur.w, self.cur.h, self.held_rows, false, true);
            self.gpu_publish(gpu::DCUR, o);
            return;
        }
        // CPU capture: the field's picture becomes `held` by a swap (no copy), the old buffer waits for the next `dcur`.
        let (w, h) = (self.cur.w, self.cur.h);
        let rows = self.held_rows.min(disp_rows());
        if self.check {
            self.check_blank("dcur", &px, &self.dcur_dirty, w);
        }
        self.dspare = std::mem::replace(&mut self.held, px);
        self.held_dirty = self.dcur_dirty;
        let rgba = to_rgba(&self.held, w, rows, &self.held_dirty);
        if self.check {
            self.check_rgba(&rgba, &self.held, w, rows);
        }
        self.done = Some(Overlay { w, h, rows, frozen: false, disp: true, px: Vec::new(), rgba, hd: None });
        if std::env::var("RAC_GSDISP").is_ok() {
            eprintln!("gsdisp publish f{}", self.frames);
        }
        self.prof_add("field_end (display overlay)", t0);
    }

    /// The movie player's picture is in the display buffer now (FUN_0028a1c8 path, rac-rs presents it): a later
    /// draw into the display buffer (the movie-end fade) goes over a clear overlay, so the presented movie shows through.
    pub(super) fn movie_shown(&mut self) {
        if self.gpu.is_some() {
            self.gpu_fill(gpu::HELD);
        } else {
            self.held.fill([0.0, 0.0, 0.0, 1.0]);
            self.held_dirty = Rows::default();
        }
        self.dcur = None;
    }

    fn px(&self, st: &St, v: &Vtx) -> Px {
        let fst = st.at & 0x100 != 0;
        Px {
            rgba: v.rgba.map(|c| c as f32),
            s: v.s,
            t: v.t,
            q: v.q,
            u: if fst { v.u as f32 / 16.0 } else { 0.0 },
            v: if fst { v.v as f32 / 16.0 } else { 0.0 },
            f: v.f as f32,
            z: v.z as f32,
        }
    }

    /// What one prim's pixels read and write (raw pointers into VRAM, the overlay and the CLUT, plus the registers the
    /// pixel pipeline reads), so the rows of a big prim can be drawn on several threads (`par_ok` says when that
    /// is safe). PC speed-up of the capture, no retail counterpart; every pixel runs the same code either way.
    fn view(&mut self) -> View {
        View {
            vram: self.vram.as_mut_ptr(),
            vram_len: self.vram.len(),
            px: match (self.to_disp, self.dcur.as_mut()) {
                (true, Some(d)) => d.as_mut_ptr(),
                _ => self.cur.px.as_mut_ptr(),
            },
            w: self.cur.w as i32,
            h: self.cur.h as i32,
            clut: self.clut.as_ptr(),
            clut_off: self.clut_off,
            texa: self.r[0x3b],
            fogcol: self.r[0x3d],
            colclamp: self.r[0x46],
            pabe: self.r[0x49],
            tc: self.tcache.as_ptr(),
            tcx: self.tcr.0,
            tcy: self.tcr.1,
            tcw: self.tcr.2,
            tch: self.tcr.3,
        }
    }

    /// Decodes the texels a textured sprite / triangle can fetch (its vertices' texture range, a margin for the bilinear
    /// taps and the rounding of the interpolation) once, so every pixel's fetches are table reads. The table holds what
    /// `View::texel_slow` returns, which a fetch outside it still calls, so the picture is the same; PC speed-up of the
    /// capture, no retail counterpart. Skipped when the prim draws into the texture's own blocks (the texels change while it
    /// draws) or the range is far bigger than the prim.
    fn tex_cache(&mut self, st: &St, pts: &[Px], dest: (i32, i32, i32, i32)) {
        self.tcr = (0, 0, 0, 0);
        if st.at & 0x10 == 0 {
            return;
        }
        let tex0 = st.tex0;
        let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
        let fst = st.at & 0x100 != 0;
        let (mut u0, mut u1, mut v0, mut v1) = (f32::MAX, f32::MIN, f32::MAX, f32::MIN);
        for p in pts {
            let (u, v) = if fst {
                (p.u, p.v)
            } else {
                let q = if p.q != 0.0 { p.q } else { 1.0 };
                (p.s / q * tw as f32, p.t / q * th as f32)
            };
            if !(u.abs() < 1e6 && v.abs() < 1e6) {
                return;
            }
            u0 = u0.min(u);
            u1 = u1.max(u);
            v0 = v0.min(v);
            v1 = v1.max(v);
        }
        let (x0, x1, y0, y1) = (u0.floor() as i32 - 2, u1.floor() as i32 + 3, v0.floor() as i32 - 2, v1.floor() as i32 + 3);
        let area = (dest.1 - dest.0 + 1).max(0) as i64 * (dest.3 - dest.2 + 1).max(0) as i64;
        let n = (x1 - x0) as i64 * (y1 - y0) as i64;
        if n <= 0 || n > (4 * area).max(2048) || dest.0 < 0 || dest.2 < 0 || dest.1 < dest.0 || dest.3 < dest.2 {
            return;
        }
        if !st.ov {
            let frame = st.frame;
            let (fbp, fbw, psm) = (((frame & 0x1ff) * 32) as u32, ((frame >> 16) & 0x3f) as u32, ((frame >> 24) & 0x3f) as u32);
            let (tl, thi) = Self::span(((tex0 >> 20) & 0x3f) as u32, (tex0 & 0x3fff) as u32, ((tex0 >> 14) & 0x3f) as u32, 0, 0, tw, th);
            let (fl, fh) = Self::span(psm, fbp, fbw, dest.0 as u32, dest.2 as u32, (dest.1 - dest.0 + 1) as u32, (dest.3 - dest.2 + 1) as u32);
            if (tl < fh && fl < thi) || fh == u32::MAX {
                return;
            }
        }
        let tprof = self.prof.is_some().then(std::time::Instant::now);
        let v = self.view();
        let w = (x1 - x0) as usize;
        self.tcache.resize(n as usize, [0.0; 4]);
        let fill = |r: usize, row: &mut [[f32; 4]]| {
            v.decode_row(st, x0, y0 + r as i32, row);
        };
        if n >= 16384 {
            on_pool(|| self.tcache.par_chunks_mut(w).enumerate().for_each(|(r, row)| fill(r, row)));
        } else {
            self.tcache.chunks_mut(w).enumerate().for_each(|(r, row)| fill(r, row));
        }
        self.tcr = (x0, y0, w as u32, (y1 - y0) as u32);
        self.prof_add("tex_cache decode", tprof);
        self.prof_add_ns("tex_cache texels", n as u64);
    }

    /// Whether the rows `y0..=y1` (columns `x0..=x1`) of this prim can be drawn in parallel: no two rows touch the
    /// same bytes and no pixel reads what another row writes.
    fn par_ok(&self, st: &St, x0: i32, x1: i32, y0: i32, y1: i32) -> bool {
        if x1 < x0 || y1 <= y0 || ((x1 - x0 + 1) as i64) * ((y1 - y0 + 1) as i64) < PAR_MIN || x0 < 0 || y0 < 0 {
            return false;
        }
        if st.ov {
            return true;
        }
        let frame = st.frame;
        let (fbp, fbw, psm) = (((frame & 0x1ff) * 32) as u32, ((frame >> 16) & 0x3f) as u32, ((frame >> 24) & 0x3f) as u32);
        // Whole bytes per pixel only (PSMT4 shares a byte between two pixels), rows inside the buffer width (wider
        // x wraps into another page row) and no wrap at the end of local memory.
        if !matches!(psm, 0x00 | 0x01 | 0x02 | 0x13 | 0x1b | 0x24 | 0x2c) || fbw == 0 || x1 as u32 >= fbw * 64 {
            return false;
        }
        if fbp + (y1 as u32 / 32 + 1) * fbw * 32 > 16384 {
            return false;
        }
        if st.at & 0x10 != 0 {
            let cl = st.clamp;
            if cl & 3 >= 2 || (cl >> 2) & 3 >= 2 {
                return false;
            }
            let tex0 = st.tex0;
            let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
            let (tl, th) = Self::span(((tex0 >> 20) & 0x3f) as u32, (tex0 & 0x3fff) as u32, ((tex0 >> 14) & 0x3f) as u32, 0, 0, tw, th);
            let (fl, fh) = Self::span(psm, fbp, fbw, x0 as u32, y0 as u32, (x1 - x0 + 1) as u32, (y1 - y0 + 1) as u32);
            if tl < fh && fl < th {
                return false;
            }
        }
        true
    }

    fn sprite(&mut self, st: &St, a: Vtx, b: Vtx) {
        let (x0, y0) = (a.x as f32 / 16.0 - st.ofx, a.y as f32 / 16.0 - st.ofy);
        let (x1, y1) = (b.x as f32 / 16.0 - st.ofx, b.y as f32 / 16.0 - st.ofy);
        let (pa, pb) = (self.px(st, &a), self.px(st, &b));
        let (lx, hx) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
        let (ly, hy) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
        let (ix0, ix1) = ((lx.ceil() as i32).max(st.sc[0]), (hx.ceil() as i32).min(st.sc[1] + 1));
        let (iy0, iy1) = ((ly.ceil() as i32).max(st.sc[2]), (hy.ceil() as i32).min(st.sc[3] + 1));
        let fst = st.at & 0x100 != 0;
        let tok = if st.zb.is_some() { pmemo::Tok::None } else { self.pm_begin(st, 6, &[a, b], (ix0, ix1, iy0, iy1)) };
        if matches!(tok, pmemo::Tok::Hit) {
            return;
        }
        let par = self.par_ok(st, ix0, ix1 - 1, iy0, iy1 - 1);
        self.tex_cache(st, &[pa, pb], (ix0, ix1 - 1, iy0, iy1 - 1));
        let v = self.view();
        if st.at & 0x10 == 0 {
            // Untextured: every pixel has the last vertex's colour and fog (the interpolated coordinates are not read), so
            // the colour pipeline runs once.
            if let Some((c, wa)) = v.shade(st, &pb) {
                let pt = Pt::new(st, &v);
                let row = |y: i32| {
                    for x in ix0.max(0)..ix1 {
                        v.put_h(st, &pt, x, y, &c, wa, pb.z);
                    }
                };
                if par {
                    par_rows(iy0.max(0)..iy1, row);
                } else {
                    (iy0.max(0)..iy1).for_each(row);
                }
            }
            self.pm_end(tok);
            return;
        }
        // Textured. The fragment colour, fog factor and Z are the last vertex's; only the texture coordinates vary, u (or s)
        // with x alone and v (or t) with y alone, so they are computed once per column and per row with the same operations
        // the per-pixel interpolation used (`pa + (pb - pa) * t`, then for ST `s / q * tw`), and the texel fetch indices
        // follow from them.
        let sh = Sh::new(st, &v);
        let pt = Pt::new(st, &v);
        let q = if pb.q != 0.0 { pb.q } else { 1.0 };
        let coord = |p: i32, lo: f32, hi: f32, a: f32, b: f32, size: f32| -> (i32, f32) {
            let t = if hi != lo { (p as f32 - lo) / (hi - lo) } else { 0.0 };
            let w = a + (b - a) * t;
            let u = if fst { w } else { w / q * size };
            if sh.bil {
                let u = u - 0.5;
                let f = ffloor(u);
                (f as i32, u - f)
            } else {
                (ffloor(u) as i32, 0.0)
            }
        };
        let (ua, ub, va, vb) = if fst { (pa.u, pb.u, pa.v, pb.v) } else { (pa.s, pb.s, pa.t, pb.t) };
        let cols: Vec<(i32, f32)> = (ix0..ix1).map(|x| coord(x, x0, x1, ua, ub, sh.tw)).collect();
        let rows: Vec<(i32, f32)> = (iy0..iy1).map(|y| coord(y, y0, y1, va, vb, sh.th)).collect();
        let row = |y: i32| {
            if y < 0 {
                return;
            }
            let (iv, fy) = rows[(y - iy0) as usize];
            for x in ix0.max(0)..ix1 {
                let (iu, fx) = cols[(x - ix0) as usize];
                let t = if sh.bil {
                    let t00 = v.texel(st, iu, iv);
                    let t10 = v.texel(st, iu + 1, iv);
                    let t01 = v.texel(st, iu, iv + 1);
                    let t11 = v.texel(st, iu + 1, iv + 1);
                    let mut o = [0.0; 4];
                    for k in 0..4 {
                        let a = t00[k] + (t10[k] - t00[k]) * fx;
                        let b = t01[k] + (t11[k] - t01[k]) * fx;
                        o[k] = a + (b - a) * fy;
                    }
                    o
                } else {
                    v.texel(st, iu, iv)
                };
                let mut c = pb.rgba;
                View::tex_fx(&sh, &t, &mut c);
                if let Some((c, wa)) = v.post(&sh, c, pb.f) {
                    v.put_h(st, &pt, x, y, &c, wa, pb.z);
                }
            }
        };
        if par {
            par_rows(iy0..iy1, row);
        } else {
            (iy0..iy1).for_each(row);
        }
        self.pm_end(tok);
    }

    fn tri(&mut self, st: &St, v: &[Vtx]) {
        let p: [(f32, f32); 3] = std::array::from_fn(|i| (v[i].x as f32 / 16.0 - st.ofx, v[i].y as f32 / 16.0 - st.ofy));
        let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
        if area == 0.0 {
            return;
        }
        let pxs: [Px; 3] = std::array::from_fn(|i| self.px(st, &v[i]));
        let iip = st.at & 8 != 0;
        let minx = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).ceil().max(st.sc[0] as f32) as i32;
        let maxx = p.iter().map(|q| q.0).fold(f32::MIN, f32::max).floor().min(st.sc[1] as f32) as i32;
        let miny = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).ceil().max(st.sc[2] as f32) as i32;
        let maxy = p.iter().map(|q| q.1).fold(f32::MIN, f32::max).floor().min(st.sc[3] as f32) as i32;
        let tok = if st.zb.is_some() { pmemo::Tok::None } else { self.pm_begin(st, 4, &v[..3], (minx, maxx + 1, miny, maxy + 1)) };
        if matches!(tok, pmemo::Tok::Hit) {
            return;
        }
        let par = self.par_ok(st, minx, maxx, miny, maxy);
        self.tex_cache(st, &pxs, (minx, maxx, miny, maxy));
        let vw = self.view();
        let (sh, pt) = (Sh::new(st, &vw), Pt::new(st, &vw));
        let (fst, fog, zon) = (st.at & 0x100 != 0, st.at & 0x20 != 0, st.zb.is_some());
        let neg = area < 0.0;
        let row = |y: i32| {
            let fy = y as f32;
            for x in minx..=maxx {
                let fx = x as f32;
                // w0 = n0 / area is negative exactly when n0 is nonzero with the sign opposite to the area's (no
                // underflow: the numerators are multiples of 2^-8 or zero), so the outside pixels skip the division.
                let n0 = (p[1].0 - fx) * (p[2].1 - fy) - (p[2].0 - fx) * (p[1].1 - fy);
                if n0 != 0.0 && n0.is_sign_negative() != neg {
                    continue;
                }
                let n1 = (p[2].0 - fx) * (p[0].1 - fy) - (p[0].0 - fx) * (p[2].1 - fy);
                if n1 != 0.0 && n1.is_sign_negative() != neg {
                    continue;
                }
                let w0 = n0 / area;
                let w1 = n1 / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                // The same sums as the oracle's mix (w0 * a + w1 * b + w2 * c, left to right); the fields the pixel
                // does not read (the other coordinate set, the fog factor when there is no fog) are left out.
                let mix = |a: f32, b: f32, c: f32| w0 * a + w1 * b + w2 * c;
                let mut q = pxs[2];
                if iip {
                    for k in 0..4 {
                        q.rgba[k] = mix(pxs[0].rgba[k], pxs[1].rgba[k], pxs[2].rgba[k]);
                    }
                }
                if fst {
                    q.u = mix(pxs[0].u, pxs[1].u, pxs[2].u);
                    q.v = mix(pxs[0].v, pxs[1].v, pxs[2].v);
                } else {
                    q.s = mix(pxs[0].s, pxs[1].s, pxs[2].s);
                    q.t = mix(pxs[0].t, pxs[1].t, pxs[2].t);
                    q.q = mix(pxs[0].q, pxs[1].q, pxs[2].q);
                }
                if fog {
                    q.f = mix(pxs[0].f, pxs[1].f, pxs[2].f);
                }
                if zon {
                    q.z = mix(pxs[0].z, pxs[1].z, pxs[2].z);
                }
                vw.pixel_h(st, &sh, &pt, x, y, &q);
            }
        };
        if par {
            par_rows(miny..maxy + 1, row);
        } else {
            (miny..=maxy).for_each(row);
        }
        self.pm_end(tok);
    }

    fn line(&mut self, st: &St, a: Vtx, b: Vtx) {
        let (x0, y0) = (a.x as f32 / 16.0 - st.ofx, a.y as f32 / 16.0 - st.ofy);
        let (x1, y1) = (b.x as f32 / 16.0 - st.ofx, b.y as f32 / 16.0 - st.ofy);
        let n = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
        let (pa, pb) = (self.px(st, &a), self.px(st, &b));
        let v = self.view();
        let (sh, pt) = (Sh::new(st, &v), Pt::new(st, &v));
        for i in 0..n {
            let t = i as f32 / n as f32;
            let mut p = pb;
            if st.at & 8 != 0 {
                for k in 0..4 {
                    p.rgba[k] = pa.rgba[k] + (pb.rgba[k] - pa.rgba[k]) * t;
                }
            }
            let (x, y) = ((x0 + (x1 - x0) * t).round() as i32, (y0 + (y1 - y0) * t).round() as i32);
            if x >= st.sc[0] && x <= st.sc[1] && y >= st.sc[2] && y <= st.sc[3] {
                v.pixel_h(st, &sh, &pt, x, y, &p);
            }
        }
    }

    pub(super) fn unpack_col(psm: u32, v: u32, texa: u64) -> [f32; 4] {
        match psm {
            0x02 | 0x0a => {
                let c = |s: u32| (((v >> s) & 0x1f) << 3) as f32;
                let rgb0 = v & 0x7fff == 0;
                let a = if v & 0x8000 != 0 { (texa >> 32) & 0xff } else if texa & 0x8000 != 0 && rgb0 { 0 } else { texa & 0xff };
                [c(0), c(5), c(10), a as f32]
            }
            0x01 => {
                let a = if texa & 0x8000 != 0 && v & 0xff_ffff == 0 { 0 } else { texa & 0xff };
                [(v & 0xff) as f32, ((v >> 8) & 0xff) as f32, ((v >> 16) & 0xff) as f32, a as f32]
            }
            _ => [(v & 0xff) as f32, ((v >> 8) & 0xff) as f32, ((v >> 16) & 0xff) as f32, (v >> 24) as f32],
        }
    }

    /// CLUT load on a TEX0/TEX2 write (CLD), CSM1 only.
    pub(super) fn clut_load(&mut self, tex0: u64) {
        let psm = ((tex0 >> 20) & 0x3f) as u32;
        let (cbp, cpsm, csa, cld) = (((tex0 >> 37) & 0x3fff) as u32, ((tex0 >> 51) & 0xf) as u32, ((tex0 >> 56) & 0x1f) as usize, (tex0 >> 61) & 7);
        let t4 = matches!(psm, 0x14 | 0x24 | 0x2c);
        if !t4 && !matches!(psm, 0x13 | 0x1b) {
            return;
        }
        let load = match cld {
            1..=3 => true,
            4 => cbp != self.cbp0,
            5 => cbp != self.cbp1,
            _ => false,
        };
        if cld == 2 || cld == 4 {
            self.cbp0 = cbp;
        }
        if cld == 3 || cld == 5 {
            self.cbp1 = cbp;
        }
        let cfmt = if cpsm & 2 != 0 { 0x02 } else { 0x00 };
        if load {
            self.flush(cbp, cbp + 4);
        }
        let texa = self.r[0x3b];
        if t4 {
            let off = if cfmt == 2 { csa * 16 } else { (csa & 15) * 16 };
            self.clut_off = off;
            if load && self.gpu.is_some() {
                self.gpu_clut(cbp, cfmt, true, off);
            } else if load {
                for i in 0..16u32 {
                    let raw = self.rd_px(cfmt, cbp, 1, i % 8, i / 8);
                    self.clut[off + i as usize] = Self::unpack_col(cfmt, raw, texa);
                }
            }
        } else if load && self.gpu.is_some() {
            self.gpu_clut(cbp, cfmt, false, 0);
        } else if load {
            for i in 0..256u32 {
                let p = (i & 0xe7) | ((i & 8) << 1) | ((i & 0x10) >> 1);
                let raw = self.rd_px(cfmt, cbp, 1, p % 16, p / 16);
                self.clut[i as usize] = Self::unpack_col(cfmt, raw, texa);
            }
        }
    }
}

/// Which rows of a picture buffer may differ from the blank pixel [0, 0, 0, 1]: one bit per row.
#[derive(Clone, Copy, Default)]
pub(super) struct Rows([u64; (super::H as usize).div_ceil(64)]);

impl Rows {
    /// Marks rows `lo..=hi` (clamped to the picture).
    pub(super) fn mark(&mut self, lo: i32, hi: i32) {
        let (lo, hi) = (lo.max(0), hi.min(super::H as i32 - 1));
        for y in lo..=hi {
            self.0[y as usize / 64] |= 1 << (y % 64);
        }
    }

    fn has(&self, y: i32) -> bool {
        self.0[y as usize / 64] >> (y % 64) & 1 != 0
    }
}

/// The overlay pixel of an untouched row.
const BLANK: [f32; 4] = [0.0, 0.0, 0.0, 1.0];

/// Blanks the dirty rows of a picture buffer.
fn blank_rows(buf: &mut [[f32; 4]], w: u32, dirty: &Rows) {
    for (y, r) in buf.chunks_mut(w as usize).enumerate() {
        if dirty.has(y as i32) {
            r.fill(BLANK);
        }
    }
}

/// One premultiplied overlay pixel as RGBA8: the colour normalised (/ 255, clamped to 0..1), then x 255 + 0.5 rounded down;
/// alpha is already 0..1.
#[inline]
fn px8(p: &[f32; 4]) -> [u8; 4] {
    let n = |c: f32| ((c / 255.0).clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    [n(p[0]), n(p[1]), n(p[2]), (p[3] * 255.0 + 0.5) as u8]
}

/// The first `rows` rows of a picture as RGBA8; the rows that are not dirty are blank and become the constant.
fn to_rgba(pic: &[[f32; 4]], w: u32, rows: u32, dirty: &Rows) -> Vec<[u8; 4]> {
    let w = w as usize;
    let blank = px8(&BLANK);
    let mut out = Vec::with_capacity(w * rows as usize);
    for (y, r) in pic.chunks(w).take(rows as usize).enumerate() {
        if dirty.has(y as i32) {
            out.extend(r.iter().map(px8));
        } else {
            out.resize(out.len() + w, blank);
        }
    }
    out
}

/// A prim's shade state (texture, TFX, fog, alpha test) decoded once from the registers, so the per-pixel code only
/// reads it. The values are the ones `shade` derived for every pixel before.
#[derive(Clone, Copy)]
struct Sh {
    tex: bool,
    fst: bool,
    bil: bool,
    /// The texture's width and height as floats.
    tw: f32,
    th: f32,
    tcc: bool,
    tfx: u64,
    fog: bool,
    fogc: [f32; 3],
    ate: bool,
    aref: f32,
    atst: u64,
    afail: u64,
}

impl Sh {
    fn new(st: &St, v: &View) -> Sh {
        let (at, tex0, test) = (st.at, st.tex0, st.test);
        let fc = v.fogcol;
        Sh {
            tex: at & 0x10 != 0,
            fst: at & 0x100 != 0,
            bil: st.tex1 & 0x20 != 0,
            tw: (1u32 << ((tex0 >> 26) & 0xf).min(10)) as f32,
            th: (1u32 << ((tex0 >> 30) & 0xf).min(10)) as f32,
            tcc: tex0 & (1 << 34) != 0,
            tfx: (tex0 >> 35) & 3,
            fog: at & 0x20 != 0,
            fogc: std::array::from_fn(|k| ((fc >> (8 * k)) & 0xff) as f32),
            ate: test & 1 != 0,
            aref: ((test >> 4) & 0xff) as f32,
            atst: (test >> 1) & 7,
            afail: (test >> 12) & 3,
        }
    }
}

/// A prim's blend state, decoded once (see `Sh`).
#[derive(Clone, Copy)]
struct Pt {
    ov: bool,
    abe: bool,
    /// PABE is on (the blend is skipped for fragments with alpha < 128).
    pabe: bool,
    al: u64,
    fix: f32,
    a: u8,
    b: u8,
    d: u8,
    cfs: u64,
}

impl Pt {
    fn new(st: &St, v: &View) -> Pt {
        let al = st.alpha;
        Pt { ov: st.ov, abe: st.at & 0x40 != 0, pabe: v.pabe & 1 != 0, al, fix: ((al >> 32) & 0xff) as f32, a: (al & 3) as u8, b: ((al >> 2) & 3) as u8, d: ((al >> 6) & 3) as u8, cfs: (al >> 4) & 3 }
    }
}

/// See `Gs::view`.
#[derive(Clone, Copy)]
struct View {
    vram: *mut u8,
    vram_len: usize,
    px: *mut [f32; 4],
    w: i32,
    h: i32,
    clut: *const [f32; 4],
    clut_off: usize,
    texa: u64,
    fogcol: u64,
    colclamp: u64,
    pabe: u64,
    /// The prim's decoded texels (`tex_cache`): `tcw` x `tch` entries for texel coordinates from (`tcx`, `tcy`); `tcw` 0 = none.
    tc: *const [f32; 4],
    tcx: i32,
    tcy: i32,
    tcw: u32,
    tch: u32,
}

// SAFETY: the pointers stay valid for the prim (the Gs is borrowed mutably for it) and rows never share bytes (`par_ok`).
unsafe impl Send for View {}
unsafe impl Sync for View {}

/// The capture's own thread pool (the global one is the renderer's 4 frame workers, one of them the render thread):
/// RAC_GSCAP_THREADS threads, default the hardware threads up to 8.
fn pool() -> Option<&'static rayon::ThreadPool> {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        let n = std::env::var("RAC_GSCAP_THREADS").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or_else(|| std::thread::available_parallelism().map_or(1, |n| n.get()).min(8));
        rayon::ThreadPoolBuilder::new().num_threads(n.max(1)).thread_name(|i| format!("gs-capture-{i}")).build().ok()
    })
    .as_ref()
}

/// Runs `f` on the capture's pool (parallel iterators inside use its threads).
pub(super) fn on_pool<R: Send>(f: impl FnOnce() -> R + Send) -> R {
    match pool() {
        Some(p) => p.install(f),
        None => f(),
    }
}

fn par_rows(rows: std::ops::Range<i32>, row: impl Fn(i32) + Send + Sync) {
    match pool() {
        Some(p) => p.install(|| rows.into_par_iter().with_min_len(4).for_each(row)),
        None => rows.for_each(row),
    }
}

/// Bounding-box pixels from which a prim's rows are drawn on several threads.
const PAR_MIN: i64 = 512;

impl View {
    fn clut(&self, i: usize) -> [f32; 4] {
        assert!(i < 512);
        // SAFETY: the CLUT has 512 entries.
        unsafe { *self.clut.add(i) }
    }

    fn rd<const N: usize>(&self, a: usize) -> [u8; N] {
        assert!(a + N <= self.vram_len);
        let mut b = [0u8; N];
        // SAFETY: in bounds (checked); no other row writes these bytes.
        unsafe { std::ptr::copy_nonoverlapping(self.vram.add(a), b.as_mut_ptr(), N) };
        b
    }

    fn wr(&self, a: usize, b: &[u8]) {
        assert!(a + b.len() <= self.vram_len);
        // SAFETY: as `rd`.
        unsafe { std::ptr::copy_nonoverlapping(b.as_ptr(), self.vram.add(a), b.len()) };
    }

    /// `Gs::rd_px` through the view.
    fn rd_px(&self, psm: u32, bp: u32, bw: u32, x: u32, y: u32) -> u32 {
        match psm {
            0x00 => u32::from_le_bytes(self.rd::<4>(Gs::a32(bp, bw, x, y))),
            0x01 => u32::from_le_bytes(self.rd::<4>(Gs::a32(bp, bw, x, y))) & 0xff_ffff,
            0x02 => u16::from_le_bytes(self.rd::<2>(Gs::a16(bp, bw, x, y))) as u32,
            0x13 => self.rd::<1>(Gs::a8(bp, bw, x, y))[0] as u32,
            0x14 => {
                let n = Gs::a4(bp, bw, x, y);
                (self.rd::<1>(n >> 1)[0] >> ((n & 1) * 4)) as u32 & 0xf
            }
            0x1b => self.rd::<1>(Gs::a32(bp, bw, x, y) + 3)[0] as u32,
            0x24 => self.rd::<1>(Gs::a32(bp, bw, x, y) + 3)[0] as u32 & 0xf,
            0x2c => (self.rd::<1>(Gs::a32(bp, bw, x, y) + 3)[0] >> 4) as u32,
            _ => 0,
        }
    }

    /// `Gs::wr_px` through the view.
    fn wr_px(&self, psm: u32, bp: u32, bw: u32, x: u32, y: u32, v: u32) {
        match psm {
            0x00 => self.wr(Gs::a32(bp, bw, x, y), &v.to_le_bytes()),
            0x01 => self.wr(Gs::a32(bp, bw, x, y), &v.to_le_bytes()[..3]),
            0x02 => self.wr(Gs::a16(bp, bw, x, y), &(v as u16).to_le_bytes()),
            0x13 => self.wr(Gs::a8(bp, bw, x, y), &[v as u8]),
            0x14 => {
                let n = Gs::a4(bp, bw, x, y);
                let b = self.rd::<1>(n >> 1)[0];
                self.wr(n >> 1, &[if n & 1 != 0 { (b & 0x0f) | ((v as u8) << 4) } else { (b & 0xf0) | (v as u8 & 0xf) }]);
            }
            0x1b => self.wr(Gs::a32(bp, bw, x, y) + 3, &[v as u8]),
            0x24 => {
                let a = Gs::a32(bp, bw, x, y) + 3;
                self.wr(a, &[(self.rd::<1>(a)[0] & 0xf0) | (v as u8 & 0xf)]);
            }
            0x2c => {
                let a = Gs::a32(bp, bw, x, y) + 3;
                self.wr(a, &[(self.rd::<1>(a)[0] & 0x0f) | ((v as u8) << 4)]);
            }
            _ => {}
        }
    }

    /// One pixel: `shade`, then `put`.
    fn pixel(&self, st: &St, x: i32, y: i32, p: &Px) {
        if x < 0 || y < 0 {
            return;
        }
        self.pixel_h(st, &Sh::new(st, self), &Pt::new(st, self), x, y, p);
    }

    /// `pixel` with the prim's decoded state (`Sh`, `Pt`).
    #[inline]
    fn pixel_h(&self, st: &St, sh: &Sh, pt: &Pt, x: i32, y: i32, p: &Px) {
        if x < 0 || y < 0 {
            return;
        }
        if let Some((c, wa)) = self.shade_h(st, sh, p) {
            self.put_h(st, pt, x, y, &c, wa, p.z);
        }
    }

    /// Texture, TFX, fog and alpha test of one fragment: its colour and whether the alpha is written; None when the
    /// alpha test drops it.
    fn shade(&self, st: &St, p: &Px) -> Option<([f32; 4], bool)> {
        self.shade_h(st, &Sh::new(st, self), p)
    }

    #[inline]
    fn shade_h(&self, st: &St, sh: &Sh, p: &Px) -> Option<([f32; 4], bool)> {
        let mut c = p.rgba;
        if sh.tex {
            let (u, v) = if sh.fst {
                (p.u, p.v)
            } else {
                let q = if p.q != 0.0 { p.q } else { 1.0 };
                (p.s / q * sh.tw, p.t / q * sh.th)
            };
            let t = if sh.bil { self.bilinear(st, u - 0.5, v - 0.5) } else { self.texel(st, ffloor(u) as i32, ffloor(v) as i32) };
            Self::tex_fx(sh, &t, &mut c);
        }
        self.post(sh, c, p.f)
    }

    /// The TFX combine of a fetched texel `t` with the fragment colour `c`.
    #[inline(always)]
    fn tex_fx(sh: &Sh, t: &[f32; 4], c: &mut [f32; 4]) {
        let tcc = sh.tcc;
        match sh.tfx {
            0 => {
                for k in 0..3 {
                    c[k] = (t[k] * c[k] / 128.0).min(255.0);
                }
                c[3] = if tcc { (t[3] * c[3] / 128.0).min(255.0) } else { c[3] };
            }
            1 => {
                for k in 0..3 {
                    c[k] = t[k];
                }
                if tcc {
                    c[3] = t[3];
                }
            }
            tfx => {
                let a = c[3];
                for k in 0..3 {
                    c[k] = (t[k] * c[k] / 128.0 + a).min(255.0);
                }
                c[3] = if tcc { if tfx == 2 { (t[3] + a).min(255.0) } else { t[3] } } else { a };
            }
        }
    }

    /// Fog and alpha test of a fragment colour (`f` the fog factor).
    #[inline(always)]
    fn post(&self, sh: &Sh, mut c: [f32; 4], pf: f32) -> Option<([f32; 4], bool)> {
        if sh.fog {
            let f = pf / 255.0;
            for k in 0..3 {
                c[k] = c[k] * f + sh.fogc[k] * (1.0 - f);
            }
        }
        // Alpha test.
        let mut wa = true;
        if sh.ate {
            let aref = sh.aref;
            let a = ffloor(c[3]);
            let pass = match sh.atst {
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
                match sh.afail {
                    0 | 2 => return None,
                    1 => {}
                    _ => wa = false,
                }
            }
        }
        Some((c, wa))
    }

    /// Blend and write of a shaded fragment at (x, y), x and y >= 0.
    #[inline]
    fn put_h(&self, st: &St, pt: &Pt, x: i32, y: i32, c: &[f32; 4], wa: bool, z: f32) {
        let c = *c;
        let al = pt.al;
        let fix = pt.fix;
        let pabe = pt.pabe && c[3] < 128.0;
        let blend = pt.abe && !pabe;
        if pt.ov {
            let (w, h) = (self.w, self.h);
            if x >= w || y >= h {
                return;
            }
            // SAFETY: (x, y) is inside the w x h overlay and only this prim row writes it (`par_ok`).
            let o = unsafe { &mut *self.px.add((y * w + x) as usize) };
            if !blend {
                *o = [c[0], c[1], c[2], 0.0];
                return;
            }
            let (a, b, d) = (pt.a, pt.b, pt.d);
            let cf = match pt.cfs {
                0 => c[3] / 128.0,
                1 => 1.0, // destination alpha: the overlay does not know the scene's alpha [I]
                _ => fix / 128.0,
            };
            // out = (A - B) * C + D, linear in Cd: out = ks * Cs + kd * Cd.
            let coef = |which: u8| -> f32 {
                (if a == which { cf } else { 0.0 }) - (if b == which { cf } else { 0.0 }) + (if d == which { 1.0 } else { 0.0 })
            };
            let (ks, kd) = (coef(0), coef(1));
            // Subtractive prims (ALPHA 0x42) are drawn by the viewer itself (queue_b::screen_quads) [C: weapon_fx.md stage 12].
            if ks < 0.0 {
                return;
            }
            for k in 0..3 {
                o[k] = ks * c[k] + kd * o[k];
            }
            o[3] *= kd;
            return;
        }
        // VRAM target.
        let mut zw = None;
        if let Some(zb) = &st.zb {
            let zi = (z.max(0.0) as u32).min(if zb.z24 { 0xff_ffff } else { u32::MAX });
            let a = zb.addr(x as u32, y as u32);
            let old = u32::from_le_bytes(self.rd::<4>(a)) & if zb.z24 { 0xff_ffff } else { u32::MAX };
            let pass = match zb.tst {
                0 => false,
                1 => true,
                2 => zi >= old,
                _ => zi > old,
            };
            if !pass {
                return;
            }
            if !zb.msk && wa {
                zw = Some((a, zi, if zb.z24 { 3 } else { 4 }));
            }
        }
        let frame = st.frame;
        let (fbp, fbw, psm) = (((frame & 0x1ff) * 32) as u32, ((frame >> 16) & 0x3f) as u32, ((frame >> 24) & 0x3f) as u32);
        let msk = (frame >> 32) as u32;
        let dst = self.rd_px(psm, fbp, fbw, x as u32, y as u32);
        let dc = Gs::unpack_col(psm, dst, self.texa);
        let mut out = c;
        if blend {
            let pick = |s: u64, cs: f32, cd: f32| match s & 3 {
                0 => cs,
                1 => cd,
                _ => 0.0,
            };
            let cf = match (al >> 4) & 3 {
                0 => c[3],
                1 => dc[3],
                _ => fix,
            };
            for k in 0..3 {
                let v = (pick(al, c[k], dc[k]) - pick(al >> 2, c[k], dc[k])) * cf / 128.0 + pick(al >> 6, c[k], dc[k]);
                out[k] = if self.colclamp & 1 != 0 { v.clamp(0.0, 255.0) } else { (v as i32 & 0xff) as f32 };
            }
        }
        let mut a = if wa { out[3] } else { dc[3] };
        if st.fba & 1 != 0 {
            a = a.max(128.0);
        }
        let v = match psm {
            0x02 | 0x0a => {
                let b5 = |f: f32| (f.clamp(0.0, 255.0) as u32) >> 3;
                b5(out[0]) | b5(out[1]) << 5 | b5(out[2]) << 10 | ((a >= 128.0) as u32) << 15
            }
            _ => {
                let b8 = |f: f32| f.clamp(0.0, 255.0) as u32;
                b8(out[0]) | b8(out[1]) << 8 | b8(out[2]) << 16 | b8(a) << 24
            }
        };
        let v = (v & !msk) | (dst & msk);
        self.wr_px(psm, fbp, fbw, x as u32, y as u32, v);
        if let Some((a, zi, n)) = zw {
            self.wr(a, &zi.to_le_bytes()[..n]);
        }
    }

    fn wrap(m: u64, c: i32, size: u32, lo: u32, hi: u32) -> u32 {
        match m & 3 {
            0 => (c as u32) & (size - 1),
            1 => c.clamp(0, size as i32 - 1) as u32,
            2 => c.max(lo as i32).min(hi as i32).max(0) as u32,
            _ => ((c as u32) & lo) | hi,
        }
    }

    fn texel(&self, st: &St, u: i32, v: i32) -> [f32; 4] {
        let (cx, cy) = (u.wrapping_sub(self.tcx) as u32, v.wrapping_sub(self.tcy) as u32);
        if cx < self.tcw && cy < self.tch {
            // SAFETY: inside the tcw x tch table `tex_cache` filled.
            return unsafe { *self.tc.add(cy as usize * self.tcw as usize + cx as usize) };
        }
        self.texel_slow(st, u, v)
    }

    fn texel_slow(&self, st: &St, u: i32, v: i32) -> [f32; 4] {
        let tex0 = st.tex0;
        let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
        let cl = st.clamp;
        let x = View::wrap(cl, u, tw, ((cl >> 4) & 0x3ff) as u32, ((cl >> 14) & 0x3ff) as u32);
        let y = View::wrap(cl >> 2, v, th, ((cl >> 24) & 0x3ff) as u32, ((cl >> 34) & 0x3ff) as u32);
        let (tbp, tbw, psm) = ((tex0 & 0x3fff) as u32, ((tex0 >> 14) & 0x3f) as u32, ((tex0 >> 20) & 0x3f) as u32);
        let raw = self.rd_px(psm, tbp, tbw, x, y);
        match psm {
            0x13 | 0x1b => self.clut(raw as usize & 0xff),
            0x14 | 0x24 | 0x2c => self.clut((raw as usize & 0xf) + self.clut_off),
            _ => Gs::unpack_col(psm, raw, self.texa),
        }
    }

    /// `texel_slow(st, x0 + i, y)` for every entry of `out` (a row of `tex_cache`'s table), the per-texel format switch and
    /// address set-up hoisted for CT32 and T8 (the formats of the map's panels and glyphs).
    fn decode_row(&self, st: &St, x0: i32, y: i32, out: &mut [[f32; 4]]) {
        let tex0 = st.tex0;
        let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
        let cl = st.clamp;
        let (tbp, tbw, psm) = ((tex0 & 0x3fff) as u32, ((tex0 >> 14) & 0x3f) as u32, ((tex0 >> 20) & 0x3f) as u32);
        let (ulo, uhi) = (((cl >> 4) & 0x3ff) as u32, ((cl >> 14) & 0x3ff) as u32);
        let yy = View::wrap(cl >> 2, y, th, ((cl >> 24) & 0x3ff) as u32, ((cl >> 34) & 0x3ff) as u32);
        match psm {
            0x00 => {
                let l = &super::luts().0;
                let (base, rowoff) = (tbp * 256 + (yy / 32) * tbw * 8192, (yy % 32) * 64);
                for (i, o) in out.iter_mut().enumerate() {
                    let x = View::wrap(cl, x0 + i as i32, tw, ulo, uhi);
                    let a = ((base + (x / 64) * 8192 + l[(rowoff + x % 64) as usize]) & 0x3f_ffff) as usize;
                    *o = Gs::unpack_col(0, u32::from_le_bytes(self.rd::<4>(a)), self.texa);
                }
            }
            0x13 => {
                let l = &super::luts().2;
                let (base, rowoff) = (tbp * 256 + (yy / 64) * (tbw / 2).max(1) * 8192, (yy % 64) * 128);
                for (i, o) in out.iter_mut().enumerate() {
                    let x = View::wrap(cl, x0 + i as i32, tw, ulo, uhi);
                    let a = ((base + (x / 128) * 8192 + l[(rowoff + x % 128) as usize]) & 0x3f_ffff) as usize;
                    *o = self.clut(self.rd::<1>(a)[0] as usize);
                }
            }
            _ => {
                for (i, o) in out.iter_mut().enumerate() {
                    *o = self.texel_slow(st, x0 + i as i32, y);
                }
            }
        }
    }

    fn bilinear(&self, st: &St, u: f32, v: f32) -> [f32; 4] {
        let (x0, y0) = (ffloor(u), ffloor(v));
        let (fx, fy) = (u - x0, v - y0);
        let (x0, y0) = (x0 as i32, y0 as i32);
        let t00 = self.texel(st, x0, y0);
        let t10 = self.texel(st, x0 + 1, y0);
        let t01 = self.texel(st, x0, y0 + 1);
        let t11 = self.texel(st, x0 + 1, y0 + 1);
        let mut o = [0.0; 4];
        for k in 0..4 {
            let a = t00[k] + (t10[k] - t00[k]) * fx;
            let b = t01[k] + (t11[k] - t01[k]) * fx;
            o[k] = a + (b - a) * fy;
        }
        o
    }

}

/// `f32::floor` without the library call (the baseline x86-64 target has no SSE4.1 rounding): the same value for every
/// input but the sign of a zero result.
#[inline(always)]
fn ffloor(x: f32) -> f32 {
    if x.abs() < 8388608.0 {
        let t = x as i32 as f32;
        t - (t > x) as i32 as f32
    } else {
        x.floor()
    }
}

/// Rows of the display buffer: 512 on PAL, 448 on NTSC (the USA disc scissors its display buffer at 448 rows).
fn disp_rows() -> u32 {
    if ee_oracle::ver::ntsc() { 448 } else { super::H }
}
