//! The GS-full pipeline: the `Job` stream ee-oracle produces per field (VIF1 chains, GIF DMA) -> VIF1 front end + VU1
//! (`vif.rs`, `vu1.rs`, native microprograms) -> the software GS (`gs.rs`) -> the CRTC read (`crtc.rs`) -> the game's
//! picture as an RGBA image.
//!
//! `Pipe` / `raster` / `crtc::present` are the whole pipeline on one thread (what the proof harness drives, field by
//! field). `start` runs it as two more threads behind bounded channels, so that the throughput is that of the slowest
//! stage and not the sum:
//!
//!   EE thread (retail.rs) --FieldIn--> "gsfull-vu1": VIF1 + VU1 --Field--> "gsfull-gs": raster + CRTC read -> slot
//!
//! A stage blocks (parks) while its neighbour has not caught up; none can be dropped (Jobs carry state such as texture
//! uploads). The newest finished picture is taken with `GsFull::take`.
//!
//! Environment: RAC_GSFULL_THREADS=N rasteriser workers (default: `threads()`), RAC_GSFULL_QUEUE=N fields per channel
//! (default 1), RAC_GSFULL_LOG=1 one stderr line per field (hash, ms per stage), RAC_GSFULL_DUMP=DIR raw RGBA of every
//! presented field to DIR/fNNNN.rgba.
use super::crtc::{present, Crtc};
use super::gs::Gs;
use super::vif::{Out, Vif1};
use super::vu1::Vu1;
use ee_oracle::gs::Job;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::Instant;

/// Fields a channel between two stages holds (RAC_GSFULL_QUEUE overrides).
pub const QUEUE: usize = 1;

/// One EE field as the oracle hands it over: the graphics jobs and the CRTC registers at the end of the field.
pub struct FieldIn {
    pub jobs: Vec<Job>,
    pub crtc: Crtc,
}

/// The GS-bound qwords of one field and where each run came from (see `Out::segs`).
#[derive(Default)]
pub struct Field {
    pub qs: Vec<u128>,
    pub segs: Vec<(u32, u8, u8)>,
    /// What the CRTC shows once the field is drawn.
    pub crtc: Crtc,
    /// DISPFB of the last job (what ee-oracle reported when that job was kicked).
    pub job_dispfb: u64,
    /// VIF1 + VU1 time of this field, ns, and the VU1 error count so far.
    pub vu_ns: u64,
    pub vu_errors: u64,
    /// The EE field number (0 = the first field run in this session).
    pub field: u64,
}

/// VIF1 + VU1 state that lives across fields.
pub struct Pipe {
    vif: Vif1,
    pub vu: Vu1,
    pub out: Out,
}

impl Pipe {
    /// `state`: a savestate directory (vu1Memory.bin / vu1MicroMem.bin start VU1 as the savestate left it).
    /// An empty `state` (a cold boot without RAC_GSFULL_STATE) starts VU1 blank: the boot uploads its own programs (MPG).
    pub fn new(state: &str) -> Option<Pipe> {
        let blank = || Some(vec![0u8; 16 << 10]);
        let d = if state.is_empty() { blank() } else { std::fs::read(format!("{state}/vu1Memory.bin")).ok() }?;
        let mi = if state.is_empty() { blank() } else { std::fs::read(format!("{state}/vu1MicroMem.bin")).ok() }?;
        let w: Vec<u32> = mi.get(..64)?.chunks_exact(4).map(|c| u32::from_le_bytes(c.try_into().unwrap())).collect();
        let out = Out { prog: super::vif::identify(&w), ..Default::default() };
        Some(Pipe { vif: Vif1::new(), vu: Vu1::new(&d, &mi), out })
    }

