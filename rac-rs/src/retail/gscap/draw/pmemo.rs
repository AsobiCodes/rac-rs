//! Memo of big prims (PC speed-up, no retail counterpart; RAC_GSCAP_PMEMO=0 turns it off).
//!
//! A menu or the galactic map redraws the same big panels every field: full-screen clears, the panel pictures
//! composited into the overlay, the render-to-texture panels (~2.5M pixels a field, 25 ms on the capture thread). A prim's
//! pixels are a pure function of what it reads: its registers and vertices, the texture's blocks, the CLUT and the
//! destination rectangle before it draws (blend, alpha test / FBA and the frame mask all read the destination; the
//! capture has no depth buffer). Every sprite / triangle of at least `PM_MIN_PX` pixels is therefore keyed by a 128-bit
//! hash of the prim record and of the contents of the texture range, the CLUT and the destination range; a prim that was
//! drawn before from identical inputs gets the destination rectangle it produced then, copied back, instead of being
//! rasterized again. A record is rasterized (and its output kept) only from its second sighting, and a record whose
//! last three sightings all missed is looked up only one time in 64, so a changing picture costs one record hash; after 48 lookups in a row without a hit only every 32nd record is looked up.

use super::{Gs, St, Vtx};
use rayon::prelude::*;
use std::collections::HashMap;

const PM_MIN_PX: i64 = 3000;
const PM_BYTES: usize = 160 << 20;
const PAR_CHUNK: usize = 1 << 17;
const GIVE_UP: u8 = 3;
const COLD_LOOKUPS: u32 = 48;

pub(crate) type H = (u64, u64);

const K1: u64 = 0xff51_afd7_ed55_8ccd;
const K2: u64 = 0xc4ce_b9fe_1a85_ec53;
const K3: u64 = 0x9e37_79b9_7f4a_7c15;

/// 4 lanes of xor-multiply-rotate over the 8-byte words (each step is a bijection of its lane, so one differing word
/// always changes the lane), then the length and the tail bytes.
fn lane_hash(b: &[u8]) -> H {
    let mut l = [K3, K2, K1, 0x2545_f491_4f6c_dd1d];
    let mut it = b.chunks_exact(32);
    for c in &mut it {
        for (k, lane) in l.iter_mut().enumerate() {
            let w = u64::from_le_bytes(c[k * 8..k * 8 + 8].try_into().unwrap_or([0; 8]));
            *lane = (*lane ^ w).wrapping_mul(K1).rotate_left(31);
        }
    }
    for &x in it.remainder() {
        l[0] = (l[0] ^ x as u64).wrapping_mul(K2).rotate_left(13);
    }
    l[0] ^= b.len() as u64;
    (l[0].wrapping_add(l[1].wrapping_mul(K2)) ^ l[1].rotate_left(29), l[2].wrapping_add(l[3].wrapping_mul(K1)) ^ l[3].rotate_left(43))
}

fn comb(a: H, b: H) -> H {
    ((a.0 ^ b.0.wrapping_mul(K1)).rotate_left(23).wrapping_mul(K3), (a.1 ^ b.1.wrapping_mul(K2)).rotate_left(37).wrapping_mul(K1))
}

/// Hash of a byte slice, its chunks hashed on the capture's thread pool when it is big.
fn par_hash(b: &[u8]) -> H {
    if b.len() < 2 * PAR_CHUNK {
        return lane_hash(b);
    }
    let v: Vec<H> = super::on_pool(|| b.par_chunks(PAR_CHUNK).map(lane_hash).collect());
    v.into_iter().fold((K3, K2), comb)
}

fn px_bytes(s: &[[f32; 4]]) -> &[u8] {
    // SAFETY: four f32 per pixel, no padding; any bit pattern is a valid byte.
    unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, std::mem::size_of_val(s)) }
}

fn u64_bytes(s: &[u64]) -> &[u8] {
    // SAFETY: as `px_bytes`.
    unsafe { std::slice::from_raw_parts(s.as_ptr() as *const u8, std::mem::size_of_val(s)) }
}

/// The destination rectangle of a memoised prim: block bytes `lo..hi` of local memory, or overlay rows / columns.
#[derive(Clone, Copy)]
pub(crate) enum Dest {
    Vram { lo: usize, hi: usize },
    Ov { disp: bool, x0: usize, x1: usize, y0: usize, y1: usize },
}

enum Out {
    Vram { lo: usize, data: Vec<u8> },
    Ov { disp: bool, x0: usize, x1: usize, y0: usize, y1: usize, data: Vec<[f32; 4]> },
}

struct Entry {
    out: Out,
    used: u64,
}

impl Entry {
    fn size(&self) -> usize {
        match &self.out {
            Out::Vram { data, .. } => data.len(),
            Out::Ov { data, .. } => data.len() * 16,
        }
    }
}

pub(crate) enum Tok {
    None,
    Hit,
    Miss(H, Dest),
}

