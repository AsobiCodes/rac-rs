//! `fastfill::sprite` against the generic path (`shade` + `put_h` per pixel): the VRAM bytes must be identical, for random frame /
//! blend / mask / alpha-test states and rectangles, and the states the path does not take must return false untouched.

use super::*;
use std::sync::atomic::Ordering;
use std::sync::Mutex;

/// `fastfill::OFF` is global: tests that run a prim through `fastfill` hold this.
static LOCK: Mutex<()> = Mutex::new(());

struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }

    fn bytes(&mut self, n: usize) -> Vec<u8> {
        (0..n).map(|_| (self.next() >> 24) as u8).collect()
    }
}

fn view(vram: &mut [u8], colclamp: u64, pabe: u64, fogcol: u64) -> View {
    View {
        vram: vram.as_mut_ptr(),
        vram_len: vram.len(),
        px: std::ptr::null_mut(),
        w: 0,
        h: 0,
        clut: std::ptr::null(),
        clut_off: 0,
        texa: 0,
        fogcol,
        colclamp,
        pabe,
        tc: std::ptr::null(),
        tcx: 0,
        tcy: 0,
        tcw: 0,
        tch: 0,
        rp: cpu_repl::RpView::NONE,
    }
}

fn state(at: u64, alpha: u64, test: u64, frame: u64, fba: u64) -> St {
    St { at, tex0: 0, tex1: 0, clamp: 0, alpha, test, frame, fba, ofx: 0.0, ofy: 0.0, sc: [0, 2047, 0, 2047], ov: false, zb: None }
}

fn px(rgba: [f32; 4], f: f32) -> Px {
    Px { rgba, s: 0.0, t: 0.0, q: 0.0, u: 0.0, v: 0.0, f, z: 0.0 }
}

/// What the capture did before the fast path: shade once, `put_h` every pixel.
fn generic(v: &View, st: &St, pb: &Px, r: (i32, i32, i32, i32)) {
    if let Some((c, wa)) = v.shade(st, pb) {
        let pt = Pt::new(st, v);
        for y in r.2.max(0)..r.3 {
            for x in r.0.max(0)..r.1 {
                v.put_h(st, &pt, x, y, &c, wa, pb.z);
            }
        }
    }
}

/// Runs both on copies of `base`; the way the capture does (fast path, else generic) must leave the generic path's bytes.
fn same(base: &[u8], st: &St, v0: &View, pb: &Px, r: (i32, i32, i32, i32), par: bool, what: &str) -> bool {
    let (mut a, mut b) = (base.to_vec(), base.to_vec());
    let va = View { vram: a.as_mut_ptr(), ..*v0 };
    let vb = View { vram: b.as_mut_ptr(), ..*v0 };
    let took = fastfill::sprite(&va, st, pb, r, par);
    if !took {
        assert!(a == base, "{what}: the declined fast path wrote VRAM");
        generic(&va, st, pb, r);
    }
    generic(&vb, st, pb, r);
    if a != b {
        let n = a.iter().zip(&b).filter(|(x, y)| x != y).count();
        let first = a.iter().zip(&b).position(|(x, y)| x != y).unwrap();
        panic!("{what}: {n} bytes differ, first at {first:#x}; at {:#x} alpha {:#x} frame {:#x} test {:#x} fba {} rect {r:?} px {:?}", st.at, st.alpha, st.frame, st.test, st.fba, pb.rgba);
    }
    took
}

#[test]
fn in_page_offsets_stay_in_the_page() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let l = &super::super::luts().0;
    assert_eq!(l.len(), 64 * 32);
    assert!(l.iter().all(|&o| o < 8192 && o % 4 == 0));
    let mut seen = vec![false; 2048];
    for &o in l {
        seen[(o / 4) as usize] = true;
    }
    assert!(seen.iter().all(|&s| s), "the table is a permutation of the page's words");
}