    /// One field's jobs in, its GS qwords out.
    pub fn process(&mut self, jobs: Vec<Job>, crtc: Crtc) -> Field {
        let mut f = Field { crtc, ..Default::default() };
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
                    self.vif.feed(&w, &marks, &mut self.vu, &mut self.out);
                    ee_oracle::gs::recycle_words(w);
                    let base = f.qs.len() as u32;
                    f.qs.extend_from_slice(&self.out.qs);
                    for &(s, p, g) in &self.out.segs {
                        f.segs.push((base + s, p, g));
                    }
                }
                Job::Gif(qs, _) => {
                    f.segs.push((f.qs.len() as u32, 3, 0));
                    f.qs.extend(qs);
                }
            }
        }
        f.vu_errors = self.vu.st.errors;
        // RAC_GSFULL_QDUMP=<file>: the field's GS qwords (debug; overwritten each field).
        if let Some(p) = std::env::var_os("RAC_GSFULL_QDUMP") {
            let b: Vec<u8> = f.qs.iter().flat_map(|x| x.to_le_bytes()).collect();
            let _ = std::fs::write(p, b);
        }
        f
    }
}

/// Feeds a field to the GS. `hud` marks the segments after the last PATH1 one (the widescreen HUD rule's input).
pub fn raster(g: &mut Gs, f: &Field) {
    let last1 = f.segs.iter().rposition(|s| s.1 == 1).map(|x| x as i64).unwrap_or(-1);
    for (i, seg) in f.segs.iter().enumerate() {
        g.hud = (i as i64) > last1;
        let end = f.segs.get(i + 1).map(|s| s.0 as usize).unwrap_or(f.qs.len());
        g.path = seg.1;
        g.prog = seg.2;
        for &q in &f.qs[seg.0 as usize..end] {
            g.gif(q);
        }
    }
}

