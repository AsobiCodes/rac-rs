//! [PC] RAC_DIAG=1 (the player's diagnostic record, no retail counterpart; Play.cmd sets it): every session writes
//! `<player dir>/diag/<yyyy-mm-dd_hhmmss>/` (the newest 10 stay, 500 MB at most together; each file stops at 50 MB, 50 mark pictures at most: session.rs):
//! - `system.txt`: exe build id, PC, GPU + driver + backend, display, window / render size, upscaler, frame generation, env
//!   switches, pc_options.txt, mods, the optional DLLs beside the exe (system.rs); option changes appended with their time.
//! - `perf.csv`: one `S` row per second (fps, 1% low of the last 10 s, worst frame, the 13 CPU phases of viewer/profile.rs, the GPU time per pass
//!   of viewer/passtime.rs, the cost counters, causes, the game's facts through `Pack::diag_facts`), `M` rows for the mark key.
//! - `spikes.txt`: every frame over max(2x the last non-load second's median, 25 ms) with its CPU breakdown, the causes seen in it
//!   (load step, pipeline build, texture upload, swapchain, screenshot, options save) and its second's row.
//! - `mark_N.png`: F9 (`MARK_KEY`) writes an M row and saves the picture.
//!
//! Reuses the frame profiler (profile.rs runs without printing while this is on) and the pass timer (passtime.rs keeps a second
//! accumulator for `gpu_take`). The render thread only aggregates; the text goes to a writer thread (session.rs). Off: every hook
//! is one `OnceLock` read. Game-neutral: game facts only through `Pack::diag_facts` (default: none).

mod session;
pub mod stats;
mod system;

pub use stats::Facts;
use stats::{GpuSecond, Rolling, Second, PHASES};
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

/// Cause bits (`cause`; stats.rs `CAUSE_NAMES`).
pub const LOAD: u32 = 1;
pub const PIPELINE: u32 = 2;
pub const TEXTURE: u32 = 4;
pub const SWAPCHAIN: u32 = 8;
pub const SCREENSHOT: u32 = 16;
pub const OPTIONS: u32 = 32;

/// Cost counters (`count`; stats.rs `COUNT_NAMES`).
pub const LIGHTS: usize = 0;
pub const FIRES: usize = 1;
pub const FIRE_MAPS: usize = 2;
pub const PARTICLES: usize = 3;
pub const SMOKE_FIRES: usize = 4;

/// The mark key: unused by the game's default binds, the debug keys and the dev tools (RAC_SMOOTH_KEY test launchers also use it).
pub const MARK_KEY: winit::keyboard::KeyCode = winit::keyboard::KeyCode::F9;

/// Full spike records per second (the rest are counted only).
const SPIKES_PER_SECOND: u32 = 5;

/// RAC_DIAG=1.
pub fn on() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_DIAG").is_ok_and(|v| v == "1"))
}

static PENDING: AtomicU32 = AtomicU32::new(0);
static COUNTS: [AtomicU32; 5] = [const { AtomicU32::new(0) }; 5];
/// Frame generation's swapchain present count (generated frames included; fgen ffx_fg.rs / sl.rs), 0 = none yet.
static PRESENTS: AtomicU64 = AtomicU64::new(0);
static FG_NOTED: AtomicBool = AtomicBool::new(false);
static MARK: AtomicBool = AtomicBool::new(false);
static SHOT: AtomicBool = AtomicBool::new(false);
/// Text facts set from other places: window size, render size.
static SIZES: Mutex<(String, String)> = Mutex::new((String::new(), String::new()));
/// The options text last written (`options_saved` diffs against it).
static OPTS: Mutex<String> = Mutex::new(String::new());

/// The session (folder + writer), made at the first use while on.
fn session() -> Option<&'static session::Session> {
	static S: OnceLock<Option<session::Session>> = OnceLock::new();
	if !on() {
		return None;
	}
	S.get_or_init(|| {
		let stamp = crate::runlog::local_time_text();
		let s = session::open(&crate::pcopts::player_dir().join("diag"), &stamp)?;
		let opts = std::fs::read_to_string(crate::pcopts::file()).unwrap_or_default();
		let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.to_path_buf()));
		s.append("system.txt", system::start_text(&stamp, &opts, exe_dir.as_deref()));
		s.append("perf.csv", stats::header() + "\n");
		*OPTS.lock().unwrap_or_else(|e| e.into_inner()) = opts;
		eprintln!("diag: recording to {}", s.dir.display());
		Some(s)
	})
	.as_ref()
}