#[test]
fn random_states_match_the_generic_path() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rng = Rng(0x1234_5678_9abc_def1);
    let base = rng.bytes(4 << 20);
    let (mut took, mut declined) = (0, 0);
    for case in 0..500 {
        let abe = rng.below(4) != 0;
        let msk_kind = rng.below(5);
        let msk = match msk_kind {
            0 | 1 => 0u64,
            2 => 0xffff_ffff,
            3 => 0xff00_0000,
            _ => rng.next() & 0xffff_ffff,
        };
        // Mostly PSMCT32; a few other formats and the Z-buffer / overlay flags must decline.
        let psm = if rng.below(10) == 0 { [1u64, 2, 0x13, 0x1b][rng.below(4) as usize] } else { 0 };
        let fbp = if rng.below(4) == 0 { rng.below(0x200) } else { rng.below(0x40) };
        let bw = rng.below(11);
        let frame = fbp | bw << 16 | psm << 24 | msk << 32;
        let alpha = rng.below(4) | rng.below(4) << 2 | rng.below(4) << 4 | rng.below(4) << 6 | rng.below(256) << 32;
        let atst = rng.below(8);
        let test = if rng.below(3) == 0 { 1 | atst << 1 | rng.below(256) << 4 | rng.below(4) << 12 } else { 0 };
        let at = (if abe { 0x40 } else { 0 }) | if rng.below(4) == 0 { 0x20 } else { 0 } | if rng.below(8) == 0 { 8 } else { 0 };
        let mut st = state(at, alpha, test, frame, rng.below(2));
        match rng.below(40) {
            0 => st.ov = true,
            1 => st.zb = Some(Zb { bp: 0, bw: 1, z24: false, msk: false, tst: 1 }),
            2 => st.at |= 0x10,
            _ => {}
        }
        let mut vb = base.clone();
        let v0 = view(&mut vb, rng.below(2), rng.below(2), rng.next() & 0xff_ffff);
        let ch = |r: &mut Rng| (if r.below(3) == 0 { r.below(256) as f32 } else { r.below(300) as f32 * 0.7 }).min(400.0);
        let rgba = [ch(&mut rng), ch(&mut rng), ch(&mut rng), if rng.below(3) == 0 { rng.below(256) as f32 } else { [0.0, 64.0, 127.0, 128.0, 255.0][rng.below(5) as usize] }];
        let pb = px(rgba, rng.below(256) as f32);
        let (x0, y0) = (rng.below(120) as i32 - 3, rng.below(80) as i32 - 3);
        let (w, h) = if rng.below(5) == 0 { (rng.below(4) as i32, rng.below(4) as i32) } else { (rng.below(260) as i32, rng.below(120) as i32) };
        let r = (x0, x0 + w, y0, y0 + h);
        if same(&base, &st, &v0, &pb, r, false, &format!("case {case}")) {
            took += 1;
        } else {
            declined += 1;
        }
    }
    assert!(took > 300, "the fast path took {took} of 500 cases ({declined} declined)");
    assert!(declined > 10, "the fallback cases ran");
}

/// Page-aligned clears (whole-page fills) and the wrap at the end of local memory.
#[test]
fn aligned_and_wrapping_fills_match() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rng = Rng(77);
    let base = rng.bytes(4 << 20);
    for (fbp, bw, r) in [
        (0u64, 8u64, (0, 512, 0, 128)),
        (0, 8, (0, 512, 0, 127)),
        (0, 8, (64, 448, 32, 96)),
        (0, 8, (1, 511, 1, 127)),
        (0x1df, 8, (0, 512, 0, 127)),
        (0x1df, 8, (0, 512, 0, 128)),
        (0x1f0, 8, (0, 512, 0, 448)),
        (0x100, 2, (0, 130, 0, 70)),
        (0x1ff, 3, (0, 190, 0, 100)),
        (0, 0, (0, 200, 0, 70)),
    ] {
        for (msk, abe, test) in [(0u64, false, 0u64), (0, true, 0), (0xff00_ffff, false, 0), (0, false, 1 | 1 << 12 | 128 << 4 | 5 << 1)] {
            let frame = fbp | bw << 16 | msk << 32;
            let st = state(if abe { 0x40 } else { 0 }, 0x44, test, frame, 0);
            let mut vb = base.clone();
            let v0 = view(&mut vb, 1, 0, 0);
            let pb = px([10.0, 200.0, 77.0, 100.0], 0.0);
            same(&base, &st, &v0, &pb, r, false, &format!("fbp {fbp:#x} bw {bw} rect {r:?} msk {msk:#x}"));
        }
    }
}

