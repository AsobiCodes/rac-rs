//! rac-rs copy of the oracle GS capture (ee_oracle::gs, oracle/src/gs.rs + gs/draw.rs; keep in step with it): the big
//! prims of a field are drawn on several threads (draw.rs `par_ok`), every pixel bit-identical to the oracle's
//! (scratch_menu/gsbench compares the two). The menus drew at 9 fields/s through the single-threaded original.
//!
//! GS capture (opt-in): the GIF packets the game sends over VIF1 DIRECT and
//! GIF DMA (paths 2 and 3) drive a GS register model, local memory (VRAM,
//! loaded from the savestate's GS.bin) and a software rasterizer for 2D work.
//! The 3D world goes through VU1 XGKICK (path 1) and is not captured: rac-rs
//! draws it. Prims aimed at the display buffers land in an overlay that rac-rs
//! composites over its own frame; everything else (render-to-texture panels)
//! is drawn into VRAM so later prims can sample it.

mod draw;

use crate::gsfull::vif::{Out, Vif1};
use crate::gsfull::vu1::Vu1;

#[derive(Clone, Copy, Default, Debug)]
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

/// A packet for the capture: a VIF1 stream (words, block marks) or GIF path 3 qwords, with the DISPFB then.
pub enum Job {
    Vif1(Vec<u32>, Vec<(usize, u32)>, u64),
    Gif(Vec<u128>, u64),
}

/// A host -> local transfer waiting to be written: its BITBLTBUF, TRXPOS and TRXREG, block range and data.
struct Pend {
    bb: u64,
    pos: u64,
    rr: u64,
    lo: u32,
    hi: u32,
    data: Vec<u128>,
}

#[derive(Default, Clone, Copy)]
struct Trx {
    x: u32,
    y: u32,
}

/// The overlay: premultiplied colour plus how much of the scene shows through (final = rgb + a * scene).
pub struct Overlay {
    pub w: u32,
    pub h: u32,
    /// Rows of the frame (the main buffer's scissor height); the rest is unused.
    pub rows: u32,
    /// The game restored a grabbed frame into the main buffer (the pause menu): the scene is frozen.
    pub frozen: bool,
    /// Drawn straight into the display buffer on a field with no frame copy (the main loop's no-present fields,
    /// 0x15f5b8 != 0): the last shown picture with that field's draws over it.
    pub disp: bool,
    pub px: Vec<[f32; 4]>,
    /// The first `rows` rows as RGBA8, as the CPU capture hands them over (empty with the GPU capture: its `px` holds the
    /// normalised floats and `retail.rs ov_px` converts them).
    pub rgba: Vec<[u8; 4]>,
    /// The sharp HUD (lane G, PC extra): the same picture drawn again at the window's resolution by the GPU capture
    /// (`draw/gpu.rs` hd pass); `retail.rs ov_px` hands it over instead of `px` when it is present. None: the 512-wide picture.
    pub hd: Option<Hd>,
}

/// An overlay drawn at the window's resolution (RGBA8, `w` x `rows`): the 4:3 frame fills the window height.
pub struct Hd {
    pub w: u32,
    pub rows: u32,
    pub rgba: Vec<[u8; 4]>,
}

