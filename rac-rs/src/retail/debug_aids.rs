//! Dev and test aids: the per-100-fields speed lines (`Smoke`, `WPerf`), the class probe, the debug-shot allowance and
//! the slow-machine imitations of tests.

use super::*;

/// [PC] DEV: RAC_SHOT_SEQ shots (viewer/handler.rs) let the game run one more field past RAC_SHOT_FIELD per saved frame.
pub static SHOT_ALLOW: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

/// RAC_RETAIL_SMOKE=N (tools/smoke.ps1, the speed check before an exe goes out): every 100 fields one `smoke:` line
/// on stderr with the EE thread's `Machine::frame` time per field and the wall-clock fields/s of the worker; after N
/// fields the RAM hash (movetest's trail hash, `rec::hash_ram`) and the process exits. Off: one branch per field.
pub(super) struct Smoke {
	pub(super) end: u32,
	pub(super) start: std::time::Instant,
	pub(super) lap: std::time::Instant,
	pub(super) ns: u64,
	pub(super) max: u64,
	pub(super) all: u64,
}

impl Smoke {
	pub(super) fn from_env() -> Smoke {
		let end = std::env::var("RAC_RETAIL_SMOKE").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
		let now = std::time::Instant::now();
		Smoke { end, start: now, lap: now, ns: 0, max: 0, all: 0 }
	}

	/// After field `n` (1-based: fields run so far) whose `Machine::frame` took `ns`.
	pub(super) fn field(&mut self, n: u32, ns: u64, ram: &[u8]) {
		if self.end == 0 {
			return;
		}
		(self.ns, self.max, self.all) = (self.ns + ns, self.max.max(ns), self.all + ns);
		if n % 100 == 0 {
			let wall = self.lap.elapsed().as_secs_f64();
			eprintln!("smoke: field {n} ee {:.2} ms/field (max {:.2}) | worker {:.1} fields/s", self.ns as f64 / 1e8, self.max as f64 / 1e6, 100.0 / wall.max(1e-9));
			(self.ns, self.max, self.lap) = (0, 0, std::time::Instant::now());
		}
		if n >= self.end {
			let wall = self.start.elapsed().as_secs_f64();
			eprintln!("smoke: done {n} fields | ee {:.2} ms/field | worker {:.1} fields/s | ram {:016x}", self.all as f64 / 1e6 / n as f64, n as f64 / wall.max(1e-9), rec::hash_ram(ram));
			eprintln!("{}", ee_oracle::hs::stats_line()); // speed hands: deopt report (RAC_HS_STATS=1 adds the top deopt pcs)
			if let Ok(p) = std::env::var("RAC_RETAIL_SMOKE_RAM") {
				// Dev only: the RAM at the end of the smoke run, for diffing two runs.
				let _ = std::fs::write(p, ram);
			}
			crate::gscap_guard::quit();
			std::process::exit(0);
		}
	}
}

/// RAC_PERF_LOG=1 (PC debug aid): the worker thread's field split into sections, one `perf:` line per 100 fields with the
/// average and the worst of each section (ms) and the fields whose busy time passed 20 ms (a 50 Hz field).
/// Sections: wait (for the main thread's pad), pre (pad hooks), ee (Machine::frame), mid (front-end bookkeeping),
/// pace (waits for the GS capture), snap (efx prep + snap + VU1 thread), post (efx build, lag), take (waits for the overlay), tail.
pub(super) struct WPerf {
	pub(super) on: bool,
	pub(super) last: std::time::Instant,
	pub(super) cur: [u64; 9],
	pub(super) sum: [u64; 9],
	pub(super) max: [u64; 9],
	pub(super) over: u32,
	pub(super) n: u32,
	pub(super) worst: (u64, u32),
	/// Not part of the sections: efx prep, snap() alone, the VU1 thread's join wait (ns, summed over the 100 fields).
	ex: [u64; 3],
	lap100: std::time::Instant,
	/// Instructions the EE ran in this field (slow-field print).
	pub(super) icd: u64,
	/// RAC_PC_SAMPLE=1: the EE pc sampled every 200 us (racy read, diagnostics only); printed for a slow field.
	pub(super) samples: Option<std::sync::Arc<std::sync::Mutex<Vec<u32>>>>,
}

const WPERF_NAMES: [&str; 9] = ["wait", "pre", "ee", "mid", "pace", "snap", "post", "take", "tail"];

impl WPerf {
	pub(super) fn from_env() -> WPerf {
		let now = std::time::Instant::now();
		WPerf { on: std::env::var("RAC_PERF_LOG").is_ok_and(|v| v == "1"), last: now, cur: [0; 9], sum: [0; 9], max: [0; 9], over: 0, n: 0, worst: (0, 0), ex: [0; 3], lap100: now, icd: 0, samples: None }
	}