/// Starts the session (start-up, before the first frame; later hooks would make it anyway).
pub fn init() {
	let _ = session();
}

/// 64-bit FNV-1a (the exe's build id).
pub fn fnv64(b: &[u8]) -> u64 {
	b.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &x| (h ^ x as u64).wrapping_mul(0x0100_0000_01b3))
}

/// Something that can cost a frame happened (any thread): the frame that ends next carries it.
#[inline]
pub fn cause(bit: u32) {
	if on() {
		PENDING.fetch_or(bit, Relaxed);
	}
}

/// This frame's value of cost counter `k` (the row keeps the second's largest).
#[inline]
pub fn count(k: usize, n: usize) {
	if on() {
		COUNTS[k].fetch_max(n as u32, Relaxed);
	}
}

/// A key press of the game window (host): the mark key asks for an M row and a picture.
pub fn key(k: winit::keyboard::KeyCode, pressed: bool, repeat: bool) {
	if on() && pressed && !repeat && k == MARK_KEY {
		MARKS.fetch_add(1, Relaxed);
		MARK.store(true, Relaxed);
		SHOT.store(true, Relaxed);
	}
}

/// Whether the host should capture this frame's picture for a mark (once per press).
pub fn shot_due() -> bool {
	on() && SHOT.swap(false, Relaxed)
}

/// The mark's picture (host, after `shot_due`).
pub fn shot(img: Option<crate::render::Image>) {
	let (Some(s), Some(img)) = (session(), img) else { return };
	cause(SCREENSHOT);
	s.send(session::Msg::Png(format!("mark_{}.png", MARKS.load(Relaxed)), img));
}

static MARKS: AtomicU32 = AtomicU32::new(0);

/// The options file was written (pcopts.rs `store_via`): its changes go into system.txt with the time.
pub fn options_saved(text: &str) {
	let Some(s) = session() else { return };
	cause(OPTIONS);
	let mut old = OPTS.lock().unwrap_or_else(|e| e.into_inner());
	let d = stats::options_diff(&old, text);
	if !d.is_empty() {
		s.append("system.txt", format!("\n[options changed at t={:.1} s, {}]\n{}", t_now(), crate::runlog::local_time_text(), d.iter().map(|l| format!("  {l}\n")).collect::<String>()));
	}
	*old = text.to_string();
}

/// The window's device is made (viewer/gpu/setup.rs): the GPU, display and picture block of system.txt.
pub fn note_gpu(info: &wgpu::AdapterInfo, present: &str, latency: u32, window: (u32, u32), render: (u32, u32), monitor: Option<winit::monitor::MonitorHandle>) {
	let Some(s) = session() else { return };
	note_sizes(window, render);
	let mon = monitor.map_or("unknown".to_string(), |m| {
		let sz = m.size();
		format!("{} {}x{} @ {:.2} Hz", m.name().unwrap_or_default(), sz.width, sz.height, m.refresh_rate_millihertz().unwrap_or(0) as f32 / 1000.0)
	});
	let up = crate::viewer::upscale_choice::KINDS.get(crate::viewer::upscale_choice::kind() as usize).copied().unwrap_or("?");
	let q = crate::viewer::upscale_choice::QUALITIES.get(crate::viewer::upscale_choice::quality() as usize).copied().unwrap_or("?");
	s.append("system.txt", format!(
		"\n[gpu, at t={:.1} s]\n  {} ({:?}, vendor 0x{:04x} device 0x{:04x}, {:?})\n  driver: {} {}\n  present mode {present}, frame latency {latency}\n  display: {mon}\n  window {}x{}, render {}x{} ({} %)\n  upscaler: {up}, quality {q}\n",
		t_now(), info.name, info.backend, info.vendor, info.device, info.device_type, info.driver, info.driver_info, window.0, window.1, render.0, render.1, (crate::pack::get().render_scale_pct)()
	));
}

