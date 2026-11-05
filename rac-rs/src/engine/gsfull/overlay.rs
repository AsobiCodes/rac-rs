//! R&C2 HUD overlay mode (rac-rs/src/rc2 window and `--shots --native`): rac-rs draws the 3D world natively (rc2/native),
//! so the VU1 world programs do not run; the GS draws the HUD and the effect programs `progs` still runs into a cleared,
//! see-through buffer (gs_overlay.rs) that is laid over the native picture (`crtc::present_overlay`). Along the way the
//! VIF1 stream hands the native picture the water surface input (water.rs) and the textures it asked for (texreq.rs).
//! Pipe / Vif1 / Gs keep R&C1's path unchanged while `Vif1::ovl` is None and `Gs::set_overlay` was never called.
use super::crtc::Crtc;
use super::gs::Gs;
use super::pipe::{Field, Pipe};
use super::vif::Out;
use super::vu1::Vu1;
use super::water::{GifRegs, WaterBatch};
use ee_oracle::gs::Job;

/// Most qwords one effect-program run may emit in overlay mode (normal particle runs: a few thousand).
const FX_MAX_QW: usize = 100_000;

/// VU1 programs (bit n = program id n) that still run in HUD overlay mode: the effects the native picture does not draw yet.
/// Program 2 = sprites / particles [C: Aranos runs {2, 5, 6, 7, 11}; 5, 6 terrain, 7 ties, 11 mobies], program 10 = the
/// water surface [C: level 1 field 2700 RAC2_ONLYPROG=10 draws the swamp plane] (not on by default: in overlay mode its
/// draws come out as garbage texture blocks and fail the stale GS Z test). RAC2_FX_PROGS="2,10" overrides.
pub fn progs() -> u32 {
    static M: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *M.get_or_init(|| match std::env::var("RAC2_FX_PROGS") {
        Ok(v) => v.split(',').filter_map(|x| x.trim().parse::<u32>().ok()).filter(|&n| n < 16).fold(0, |m, n| m | 1 << n),
        Err(_) => 1 << 2,
    })
}

/// The overlay state of the VIF1 front end (Vif1::ovl).
#[derive(Default)]
pub struct VifOverlay {
    /// GS registers as the GIF packets so far left them, and the qwords of `Out::qs` already fed to it.
    gif: GifRegs,
    parsed: usize,
    /// The water batches (VU1 program 10 inputs) of this field.
    water: Vec<WaterBatch>,
    /// VU1 data memory at the last skipped run scanned for wanted TEX0s (only qwords written since count).
    texseen: Vec<[u32; 4]>,
    /// (qword, TEX0) where a skipped VU1 run would have drawn with a TEX0 the native picture wants (texreq::mark at that
    /// point of the stream), for the current job.
    texmarks: Vec<(u32, u64)>,
}

impl VifOverlay {
    /// One MSCAL / MSCNT in overlay mode (Vif1::exec): the water input is captured, the world programs are skipped (an
    /// empty PATH1 segment still marks where the 3D draw was, so the PATH2 / PATH3 draws after the last one read as the
    /// HUD), the effect programs run with a cap on their output.
    pub(super) fn exec(&mut self, addr: usize, top: u32, itop: u32, vu: &mut Vu1, out: &mut Out) {
        if out.prog == super::water::WATER_PROG {
            for &q in &out.qs[self.parsed.min(out.qs.len())..] {
                self.gif.feed(q);
            }
            self.parsed = out.qs.len();
            if let Some(b) = super::water::capture(&vu.data, top as usize, &self.gif) {
                // Its texture is uploaded just before and overwritten later in the field: a wanted one is read here.
                if super::texreq::wanted(b.tex0) && !self.texmarks.iter().any(|m| m.1 == b.tex0) {
                    self.texmarks.push((out.qs.len() as u32, b.tex0));
                }
                self.water.push(b);
            }
        }
        if (progs() >> (out.prog & 15)) & 1 == 0 {
            out.seg(1);
            self.scan_tex(vu, out);
            return;
        }
        // (a run that errors leaves the packets it already sent; the field goes on)
        let (nq, ns) = (out.qs.len(), out.segs.len());
        if vu.run(addr, top, itop, out).is_err() {
            out.stats.run_errs += 1;
        }
        // An effect program that emits a runaway count (4 M qwords in one run on a few level 1 fields, a bogus particle
        // count [?]) would take seconds to draw: its output of that run is dropped.
        if out.qs.len() - nq > FX_MAX_QW {
            out.qs.truncate(nq);
            out.segs.truncate(ns);
            out.seg(1);
        }
    }

    /// A skipped run's TEX0s (A+D TEX0_1 / TEX0_2 qwords it uploaded) that the native picture wants: textures the game
    /// uploads just before the draw and overwrites later in the field (Ratchet's head at TBP 0) are read at this point of
    /// the stream instead of after the field. The first skipped run after textures became wanted only takes the snapshot
    /// (older memory would match TEX0s of earlier draws, whose texels are gone).
    fn scan_tex(&mut self, vu: &Vu1, out: &Out) {
        if !super::texreq::pending() {
            self.texseen.clear();
        } else if self.texseen.len() != vu.data.len() {
            self.texseen = vu.data.clone();
        } else {
            for (q, old) in vu.data.iter().zip(&self.texseen) {
                if q != old && matches!(q[2] & 0xff, 6 | 7) {
                    let t = q[0] as u64 | (q[1] as u64) << 32;
                    if super::texreq::wanted(t) && !self.texmarks.iter().any(|m| m.1 == t) {
                        self.texmarks.push((out.qs.len() as u32, t));
                    }
                }
            }
            self.texseen.copy_from_slice(&vu.data);
        }
    }
}

