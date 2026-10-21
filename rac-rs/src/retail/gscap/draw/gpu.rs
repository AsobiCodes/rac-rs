//! GS capture on the GPU (on by default; RAC_GSCAP_GPU=0 draws on the CPU, `low` picks the power-saving adapter, other
//! text part of an adapter's name): the pixel work of the capture (prims, transfers, local copies, CLUT loads, the
//! overlay pictures) runs as compute kernels (gpu.wgsl) on local memory, CLUT and overlay planes that live on the GPU.
//! The CPU keeps the register model, the deferred uploads and each prim's set-up (the same f32 operations as draw.rs)
//! and reads back only the published overlays. PC speed-up of the capture, no retail counterpart: every pixel is the
//! value the CPU path writes (gpu.wgsl: the GPU's float unit where it is exact, integer arithmetic elsewhere); the gsb
//! bench compares the two field by field.
//!
//! Work is recorded as steps and submitted when an overlay is published, a field ends, or a few batches are waiting.
//! Prims go into batches binned into 16 x 16 tiles; each pixel sees its prims in the CPU's order. A batch is shaded
//! one workgroup per (tile, prim) pair (k_shade) and blended one thread per pixel through its tile's list (k_put), or,
//! too big for the shade buffer, both per pixel (k_raster); a pixel that meets a value only the integer path takes
//! stops there and k_redo finishes it. A batch ends where the order would not hold: a prim reading blocks the batch
//! writes, writing blocks it reads, or writing them in another layout; a prim whose own pixels interact (it samples its
//! own target, or two of its pixels share memory) runs on one thread in the CPU's pixel order. Uploads and CLUT loads
//! that touch nothing the open batch reads or writes run before it.

use super::super::{hd_dims, hd_note_rows, hd_target, luts, zlut, Gs, Hd, Overlay, Vtx, BT32, CT32, SKY_PROGRAMS};
use super::{disp_rows, St};

const STRIDE: usize = 80;
const PLANE: u64 = 512 * 512 * 16;
pub(crate) const CUR: u32 = 0;
pub(crate) const DCUR: u32 = 1;
pub(crate) const HELD: u32 = 2;
const STAGE: u32 = 3;
const OV_FILL: u32 = 0;
const OV_COPY: u32 = 1;
const OV_NORM: u32 = 2;

const K_RASTER: u8 = 0;
const K_SERIAL: u8 = 1;
const K_XFER: u8 = 2;
const K_COPY: u8 = 3;
const K_CLUT: u8 = 4;
const K_OV: u8 = 5;
const K_SFTEST: u8 = 6;
const K_REDO: u8 = 7;
const K_SHADE: u8 = 8;
const K_PUT: u8 = 9;
const K_HD: u8 = 10;
const K_HOV: u8 = 11;
/// `k_hd` for a step without a textured prim (gpu.wgsl `HDT` off: no sampler in the kernel).
const K_HDF: u8 = 12;
const KERNELS: [&str; 13] = ["k_raster", "k_serial", "k_xfer", "k_copy", "k_clut", "k_ov", "k_sftest", "k_redo", "k_shade", "k_put", "k_hd", "k_hov", "k_hd"];
/// The sharp HUD's planes (gpu.wgsl `HPS`): pixels of one plane, at most 2048 x 1536. Planes CUR / DCUR / HELD hold 8 bytes a pixel
/// (f16 rgba), the stage plane 4 (RGBA8): `HPS * 28` bytes, then the twin plane (`TWIN`).
const HPS: u64 = 3_145_728;
/// The sharp HUD's twin plane of a small offscreen target (stage 3, the shop's 512 x 128 scratch): plane id `TWIN`, `SPS` pixels of 8 bytes
/// (f16 rgba), after the stage plane: `HPS * 32` bytes in all (gpu.wgsl `SPS`, `hd_ld`).
const TWIN: u32 = 4;
const SPS: u64 = HPS / 2;

/// Words of transfer data / prim records / tile lists and steps one submit holds at most.
const DW_CAP: usize = 8 << 20;
const PW_CAP: usize = 4 << 20;
const TW_CAP: usize = 8 << 20;
const STEP_CAP: usize = 16384;
/// CLUT slots (a CLUT load writes a new one, so a prim recorded before it keeps its own).
const SLOTS: u32 = 1024;
/// Tiles of one raster step (gpu.wgsl `RQ`: a redo word per pixel).
const RQ_TILES: usize = 16384;
/// (tile, prim) pairs of a shade + put batch (gpu.wgsl `SVB`: 256 pixels of 6 words each).
const SV_PAIRS: usize = 4096;

#[derive(Clone)]
struct Step {
    k: u8,
    p: [u32; 16],
    g: (u32, u32),
    /// Placement number (`Gpu::seq`): the step can grow while nothing was placed after it.
    seq: u64,
    /// A raster step (debug timing): its prims, tiles and longest tile list.
    info: (u32, u32, u32),
    /// A transfer step: its registers, the next pixel and the next data word that would continue it.
    x: Option<(u64, u64, u64, u32, u32)>,
}

fn groups(n: u32) -> (u32, u32) {
    let n = n.max(1);
    (n.min(65535), n.div_ceil(65535))
}

fn hit(a: (u32, u32), b: (u32, u32)) -> bool {
    a.0 < b.1 && b.0 < a.1
}

/// [PC] (speed) The open batch's read / write block ranges as disjoint ranges: ranges that overlap or touch (the same layout, for
/// writes) are one. Every use is an overlap test (`hit`) against a range, which a union answers as its parts did, so the clash
/// rules are the same; a P1 page's thousands of tiny prims each wrote their own (lo, hi) and the lists grew to hundreds of entries
/// that every prim scanned.
fn add_read(v: &mut Vec<(u32, u32)>, t: (u32, u32)) {
    if t.0 >= t.1 {
        if !v.contains(&t) {
            v.push(t);
        }
        return;
    }
    let (mut l, mut h) = t;
    let mut i = 0;
    while i < v.len() {
        let e = v[i];
        if e.0 < e.1 && e.0 <= h && l <= e.1 {
            if e.0 <= l && h <= e.1 {
                return;
            }
            l = l.min(e.0);
            h = h.max(e.1);
            v.swap_remove(i);
        } else {
            i += 1;
        }
    }
    v.push((l, h));
}

fn add_write(v: &mut Vec<(u32, u32, u64)>, w: (u32, u32, u64)) {
    if w.0 >= w.1 {
        if !v.contains(&w) {
            v.push(w);
        }
        return;
    }
    let (mut l, mut h, c) = w;
    let mut i = 0;
    while i < v.len() {
        let e = v[i];
        if e.2 == c && e.0 < e.1 && e.0 <= h && l <= e.1 {
            if e.0 <= l && h <= e.1 {
                return;
            }
            l = l.min(e.0);
            h = h.max(e.1);
            v.swap_remove(i);
        } else {
            i += 1;
        }
    }
    v.push((l, h, c));
}

/// Whether distinct pixels of a rectangle ending at (`x1`, `y1`) (exclusive) of a buffer never share memory: the rows
/// stay inside the buffer width (wider x wraps into another page row) and the buffer does not wrap at the end of local
/// memory. Formats `wr_px` does not write count as safe.
fn alias_free(psm: u32, bp: u32, bw: u32, x1: u32, y1: u32) -> bool {
    let (pw, ph, bw) = match psm {
        0x00 | 0x01 | 0x1b | 0x24 | 0x2c => (64, 32, bw),
        0x02 => (64, 64, bw),
        0x13 => (128, 64, (bw / 2).max(1)),
        0x14 => (128, 128, (bw / 2).max(1)),
        _ => return true,
    };
    bw > 0 && x1 <= pw * bw && bp as u64 * 256 + ((y1.max(1) - 1) / ph + 1) as u64 * bw as u64 * 8192 <= 0x40_0000
}

pub(crate) struct Gpu {
    dev: wgpu::Device,
    q: wgpu::Queue,
    pipes: Vec<wgpu::ComputePipeline>,
    bgl: wgpu::BindGroupLayout,
    /// params, vram, ovl, clut, lut, prims, tiles, dat.
    bufs: Vec<wgpu::Buffer>,
    bg: wgpu::BindGroup,
    rb: wgpu::Buffer,
    /// RAC_GSGPU_TIME=1 (debug): a timestamp pair per step (query set, resolve buffer, readable copy), and the step cap.
    ts: Option<(wgpu::QuerySet, wgpu::Buffer, wgpu::Buffer)>,
    cap: usize,
    steps: Vec<Step>,
    pw: Vec<u32>,
    tw: Vec<u32>,
    dw: Vec<u32>,
    /// The open batch: its prims (index, box), the blocks they read and write (with the write layout).
    batch: Vec<(u32, [i32; 4])>,
    reads: Vec<(u32, u32)>,
    writes: Vec<(u32, u32, u64)>,
    /// Steps that run before the open batch, and the CLUT loads among them.
    pre: Vec<Step>,
    loads: u32,
    seq: u64,
    slot: u32,
    grid: Vec<u32>,
    lists: Vec<Vec<u32>>,
    tiles: Vec<(u32, u32)>,
    /// The last transfer's (BITBLTBUF, TRXPOS, TRXREG) with its block span and `serial` (a qword at a time arrives often).
    xc: Option<((u64, u64, u64), (u32, u32), bool)>,
    /// Raster batches since the last submit, and how many start one early.
    since: u32,
    early: u32,
    /// Batches of at most this many tiles shade + put.
    split: usize,
    /// The overlay published since the last submit (its pixels are in the stage plane).
    pub(crate) stage: Option<Overlay>,
    /// The sharp HUD (lane G): the scale in force (kx, ky in 1/256 window pixels per GS pixel; 0 = off), which planes hold a
    /// valid sharp picture, the sharp prims of the open batch (index, box in sharp pixels), the size of the stage plane's
    /// picture to read back with `stage`, and the picture `submit` read (w, rows, RGBA8).
    hd: (u32, u32, u32),
    hd_ok: [bool; 3],
    hd_batch: Vec<(u32, [i32; 4])>,
    stage_hd: Option<(u32, u32)>,
    hd_rb: Option<wgpu::Buffer>,
    hd_out: Option<(u32, u32, Vec<[u8; 4]>)>,
    /// Stage 3: the offscreen 2D target (a render-to-texture scratch the menus draw text and icons into, then composite 1:1 into the
    /// main buffer) whose prims are drawn again, sharp, into plane `TWIN`. `offs`: the page numbers (fbp) of the CT32 512-wide targets
    /// drawn into lately; `twin`: the one an overlay prim textured from (learnt there); `twin_ok`: plane `TWIN` holds the target's
    /// picture (a full opaque fill started it, only prims the sharp pass can draw followed); `twin_rows`: its rows; `twin_span`: its blocks.
    offs: Vec<u32>,
    twin: Option<u32>,
    twin_ok: bool,
    twin_rows: u32,
    twin_span: (u32, u32),
    /// [PC] (speed) Lazy readback (`Gs::set_lazy`): a publishing `submit` only asks for the maps and `rb_finish` collects the
    /// picture later, so the capture records the next job while the GPU finishes this one.
    pub(crate) lazy: bool,
    rb_pend: Option<RbPend>,
    pub(crate) name: String,
    /// Debug counters (`Gs::gpu_stats`).
    pub(crate) st: std::collections::BTreeMap<&'static str, u64>,
}