/// The frame generation block of system.txt, written with the first perf.csv row (the files probe has run by then; at
/// `note_gpu` time every kind still reads as offered).
fn fg_text() -> String {
	let fg = crate::viewer::fgen_choice::KINDS;
	let offered: Vec<&str> = (1..fg.len() as i32).filter(|&v| crate::viewer::fgen_choice::offered(v)).map(|v| fg[v as usize]).collect();
	let name = |v: u8| fg.get(v as usize).copied().unwrap_or("?");
	format!("\n[frame generation, at t={:.1} s]\n  chosen {}, running {}, offered: {}\n", t_now(), name(crate::viewer::fgen_choice::chosen()), name(crate::viewer::fgen_choice::started()), if offered.is_empty() { "none".to_string() } else { offered.join(" ") })
}

/// Frame generation presented: `total` = its running present count (rendered + generated frames).
pub fn presents(total: u64) {
	if on() {
		PRESENTS.store(total, Relaxed);
	}
}

/// The window or render size changed (resize, render scale).
pub fn note_sizes(window: (u32, u32), render: (u32, u32)) {
	if on() {
		*SIZES.lock().unwrap_or_else(|e| e.into_inner()) = (format!("{}x{}", window.0, window.1), format!("{}x{}", render.0, render.1));
	}
}

fn t0() -> Instant {
	static T: OnceLock<Instant> = OnceLock::new();
	*T.get_or_init(Instant::now)
}

fn t_now() -> f32 {
	t0().elapsed().as_secs_f32()
}

/// The render thread's running second.
struct Agg {
	start: Instant,
	last: Option<Instant>,
	sec: Second,
	/// The last second's median interval (the spike rule).
	median: f32,
	/// Spike texts of this second (written with its row).
	spikes: String,
	/// PRESENTS at the last S row.
	presents: u64,
	/// The last seconds' intervals (the rolling 1% low).
	roll: Rolling,
}

thread_local! {
	static AGG: RefCell<Option<Agg>> = const { RefCell::new(None) };
}

/// One display frame ended (viewer/profile.rs `frame_end`, render thread): the redraw's phase times (ms) and its spans.
pub fn frame(phase_names: &[&'static str; PHASES], phase_ms: &[f32; PHASES], redraw_ms: f32, ticks: u32, spans: &[(&'static str, std::time::Duration, u32)]) {
	let Some(s) = session() else { return };
	let now = Instant::now();
	let causes = PENDING.swap(0, Relaxed);
	AGG.with(|a| {
		let mut b = a.borrow_mut();
		let a = b.get_or_insert_with(|| Agg { start: now, last: None, sec: Second::default(), median: 0.0, spikes: String::new(), presents: 0, roll: Rolling::default() });
		let interval = a.last.map(|l| (now - l).as_secs_f32() * 1000.0);
		a.last = Some(now);
		a.sec.add(interval, ticks, phase_ms, causes);
		if let Some(i) = interval.filter(|&i| i > stats::spike_limit(a.median)) {
			a.sec.spikes += 1;
			if a.sec.spikes <= SPIKES_PER_SECOND {
				a.spikes += &spike_text(i, a.median, redraw_ms, ticks, phase_names, phase_ms, spans, causes);
			}
		}
		if MARK.swap(false, Relaxed) {
			let k = MARKS.load(Relaxed);
			let secs = (now - a.start).as_secs_f32();
			let (f, gpu) = (facts(), GpuSecond::default());
			let r = row_text(&a.sec, "M", secs, &f, &gpu, None, a.roll.low1(&a.sec.intervals));
			s.append("perf.csv", r.clone() + "\n");
			s.append("marks.txt", format!("MARK {k} at t={:.2} s ({}): picture mark_{k}.png\n  on screen: {}\n  row so far: {r}\n", t_now(), crate::runlog::local_time_text(), facts_text(&f)));
		}
		let secs = (now - a.start).as_secs_f32();
		if secs >= 1.0 {
			let f = facts();
			let gpu = crate::viewer::passtime::diag_take().unwrap_or_default();
			let p = PRESENTS.load(Relaxed);
			// a new swapchain (FSR <-> DLSS) restarts the count: that second reads nothing
			let presented = (p > a.presents && a.presents > 0).then(|| (p - a.presents) as f32 / secs);
			a.presents = p;
			let r = row_text(&a.sec, "S", secs, &f, &gpu, presented, a.roll.low1(&a.sec.intervals));
			a.roll.push(&a.sec.intervals);
			if !FG_NOTED.swap(true, Relaxed) {
				s.append("system.txt", fg_text());
			}
			a.median = stats::next_median(a.median, a.sec.rates(secs).4, a.sec.causes);
			s.append("perf.csv", r.clone() + "\n");
			if !a.spikes.is_empty() {
				let extra = a.sec.spikes.saturating_sub(SPIKES_PER_SECOND);
				let more = if extra > 0 { format!("  (+{extra} more spikes this second)\n") } else { String::new() };
				s.append("spikes.txt", format!("{}{more}  second: {r}\n\n", std::mem::take(&mut a.spikes)));
			}
			a.sec = Second::default();
			a.start = now;
		}
	});
}