/// One overlay field: the GS qwords and the texture marks (field qword, TEX0) for `raster`.
#[derive(Default)]
pub struct OvlField {
    pub f: Field,
    pub texmarks: Vec<(u32, u64)>,
}

impl Pipe {
    /// One field's jobs in overlay mode (`overlay` true) or the normal one; the water batches go to water::WATER.
    pub fn process_rc2(&mut self, jobs: Vec<Job>, crtc: Crtc, overlay: bool) -> OvlField {
        if !overlay {
            self.vif.ovl = None;
            return OvlField { f: self.process(jobs, crtc), texmarks: Vec::new() };
        }
        let mut o = self.vif.ovl.take().unwrap_or_default();
        let mut f = Field { crtc, ..Default::default() };
        let mut texmarks = Vec::new();
        f.job_dispfb = jobs
            .iter()
            .rev()
            .find_map(|j| match j {
                Job::Vif1(_, _, d) | Job::Gif(_, d) => Some(*d),
            })
            .unwrap_or(0);
        for j in jobs {
            match j {
                Job::Vif1(w, marks, _) => {
                    self.out.qs.clear();
                    self.out.segs.clear();
                    o.texmarks.clear();
                    o.parsed = 0;
                    self.vif.ovl = Some(o);
                    self.vif.feed(&w, &marks, &mut self.vu, &mut self.out);
                    ee_oracle::gs::recycle_words(w);
                    o = self.vif.ovl.take().unwrap_or_default();
                    for &q in &self.out.qs[o.parsed.min(self.out.qs.len())..] {
                        o.gif.feed(q);
                    }
                    let base = f.qs.len() as u32;
                    f.qs.extend_from_slice(&self.out.qs);
                    f.segs.extend(self.out.segs.iter().map(|&(s, p, g)| (base + s, p, g)));
                    texmarks.extend(o.texmarks.iter().map(|&(q, t)| (base + q, t)));
                }
                Job::Gif(qs, _) => {
                    qs.iter().for_each(|&q| o.gif.feed(q));
                    f.segs.push((f.qs.len() as u32, 3, 0));
                    f.qs.extend(qs);
                }
            }
        }
        f.vu_errors = self.vu.st.errors;
        if let Ok(mut w) = super::water::WATER.lock() {
            *w = std::mem::take(&mut o.water);
        }
        self.vif.ovl = Some(o);
        OvlField { f, texmarks }
    }
}

/// A segment (start qword, path, VU1 program) of an effect program the overlay still draws (`progs`).
pub fn is_fx(s: &(u32, u8, u8)) -> bool {
    s.1 == 1 && (progs() >> (s.2 & 15)) & 1 != 0
}

/// Index of the last PATH1 segment that is not an effect program (the 3D draw the native picture replaces), -1 when none.
pub fn last_world_seg(segs: &[(u32, u8, u8)]) -> i64 {
    segs.iter().rposition(|s| s.1 == 1 && !is_fx(s)).map(|x| x as i64).unwrap_or(-1)
}

/// Segment `i` (after `last_world_seg` `last1`) draws the HUD (everything after the 3D draw) or an effect: its overlay pixels count.
pub fn is_hud(i: usize, seg: &(u32, u8, u8), last1: i64) -> bool {
    (i as i64) > last1 || is_fx(seg)
}

/// Feeds a field to the GS. An overlay field draws the HUD into a cleared, see-through buffer (`Gs::set_overlay`); the 3D
/// is not run at all. The effect programs (`progs`) also run: their PATH1 segments are drawn into the overlay like the
/// HUD, and do not count as the 3D draw that precedes it. A normal field is `pipe::raster` (overlay mode off).
pub fn raster(g: &mut Gs, of: &OvlField, overlay: bool) {
    let f = &of.f;
    let shown = f.crtc.shown().map_or((512, 448), |s| (s.4, s.5));
    g.set_overlay(overlay, shown);
    if !overlay {
        g.set_fx(false);
        super::pipe::raster(g, f);
        return;
    }
    let last1 = last_world_seg(&f.segs);
    let mut mk = 0;
    for (i, seg) in f.segs.iter().enumerate() {
        g.set_fx(is_fx(seg));
        g.set_seg(seg.1, seg.2, is_hud(i, seg, last1));
        let end = f.segs.get(i + 1).map(|s| s.0 as usize).unwrap_or(f.qs.len());
        for k in seg.0 as usize..end {
            while let Some(&(_, t)) = of.texmarks.get(mk).filter(|m| m.0 as usize <= k) {
                super::texreq::mark(g, t);
                mk += 1;
            }
            g.gif(f.qs[k]);
        }
    }
    for &(_, t) in &of.texmarks[mk.min(of.texmarks.len())..] {
        super::texreq::mark(g, t);
    }
}
