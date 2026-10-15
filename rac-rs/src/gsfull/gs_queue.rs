//! Command queue between the GS front end (`gs.rs`, serial GIF parsing) and the rasteriser.
//!
//! The front end never touches GS memory while drawing is in flight: everything that reads or writes it
//! (host->local uploads, CLUT loads, local->local copies, prims) becomes a command in a `Batch`. A batch is
//! a list of "pre" commands (uploads, CLUT loads, copies, executed in order first) plus prims, drawn either
//! in parallel (binned into 32x32 screen tiles, prim order kept per tile, largest tiles first on a private
//! rayon pool) or serially in prim order. Batches run in submission order on a back-end thread, so the front
//! end keeps parsing while earlier batches draw.
//!
//! Ordering rules that keep the result identical to the serial prototype (ranges are 256-byte blocks of
//! GS memory; R / W = blocks read / written by the prims already in the batch):
//! - a pixel (x, y) of one frame buffer (FBP, FBW, PSM) has one address and z planes are indexed by (x, y),
//!   so tiles never share memory while every prim stays inside its frame width (else it is drawn serially);
//! - a prim whose texture is in W (read after write), whose frame rows overlap R (write after read) or
//!   another frame buffer of the batch (write after write) starts a new batch;
//! - a prim that reads its own frame rows (feedback), uses an unusual frame format or leaves its frame width
//!   goes to a serial batch (drawn in the prototype's pixel order);
//! - a pre command runs before the batch's earlier prims, so an upload overlapping R or W, a CLUT load
//!   overlapping W, and any local copy start a new batch;
//! - a CLUT load identical to the last one emitted, with no write to its source since, is dropped (the CLUT
//!   would not change).
use super::gs_mem::{clut_read, local_copy, luts, put_run, trx_advance, Trx};
use super::gs_raster::{raster_prim, sprite_inplace, Acc, Ctx, Mem, Prim, TS};
use std::sync::atomic::{fence, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::Instant;

const TN: usize = (1024 / TS) as usize;
const NT: usize = TN * TN;
const CHUNK: usize = 2048;
const NBLK: u32 = 16384;
/// Prims per batch before it is handed to the back end without a hazard (keeps the back end busy).
const MAXB: usize = 16384;

pub enum Pre {
    Upload { bb: u64, pos: u64, rr: u64, t: Trx, data: Vec<u128> },
    Clut { cbp: u32, cfmt: u32, t4: bool, off: u32, texa: u64, snap: bool },
    Copy { bb: u64, pos: u64, rr: u64 },
}

pub struct Batch {
    serial: bool,
    base_snap: bool,
    pre: Vec<Pre>,
    prims: Vec<Prim>,
    ctxs: Vec<Ctx>,
    zptrs: Vec<usize>,
    /// CLUT snapshots made by the pre commands (read by the prims through `Ctx::clut`).
    snaps: Vec<[[u8; 4]; 512]>,
    /// The pre commands touch no block the previous batch's prims read or write (and there is no local copy),
    /// so they may run while the previous batch is still drawing.
    overlap: bool,
}

impl Batch {
    fn new() -> Box<Batch> {
        Box::new(Batch { serial: false, base_snap: false, pre: Vec::new(), prims: Vec::with_capacity(MAXB), ctxs: Vec::new(), zptrs: Vec::new(), snaps: Vec::new(), overlap: false })
    }
    fn clear(&mut self) {
        self.serial = false;
        self.base_snap = false;
        self.pre.clear();
        self.prims.clear();
        self.ctxs.clear();
        self.zptrs.clear();
        self.snaps.clear();
        self.overlap = false;
    }
    fn is_empty(&self) -> bool {
        self.pre.is_empty() && self.prims.is_empty()
    }
}

#[derive(Default, Clone, Debug)]
pub struct RasterStats {
    /// Front end: batches handed over, by reason.
    pub batches: u64,
    pub batched_prims: u64,
    pub serial_prims: u64,
    /// Serial prims by reason: frame format, FBW 0 / past the frame width / past 4 MB, reads own frame.
    pub serial_fmt: u64,
    pub serial_wide: u64,
    pub serial_feedback: u64,
    pub split_raw: u64,
    pub split_waw: u64,
    pub split_war: u64,
    pub split_upload: u64,
    pub split_clut: u64,
    pub split_mode: u64,
    pub split_full: u64,
    pub split_other: u64,
    pub clut_emitted: u64,
    pub clut_skipped: u64,
    /// Back end.
    pub upload_qw: u64,
    pub pre_ns: u64,
    pub par_ns: u64,
    pub serial_ns: u64,
    pub px: Acc,
}

#[derive(Clone, Copy)]
enum Why {
    Raw,
    Waw,
    War,
    Upload,
    Clut,
    Mode,
    Full,
    Other,
}

struct Bits {
    w: Vec<u64>,
    lo: usize,
    hi: usize,
}

fn mask(i: usize, a: usize, b: usize, lo: u32, hi: u32) -> u64 {
    let mut m = !0u64;
    if i == a {
        m &= !0u64 << (lo % 64);
    }
    if i == b {
        let e = (hi - 1) % 64;
        m &= if e == 63 { !0 } else { (1u64 << (e + 1)) - 1 };
    }
    m
}

impl Bits {
    fn new() -> Bits {
        Bits { w: vec![0; NBLK as usize / 64 + 1], lo: usize::MAX, hi: 0 }
    }
    fn any(&self, lo: u32, hi: u32) -> bool {
        let hi = hi.min(NBLK);
        if lo >= hi {
            return false;
        }
        let (a, b) = ((lo / 64) as usize, ((hi - 1) / 64) as usize);
        if a > self.hi || b < self.lo {
            return false;
        }
        (a.max(self.lo)..=b.min(self.hi)).any(|i| self.w[i] & mask(i, a, b, lo, hi) != 0)
    }
    fn set(&mut self, lo: u32, hi: u32) {
        let hi = hi.min(NBLK);
        if lo >= hi {
            return;
        }
        let (a, b) = ((lo / 64) as usize, ((hi - 1) / 64) as usize);
        for i in a..=b {
            self.w[i] |= mask(i, a, b, lo, hi);
        }
        self.lo = self.lo.min(a);
        self.hi = self.hi.max(b);
    }
    fn clear(&mut self) {
        if self.lo <= self.hi {
            self.w[self.lo..=self.hi].iter_mut().for_each(|w| *w = 0);
        }
        self.lo = usize::MAX;
        self.hi = 0;
    }
}

fn overlap(a: (u32, u32), b: (u32, u32)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

struct Target {
    key: (u32, u32, u32),
    lo: u32,
    hi: u32,
}

#[derive(Clone, Copy, PartialEq)]
struct ClutKey {
    cbp: u32,
    cfmt: u32,
    t4: bool,
    off: u32,
    texa: u64,
}

enum Msg {
    Run(Box<Batch>),
    Sync(mpsc::Sender<()>),
}

enum Back {
    Inline(Box<Exec>),
    Thread { tx: Option<mpsc::SyncSender<Msg>>, ret: mpsc::Receiver<Box<Batch>>, join: Option<std::thread::JoinHandle<()>>, stats: Arc<Mutex<RasterStats>> },
}

/// Front-end side: builds batches and hands them to the back end.
pub struct Queue {
    cur: Box<Batch>,
    rd: Bits,
    wr: Bits,
    /// Blocks read / written by the prims of the batch submitted last.
    prev_rd: Bits,
    prev_wr: Bits,
    /// Block ranges the current batch's pre commands write (uploads) / read (CLUT loads); local copy present.
    pre_w: Vec<(u32, u32)>,
    pre_r: Vec<(u32, u32)>,
    pre_copy: bool,
    targets: Vec<Target>,
    tex_ok: Option<(u32, u32)>,
    cur_ctx: Option<u32>,
    nsnap: u32,
    cur_snap: Option<u32>,
    last_clut_pre: Option<usize>,
    open_direct: Option<(u64, u64, u64, Trx)>,
    up_ok: Option<(u32, u32)>,
    clut_src: Option<ClutKey>,
    zptrs: Vec<usize>,
    back: Back,
    free: Vec<Box<Batch>>,
    pub stats: RasterStats,
}

impl Queue {
    /// `vram` must stay valid (4 MB, not moved) for the life of the queue. threads 0 = draw inline at
    /// submission (no back-end thread), 1 = back-end thread without pool, n = back-end thread + n workers.
    pub fn new(vram: *mut u8, threads: usize) -> Queue {
        let pool = (threads > 1).then(|| rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|i| format!("gs-raster-{i}")).build().expect("gs raster pool"));
        let exec = Box::new(Exec { pool, vram: vram as usize, clut: Box::new([[0; 4]; 512]), bins: Vec::new(), stats: RasterStats::default(), fly: None, wake: Arc::new(Wake::new()) });
        let back = if threads == 0 {
            Back::Inline(exec)
        } else {
            let (tx, rx) = mpsc::sync_channel::<Msg>(4);
            let (rtx, ret) = mpsc::channel::<Box<Batch>>();
            let stats = Arc::new(Mutex::new(RasterStats::default()));
            let st = stats.clone();
            let join = std::thread::Builder::new()
                .name("gs-backend".into())
                .spawn(move || {
                    let mut exec = exec;
                    loop {
                        // While a batch draws on the pool, take the next message without blocking so that its pre
                        // commands can run meanwhile (when the queue proved they cannot interfere).
                        let m = if exec.fly.is_some() {
                            match rx.try_recv() {
                                Ok(m) => Some(m),
                                Err(mpsc::TryRecvError::Empty) => {
                                    exec.finish(&rtx);
                                    rx.recv().ok()
                                }
                                Err(mpsc::TryRecvError::Disconnected) => None,
                            }
                        } else {
                            rx.recv().ok()
                        };
                        match m {
                            None => break,
                            Some(Msg::Run(b)) => exec.run_async(b, &rtx),
                            Some(Msg::Sync(ack)) => {
                                exec.finish(&rtx);
                                *st.lock().unwrap() = exec.stats.clone();
                                let _ = ack.send(());
                            }
                        }
                    }
                    exec.finish(&rtx);
                })
                .expect("gs back-end thread");
            Back::Thread { tx: Some(tx), ret, join: Some(join), stats }
        };
        Queue {
            cur: Batch::new(),
            rd: Bits::new(),
            wr: Bits::new(),
            prev_rd: Bits::new(),
            prev_wr: Bits::new(),
            pre_w: Vec::new(),
            pre_r: Vec::new(),
            pre_copy: false,
            targets: Vec::new(),
            tex_ok: None,
            cur_ctx: None,
            nsnap: 0,
            cur_snap: None,
            last_clut_pre: None,
            open_direct: None,
            up_ok: None,
            clut_src: None,
            zptrs: Vec::new(),
            back,
            free: Vec::new(),
            stats: RasterStats::default(),
        }
    }

    pub fn add_zplane(&mut self, p: *mut u32) {
        self.zptrs.push(p as usize);
    }

    /// The front end's context changed: the next prim gets a new context entry.
    pub fn new_ctx(&mut self) {
        self.cur_ctx = None;
    }

    fn submit(&mut self, why: Why) {
        if self.cur.is_empty() {
            return;
        }
        let s = &mut self.stats;
        match why {
            Why::Raw => s.split_raw += 1,
            Why::Waw => s.split_waw += 1,
            Why::War => s.split_war += 1,
            Why::Upload => s.split_upload += 1,
            Why::Clut => s.split_clut += 1,
            Why::Mode => s.split_mode += 1,
            Why::Full => s.split_full += 1,
            Why::Other => s.split_other += 1,
        }
        s.batches += 1;
        s.batched_prims += self.cur.prims.len() as u64;
        let fresh = match &mut self.back {
            Back::Thread { ret, .. } => ret.try_recv().ok(),
            Back::Inline(_) => None,
        };
        let fresh = fresh.or_else(|| self.free.pop()).unwrap_or_else(Batch::new);
        let mut b = std::mem::replace(&mut self.cur, fresh);
        b.zptrs.extend_from_slice(&self.zptrs);
        b.overlap = !self.pre_copy
            && self.pre_w.iter().all(|&(lo, hi)| !self.prev_rd.any(lo, hi) && !self.prev_wr.any(lo, hi))
            && self.pre_r.iter().all(|&(lo, hi)| !self.prev_wr.any(lo, hi));
        match &mut self.back {
            Back::Inline(e) => {
                e.run_inline(&mut b);
                b.clear();
                self.free.push(b);
            }
            Back::Thread { tx, .. } => {
                tx.as_ref().unwrap().send(Msg::Run(b)).expect("gs back end gone");
            }
        }
        std::mem::swap(&mut self.prev_rd, &mut self.rd);
        std::mem::swap(&mut self.prev_wr, &mut self.wr);
        self.rd.clear();
        self.wr.clear();
        self.pre_w.clear();
        self.pre_r.clear();
        self.pre_copy = false;
        self.targets.clear();
        self.tex_ok = None;
        self.cur_ctx = None;
        self.nsnap = 0;
        self.cur_snap = None;
        self.last_clut_pre = None;
        self.open_direct = None;
        self.up_ok = None;
    }

    /// Hand over everything queued and wait until it is drawn. Returns the back end's statistics.
    pub fn sync(&mut self) -> RasterStats {
        self.submit(Why::Other);
        let mut s = match &mut self.back {
            Back::Inline(e) => e.stats.clone(),
            Back::Thread { tx, stats, .. } => {
                let (atx, arx) = mpsc::channel();
                tx.as_ref().unwrap().send(Msg::Sync(atx)).expect("gs back end gone");
                arx.recv().expect("gs back end gone");
                let s = stats.lock().unwrap().clone();
                s
            }
        };
        let f = &self.stats;
        s.batches = f.batches;
        s.batched_prims = f.batched_prims;
        s.serial_prims = f.serial_prims;
        s.serial_fmt = f.serial_fmt;
        s.serial_wide = f.serial_wide;
        s.serial_feedback = f.serial_feedback;
        s.split_raw = f.split_raw;
        s.split_waw = f.split_waw;
        s.split_war = f.split_war;
        s.split_upload = f.split_upload;
        s.split_clut = f.split_clut;
        s.split_mode = f.split_mode;
        s.split_full = f.split_full;
        s.split_other = f.split_other;
        s.clut_emitted = f.clut_emitted;
        s.clut_skipped = f.clut_skipped;
        s
    }

    /// A write to blocks [lo, hi) is queued: forget the CLUT source if it is touched.
    fn note_write(&mut self, lo: u32, hi: u32) {
        if let Some(k) = self.clut_src {
            if overlap((k.cbp, k.cbp + 4), (lo, hi)) {
                self.clut_src = None;
            }
        }
    }

    /// Host->local upload of `data` starting at transfer position `t`; it writes blocks [lo, hi).
    pub fn upload(&mut self, bb: u64, pos: u64, rr: u64, t: Trx, data: Vec<u128>, lo: u32, hi: u32) {
        if !self.cur.prims.is_empty() && (self.rd.any(lo, hi) || self.wr.any(lo, hi)) {
            self.submit(Why::Upload);
        }
        self.note_write(lo, hi);
        self.pre_w.push((lo, hi));
        self.cur.pre.push(Pre::Upload { bb, pos, rr, t, data });
        self.open_direct = None;
    }

    /// One qword of a direct (not deferred) upload at transfer position `t`; the transfer writes [lo, hi).
    pub fn direct(&mut self, bb: u64, pos: u64, rr: u64, t: Trx, q: u128, lo: u32, hi: u32) {
        if self.up_ok != Some((lo, hi)) {
            if !self.cur.prims.is_empty() && (self.rd.any(lo, hi) || self.wr.any(lo, hi)) {
                self.submit(Why::Upload);
            }
            self.note_write(lo, hi);
            self.pre_w.push((lo, hi));
            self.up_ok = Some((lo, hi));
        }
        let mut next = t;
        trx_advance(bb, rr, &mut next, 1);
        if self.open_direct == Some((bb, pos, rr, t)) {
            if let Some(Pre::Upload { data, .. }) = self.cur.pre.last_mut() {
                data.push(q);
                self.open_direct = Some((bb, pos, rr, next));
                return;
            }
        }
        self.cur.pre.push(Pre::Upload { bb, pos, rr, t, data: vec![q] });
        self.open_direct = Some((bb, pos, rr, next));
    }

    /// CLUT load. Returns false when it was dropped because it cannot change the CLUT.
    pub fn clut(&mut self, cbp: u32, cfmt: u32, t4: bool, off: u32, texa: u64) -> bool {
        let key = ClutKey { cbp, cfmt, t4, off: if t4 { off } else { 0 }, texa: if cfmt == 0x02 { texa } else { 0 } };
        if self.clut_src == Some(key) {
            self.stats.clut_skipped += 1;
            return false;
        }
        self.stats.clut_emitted += 1;
        if !self.cur.prims.is_empty() && self.wr.any(cbp, cbp + 4) {
            self.submit(Why::Clut);
        }
        self.pre_r.push((cbp, cbp + 4));
        self.cur.pre.push(Pre::Clut { cbp, cfmt, t4, off, texa, snap: false });
        self.last_clut_pre = Some(self.cur.pre.len() - 1);
        self.cur_snap = None;
        self.open_direct = None;
        self.clut_src = Some(key);
        true
    }

    /// Local->local copy.
    pub fn copy(&mut self, bb: u64, pos: u64, rr: u64) {
        if !self.cur.prims.is_empty() {
            self.submit(Why::Other);
        }
        self.pre_copy = true;
        self.cur.pre.push(Pre::Copy { bb, pos, rr });
        self.open_direct = None;
        self.clut_src = None;
    }

    fn snap_index(&mut self) -> u32 {
        if let Some(i) = self.cur_snap {
            return i;
        }
        let i = self.nsnap;
        self.nsnap += 1;
        match self.last_clut_pre {
            None => self.cur.base_snap = true,
            Some(k) => {
                if let Pre::Clut { snap, .. } = &mut self.cur.pre[k] {
                    *snap = true;
                }
            }
        }
        self.cur_snap = Some(i);
        i
    }

    /// Queue one prim with context `cx`; `tex` = block range its texture reads.
    pub fn push(&mut self, cx: &Ctx, mut p: Prim, tex: Option<(u32, u32)>) {
        let [_, y0, x1, y1] = p.bb;
        // Frame buffer block range of the rows this prim can touch (formats with 64-pixel-wide pages).
        let flat = matches!(cx.fpsm, 0x00 | 0x01 | 0x02 | 0x0a);
        let ph = if matches!(cx.fpsm, 0x02 | 0x0a) { 64 } else { 32 };
        let mut fr = (cx.fbp + (y0 as u32 / ph) * cx.fbw * 32, cx.fbp + (y1 as u32 / ph + 1) * cx.fbw * 32);
        let serial = if !flat {
            fr = (0, NBLK);
            Some(0)
        } else if cx.fbw == 0 || x1 as u32 >= cx.fbw * 64 || fr.1 > NBLK {
            fr = (0, NBLK);
            Some(1)
        } else if tex.is_some_and(|t| overlap(t, fr)) {
            Some(2)
        } else {
            None
        };
        if !self.cur.prims.is_empty() && self.cur.serial != serial.is_some() {
            self.submit(Why::Mode);
        }
        if let Some(why) = serial {
            let s = &mut self.stats;
            s.serial_prims += 1;
            match why {
                0 => s.serial_fmt += 1,
                1 => s.serial_wide += 1,
                _ => s.serial_feedback += 1,
            }
            self.cur.serial = true;
        } else if !self.cur.prims.is_empty() {
            let mut why = None;
            if let Some(t) = tex {
                if self.tex_ok != Some(t) && self.wr.any(t.0, t.1) {
                    why = Some(Why::Raw);
                }
            }
            if why.is_none() {
                let key = (cx.fbp, cx.fbw, cx.fpsm);
                match self.targets.iter().find(|e| e.key == key) {
                    Some(e) if fr.0 >= e.lo && fr.1 <= e.hi => {}
                    e => {
                        let env = e.map(|e| (e.lo.min(fr.0), e.hi.max(fr.1))).unwrap_or(fr);
                        if self.targets.iter().any(|o| o.key != key && overlap((o.lo, o.hi), env)) {
                            why = Some(Why::Waw);
                        } else if self.rd.any(env.0, env.1) {
                            why = Some(Why::War);
                        }
                    }
                }
            }
            if let Some(w) = why {
                self.submit(w);
            }
        }
        if let Some(t) = tex {
            if self.tex_ok != Some(t) {
                self.rd.set(t.0, t.1);
                self.tex_ok = Some(t);
            }
        }
        let key = (cx.fbp, cx.fbw, cx.fpsm);
        match self.targets.iter_mut().find(|e| e.key == key) {
            Some(e) => {
                if fr.0 < e.lo || fr.1 > e.hi {
                    e.lo = e.lo.min(fr.0);
                    e.hi = e.hi.max(fr.1);
                    let (lo, hi) = (e.lo, e.hi);
                    self.wr.set(lo, hi);
                    self.note_write(lo, hi);
                }
            }
            None => {
                self.targets.push(Target { key, lo: fr.0, hi: fr.1 });
                self.wr.set(fr.0, fr.1);
                self.note_write(fr.0, fr.1);
            }
        }
        if self.cur_ctx.is_none() {
            let mut c = *cx;
            if c.paletted() {
                c.clut = self.snap_index();
            }
            self.cur.ctxs.push(c);
            self.cur_ctx = Some((self.cur.ctxs.len() - 1) as u32);
        }
        p.ctx = self.cur_ctx.unwrap();
        self.cur.prims.push(p);
        self.up_ok = None;
        if self.cur.prims.len() >= MAXB {
            self.submit(Why::Full);
        }
    }
}

impl Drop for Queue {
    fn drop(&mut self) {
        if let Back::Thread { tx, join, .. } = &mut self.back {
            drop(tx.take());
            if let Some(j) = join.take() {
                let _ = j.join();
            }
        }
    }
}

struct ChunkBins {
    off: Vec<u32>,
    items: Vec<u32>,
}

/// Palette used by prims without a CLUT.
static NO_CLUT: [[u8; 4]; 512] = [[0; 4]; 512];

/// Back-end side: runs batches in order.
struct Exec {
    pool: Option<rayon::ThreadPool>,
    vram: usize,
    clut: Box<[[u8; 4]; 512]>,
    bins: Vec<ChunkBins>,
    stats: RasterStats,
    /// The batch drawing on the pool right now (the back-end thread is free to prepare the next one).
    fly: Option<Fly>,
    /// Wakes the waiters of the batch in flight (barrier, completion); one at a time.
    wake: Arc<Wake>,
}

struct Shared<T>(*mut T);
impl<T> Shared<T> {
    fn at(&self, i: usize) -> *mut T {
        unsafe { self.0.add(i) }
    }
}
unsafe impl<T> Send for Shared<T> {}
unsafe impl<T> Sync for Shared<T> {}

/// A batch handed to the pool: it owns the batch, and `job` points into it.
struct Fly {
    b: Box<Batch>,
    job: Box<Job>,
    t0: Instant,
}

/// What the pool threads of one asynchronous batch share. Lives in a Box until `Exec::finish`, which waits for
/// every worker, so the borrowed slices (transmuted to 'static) stay valid while they are used.
struct Job {
    prims: &'static [Prim],
    ctxs: &'static [Ctx],
    snaps: &'static [[[u8; 4]; 512]],
    mem: Mem,
    bins: Shared<ChunkBins>,
    chunk: usize,
    nch: usize,
    serial: bool,
    next_chunk: AtomicUsize,
    binned: AtomicUsize,
    next_tile: AtomicUsize,
    finished: AtomicUsize,
    want: usize,
    total: Mutex<Acc>,
    wake: Arc<Wake>,
}
unsafe impl Send for Job {}
unsafe impl Sync for Job {}

fn clut_for<'a>(c: &Ctx, snaps: &'a [[[u8; 4]; 512]]) -> &'a [[u8; 4]; 512] {
    if c.paletted() {
        &snaps[c.clut as usize]
    } else {
        &NO_CLUT
    }
}

