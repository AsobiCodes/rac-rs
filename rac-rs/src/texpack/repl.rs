//! The replacement textures of one level and the worker threads that load them (lane tex, S2). `level_loaded` makes a `LevelRepl`
//! from the pack hits: per world texture (texture key) up to 4 files (one per original mip level), decoded off the render thread into
//! a `ReplTex` chain. The scene build uploads what is ready and keeps the original for the rest; the rest is swapped in later
//! (`viewer/gpu/repl_upload.rs`). The CPU copies live as long as the `LevelRepl` (level unload drops them, workers stop on their own).

use super::chain::{self, ReplTex};
use super::dds::{self, Image};
use super::index::PackFile;
use super::name::Ext;
use crate::level::Texture;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering::Relaxed};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// Identity of the original texture a replacement was named for: size and XXH3 of its RGBA. Keys are shared with other textures
/// (the warp actors' keys 0..2, ship and gadget textures), so an upload only takes a replacement whose original it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fp {
	pub w: u32,
	pub h: u32,
	pub hash: u64,
}

impl Fp {
	pub fn of(t: &Texture) -> Fp {
		Fp { w: t.w as u32, h: t.h as u32, hash: xxhash_rust::xxh3::xxh3_64(bytemuck::cast_slice(&t.rgba)) }
	}

	/// Cheap test first (size), the hash only when the sizes agree.
	pub fn is(&self, t: &Texture) -> bool {
		self.w == t.w as u32 && self.h == t.h as u32 && *self == Fp::of(t)
	}
}

/// Load state of one replacement.
pub enum Cpu {
	Pending,
	Ready(Arc<ReplTex>),
	Failed,
}

/// One replaced texture: the files, the decoded chain, and the GPU textures made from it ([0] with the authored chain, [1] as a
/// texture without authored levels: `ReplTex::plain`).
pub struct Entry {
	pub fp: Fp,
	/// Levels of the original's chain (1..=4).
	pub n: usize,
	files: [Option<PackFile>; 4],
	pub cpu: Mutex<Cpu>,
	pub gpu: Mutex<[Option<wgpu::Texture>; 2]>,
}

/// What `level_loaded` hands over per texture.
pub struct Job {
	pub key: u32,
	pub fp: Fp,
	pub n: usize,
	pub files: [Option<PackFile>; 4],
}

#[derive(Default)]
struct Stats {
	ready: usize,
	failed: usize,
	over_budget: usize,
	bytes_bc: usize,
	bytes_rgba: usize,
	errors: Vec<String>,
}

pub struct LevelRepl {
	pub level: u32,
	entries: HashMap<u32, Entry>,
	order: Vec<u32>,
	next: AtomicUsize,
	remaining: AtomicUsize,
	/// Counts finished entries; the swap code rescans its pending list when it changed.
	pub done: AtomicUsize,
	bytes: AtomicUsize,
	stats: Mutex<Stats>,
	t0: Instant,
}

/// Bytes of all live replacement chains (every level alive: the prefetch holds two).
static USED: AtomicUsize = AtomicUsize::new(0);

/// Budget for CPU copies of replacement textures in MB (`RAC_TEXPACK_MB`, default 1536: 8 GB machines and the Steam Deck).
fn budget() -> usize {
	std::env::var("RAC_TEXPACK_MB").ok().and_then(|v| v.parse::<usize>().ok()).unwrap_or(1536) << 20
}

pub(super) fn load_file(f: &PackFile) -> Result<Image, String> {
	let bytes = std::fs::read(&f.path).map_err(|e| format!("{}: {e}", f.path.display()))?;
	match f.ext {
		Ext::Dds => dds::load(&bytes),
		Ext::Png => super::png::load(&bytes),
	}
	.map_err(|e| format!("{}: {e}", f.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())))
}

impl LevelRepl {
	pub fn new(level: u32, jobs: Vec<Job>) -> Arc<LevelRepl> {
		let order = jobs.iter().map(|j| j.key).collect::<Vec<_>>();
		let entries = jobs.into_iter().map(|j| (j.key, Entry { fp: j.fp, n: j.n, files: j.files, cpu: Mutex::new(Cpu::Pending), gpu: Mutex::new([None, None]) })).collect::<HashMap<_, _>>();
		Arc::new(LevelRepl { level, remaining: AtomicUsize::new(order.len()), entries, order, next: AtomicUsize::new(0), done: AtomicUsize::new(0), bytes: AtomicUsize::new(0), stats: Mutex::default(), t0: Instant::now() })
	}

	/// Starts `threads` workers; they end when the queue is empty or the level is dropped.
	pub fn start(self: &Arc<Self>, threads: usize) {
		for i in 0..threads.max(1) {
			let weak: Weak<LevelRepl> = Arc::downgrade(self);
			let _ = std::thread::Builder::new().name(format!("texpack-load-{i}")).spawn(move || {
				while let Some(r) = weak.upgrade() {
					let at = r.next.fetch_add(1, Relaxed);
					let Some(&key) = r.order.get(at) else { break };
					r.run(key);
				}
			});
		}
	}

	pub fn entry(&self, key: u32) -> Option<&Entry> {
		self.entries.get(&key)
	}

	pub fn len(&self) -> usize {
		self.order.len()
	}

	pub fn is_empty(&self) -> bool {
		self.order.is_empty()
	}