/// Big sprites run their rows on the pool; every blend coefficient and clamp setting.
#[test]
fn parallel_blends_match() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rng = Rng(5);
    let base = rng.bytes(4 << 20);
    for al in 0..=255u64 {
        let alpha = (al & 0xff) | 0x60 << 32;
        for clamp in [0, 1] {
            let st = state(0x40, alpha, 0, 0x10 | 8 << 16, (al & 1) as u64);
            let mut vb = base.clone();
            let v0 = view(&mut vb, clamp, 0, 0);
            let pb = px([(al * 3 % 256) as f32, 255.0, (al * 7 % 256) as f32, (al * 5 % 256) as f32], 0.0);
            let big = al % 16 == 0;
            let r = if big { (0, 512, 0, 300) } else { (3, 40, 2, 200) };
            same(&base, &st, &v0, &pb, r, big, &format!("alpha {alpha:#x} clamp {clamp}"));
        }
    }
}

/// One prim's GS registers (context 1).
#[derive(Clone, Copy)]
struct Scn {
    at: u64,
    tex0: u64,
    tex1: u64,
    clamp: u64,
    alpha: u64,
    test: u64,
    frame: u64,
    fba: u64,
    colclamp: u64,
    pabe: u64,
}

fn gs_for(vram: Vec<u8>, rng: &mut Rng) -> Gs {
    let mut g = Gs::new(vram);
    g.gpu = None;
    g.pm = None;
    for c in g.clut.iter_mut() {
        *c = [(rng.next() & 0xff) as f32, (rng.next() & 0xff) as f32, (rng.next() & 0xff) as f32, [0.0, 64.0, 128.0, 255.0][rng.below(4) as usize]];
    }
    g
}

fn set(g: &mut Gs, s: &Scn) {
    g.r[0x1a] = 1;
    g.r[0] = s.at;
    g.r[0x06] = s.tex0;
    g.r[0x4c] = s.frame;
    g.r[0x47] = s.test;
    g.r[0x42] = s.alpha;
    g.r[0x18] = 0;
    g.r[0x40] = 2047 << 16 | 2047 << 48;
    g.r[0x14] = s.tex1;
    g.r[0x08] = s.clamp;
    g.r[0x4a] = s.fba;
    g.r[0x46] = s.colclamp;
    g.r[0x49] = s.pabe;
}

fn vtx(x: f32, y: f32, rgba: [u8; 4], st: (f32, f32, f32), uv: (f32, f32)) -> Vtx {
    Vtx { x: (x * 16.0) as u16, y: (y * 16.0) as u16, z: 0, rgba, q: st.2, s: st.0, t: st.1, u: (uv.0 * 16.0) as u16, v: (uv.1 * 16.0) as u16, f: 100 }
}

/// The prims of a scene: a textured triangle pair (a strip) and a textured sprite, over a (x, y) box.
fn draw_scene(g: &mut Gs, s: &Scn, box_: (f32, f32, f32, f32), rgba: [u8; 4]) {
    let (x0, y0, x1, y1) = box_;
    let (tw, th) = (((1u32 << ((s.tex0 >> 26) & 0xf).min(10)) as f32), ((1u32 << ((s.tex0 >> 30) & 0xf).min(10)) as f32));
    let v = |x: f32, y: f32, fx: f32, fy: f32| vtx(x, y, rgba, (fx, fy, 1.0), (fx * tw, fy * th));
    let (a, b, c, d) = (v(x0, y0, 0.0, 0.0), v(x1, y0 + 3.0, 1.0, 0.0), v(x0 + 2.0, y1, 0.0, 1.0), v(x1, y1, 1.0, 1.0));
    set(g, s);
    g.draw(4, &[a, b, c]);
    g.draw(4, &[b, c, d]);
    g.draw(6, &[a, d]);
}

fn rand_scn(rng: &mut Rng) -> Scn {
    let psm = if rng.below(8) == 0 { [1u64, 2, 0x13][rng.below(3) as usize] } else { 0 };
    let msk = [0u64, 0, 0, 0xff00_0000, 0xffff_ffff, rng.next() & 0xffff_ffff][rng.below(6) as usize];
    let frame = 0x1df | 8 << 16 | psm << 24 | msk << 32;
    let tpsm = [0u64, 0x13, 0x13, 0x14, 0x02, 0x01][rng.below(6) as usize];
    let (tw, th) = (rng.below(8) + 1, rng.below(8) + 1);
    let tex0 = 0x3000 | 2 << 14 | tpsm << 20 | tw << 26 | th << 30 | (rng.below(2) << 34) | (rng.below(4) << 35);
    let alpha = rng.below(4) | rng.below(4) << 2 | rng.below(4) << 4 | rng.below(4) << 6 | rng.below(256) << 32;
    let test = if rng.below(3) == 0 { 1 | rng.below(8) << 1 | rng.below(256) << 4 | rng.below(4) << 12 } else { 0 };
    let at = 0x10 | (if rng.below(4) != 0 { 0x40 } else { 0 }) | (if rng.below(2) == 0 { 0x20 } else { 0 }) | (if rng.below(2) == 0 { 8 } else { 0 }) | (if rng.below(3) == 0 { 0x100 } else { 0 });
    let tex1 = if rng.below(4) == 0 { 0x20 } else { 0 };
    Scn { at, tex0, tex1, clamp: rng.below(16), alpha, test, frame, fba: rng.below(2), colclamp: rng.below(2), pabe: rng.below(2) }
}