	pub(super) fn sample_pc(&mut self, pc_addr: usize) {
		if !self.on || !std::env::var_os("RAC_PC_SAMPLE").is_some() {
			return;
		}
		let v = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
		self.samples = Some(v.clone());
		std::thread::spawn(move || loop {
			std::thread::sleep(std::time::Duration::from_micros(200));
			let pc = unsafe { std::ptr::read_volatile(pc_addr as *const u32) };
			v.lock().unwrap().push(pc);
		});
	}

	pub(super) fn extra(&mut self, i: usize, ns: u64) {
		self.ex[i] += ns;
	}

	pub(super) fn lap(&mut self, i: usize) {
		if self.on {
			let now = std::time::Instant::now();
			self.cur[i] += now.duration_since(self.last).as_nanos() as u64;
			self.last = now;
		}
	}

	pub(super) fn field(&mut self, field: u32) {
		if !self.on {
			return;
		}
		let busy: u64 = self.cur[1..].iter().sum();
		for i in 0..9 {
			self.sum[i] += self.cur[i];
			self.max[i] = self.max[i].max(self.cur[i]);
		}
		if busy > 20_000_000 {
			self.over += 1;
		}
		if busy > self.worst.0 {
			self.worst = (busy, field);
		}
		if self.cur[2] > 30_000_000 {
			eprintln!("perf: slow field {field} ({} M insns): {}", self.icd / 1_000_000, WPERF_NAMES.iter().zip(self.cur).map(|(n, v)| format!("{n} {:.1}", v as f64 / 1e6)).collect::<Vec<_>>().join(" "));
		}
		if let Some(sm) = &self.samples {
			let mut v = sm.lock().unwrap();
			if self.cur[2] > 30_000_000 || std::env::var_os("RAC_PERF_ALL").is_some() {
				eprintln!("perf: insns last field {} M", self.icd / 1_000_000);
				let mut h: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
				for &pc in v.iter() {
					*h.entry(pc & !0xff).or_default() += 1;
				}
				let mut hv: Vec<_> = h.into_iter().collect();
				hv.sort_by_key(|x| std::cmp::Reverse(x.1));
				eprintln!("perf: pc samples ({}) by 256-byte block: {}", v.len(), hv.iter().take(12).map(|(a, n)| format!("{a:#x}:{n}")).collect::<Vec<_>>().join(" "));
				let mut h2: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
				for &pc in v.iter() {
					*h2.entry(pc).or_default() += 1;
				}
				let mut hv2: Vec<_> = h2.into_iter().collect();
				hv2.sort_by_key(|x| std::cmp::Reverse(x.1));
				eprintln!("perf: top pcs: {}", hv2.iter().take(12).map(|(a, n)| format!("{a:#x}:{n}")).collect::<Vec<_>>().join(" "));
			}
			v.clear();
		}
		self.cur = [0; 9];
		self.n += 1;
		if self.n == 100 {
			let wall = self.lap100.elapsed().as_secs_f64();
			let avg = WPERF_NAMES.iter().zip(self.sum.iter().zip(self.max)).map(|(n, (s, m))| format!("{n} {:.1}/{:.1}", *s as f64 / 1e8, m as f64 / 1e6)).collect::<Vec<_>>().join(" ");
			eprintln!("perf: field {field} {:.1} fields/s | avg/max ms: {avg} | prep {:.1} snap() {:.1} vujoin {:.1} | busy>20ms: {} worst {:.1} ms at {}", 100.0 / wall.max(1e-9), self.ex[0] as f64 / 1e8, self.ex[1] as f64 / 1e8, self.ex[2] as f64 / 1e8, self.over, self.worst.0 as f64 / 1e6, self.worst.1);
			(self.sum, self.max, self.over, self.n, self.worst, self.ex, self.lap100) = ([0; 9], [0; 9], 0, 0, (0, 0), [0; 3], std::time::Instant::now());
		}
	}
}

/// Debug probe `RAC_DBG_CLASS=N`: log the retail moby fields (sync) and the draw inputs (set_props) of class N (read once).
pub fn dbg_class() -> Option<u32> {
	static C: std::sync::OnceLock<Option<u32>> = std::sync::OnceLock::new();
	*C.get_or_init(|| std::env::var("RAC_DBG_CLASS").ok().and_then(|v| v.parse().ok()))
}

/// RAC_TEST_SLOW_EE=F (F > 1, tests only): imitates a CPU F times slower for the game thread (see the worker loop).
pub fn test_slow_ee() -> Option<f64> {
	static V: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
	*V.get_or_init(|| std::env::var("RAC_TEST_SLOW_EE").ok().and_then(|v| v.parse().ok()).filter(|&f: &f64| f > 1.0))
}

/// RAC_TEST_SLOW_RENDER=MS (tests only): every redraw takes MS longer, as a slow GPU would (viewer/handler.rs).
pub fn test_slow_render() -> Option<std::time::Duration> {
	static V: std::sync::OnceLock<Option<f64>> = std::sync::OnceLock::new();
	V.get_or_init(|| std::env::var("RAC_TEST_SLOW_RENDER").ok().and_then(|v| v.parse().ok()).filter(|&m: &f64| m > 0.0)).map(|m| std::time::Duration::from_secs_f64(m / 1000.0))
}
