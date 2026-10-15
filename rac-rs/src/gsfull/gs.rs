//! Software GS for the GS-full mode (draws the game's own GIF output: all three paths).
//!
//! Front end (this file, serial): GIF parsing (PACKED / REGLIST / IMAGE), registers, host->local uploads
//! (deferred until something reads them, as in the prototype), local->local copies, CLUT loads, vertex
//! queue and primitive assembly. Everything that touches GS memory is queued (`gs_queue`) and runs on a
//! back-end thread; prims are rasterised in parallel (`gs_raster`). Call `finish` before looking at memory.
//!
//! Semantics are those of the gsfull prototype (<dev>/scratch_gsfull/gsfull/src/gs.rs, see
//! <dev>/scratch_gsfull/GSFULL.md): frame buffers and textures live in the 4 MB local memory with the
//! real swizzles; z buffers are separate 1024x1024 planes keyed by (ZBP, FBW) [I: prototype model, not
//! stored in local memory]. The parallel renderer is proven identical (local memory and z planes, every
//! field) to the prototype on recorded fields: <dev>/scratch_play_gs (`gsbench`, GSBENCH.md).
use super::gs_mem::{luts, put, rd_px, span, trx_advance, unpack_col, wr_px, Trx, VRAM_SIZE};
use super::gs_queue::{Queue, RasterStats};
use super::gs_raster::{Ctx, Prim};
use std::collections::HashMap;

#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Vtx {
    pub x: u16,
    pub y: u16,
    pub z: u32,
    pub rgba: [u8; 4],
    pub q: f32,
    pub s: f32,
    pub t: f32,
    pub u: u16,
    pub v: u16,
    pub f: u8,
}

#[derive(Default)]
struct Tag {
    nloop: u32,
    flg: u32,
    nreg: u32,
    regs: u64,
    j: u32,
}

struct Pend {
    bb: u64,
    pos: u64,
    rr: u64,
    lo: u32,
    hi: u32,
    data: Vec<u128>,
}

#[derive(Default, Clone, Debug)]
pub struct GsStats {
    pub qw_in: u64,
    pub prims: [u64; 8],
    pub tris_drawn: u64,
    pub sprites_drawn: u64,
    pub up_qw: u64,
    pub up_flushed_qw: u64,
    pub up_deferred: u64,
    pub up_direct_qw: u64,
    pub clut_loads: u64,
    pub local_copies: u64,
    pub local_copy_px: u64,
    pub ctx_builds: u64,
}

pub struct Gs {
    // Declared first so it is dropped first: its back-end thread holds pointers into `vram` and the z planes.
    queue: Queue,
    vram: Vec<u8>,
    pub r: [u64; 0x80],
    tag: Tag,
    rgbaq: ([u8; 4], f32),
    st: (f32, f32),
    packed_q: f32,
    uv: (u16, u16),
    fog: u8,
    vq: [Vtx; 3],
    nvq: usize,
    trx: Trx,
    pend: Vec<Pend>,
    pend_bits: Vec<u64>,
    pend_gen: u64,
    pend_seen: u64,
    deferring: bool,
    clut_off: usize,
    cbp0: u32,
    cbp1: u32,
    clut_ver: u64,
    zplanes: Vec<Box<[u32]>>,
    zkeys: Vec<(u32, u32)>,
    /// GIF path (1-3) and VU1 program of the data being fed; only used by `hud_squeeze` / `skip_prog`.
    pub path: u8,
    pub hud: bool,
    pub prog: u8,
    pub stats: GsStats,
    /// Defer host->local uploads until something reads them (prototype default).
    pub lazy: bool,
    span_cache: HashMap<u64, (u32, u32)>,
    trx_span: Option<(u64, u64, u64, u32, u32)>,
    /// fbp (blocks) -> (fbw, psm, scissor x1+1, scissor y1+1) of the last prim drawn there.
    pub fb_info: HashMap<u32, (u32, u32, u32, u32)>,
    last_fb_info: Option<(u32, (u32, u32, u32, u32))>,
    /// Prototype widescreen HUD rule (GSFULL_HUDSQ / GSFULL_HUDMASK): 1.0 = off.
    pub hud_squeeze: f32,
    pub hud_mask: u32,
    /// Skip draws of one VU1 program (prototype GSFULL_NOMOBY = Some(11)).
    pub skip_prog: Option<u8>,
    ctx_dirty: bool,
    cur: Ctx,
    cur_tex: Option<(u32, u32)>,
    rstats: RasterStats,
}