	/// Keys still being loaded.
	pub fn pending(&self) -> usize {
		self.remaining.load(Relaxed)
	}

	/// Waits until everything is loaded or `max` has passed (the scene build gives the workers a head start; 0 = do not wait).
	pub fn wait(&self, max: Duration) {
		let t = Instant::now();
		while self.pending() > 0 && t.elapsed() < max {
			std::thread::sleep(Duration::from_millis(2));
		}
	}

	fn run(&self, key: u32) {
		let e = &self.entries[&key];
		let res = self.build(e);
		let state = match res {
			Ok(t) => {
				let b = t.bytes();
				if USED.fetch_add(b, Relaxed) + b > budget() {
					USED.fetch_sub(b, Relaxed);
					self.stats.lock().unwrap().over_budget += 1;
					Cpu::Failed
				} else {
					self.bytes.fetch_add(b, Relaxed);
					let mut s = self.stats.lock().unwrap();
					s.ready += 1;
					if t.fmt.compressed() { s.bytes_bc += b } else { s.bytes_rgba += b }
					Cpu::Ready(Arc::new(t))
				}
			}
			Err(m) => {
				let mut s = self.stats.lock().unwrap();
				s.failed += 1;
				if s.errors.len() < 3 {
					s.errors.push(m);
				}
				Cpu::Failed
			}
		};
		*e.cpu.lock().unwrap() = state;
		self.done.fetch_add(1, Relaxed);
		if self.remaining.fetch_sub(1, Relaxed) == 1 {
			let s = self.stats.lock().unwrap();
			eprintln!(
				"texpack: level {}: {} replacements ready ({:.1} MB compressed + {:.1} MB RGBA8), {} failed, {} over the {} MB budget, loaded in {:.0} ms",
				self.level, s.ready, s.bytes_bc as f64 / 1048576.0, s.bytes_rgba as f64 / 1048576.0, s.failed, s.over_budget, budget() >> 20, self.t0.elapsed().as_secs_f64() * 1000.0
			);
			for m in &s.errors {
				eprintln!("texpack: level {}: {m}", self.level);
			}
		}
	}

	fn build(&self, e: &Entry) -> Result<ReplTex, String> {
		let mut imgs: Vec<Option<Image>> = Vec::new();
		for (k, f) in e.files.iter().enumerate() {
			imgs.push(match f {
				Some(f) if k == 0 => Some(load_file(f)?),
				// A level-k file that cannot be read is generated like a missing one.
				Some(f) => load_file(f).ok(),
				None => None,
			});
		}
		chain::assemble(&imgs, e.n)
	}
}

impl Drop for LevelRepl {
	fn drop(&mut self) {
		USED.fetch_sub(self.bytes.load(Relaxed), Relaxed);
	}
}

thread_local! {
	static THREAD_REPL: std::cell::RefCell<Option<Arc<LevelRepl>>> = const { std::cell::RefCell::new(None) };
}

/// `level_loaded` leaves the level's replacements for the scene build that runs next on the same thread (the level job and
/// `build_scene` share a thread in both load paths).
pub fn set_thread_repl(r: Option<Arc<LevelRepl>>) {
	THREAD_REPL.with(|t| *t.borrow_mut() = r);
}

pub fn take_thread_repl() -> Option<Arc<LevelRepl>> {
	THREAD_REPL.with(|t| t.borrow_mut().take())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fingerprint_tells_textures_apart() {
		let a = Texture { w: 2, h: 1, rgba: vec![[1, 2, 3, 4], [5, 6, 7, 8]] };
		let b = Texture { w: 2, h: 1, rgba: vec![[1, 2, 3, 4], [5, 6, 7, 9]] };
		let fp = Fp::of(&a);
		assert!(fp.is(&a) && !fp.is(&b));
	}

	#[test]
	fn workers_load_a_job_and_the_level_drop_frees_the_budget() {
		let dir = std::env::temp_dir().join(format!("texpack_repl_{}", std::process::id()));
		std::fs::create_dir_all(&dir).unwrap();
		let mut blk = [0u8; 16];
		blk[0] = 128;
		blk[1] = 128;
		let f = super::super::dds::tests::dds(4, 4, b"DXT5", [0; 4], 0, &[&blk]);
		let path = dir.join("a.dds");
		std::fs::write(&path, f).unwrap();
		let file = PackFile { path, ext: Ext::Dds, mod_idx: 0 };
		let t = Texture { w: 1, h: 1, rgba: vec![[0; 4]] };
		let before = USED.load(Relaxed);
		let r = LevelRepl::new(7, vec![Job { key: 3, fp: Fp::of(&t), n: 1, files: [Some(file), None, None, None] }]);
		r.start(2);
		r.wait(Duration::from_secs(5));
		assert_eq!(r.pending(), 0);
		assert!(matches!(&*r.entry(3).unwrap().cpu.lock().unwrap(), Cpu::Ready(t) if t.fmt == super::super::dds::Fmt::Bc3 && t.bytes() == 16));
		assert_eq!(USED.load(Relaxed), before + 16);
		drop(r);
		// A worker may still hold its last reference for a moment.
		for _ in 0..500 {
			if USED.load(Relaxed) == before {
				break;
			}
			std::thread::sleep(Duration::from_millis(4));
		}
		assert_eq!(USED.load(Relaxed), before);
		let _ = std::fs::remove_dir_all(&dir);
	}
}