#[derive(Default)]
pub(crate) struct PMemo {
    /// Prim record -> consecutive misses.
    seen: HashMap<H, u8>,
    map: HashMap<H, Entry>,
    bytes: usize,
    tick: u64,
    hit: u64,
    miss: u64,
    first: u64,
    skip: u64,
    /// Lookups in a row that found nothing (a scene that never repeats, e.g. gameplay): lookups then thin out.
    cold: u32,
}

impl PMemo {
    pub(crate) fn new() -> Option<Box<PMemo>> {
        (std::env::var("RAC_GSCAP_PMEMO").map_or(true, |v| v != "0")).then(Default::default)
    }
}

impl Gs {
    /// The memo's counters for the prof line.
    pub(crate) fn pm_stats(&self) -> String {
        self.pm.as_ref().map_or("off".into(), |p| format!("hit {} miss {} first {} skip {} kept {} MB", p.hit, p.miss, p.first, p.skip, p.bytes >> 20))
    }

    /// Before a sprite / triangle rasterizes the destination rectangle `r` (x0, x1, y0, y1; x1 / y1 exclusive): `Hit` when
    /// the rectangle was restored from an earlier identical prim (nothing to draw), `Miss` when the prim must be drawn
    /// and then handed to `pm_end`, `None` when it is not memoised.
    pub(super) fn pm_begin(&mut self, st: &St, ty: u32, vs: &[Vtx], r: (i32, i32, i32, i32)) -> Tok {
        let Some(mut pm) = self.pm.take() else { return Tok::None };
        let t = self.pm_look(&mut pm, st, ty, vs, r);
        self.pm = Some(pm);
        t
    }

    fn pm_look(&mut self, pm: &mut PMemo, st: &St, ty: u32, vs: &[Vtx], r: (i32, i32, i32, i32)) -> Tok {
        let (w, h) = (self.cur.w as i32, self.cur.h as i32);
        let (mut x0, mut x1, mut y0, mut y1) = r;
        x0 = x0.max(0);
        y0 = y0.max(0);
        if st.ov {
            x1 = x1.min(w);
            y1 = y1.min(h);
        }
        if x1 <= x0 || y1 <= y0 || ((x1 - x0) as i64) * ((y1 - y0) as i64) < PM_MIN_PX {
            return Tok::None;
        }
        let tme = st.at & 0x10 != 0;
        let mut tex: Option<(usize, usize)> = None;
        if tme {
            let cl = st.clamp;
            // Region clamps can read outside the texture's own size.
            if cl & 3 >= 2 || (cl >> 2) & 3 >= 2 {
                pm.skip += 1;
                return Tok::None;
            }
            let tex0 = st.tex0;
            let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
            let (lo, hi) = Self::span(((tex0 >> 20) & 0x3f) as u32, (tex0 & 0x3fff) as u32, ((tex0 >> 14) & 0x3f) as u32, 0, 0, tw, th);
            if hi == u32::MAX || (hi as usize) * 256 > self.vram.len() || lo >= hi {
                pm.skip += 1;
                return Tok::None;
            }
            tex = Some((lo as usize * 256, hi as usize * 256));
        }
        let dest = if st.ov {
            Dest::Ov { disp: self.to_disp && self.dcur.is_some(), x0: x0 as usize, x1: x1 as usize, y0: y0 as usize, y1: y1 as usize }
        } else {
            let frame = st.frame;
            let (fbp, fbw, psm) = (((frame & 0x1ff) * 32) as u32, ((frame >> 16) & 0x3f) as u32, ((frame >> 24) & 0x3f) as u32);
            if !matches!(psm, 0x00 | 0x01 | 0x02) || fbw == 0 {
                pm.skip += 1;
                return Tok::None;
            }
            let (lo, hi) = Self::span(psm, fbp, fbw, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32);
            if hi == u32::MAX || (hi as usize) * 256 > self.vram.len() || lo >= hi {
                pm.skip += 1;
                return Tok::None;
            }
            Dest::Vram { lo: lo as usize * 256, hi: hi as usize * 256 }
        };
        // The prim's record: everything but the contents of what it reads.
        let mut s: Vec<u64> = vec![
            ty as u64, st.at, st.tex0, st.tex1, st.clamp, st.alpha, st.test, st.frame, st.fba, st.ofx.to_bits() as u64, st.ofy.to_bits() as u64, st.sc[0] as u64, st.sc[1] as u64, st.sc[2] as u64, st.sc[3] as u64, st.ov as u64, x0 as u64, x1 as u64, y0 as u64,
            y1 as u64, w as u64, h as u64, self.r[0x3b], self.r[0x3d], self.r[0x46], self.r[0x49], self.clut_off as u64, self.to_disp as u64, vs.len() as u64,
        ];
        for v in vs {
            s.extend([v.x as u64, v.y as u64, v.z as u64, u32::from_le_bytes(v.rgba) as u64, v.q.to_bits() as u64, v.s.to_bits() as u64, v.t.to_bits() as u64, v.u as u64, v.v as u64, v.f as u64]);
        }
        let rec = lane_hash(u64_bytes(&s));
        let misses = match pm.seen.get(&rec) {
            None => {
                if pm.seen.len() > 16384 {
                    pm.seen.clear();
                }
                pm.seen.insert(rec, 0);
                pm.first += 1;
                return Tok::None;
            }
            Some(&m) => m,
        };
        pm.tick += 1;
        if misses >= GIVE_UP && pm.tick % 64 != 0 {
            // Hopeless record (its picture changes every time): looked up only now and then.
            pm.skip += 1;
            return Tok::None;
        }
        if pm.cold >= COLD_LOOKUPS && pm.tick % 32 != 0 {
            pm.skip += 1;
            return Tok::None;
        }
        // Seen before: the key adds the contents it reads (texture blocks, CLUT, the destination rectangle).
        let t0 = self.prof.is_some().then(std::time::Instant::now);
        let mut key = rec;
        if let Some((lo, hi)) = tex {
            key = comb(key, par_hash(&self.vram[lo..hi]));
            key = comb(key, lane_hash(px_bytes(&self.clut)));
        }
        key = comb(key, self.pm_dest_hash(dest));
        self.prof_add("pm hash", t0);
        let t0 = self.prof.is_some().then(std::time::Instant::now);
        if let Some(e) = pm.map.get_mut(&key) {
            e.used = pm.tick;
            match &e.out {
                Out::Vram { lo, data } => self.vram[*lo..*lo + data.len()].copy_from_slice(data),
                Out::Ov { disp, x0, x1, y0, y1, data } => {
                    let w = self.cur.w as usize;
                    let buf = match (disp, self.dcur.as_mut()) {
                        (true, Some(d)) => d.as_mut_slice(),
                        _ => self.cur.px.as_mut_slice(),
                    };
                    let n = x1 - x0;
                    for (k, y) in (*y0..*y1).enumerate() {
                        buf[y * w + x0..y * w + x1].copy_from_slice(&data[k * n..(k + 1) * n]);
                    }
                }
            }
            pm.hit += 1;
            pm.cold = 0;
            self.prof_add("pm restore", t0);
            pm.seen.insert(rec, 0);
            return Tok::Hit;
        }
        pm.miss += 1;
        pm.cold = pm.cold.saturating_add(1);
        pm.seen.insert(rec, misses.saturating_add(1));
        Tok::Miss(key, dest)
    }