impl Gs {
    /// `vram` is the 4 MB local memory image (e.g. a savestate's GS.bin). `threads`: raster workers
    /// (0 = draw synchronously on the caller's thread, 1 = one back-end thread, n = back end + n workers).
    pub fn new(mut vram: Vec<u8>, threads: usize) -> Gs {
        assert_eq!(vram.len(), VRAM_SIZE);
        let queue = Queue::new(vram.as_mut_ptr(), threads);
        Gs {
            queue,
            vram,
            r: [0; 0x80],
            tag: Tag::default(),
            rgbaq: ([0x80; 4], 1.0),
            st: (0.0, 0.0),
            packed_q: 1.0,
            uv: (0, 0),
            fog: 0,
            vq: [Vtx::default(); 3],
            nvq: 0,
            trx: Trx::default(),
            pend: Vec::new(),
            pend_bits: vec![0; 16384 / 64 + 1],
            pend_gen: 0,
            pend_seen: 0,
            deferring: false,
            clut_off: 0,
            cbp0: 0,
            cbp1: 0,
            clut_ver: 1,
            zplanes: Vec::new(),
            zkeys: Vec::new(),
            path: 0,
            hud: false,
            prog: 0,
            stats: GsStats::default(),
            lazy: true,
            span_cache: HashMap::new(),
            trx_span: None,
            fb_info: HashMap::new(),
            last_fb_info: None,
            hud_squeeze: 1.0,
            hud_mask: 0xc,
            skip_prog: None,
            ctx_dirty: true,
            cur: Ctx::default(),
            cur_tex: None,
            rstats: RasterStats::default(),
        }
    }

    /// The 4 MB local memory image stored in a p2s.py savestate's GS.bin.
    pub fn load_vram(path: &str) -> Option<Vec<u8>> {
        let d = std::fs::read(path).ok()?;
        (d.len() == 425 + VRAM_SIZE + 84).then(|| d[425..425 + VRAM_SIZE].to_vec())
    }

    /// Wait until everything fed so far is drawn.
    pub fn finish(&mut self) {
        self.rstats = self.queue.sync();
    }

    /// Rasteriser statistics as of the last `finish`.
    pub fn raster_stats(&self) -> &RasterStats {
        &self.rstats
    }

    /// GS local memory (after `finish`).
    pub fn vram(&mut self) -> &[u8] {
        self.finish();
        &self.vram
    }

    // ---------------- GIF ----------------

    pub fn gif(&mut self, q: u128) {
        self.stats.qw_in += 1;
        if self.tag.nloop == 0 {
            let nloop = (q & 0x7fff) as u32;
            let pre = (q >> 46) & 1 != 0;
            let prim = ((q >> 47) & 0x7ff) as u64;
            let flg = ((q >> 58) & 3) as u32;
            let nreg = ((q >> 60) & 0xf) as u32;
            self.tag = Tag { nloop, flg, nreg: if nreg == 0 { 16 } else { nreg }, regs: (q >> 64) as u64, j: 0 };
            if pre && flg == 0 {
                self.reg(0, prim);
            }
            return;
        }
        match self.tag.flg {
            0 => {
                let d = (self.tag.regs >> (4 * self.tag.j)) & 0xf;
                self.packed(d as u8, q);
                self.next_reg();
            }
            1 => {
                for half in [q as u64, (q >> 64) as u64] {
                    if self.tag.nloop == 0 {
                        break;
                    }
                    let d = (self.tag.regs >> (4 * self.tag.j)) & 0xf;
                    if d < 0xe {
                        self.reg(d as u8, half);
                    }
                    self.next_reg();
                }
            }
            _ => {
                self.image(q);
                self.tag.nloop -= 1;
            }
        }
    }

    #[inline]
    fn next_reg(&mut self) {
        self.tag.j += 1;
        if self.tag.j == self.tag.nreg {
            self.tag.j = 0;
            self.tag.nloop -= 1;
        }
    }