/// A lazy readback in flight: how many map callbacks have run, how many it needs, and the sharp picture's size.
struct RbPend {
    done: std::sync::Arc<std::sync::atomic::AtomicU32>,
    want: u32,
    hd: Option<(u32, u32)>,
}

/// The device being opened since `prestart`, and the capture of a dropped `Gs` (`recycle`): opening one takes most of a
/// second (device, kernels, self test), which a new capture would otherwise spend while the game waits for it.
static OPENING: std::sync::Mutex<Option<std::thread::JoinHandle<Option<Box<Gpu>>>>> = std::sync::Mutex::new(None);
static FREE: std::sync::Mutex<Option<Box<Gpu>>> = std::sync::Mutex::new(None);
/// What `prestart`'s threads (the opening and the kernel builds) call first (retail/hot.rs `idle`).
static IDLE: std::sync::OnceLock<fn()> = std::sync::OnceLock::new();

pub(crate) enum Take {
    Ready(Box<Gpu>),
    Later,
    Never,
}

fn want() -> Option<String> {
    let w = std::env::var("RAC_GSCAP_GPU").ok().filter(|w| !w.is_empty()).unwrap_or_else(|| "1".into());
    (w != "0").then_some(w)
}

/// The Vulkan pipeline cache of the kernels, kept in the player folder (`gpu_kernels_<adapter, driver and shader hash>.bin`; a file
/// that fails its check or that the driver refuses is ignored, the kernels are then built and the file is written again).
pub(crate) struct PipeCache {
    cache: wgpu::PipelineCache,
    path: std::path::PathBuf,
    loaded: usize,
}

impl PipeCache {
    fn open(dev: &wgpu::Device, info: &wgpu::AdapterInfo, shader: &str) -> PipeCache {
        let fnv = |h: u64, b: &[u8]| b.iter().fold(h, |h, &x| (h ^ x as u64).wrapping_mul(0x0100_0000_01b3));
        let mut key = fnv(0xcbf2_9ce4_8422_2325, info.name.as_bytes());
        key = fnv(key, info.driver.as_bytes());
        key = fnv(key, info.driver_info.as_bytes());
        key = fnv(key, &info.vendor.to_le_bytes());
        key = fnv(key, &info.device.to_le_bytes());
        key = fnv(key, shader.as_bytes());
        let dir = std::env::var_os("RAC_GSGPU_CACHEDIR").map(std::path::PathBuf::from).unwrap_or_else(crate::retail::memcard::player_dir);
        let path = dir.join(format!("gpu_kernels_{key:016x}.bin"));
        // File: 8 bytes check (FNV of the rest), then the driver's data.
        let data = std::fs::read(&path).ok().filter(|d| d.len() > 8 && fnv(0xcbf2_9ce4_8422_2325, &d[8..]).to_le_bytes() == d[..8]);
        let loaded = data.as_ref().map_or(0, |d| d.len() - 8);
        // SAFETY: the data is either None or what `PipelineCache::get_data` returned for this adapter and shader (checked above; the
        // key names the adapter, driver and shader); `fallback` lets wgpu start an empty cache when the driver refuses it anyway.
        let cache = unsafe { dev.create_pipeline_cache(&wgpu::PipelineCacheDescriptor { label: Some("gs kernels"), data: data.as_deref().map(|d| &d[8..]), fallback: true }) };
        PipeCache { cache, path, loaded }
    }