/// Textured triangles and sprites into VRAM: the whole capture path (`Gs::draw`) with the fast pixel writes against the same prims
/// with every fast path off.
#[test]
fn textured_prims_match_with_the_fast_paths_off() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rng = Rng(0xfeed_f00d);
    let base = rng.bytes(4 << 20);
    for case in 0..160 {
        let s = rand_scn(&mut rng);
        let rgba = [(rng.next() & 0xff) as u8, 128, (rng.next() & 0xff) as u8, [0u8, 64, 128, 200][rng.below(4) as usize]];
        let (x0, y0) = (rng.below(60) as f32 + 0.25, rng.below(40) as f32 + 0.5);
        let (w, h) = (rng.below(150) as f32 + 3.0, rng.below(90) as f32 + 3.0);
        let seed = rng.next();
        let mut res = Vec::new();
        for off in [true, false] {
            fastfill::OFF.store(off, Ordering::Relaxed);
            let mut g = gs_for(base.clone(), &mut Rng(seed));
            draw_scene(&mut g, &s, (x0, y0, x0 + w, y0 + h), rgba);
            res.push(std::mem::take(&mut g.vram));
        }
        fastfill::OFF.store(false, Ordering::Relaxed);
        assert!(res[0] == res[1], "case {case}: VRAM differs; at {:#x} tex0 {:#x} alpha {:#x} test {:#x} frame {:#x} fba {} box {x0} {y0} {w} {h}", s.at, s.tex0, s.alpha, s.test, s.frame, s.fba);
    }
}

/// Timing of the prims the vendor draws (cargo test --release --lib fastfill_bench -- --ignored --nocapture).
#[test]
#[ignore]
fn fastfill_bench() {
    let _l = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut rng = Rng(9);
    let base = rng.bytes(4 << 20);
    let tex0 = |psm: u64| 0x3000 | 2 << 14 | psm << 20 | 7 << 26 | 7 << 30 | 1 << 34;
    let blend = |tpsm: u64, st: bool| Scn { at: 0x10 | 0x40 | 8 | if st { 0 } else { 0x100 }, tex0: tex0(tpsm), tex1: 0, clamp: 5, alpha: 0x44, test: 0, frame: 0x1df | 8 << 16, fba: 0, colclamp: 1, pabe: 0 };
    for (name, s, w, h) in [("T8 tri 153x127 blend", blend(0x13, true), 153.0, 127.0), ("CT32 tri 153x127 blend", blend(0, true), 153.0, 127.0), ("T8 16x16 blend", blend(0x13, true), 16.0, 16.0), ("T8 tri 153x127 FST", blend(0x13, false), 153.0, 127.0)] {
        for off in [true, false] {
            fastfill::OFF.store(off, Ordering::Relaxed);
            let n = if w < 50.0 { 3000 } else { 100 };
            let mut g = gs_for(base.clone(), &mut Rng(1));
            draw_scene(&mut g, &s, (10.0, 10.0, 10.0 + w, 10.0 + h), [128, 128, 128, 128]);
            let t = std::time::Instant::now();
            for _ in 0..n {
                draw_scene(&mut g, &s, (10.0, 10.0, 10.0 + w, 10.0 + h), [128, 128, 128, 128]);
            }
            let us = t.elapsed().as_secs_f64() * 1e6 / n as f64;
            println!("{name:28} fast {:5}: {us:8.1} us per tri-pair+sprite, {:6.1} ns per bbox px of the 3 prims", !off, us * 1e3 / (3.0 * w as f64 * h as f64));
        }
    }
    fastfill::OFF.store(false, Ordering::Relaxed);
}