    #[inline]
    fn packed(&mut self, d: u8, q: u128) {
        let lo = q as u64;
        match d {
            0 => self.reg(0, lo & 0x7ff),
            1 => {
                let c = |s: u32| ((q >> s) & 0xff) as u8;
                self.rgbaq = ([c(0), c(32), c(64), c(96)], self.packed_q);
            }
            2 => {
                self.st = (f32::from_bits(q as u32), f32::from_bits((q >> 32) as u32));
                self.packed_q = f32::from_bits((q >> 64) as u32);
            }
            3 => self.uv = ((q & 0x3fff) as u16, ((q >> 32) & 0x3fff) as u16),
            4 => {
                let v = (q & 0xffff) as u64 | (((q >> 32) & 0xffff) as u64) << 16 | (((q >> 68) & 0xff_ffff) as u64) << 32 | (((q >> 100) & 0xff) as u64) << 56;
                self.reg(if (q >> 111) & 1 != 0 { 0x0c } else { 0x04 }, v);
            }
            5 => {
                let v = (q & 0xffff) as u64 | (((q >> 32) & 0xffff) as u64) << 16 | (((q >> 64) & 0xffff_ffff) as u64) << 32;
                self.reg(if (q >> 111) & 1 != 0 { 0x0d } else { 0x05 }, v);
            }
            0xa => self.fog = ((q >> 100) & 0xff) as u8,
            0xe => self.reg(((q >> 64) & 0x7f) as u8, lo),
            0xf => {}
            _ => self.reg(d, lo),
        }
    }

    fn reg(&mut self, a: u8, v: u64) {
        let a = a & 0x7f;
        match a {
            0x00 => {
                if self.r[0] != v {
                    self.r[0] = v;
                    self.ctx_dirty = true;
                }
                self.nvq = 0;
            }
            0x01 => self.rgbaq = ([v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8], f32::from_bits((v >> 32) as u32)),
            0x02 => self.st = (f32::from_bits(v as u32), f32::from_bits((v >> 32) as u32)),
            0x03 => self.uv = ((v & 0x3fff) as u16, ((v >> 16) & 0x3fff) as u16),
            0x04 | 0x05 | 0x0c | 0x0d => {
                let f = a == 0x04 || a == 0x0c;
                if f {
                    self.fog = (v >> 56) as u8;
                }
                let vt = Vtx {
                    x: v as u16,
                    y: (v >> 16) as u16,
                    z: if f { ((v >> 32) & 0xff_ffff) as u32 } else { (v >> 32) as u32 },
                    rgba: self.rgbaq.0,
                    q: self.rgbaq.1,
                    s: self.st.0,
                    t: self.st.1,
                    u: self.uv.0,
                    v: self.uv.1,
                    f: self.fog,
                };
                self.kick(vt, a == 0x04 || a == 0x05);
            }
            0x0a => self.fog = (v >> 56) as u8,
            0x53 => {
                self.r[0x53] = v;
                self.trx = Trx::default();
                self.deferring = false;
                match v & 3 {
                    0 => self.trx_start(),
                    2 => {
                        self.flush_all();
                        self.local_copy();
                    }
                    _ => {}
                }
            }
            0x54 => self.image(v as u128),
            0x06 | 0x07 => {
                let (ver, off) = (self.clut_ver, self.clut_off);
                if self.r[a as usize] != v {
                    self.r[a as usize] = v;
                    self.ctx_dirty = true;
                }
                self.clut_load(v);
                if self.clut_ver != ver || self.clut_off != off {
                    self.ctx_dirty = true;
                }
            }
            0x16 | 0x17 => {
                let k = (a - 0x10) as usize;
                let m = (0x3fu64 << 20) | (!0u64 << 37);
                let t = (self.r[k] & !m) | (v & m);
                let (ver, off) = (self.clut_ver, self.clut_off);
                if self.r[k] != t {
                    self.r[k] = t;
                    self.ctx_dirty = true;
                }
                self.clut_load(t);
                if self.clut_ver != ver || self.clut_off != off {
                    self.ctx_dirty = true;
                }
            }
            _ => {
                if self.r[a as usize] != v {
                    self.r[a as usize] = v;
                    self.ctx_dirty = true;
                }
            }
        }
    }