/// Microseconds a waiter spins before it yields and then blocks (RAC_GSFULL_SPIN_US; waits that short are the normal case
/// on an idle machine, and a block costs a wake-up of tens of microseconds).
fn spin_us() -> u64 {
    static V: OnceLock<u64> = OnceLock::new();
    *V.get_or_init(|| std::env::var("RAC_GSFULL_SPIN_US").ok().and_then(|v| v.parse().ok()).unwrap_or(20))
}

/// A wait that parks: the waiter spins briefly, yields a few times, then sleeps on a condition variable until a notifier
/// (called after the atomic it publishes is stored) wakes it, so that waiters cost no CPU when the machine is busy and a
/// worker holding a chunk is descheduled. A 1 ms timeout backs the wake-up up. The notify side is two instructions when
/// nobody sleeps.
struct Wake {
    m: Mutex<()>,
    cv: Condvar,
    sleeping: AtomicUsize,
}

impl Wake {
    fn new() -> Wake {
        Wake { m: Mutex::new(()), cv: Condvar::new(), sleeping: AtomicUsize::new(0) }
    }

    /// Call after the store that can make a waiter's condition true. The caller must not touch the memory holding
    /// `self` afterwards unless it keeps it alive (a clone of the Arc).
    fn notify(&self) {
        fence(Ordering::SeqCst);
        if self.sleeping.load(Ordering::Relaxed) != 0 {
            drop(self.m.lock().unwrap());
            self.cv.notify_all();
        }
    }

