//! [PC] Background build of the level pipelines and their disk cache (lane pwarm). The fire-shadow sets of ~16 `metalpipe::Pipes` are
//! ~45 big pipelines; built one after another on the main thread at the first frame with fire lights they froze the window ~19 s on
//! NVIDIA (the driver's shader compiler, ~0.4 s each). `Pipes::prewarm` queues them here instead: a small pool builds them in parallel
//! (`OnceLock` in `Pipes` makes a main-thread `get` wait for a set under construction, never build it twice). On Vulkan the pipelines
//! also go through a `PipelineCache` file of the player folder (`gpu_level_<key>.bin`, pipecache.rs; the key holds the adapter, driver
//! and every level shader source) that is written when the pool drains and now and then (`tick`), so a second start compiles nothing.
//! RAC_PIPECACHE=0 = no file.
//! The pictures are unchanged (the same pipelines, only built earlier / in parallel / from the cache), except that a fire's shadow can
//! start a moment late while the pool is still building (metalpipe.rs `Gpu::fls_sets`; never with the test clock).

use crate::pipecache::PipeCache;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

type Job = Box<dyn FnOnce() + Send>;

static CACHE: Mutex<Option<Arc<PipeCache>>> = Mutex::new(None);

/// The level pipeline cache of the window device (None: DX12, no PIPELINE_CACHE feature, or RAC_PIPECACHE=0).
pub(crate) fn cache() -> Option<Arc<PipeCache>> {
	CACHE.lock().ok()?.clone()
}

/// Opens the cache file for the window device (gpu/setup.rs, once per `Gpu::new`).
pub(crate) fn init(device: &wgpu::Device, info: &wgpu::AdapterInfo) {
	let on = info.backend == wgpu::Backend::Vulkan && device.features().contains(wgpu::Features::PIPELINE_CACHE) && std::env::var("RAC_PIPECACHE").map_or(true, |v| v != "0");
	let c = on.then(|| {
		let (soft0, soft1) = (super::volfog::soft::level_source(super::SHADER, false), super::volfog::soft::level_source(super::SHADER, true));
		let bindless = super::bindless::shader_source(&soft0);
		let (veg, fur) = (super::vegfx::pipes::source_with(false), super::furfx::pipes::source_with(false));
		// Lane fshade (review W4 finding 5): the fire modules' edits and whether they take the subgroup twin (flitquad.rs; its
		// `device_created` runs before this).
		let fire = super::flitquad::fire_text(&soft1, super::flitquad::on()).map_or(String::new(), |t| t.0);
		// [PC] lane noaa: a 1-sample run (scene_msaa.rs) keys its own file; the 4x key stays as before.
		let mut key: Vec<&str> = vec![super::SHADER, &soft0, &soft1, &bindless, &veg, &fur, &fire];
		if super::scene_msaa::msaa() != super::scene_msaa::FULL {
			key.push("scene msaa 1");
		}
		Arc::new(PipeCache::open_named(device, info, &key, "gpu_level", "level pipelines"))
	});
	if let Ok(mut g) = CACHE.lock() {
		*g = c;
	}
}

struct Pool {
	tx: Mutex<Sender<Job>>,
	pending: Arc<AtomicUsize>,
	started: Arc<Mutex<Option<Instant>>>,
}

fn pool() -> &'static Pool {
	static POOL: OnceLock<Pool> = OnceLock::new();
	POOL.get_or_init(|| {
		let workers = (std::thread::available_parallelism().map_or(4, |n| n.get()) / 2).clamp(2, 8);
		let (tx, rx) = channel::<Job>();
		let rx = Arc::new(Mutex::new(rx));
		let (pending, started, total) = (Arc::new(AtomicUsize::new(0)), Arc::new(Mutex::new(None::<Instant>)), Arc::new(AtomicUsize::new(0)));
		for i in 0..workers {
			let (rx, pending, started, total) = (rx.clone(), pending.clone(), started.clone(), total.clone());
			let _ = std::thread::Builder::new().name(format!("pipewarm{i}")).spawn(move || loop {
				let job = match rx.lock() {
					Ok(r) => r.recv(),
					Err(_) => return,
				};
				let Ok(job) = job else { return };
				let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job));
				total.fetch_add(1, Ordering::Relaxed);
				if pending.fetch_sub(1, Ordering::AcqRel) == 1 {
					let ms = started.lock().ok().and_then(|mut s| s.take()).map_or(0, |t| t.elapsed().as_millis());
					if let Some(c) = cache() {
						c.save();
					}
					eprintln!("pipewarm: {} pipeline sets built in {ms} ms on {workers} threads", total.swap(0, Ordering::Relaxed));
				}
			});
		}
		Pool { tx: Mutex::new(tx), pending, started }
	})
}

/// Queues a build job.
pub(crate) fn submit(job: Job) {
	let p = pool();
	if p.pending.fetch_add(1, Ordering::AcqRel) == 0 {
		if let Ok(mut s) = p.started.lock() {
			*s = Some(Instant::now());
		}
	}
	if let Ok(tx) = p.tx.lock() {
		let _ = tx.send(job);
	}
}

/// No queued build is waiting or running (metalpipe.rs `Gpu::fls_sets`: a set still missing then is built on the caller's thread).
pub(crate) fn idle() -> bool {
	pool().pending.load(Ordering::Acquire) == 0
}

/// Once per rendered frame (metalpipe.rs `Gpu::fls_sets`). Test build 7 (review of the load freeze): the pool writes the cache file only
/// when it drains, so the pipelines the render thread builds itself (the first frame of a level, a level without fire lights) were
/// never kept. At frame 600 and then every 3600 frames a short thread writes it (pipecache.rs `save`: only when the data grew).
pub(crate) fn tick() {
	static FRAMES: AtomicUsize = AtomicUsize::new(0);
	let n = FRAMES.fetch_add(1, Ordering::Relaxed) + 1;
	if n % 3600 != 600 {
		return;
	}
	if let Some(c) = cache() {
		let _ = std::thread::Builder::new().name("pipecache-save".into()).spawn(move || c.save());
	}
}