    #[inline]
    fn kick(&mut self, v: Vtx, draw: bool) {
        let ty = (self.r[0] & 7) as u32;
        let need = match ty {
            0 => 1,
            1 | 2 | 6 => 2,
            3..=5 => 3,
            _ => {
                self.nvq = 0;
                return;
            }
        };
        self.vq[self.nvq] = v;
        self.nvq += 1;
        let n = self.nvq;
        if n < need {
            return;
        }
        let vs = [self.vq[0], if n > 1 { self.vq[1] } else { Vtx::default() }, if n > 2 { self.vq[2] } else { Vtx::default() }];
        match ty {
            2 => {
                self.vq[0] = self.vq[1];
                self.nvq = 1;
            }
            4 => {
                self.vq[0] = self.vq[1];
                self.vq[1] = self.vq[2];
                self.nvq = 2;
            }
            5 => {
                self.vq[1] = self.vq[2];
                self.nvq = 2;
            }
            _ => self.nvq = 0,
        }
        if draw {
            self.draw(ty, &vs);
        }
    }

    fn attrs(&self) -> u64 {
        if self.r[0x1a] & 1 != 0 { self.r[0] } else { (self.r[0] & 7) | (self.r[0x1b] & !7) }
    }

    // ---------------- local memory ----------------

    pub fn rd_px(&mut self, psm: u32, bp: u32, bw: u32, x: u32, y: u32) -> u32 {
        self.finish();
        unsafe { rd_px(self.vram.as_ptr(), luts(), psm, bp, bw, x, y) }
    }

    pub fn wr_px(&mut self, psm: u32, bp: u32, bw: u32, x: u32, y: u32, v: u32) {
        self.finish();
        unsafe { wr_px(self.vram.as_mut_ptr(), luts(), psm, bp, bw, x, y, v) }
    }

    fn image(&mut self, q: u128) {
        if self.r[0x53] & 3 != 0 {
            return;
        }
        self.stats.up_qw += 1;
        if self.deferring {
            if let Some(p) = self.pend.last_mut() {
                p.data.push(q);
                return;
            }
        }
        let (bb, pos, rr) = (self.r[0x50], self.r[0x51], self.r[0x52]);
        let (lo, hi) = match self.trx_span {
            Some((b, p, r, lo, hi)) if b == bb && p == pos && r == rr => (lo, hi),
            _ => {
                let s = trx_blocks(bb, pos, rr);
                self.trx_span = Some((bb, pos, rr, s.0, s.1));
                s
            }
        };
        let t = self.trx;
        trx_advance(bb, rr, &mut self.trx, 1);
        self.queue.direct(bb, pos, rr, t, q, lo, hi);
        self.stats.up_direct_qw += 1;
    }