    fn wait(&self, mut ready: impl FnMut() -> bool) {
        if ready() {
            return;
        }
        let t = Instant::now();
        let us = spin_us();
        while t.elapsed().as_micros() < us as u128 {
            for _ in 0..16 {
                std::hint::spin_loop();
            }
            if ready() {
                return;
            }
        }
        for _ in 0..4 {
            std::thread::yield_now();
            if ready() {
                return;
            }
        }
        let mut g = self.m.lock().unwrap();
        self.sleeping.fetch_add(1, Ordering::SeqCst);
        fence(Ordering::SeqCst);
        while !ready() {
            g = self.cv.wait_timeout(g, std::time::Duration::from_millis(1)).unwrap().0;
        }
        self.sleeping.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The work of one pool thread for a batch (every thread of the pool runs it for a parallel batch, one for a
/// serial batch).
///
/// Parallel batch: phase 1 bins chunks of prims (per chunk a counting sort of (tile, prim) pairs, prim order kept
/// inside each tile), a barrier waits for all chunks, phase 2 draws the non-empty tiles in index order.
unsafe fn run_job(j: &Job) {
    let l = luts();
    let (prims, ctxs, snaps) = (j.prims, j.ctxs, j.snaps);
    let mut acc = Acc::default();
    if j.serial {
        for p in prims {
            let c = &ctxs[p.ctx as usize];
            raster_prim(p, c, clut_for(c, snaps), &j.mem, l, [0, 0, 1023, 1023], &mut acc);
        }
    } else {
        loop {
            let c = j.next_chunk.fetch_add(1, Ordering::Relaxed);
            if c >= j.nch {
                break;
            }
            let b = &mut *j.bins.at(c);
            let ps = &prims[c * j.chunk..((c + 1) * j.chunk).min(prims.len())];
            b.off.iter_mut().for_each(|o| *o = 0);
            let tiles = |p: &Prim| {
                let [x0, y0, x1, y1] = p.bb;
                let (tx0, tx1) = (x0 as usize / TS as usize, x1 as usize / TS as usize);
                ((y0 as usize / TS as usize)..=(y1 as usize / TS as usize)).flat_map(move |ty| (tx0..=tx1).map(move |tx| ty * TN + tx))
            };
            for p in ps {
                for t in tiles(p) {
                    b.off[t + 1] += 1;
                }
            }
            for t in 0..NT {
                b.off[t + 1] += b.off[t];
            }
            b.items.resize(b.off[NT] as usize, 0);
            let mut pos: Vec<u32> = b.off[..NT].to_vec();
            for (i, p) in ps.iter().enumerate() {
                for t in tiles(p) {
                    b.items[pos[t] as usize] = (c * j.chunk + i) as u32;
                    pos[t] += 1;
                }
            }
            j.binned.fetch_add(1, Ordering::Release);
            j.wake.notify();
        }
        // Barrier: every chunk is binned (a worker that found no chunk left waits for the others).
        j.wake.wait(|| j.binned.load(Ordering::Acquire) >= j.nch);
        let bins: &[ChunkBins] = std::slice::from_raw_parts(j.bins.at(0) as *const ChunkBins, j.nch);
        loop {
            let t = j.next_tile.fetch_add(1, Ordering::Relaxed);
            if t >= NT {
                break;
            }
            if bins.iter().all(|b| b.off[t] == b.off[t + 1]) {
                continue;
            }
            let (tx, ty) = ((t % TN) as i32 * TS, (t / TN) as i32 * TS);
            let rect = [tx, ty, tx + TS - 1, ty + TS - 1];
            for b in bins {
                for &pi in &b.items[b.off[t] as usize..b.off[t + 1] as usize] {
                    let p = &prims[pi as usize];
                    let c = &ctxs[p.ctx as usize];
                    raster_prim(p, c, clut_for(c, snaps), &j.mem, l, rect, &mut acc);
                }
            }
        }
    }
    j.total.lock().unwrap().add(&acc);
    // The batch's owner may free `j` the moment `finished` is complete: keep the wake alive across the notify.
    let wake = j.wake.clone();
    j.finished.fetch_add(1, Ordering::Release);
    wake.notify();
}

impl Exec {
    /// Pre commands in order: uploads, CLUT loads (snapshots go to the batch), local copies.
    fn pre(&mut self, b: &mut Batch) {
        let t0 = Instant::now();
        let m = self.vram as *mut u8;
        b.snaps.clear();
        if b.base_snap {
            b.snaps.push(*self.clut);
        }
        for p in &b.pre {
            match p {
                Pre::Upload { bb, pos, rr, t, data } => {
                    let mut t = *t;
                    unsafe { put_run(m, *bb, *pos, *rr, &mut t, data) };
                    self.stats.upload_qw += data.len() as u64;
                }
                Pre::Clut { cbp, cfmt, t4, off, texa, snap } => {
                    unsafe { clut_read(m, &mut self.clut, *cbp, *cfmt, *t4, *off as usize, *texa) };
                    if *snap {
                        b.snaps.push(*self.clut);
                    }
                }
                Pre::Copy { bb, pos, rr } => unsafe { local_copy(m, *bb, *pos, *rr) },
            }
        }
        self.stats.pre_ns += t0.elapsed().as_nanos() as u64;
    }

    /// Draw a batch to the end on this thread (`threads` 0 or 1, or a serial batch that is a strip pipeline).
    fn draw_here(&mut self, b: &Batch) {
        let t1 = Instant::now();
        let mem = Mem { vram: self.vram as *mut u8, zp: b.zptrs.as_ptr() as *const *mut u32 };
        let pipe = if b.serial && self.pool.is_some() { self.pipe_plan(b) } else { None };
        let acc = if let Some(rects) = pipe {
            self.pipe(b, &mem, &rects)
        } else {
            let l = luts();
            let mut acc = Acc::default();
            for p in &b.prims {
                let c = &b.ctxs[p.ctx as usize];
                unsafe { raster_prim(p, c, clut_for(c, &b.snaps), &mem, l, [0, 0, 1023, 1023], &mut acc) };
            }
            acc
        };
        self.stats.px.add(&acc);
        self.stats.serial_ns += t1.elapsed().as_nanos() as u64;
    }

    /// Run a batch to the end on this thread (inline mode, no back-end thread).
    fn run_inline(&mut self, b: &mut Batch) {
        self.pre(b);
        if !b.prims.is_empty() {
            self.draw_here(b);
        }
    }

    /// Back-end thread: prepare `b` (its pre commands may overlap the batch still drawing when the queue marked
    /// it so), then start it on the pool. Batches that need no pool run to the end here.
    fn run_async(&mut self, mut b: Box<Batch>, rtx: &mpsc::Sender<Box<Batch>>) {
        if !b.overlap {
            self.finish(rtx);
        }
        self.pre(&mut b);
        self.finish(rtx);
        if b.prims.is_empty() {
            b.clear();
            let _ = rtx.send(b);
            return;
        }
        let pooled = self.pool.is_some();
        if !pooled || (b.serial && self.pipe_plan(&b).is_some()) {
            self.draw_here(&b);
            b.clear();
            let _ = rtx.send(b);
            return;
        }
        let pool = self.pool.as_ref().unwrap();
        let serial = b.serial;
        let chunk = b.prims.len().div_ceil(16).clamp(256, CHUNK);
        let nch = if serial { 0 } else { b.prims.len().div_ceil(chunk) };
        while self.bins.len() < nch {
            self.bins.push(ChunkBins { off: vec![0; NT + 1], items: Vec::new() });
        }
        let want = if serial { 1 } else { pool.current_num_threads() };
        // The slices point into the boxed batch, which `Fly` keeps alive (and unmoved) until the workers are done.
        let job = Box::new(unsafe {
            Job {
                prims: std::mem::transmute::<&[Prim], &'static [Prim]>(&b.prims[..]),
                ctxs: std::mem::transmute::<&[Ctx], &'static [Ctx]>(&b.ctxs[..]),
                snaps: std::mem::transmute::<&[[[u8; 4]; 512]], &'static [[[u8; 4]; 512]]>(&b.snaps[..]),
                mem: Mem { vram: self.vram as *mut u8, zp: b.zptrs.as_ptr() as *const *mut u32 },
                bins: Shared(self.bins.as_mut_ptr()),
                chunk,
                nch,
                serial,
                next_chunk: AtomicUsize::new(0),
                binned: AtomicUsize::new(0),
                next_tile: AtomicUsize::new(0),
                finished: AtomicUsize::new(0),
                want,
                total: Mutex::new(Acc::default()),
                wake: self.wake.clone(),
            }
        });
        let jp = job.as_ref() as *const Job as usize;
        let t0 = Instant::now();
        if serial {
            pool.spawn(move || unsafe { run_job(&*(jp as *const Job)) });
        } else {
            pool.spawn_broadcast(move |_| unsafe { run_job(&*(jp as *const Job)) });
        }
        self.fly = Some(Fly { b, job, t0 });
    }

    /// Wait for the batch drawing on the pool (if any), add up its statistics and give the batch back.
    fn finish(&mut self, rtx: &mpsc::Sender<Box<Batch>>) {
        let Some(f) = self.fly.take() else { return };
        self.wake.wait(|| f.job.finished.load(Ordering::Acquire) >= f.job.want);
        let ns = f.t0.elapsed().as_nanos() as u64;
        if f.job.serial {
            self.stats.serial_ns += ns;
        } else {
            self.stats.par_ns += ns;
        }
        self.stats.px.add(&f.job.total.lock().unwrap());
        let Fly { mut b, job, .. } = f;
        drop(job);
        b.clear();
        let _ = rtx.send(b);
    }

    /// Pixel rectangles of a serial batch made only of in-place sprite passes (`sprite_inplace`) over the same
    /// rows with increasing, disjoint columns (the screen-wide strips of a full-screen filter), else None.
    fn pipe_plan(&self, b: &Batch) -> Option<Vec<[i32; 4]>> {
        let mut rects: Vec<[i32; 4]> = Vec::with_capacity(b.prims.len());
        for p in &b.prims {
            let r = sprite_inplace(&b.ctxs[p.ctx as usize], p)?;
            if let Some(q) = rects.last() {
                if r[1] != q[1] || r[3] != q[3] || r[0] <= q[2] {
                    return None;
                }
            }
            rects.push(r);
        }
        (rects.len() > 1 || rects.first().is_some_and(|r| r[3] > r[1])).then_some(rects)
    }

    /// Run the strips of `pipe_plan` as a pipeline: each strip is claimed (in serial order) by one pool thread
    /// and drawn top to bottom, one row at a time, exactly in serial raster order inside the strip. Row y of
    /// strip i reads the last column of strip i - 1 in rows y - 1..=y + 1, which the serial order has finished
    /// (the whole earlier strip is drawn first), so it waits until strip i - 1 has completed rows 0..=y + 1.
    /// Strips are claimed in order, so a strip only waits for strips already running: no deadlock.
    fn pipe(&mut self, b: &Batch, m: &Mem, rects: &[[i32; 4]]) -> Acc {
        let pool = self.pool.as_ref().unwrap();
        let l = luts();
        let h = (rects[0][3] - rects[0][1] + 1) as u32;
        let done: Vec<AtomicU32> = (0..rects.len()).map(|_| AtomicU32::new(0)).collect();
        let next = AtomicUsize::new(0);
        let wake = Wake::new();
        let total = Mutex::new(Acc::default());
        let (prims, ctxs, snaps) = (&b.prims[..], &b.ctxs[..], &b.snaps[..]);
        pool.broadcast(|_| {
            let mut acc = Acc::default();
            loop {
                let si = next.fetch_add(1, Ordering::Relaxed);
                if si >= rects.len() {
                    break;
                }
                let r = rects[si];
                let p = &prims[si];
                let c = &ctxs[p.ctx as usize];
                let clut = clut_for(c, snaps);
                for row in 0..h {
                    if si > 0 {
                        let need = (row + 2).min(h);
                        wake.wait(|| done[si - 1].load(Ordering::Acquire) >= need);
                    }
                    let y = r[1] + row as i32;
                    unsafe { raster_prim(p, c, clut, m, l, [r[0], y, r[2], y], &mut acc) };
                    done[si].store(row + 1, Ordering::Release);
                    wake.notify();
                }
            }
            total.lock().unwrap().add(&acc);
        });
        total.into_inner().unwrap()
    }
}
