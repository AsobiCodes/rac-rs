//! GS pixel pipeline for the captured 2D work: sprite/triangle/line raster, texture fetch (CT32/24/16, T8/T4
//! with CLUT), TFX combine, fog, alpha test and blend. Prims into the main render buffer go to the overlay
//! (premultiplied colour plus scene transmission); prims into other buffers are drawn into VRAM.

use super::{Gs, Overlay, Vtx};

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
}

impl Gs {
    pub(super) fn draw(&mut self, ty: u32, vs: &[Vtx]) {
        let at = self.attrs();
        let c = ((at >> 9) & 1) as usize;
        let frame = self.r[0x4c + c];
        let fbp = (frame & 0x1ff) as u32;
        let tme = at & 0x10 != 0;
        let tex0 = self.r[0x06 + c];
        let tbp = (tex0 & 0x3fff) as u32;
        let test = self.r[0x47 + c];
        let disp = (self.dispfb & 0x1ff) as u32;
        // The main render buffer is learnt from the sprites that copy it to the display buffer; the first strip
        // of each copy ends the frame.
        let copy = fbp == disp && tme && ty == 6 && tbp % 32 == 0 && tbp / 32 != disp;
        if copy {
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
        } else if fbp == disp || copy {
            return;
        }
        let xy = self.r[0x18 + c];
        let sc = self.r[0x40 + c];
        if tme {
            let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
            let (lo, hi) = Self::span(((tex0 >> 20) & 0x3f) as u32, tbp, ((tex0 >> 14) & 0x3f) as u32, 0, 0, tw, th);
            self.flush(lo, hi);
        }
        if !ov {
            let (x0, x1, y0, y1) = ((sc & 0x7ff) as u32, ((sc >> 16) & 0x7ff) as u32, ((sc >> 32) & 0x7ff) as u32, ((sc >> 48) & 0x7ff) as u32);
            if x1 >= x0 && y1 >= y0 {
                let (lo, hi) = Self::span(((frame >> 24) & 0x3f) as u32, fbp * 32, ((frame >> 16) & 0x3f) as u32, x0, y0, x1 - x0 + 1, y1 - y0 + 1);
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
            ov,
        };
        match ty {
            6 => self.sprite(&st, vs[0], vs[1]),
            3..=5 => self.tri(&st, &vs[..3]),
            1 | 2 => self.line(&st, vs[0], vs[1]),
            0 => {
                let p = self.px(&st, &vs[0]);
                let (x, y) = (vs[0].x as f32 / 16.0 - st.ofx, vs[0].y as f32 / 16.0 - st.ofy);
                self.pixel(&st, x.round() as i32, y.round() as i32, &p);
            }
            _ => {}
        }
    }

    /// Ends the overlay frame: the finished overlay becomes `done`.
    fn publish(&mut self) {
        let blank = Overlay { w: self.cur.w, h: self.cur.h, rows: self.cur.h, frozen: false, px: vec![[0.0, 0.0, 0.0, 1.0]; self.cur.px.len()] };
        let mut o = std::mem::replace(&mut self.cur, blank);
        if let Some(f) = self.main_fbp {
            o.rows = ((self.main_end.saturating_sub(f * 32)) / (self.main_bw.max(1) * 32) * 32).clamp(1, o.h);
        }
        for p in &mut o.px {
            for k in 0..3 {
                p[k] = (p[k] / 255.0).clamp(0.0, 1.0);
            }
        }
        self.done = Some(o);
        if self.log && std::env::var("ORACLE_GSSEQ").is_ok() {
            eprintln!("seq publish {}", self.frames + 1);
        }
        self.mscal = 0;
        self.frames += 1;
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
        }
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
        for y in iy0..iy1 {
            let ty = if y1 != y0 { (y as f32 - y0) / (y1 - y0) } else { 0.0 };
            for x in ix0..ix1 {
                let tx = if x1 != x0 { (x as f32 - x0) / (x1 - x0) } else { 0.0 };
                let mut p = pb;
                if fst {
                    p.u = pa.u + (pb.u - pa.u) * tx;
                    p.v = pa.v + (pb.v - pa.v) * ty;
                } else {
                    p.s = pa.s + (pb.s - pa.s) * tx;
                    p.t = pa.t + (pb.t - pa.t) * ty;
                    p.q = pb.q;
                }
                self.pixel(st, x, y, &p);
            }
        }
    }