/// FNV-1a of a byte image (the frame hashes of the logs and tests).
pub fn hash(b: &[u8]) -> u64 {
    b.chunks(8).fold(0xcbf2_9ce4_8422_2325u64, |h, c| {
        let mut w = [0u8; 8];
        w[..c.len()].copy_from_slice(c);
        (h ^ u64::from_le_bytes(w)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// One presented picture.
pub struct Frame {
    /// The EE field it was drawn for (0 = the first field run in this session).
    pub field: u64,
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Counters of one stage (all times in ns).
#[derive(Default)]
pub struct Stage {
    pub fields: AtomicU64,
    /// Time spent working, and blocked waiting for the neighbours (input empty / output full).
    pub busy_ns: AtomicU64,
    pub wait_ns: AtomicU64,
    pub last_ns: AtomicU64,
    pub max_ns: AtomicU64,
}

impl Stage {
    pub fn work(&self, ns: u64) {
        self.fields.fetch_add(1, Ordering::Relaxed);
        self.busy_ns.fetch_add(ns, Ordering::Relaxed);
        self.last_ns.store(ns, Ordering::Relaxed);
        self.max_ns.fetch_max(ns, Ordering::Relaxed);
    }

    pub fn wait(&self, ns: u64) {
        self.wait_ns.fetch_add(ns, Ordering::Relaxed);
    }

    /// Mean working ms per field over the session so far.
    pub fn mean_ms(&self) -> f64 {
        self.busy_ns.load(Ordering::Relaxed) as f64 / self.fields.load(Ordering::Relaxed).max(1) as f64 / 1e6
    }

    /// Mean blocked ms per field.
    pub fn wait_ms(&self) -> f64 {
        self.wait_ns.load(Ordering::Relaxed) as f64 / self.fields.load(Ordering::Relaxed).max(1) as f64 / 1e6
    }

    pub fn max_ms(&self) -> f64 {
        self.max_ns.load(Ordering::Relaxed) as f64 / 1e6
    }
}

/// The GS-full threads' shared state.
#[derive(Default)]
pub struct GsFull {
    slot: Mutex<Option<Frame>>,
    /// The EE thread (retail.rs fills it: `Machine::frame` as work, the send into the pipeline as wait), the VIF1 + VU1
    /// stage, and the raster + CRTC read stage.
    pub ee: Stage,
    pub vu: Stage,
    pub ras: Stage,
    /// A thread ended (a panic or the channel closed).
    pub dead: AtomicBool,
    /// Size of the newest picture (the window stretches it to 4:3, or to 16:9 in wide mode).
    dims: AtomicU32,
    /// 16:9 view (`wide.rs`): the picture holds a squeezed 16:9 image, to be shown at 16:9.
    pub wide: bool,
}

impl GsFull {
    /// The newest finished picture (each is handed out once).
    pub fn take(&self) -> Option<Frame> {
        self.slot.lock().ok()?.take()
    }

    /// Pictures drawn so far.
    pub fn fields(&self) -> u64 {
        self.ras.fields.load(Ordering::Relaxed)
    }

    /// Mean ms per field of the slowest stage's work.
    pub fn mean_ms(&self) -> f64 {
        self.vu.mean_ms().max(self.ras.mean_ms())
    }

    /// Size (w, h) of the newest picture.
    pub fn dims(&self) -> (u32, u32) {
        let d = self.dims.load(Ordering::Relaxed);
        (d >> 16, d & 0xffff)
    }

    /// One line with the mean ms per field of every stage over the session so far.
    pub fn report(&self) -> String {
        format!(
            "EE frame() {:.2} ms (max {:.2}, blocked on send {:.2}), vif+vu1 {:.2} ms (max {:.2}, blocked {:.2}), raster+read {:.2} ms (max {:.2}, blocked {:.2})",
            self.ee.mean_ms(),
            self.ee.max_ms(),
            self.ee.wait_ms(),
            self.vu.mean_ms(),
            self.vu.max_ms(),
            self.vu.wait_ms(),
            self.ras.mean_ms(),
            self.ras.max_ms(),
            self.ras.wait_ms()
        )
    }
}

/// Rasteriser workers: RAC_GSFULL_THREADS, else `default_threads()`.
pub fn threads() -> usize {
    std::env::var("RAC_GSFULL_THREADS").ok().and_then(|v| v.parse().ok()).unwrap_or_else(default_threads)
}

/// Measured default (GS_integrate.md, stage G-2): enough workers to finish a busy field inside the budget, few enough
/// to leave cores to the other stages and the game. A third of the logical cores, 2..8.
pub fn default_threads() -> usize {
    (std::thread::available_parallelism().map_or(8, |n| n.get()) / 3).clamp(2, 8)
}

fn queue_len() -> usize {
    std::env::var("RAC_GSFULL_QUEUE").ok().and_then(|v| v.parse().ok()).unwrap_or(QUEUE)
}

/// Marks the shared state dead when a stage's thread ends.
struct Flag(Arc<GsFull>);
impl Drop for Flag {
    fn drop(&mut self) {
        self.0.dead.store(true, Ordering::SeqCst);
    }
}

/// Starts the GS-full threads for the savestate `state` (its GS.bin is the starting local memory). The sender takes one
/// `FieldIn` per EE field; one with no jobs draws nothing and presents nothing.
pub fn start(state: &str) -> Option<(SyncSender<FieldIn>, Arc<GsFull>)> {
    // An empty `state` (cold boot): blank local memory, as the console's GS after the BIOS.
    let vram = if state.is_empty() { Some(vec![0u8; super::gs_mem::VRAM_SIZE]) } else { Gs::load_vram(&format!("{state}/GS.bin")) }?;
    let mut pipe = Pipe::new(state)?;
    let wide = super::wide::enabled();
    let shared = Arc::new(GsFull { wide, ..Default::default() });
    let q = queue_len();
    let (tx, rx) = sync_channel::<FieldIn>(q);
    let (ftx, frx) = sync_channel::<Field>(q);
    let nthreads = threads();
    let log = std::env::var("RAC_GSFULL_LOG").is_ok_and(|v| v != "0");
    let dump = std::env::var("RAC_GSFULL_DUMP").ok();
    // Diagnostics: RAC_GSFULL_DROP=1 discards every field in the first stage (the EE thread then runs alone).
    let drop_fields = std::env::var("RAC_GSFULL_DROP").is_ok_and(|v| v != "0");
    // Stage 1: VIF1 + VU1.
    let sh = shared.clone();
    std::thread::Builder::new()
        .name("gsfull-vu1".into())
        .stack_size(64 << 20)
        .spawn(move || {
            let _flag = Flag(sh.clone());
            let mut n = 0u64;
            loop {
                let t = Instant::now();
                let Ok(inp) = rx.recv() else { break };
                sh.vu.wait(t.elapsed().as_nanos() as u64);
                let field = n;
                n += 1;
                if inp.jobs.is_empty() || drop_fields {
                    continue;
                }
                let t = Instant::now();
                let mut f = pipe.process(inp.jobs, inp.crtc);
                f.field = field;
                f.vu_ns = t.elapsed().as_nanos() as u64;
                sh.vu.work(f.vu_ns);
                let t = Instant::now();
                if ftx.send(f).is_err() {
                    break;
                }
                sh.vu.wait(t.elapsed().as_nanos() as u64);
            }
        })
        .ok()?;
    // Stage 2: the software GS and the CRTC read.
    let sh = shared.clone();
    std::thread::Builder::new()
        .name("gsfull-gs".into())
        .stack_size(64 << 20)
        .spawn(move || {
            let _flag = Flag(sh.clone());
            let mut g = Gs::new(vram, nthreads);
            if wide {
                super::wide::apply_hud(&mut g);
            }
            loop {
                let t = Instant::now();
                let Ok(f) = frx.recv() else { break };
                sh.ras.wait(t.elapsed().as_nanos() as u64);
                let t = Instant::now();
                raster(&mut g, &f);
                let pic = present(&mut g, &f.crtc);
                let ns = t.elapsed().as_nanos() as u64;
                sh.ras.work(ns);
                super::gs_raster::PROBE_FIELD.fetch_add(1, Ordering::Relaxed);
                if log {
                    eprintln!(
                        "gsfull: field {} hash {:016x} {}x{} vif+vu1 {:.2} ms, raster+read {:.2} ms, {} qw, vu1 errors {}",
                        f.field,
                        hash(&pic.rgba),
                        pic.w,
                        pic.h,
                        f.vu_ns as f64 / 1e6,
                        ns as f64 / 1e6,
                        f.qs.len(),
                        f.vu_errors
                    );
                }
                // RAC_GSFULL_BUFDUMP=FIELD:DIR (debug): the CRTC registers and the buffers at FBP 0, 0x80, 0xf0 of that field as RGBA.
                if let Ok(v) = std::env::var("RAC_GSFULL_BUFDUMP") {
                    if let Some((n, d)) = v.split_once(':') {
                        if n.parse::<u64>().ok() == Some(f.field as u64) {
                            let _ = std::fs::create_dir_all(d);
                            let _ = std::fs::write(format!("{d}/crtc.txt"), format!("{:x?}", f.crtc));
                            for fbp in [0u64, 0x80, 0xf0, 0x140] {
                                let mut c = f.crtc;
                                c.pmode = 1;
                                c.dispfb[0] = (c.dispfb[0] & !0x1ff) | fbp;
                                let p = present(&mut g, &c);
                                let _ = std::fs::write(format!("{d}/b{:x}_{}x{}.rgba", fbp, p.w, p.h), &p.rgba);
                            }
                        }
                    }
                }
                if let Some(d) = &dump {
                    let _ = std::fs::create_dir_all(d);
                    let _ = std::fs::write(format!("{d}/f{:04}.rgba", f.field), &pic.rgba);
                }
                sh.dims.store(pic.w << 16 | pic.h, Ordering::Relaxed);
                if let Ok(mut s) = sh.slot.lock() {
                    *s = Some(Frame { field: f.field, w: pic.w, h: pic.h, rgba: pic.rgba });
                }
            }
        })
        .ok()?;
    Some((tx, shared))
}