    fn save(&self) {
        let Some(d) = self.cache.get_data() else { return };
        if d.len() == self.loaded {
            return;
        }
        let fnv = d.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x0100_0000_01b3));
        let mut f = fnv.to_le_bytes().to_vec();
        f.extend_from_slice(&d);
        if let Some(p) = self.path.parent() {
            let _ = std::fs::create_dir_all(p);
        }
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, &f).is_ok() && std::fs::rename(&tmp, &self.path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

impl Gpu {
    /// Starts opening the GPU capture in the background (the game's start, before the first `Gs`).
    pub(crate) fn prestart(idle: fn()) {
        let (Some(w), Ok(mut o)) = (want(), OPENING.lock()) else { return };
        if o.is_none() {
            let _ = IDLE.set(idle);
            *o = std::thread::Builder::new()
                .name("gs-gpu-open".into())
                .spawn(move || {
                    idle();
                    Self::open(&w)
                })
                .ok();
        }
    }

    /// A GPU capture starting from `vram`: a recycled one, the one `prestart` opened (`Later` while it is still opening: the
    /// first time a driver builds the kernels can take a minute), or one opened here when none was started.
    pub(crate) fn take(vram: &[u8]) -> Take {
        let Some(want) = want() else { return Take::Never };
        let free = FREE.lock().ok().and_then(|mut f| f.take());
        let g = match free {
            Some(g) => Some(g),
            None => {
                let Ok(mut o) = OPENING.lock() else { return Take::Never };
                if o.as_ref().is_some_and(|h| !h.is_finished()) {
                    return Take::Later;
                }
                let h = o.take();
                drop(o);
                match h {
                    Some(h) => h.join().ok().flatten(),
                    None => Self::open(&want),
                }
            }
        };
        let Some(mut g) = g else {
            eprintln!("gscap: no GPU for the capture ({want}), drawing on the CPU");
            return Take::Never;
        };
        g.reset(vram);
        Take::Ready(g)
    }

    /// Overlay plane `p` from the CPU path's picture.
    fn plane(&mut self, p: u32, px: &[[f32; 4]]) {
        self.q.write_buffer(&self.bufs[2], p as u64 * PLANE, bytemuck::cast_slice(px));
    }

    /// A dropped capture's GPU, kept for the next one.
    pub(crate) fn recycle(mut g: Box<Gpu>) {
        g.rb_drop();
        g.submit(None);
        if let Ok(mut f) = FREE.lock() {
            *f = Some(g);
        }
    }

    /// The state a new capture starts from: `vram`, empty CLUT slots, clear overlay planes, nothing recorded.
    fn reset(&mut self, vram: &[u8]) {
        self.rb_drop();
        self.lazy = false;
        self.submit(None);
        self.stage = None;
        self.hd = (0, 0, 0);
        self.hd_ok = [false; 3];
        self.offs.clear();
        self.twin = None;
        self.twin_ok = false;
        self.hd_batch.clear();
        self.stage_hd = None;
        self.hd_out = None;
        self.xc = None;
        self.since = 0;
        self.slot = 0;
        self.q.write_buffer(&self.bufs[1], 0, &vram[..(4 << 20).min(vram.len())]);
        self.q.write_buffer(&self.bufs[3], 0, &vec![0u8; SLOTS as usize * 512 * 4]);
        let clear: Vec<u32> = (0..512 * 512).flat_map(|_| [0, 0, 0, 0x3f80_0000u32]).collect();
        for p in 0..4 {
            self.q.write_buffer(&self.bufs[2], p * PLANE, bytemuck::cast_slice(&clear));
        }
        self.q.submit([]);
    }

    fn open(want: &str) -> Option<Box<Gpu>> {
        let t0 = std::time::Instant::now();
        let inst = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let adapter = if want == "1" || want == "low" {
            let pp = if want == "low" { wgpu::PowerPreference::LowPower } else { wgpu::PowerPreference::HighPerformance };
            pollster::block_on(inst.request_adapter(&wgpu::RequestAdapterOptions { power_preference: pp, compatible_surface: None, force_fallback_adapter: false, apply_limit_buckets: false })).ok()?
        } else {
            let w = want.to_lowercase();
            pollster::block_on(inst.enumerate_adapters(wgpu::Backends::all())).into_iter().find(|a| a.get_info().name.to_lowercase().contains(&w))?
        };
        let info = adapter.get_info();
        let timing = std::env::var_os("RAC_GSGPU_TIME").is_some() && adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        let mut features = if timing { wgpu::Features::TIMESTAMP_QUERY } else { wgpu::Features::empty() };
        // Vulkan keeps the compiled kernels in a pipeline cache file of the player folder (`PipeCache`): later starts skip the driver's build
        // even when the driver's own cache has lost them.
        let has_cache = adapter.features().contains(wgpu::Features::PIPELINE_CACHE) && std::env::var_os("RAC_GSGPU_NOCACHE").is_none();
        if has_cache {
            features |= wgpu::Features::PIPELINE_CACHE;
        }
        let (dev, q) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor { label: Some("gs capture"), required_features: features, ..Default::default() })).ok()?;
        let sm = dev.create_shader_module(wgpu::ShaderModuleDescriptor { label: Some("gs capture"), source: wgpu::ShaderSource::Wgsl(match std::env::var("RAC_GSGPU_WGSL") {
            // Debug: a shader from disk (timing experiments without a rebuild).
            Ok(f) => std::fs::read_to_string(f).ok()?.into(),
            Err(_) => include_str!("gpu.wgsl").into(),
        }) });
        eprintln!("gscap gpu: {} ({:?})", info.name, info.backend);
        let ent = |binding: u32, ty: wgpu::BufferBindingType, dynamic: bool| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::COMPUTE,
            ty: wgpu::BindingType::Buffer { ty, has_dynamic_offset: dynamic, min_binding_size: if dynamic { wgpu::BufferSize::new(256) } else { None } },
            count: None,
        };
        let rw = wgpu::BufferBindingType::Storage { read_only: false };
        let ro = wgpu::BufferBindingType::Storage { read_only: true };
        let bgl = dev.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gs capture"),
            entries: &[ent(0, wgpu::BufferBindingType::Uniform, true), ent(1, rw, false), ent(2, rw, false), ent(3, rw, false), ent(4, ro, false), ent(5, ro, false), ent(6, ro, false), ent(7, ro, false), ent(8, rw, false)],
        });
        let pl = dev.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor { label: Some("gs capture"), bind_group_layouts: &[Some(&bgl)], immediate_size: 0 });
        let tk = std::time::Instant::now();
        let shader_text: std::borrow::Cow<str> = match std::env::var("RAC_GSGPU_WGSL") {
            Ok(f) => std::fs::read_to_string(f).unwrap_or_default().into(),
            Err(_) => include_str!("gpu.wgsl").into(),
        };
        let pc = has_cache.then(|| PipeCache::open(&dev, &info, &shader_text));
        let pipes = Self::pipes(&dev, &pl, &sm, std::env::var_os("RAC_GSGPU_SOFT").is_some(), pc.as_ref());
        let tk = tk.elapsed().as_millis();
        let st = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        let mk = |label: &str, size: u64, usage| dev.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size, usage, mapped_at_creation: false });
        let bufs = vec![
            mk("gs params", 256 * 1024, wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST),
            mk("gs vram", 4 << 20, st),
            // The four planes, then a word per pixel of a raster step (gpu.wgsl `RQ`).
            mk("gs overlay planes", PLANE * 4 + RQ_TILES as u64 * 256 * 4 + SV_PAIRS as u64 * 256 * 6 * 4, st),
            mk("gs clut slots", SLOTS as u64 * 512 * 4, st),
            mk("gs swizzle", 34816 * 4, st),
            mk("gs prims", 1 << 20, st),
            mk("gs tiles", 1 << 20, st),
            mk("gs data", 1 << 20, st),
            // The sharp HUD's planes (`HPS`): a stand-in until the first sharp picture (`hd_alloc`).
            mk("gs hd planes", 256, st),
        ];
        let rb = mk("gs readback", PLANE, wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST);
        // Swizzle tables: CT32, CT16, T8, T4, Z32, and the inverse of CT32 (`upload_main`).
        let l = luts();
        let mut lut: Vec<u32> = [&l.0, &l.1, &l.2, &l.3, zlut()].iter().flat_map(|v| v.iter().copied()).collect();
        let mut inv = vec![0u32; 2048];
        for (r, row) in BT32.iter().enumerate() {
            for (c, &bb) in row.iter().enumerate() {
                for (rr, crow) in CT32.iter().enumerate() {
                    for (cc, &ww) in crow.iter().enumerate() {
                        inv[bb as usize * 64 + ww as usize] = (c * 8 + cc) as u32 | ((r * 8 + rr) as u32) << 8;
                    }
                }
            }
        }
        lut.extend(inv);
        assert_eq!(lut.len(), 34816);
        q.write_buffer(&bufs[4], 0, bytemuck::cast_slice(&lut));
        let bg = Self::bind(&dev, &bgl, &bufs);
        let ts = timing.then(|| {
            (
                dev.create_query_set(&wgpu::QuerySetDescriptor { label: Some("gs steps"), ty: wgpu::QueryType::Timestamp, count: 4096 }),
                dev.create_buffer(&wgpu::BufferDescriptor { label: Some("gs ts"), size: 4096 * 8, usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC, mapped_at_creation: false }),
                dev.create_buffer(&wgpu::BufferDescriptor { label: Some("gs ts read"), size: 4096 * 8, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }),
            )
        });
        eprintln!("gscap: GPU capture on {:?} {}", info.backend, info.name);
        let mut g = Box::new(Gpu {
            dev,
            q,
            pipes,
            bgl,
            bufs,
            bg,
            rb,
            ts,
            cap: if timing { 2040 } else { STEP_CAP },
            steps: Vec::new(),
            pw: Vec::new(),
            tw: Vec::new(),
            dw: Vec::new(),
            batch: Vec::new(),
            reads: Vec::new(),
            writes: Vec::new(),
            pre: Vec::new(),
            loads: 0,
            seq: 1,
            slot: 0,
            grid: vec![u32::MAX; 65536],
            lists: Vec::new(),
            tiles: Vec::new(),
            xc: None,
            since: 0,
            split: usize::MAX,
            early: 4,
            stage: None,
            hd: (0, 0, 0),
            hd_ok: [false; 3],
            hd_batch: Vec::new(),
            stage_hd: None,
            hd_rb: None,
            hd_out: None,
            offs: Vec::new(),
            twin: None,
            twin_ok: false,
            twin_rows: 0,
            twin_span: (0, 0),
            lazy: false,
            rb_pend: None,
            name: info.name,
            st: Default::default(),
        });
        // The GPU's own float unit must round as the CPU does (gpu.wgsl `okf`): checked once, else the integer path.
        // RAC_GSGPU_SELFTEST=N (debug): N cases per operation, and the report is printed whatever it finds.
        let stn = std::env::var("RAC_GSGPU_SELFTEST").ok().and_then(|v| v.parse::<usize>().ok());
        let (rep, bad) = g.selftest(stn.unwrap_or(4096));
        if stn.is_some() {
            eprintln!("gscap: {rep}");
        }
        if bad > 0 {
            eprintln!("gscap: {rep}; using the integer float path");
            g.pipes = Self::pipes(&g.dev, &pl, &sm, true, pc.as_ref());
        }
        // `reset` gives a new capture its local memory and clear planes (the self test wrote into the first).
        eprintln!("gscap: GPU capture ready in {} ms (kernels {tk} ms)", t0.elapsed().as_millis());
        Some(g)
    }

    /// The kernels; `soft`: every f32 operation on the integer path. The driver's first build of a kernel costs seconds of one core and
    /// a few hundred MB (gpu.wgsl is written to keep that small), so a few workers take the kernels in turn, the costly ones first,
    /// and the two with the whole pixel pipeline never together: the game's own threads keep their cores while this runs.
    fn pipes(dev: &wgpu::Device, pl: &wgpu::PipelineLayout, sm: &wgpu::ShaderModule, soft: bool, cache: Option<&PipeCache>) -> Vec<wgpu::ComputePipeline> {
        // `KERNELS` indices, costly first.
        const ORDER: [usize; 13] = [7, 1, 0, 8, 9, 10, 12, 6, 2, 3, 4, 5, 11];
        let workers = std::env::var("RAC_GSGPU_BUILDERS").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or_else(|| (std::thread::available_parallelism().map_or(4, |n| n.get()) / 4).clamp(1, 3));
        let log = std::env::var_os("RAC_GSGPU_BUILDLOG").is_some();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let big = std::sync::Mutex::new(());
        let out: std::sync::Mutex<Vec<Option<wgpu::ComputePipeline>>> = std::sync::Mutex::new((0..KERNELS.len()).map(|_| None).collect());
        std::thread::scope(|sc| {
            for _ in 0..workers {
                sc.spawn(|| {
                    if let Some(f) = IDLE.get() {
                        f();
                    }
                    loop {
                        let n = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let Some(&ki) = ORDER.get(n) else { break };
                        let k = KERNELS[ki];
                        let c = [
                            ("SOFT", if soft { 1.0 } else { 0.0 }),
                            ("FAST", if matches!(k, "k_raster" | "k_shade" | "k_put") { 1.0 } else { 0.0 }),
                            ("HDT", if ki == K_HDF as usize { 0.0 } else { 1.0 }),
                        ];
                        let _one = matches!(k, "k_redo" | "k_serial").then(|| big.lock());
                        let t = std::time::Instant::now();
                        let p = dev.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
                            label: Some(k),
                            layout: Some(pl),
                            module: sm,
                            entry_point: Some(k),
                            compilation_options: wgpu::PipelineCompilationOptions { constants: &c, ..Default::default() },
                            cache: cache.map(|c| &c.cache),
                        });
                        if log || t.elapsed().as_millis() > 1000 {
                            eprintln!("gscap: kernel {k} took {} ms to build", t.elapsed().as_millis());
                        }
                        if let Ok(mut o) = out.lock() {
                            o[ki] = Some(p);
                        }
                    }
                });
            }
        });
        if let Some(c) = cache {
            c.save();
        }
        out.into_inner().unwrap().into_iter().map(|p| p.unwrap()).collect()
    }

    fn bind(dev: &wgpu::Device, bgl: &wgpu::BindGroupLayout, b: &[wgpu::Buffer]) -> wgpu::BindGroup {
        let e = |i: usize| wgpu::BindGroupEntry { binding: i as u32, resource: b[i].as_entire_binding() };
        dev.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("gs capture"),
            layout: bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::Buffer(wgpu::BufferBinding { buffer: &b[0], offset: 0, size: wgpu::BufferSize::new(256) }) },
                e(1),
                e(2),
                e(3),
                e(4),
                e(5),
                e(6),
                e(7),
                e(8),
            ],
        })
    }

    /// [PC] (speed) `Gs::span` memoizes (and short-cuts tiny boxes) itself; the HashMap that stood here hashed 28 bytes with SipHash
    /// per prim and was cleared every 4096 distinct boxes (a P1 page's thousands of tiny prims).
    fn span(&mut self, k: (u32, u32, u32, u32, u32, u32, u32)) -> (u32, u32) {
        Gs::span(k.0, k.1, k.2, k.3, k.4, k.5, k.6)
    }

    fn step(&mut self, k: u8, p: &[u32], g: (u32, u32)) -> Step {
        let mut a = [0u32; 16];
        a[..p.len()].copy_from_slice(p);
        self.seq += 1;
        Step { k, p: a, g, seq: self.seq, x: None, info: (0, 0, 0) }
    }

    /// Whether a step may go before the open batch (see `place`).
    fn fits(&self, rd: &[(u32, u32)], wr: &[(u32, u32)], hoist: bool) -> bool {
        hoist
            && self.loads < SLOTS - 24
            && !wr.iter().any(|&w| self.reads.iter().any(|&r| hit(r, w)) || self.writes.iter().any(|&(l, h, _)| hit((l, h), w)))
            && !rd.iter().any(|&r| self.writes.iter().any(|&(l, h, _)| hit((l, h), r)))
    }

    /// Places a step: before the open batch when `hoist` and it touches nothing the batch uses (`rd` blocks it reads,
    /// `wr` blocks it writes), else after it (the batch ends).
    fn place(&mut self, s: Step, rd: &[(u32, u32)], wr: &[(u32, u32)], hoist: bool) {
        if !self.batch.is_empty() {
            if self.fits(rd, wr, hoist) {
                self.pre.push(s);
                return;
            }
            self.close();
        }
        self.steps.push(s);
        if self.steps.len() >= self.cap {
            self.submit(None);
        }
    }

    /// The step placed last, when nothing was placed after it.
    fn last(&mut self) -> Option<&mut Step> {
        let seq = self.seq;
        if self.pre.last().is_some_and(|s| s.seq == seq) {
            return self.pre.last_mut();
        }
        self.steps.last_mut().filter(|s| s.seq == seq)
    }

    /// Ends the open batch: bins its prims into tiles and records the raster step after the hoisted steps.
    fn close(&mut self) {
        let pre = std::mem::take(&mut self.pre);
        self.steps.extend(pre);
        if self.batch.is_empty() {
            self.hd_flush();
            return;
        }
        let batch = std::mem::take(&mut self.batch);
        for &(i, b) in &batch {
            for ty in b[2] as u32 / 16..=(b[3] as u32 - 1) / 16 {
                for tx in b[0] as u32 / 16..=(b[1] as u32 - 1) / 16 {
                    let c = (ty * 256 + tx) as usize;
                    if self.grid[c] == u32::MAX {
                        self.grid[c] = self.tiles.len() as u32;
                        if self.lists.len() <= self.tiles.len() {
                            self.lists.push(Vec::new());
                        }
                        self.lists[self.tiles.len()].clear();
                        self.tiles.push((tx, ty));
                    }
                    self.lists[self.grid[c] as usize].push(i);
                }
            }
        }
        let base = self.tw.len() as u32;
        let n = self.tiles.len();
        let mut at = base + n as u32 * 4;
        for (k, &(tx, ty)) in self.tiles.iter().enumerate() {
            let len = self.lists[k].len() as u32;
            self.tw.extend([tx, ty, at, len]);
            at += len;
            self.grid[(ty * 256 + tx) as usize] = u32::MAX;
        }
        for k in 0..n {
            self.tw.extend_from_slice(&self.lists[k]);
        }
        self.tiles.clear();
        self.reads.clear();
        self.writes.clear();
        self.loads = 0;
        *self.st.entry("batches").or_default() += 1;
        *self.st.entry("batch prims").or_default() += batch.len() as u64;
        *self.st.entry("batch tiles").or_default() += n as u64;
        let most = self.lists[..n].iter().map(|l| l.len() as u32).max().unwrap_or(0);
        let pairs: usize = self.lists[..n].iter().map(|l| l.len()).sum();
        // One thread per pixel through a whole tile list keeps most of the GPU idle (a menu: few tiles, long lists):
        // shade every (tile, prim) pair at once, then blend in order.
        if n <= self.split && pairs <= SV_PAIRS && pairs > n {
            for k in 0..n {
                let len = self.lists[k].len();
                self.tw.extend(std::iter::repeat_n(k as u32, len));
            }
            let p = [base, n as u32, pairs as u32];
            let mut a = self.step(K_SHADE, &p, groups(pairs as u32));
            a.seq = 0;
            a.info = (batch.len() as u32, n as u32, most);
            let mut b = self.step(K_PUT, &p, groups(n as u32));
            b.seq = 0;
            let mut r = self.step(K_REDO, &p, groups(n as u32));
            r.seq = 0;
            self.steps.extend([a, b, r]);
        } else {
        for c in (0..n).step_by(RQ_TILES) {
            let (b, m) = (base + c as u32 * 4, (n - c).min(RQ_TILES) as u32);
            let mut s = self.step(K_RASTER, &[b, m], groups(m));
            s.seq = 0;
            s.info = (batch.len() as u32, m, most);
            // The pixels that met a value the hardware float path cannot take, drawn again from that prim on.
            let mut r = self.step(K_REDO, &[b, m], groups(m));
            r.seq = 0;
            self.steps.push(s);
            self.steps.push(r);
        }
        }
        // The sharp HUD's second pass over this batch's overlay prims (after the batch, before any submit below: `pw` restarts there).
        self.hd_flush();
        // Hands the GPU work while the job goes on (a published overlay then waits only for the rest).
        self.since += 1;
        if self.since >= self.early {
            self.submit(None);
            return;
        }
        if self.tw.len() >= TW_CAP || self.steps.len() >= STEP_CAP {
            self.submit(None);
        }
    }

    /// A prim record: into the open batch (`bb` its pixels, `ts` the blocks it samples, `ws` the blocks it writes with
    /// their layout), or alone on one thread (`serial`).
    fn prim(&mut self, r: &[u32; STRIDE], bb: [i32; 4], ts: Option<(u32, u32)>, ws: &[(u32, u32, u64)], serial: bool, hd: Option<[i32; 4]>) {
        if self.pw.len() + STRIDE > PW_CAP {
            self.submit(None);
        }
        // `close` may submit (the job goes to the GPU and `pw` starts again): the prim's index is taken after it.
        if serial {
            self.close();
            let i = (self.pw.len() / STRIDE) as u32;
            self.pw.extend_from_slice(r);
            let s = self.step(K_SERIAL, &[i], (1, 1));
            self.steps.push(s);
            if let Some(b) = hd {
                self.hd_batch.push((i, b));
                self.hd_flush();
            }
            if self.steps.len() >= self.cap {
                self.submit(None);
            }
            return;
        }
        if !self.batch.is_empty() {
            let clash = ts.is_some_and(|t| self.writes.iter().any(|&(l, h, _)| hit((l, h), t)))
                || ws.iter().any(|&(l, h, c)| self.reads.iter().any(|&r| hit(r, (l, h))) || self.writes.iter().any(|&(l2, h2, c2)| c2 != c && hit((l, h), (l2, h2))));
            if clash {
                self.close();
            }
        }
        let i = (self.pw.len() / STRIDE) as u32;
        self.pw.extend_from_slice(r);
        self.batch.push((i, bb));
        if let Some(b) = hd {
            self.hd_batch.push((i, b));
        }
        if let Some(t) = ts {
            add_read(&mut self.reads, t);
        }
        for &w in ws {
            add_write(&mut self.writes, w);
        }
        self.seq += 1;
    }

    /// Host -> local transfer pixels `first..` (`ppq` a qword) from `data`; `main` (first block, end block, width) when
    /// they may land in the main buffer (`upload_main`).
    #[allow(clippy::too_many_arguments)]
    fn xfer(&mut self, regs: [u32; 7], key: (u64, u64, u64), first: u32, ppq: u32, data: &[u128], span: (u32, u32), serial: bool, main: Option<(u32, u32, u32)>) {
        self.twin_touch(span);
        for (c, ch) in data.chunks(1 << 20).enumerate() {
            let first = first + (c << 20) as u32 * ppq;
            // A step that cannot go before the open batch ends it first: `close` may submit, and the data offset below must
            // be taken after that (`dw` starts again).
            if !self.batch.is_empty() && !self.fits(&[], &[span], main.is_none()) {
                self.close();
            }
            if self.dw.len() + ch.len() * 4 > DW_CAP {
                self.submit(None);
            }
            let dword = self.dw.len() as u32;
            for q in ch {
                self.dw.extend([*q as u32, (*q >> 32) as u32, (*q >> 64) as u32, (*q >> 96) as u32]);
            }
            let n = ch.len() as u32 * ppq;
            let next = (key.0, key.1, key.2, first + n, self.dw.len() as u32);
            if let Some(s) = self.last() {
                if s.k == K_XFER && s.x.is_some_and(|x| (x.0, x.1, x.2) == key && x.3 == first && x.4 == dword) {
                    s.p[8] += n;
                    s.x = Some(next);
                    if s.p[10] == 0 {
                        s.g = groups(s.p[8].div_ceil(64));
                    }
                    continue;
                }
            }
            let m = main.unwrap_or((0, 0, 0));
            let mut s = self.step(K_XFER, &[regs[0], regs[1], regs[2], regs[3], regs[4], regs[5], regs[6], first, n, dword, serial as u32, main.is_some() as u32, m.0, m.1, m.2], if serial { (1, 1) } else { groups(n.div_ceil(64)) });
            s.x = Some(next);
            // The sharp HUD's picture is cleared where the upload lands in the main buffer (gpu.wgsl `hd_clear`): its scale.
            s.p[15] = if main.is_some() && self.hd_ok[CUR as usize] { self.hd.0 | self.hd.1 << 16 } else { 0 };
            // An upload that changes the overlay keeps its place after the batch's prims.
            self.place(s, &[], &[span], main.is_none());
        }
    }

    fn clut(&mut self, cbp: u32, cfmt: u32, t4: bool, off: u32, texa: u64) {
        if !self.batch.is_empty() {
            self.loads += 1;
        }
        let prev = self.slot;
        self.slot = (self.slot + 1) % SLOTS;
        // As in `xfer`: end the batch first when the load cannot go before it (`close` may submit and restart `dw`).
        if !self.batch.is_empty() && !self.fits(&[(cbp, cbp + 4)], &[], true) {
            self.close();
        }
        if self.dw.len() + 8 > DW_CAP {
            self.submit(None);
        }
        let at = self.dw.len() as u32;
        self.dw.extend([prev, self.slot, cbp, cfmt, t4 as u32, off, texa as u32, (texa >> 32) as u32]);
        let rd = (cbp, cbp + 4);
        // Joins the load before it when it could have been placed there on its own.
        let safe = self.batch.is_empty() || (self.loads < SLOTS - 24 && !self.writes.iter().any(|&(l, h, _)| hit((l, h), rd)));
        if safe {
            if let Some(s) = self.last() {
                if s.k == K_CLUT && s.p[0] + s.p[1] * 8 == at {
                    s.p[1] += 1;
                    return;
                }
            }
        }
        let s = self.step(K_CLUT, &[at, 1], (1, 1));
        self.place(s, &[rd], &[], true);
    }

    fn ov(&mut self, op: u32, src: u32, dst: u32) {
        let s = self.step(K_OV, &[op, src, dst], (1024, 1));
        self.place(s, &[], &[], false);
        self.hd_ov(op, src, dst);
    }

    /// The sharp HUD planes' twin of an `ov` step (`hd_ok`: which planes hold a valid sharp picture; a plane the sharp pass did not
    /// follow from a clear stays invalid, and a publish of it carries no sharp picture).
    fn hd_ov(&mut self, op: u32, src: u32, dst: u32) {
        if op == OV_FILL && dst == CUR {
            // A new frame: the scale changes only here.
            let t = hd_target();
            if t != self.hd {
                self.hd = t;
                self.hd_ok = [false; 3];
                self.twin_ok = false;
                if t.0 != 0 {
                    self.hd_alloc();
                }
            }
        }
        if self.hd.0 == 0 || self.hd_rb.is_none() {
            return;
        }
        let (s, d) = (src as usize, dst as usize);
        match op {
            OV_FILL if d < 3 => self.hd_ok[d] = true,
            OV_COPY if d < 3 && s < 3 => {
                self.hd_ok[d] = self.hd_ok[s];
                if !self.hd_ok[d] {
                    return;
                }
            }
            OV_NORM if s < 3 && self.hd_ok[s] => {}
            _ => return,
        }
        let (wp, hp) = hd_dims(self.hd.0, self.hd.1, self.hd.2);
        let st = self.step(K_HOV, &[op, src, dst, wp * hp], groups((wp * hp).div_ceil(256)));
        self.place(st, &[], &[], false);
    }

    /// Stage 3. A write into the twin's blocks other than its own prims (an upload, a copy, a prim into another buffer that aliases it)
    /// ends the twin: plane `TWIN` no longer holds what the 512 picture of the scratch holds.
    fn twin_touch(&mut self, sp: (u32, u32)) {
        if self.twin_ok && hit(sp, self.twin_span) {
            self.twin_ok = false;
            *self.st.entry("twin dropped: other write").or_default() += 1;
        }
    }

    /// Stage 3. Whether a prim drawn into a VRAM target also goes to plane `TWIN` (the sharp pass draws it again at the window's
    /// resolution, with the sharp sampler on its textures). The CT32 512-wide targets drawn into are noted (`offs`); an overlay prim that
    /// textures from one 1:1 (gpu_raster) makes it the twin. A twin starts with a full opaque fill of its scissor (the clear sprite
    /// before each menu item), and stays while every prim into the target is one the sharp pass draws like the 512 path (CT32, no frame
    /// mask, no Z buffer, colour clamp on); any other write drops it until the next fill.
    fn twin_prim(&mut self, st: &St, kind: u32, bb: [i32; 4], clamp: bool) -> bool {
        let fr = st.frame;
        let (fbp, fbw, psm) = ((fr & 0x1ff) as u32, ((fr >> 16) & 0x3f) as u32, ((fr >> 24) & 0x3f) as u32);
        if psm == 0 && fbw == 8 && !self.offs.contains(&fbp) {
            if self.offs.len() >= 4 {
                self.offs.remove(0);
            }
            self.offs.push(fbp);
        }
        if self.twin != Some(fbp) {
            return false;
        }
        if self.hd.0 == 0 || self.hd_rb.is_none() {
            self.twin_ok = false;
            return false;
        }
        let ok = psm == 0 && fbw == 8 && (fr >> 32) as u32 == 0 && st.zb.is_none() && clamp;
        if !ok {
            self.twin_ok = false;
            let why = if psm != 0 {
                "twin off: psm"
            } else if fbw != 8 {
                "twin off: width"
            } else if (fr >> 32) as u32 != 0 {
                "twin off: frame mask"
            } else if st.zb.is_some() {
                "twin off: z buffer"
            } else {
                "twin off: colour wrap"
            };
            *self.st.entry(why).or_default() += 1;
            return false;
        }
        // The clear sprite: untextured, opaque, an alpha test that passes, the target's whole scissor (full width, from the top) covered.
        let h = (st.sc[3] + 1).max(0) as u32;
        let (wp, _) = hd_dims(self.hd.0, self.hd.1, self.hd.2);
        let fits = h > 0 && h <= 256 && ((h * self.hd.1).div_ceil(256) as u64 + 1) * wp as u64 <= SPS;
        let passes = st.test & 1 == 0 || (st.test >> 1) & 7 == 1;
        if kind == 2 && st.at & 0x50 == 0 && passes && fits && st.sc[0] == 0 && st.sc[1] == 511 && st.sc[2] == 0 && bb[0] <= 0 && bb[1] >= 512 && bb[2] <= 0 && bb[3] >= h as i32 {
            self.twin_ok = true;
            *self.st.entry("twin clears").or_default() += 1;
            self.twin_rows = h;
            self.twin_span = self.span((0, fbp * 32, 8, 0, 0, 512, h));
        }
        if self.twin_ok {
            *self.st.entry("twin prims").or_default() += 1;
        } else {
            *self.st.entry("twin prims before a clear").or_default() += 1;
        }
        self.twin_ok
    }

    /// The sharp HUD's planes (`HPS`): allocated at the first sharp picture.
    fn hd_alloc(&mut self) {
        if self.hd_rb.is_some() {
            return;
        }
        let st = wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::COPY_SRC;
        self.bufs[8] = self.dev.create_buffer(&wgpu::BufferDescriptor { label: Some("gs hd planes"), size: HPS * 32, usage: st, mapped_at_creation: false });
        self.hd_rb = Some(self.dev.create_buffer(&wgpu::BufferDescriptor { label: Some("gs hd readback"), size: HPS * 4, usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false }));
        self.bg = Self::bind(&self.dev, &self.bgl, &self.bufs);
    }

    /// The sharp prims of the batch just recorded (or the serial prim), binned into 16 x 16 tiles of the sharp pixels: one step
    /// (gpu.wgsl `k_hd`) that draws them again at the window's resolution, tile by tile, each pixel through its prims in order.
    fn hd_flush(&mut self) {
        if self.hd_batch.is_empty() {
            return;
        }
        let batch = std::mem::take(&mut self.hd_batch);
        let (kx, ky, rows) = self.hd;
        let (wp, hp) = hd_dims(kx, ky, rows);
        for &(i, b) in &batch {
            let (x0, x1, y0, y1) = (b[0].max(0) as u32, (b[1].max(0) as u32).min(wp), b[2].max(0) as u32, (b[3].max(0) as u32).min(hp));
            if x0 >= x1 || y0 >= y1 {
                continue;
            }
            for ty in y0 / 16..=(y1 - 1) / 16 {
                for tx in x0 / 16..=(x1 - 1) / 16 {
                    let c = (ty * 256 + tx) as usize;
                    if self.grid[c] == u32::MAX {
                        self.grid[c] = self.tiles.len() as u32;
                        if self.lists.len() <= self.tiles.len() {
                            self.lists.push(Vec::new());
                        }
                        self.lists[self.tiles.len()].clear();
                        self.tiles.push((tx, ty));
                    }
                    self.lists[self.grid[c] as usize].push(i);
                }
            }
        }
        let n = self.tiles.len();
        if n == 0 {
            return;
        }
        let base = self.tw.len() as u32;
        let mut at = base + n as u32 * 4;
        for (k, &(tx, ty)) in self.tiles.iter().enumerate() {
            let len = self.lists[k].len() as u32;
            self.tw.extend([tx, ty, at, len]);
            at += len;
            self.grid[(ty * 256 + tx) as usize] = u32::MAX;
        }
        for k in 0..n {
            self.tw.extend_from_slice(&self.lists[k]);
        }
        self.tiles.clear();
        // A step without a textured prim (record word 8 `at`, bit 0x10) takes the kernel without the sampler.
        let tex = batch.iter().any(|&(i, _)| self.pw[i as usize * STRIDE + 8] & 0x10 != 0);
        let mut s = self.step(if tex { K_HD } else { K_HDF }, &[base, n as u32, kx, ky, wp, hp, rows], groups(n as u32));
        s.seq = 0;
        self.steps.push(s);
        *self.st.entry("hd prims").or_default() += batch.len() as u64;
        *self.st.entry("hd tiles").or_default() += n as u64;
    }

    /// Grows buffer `i` to hold `bytes` (a new bind group).
    fn fit(&mut self, i: usize, bytes: usize) {
        if (self.bufs[i].size() as usize) < bytes {
            let size = (bytes.next_power_of_two() as u64).max(self.bufs[i].size() * 2);
            let usage = self.bufs[i].usage();
            self.bufs[i] = self.dev.create_buffer(&wgpu::BufferDescriptor { label: Some("gs capture"), size, usage, mapped_at_creation: false });
            self.bg = Self::bind(&self.dev, &self.bgl, &self.bufs);
        }
    }

    /// Runs everything recorded; with `read` (buffer 1 = VRAM or 2 = overlay planes, byte offset) returns that 4 MiB.
    fn submit(&mut self, read: Option<(usize, u64)>) -> Option<Vec<u32>> {
        let t_prep = std::time::Instant::now();
        self.close();
        self.since = 0;
        if self.steps.is_empty() && read.is_none() {
            return None;
        }
        let steps = std::mem::take(&mut self.steps);
        if !steps.is_empty() {
            let mut pb = vec![0u32; steps.len() * 64];
            for (i, s) in steps.iter().enumerate() {
                pb[i * 64..i * 64 + 16].copy_from_slice(&s.p);
            }
            self.fit(0, pb.len() * 4);
            self.q.write_buffer(&self.bufs[0], 0, bytemuck::cast_slice(&pb));
            for (i, w) in [(5, std::mem::take(&mut self.pw)), (6, std::mem::take(&mut self.tw)), (7, std::mem::take(&mut self.dw))] {
                if !w.is_empty() {
                    self.fit(i, w.len() * 4);
                    self.q.write_buffer(&self.bufs[i], 0, bytemuck::cast_slice(&w));
                }
            }
        }
        let mut enc = self.dev.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("gs capture") });
        if let (false, Some((qs, qr, qm))) = (steps.is_empty(), &self.ts) {
            for (i, s) in steps.iter().enumerate() {
                let tw = wgpu::ComputePassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(2 * i as u32), end_of_pass_write_index: Some(2 * i as u32 + 1) };
                let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("gs step"), timestamp_writes: Some(tw) });
                pass.set_pipeline(&self.pipes[s.k as usize]);
                pass.set_bind_group(0, &self.bg, &[(i * 256) as u32]);
                pass.dispatch_workgroups(s.g.0, s.g.1, 1);
            }
            let n = 2 * steps.len() as u32;
            enc.resolve_query_set(qs, 0..n, qr, 0);
            enc.copy_buffer_to_buffer(qr, 0, qm, 0, n as u64 * 8);
        } else if !steps.is_empty() {
            let mut pass = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("gs capture"), timestamp_writes: None });
            for (i, s) in steps.iter().enumerate() {
                pass.set_pipeline(&self.pipes[s.k as usize]);
                pass.set_bind_group(0, &self.bg, &[(i * 256) as u32]);
                pass.dispatch_workgroups(s.g.0, s.g.1, 1);
            }
        }
        // The sharp HUD's published picture (the stage plane after the planes) goes with the 4 MiB.
        let hdr = if read.is_some() { self.stage_hd.take().zip(self.hd_rb.as_ref()) } else { None };
        if let Some((b, off)) = read {
            enc.copy_buffer_to_buffer(&self.bufs[b], off, &self.rb, 0, PLANE);
            if let Some(((w, rows), rbh)) = hdr {
                enc.copy_buffer_to_buffer(&self.bufs[8], HPS * 24, rbh, 0, w as u64 * rows as u64 * 4);
            }
        }
        *self.st.entry("submits").or_default() += 1;
        *self.st.entry("steps").or_default() += steps.len() as u64;
        for s in &steps {
            *self.st.entry(KERNELS[s.k as usize]).or_default() += 1;
        }
        self.q.submit([enc.finish()]);
        *self.st.entry("submit prep us").or_default() += t_prep.elapsed().as_micros() as u64;
        if let (false, Some((_, _, qm))) = (steps.is_empty(), &self.ts) {
            qm.slice(..).map_async(wgpu::MapMode::Read, |_| {});
            self.dev.poll(wgpu::PollType::wait_indefinitely()).ok()?;
            let v: Vec<u64> = bytemuck::cast_slice(&qm.slice(..).get_mapped_range().ok()?[..steps.len() * 16]).to_vec();
            qm.unmap();
            let per = self.q.get_timestamp_period() as f64;
            for (i, s) in steps.iter().enumerate() {
                let ns = (v[2 * i + 1].saturating_sub(v[2 * i]) as f64 * per) as u64;
                if (s.k == K_RASTER || s.k == K_SHADE) && ns > 300_000 {
                    eprintln!("gscap gpu: batch {} us, {} prims, {} tiles, longest tile list {}", ns / 1000, s.info.0, s.info.1, s.info.2);
                }
                *self.st.entry(["gpu ns k_raster", "gpu ns k_serial", "gpu ns k_xfer", "gpu ns k_copy", "gpu ns k_clut", "gpu ns k_ov", "gpu ns k_sftest", "gpu ns k_redo", "gpu ns k_shade", "gpu ns k_put", "gpu ns k_hd", "gpu ns k_hov", "gpu ns k_hd flat"][s.k as usize]).or_default() += ns;
            }
        }
        read?;
        if self.lazy {
            // [PC] (speed) Only ask for the maps; the GPU copies while the capture records on (`rb_finish`).
            let n = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
            let c = n.clone();
            self.rb.slice(..).map_async(wgpu::MapMode::Read, move |_| {
                c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            });
            let hd = hdr.map(|((w, rows), rbh)| {
                let c = n.clone();
                rbh.slice(0..w as u64 * rows as u64 * 4).map_async(wgpu::MapMode::Read, move |_| {
                    c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                });
                (w, rows)
            });
            self.rb_pend = Some(RbPend { done: n, want: 1 + hd.is_some() as u32, hd });
            return None;
        }
        let t = std::time::Instant::now();
        self.rb.slice(..).map_async(wgpu::MapMode::Read, |_| {});
        let hdn = hdr.map(|((w, rows), rbh)| (w, rows, rbh.slice(0..w as u64 * rows as u64 * 4)));
        if let Some((_, _, sl)) = &hdn {
            sl.map_async(wgpu::MapMode::Read, |_| {});
        }
        self.dev.poll(wgpu::PollType::wait_indefinitely()).ok()?;
        *self.st.entry("readback wait us").or_default() += t.elapsed().as_micros() as u64;
        let mut v = vec![0u32; PLANE as usize / 4];
        v.copy_from_slice(bytemuck::cast_slice(&self.rb.slice(..).get_mapped_range().ok()?));
        self.rb.unmap();
        if let (Some((w, rows, sl)), Some(rbh)) = (hdn, self.hd_rb.as_ref()) {
            let px: Vec<[u8; 4]> = bytemuck::cast_slice(&sl.get_mapped_range().ok()?).to_vec();
            rbh.unmap();
            self.hd_out = Some((w, rows, px));
        }
        Some(v)
    }

    /// [PC] (speed) The lazy readback's picture (stage plane; the sharp picture goes to `hd_out`): `block` waits for the GPU,
    /// else None while it is still copying.
    fn rb_finish(&mut self, block: bool) -> Option<Vec<u32>> {
        let p = self.rb_pend.as_ref()?;
        let (done, want, hd) = (p.done.clone(), p.want, p.hd);
        self.dev.poll(if block { wgpu::PollType::wait_indefinitely() } else { wgpu::PollType::Poll }).ok();
        if done.load(std::sync::atomic::Ordering::SeqCst) < want {
            return None;
        }
        self.rb_pend = None;
        let mut v = vec![0u32; PLANE as usize / 4];
        v.copy_from_slice(bytemuck::cast_slice(&self.rb.slice(..).get_mapped_range().ok()?));
        self.rb.unmap();
        if let (Some((w, rows)), Some(rbh)) = (hd, self.hd_rb.as_ref()) {
            let px: Vec<[u8; 4]> = bytemuck::cast_slice(&rbh.slice(0..w as u64 * rows as u64 * 4).get_mapped_range().ok()?).to_vec();
            rbh.unmap();
            self.hd_out = Some((w, rows, px));
        }
        Some(v)
    }

    /// A lazy readback still in flight is waited for and dropped (the buffers go to the next capture).
    fn rb_drop(&mut self) {
        if self.rb_pend.is_some() {
            self.rb_finish(true);
            self.hd_out = None;
        }
    }

    /// Soft-float self test: `n` random (and special) operand pairs per operation through `k_sftest` against the CPU's
    /// own f32 operations. Returns a report line.
    pub(crate) fn selftest(&mut self, n: usize) -> (String, usize) {
        let n = n.min(1 << 18);
        let mut s = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let special = [0u32, 0x8000_0000, 0x3f80_0000, 0xbf80_0000, 0x7f80_0000, 0xff80_0000, 0x7fc0_0000, 0x7f80_0001, 0x0000_0001, 0x807f_ffff, 0x7f7f_ffff, 0x0080_0000, 0x3f00_0000, 0x4b00_0000, 0x4effffff, 0x4f000000, 0xcf000000, 0x4f800000, 0x3effffff];
        let mut cases = Vec::new();
        for op in 0..15u32 {
            for k in 0..n {
                let pick = |r: u64| -> u32 {
                    match r % 8 {
                        0 => special[(r >> 8) as usize % special.len()],
                        1 => ((r >> 8) as u32 & 0x807f_ffff) | (((r >> 40) as u32 % 4) << 23),
                        2 => (((r >> 8) as i32 % 70000) as f32 / 16.0).to_bits(),
                        3 => (((r >> 8) as u32 % 256) as f32).to_bits(),
                        _ => (r >> 16) as u32,
                    }
                };
                let (a, b) = (pick(rnd()), if k % 5 == 0 { pick(rnd()) } else { (rnd() >> 3) as u32 });
                cases.push((op, a, b));
            }
        }
        let mut out = Vec::new();
        let (mut bad, mut nan, mut fast_bad) = (0usize, 0usize, 0usize);
        let mut first = String::new();
        for ch in cases.chunks(1 << 18) {
            let d: Vec<u32> = ch.iter().flat_map(|c| [c.0, c.1, c.2]).collect();
            self.dw = d;
            let st = self.step(K_SFTEST, &[ch.len() as u32], groups((ch.len() as u32).div_ceil(64)));
            self.steps.push(st);
            let r = self.submit(Some((2, 0))).unwrap_or_default();
            out.extend_from_slice(&r[..ch.len()]);
        }
        let ffl = |x: f32| -> f32 { super::ffloor(x) };
        for (i, &(op, a, b)) in cases.iter().enumerate() {
            let (fa, fb) = (std::hint::black_box(f32::from_bits(a)), std::hint::black_box(f32::from_bits(b)));
            let (want, isf) = match op {
                0 => ((fa + fb).to_bits(), true),
                1 => ((fa - fb).to_bits(), true),
                2 => ((fa * fb).to_bits(), true),
                3 => ((fa / fb).to_bits(), true),
                4 => ((fa as i32) as u32, false),
                5 => (fa as u32, false),
                6 => (fa.round() as i32 as u32, false),
                7 | 13 => (ffl(fa).to_bits(), true),
                8 => ((fa < fb) as u32 | ((fa == fb) as u32) << 1, false),
                9 => ((fa + fb).to_bits(), true),
                10 => ((fa - fb).to_bits(), true),
                11 => ((fa * fb).to_bits(), true),
                12 => ((fa / fb).to_bits(), true),
                _ => ((std::hint::black_box(fa * fb) + fa).to_bits(), true),
            };
            let got = out[i];
            if got != want {
                if isf && f32::from_bits(got).is_nan() && f32::from_bits(want).is_nan() {
                    nan += 1;
                    continue;
                }
                bad += 1;
                fast_bad += (op >= 9) as usize;
                if first.len() < 600 {
                    first += &format!(" [op {op} {a:08x} {b:08x}: gpu {got:08x} cpu {want:08x}]");
                }
            }
        }
        (format!("float self test on {}: {} cases, {bad} wrong ({fast_bad} on the fast path), {nan} NaN payload differences{first}", self.name, cases.len()), fast_bad)
    }
}