    fn tri(&mut self, st: &St, v: &[Vtx]) {
        let p: Vec<(f32, f32)> = v.iter().map(|v| (v.x as f32 / 16.0 - st.ofx, v.y as f32 / 16.0 - st.ofy)).collect();
        let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
        if area == 0.0 {
            return;
        }
        let pxs: Vec<Px> = v.iter().map(|v| self.px(st, v)).collect();
        let iip = st.at & 8 != 0;
        let minx = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).ceil().max(st.sc[0] as f32) as i32;
        let maxx = p.iter().map(|q| q.0).fold(f32::MIN, f32::max).floor().min(st.sc[1] as f32) as i32;
        let miny = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).ceil().max(st.sc[2] as f32) as i32;
        let maxy = p.iter().map(|q| q.1).fold(f32::MIN, f32::max).floor().min(st.sc[3] as f32) as i32;
        for y in miny..=maxy {
            for x in minx..=maxx {
                let (fx, fy) = (x as f32, y as f32);
                let w0 = ((p[1].0 - fx) * (p[2].1 - fy) - (p[2].0 - fx) * (p[1].1 - fy)) / area;
                let w1 = ((p[2].0 - fx) * (p[0].1 - fy) - (p[0].0 - fx) * (p[2].1 - fy)) / area;
                let w2 = 1.0 - w0 - w1;
                if w0 < 0.0 || w1 < 0.0 || w2 < 0.0 {
                    continue;
                }
                let w = [w0, w1, w2];
                let mix = |f: &dyn Fn(&Px) -> f32| w[0] * f(&pxs[0]) + w[1] * f(&pxs[1]) + w[2] * f(&pxs[2]);
                let mut q = pxs[2];
                if iip {
                    for k in 0..4 {
                        q.rgba[k] = mix(&|p| p.rgba[k]);
                    }
                }
                q.s = mix(&|p| p.s);
                q.t = mix(&|p| p.t);
                q.q = mix(&|p| p.q);
                q.u = mix(&|p| p.u);
                q.v = mix(&|p| p.v);
                q.f = mix(&|p| p.f);
                self.pixel(st, x, y, &q);
            }
        }
    }

    fn line(&mut self, st: &St, a: Vtx, b: Vtx) {
        let (x0, y0) = (a.x as f32 / 16.0 - st.ofx, a.y as f32 / 16.0 - st.ofy);
        let (x1, y1) = (b.x as f32 / 16.0 - st.ofx, b.y as f32 / 16.0 - st.ofy);
        let n = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
        let (pa, pb) = (self.px(st, &a), self.px(st, &b));
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
                self.pixel(st, x, y, &p);
            }
        }
    }

    /// One pixel through texture, fog, alpha test and blend.
    fn pixel(&mut self, st: &St, x: i32, y: i32, p: &Px) {
        if x < 0 || y < 0 {
            return;
        }
        let at = st.at;
        let mut c = p.rgba;
        if at & 0x10 != 0 {
            let tex0 = st.tex0;
            let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
            let (u, v) = if at & 0x100 != 0 {
                (p.u, p.v)
            } else {
                let q = if p.q != 0.0 { p.q } else { 1.0 };
                (p.s / q * tw as f32, p.t / q * th as f32)
            };
            let t = if st.tex1 & 0x20 != 0 { self.bilinear(st, u - 0.5, v - 0.5) } else { self.texel(st, u.floor() as i32, v.floor() as i32) };
            let tcc = tex0 & (1 << 34) != 0;
            match (tex0 >> 35) & 3 {
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
        if at & 0x20 != 0 {
            let fc = self.r[0x3d];
            let f = p.f / 255.0;
            for k in 0..3 {
                c[k] = c[k] * f + ((fc >> (8 * k)) & 0xff) as f32 * (1.0 - f);
            }
        }
        // Alpha test.
        let test = st.test;
        let mut wa = true;
        if test & 1 != 0 {
            let aref = ((test >> 4) & 0xff) as f32;
            let a = c[3].floor();
            let pass = match (test >> 1) & 7 {
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
                match (test >> 12) & 3 {
                    0 | 2 => return,
                    1 => {}
                    _ => wa = false,
                }
            }
        }
        let abe = at & 0x40 != 0;
        let al = st.alpha;
        let fix = ((al >> 32) & 0xff) as f32;
        let pabe = self.r[0x49] & 1 != 0 && c[3] < 128.0;
        let blend = abe && !pabe;
        if st.ov {
            let (w, h) = (self.cur.w as i32, self.cur.h as i32);
            if x >= w || y >= h {
                return;
            }
            if self.log && std::env::var("ORACLE_GSPIX").is_ok_and(|v| v == format!("{x},{y}")) {
                eprintln!("pix fr {:x} frame {} at {:x} tex0 {:x} test {:x} alpha {:x} c {c:?} blend {blend}", st.frame, self.frames, st.at, st.tex0, st.test, st.alpha);
            }
            let o = &mut self.cur.px[(y * w + x) as usize];
            if !blend {
                *o = [c[0], c[1], c[2], 0.0];
                return;
            }
            let sel = |s: u64| (s & 3) as u8;
            let (a, b, d) = (sel(al), sel(al >> 2), sel(al >> 6));
            let cf = match (al >> 4) & 3 {
                0 => c[3] / 128.0,
                1 => 1.0, // destination alpha: the overlay does not know the scene's alpha [I]
                _ => fix / 128.0,
            };
            // out = (A - B) * C + D, linear in Cd: out = ks * Cs + kd * Cd.
            let coef = |which: u8| -> f32 {
                (if a == which { cf } else { 0.0 }) - (if b == which { cf } else { 0.0 }) + (if d == which { 1.0 } else { 0.0 })
            };
            let (ks, kd) = (coef(0), coef(1));
            if ks < 0.0 && self.ov_skip_sub {
                return;
            }
            for k in 0..3 {
                o[k] = ks * c[k] + kd * o[k];
            }
            o[3] *= kd;
            return;
        }
        // VRAM target.
        let frame = st.frame;
        let (fbp, fbw, psm) = (((frame & 0x1ff) * 32) as u32, ((frame >> 16) & 0x3f) as u32, ((frame >> 24) & 0x3f) as u32);
        let msk = (frame >> 32) as u32;
        let dst = self.rd_px(psm, fbp, fbw, x as u32, y as u32);
        let dc = Self::unpack_col(psm, dst, self.r[0x3b]);
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
                out[k] = if self.r[0x46] & 1 != 0 { v.clamp(0.0, 255.0) } else { (v as i32 & 0xff) as f32 };
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
    }

    fn unpack_col(psm: u32, v: u32, texa: u64) -> [f32; 4] {
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

    fn wrap(m: u64, c: i32, size: u32, lo: u32, hi: u32) -> u32 {
        match m & 3 {
            0 => (c as u32) & (size - 1),
            1 => c.clamp(0, size as i32 - 1) as u32,
            2 => c.max(lo as i32).min(hi as i32).max(0) as u32,
            _ => ((c as u32) & lo) | hi,
        }
    }

    fn texel(&self, st: &St, u: i32, v: i32) -> [f32; 4] {
        let tex0 = st.tex0;
        let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
        let cl = st.clamp;
        let x = Self::wrap(cl, u, tw, ((cl >> 4) & 0x3ff) as u32, ((cl >> 14) & 0x3ff) as u32);
        let y = Self::wrap(cl >> 2, v, th, ((cl >> 24) & 0x3ff) as u32, ((cl >> 34) & 0x3ff) as u32);
        let (tbp, tbw, psm) = ((tex0 & 0x3fff) as u32, ((tex0 >> 14) & 0x3f) as u32, ((tex0 >> 20) & 0x3f) as u32);
        let raw = self.rd_px(psm, tbp, tbw, x, y);
        match psm {
            0x13 | 0x1b => self.clut[raw as usize & 0xff],
            0x14 | 0x24 | 0x2c => self.clut[(raw as usize & 0xf) + self.clut_off],
            _ => Self::unpack_col(psm, raw, self.r[0x3b]),
        }
    }

    fn bilinear(&self, st: &St, u: f32, v: f32) -> [f32; 4] {
        let (x0, y0) = (u.floor(), v.floor());
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
            if load {
                for i in 0..16u32 {
                    let raw = self.rd_px(cfmt, cbp, 1, i % 8, i / 8);
                    self.clut[off + i as usize] = Self::unpack_col(cfmt, raw, texa);
                }
            }
        } else if load {
            for i in 0..256u32 {
                let p = (i & 0xe7) | ((i & 8) << 1) | ((i & 0x10) >> 1);
                let raw = self.rd_px(cfmt, cbp, 1, p % 16, p / 16);
                self.clut[i as usize] = Self::unpack_col(cfmt, raw, texa);
            }
        }
    }
}