fn facts() -> Facts {
	(crate::pack::get().diag_facts)()
}

fn facts_text(f: &Facts) -> String {
	let hero = f.hero.map_or("none".to_string(), |h| format!("{:.1} {:.1} {:.1}", h[0], h[1], h[2]));
	format!("level {} ({}), {}, hero at {hero}, camera {}", if f.level.is_empty() { "?" } else { f.level }, f.level_id, stats::state(f, 0), if f.camera.is_empty() { "?" } else { f.camera })
}

/// A perf.csv row of second `sec` (counters taken and reset for an S row).
fn row_text(sec: &Second, kind: &str, secs: f32, f: &Facts, gpu: &GpuSecond, presented: Option<f32>, low1: f32) -> String {
	let counts: [u32; 5] = std::array::from_fn(|k| if kind == "S" { COUNTS[k].swap(0, Relaxed) } else { COUNTS[k].load(Relaxed) });
	let (w, r) = SIZES.lock().map(|s| s.clone()).unwrap_or_default();
	let up = crate::viewer::upscale_choice::KINDS.get(crate::viewer::upscale_choice::kind() as usize).copied().unwrap_or("?");
	let fg = crate::viewer::fgen_choice::KINDS.get(crate::viewer::fgen_choice::started() as usize).copied().unwrap_or("?");
	stats::row(sec, &stats::RowIn { kind, t: t_now(), secs, facts: f, gpu, counts, window: &w, render: &r, upscaler: up, fg, presented, low1 })
}

#[allow(clippy::too_many_arguments)]
fn spike_text(interval: f32, median: f32, redraw_ms: f32, ticks: u32, names: &[&'static str; PHASES], ph: &[f32; PHASES], spans: &[(&'static str, std::time::Duration, u32)], causes: u32) -> String {
	use std::fmt::Write;
	let mut s = format!("SPIKE t={:.3} s: {interval:.1} ms frame (median {median:.1} ms), redraw {redraw_ms:.1} ms, {ticks} game ticks, causes: {}\n  cpu:", t_now(), if causes == 0 { "none seen".to_string() } else { stats::cause_text(causes) });
	for (n, v) in names.iter().zip(ph) {
		if *v >= 0.05 {
			let _ = write!(s, " {n} {v:.2}");
		}
	}
	let mut sp: Vec<&(&'static str, std::time::Duration, u32)> = spans.iter().collect();
	sp.sort_by(|a, b| b.1.cmp(&a.1));
	s += "\n  spans:";
	for (n, d, c) in sp.iter().take(12) {
		let _ = write!(s, " {n} {:.2}ms x{c}", d.as_secs_f32() * 1000.0);
	}
	s += "\n";
	s
}

/// UTC "YYYY-MM-DD HH:MM:SS" now (runlog.rs `local_time_text` off Windows).
pub fn utc_now() -> String {
	session::utc_stamp(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs()))
}