impl Gs {
    /// `put` of a host -> local transfer on the GPU: data qwords `q0..` of the transfer `bb`/`pos`/`rr`. The CPU keeps what
    /// `upload_main` decides besides the pixels (the frozen scene, the VU1 program count).
    pub(in super::super) fn gpu_xfer(&mut self, bb: u64, pos: u64, rr: u64, q0: u32, data: &[u128]) {
        let t0 = self.prof.is_some().then(std::time::Instant::now);
        self.gpu_xfer_(bb, pos, rr, q0, data);
        self.prof_add("gpu rec xfer", t0);
    }

    fn gpu_xfer_(&mut self, bb: u64, pos: u64, rr: u64, q0: u32, data: &[u128]) {
        let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
        let (dx, dy) = (((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
        let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
        let ppq = 128 / Self::bpp(dpsm);
        let total = rw * rh;
        let first = q0 * ppq;
        if data.is_empty() || first >= total {
            return;
        }
        let g = self.gpu.as_mut().unwrap();
        let (span, serial) = match g.xc {
            Some((k, s, se)) if k == (bb, pos, rr) => (s, se),
            _ => {
                let v = (g.span((dpsm, dbp, dbw, dx, dy, rw, rh)), !alias_free(dpsm, dbp, dbw, dx + rw, dy + rh));
                g.xc = Some(((bb, pos, rr), v.0, v.1));
                v
            }
        };
        let mut main = None;
        if let (0, Some(f)) = (dpsm, self.main_fbp) {
            let (m0, m1) = (f * 32, self.main_end);
            if span.0 < m1 && m0 < span.1 {
                main = Some((m0, m1, self.main_bw));
                if !(self.cur.frozen && self.mscal >= SKY_PROGRAMS) {
                    let end = ((q0 + data.len() as u32) * ppq).min(total);
                    for g in first..end {
                        let blk = (Self::a32(dbp, dbw, dx + g % rw, dy + g / rw) / 256) as u32;
                        if blk >= m0 && blk < m1 {
                            self.mscal = self.mscal.max(SKY_PROGRAMS);
                            self.cur.frozen = true;
                            break;
                        }
                    }
                }
            }
        }
        self.gpu.as_mut().unwrap().xfer([dbp, dbw, dpsm, dx, dy, rw, rh], (bb, pos, rr), first, ppq, data, span, serial, main);
    }

    /// `local_copy` on the GPU.
    pub(in super::super) fn gpu_copy(&mut self) {
        let bb = self.r[0x50];
        let (sbp, sbw, spsm) = ((bb & 0x3fff) as u32, ((bb >> 16) & 0x3f) as u32, ((bb >> 24) & 0x3f) as u32);
        let (dbp, dbw, dpsm) = (((bb >> 32) & 0x3fff) as u32, ((bb >> 48) & 0x3f) as u32, ((bb >> 56) & 0x3f) as u32);
        let pos = self.r[0x51];
        let (sx, sy, dx, dy) = ((pos & 0x7ff) as u32, ((pos >> 16) & 0x7ff) as u32, ((pos >> 32) & 0x7ff) as u32, ((pos >> 48) & 0x7ff) as u32);
        let rr = self.r[0x52];
        let (rw, rh) = ((rr & 0xfff) as u32, ((rr >> 32) & 0xfff) as u32);
        if rw == 0 || rh == 0 {
            return;
        }
        let g = self.gpu.as_mut().unwrap();
        let s = g.span((spsm, sbp, sbw, sx, sy, rw, rh));
        let d = g.span((dpsm, dbp, dbw, dx, dy, rw, rh));
        g.twin_touch(d);
        let serial = hit(s, d) || !alias_free(dpsm, dbp, dbw, dx + rw, dy + rh);
        let st = g.step(K_COPY, &[sbp, sbw, spsm, dbp, dbw, dpsm, sx, sy, dx, dy, rw, rh, serial as u32], if serial { (1, 1) } else { groups((rw * rh).div_ceil(64)) });
        g.place(st, &[s], &[d], false);
    }

    /// The loading half of `clut_load` on the GPU.
    pub(in super::super) fn gpu_clut(&mut self, cbp: u32, cfmt: u32, t4: bool, off: usize) {
        let t0 = self.prof.is_some().then(std::time::Instant::now);
        self.gpu_clut_(cbp, cfmt, t4, off);
        self.prof_add("gpu rec clut", t0);
    }

    fn gpu_clut_(&mut self, cbp: u32, cfmt: u32, t4: bool, off: usize) {
        let texa = self.r[0x3b];
        self.gpu.as_mut().unwrap().clut(cbp, cfmt, t4, off as u32, texa);
    }

    pub(in super::super) fn gpu_ov(&mut self, op: u32, src: u32, dst: u32) {
        self.gpu.as_mut().unwrap().ov(op, src, dst);
    }

    pub(in super::super) fn gpu_fill(&mut self, plane: u32) {
        self.gpu_ov(OV_FILL, plane, plane);
    }

    /// `publish` / `field_end` on the GPU: `src` (the main or display picture) becomes the held picture and, normalised,
    /// the stage the end of the job reads back as `o`.
    pub(in super::super) fn gpu_publish(&mut self, src: u32, o: Overlay) {
        // The sharp picture goes with it when the plane holds one (and the overlay has the 512 GS pixels' width).
        let g = self.gpu.as_mut().unwrap();
        // A frame of other rows than the scale assumed has none (the next frame's scale follows `hd_note_rows`).
        hd_note_rows(o.rows);
        let hd = (g.hd.0 != 0 && o.w == 512 && o.rows == g.hd.2 && g.hd_ok[src as usize]).then(|| hd_dims(g.hd.0, g.hd.1, g.hd.2));
        self.gpu_ov(OV_COPY, src, HELD);
        self.gpu_ov(OV_NORM, src, STAGE);
        if src == CUR {
            self.gpu_fill(CUR);
        }
        let g = self.gpu.as_mut().unwrap();
        g.stage = Some(o);
        g.stage_hd = hd;
    }

    /// A field end while the GPU capture was still opening (`Take::Later`): once it is ready it takes over
    /// from the CPU path with the CPU's local memory, CLUT, overlay pictures and transfer position, so its pixels go on
    /// as the CPU's would.
    pub(in super::super) fn gpu_attach(&mut self) {
        let mut g = match Gpu::take(&self.vram) {
            Take::Later => return,
            Take::Never => {
                self.gpu_wait = 0;
                return;
            }
            Take::Ready(g) => g,
        };
        self.gpu_wait = 0;
        // The current CLUT is slot 0 after `reset`.
        let clut: Vec<u32> = self.clut.iter().map(|c| c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16 | (c[3] as u32) << 24).collect();
        g.q.write_buffer(&g.bufs[3], 0, bytemuck::cast_slice(&clut));
        g.plane(CUR, &self.cur.px);
        g.plane(HELD, &self.held);
        if let Some(d) = self.dcur.take() {
            g.plane(DCUR, &d);
            self.dcur = Some(Vec::new());
        }
        // `trx` in qwords (`gq`): whole qwords so far, a finished transfer at its end.
        let rw = (self.r[0x52] & 0xfff) as u32;
        let ppq = 128 / Self::bpp(((self.r[0x50] >> 56) & 0x3f) as u32);
        self.gq = (self.trx.y * rw + self.trx.x).div_ceil(ppq);
        self.pm = None;
        self.tcache = Vec::new();
        g.lazy = self.lazy;
        self.gpu = Some(g);
        eprintln!("gscap: the GPU capture took over at field {}", self.fields);
    }

    /// End of a job: a published overlay is read back into `done`; a field end submits the field's work.
    pub(in super::super) fn gpu_end_job(&mut self, field_end: bool) {
        let t0 = self.prof.is_some().then(std::time::Instant::now);
        let g = self.gpu.as_mut().unwrap();
        if let Some(mut o) = g.stage.take() {
            g.hd_out = None;
            if g.lazy {
                // [PC] (speed) The readback in flight goes out first (it uses the same buffers); this one is collected by `gpu_finish`.
                self.gpu_finish(true);
                let g = self.gpu.as_mut().unwrap();
                g.submit(Some((2, STAGE as u64 * PLANE)));
                self.pend_o = Some((self.ends_run + 1, o));
                self.prof_add("gpu end job (publish)", t0);
                return;
            }
            let px = g.submit(Some((2, STAGE as u64 * PLANE))).unwrap_or_default();
            o.px = bytemuck::allocation::cast_vec(px);
            o.hd = g.hd_out.take().map(|(w, rows, rgba)| Hd { w, rows, rgba });
            if let Some(h) = &o.hd {
                static SHOWN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
                if SHOWN.swap(h.w << 16 | h.rows, std::sync::atomic::Ordering::Relaxed) != h.w << 16 | h.rows {
                    eprintln!("gscap: sharp HUD {} x {} (frame of {} rows)", h.w, h.rows, o.rows);
                }
            }
            self.done = Some(o);
            self.prof_add("gpu end job (publish)", t0);
        } else if field_end {
            g.submit(None);
            self.prof_add("gpu end job (field end)", t0);
        }
    }

    /// [PC] (speed) Collects the lazy readback into `ready` once the GPU has copied it (`block`: waits for it).
    pub(crate) fn gpu_finish(&mut self, block: bool) {
        if self.pend_o.is_none() {
            return;
        }
        let Some(g) = self.gpu.as_mut() else { return };
        let Some(px) = g.rb_finish(block) else { return };
        let hd = g.hd_out.take();
        let Some((tag, mut o)) = self.pend_o.take() else { return };
        o.px = bytemuck::allocation::cast_vec(px);
        o.hd = hd.map(|(w, rows, rgba)| Hd { w, rows, rgba });
        if let Some(h) = &o.hd {
            static SHOWN: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
            if SHOWN.swap(h.w << 16 | h.rows, std::sync::atomic::Ordering::Relaxed) != h.w << 16 | h.rows {
                eprintln!("gscap: sharp HUD {} x {} (frame of {} rows)", h.w, h.rows, o.rows);
            }
        }
        self.ready.push_back((tag, o));
    }

    /// The tag of the picture still being read back (`ends` must not pass it).
    pub fn pending_tag(&self) -> Option<u64> {
        self.pend_o.as_ref().map(|p| p.0)
    }

    /// Debug / bench: the GPU capture's counters since the last call.
    pub fn gpu_stats(&mut self) -> String {
        self.gpu.as_mut().map(|g| std::mem::take(&mut g.st).iter().map(|(k, v)| format!("{k} {v}")).collect::<Vec<_>>().join(", ")).unwrap_or_default()
    }

    /// `raster` on the GPU: the prim's set-up as `sprite` / `tri` / `line` / the point case, as a record for gpu.wgsl.
    pub(super) fn gpu_raster(&mut self, st: &St, ty: u32, vs: &[Vtx]) {
        let mut r = [0u32; STRIDE];
        let mut g = [0u32; 9];
        let mut pts = [None; 3];
        let mut serial = false;
        let (kind, bb) = match ty {
            6 => {
                let (a, b) = (vs[0], vs[1]);
                let (x0, y0) = (a.x as f32 / 16.0 - st.ofx, a.y as f32 / 16.0 - st.ofy);
                let (x1, y1) = (b.x as f32 / 16.0 - st.ofx, b.y as f32 / 16.0 - st.ofy);
                let (lx, hx) = if x0 <= x1 { (x0, x1) } else { (x1, x0) };
                let (ly, hy) = if y0 <= y1 { (y0, y1) } else { (y1, y0) };
                let (ix0, ix1) = ((lx.ceil() as i32).max(st.sc[0]), (hx.ceil() as i32).min(st.sc[1] + 1));
                let (iy0, iy1) = ((ly.ceil() as i32).max(st.sc[2]), (hy.ceil() as i32).min(st.sc[3] + 1));
                pts[0] = Some(self.px(st, &a));
                pts[1] = Some(self.px(st, &b));
                g[..4].copy_from_slice(&[x0.to_bits(), y0.to_bits(), x1.to_bits(), y1.to_bits()]);
                (2, [ix0, ix1, iy0, iy1])
            }
            3..=5 => {
                let p: [(f32, f32); 3] = std::array::from_fn(|i| (vs[i].x as f32 / 16.0 - st.ofx, vs[i].y as f32 / 16.0 - st.ofy));
                let area = (p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1);
                if area == 0.0 {
                    return;
                }
                for (k, v) in vs[..3].iter().enumerate() {
                    pts[k] = Some(self.px(st, v));
                }
                let minx = p.iter().map(|q| q.0).fold(f32::MAX, f32::min).ceil().max(st.sc[0] as f32) as i32;
                let maxx = p.iter().map(|q| q.0).fold(f32::MIN, f32::max).floor().min(st.sc[1] as f32) as i32;
                let miny = p.iter().map(|q| q.1).fold(f32::MAX, f32::min).ceil().max(st.sc[2] as f32) as i32;
                let maxy = p.iter().map(|q| q.1).fold(f32::MIN, f32::max).floor().min(st.sc[3] as f32) as i32;
                for k in 0..3 {
                    g[k * 2] = p[k].0.to_bits();
                    g[k * 2 + 1] = p[k].1.to_bits();
                }
                g[6] = area.to_bits();
                (3, [minx, maxx.saturating_add(1), miny, maxy.saturating_add(1)])
            }
            1 | 2 => {
                let (a, b) = (vs[0], vs[1]);
                let (x0, y0) = (a.x as f32 / 16.0 - st.ofx, a.y as f32 / 16.0 - st.ofy);
                let (x1, y1) = (b.x as f32 / 16.0 - st.ofx, b.y as f32 / 16.0 - st.ofy);
                let n = (x1 - x0).abs().max((y1 - y0).abs()).ceil().max(1.0) as i32;
                pts[0] = Some(self.px(st, &a));
                pts[1] = Some(self.px(st, &b));
                g[..5].copy_from_slice(&[x0.to_bits(), y0.to_bits(), x1.to_bits(), y1.to_bits(), n as u32]);
                serial = true;
                (1, [st.sc[0], st.sc[1] + 1, st.sc[2], st.sc[3] + 1])
            }
            0 => {
                pts[0] = Some(self.px(st, &vs[0]));
                let (x, y) = (vs[0].x as f32 / 16.0 - st.ofx, vs[0].y as f32 / 16.0 - st.ofy);
                let (x, y) = (x.round() as i32, y.round() as i32);
                (0, [x, x.saturating_add(1), y, y.saturating_add(1)])
            }
            _ => return,
        };
        // Pixels left of / above the origin are never drawn, nor overlay pixels outside it.
        let lim = if st.ov { 512 } else { i32::MAX };
        let bb = [bb[0].max(0), bb[1].min(lim), bb[2].max(0), bb[3].min(lim)];
        if bb[0] >= bb[1] || bb[2] >= bb[3] {
            return;
        }
        if bb[1] > 4096 || bb[3] > 4096 {
            serial = true;
        }
        let why = |gp: &mut Gpu, k: &'static str| *gp.st.entry(k).or_default() += 1;
        r[0] = kind;
        r[2] = if self.to_disp && self.dcur.is_some() { DCUR } else { CUR };
        r[3] = self.gpu.as_ref().unwrap().slot;
        for k in 0..4 {
            r[4 + k] = bb[k] as u32;
        }
        let w2 = |r: &mut [u32; STRIDE], i: usize, v: u64| {
            r[i] = v as u32;
            r[i + 1] = (v >> 32) as u32;
        };
        r[8] = st.at as u32;
        w2(&mut r, 9, st.tex0);
        r[11] = st.tex1 as u32;
        w2(&mut r, 12, st.clamp);
        w2(&mut r, 14, st.alpha);
        r[16] = st.test as u32;
        w2(&mut r, 17, st.frame);
        r[19] = st.fba as u32;
        w2(&mut r, 20, self.r[0x3b]);
        r[22] = self.r[0x3d] as u32;
        r[23] = self.r[0x46] as u32;
        r[24] = self.r[0x49] as u32;
        r[25] = self.clut_off as u32;
        r[26] = st.ov as u32;
        if let Some(z) = &st.zb {
            r[27..33].copy_from_slice(&[z.bp, z.bw, z.z24 as u32, z.msk as u32, z.tst, 1]);
        }
        r[33..42].copy_from_slice(&g);
        // The sharp HUD (lane G): the prim's box in sharp pixels (its float geometry and scissor, not the 512 picture's pixel box),
        // and its scissor for gpu.wgsl `k_hd` (r[75..79], f32 in GS pixels).
        let mut hd = None;
        // Stage 3: a prim into the twin target is drawn again into plane `TWIN`; an overlay prim textured from the twin samples that plane
        // (record word 79, gpu.wgsl `hd_shade`).
        let clamp = self.r[0x46] & 1 != 0;
        let twin = !st.ov && self.gpu.as_mut().unwrap().twin_prim(st, kind, bb, clamp);
        if st.ov && st.at & 0x10 != 0 {
            let gp = self.gpu.as_mut().unwrap();
            let t = st.tex0;
            let (tbp, tbw, tpsm) = ((t & 0x3fff) as u32, ((t >> 14) & 0x3f) as u32, ((t >> 20) & 0x3f) as u32);
            let (tw, th) = (1u32 << ((t >> 26) & 0xf).min(10), 1u32 << ((t >> 30) & 0xf).min(10));
            if tpsm == 0 && tbw == 8 && tbp % 32 == 0 && gp.hd.0 != 0 && gp.hd_rb.is_some() {
                let fb = tbp / 32;
                if gp.twin != Some(fb) && gp.offs.contains(&fb) {
                    gp.twin = Some(fb);
                    gp.twin_ok = false;
                }
                if gp.twin == Some(fb) && gp.twin_ok && tw == 512 && th <= gp.twin_rows {
                    r[79] = 1;
                    *gp.st.entry("twin composites").or_default() += 1;
                } else if gp.twin == Some(fb) {
                    *gp.st.entry("twin composites without a twin").or_default() += 1;
                }
            }
        }
        if st.ov || twin {
            let gp = self.gpu.as_ref().unwrap();
            if gp.hd.0 != 0 && gp.hd_rb.is_some() && (twin || gp.hd_ok[r[2] as usize]) {
                let f = |i: usize| f32::from_bits(g[i]);
                let (lx, hx, ly, hy) = match kind {
                    0 => (bb[0] as f32, bb[1] as f32, bb[2] as f32, bb[3] as f32),
                    2 => (f(0).min(f(2)), f(0).max(f(2)), f(1).min(f(3)), f(1).max(f(3))),
                    3 => (f(0).min(f(2)).min(f(4)), f(0).max(f(2)).max(f(4)), f(1).min(f(3)).min(f(5)), f(1).max(f(3)).max(f(5))),
                    _ => (f(0).min(f(2)) - 1.0, f(0).max(f(2)) + 2.0, f(1).min(f(3)) - 1.0, f(1).max(f(3)) + 2.0),
                };
                let sc = [st.sc[0].max(0) as f32, (st.sc[1] + 1).min(512) as f32, st.sc[2].max(0) as f32, (st.sc[3] + 1).min(512) as f32];
                let (a0, a1, b0, b1) = (lx.max(sc[0]), hx.min(sc[1]), ly.max(sc[2]), hy.min(sc[3]));
                if a0 < a1 && b0 < b1 {
                    let (kx, ky) = (gp.hd.0 as f32 / 256.0, gp.hd.1 as f32 / 256.0);
                    // A prim that reaches into the picture's last column or row owns every sharp pixel of that 512 cell (gpu.wgsl `k_hd`
                    // samples the border cells where the 512 picture does): its box runs to the picture's end, so the last tile is listed.
                    let xe = if a1 > 511.0 { i32::MAX } else { (a1 * kx + 0.5).ceil() as i32 };
                    let ye = if b1 > gp.hd.2 as f32 - 1.0 { i32::MAX } else { (b1 * ky + 0.5).ceil() as i32 };
                    hd = Some([(a0 * kx - 0.5).floor() as i32, xe, (b0 * ky - 0.5).floor() as i32, ye]);
                    for (k, v) in sc.iter().enumerate() {
                        r[75 + k] = v.to_bits();
                    }
                    if twin {
                        r[2] = TWIN;
                    }
                }
            }
        }
        for (k, p) in pts.iter().enumerate() {
            if let Some(p) = p {
                let b = 42 + k * 11;
                r[b..b + 11].copy_from_slice(&[p.rgba[0].to_bits(), p.rgba[1].to_bits(), p.rgba[2].to_bits(), p.rgba[3].to_bits(), p.s.to_bits(), p.t.to_bits(), p.q.to_bits(), p.u.to_bits(), p.v.to_bits(), p.f.to_bits(), p.z.to_bits()]);
            }
        }
        // What the prim samples and writes, for the batch rules.
        let gp = self.gpu.as_mut().unwrap();
        let sp_r = &mut self.sp_r;
        let ts = (st.at & 0x10 != 0).then(|| {
            let tex0 = st.tex0;
            if let Some((k, v)) = *sp_r {
                if k == (tex0, st.clamp) {
                    return v;
                }
            }
            let (tw, th) = (1u32 << ((tex0 >> 26) & 0xf).min(10), 1u32 << ((tex0 >> 30) & 0xf).min(10));
            let cl = st.clamp;
            // The texel coordinates `wrapc` (gpu.wgsl) can give, first and last: region clamp keeps them in
            // [min(MIN, MAX), MAX], region repeat in [MAX, MIN | MAX] (a 1024-wide guess ran past the end of local memory
            // and claimed all of it, so the map's icons near the top of memory went serial).
            let reach = |m: u64, size: u32, lo: u32, hi: u32| match m & 3 {
                2 => (lo.min(hi), hi),
                3 => (hi, lo | hi),
                _ => (0, size - 1),
            };
            let (x0, x1) = reach(cl, tw, ((cl >> 4) & 0x3ff) as u32, ((cl >> 14) & 0x3ff) as u32);
            let (y0, y1) = reach(cl >> 2, th, ((cl >> 24) & 0x3ff) as u32, ((cl >> 34) & 0x3ff) as u32);
            let v = gp.span((((tex0 >> 20) & 0x3f) as u32, (tex0 & 0x3fff) as u32, ((tex0 >> 14) & 0x3f) as u32, x0, y0, x1 - x0 + 1, y1 - y0 + 1));
            *sp_r = Some(((tex0, st.clamp), v));
            v
        });
        // [PC] (speed) at most two written ranges (the frame's, the Z buffer's): on the stack, not a Vec per prim.
        let mut wa = [(0u32, 0u32, 0u64); 2];
        let mut nw = 0;
        if !st.ov {
            let frame = st.frame;
            let (fbp, fbw, psm) = (((frame & 0x1ff) * 32) as u32, ((frame >> 16) & 0x3f) as u32, ((frame >> 24) & 0x3f) as u32);
            let (x1, y1) = (bb[1] as u32, bb[3] as u32);
            if matches!(psm, 0x00 | 0x01 | 0x02 | 0x13 | 0x14 | 0x1b | 0x24 | 0x2c) {
                let f = gp.span((psm, fbp, fbw, bb[0] as u32, bb[2] as u32, x1 - bb[0] as u32, y1 - bb[2] as u32));
                wa[nw] = (f.0, f.1, (st.frame as u32 & 0x3f3f_01ff) as u64);
                nw += 1;
                serial |= !alias_free(psm, fbp, fbw, x1, y1);
            }
            if let Some(z) = &st.zb {
                // The Z buffer's pages (its blocks are in the Z order, inside the same pages as CT32's).
                let p0 = (bb[2] as u32 / 32) * z.bw + bb[0] as u32 / 64;
                let p1 = ((y1 - 1) / 32) * z.bw + (x1 - 1) / 64;
                let zs = (z.bp + p0 * 32, z.bp + (p1 + 1) * 32);
                serial |= !alias_free(0, z.bp, z.bw, x1, y1) || wa[..nw].iter().any(|&(l, h, _)| hit((l, h), zs));
                wa[nw] = (zs.0, zs.1, 1 << 40 | (z.bp | z.bw << 16 | (z.z24 as u32) << 24) as u64);
                nw += 1;
            }
            if let Some(t) = ts {
                serial |= wa[..nw].iter().any(|&(l, h, _)| hit((l, h), t));
            }
            if twin {
                // A prim that samples its own target cannot be drawn into the twin (the 512 path reads the part drawn before it).
                if ts.is_some_and(|t| wa[..nw].iter().any(|&(l, h, _)| hit((l, h), t))) || hd.is_none() {
                    *gp.st.entry("twin dropped: samples itself or no box").or_default() += 1;
                    gp.twin_ok = false;
                    hd = None;
                    r[2] = CUR;
                }
            } else {
                for &(l, h, _) in &wa[..nw] {
                    gp.twin_touch((l, h));
                }
            }
        }
        let ws = &wa[..nw];
        if serial {
            why(gp, if kind == 1 { "serial: line" } else if ts.is_some_and(|t| ws.iter().any(|&(l, h, _)| hit((l, h), t))) { "serial: samples its target" } else { "serial: other" });
            *gp.st.entry("serial pixels").or_default() += ((bb[1] - bb[0]) as u64) * ((bb[3] - bb[2]) as u64);        }
        gp.prim(&r, bb, ts, ws, serial, hd);
    }
}

/// Overlay metadata of a publish (`Gs::publish` / `field_end` fill it in; the pixels come from the GPU).
pub(crate) fn meta(w: u32, h: u32, rows: u32, frozen: bool, disp: bool) -> Overlay {
    Overlay { w, h, rows: rows.min(disp_rows()), frozen, disp, px: Vec::new(), rgba: Vec::new(), hd: None }
}