    fn pm_dest_hash(&self, dest: Dest) -> H {
        match dest {
            Dest::Vram { lo, hi } => par_hash(&self.vram[lo..hi]),
            Dest::Ov { disp, x0, x1, y0, y1 } => {
                let w = self.cur.w as usize;
                let buf: &[[f32; 4]] = match (disp, self.dcur.as_ref()) {
                    (true, Some(d)) => d,
                    _ => &self.cur.px,
                };
                let rows: Vec<H> = if (x1 - x0) * (y1 - y0) >= 16384 {
                    super::on_pool(|| (y0..y1).into_par_iter().with_min_len(8).map(|y| lane_hash(px_bytes(&buf[y * w + x0..y * w + x1]))).collect())
                } else {
                    (y0..y1).map(|y| lane_hash(px_bytes(&buf[y * w + x0..y * w + x1]))).collect()
                };
                rows.into_iter().fold((K1, K3), comb)
            }
        }
    }

    /// After the prim of a `Miss` was drawn: keeps the destination rectangle it made.
    pub(super) fn pm_end(&mut self, tok: Tok) {
        let Tok::Miss(key, dest) = tok else { return };
        let Some(mut pm) = self.pm.take() else { return };
        let t0 = self.prof.is_some().then(std::time::Instant::now);
        let out = match dest {
            Dest::Vram { lo, hi } => Out::Vram { lo, data: self.vram[lo..hi].to_vec() },
            Dest::Ov { disp, x0, x1, y0, y1 } => {
                let w = self.cur.w as usize;
                let buf: &[[f32; 4]] = match (disp, self.dcur.as_ref()) {
                    (true, Some(d)) => d,
                    _ => &self.cur.px,
                };
                let mut data = Vec::with_capacity((x1 - x0) * (y1 - y0));
                for y in y0..y1 {
                    data.extend_from_slice(&buf[y * w + x0..y * w + x1]);
                }
                Out::Ov { disp, x0, x1, y0, y1, data }
            }
        };
        let e = Entry { out, used: pm.tick };
        pm.bytes += e.size();
        if let Some(old) = pm.map.insert(key, e) {
            pm.bytes -= old.size();
        }
        if pm.bytes > PM_BYTES {
            // Drop what the last fields did not use; everything when that is not enough.
            let tick = pm.tick;
            let stale: Vec<H> = pm.map.iter().filter(|(_, e)| e.used + 2048 < tick).map(|(k, _)| *k).collect();
            for k in stale {
                if let Some(e) = pm.map.remove(&k) {
                    pm.bytes -= e.size();
                }
            }
            if pm.bytes > PM_BYTES {
                pm.map.clear();
                pm.bytes = 0;
            }
        }
        self.pm = Some(pm);
        self.prof_add("pm store", t0);
    }
}