/// The sharp HUD's scale, set by the main thread every frame (`set_hd_target`): the GS pixel is `kx / 256` by `ky / 256` window
/// pixels, and the frame has `rows` rows; packed `kx | ky << 16 | rows << 32`; 0 = off (window too small, `hud=original`,
/// RAC_PC_EXT=0).
static HD_TARGET: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Rows of the last published frame (the main buffer's scissor height: 448 in play on both discs, other in some screens): the
/// next frame's sharp scale assumes them (a frame with other rows has no sharp picture, then the scale follows).
static HD_SEEN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// The highest window height the sharp HUD draws at (the planes' size, `draw/gpu.rs` `HPS`); taller windows scale it up.
pub const HD_MAX_H: u32 = 1536;

/// (kx, ky, rows) of the sharp HUD, 0 = off.
pub(crate) fn hd_target() -> (u32, u32, u32) {
    let t = HD_TARGET.load(std::sync::atomic::Ordering::Relaxed);
    ((t & 0xffff) as u32, ((t >> 16) & 0xffff) as u32, (t >> 32) as u32)
}

/// A frame of `rows` rows was published.
pub(crate) fn hd_note_rows(rows: u32) {
    HD_SEEN.store(rows, std::sync::atomic::Ordering::Relaxed);
}

/// Rows of the game's frame until the first one is published: 448 on the USA disc, 512 on PAL (`draw.rs disp_rows`).
fn hd_rows() -> u32 {
    if ee_oracle::ver::ntsc() {
        448
    } else {
        512
    }
}

/// The fewest output (window) pixels per GS row at which the sharp HUD is drawn; a smaller window gets the 512 picture as before.
/// Below 2.0 the second picture adds little (a 1280 x 800 Steam Deck window has 1.79 per row of the 448-row frame, 1080p has 2.41),
/// and its fill cost (the pause menu and the ring, 0.5 - 2.2 ms on a fast desktop GPU) is not worth it on a slow one.
pub(crate) const HD_MIN_PX_PER_ROW: f32 = 2.0;

/// The sharp HUD for a window `window_h` pixels high (`on`: the option and the extension allow it). Off below `HD_MIN_PX_PER_ROW`
/// window pixels per GS row; above `HD_MAX_H` the picture is drawn there and scaled up.
pub fn set_hd_target(window_h: u32, on: bool) {
    let seen = HD_SEEN.load(std::sync::atomic::Ordering::Relaxed);
    let rows = if (64..=512).contains(&seen) { seen } else { hd_rows() };
    let t = if on && window_h as f32 >= rows as f32 * HD_MIN_PX_PER_ROW {
        let h = window_h.min(HD_MAX_H);
        (h * 2 / 3) as u64 | ((h * 256 / rows) as u64) << 16 | (rows as u64) << 32
    } else {
        0
    };
    HD_TARGET.store(t, std::sync::atomic::Ordering::Relaxed);
}

/// The size of the sharp HUD planes for a scale: width 512 GS pixels, `rows` rows.
pub(crate) fn hd_dims(kx: u32, ky: u32, rows: u32) -> (u32, u32) {
    (2 * kx, (rows * ky).div_ceil(256))
}

pub struct Gs {
    pub vram: Vec<u8>,
    // VIF1 stream state
    vif_skip: u32,
    vif_direct: u32,
    vif_cl: u32,
    vif_wl: u32,
    vif_bad: bool,
    vif_unpack: bool,
    vif_phase: u32,
    vif_hist: Vec<(usize, u32)>,
    /// Debug: (word offset, source address, qwords, tag id) per block of the current VIF1 kick.
    pub blocks: Vec<(usize, u32, u32, u32)>,
    qw: [u32; 4],
    qn: usize,
    // GIF
    tag: Tag,
    pub r: [u64; 0x80],
    rgbaq: ([u8; 4], f32),
    st: (f32, f32),
    packed_q: f32,
    uv: (u16, u16),
    fog: u8,
    vq: Vec<Vtx>,
    trx: Trx,
    pub cur: Overlay,
    /// The last finished overlay (published when the game starts drawing into the other display buffer).
    pub done: Option<Overlay>,
    /// [PC] (speed) Lazy readback (`set_lazy`): the overlay being read back (tag = END jobs run + 1 when it was published) and the
    /// finished ones, oldest first (`gpu_finish`); `ends_run` counts the END jobs run.
    pend_o: Option<(u64, Overlay)>,
    pub ready: std::collections::VecDeque<(u64, Overlay)>,
    ends_run: u64,
    lazy: bool,
    /// The last published overlay (unnormalised): what the display buffer shows until the next copy.
    held: Vec<[f32; 4]>,
    held_rows: u32,
    /// CPU capture: the rows of `cur.px`, `held` and `dcur` that may differ from the blank pixel (`Rows`); every pass over those
    /// buffers (blanking, the RGBA8 conversion) visits these rows only.
    cur_dirty: draw::Rows,
    held_dirty: draw::Rows,
    dcur_dirty: draw::Rows,
    /// The previous display picture's buffer, reused for the next `dcur` (no allocation per no-present field).
    dspare: Vec<[f32; 4]>,
    /// RAC_GSCAP_CHECK=1 (debug): every CPU overlay is converted again the old way (whole buffer) and compared, and the blank-row
    /// invariant is checked; mismatches are counted and printed.
    check: bool,
    /// This field's display-buffer picture while it is drawn, and whether the current prim draws into it.
    dcur: Option<Vec<[f32; 4]>>,
    to_disp: bool,
    /// A frame copy into the display buffer ran since the last field end.
    copied: bool,
    pend: Vec<Pend>,
    /// [PC] (speed) A bit per block (16384 = the 4 MB of local memory) set for every block range in `pend`: the USA disc keeps
    /// ~1500 uploads waiting (PAL ~50) and `flush` / `trx_start` scanned the whole list for each of ~22000 draw checks a field.
    /// A range with no bit set overlaps no entry, so those checks skip the scan; a range that is empty or beyond the
    /// memory is treated as overlapping everything. Rebuilt after `flush_work`, set by `trx_start`.
    pbits: [u64; 256],
    deferring: bool,
    main_fbp: Option<u32>,
    last_copy: bool,
    /// Main buffer width (64-pixel units, from the copy's TBW) and its end block (rows the frame scissors).
    main_bw: u32,
    main_end: u32,
    /// VU1 microprogram starts (MSCAL/MSCALF/MSCNT) since the last overlay was published.
    mscal: u32,
    /// DISPFB of the displayed circuit (set by the DMA kick).
    pub dispfb: u64,
    /// Overlays published so far.
    pub frames: u64,
    clut: Vec<[f32; 4]>,
    clut_off: usize,
    cbp0: u32,
    cbp1: u32,
    pub stats: std::collections::BTreeMap<String, u64>,
    pub log: bool,
    /// The worker's `DRAW_OFF` marker is in force: register state, texture uploads and local copies still run (the game's VRAM
    /// stays exact), prims are not rasterized (the front end is the mirror's picture, so its draws are of no use).
    no_draw: bool,
    /// The worker's `FULL_ON` marker is in force (front end mode 6: the New Game caption screens and the fades around the
    /// intro movies): every prim into the display buffer is drawn into the display picture (`draw`), prims into other
    /// buffers (the fade's main-buffer copies, never sampled) are skipped.
    full: bool,
    /// The worker's `LOAD_ON` marker is in force (the front end's level-load loop, retail.rs `in_load_loop`): the loop's
    /// main-to-display copy sprites are drawn untextured in the main buffer's clear colour (`draw_in`), not from stale VRAM.
    load: bool,
    /// Field-end markers seen so far, and RAC_GSCAP_TRACE=FROM-TO (debug): the prims, transfers and jobs of those fields are logged.
    pub fields: u64,
    trace: Option<(u64, u64)>,
    /// RAC_GSCAP_PROF=1 (debug): wall time per kind of work, printed every 100 fields.
    prof: Option<std::collections::HashMap<String, (u64, u64)>>,
    ras_ns: u64,
    /// The big-prim memo (draw/pmemo.rs); None with RAC_GSCAP_PMEMO=0.
    pm: Option<Box<draw::pmemo::PMemo>>,
    /// `tex_cache`'s decoded texels and their rectangle (x, y, w, h).
    tcache: Vec<[f32; 4]>,
    tcr: (i32, i32, u32, u32),
    /// P1 mode (`P1_ON`): the VU1 model of the path-1 jobs of a render-to-texture menu page, see `P1`.
    p1: Option<Box<P1>>,
    /// Directory whose vu1Memory.bin / vu1MicroMem.bin start the model (the savestate; none on a cold boot).
    p1_dir: String,
    /// The path-1 packet being fed (`p1_pump`): only prims into buffers other than the display / main ones are drawn.
    p1_only: bool,
    /// DIRECT qwords of the current VIF1 job fed so far (the anchor of the queued path-1 packets).
    p1_nd: u32,
    /// RAC_GSCAP_GPU: the pixel work runs on the GPU (draw/gpu.rs), which then holds local memory and the overlay pictures.
    gpu: Option<Box<draw::gpu::Gpu>>,
    /// GPU capture: qwords of the current host -> local transfer so far (`trx` of the CPU path).
    gq: u32,
    /// The GPU capture takes over at the first field end from this field on once it is ready (`gpu_attach`); 0 none.
    gpu_wait: u64,
    /// [PC] (speed) The last block spans `draw_in` / `gpu_raster` asked for, keyed by the registers they derive from (tiny prims
    /// repeat the same texture and scissor): TEX0 -> texture span, (FRAME, SCISSOR) -> scissor span, (TEX0, CLAMP) -> sampled span.
    sp_t: Option<(u64, (u32, u32))>,
    sp_f: Option<((u64, u64), (u32, u32))>,
    sp_r: Option<((u64, u64), (u32, u32))>,
}


/// P1 mode: the menu page draws an actor (the post-game page's green robot, class 1957) through VU1 (path 1) into a
/// render-to-texture panel; the capture draws only paths 2 and 3, so the panel stayed empty. While a page that does
/// this is up (retail.rs sends `P1_ON` / `P1_OFF`), the job's VIF1 stream also runs through the GS-full VIF1 + VU1
/// model (gsfull/vif.rs, vu1.rs: the same programs `--gsfull` runs). Its path-1 packets are fed to the capture
/// between the DIRECT qwords where the program ran (`p1_pump`), drawing only into non-display buffers (`p1_only`),
/// with the Z buffer on (draw.rs `ZB`).
struct P1 {
    vif: Vif1,
    vu: Vu1,
    out: Out,
    /// Path-1 packets of the current job: (DIRECT qwords before it, the qwords).
    q: std::collections::VecDeque<(u32, Vec<u128>)>,
}

/// VU1 programs a frame starts before its scene: the sky's own (0 on Eudora, 2 on Rilgar); the scene starts ~2700.
const SKY_PROGRAMS: u32 = 64;

/// The DISPFB word of the worker's "a movie is up" marker (an empty `Job::Gif`).
pub const MOVIE_SHOWN: u64 = u64::MAX - 1;
/// Worker markers (empty `Job::Gif`): from here on the capture only keeps the GS state and VRAM transfers (`no_draw`) / draws again.
pub const DRAW_OFF: u64 = u64::MAX - 2;
pub const DRAW_ON: u64 = u64::MAX - 3;
/// Worker markers: the front end shows the GS's own display-buffer picture (see `Gs::full`) / is back to the mirror's picture.
pub const FULL_ON: u64 = u64::MAX - 4;
pub const FULL_OFF: u64 = u64::MAX - 5;
/// Worker markers: a menu page with a VU1-drawn render-to-texture actor is up (see `P1`) / is gone.
pub const P1_ON: u64 = u64::MAX - 6;
pub const P1_OFF: u64 = u64::MAX - 7;
/// Worker markers: the front end runs its level-load loop (see `Gs::load`) / left it.
pub const LOAD_ON: u64 = u64::MAX - 8;
pub const LOAD_OFF: u64 = u64::MAX - 9;

const W: u32 = 512;
const H: u32 = 512;

impl Drop for Gs {
    fn drop(&mut self) {
        if let Some(g) = self.gpu.take() {
            draw::gpu::Gpu::recycle(g);
        }
    }
}

/// Starts opening the GPU capture in the background (draw/gpu.rs `Gpu::prestart`); `idle` marks its threads background work.
pub fn gpu_prestart(idle: fn()) {
    draw::gpu::Gpu::prestart(idle);
}

/// Opens a GPU capture and drops it (`rac-rs warm-gpu`): false without one.
pub fn gpu_warm() -> bool {
    matches!(draw::gpu::Gpu::take(&vec![0; 4 << 20]), draw::gpu::Take::Ready(_))
}

impl Gs {
    pub fn new(vram: Vec<u8>) -> Gs {
        let (gpu, gpu_wait) = match draw::gpu::Gpu::take(&vram) {
            draw::gpu::Take::Ready(g) => (Some(g), 0),
            draw::gpu::Take::Later => (None, 1),
            draw::gpu::Take::Never => (None, 0),
        };
        Gs {
            vram,
            vif_skip: 0,
            vif_direct: 0,
            vif_cl: 1,
            vif_wl: 1,
            vif_bad: false,
            vif_unpack: false,
            vif_phase: 0,
            vif_hist: Vec::new(),
            blocks: Vec::new(),
            qw: [0; 4],
            qn: 0,
            tag: Tag::default(),
            r: [0; 0x80],
            rgbaq: ([0x80; 4], 1.0),
            st: (0.0, 0.0),
            packed_q: 1.0,
            uv: (0, 0),
            fog: 0,
            vq: Vec::new(),
            trx: Trx::default(),
            cur: Overlay { w: W, h: H, rows: H, frozen: false, disp: false, px: vec![[0.0, 0.0, 0.0, 1.0]; (W * H) as usize], rgba: Vec::new(), hd: None },
            done: None,
            pend_o: None,
            ready: Default::default(),
            ends_run: 0,
            lazy: false,
            held: vec![[0.0, 0.0, 0.0, 1.0]; (W * H) as usize],
            held_rows: H,
            dcur: None,
            cur_dirty: Default::default(),
            held_dirty: Default::default(),
            dcur_dirty: Default::default(),
            dspare: Vec::new(),
            check: std::env::var_os("RAC_GSCAP_CHECK").is_some(),
            to_disp: false,
            copied: false,
            main_fbp: None,
            last_copy: false,
            pend: Vec::new(),
            pbits: [0; 256],
            deferring: false,
            main_bw: 8,
            main_end: 0,
            mscal: 0,
            dispfb: 0,
            frames: 0,
            clut: vec![[0.0; 4]; 512],
            clut_off: 0,
            cbp0: 0,
            cbp1: 0,
            stats: Default::default(),
            log: std::env::var("ORACLE_GSLOG").is_ok(),
            no_draw: false,
            full: false,
            load: false,
            fields: 0,
            prof: std::env::var("RAC_GSCAP_PROF").is_ok().then(Default::default),
            ras_ns: 0,
            pm: if gpu.is_some() { None } else { draw::pmemo::PMemo::new() },
            tcache: Vec::new(),
            tcr: (0, 0, 0, 0),
            p1: None,
            p1_dir: String::new(),
            p1_only: false,
            p1_nd: 0,
            gpu,
            gq: 0,
            gpu_wait,
            sp_t: None,
            sp_f: None,
            sp_r: None,
            trace: std::env::var("RAC_GSCAP_TRACE").ok().and_then(|v| {
                let (a, b) = v.split_once('-')?;
                Some((a.parse().ok()?, b.parse().ok()?))
            }),
        }
    }


    /// [PC] (speed) Lazy readback: a published overlay is collected later (`gpu_finish`, `ready`) while the capture records the
    /// next job; the caller must not let `ends` pass `pending_tag`.
    pub fn set_lazy(&mut self, on: bool) {
        self.lazy = on;
        if let Some(g) = self.gpu.as_mut() {
            g.lazy = on;
        }
    }

    fn tracing(&self) -> bool {
        self.trace.is_some_and(|(a, b)| (a..=b).contains(&self.fields))
    }

    fn stat(&mut self, k: String) {
        *self.stats.entry(k).or_insert(0) += 1;
    }


    /// VIF1 words (DMA order, TTE words included). `marks` gives (word index, phase in its qword): a
    /// DMA block starts at phase 0, TTE words at phase 2. Only DIRECT/DIRECTHL data reaches the GIF.
    pub fn run(&mut self, job: Job) {
        let t0 = self.prof.is_some().then(std::time::Instant::now);
        let kind = match &job { Job::Vif1(..) => "job vif1", Job::Gif(q, _) if q.is_empty() => "job marker", _ => "job gif" };
        let end = matches!(&job, Job::Gif(q, u64::MAX) if q.is_empty());
        self.run_job(job);
        if end && self.gpu_wait != 0 && self.fields >= self.gpu_wait {
            self.gpu_attach();
        }
        if self.gpu.is_some() {
            self.gpu_end_job(end);
        }
        if end {
            self.ends_run += 1;
        }
        self.prof_add(kind, t0);
    }

    fn prof_add_ns(&mut self, k: &str, ns: u64) {
        if let Some(p) = self.prof.as_mut() {
            let e = p.entry(k.to_string()).or_insert((0, 0));
            e.0 += 1;
            e.1 += ns;
        }
    }

    fn prof_add(&mut self, k: &str, t0: Option<std::time::Instant>) {
        if let (Some(t0), Some(p)) = (t0, self.prof.as_mut()) {
            let e = p.entry(k.to_string()).or_insert((0, 0));
            e.0 += 1;
            e.1 += t0.elapsed().as_nanos() as u64;
        }
    }

    pub fn prof_dump(&mut self) {
        if let Some(p) = self.prof.as_mut() {
            let mut v: Vec<_> = p.drain().collect();
            v.sort_by_key(|(_, (_, ns))| std::cmp::Reverse(*ns));
            let sum = |pre: &str| v.iter().filter(|(k, _)| k.starts_with(pre)).map(|(_, (_, ns))| *ns).sum::<u64>() as f64 / 1e8;
            let mut o = format!("gscap prof (100 fields, ms/field; prim memo {}; totals: raster {:.2} draw {:.2} flush {:.2} job {:.2}):", self.pm_stats(), sum("RASTER"), sum("draw"), sum("flush"), sum("job"));
            for (k, (n, ns)) in v.iter().take(60) {
                o += &format!(" [{k}: {:.2} ms x{:.1}]", *ns as f64 / 1e8, *n as f64 / 100.0);
            }
            eprintln!("{o}");
            let g = self.gpu_stats();
            if !g.is_empty() {
                eprintln!("gscap gpu stats: {g}");
            }
        }
    }

    fn run_job(&mut self, job: Job) {
        match job {
            Job::Vif1(w, marks, d) => {
                if self.tracing() {
                    eprintln!("gscap f{} vif1 {} words dispfb {d:x}", self.fields, w.len());
                }
                self.dispfb = d;
                self.vif1_reset();
                if self.p1.is_some() {
                    let t0 = self.prof.is_some().then(std::time::Instant::now);
                    self.p1_feed(&w, &marks);
                    self.prof_add("p1 feed (VIF1 + VU1 model)", t0);
                }
                self.vif1(&w, &marks);
                self.p1_pump(true);
                self.blocks.clear();
                // [PC] (speed) the word buffer goes back to the machine's VIF1 kick (ee_oracle::gs::take_words).
                ee_oracle::gs::recycle_words(w);
            }
            // The field's end (retail.rs worker, after each field): a display-buffer picture drawn this field shows now.
            Job::Gif(qs, u64::MAX) if qs.is_empty() => {
                if self.tracing() {
                    eprintln!("gscap f{} END no_draw {}", self.fields, self.no_draw);
                }
                self.field_end();
                self.fields += 1;
                if self.fields % 100 == 0 {
                    self.prof_dump();
                }
            }
            // A movie went up (retail.rs worker): the player's picture replaced the display buffer.
            Job::Gif(qs, MOVIE_SHOWN) if qs.is_empty() => self.movie_shown(),
            Job::Gif(qs, DRAW_OFF) if qs.is_empty() => self.no_draw = true,
            Job::Gif(qs, DRAW_ON) if qs.is_empty() => self.no_draw = false,
            Job::Gif(qs, FULL_ON) if qs.is_empty() => self.full_mode(true),
            Job::Gif(qs, FULL_OFF) if qs.is_empty() => self.full_mode(false),
            Job::Gif(qs, P1_ON) if qs.is_empty() => self.p1_on(),
            Job::Gif(qs, P1_OFF) if qs.is_empty() => self.p1 = None,
            Job::Gif(qs, LOAD_ON) if qs.is_empty() => self.load = true,
            Job::Gif(qs, LOAD_OFF) if qs.is_empty() => self.load = false,
            Job::Gif(qs, d) => {
                if self.tracing() {
                    eprintln!("gscap f{} gif {} qwords dispfb {d:x}", self.fields, qs.len());
                }
                self.dispfb = d;
                for q in qs {
                    self.gif(q);
                }
            }
        }
    }

    /// `FULL_ON` / `FULL_OFF`: the display picture starts over (transparent until the first clear sprite draws), the gameplay
    /// state (main buffer, copy tracking) is forgotten. Off, the capture keeps only the VRAM traffic again (`no_draw`).
    fn full_mode(&mut self, on: bool) {
        self.full = on;
        self.load = false;
        self.no_draw = !on;
        if self.gpu.is_some() {
            self.gpu_fill(draw::gpu::HELD);
        } else {
            self.held.fill([0.0, 0.0, 0.0, 1.0]);
            self.held_dirty = Default::default();
        }
        self.held_rows = H;
        self.dcur = None;
        self.done = None;
        self.copied = false;
        self.main_fbp = None;
        self.last_copy = false;
        self.to_disp = false;
    }

    /// `run` for a job the oracle machine queued (`Mem::gsq` sends ee_oracle's own type).
    pub fn run_oracle(&mut self, job: ee_oracle::gs::Job) {
        self.run(match job {
            ee_oracle::gs::Job::Vif1(w, m, d) => Job::Vif1(w, m, d),
            ee_oracle::gs::Job::Gif(q, d) => Job::Gif(q, d),
        })
    }

    pub fn vif1(&mut self, w: &[u32], marks: &[(usize, u32)]) {
        if let Ok(a) = std::env::var("ORACLE_VIFDUMP") {
            if let Ok(a) = usize::from_str_radix(&a, 16) {
                if a + 64 <= w.len() && self.stats.get("dumped").is_none() {
                    self.stat("dumped".into());
                    eprintln!("dump {a:x}: {:08x?}", &w[a..a + 64]);
                }
            }
        }
        for (k, &(at, ph)) in marks.iter().enumerate() {
            let end = marks.get(k + 1).map(|m| m.0).unwrap_or(w.len()).min(w.len());
            self.vif_phase = ph;
            let mut i = at;
            while i < end {
                // UNPACK data in bulk (the last word goes through vif_word for its pad check).
                if self.vif_skip > 1 && self.vif_direct == 0 {
                    let k = ((self.vif_skip - 1) as usize).min(end - i);
                    self.vif_skip -= k as u32;
                    self.vif_phase = (self.vif_phase + k as u32) & 3;
                    i += k;
                    continue;
                }
                self.vif_word(w[i], i, end - i - 1);
                self.vif_phase = (self.vif_phase + 1) & 3;
                i += 1;
            }
        }
    }

    /// `left`: words after this one in its DMA block.
    fn vif_word(&mut self, word: u32, i: usize, left: usize) {
        if self.vif_direct > 0 {
            self.qw[self.qn] = word;
            self.qn += 1;
            if self.qn == 4 {
                self.qn = 0;
                let q = self.qw;
                if self.p1.is_some() {
                    self.p1_pump(false);
                    self.p1_nd += 1;
                }
                self.gif(q[0] as u128 | (q[1] as u128) << 32 | (q[2] as u128) << 64 | (q[3] as u128) << 96);
            }
            self.vif_direct -= 1;
            return;
        }
        if self.vif_skip > 0 {
            self.vif_skip -= 1;
            // A DMA block whose UNPACK data ends inside its last qword: the pad words are not VIF codes
            // (Eudora vertex packets leave stale words there, some with the i-bit set that would stall
            // VIF1 for good; the game never restarts it) [I].
            if self.vif_skip == 0 && self.vif_unpack && left < 4 && left == (3 - self.vif_phase) as usize {
                self.vif_skip = left as u32;
            }
            if self.vif_skip == 0 {
                self.vif_unpack = false;
            }
            return;
        }
        if self.vif_bad {
            return;
        }
        let code = word;
        if self.log {
            self.vif_hist.push((i * 4 + self.vif_phase as usize, code));
            if self.vif_hist.len() > 60 {
                self.vif_hist.remove(0);
            }
        }
        let cmd = (code >> 24) & 0x7f;
        let imm = code & 0xffff;
        let num = (code >> 16) & 0xff;
        match cmd {
            0x00 | 0x02..=0x07 | 0x10 | 0x11 | 0x13 => {}
            0x14 | 0x15 | 0x17 => self.mscal += 1,
            0x01 => {
                self.vif_cl = imm & 0xff;
                self.vif_wl = (imm >> 8) & 0xff;
            }
            0x20 => self.vif_skip = 1,
            0x30 | 0x31 => self.vif_skip = 4,
            0x4a => self.vif_skip = if num == 0 { 512 } else { num * 2 },
            0x50 | 0x51 => {
                self.vif_direct = if imm == 0 { 65536 * 4 } else { imm * 4 };
                self.qn = 0;
            }
            0x60..=0x7f => {
                let vn = (cmd >> 2) & 3;
                let vl = cmd & 3;
                let n = if num == 0 { 256 } else { num };
                let (cl, wl) = (self.vif_cl, self.vif_wl);
                let items = if wl <= cl || wl == 0 { n } else { (n / wl) * cl + (n % wl).min(cl) };
                let bits = if vl == 3 { 16 } else { (vn + 1) * (32 >> vl) };
                self.vif_skip = (items * bits).div_ceil(32);
                self.vif_unpack = self.vif_skip > 0;
            }
            _ => {
                if self.log {
                    eprintln!("vif1 bad code {code:08x} at word {i} after {:08x?} cl {} wl {}", self.vif_hist, self.vif_cl, self.vif_wl);
                    eprintln!("blocks {:x?}", self.blocks);
                }
                self.stat(format!("vif1 bad code {code:08x}"));
                self.vif_bad = true;
            }
        }
    }

    /// Directory with the savestate's VU1 memories (`P1_ON` starts the model from them).
    pub fn set_vu1_dir(&mut self, dir: &str) {
        self.p1_dir = dir.to_string();
    }

    /// `P1_ON`: starts the VIF1 + VU1 model (as `gsfull::pipe::Pipe::new`; zeroed memories without the savestate files).
    fn p1_on(&mut self) {
        if self.p1.is_some() {
            return;
        }
        // RAC_P1_ZERO=1 (debug): start from empty VU1 memories (shows that the page's chains carry their own programs and data).
        let zero = std::env::var_os("RAC_P1_ZERO").is_some();
        let rd = |n: &str| if zero { Vec::new() } else { std::fs::read(format!("{}/{n}", self.p1_dir)).unwrap_or_default() };
        let (d, m) = (rd("vu1Memory.bin"), rd("vu1MicroMem.bin"));
        let w: Vec<u32> = m.get(..64).map(|b| b.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect()).unwrap_or_default();
        let out = Out { prog: crate::gsfull::vif::identify(&w), ..Default::default() };
        self.p1 = Some(Box::new(P1 { vif: Vif1::new(), vu: Vu1::new(&d, &m), out, q: Default::default() }));
        self.p1_nd = 0;
    }

    /// Runs the job's VIF1 stream through the model and queues its path-1 packets, each with the number of DIRECT
    /// qwords (path 2) that come before it in the stream.
    fn p1_feed(&mut self, w: &[u32], marks: &[(usize, u32)]) {
        let Some(p) = self.p1.as_mut() else { return };
        p.out.qs.clear();
        p.out.segs.clear();
        p.q.clear();
        p.vif.feed(w, marks, &mut p.vu, &mut p.out);
        let mut direct = 0u32;
        for (i, &(s, path, _)) in p.out.segs.iter().enumerate() {
            let e = p.out.segs.get(i + 1).map_or(p.out.qs.len(), |n| n.0 as usize);
            match path {
                2 => direct += (e - s as usize) as u32,
                1 => p.q.push_back((direct, p.out.qs[s as usize..e].to_vec())),
                _ => {}
            }
        }
        self.p1_nd = 0;
    }

    /// Feeds the queued path-1 packets that are due before the next DIRECT qword (between GIF packets only), or all of
    /// them (`all`: the end of the job).
    fn p1_pump(&mut self, all: bool) {
        loop {
            let due = |s: &Gs| s.p1.as_ref().and_then(|p| p.q.front()).is_some_and(|f| all || (f.0 <= s.p1_nd && s.tag.nloop == 0));
            if !due(self) {
                return;
            }
            let Some((_, qs)) = self.p1.as_mut().and_then(|p| p.q.pop_front()) else { return };
            self.p1_only = true;
            for q in qs {
                self.gif(q);
            }
            self.p1_only = false;
        }
    }

    /// Resyncs the VIF1 stream at a chain start (a stuck parse never outlives one kick).
    pub fn vif1_reset(&mut self) {
        self.vif_hist.clear();
        if self.vif_bad {
            self.vif_bad = false;
            self.vif_skip = 0;
            self.vif_direct = 0;
        }
    }

    /// One GIF qword (paths 2/3).
    pub fn gif(&mut self, q: u128) {
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

    fn next_reg(&mut self) {
        self.tag.j += 1;
        if self.tag.j == self.tag.nreg {
            self.tag.j = 0;
            self.tag.nloop -= 1;
        }
    }

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
                let v = (q & 0xffff) as u64
                    | (((q >> 32) & 0xffff) as u64) << 16
                    | (((q >> 68) & 0xff_ffff) as u64) << 32
                    | (((q >> 100) & 0xff) as u64) << 56;
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
                self.r[0] = v;
                self.vq.clear();
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
                if self.log && std::env::var("ORACLE_GSSEQ").is_ok() {
                    let (bb, rr) = (self.r[0x50], self.r[0x52]);
                    eprintln!("seq f{} trx dir {} bitblt {bb:x} reg {}x{}", self.frames, v & 3, rr & 0xfff, (rr >> 32) & 0xfff);
                }
                if self.log {
                    let (bb, rr) = (self.r[0x50], self.r[0x52]);
                    self.stat(format!("trx dir {} psm {:x} {}x{} dbp {:x}", v & 3, (bb >> 56) & 0x3f, rr & 0xfff, (rr >> 32) & 0xfff, (bb >> 32) & 0x3fff));
                }
                if self.tracing() {
                    let (bb, pos, rr) = (self.r[0x50], self.r[0x51], self.r[0x52]);
                    eprintln!("gscap f{} trx dir {} bitblt {bb:x} pos {pos:x} reg {}x{}", self.fields, v & 3, rr & 0xfff, (rr >> 32) & 0xfff);
                }
                self.r[0x53] = v;
                self.trx = Trx::default();
                self.gq = 0;
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
                self.r[a as usize] = v;
                self.clut_load(v);
            }
            0x16 | 0x17 => {
                // TEX2: PSM and the CLUT fields replace those of TEX0.
                let k = (a - 0x10) as usize;
                let m = (0x3fu64 << 20) | (!0u64 << 37);
                self.r[k] = (self.r[k] & !m) | (v & m);
                let t = self.r[k];
                self.clut_load(t);
            }
            _ => self.r[a as usize] = v,
        }
    }

    fn kick(&mut self, v: Vtx, draw: bool) {
        let prim = self.r[0];
        let ty = prim & 7;
        self.vq.push(v);
        let n = self.vq.len();
        let need = match ty {
            0 => 1,
            1 | 2 | 6 => 2,
            3..=5 => 3,
            _ => {
                self.vq.clear();
                return;
            }
        };
        if n < need {
            return;
        }
        // [PC] (speed) the prim's vertices on the stack (a Vec clone per prim: the P1 pages draw ~11,000 tiny prims a field).
        let mut vbuf = [Vtx::default(); 3];
        let vheap: Vec<Vtx>;
        let vs: &[Vtx] = if n <= 3 {
            vbuf[..n].copy_from_slice(&self.vq);
            &vbuf[..n]
        } else {
            vheap = self.vq.clone();
            &vheap
        };
        match ty {
            2 => {
                self.vq.remove(0);
            }
            4 => {
                self.vq.remove(0);
            }
            5 => {
                self.vq.remove(1);
            }
            _ => self.vq.clear(),
        }
        if draw && self.no_draw && self.tracing() {
            let at = self.attrs();
            let c = ((at >> 9) & 1) as usize;
            eprintln!(
                "gscap f{} (no_draw) prim ty {ty} at {at:x} fogcol {:x} f {} frame {:x} tme {} abe {} tex0 {:x} alpha {:x} sc {:x} v0 ({},{}) {:?} v1 ({},{}) {:?}",
                self.fields,
                self.r[0x3d],
                vs[0].f,
                self.r[0x4c + c],
                (at >> 4) & 1,
                (at >> 6) & 1,
                self.r[0x06 + c],
                self.r[0x42 + c],
                self.r[0x40 + c],
                vs[0].x / 16,
                vs[0].y / 16,
                vs[0].rgba,
                vs.get(1).map_or(0, |v| v.x / 16),
                vs.get(1).map_or(0, |v| v.y / 16),
                vs.get(1).map(|v| v.rgba)
            );
        }
        if draw && !self.no_draw {
            self.draw(ty as u32, vs);
        }
    }

    /// Effective prim attributes (PRMODECONT.AC = 0 takes them from PRMODE).
    fn attrs(&self) -> u64 {
        if self.r[0x1a] & 1 != 0 { self.r[0] } else { (self.r[0] & 7) | (self.r[0x1b] & !7) }
    }

    /// A host upload into the main buffer (the pause menu restores its grabbed frame each field): the overlay shows
    /// the scene there, which rac-rs draws.
    fn upload_main(&mut self, a: usize) {
        let Some(fb) = self.main_fbp else { return };
        let blk = (a / 256) as u32;
        if blk < fb * 32 || blk >= self.main_end {
            return;
        }
        let word = (a / 4) as u32 - fb * 32 * 64;
        let (page, b, w) = (word / 2048, (word / 64) % 32, word % 64);
        // (block, word) -> (x, y) inside the page: the inverse of the PSMCT32 swizzle (BT32 / CT32), built once.
        static INV: std::sync::OnceLock<Vec<(u8, u8)>> = std::sync::OnceLock::new();
        let inv = INV.get_or_init(|| {
            let mut v = vec![(0u8, 0u8); 2048];
            for (r, row) in BT32.iter().enumerate() {
                for (c, &bb) in row.iter().enumerate() {
                    for (rr, crow) in CT32.iter().enumerate() {
                        for (cc, &ww) in crow.iter().enumerate() {
                            v[bb as usize * 64 + ww as usize] = ((c * 8 + cc) as u8, (r * 8 + rr) as u8);
                        }
                    }
                }
            }
            v
        });
        let (ix, iy) = inv[(b * 64 + w) as usize];
        let (bx, by, cx, cy) = (ix as u32 / 8, iy as u32 / 8, ix as u32 % 8, iy as u32 % 8);
        let bw = self.main_bw.max(1);
        let (x, y) = ((page % bw) * 64 + bx * 8 + cx, (page / bw) * 32 + by * 8 + cy);
        if x < self.cur.w && y < self.cur.h {
            self.cur.px[(y * self.cur.w + x) as usize] = [0.0, 0.0, 0.0, 1.0];
            self.cur_dirty.mark(y as i32, y as i32);
        }
        // The restored frame stands in for the 3D phase: later draws are overlay.
        self.mscal = self.mscal.max(SKY_PROGRAMS);
        self.cur.frozen = true;
    }

    // ---- local memory addressing (GS swizzle) ----

    fn a32(bp: u32, bw: u32, x: u32, y: u32) -> usize {
        let page = (y / 32) * bw + x / 64;
        ((bp * 256 + page * 8192 + luts().0[((y % 32) * 64 + x % 64) as usize]) & 0x3f_ffff) as usize
    }

    fn a16(bp: u32, bw: u32, x: u32, y: u32) -> usize {
        let page = (y / 64) * bw + x / 64;
        ((bp * 256 + page * 8192 + luts().1[((y % 64) * 64 + x % 64) as usize]) & 0x3f_ffff) as usize
    }

    fn a8(bp: u32, bw: u32, x: u32, y: u32) -> usize {
        let page = (y / 64) * (bw / 2).max(1) + x / 128;
        ((bp * 256 + page * 8192 + luts().2[((y % 64) * 128 + x % 128) as usize]) & 0x3f_ffff) as usize
    }

    /// Nibble address.
    fn a4(bp: u32, bw: u32, x: u32, y: u32) -> usize {
        let page = (y / 128) * (bw / 2).max(1) + x / 128;
        ((bp * 512 + page * 16384 + luts().3[((y % 128) * 128 + x % 128) as usize]) & 0x7f_ffff) as usize
    }

    fn rd32(&self, a: usize) -> u32 {
        u32::from_le_bytes(self.vram[a..a + 4].try_into().unwrap())
    }

    fn wr_px(&mut self, psm: u32, bp: u32, bw: u32, x: u32, y: u32, v: u32) {
        match psm {
            0x00 => {
                let a = Self::a32(bp, bw, x, y);
                self.vram[a..a + 4].copy_from_slice(&v.to_le_bytes());
            }
            0x01 => {
                let a = Self::a32(bp, bw, x, y);
                self.vram[a..a + 3].copy_from_slice(&v.to_le_bytes()[..3]);
            }
            0x02 => {
                let a = Self::a16(bp, bw, x, y);
                self.vram[a..a + 2].copy_from_slice(&(v as u16).to_le_bytes());
            }
            0x13 => {
                let a = Self::a8(bp, bw, x, y);
                self.vram[a] = v as u8;
            }
            0x14 => {
                let n = Self::a4(bp, bw, x, y);
                let b = &mut self.vram[n >> 1];
                *b = if n & 1 != 0 { (*b & 0x0f) | ((v as u8) << 4) } else { (*b & 0xf0) | (v as u8 & 0xf) };
            }
            0x1b => {
                let a = Self::a32(bp, bw, x, y);
                self.vram[a + 3] = v as u8;
            }
            0x24 => {
                let a = Self::a32(bp, bw, x, y);
                self.vram[a + 3] = (self.vram[a + 3] & 0xf0) | (v as u8 & 0xf);
            }
            0x2c => {
                let a = Self::a32(bp, bw, x, y);
                self.vram[a + 3] = (self.vram[a + 3] & 0x0f) | ((v as u8) << 4);
            }
            _ => {}
        }
    }

    pub fn rd_px(&self, psm: u32, bp: u32, bw: u32, x: u32, y: u32) -> u32 {
        match psm {
            0x00 => self.rd32(Self::a32(bp, bw, x, y)),
            0x01 => self.rd32(Self::a32(bp, bw, x, y)) & 0xff_ffff,
            0x02 => {
                let a = Self::a16(bp, bw, x, y);
                u16::from_le_bytes([self.vram[a], self.vram[a + 1]]) as u32
            }
            0x13 => self.vram[Self::a8(bp, bw, x, y)] as u32,
            0x14 => {
                let n = Self::a4(bp, bw, x, y);
                (self.vram[n >> 1] >> ((n & 1) * 4)) as u32 & 0xf
            }
            0x1b => self.vram[Self::a32(bp, bw, x, y) + 3] as u32,
            0x24 => self.vram[Self::a32(bp, bw, x, y) + 3] as u32 & 0xf,
            0x2c => (self.vram[Self::a32(bp, bw, x, y) + 3] >> 4) as u32,
            _ => 0,
        }
    }

    /// Bits per pixel of a transfer format.
    fn bpp(psm: u32) -> u32 {
        match psm {
            0x00 => 32,
            0x01 => 24,
            0x02 | 0x0a => 16,
            0x13 | 0x1b => 8,
            _ => 4,
        }
    }

    /// HWREG data (EE -> local memory, TRXDIR 0).
    fn image(&mut self, q: u128) {
        self.image_in(q);
    }

    fn image_in(&mut self, q: u128) {
        if self.r[0x53] & 3 != 0 {
            return;
        }
        if self.deferring {
            if let Some(p) = self.pend.last_mut() {
                p.data.push(q);
                return;
            }
        }
        if self.gpu.is_some() {
            self.gpu_xfer(self.r[0x50], self.r[0x51], self.r[0x52], self.gq, &[q]);
            self.gq += 1;
            return;
        }
        let mut t = std::mem::take(&mut self.trx);
        self.put(self.r[0x50], self.r[0x51], self.r[0x52], &mut t, q);
        self.trx = t;
    }

    /// One HWREG qword of a transfer (BITBLTBUF, TRXPOS, TRXREG) at position `t`.
    fn put(&mut self, bb: u64, pos: u64, rr: u64, t: &mut Trx, q: u128) {
        let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
        let (dx, dy) = (((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
        let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
        if rw == 0 {
            return;
        }
        let bpp = Self::bpp(dpsm);
        let mut bits = q;
        let mut left = 128u32;
        while left >= bpp {
            if t.y >= rh {
                return;
            }
            let v = (bits & ((1u128 << bpp) - 1)) as u32;
            bits >>= bpp;
            left -= bpp;
            self.wr_px(dpsm, dbp, dbw, dx + t.x, dy + t.y, v);
            if dpsm == 0 {
                self.upload_main(Self::a32(dbp, dbw, dx + t.x, dy + t.y));
            }
            t.x += 1;
            if t.x >= rw {
                t.x = 0;
                t.y += 1;
            }
        }
        if left > 0 && bpp == 24 {
            // 24-bit pixels straddle qwords: carry the remainder (rare; not modelled).
            self.stat("image ct24 straddle".into());
        }
    }

    /// TRXDIR 0 start. The game streams its 3D textures every field (~1M texels) and the capture samples few of
    /// them, so a transfer waits in `pend` until a draw, CLUT load or copy touches its blocks; uploads into the
    /// main buffer go through at once (they mark the overlay).
    fn trx_start(&mut self) {
        let (bb, pos, rr) = (self.r[0x50], self.r[0x51], self.r[0x52]);
        let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
        let (dx, dy) = (((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
        let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
        if rw == 0 || rh == 0 {
            return;
        }
        let (lo, hi) = Self::span(dpsm, dbp, dbw, dx, dy, rw, rh);
        let main = self.main_fbp.map_or((0, 0), |f| (f * 32, self.main_end));
        if lo < main.1 && main.0 < hi {
            self.flush(lo, hi);
            return;
        }
        // The same upload again replaces the waiting one (it has the same block range, so with no bit set there is none).
        if Self::pb_any(&self.pbits, lo, hi) {
            self.pend.retain(|p| !(p.bb == bb && p.pos == pos && p.rr == rr));
        }
        self.pend.push(Pend { bb, pos, rr, lo, hi, data: Vec::new() });
        Self::pb_set(&mut self.pbits, lo, hi);
        self.deferring = true;
    }

    /// Block range `[lo, hi)` a rectangle of a buffer touches. [PC] (speed) Memoized per thread (direct-mapped, a pure
    /// function of its arguments): the P1 pages' thousands of tiny prims repeat the same texture and scissor spans,
    /// each ~1000 address calculations (`span_calc`).
    fn span(psm: u32, bp: u32, bw: u32, x: u32, y: u32, w: u32, h: u32) -> (u32, u32) {
        // A rectangle of at most one 8 x 8 block cell of a 32-bit format (a tiny prim's box): its four corners are all
        // `span_calc` visits; cheaper to compute than to look up.
        if matches!(psm, 0x00 | 0x01 | 0x1b | 0x24 | 0x2c) && (1..=8).contains(&w) && (1..=8).contains(&h) {
            let (x1, y1) = (x + w - 1, y + h - 1);
            let b = [Self::a32(bp, bw, x, y), Self::a32(bp, bw, x1, y), Self::a32(bp, bw, x, y1), Self::a32(bp, bw, x1, y1)];
            let (lo, hi) = (b[0].min(b[1]).min(b[2]).min(b[3]), b[0].max(b[1]).max(b[2]).max(b[3]));
            return ((lo / 256) as u32, (hi / 256) as u32 + 1);
        }
        type K = (u32, u32, u32, u32, u32, u32, u32);
        thread_local! {
            static MEMO: std::cell::RefCell<Vec<Option<(K, (u32, u32))>>> = std::cell::RefCell::new(vec![None; 1024]);
        }
        let k: K = (psm, bp, bw, x, y, w, h);
        let mut hs = 0u64;
        for v in [psm, bp, bw, x, y, w, h] {
            hs = (hs ^ v as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            hs ^= hs >> 29;
        }
        let i = (hs >> 32) as usize & 1023;
        MEMO.with(|m| {
            let mut m = m.borrow_mut();
            if let Some((kk, v)) = m[i] {
                if kk == k {
                    return v;
                }
            }
            let v = Self::span_calc(psm, bp, bw, x, y, w, h);
            m[i] = Some((k, v));
            v
        })
    }

    fn span_calc(psm: u32, bp: u32, bw: u32, x: u32, y: u32, w: u32, h: u32) -> (u32, u32) {
        let (sw, sh, f): (u32, u32, fn(u32, u32, u32, u32) -> usize) = match psm {
            0x00 | 0x01 | 0x1b | 0x24 | 0x2c => (8, 8, Self::a32),
            0x02 | 0x0a => (16, 8, Self::a16),
            0x13 => (16, 16, Self::a8),
            0x14 => (32, 16, |b, w, x, y| Self::a4(b, w, x, y) / 2),
            _ => return (0, u32::MAX),
        };
        let (mut lo, mut hi) = (u32::MAX, 0);
        for xx in (x..x + w).step_by(sw as usize).chain(std::iter::once(x + w - 1)) {
            for yy in (y..y + h).step_by(sh as usize).chain(std::iter::once(y + h - 1)) {
                let b = (f(bp, bw, xx, yy) / 256) as u32;
                lo = lo.min(b);
                hi = hi.max(b + 1);
            }
        }
        (lo, hi)
    }

    /// The words of `pbits` and their masks for blocks `[lo, hi)`, 0 < `hi` - `lo`, `hi` <= 16384.
    fn pb_words(lo: u32, hi: u32) -> impl Iterator<Item = (usize, u64)> {
        let (w0, w1) = ((lo >> 6) as usize, ((hi - 1) >> 6) as usize);
        (w0..=w1).map(move |w| {
            let a = if w == w0 { !0u64 << (lo & 63) } else { !0 };
            let b = if w == w1 { !0u64 >> (63 - ((hi - 1) & 63)) } else { !0 };
            (w, a & b)
        })
    }

    /// Sets the blocks `[lo, hi)`; an empty or out-of-memory range sets every bit.
    fn pb_set(bits: &mut [u64; 256], lo: u32, hi: u32) {
        if hi <= lo || hi > 16384 {
            *bits = [!0; 256];
            return;
        }
        for (w, m) in Self::pb_words(lo, hi) {
            bits[w] |= m;
        }
    }

    /// False only when no block of `[lo, hi)` has a bit set (so no `pend` entry overlaps it).
    fn pb_any(bits: &[u64; 256], lo: u32, hi: u32) -> bool {
        if hi <= lo || hi > 16384 {
            return true;
        }
        Self::pb_words(lo, hi).any(|(w, m)| bits[w] & m != 0)
    }

    /// Writes the waiting transfers that touch blocks `[lo, hi)`, with every older one overlapping those, in order.
    fn flush(&mut self, lo: u32, hi: u32) {
        let hit = |p: &Pend, lo: u32, hi: u32| p.lo < hi && lo < p.hi;
        if !Self::pb_any(&self.pbits, lo, hi) || !self.pend.iter().any(|p| hit(p, lo, hi)) {
            return;
        }
        let t0 = self.prof.is_some().then(std::time::Instant::now);
        self.flush_work(lo, hi);
        self.prof_add("flush (deferred uploads written)", t0);
    }

    fn flush_work(&mut self, lo: u32, hi: u32) {
        let hit = |p: &Pend, lo: u32, hi: u32| p.lo < hi && lo < p.hi;
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
        self.pbits = [0; 256];
        for p in &self.pend {
            Self::pb_set(&mut self.pbits, p.lo, p.hi);
        }
        for p in &run {
            if self.gpu.is_some() {
                self.gpu_xfer(p.bb, p.pos, p.rr, 0, &p.data);
                self.gq = p.data.len() as u32;
                continue;
            }
            let mut t = Trx::default();
            for &q in &p.data {
                self.put(p.bb, p.pos, p.rr, &mut t, q);
            }
            self.trx = t;
        }
        // A transfer still receiving data continues straight into VRAM.
        if active {
            self.deferring = false;
        }
    }

    /// Writes every waiting transfer (before reading VRAM from outside).
    pub fn flush_all(&mut self) {
        self.flush(0, u32::MAX);
    }

    /// TRXDIR 2: local -> local copy.
    fn local_copy(&mut self) {
        let bb = self.r[0x50];
        let (sbp, sbw, spsm) = ((bb & 0x3fff) as u32, ((bb >> 16) & 0x3f) as u32, ((bb >> 24) & 0x3f) as u32);
        let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
        let pos = self.r[0x51];
        let (sx, sy, dx, dy) = ((pos & 0x7ff) as u32, ((pos >> 16) & 0x7ff) as u32, ((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
        let rr = self.r[0x52];
        let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
        if self.gpu.is_some() {
            self.gpu_copy();
            self.stat(format!("local copy {rw}x{rh} psm {spsm:x}->{dpsm:x}"));
            return;
        }
        for y in 0..rh {
            for x in 0..rw {
                let v = self.rd_px(spsm, sbp, sbw, sx + x, sy + y);
                self.wr_px(dpsm, dbp, dbw, dx + x, dy + y, v);
            }
        }
        self.stat(format!("local copy {rw}x{rh} psm {spsm:x}->{dpsm:x}"));
    }
}

/// In-page swizzle offsets (bytes; nibbles for PSMT4) of CT32, CT16, T8 and T4, by `y * page width + x`.
type Luts = (Vec<u32>, Vec<u32>, Vec<u32>, Vec<u32>);

fn luts() -> &'static Luts {
    static L: std::sync::OnceLock<Luts> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        let t = |w: u32, h: u32, f: fn(u32, u32, u32, u32) -> usize, bw: u32| (0..w * h).map(|i| f(0, bw, i % w, i / w) as u32).collect();
        (t(64, 32, r32, 1), t(64, 64, r16, 1), t(128, 64, r8, 2), t(128, 128, r4, 2))
    })
}

/// In-page swizzle offsets (bytes) of PSMZ32 / PSMZ24: the CT32 column layout with the Z block order, by `y * 64 + x`.
fn zlut() -> &'static Vec<u32> {
    static L: std::sync::OnceLock<Vec<u32>> = std::sync::OnceLock::new();
    L.get_or_init(|| {
        (0..64 * 32u32)
            .map(|i| {
                let (x, y) = (i % 64, i / 64);
                let blk = BTZ32[((y % 32) / 8) as usize][((x % 64) / 8) as usize] as u32;
                (blk * 64 + CT32[(y % 8) as usize][(x % 8) as usize] as u32) * 4
            })
            .collect()
    })
}

const BTZ32: [[u8; 8]; 4] = [[24, 25, 28, 29, 8, 9, 12, 13], [26, 27, 30, 31, 10, 11, 14, 15], [16, 17, 20, 21, 0, 1, 4, 5], [18, 19, 22, 23, 2, 3, 6, 7]];

fn r32(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 32) * bw + x / 64;
    let blk = BT32[((y % 32) / 8) as usize][((x % 64) / 8) as usize] as u32;
    ((((bp + page * 32 + blk) * 64 + CT32[(y % 8) as usize][(x % 8) as usize] as u32) * 4) & 0x3f_ffff) as usize
}

fn r16(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 64) * bw + x / 64;
    let blk = BT16[((y % 64) / 8) as usize][((x % 64) / 16) as usize] as u32;
    let hw = CT32[(y % 8) as usize][(x % 8) as usize] as u32 * 2 + ((x >> 3) & 1);
    ((((bp + page * 32 + blk) * 128 + hw) * 2) & 0x3f_ffff) as usize
}

fn r8(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 64) * (bw / 2).max(1) + x / 128;
    let blk = BT32[((y % 64) / 16) as usize][((x % 128) / 16) as usize] as u32;
    let (xx, yy) = (x & 15, y & 15);
    let sw = (((yy + 2) >> 2) & 1) * 4;
    let py = (((yy & !3) >> 1) + (yy & 1)) & 7;
    let w = CT32[py as usize][((xx + sw) & 7) as usize] as u32;
    ((((bp + page * 32 + blk) * 64 + w) * 4 + ((xx >> 3) & 1) * 2 + ((yy >> 1) & 1)) & 0x3f_ffff) as usize
}

fn r4(bp: u32, bw: u32, x: u32, y: u32) -> usize {
    let page = (y / 128) * (bw / 2).max(1) + x / 128;
    let blk = BT16[((y % 128) / 16) as usize][((x % 128) / 32) as usize] as u32;
    let (xx, yy) = (x & 31, y & 15);
    let sw = (((yy + 2) >> 2) & 1) * 4;
    let py = (((yy & !3) >> 1) + (yy & 1)) & 7;
    let w = CT32[py as usize][((xx + sw) & 7) as usize] as u32;
    ((((bp + page * 32 + blk) * 64 + w) * 8 + ((xx >> 3) & 3) * 2 + ((yy >> 1) & 1)) & 0x7f_ffff) as usize
}

const BT32: [[u8; 8]; 4] = [
    [0, 1, 4, 5, 16, 17, 20, 21],
    [2, 3, 6, 7, 18, 19, 22, 23],
    [8, 9, 12, 13, 24, 25, 28, 29],
    [10, 11, 14, 15, 26, 27, 30, 31],
];
const BT16: [[u8; 4]; 8] = [
    [0, 2, 8, 10],
    [1, 3, 9, 11],
    [4, 6, 12, 14],
    [5, 7, 13, 15],
    [16, 18, 24, 26],
    [17, 19, 25, 27],
    [20, 22, 28, 30],
    [21, 23, 29, 31],
];
const CT32: [[u8; 8]; 8] = [
    [0, 1, 4, 5, 8, 9, 12, 13],
    [2, 3, 6, 7, 10, 11, 14, 15],
    [16, 17, 20, 21, 24, 25, 28, 29],
    [18, 19, 22, 23, 26, 27, 30, 31],
    [32, 33, 36, 37, 40, 41, 44, 45],
    [34, 35, 38, 39, 42, 43, 46, 47],
    [48, 49, 52, 53, 56, 57, 60, 61],
    [50, 51, 54, 55, 58, 59, 62, 63],
];