    fn trx_start(&mut self) {
        let (bb, pos, rr) = (self.r[0x50], self.r[0x51], self.r[0x52]);
        let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
        let (dx, dy) = (((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
        let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
        if rw == 0 || rh == 0 {
            return;
        }
        let (lo, hi) = span(dpsm, dbp, dbw, dx, dy, rw, rh);
        // Uploads into a render target go through at once; the rest wait until something reads them.
        let mut direct = !self.lazy;
        for c in 0..2 {
            let (flo, fhi) = self.fb_span(c);
            if lo < fhi && flo < hi {
                direct = true;
            }
        }
        if direct {
            self.flush(lo, hi);
            return;
        }
        self.pend.retain(|p| !(p.bb == bb && p.pos == pos && p.rr == rr));
        self.pend.push(Pend { bb, pos, rr, lo, hi, data: Vec::new() });
        self.pend_gen += 1;
        self.stats.up_deferred += 1;
        self.deferring = true;
        self.set_bits(lo, hi);
    }

    fn set_bits(&mut self, lo: u32, hi: u32) {
        for b in lo..hi.min(16384) {
            self.pend_bits[(b / 64) as usize] |= 1 << (b % 64);
        }
    }

    fn rebuild_bits(&mut self) {
        self.pend_bits.iter_mut().for_each(|w| *w = 0);
        let spans: Vec<(u32, u32)> = self.pend.iter().map(|p| (p.lo, p.hi)).collect();
        for (lo, hi) in spans {
            self.set_bits(lo, hi);
        }
    }

    fn any_pending(&self, lo: u32, hi: u32) -> bool {
        if self.pend.is_empty() {
            return false;
        }
        let hi = hi.min(16384);
        let mut b = lo;
        while b < hi {
            let w = self.pend_bits[(b / 64) as usize];
            if w >> (b % 64) != 0 {
                let nb = b + (w >> (b % 64)).trailing_zeros();
                return nb < hi;
            }
            b = (b / 64 + 1) * 64;
        }
        false
    }

    /// Block range of the render target of context `c` (its scissored rows).
    fn fb_span(&self, c: usize) -> (u32, u32) {
        let fr = self.r[0x4c + c];
        let sc = self.r[0x40 + c];
        let fbp = (fr & 0x1ff) as u32;
        let fbw = ((fr >> 16) & 0x3f) as u32;
        let rows = ((sc >> 48) & 0x7ff) as u32 / 32 + 1;
        (fbp * 32, fbp * 32 + rows * fbw.max(1) * 32)
    }

    /// Apply the deferred uploads that touch blocks [lo, hi) (and those they overlap), in order.
    fn flush(&mut self, lo: u32, hi: u32) {
        let hit = |p: &Pend, lo: u32, hi: u32| p.lo < hi && lo < p.hi;
        if !self.pend.iter().any(|p| hit(p, lo, hi)) {
            return;
        }
        let n = self.pend.len();
        let mut sel: Vec<bool> = self.pend.iter().map(|p| hit(p, lo, hi)).collect();
        for j in (0..n).rev() {
            if sel[j] {
                let (l, h) = (self.pend[j].lo, self.pend[j].hi);
                for i in 0..j {
                    if !sel[i] && hit(&self.pend[i], l, h) {
                        sel[i] = true;
                    }
                }
            }
        }
        let active = self.deferring && sel[n - 1];
        let (mut run, mut keep) = (Vec::new(), Vec::new());
        for (p, s) in std::mem::take(&mut self.pend).into_iter().zip(sel) {
            if s {
                run.push(p);
            } else {
                keep.push(p);
            }
        }
        self.pend = keep;
        for p in run {
            let mut t = Trx::default();
            trx_advance(p.bb, p.rr, &mut t, p.data.len());
            self.stats.up_flushed_qw += p.data.len() as u64;
            self.queue.upload(p.bb, p.pos, p.rr, Trx::default(), p.data, p.lo, p.hi);
            self.trx = t;
        }
        if active {
            self.deferring = false;
        }
        self.rebuild_bits();
    }

    /// Apply all deferred uploads.
    pub fn flush_all(&mut self) {
        self.flush(0, u32::MAX);
    }

    fn local_copy(&mut self) {
        let (bb, pos, rr) = (self.r[0x50], self.r[0x51], self.r[0x52]);
        self.queue.copy(bb, pos, rr);
        self.stats.local_copies += 1;
        self.stats.local_copy_px += ((rr & 0xfff) * ((rr >> 32) & 0xfff)) as u64;
    }

    // ---------------- CLUT ----------------

    fn clut_load(&mut self, tex0: u64) {
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
            self.stats.clut_loads += 1;
            if self.any_pending(cbp, cbp + 4) {
                self.flush(cbp, cbp + 4);
            }
        }
        let texa = self.r[0x3b];
        if t4 {
            let off = if cfmt == 2 { csa * 16 } else { (csa & 15) * 16 };
            self.clut_off = off;
            if load && self.queue.clut(cbp, cfmt, true, off as u32, texa) {
                self.clut_ver += 1;
            }
        } else if load && self.queue.clut(cbp, cfmt, false, 0, texa) {
            self.clut_ver += 1;
        }
    }

    // ---------------- drawing ----------------

    /// Prototype per-draw handling of deferred uploads: flush those the prim's texture or target touches.
    fn pending_for_draw(&mut self, at: u64) {
        if self.pend.is_empty() {
            return;
        }
        let c = ((at >> 9) & 1) as usize;
        if at & 0x10 != 0 {
            let tex0 = self.r[0x06 + c];
            let key = tex0 & 0xffff_ffff_ffff & !(0x3fffu64 << 37);
            let (lo, hi) = match self.span_cache.get(&key) {
                Some(&s) => s,
                None => {
                    let tbw = ((tex0 >> 14) & 0x3f) as u32;
                    let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
                    let s = span(((tex0 >> 20) & 0x3f) as u32, (tex0 & 0x3fff) as u32, tbw, 0, 0, tw, th);
                    self.span_cache.insert(key, s);
                    s
                }
            };
            if self.any_pending(lo, hi) {
                self.flush(lo, hi);
            }
        }
        let (flo, fhi) = self.fb_span(c);
        if self.any_pending(flo, fhi) {
            self.flush(flo, fhi);
        }
    }

    fn zplane(&mut self, key: (u32, u32)) -> u32 {
        if let Some(i) = self.zkeys.iter().position(|k| *k == key) {
            return i as u32;
        }
        let mut p = vec![0u32; 1024 * 1024].into_boxed_slice();
        self.queue.add_zplane(p.as_mut_ptr());
        self.zplanes.push(p);
        self.zkeys.push(key);
        (self.zkeys.len() - 1) as u32
    }

    /// Decode the drawing registers for a prim with attributes `at`; also returns the block range its
    /// texture sampling can reach.
    fn make_ctx(&mut self, at: u64) -> (Ctx, Option<(u32, u32)>) {
        let c = ((at >> 9) & 1) as usize;
        let frame = self.r[0x4c + c];
        let tme = at & 0x10 != 0;
        let tex0 = self.r[0x06 + c];
        let tex1 = self.r[0x14 + c];
        let clamp = self.r[0x08 + c];
        let test = self.r[0x47 + c];
        let zbuf = self.r[0x4e + c];
        let sc = self.r[0x40 + c];
        let xy = self.r[0x18 + c];
        let fpsm = ((frame >> 24) & 0x3f) as u32;
        let fbp = ((frame & 0x1ff) * 32) as u32;
        let fbw = ((frame >> 16) & 0x3f) as u32;
        let zpsm = ((zbuf >> 24) & 0xf) as u32;
        let zbp = (zbuf & 0x1ff) as u32;
        let zplane = self.zplane((zbp, fbw));
        let tpsm = ((tex0 >> 20) & 0x3f) as u32;
        let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
        let mut cx = Ctx {
            at,
            tme,
            fst: at & 0x100 != 0,
            iip: at & 8 != 0,
            fge: at & 0x20 != 0,
            abe: at & 0x40 != 0,
            bilinear: tex1 & 0x20 != 0,
            tex0,
            clamp,
            alpha: self.r[0x42 + c],
            test,
            tbp: (tex0 & 0x3fff) as u32,
            tbw: ((tex0 >> 14) & 0x3f) as u32,
            tpsm,
            tw,
            th,
            fpsm,
            fbp,
            fbw,
            fbmsk: (frame >> 32) as u32,
            fba: self.r[0x4a + c] & 1 != 0,
            zte: test & 0x10000 != 0,
            ztst: ((test >> 17) & 3) as u32,
            zwrite: (zbuf >> 32) & 1 == 0,
            zmask: match zpsm {
                0 => 0xffff_ffff,
                1 => 0xff_ffff,
                _ => 0xffff,
            },
            pabe: self.r[0x49] & 1 != 0,
            colclamp: self.r[0x46] & 1 != 0,
            fogcol: self.r[0x3d],
            texa: self.r[0x3b],
            ofx: (xy & 0xffff) as i32,
            ofy: ((xy >> 32) & 0xffff) as i32,
            sc: [(sc & 0x7ff) as i32, ((sc >> 16) & 0x7ff) as i32, ((sc >> 32) & 0x7ff) as i32, ((sc >> 48) & 0x7ff) as i32],
            clut_off: 0,
            clut_ver: 0,
            clut: 0,
            zplane,
            tex1,
            mip1: self.r[0x34 + c],
            mip2: self.r[0x36 + c],
            ..Ctx::default()
        };
        cx.derive();
        let info = (fbw, fpsm, (cx.sc[1] + 1) as u32, (cx.sc[3] + 1) as u32);
        if self.last_fb_info != Some((fbp, info)) {
            self.fb_info.insert(fbp, info);
            self.last_fb_info = Some((fbp, info));
        }
        let mut tex = None;
        if tme {
            if matches!(tpsm, 0x13 | 0x1b | 0x14 | 0x24 | 0x2c) {
                cx.clut_off = self.clut_off as u32;
                cx.clut_ver = self.clut_ver;
            }
            // Texels the sampler can reach (wrap modes 2/3 can go past the texture size).
            let ext = |m: u64, size: u32, lo: u32, hi: u32| match m & 3 {
                2 => size.max(hi + 1),
                3 => size.max((lo | hi) + 1),
                _ => size,
            };
            let xw = ext(clamp, tw, ((clamp >> 4) & 0x3ff) as u32, ((clamp >> 14) & 0x3ff) as u32);
            let yh = ext(clamp >> 2, th, ((clamp >> 24) & 0x3ff) as u32, ((clamp >> 34) & 0x3ff) as u32);
            let key = (tex0 & 0x3fff_ffff_ffff) ^ ((xw as u64) << 46) ^ ((yh as u64) << 57) ^ (1u64 << 63);
            let s = match self.span_cache.get(&key) {
                Some(&s) => s,
                None => {
                    let s = span(tpsm, cx.tbp, cx.tbw, 0, 0, xw, yh);
                    self.span_cache.insert(key, s);
                    s
                }
            };
            // Mip levels the LOD can pick widen the read range (one range: the union).
            let mut s = s;
            if cx.lodsel {
                for n in 1..=((tex1 >> 2) & 7) as u32 {
                    let (bp, bw) = cx.level_tbp(n);
                    let l = span(tpsm, bp, bw, 0, 0, (xw >> n).max(1), (yh >> n).max(1));
                    s = (s.0.min(l.0), s.1.max(l.1));
                }
            }
            tex = Some(s);
        }
        (cx, tex)
    }

    fn draw(&mut self, ty: u32, vs: &[Vtx; 3]) {
        if self.skip_prog == Some(self.prog) {
            return;
        }
        let at = self.attrs();
        let mut vs = *vs;
        let f = self.hud_squeeze;
        if f != 1.0 && (if self.hud_mask == 0 { self.hud } else { (self.hud_mask >> (self.path & 31)) & 1 != 0 }) {
            let c = ((at >> 9) & 1) as usize;
            let ofx = (self.r[0x18 + c] & 0xffff) as i32;
            let fb_copy = at & 0x10 != 0 && (self.r[0x06 + c] & 0x3fff) == (self.r[0x4c + c] & 0x1ff) * 32;
            if !fb_copy {
                for v in vs.iter_mut() {
                    let sx = (v.x as i32 - ofx) as f32 / 16.0;
                    let n = if sx < 256.0 { sx * f } else { 512.0 - (512.0 - sx) * f };
                    v.x = (ofx as f32 + n * 16.0).round().clamp(0.0, 65535.0) as u16;
                }
            }
        }
        self.stats.prims[ty as usize] += 1;
        if ty == 6 {
            self.stats.sprites_drawn += 1;
        } else if (3..=5).contains(&ty) {
            self.stats.tris_drawn += 1;
        }
        if self.ctx_dirty {
            self.ctx_dirty = false;
            self.stats.ctx_builds += 1;
            self.pending_for_draw(at);
            self.pend_seen = self.pend_gen;
            let (cx, tex) = self.make_ctx(at);
            // The sampler can read mip levels too: apply the deferred uploads of every level it can pick.
            if let Some((lo, hi)) = tex {
                if self.any_pending(lo, hi) {
                    self.flush(lo, hi);
                }
            }
            if cx != self.cur {
                self.cur = cx;
                self.queue.new_ctx();
            }
            self.cur_tex = tex;
        } else if !self.pend.is_empty() && self.pend_seen != self.pend_gen {
            self.pending_for_draw(at);
            if let Some((lo, hi)) = self.cur_tex {
                if self.any_pending(lo, hi) {
                    self.flush(lo, hi);
                }
            }
            self.pend_seen = self.pend_gen;
        }
        let Some(bb) = bbox(&self.cur, ty, &vs) else { return };
        self.queue.push(&self.cur, Prim { v: vs, ctx: 0, ty: ty as u8, bb }, self.cur_tex);
    }

    /// RGBA8 image of a frame buffer region.
    pub fn read_fb(&mut self, fbp_blocks: u32, fbw: u32, psm: u32, w: u32, h: u32) -> Vec<u8> {
        self.finish();
        let (l, m) = (luts(), self.vram.as_ptr());
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let v = unsafe { rd_px(m, l, psm, fbp_blocks, fbw, x, y) };
                let c = unpack_col(psm, v, 0x80);
                out.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        out
    }

    /// RGBA8 of the w x h frame-buffer rectangle at (x0, y0) with the stored alpha (the CRTC's read); only the deferred
    /// uploads that touch the rectangle's blocks are applied.
    pub fn read_rect(&mut self, fbp_blocks: u32, fbw: u32, psm: u32, x0: u32, y0: u32, w: u32, h: u32) -> Vec<u8> {
        let (lo, hi) = span(psm, fbp_blocks, fbw, x0, y0, w, h);
        self.flush(lo, hi);
        self.finish();
        let (l, m) = (luts(), self.vram.as_ptr());
        let mut out = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                let v = unsafe { rd_px(m, l, psm, fbp_blocks, fbw, x0 + x, y0 + y) };
                out.extend_from_slice(&unpack_col(psm, v, 0x80));
            }
        }
        out
    }

    /// Local memory with all deferred uploads applied (the state itself is not changed).
    pub fn vram_effective(&mut self) -> Vec<u8> {
        self.finish();
        let mut v = self.vram.clone();
        for p in &self.pend {
            let mut t = Trx::default();
            for &q in &p.data {
                unsafe { put(v.as_mut_ptr(), p.bb, p.pos, p.rr, &mut t, q) };
            }
        }
        v
    }

    /// Z planes: ((ZBP, FBW), 1024x1024 values, row-major).
    pub fn zplanes(&mut self) -> Vec<((u32, u32), &[u32])> {
        self.finish();
        self.zkeys.iter().copied().zip(self.zplanes.iter().map(|p| &p[..])).collect()
    }
}

impl Drop for Gs {
    fn drop(&mut self) {
        self.finish();
    }
}

/// Block range written by the host->local transfer described by BITBLTBUF / TRXPOS / TRXREG.
fn trx_blocks(bb: u64, pos: u64, rr: u64) -> (u32, u32) {
    let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
    let (dx, dy) = (((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
    let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
    if rw == 0 || rh == 0 {
        return (0, 0);
    }
    span(dpsm, dbp, dbw, dx, dy, rw, rh)
}

/// Conservative pixel bounding box of a prim (inclusive, inside 0..1023); None when it covers nothing.
fn bbox(cx: &Ctx, ty: u32, vs: &[Vtx; 3]) -> Option<[i16; 4]> {
    let (ofx, ofy) = (cx.ofx, cx.ofy);
    let px = |v: &Vtx| (v.x as i32 - ofx, v.y as i32 - ofy);
    let clip = |x0: i32, y0: i32, x1: i32, y1: i32, sc: bool| -> Option<[i16; 4]> {
        let (mut x0, mut y0, mut x1, mut y1) = (x0.max(0), y0.max(0), x1.min(1023), y1.min(1023));
        if sc {
            x0 = x0.max(cx.sc[0]);
            x1 = x1.min(cx.sc[1]);
            y0 = y0.max(cx.sc[2]);
            y1 = y1.min(cx.sc[3]);
        }
        (x0 <= x1 && y0 <= y1).then_some([x0 as i16, y0 as i16, x1 as i16, y1 as i16])
    };
    match ty {
        0 => {
            let (x, y) = px(&vs[0]);
            let (x, y) = ((x + 8) >> 4, (y + 8) >> 4);
            clip(x, y, x, y, false)
        }
        1 | 2 => {
            let ((ax, ay), (bx, by)) = (px(&vs[0]), px(&vs[1]));
            clip((ax.min(bx) >> 4) - 1, (ay.min(by) >> 4) - 1, (ax.max(bx) >> 4) + 2, (ay.max(by) >> 4) + 2, true)
        }
        3..=5 => {
            let (p0, p1, p2) = (px(&vs[0]), px(&vs[1]), px(&vs[2]));
            let area = (p1.0 - p0.0) as i64 * (p2.1 - p0.1) as i64 - (p1.1 - p0.1) as i64 * (p2.0 - p0.0) as i64;
            if area == 0 {
                return None;
            }
            clip((p0.0.min(p1.0).min(p2.0) + 15) >> 4, (p0.1.min(p1.1).min(p2.1) + 15) >> 4, p0.0.max(p1.0).max(p2.0) >> 4, p0.1.max(p1.1).max(p2.1) >> 4, true)
        }
        6 => {
            let ((ax, ay), (bx, by)) = (px(&vs[0]), px(&vs[1]));
            clip((ax.min(bx) + 15) >> 4, (ay.min(by) + 15) >> 4, ((ax.max(bx) + 15) >> 4) - 1, ((ay.max(by) + 15) >> 4) - 1, true)
        }
        _ => None,
    }
}
