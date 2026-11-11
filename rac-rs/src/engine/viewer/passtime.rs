//! RAC_GPU_PASSES=1 (PC debug aid, no retail counterpart): GPU time of every render / compute pass of the frame with wgpu timestamp
//! queries. Each pass takes its stamps from `rw(label)` / `cw(label)` instead of `timestamp_writes: None`; the frame's stamps are
//! resolved at the end of the encoder (`frame_end`), read back four frames later (`frame_begin`, so the present thread has submitted
//! them) and summed per label. Every RAC_GPU_PASSES frames (default 120) one table is printed to stderr:
//! `gpupass: N frames | label avg ms (passes per frame) | ... | sum A ms, span B ms`. `span` is the first stamp to the last one of
//! the frame (gaps between passes included), `sum` the added pass times. Off: every call is one `OnceLock` read. RAC_DIAG without
//! RAC_GPU_PASSES (diag-only): per-pass stamps only (no lap stamps, no table), read once a second by `diag_take`.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

/// Stamp pairs per frame (passes beyond it run untimed). 512 (lane dlit2-9): the fire maps' cube faces and the sun cascades
/// can take a frame past 128, so the later passes ran untimed (1024 queries, under wgpu's 4096; 8 KB a readback slot).
const PAIRS: u32 = 512;
/// Readback slots: a frame's reading is mapped `LAG` frames after it was encoded.
const RING: usize = 8;
const LAG: u64 = 4;

#[derive(PartialEq)]
enum Slot {
	Free,
	/// The frame's resolve + copy is encoded; (frame number, pass count, labels).
	Encoded(u64, u32, Vec<&'static str>),
	Mapping(u32, Vec<&'static str>),
}

struct Timer {
	qs: wgpu::QuerySet,
	resolve: Vec<wgpu::Buffer>,
	read: Vec<wgpu::Buffer>,
	period: f64,
	/// The device has TIMESTAMP_QUERY_INSIDE_PASSES (`lap`).
	inside: bool,
	next: AtomicU32,
	labels: Mutex<Vec<&'static str>>,
	state: Mutex<State>,
}

struct State {
	slots: Vec<Slot>,
	ready: Vec<std::sync::Arc<std::sync::atomic::AtomicBool>>,
	frame: u64,
	every: u64,
	frames: u64,
	sum: BTreeMap<&'static str, (f64, u64)>,
	span: f64,
	total: f64,
	/// Idle time between a pass's end stamp and the next pass's start stamp, summed per label of the pass before the gap.
	gaps: BTreeMap<&'static str, f64>,
	/// [PC] lane gspeed: the first stamp of the last frame read, and the summed first-stamp-to-first-stamp time of consecutive frames
	/// (the GPU frame interval: interval - span = GPU idle between frames, e.g. waiting for the CPU).
	prev_lo: u64,
	step: (f64, u64),
	/// RAC_DIAG: the same sums for the diagnostic record, taken once per second (`diag_take`).
	diag: DiagAcc,
}

#[derive(Default)]
struct DiagAcc {
	frames: u64,
	total: f64,
	span: f64,
	sum: BTreeMap<&'static str, f64>,
}

static TIMER: OnceLock<Option<Timer>> = OnceLock::new();

/// `every` of diag-only timing (RAC_DIAG without RAC_GPU_PASSES): no table, no lap stamps, no table sums.
const DIAG_ONLY: u64 = u64::MAX;

/// RAC_GPU_PASSES's table interval; with only RAC_DIAG on, `DIAG_ONLY`.
fn wanted() -> Option<u64> {
	static W: OnceLock<Option<u64>> = OnceLock::new();
	// [PC] RAC_DIAG (diag.rs) times the passes too, without the stderr table.
	*W.get_or_init(|| std::env::var("RAC_GPU_PASSES").ok().map(|v| v.parse().ok().filter(|&n| n > 0).unwrap_or(120)).or_else(|| crate::diag::on().then_some(DIAG_ONLY)))
}

/// The device features the timing needs, as far as the adapter has them: RAC_GPU_PASSES TIMESTAMP_QUERY + _INSIDE_PASSES (the
/// lap stamps), RAC_DIAG alone TIMESTAMP_QUERY only (per-pass times; laps would cost CPU and stamp pairs it never reads); none when off.
pub(super) fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
	super::levelstats::features(adapter) | match wanted() { // [probe] RAC_LEVEL_STATS (levelstats.rs)
		None => wgpu::Features::empty(),
		Some(DIAG_ONLY) => wgpu::Features::TIMESTAMP_QUERY & adapter.features(),
		Some(_) => (wgpu::Features::TIMESTAMP_QUERY | wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES) & adapter.features(),
	}
}

/// After the window's device exists: makes the timer when RAC_GPU_PASSES or RAC_DIAG is set and the device has timestamps.
pub(super) fn init(device: &wgpu::Device, queue: &wgpu::Queue) {
	super::levelstats::init(device); // [probe] RAC_LEVEL_STATS (levelstats.rs)
	let Some(every) = wanted() else { return };
	let t = device.features().contains(wgpu::Features::TIMESTAMP_QUERY).then(|| {
		let bytes = PAIRS as u64 * 16;
		let mk = |label, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: bytes, usage, mapped_at_creation: false });
		Timer {
			qs: device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("pass time"), ty: wgpu::QueryType::Timestamp, count: PAIRS * 2 }),
			resolve: (0..RING).map(|_| mk("pass time resolve", wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC)).collect(),
			read: (0..RING).map(|_| mk("pass time read", wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST)).collect(),
			period: queue.get_timestamp_period() as f64,
			// [PC] lane gspeed: RAC_GPU_LAPS=0 keeps the table but drops the lap stamps (stamps inside a pass can cost GPU time).
			inside: every != DIAG_ONLY && device.features().contains(wgpu::Features::TIMESTAMP_QUERY_INSIDE_PASSES) && std::env::var("RAC_GPU_LAPS").map_or(true, |v| v != "0"),
			next: AtomicU32::new(0),
			labels: Mutex::new(Vec::new()),
			state: Mutex::new(State {
				slots: (0..RING).map(|_| Slot::Free).collect(),
				ready: (0..RING).map(|_| Default::default()).collect(),
				frame: 0,
				every,
				frames: 0,
				sum: BTreeMap::new(),
				span: 0.0,
				total: 0.0,
				gaps: BTreeMap::new(),
				prev_lo: 0,
				step: (0.0, 0),
				diag: DiagAcc::default(),
			}),
		}
	});
	if t.is_none() {
		if every == DIAG_ONLY {
			eprintln!("diag: the adapter has no timestamp queries: perf.csv gets no GPU pass times");
		} else {
			eprintln!("gpupass: RAC_GPU_PASSES is set but the adapter has no timestamp queries");
		}
	}
	let _ = TIMER.set(t);
}

fn timer() -> Option<&'static Timer> {
	TIMER.get().and_then(|t| t.as_ref())
}

/// The next stamp pair for a pass called `label`, or None (timing off, or the frame has more passes than `PAIRS`).
fn pair(label: &str) -> Option<(&'static wgpu::QuerySet, u32)> {
	let t = timer()?;
	let i = t.next.fetch_add(1, Ordering::Relaxed);
	if i >= PAIRS {
		return None;
	}
	t.labels.lock().ok()?.push(intern(label));
	Some((&t.qs, 2 * i))
}

/// The label as a `&'static str` (each distinct text is leaked once; only while timing is on).
fn intern(label: &str) -> &'static str {
	static NAMES: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());
	let mut names = NAMES.lock().unwrap_or_else(|e| e.into_inner());
	if let Some(n) = names.iter().find(|n| **n == label) {
		return n;
	}
	let n: &'static str = Box::leak(label.to_string().into_boxed_str());
	names.push(n);
	n
}

/// [PC] Lane gspeed: a row `label` for GPU work recorded outside wgpu passes (the upscaler's raw command buffer, copies, the
/// present copies): `open` writes the start stamp with an empty compute pass, `close` the end stamp after the work. Returns the
/// pair (None while timing is off: then nothing is recorded). Every `open` that returns Some must be `close`d in the same frame.
pub(crate) fn open(enc: &mut wgpu::CommandEncoder, label: &str) -> Option<u32> {
	let (qs, i) = pair(label)?;
	let tw = wgpu::ComputePassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: None };
	drop(enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("passtime open"), timestamp_writes: Some(tw) }));
	Some(i)
}

/// The end stamp of an `open` row.
pub(crate) fn close(enc: &mut wgpu::CommandEncoder, i: Option<u32>) {
	let (Some(t), Some(i)) = (timer(), i) else { return };
	let tw = wgpu::ComputePassTimestampWrites { query_set: &t.qs, beginning_of_pass_write_index: None, end_of_pass_write_index: Some(i + 1) };
	drop(enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("passtime close"), timestamp_writes: Some(tw) }));
}

/// `timestamp_writes` of a render pass named `label`.
pub(crate) fn rw(label: &str) -> Option<wgpu::RenderPassTimestampWrites<'static>> {
	pair(label).map(|(qs, i)| wgpu::RenderPassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) })
}

/// `timestamp_writes` of a compute pass named `label`.
pub(crate) fn cw(label: &str) -> Option<wgpu::ComputePassTimestampWrites<'static>> {
	pair(label).map(|(qs, i)| wgpu::ComputePassTimestampWrites { query_set: qs, beginning_of_pass_write_index: Some(i), end_of_pass_write_index: Some(i + 1) })
}

/// A lap stamp inside an open render pass (the pass must have been made with `rw`): the table gets a row `lap NAME` with the time
/// from the previous lap of the pass (the first lap, `start`, only opens the clock). Needs TIMESTAMP_QUERY_INSIDE_PASSES.
pub(super) fn lap(pass: &mut wgpu::RenderPass<'_>, name: &str) {
	super::levelstats::lap(pass, name); // [probe] RAC_LEVEL_STATS segments (levelstats.rs)
	let Some(t) = timer().filter(|t| t.inside) else { return };
	let i = t.next.fetch_add(1, Ordering::Relaxed);
	if i >= PAIRS {
		return;
	}
	if let Ok(mut l) = t.labels.lock() {
		l.push(intern(&format!("@{name}")));
	}
	pass.write_timestamp(&t.qs, 2 * i);
	pass.write_timestamp(&t.qs, 2 * i + 1);
}

/// `lap` with a name that is only built while the lap timer is on (the name of a lap per draw run costs nothing otherwise).
pub(super) fn lap_with(pass: &mut wgpu::RenderPass<'_>, name: impl FnOnce() -> String) {
	if timer().is_some_and(|t| t.inside) {
		lap(pass, &name());
	}
}

/// Start of a frame's encoding: forgets the stamps of a frame that was never finished, maps the readings that are old enough and
/// prints the table when due.
pub(super) fn frame_begin(device: &wgpu::Device) {
	super::levelstats::frame_begin(device); // [probe] RAC_LEVEL_STATS (levelstats.rs)
	let Some(t) = timer() else { return };
	t.next.store(0, Ordering::Relaxed);
	if let Ok(mut l) = t.labels.lock() {
		l.clear();
	}
	let Ok(mut st) = t.state.lock() else { return };
	st.frame += 1;
	let frame = st.frame;
	for k in 0..RING {
		match std::mem::replace(&mut st.slots[k], Slot::Free) {
			Slot::Encoded(f, n, labels) if frame >= f + LAG => {
				let ready = st.ready[k].clone();
				ready.store(false, Ordering::Relaxed);
				t.read[k].slice(..).map_async(wgpu::MapMode::Read, move |r| ready.store(r.is_ok(), Ordering::Relaxed));
				st.slots[k] = Slot::Mapping(n, labels);
			}
			s => st.slots[k] = s,
		}
	}
	device.poll(wgpu::PollType::Poll).ok();
	for k in 0..RING {
		if !matches!(st.slots[k], Slot::Mapping(..)) {
			continue;
		}
		let Slot::Mapping(n, labels) = std::mem::replace(&mut st.slots[k], Slot::Free) else { continue };
		if !st.ready[k].load(Ordering::Relaxed) {
			st.slots[k] = Slot::Mapping(n, labels);
			continue;
		}
		{
			let Ok(data) = t.read[k].slice(..).get_mapped_range() else { continue };
			let v: &[u64] = bytemuck::cast_slice(&data);
			take(&mut st, t.period, v, n as usize, &labels);
		}
		t.read[k].unmap();
	}
	if st.frames >= st.every {
		print_table(&mut st);
	}
}

fn take(st: &mut State, period: f64, v: &[u64], n: usize, labels: &[&'static str]) {
	let (mut lo, mut hi, mut sum) = (u64::MAX, 0u64, 0.0);
	let mut rows: Vec<(&'static str, f64)> = Vec::new();
	let mut times: Vec<(u64, u64, &'static str)> = Vec::new();
	let mut prev_lap: Option<u64> = None;
	for (i, &label) in labels.iter().enumerate().take(n) {
		let (a, b) = (v[2 * i], v[2 * i + 1]);
		if let Some(name) = label.strip_prefix('@') {
			// Lap stamps (`lap`): a row per lap, not part of the pass sum, the gaps or the span.
			if name == "start" {
				prev_lap = None;
			}
			if a != 0 {
				if let Some(p) = prev_lap.filter(|&p| a >= p) {
					let e = st.sum.entry(intern(&format!("lap {name}"))).or_insert((0.0, 0));
					e.0 += (a - p) as f64 * period * 1e-6;
					e.1 += 1;
				}
				prev_lap = Some(a);
			}
			continue;
		}
		if a == 0 || b < a {
			continue;
		}
		lo = lo.min(a);
		hi = hi.max(b);
		let ms = (b - a) as f64 * period * 1e-6;
		sum += ms;
		rows.push((label, ms));
		times.push((a, b, label));
	}
	if rows.is_empty() {
		return;
	}
	st.diag.frames += 1;
	st.diag.total += sum;
	st.diag.span += (hi - lo) as f64 * period * 1e-6;
	for &(label, ms) in &rows {
		*st.diag.sum.entry(label).or_insert(0.0) += ms;
	}
	if st.every == DIAG_ONLY {
		return;
	}
	for (label, ms) in rows {
		let e = st.sum.entry(label).or_insert((0.0, 0));
		e.0 += ms;
		e.1 += 1;
	}
	// Gaps: sorted by start stamp, the time from one pass's end to the next pass's start (barriers, resolves, drains), by the pass before it.
	times.sort_by_key(|t| t.0);
	if order_on() {
		*last_order().lock().unwrap_or_else(|e| e.into_inner()) = order_line(&times, period);
	}
	for w in times.windows(2) {
		if w[1].0 > w[0].1 {
			*st.gaps.entry(w[0].2).or_insert(0.0) += (w[1].0 - w[0].1) as f64 * period * 1e-6;
		}
	}
	// Consecutive frames only (frame numbers are not kept: a jump over 100 ms is a skipped or dropped frame).
	let ms = |a: u64, b: u64| (b - a) as f64 * period * 1e-6;
	if st.prev_lo != 0 && lo > st.prev_lo && ms(st.prev_lo, lo) < 100.0 {
		st.step.0 += ms(st.prev_lo, lo);
		st.step.1 += 1;
	}
	st.prev_lo = lo;
	st.frames += 1;
	st.total += sum;
	st.span += (hi - lo) as f64 * period * 1e-6;
}

/// [PC] RAC_DIAG (diag.rs): the GPU frames read since the last call: per-frame averages of the pass sum, the span and each pass
/// (biggest first). None without timestamps or before the first reading.
pub(crate) fn diag_take() -> Option<crate::diag::stats::GpuSecond> {
	let t = timer()?;
	let mut st = t.state.lock().ok()?;
	let d = std::mem::take(&mut st.diag);
	if d.frames == 0 {
		return None;
	}
	let n = d.frames as f64;
	let mut passes: Vec<(&'static str, f64)> = d.sum.iter().map(|(l, ms)| (*l, ms / n)).collect();
	passes.sort_by(|a, b| b.1.total_cmp(&a.1));
	Some(crate::diag::stats::GpuSecond { frames: d.frames, sum: d.total / n, span: d.span / n, passes })
}

fn print_table(st: &mut State) {
	let n = st.frames.max(1) as f64;
	let mut rows: Vec<_> = st.sum.iter().collect();
	rows.sort_by(|a, b| b.1.0.total_cmp(&a.1.0));
	let body: Vec<String> = rows.iter().map(|(l, (ms, c))| format!("{l} {:.3} ({:.1})", ms / n, *c as f64 / n)).collect();
	let step = if st.step.1 > 0 { st.step.0 / st.step.1 as f64 } else { 0.0 };
	eprintln!("gpupass: {} frames | {} | sum {:.3} ms, span {:.3} ms, interval {:.3} ms", st.frames, body.join(" | "), st.total / n, st.span / n, step);
	st.step = (0.0, 0);
	let mut gaps: Vec<_> = st.gaps.iter().collect();
	gaps.sort_by(|a, b| b.1.total_cmp(a.1));
	let gaps: Vec<String> = gaps.iter().map(|(l, ms)| format!("{l} {:.3}", **ms / n)).collect();
	eprintln!("gpugap: after pass | {} | total {:.3} ms", gaps.join(" | "), st.gaps.values().sum::<f64>() / n);
	if order_on() {
		eprintln!("gpuorder: {}", last_order().lock().unwrap_or_else(|e| e.into_inner()));
	}
	st.gaps.clear();
	st.sum.clear();
	st.frames = 0;
	st.total = 0.0;
	st.span = 0.0;
}

/// [PC] lane gspeed: RAC_GPU_ORDER=1 (with RAC_GPU_PASSES) adds a `gpuorder:` line to each table: the last frame's passes in GPU
/// order, `label ms +gap` (gap = idle to the next pass's start stamp, in ms), to see which pass pair a gap sits between.
fn order_on() -> bool {
	static ON: OnceLock<bool> = OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_GPU_ORDER").is_ok_and(|v| v == "1"))
}

fn last_order() -> &'static Mutex<String> {
	static LAST: OnceLock<Mutex<String>> = OnceLock::new();
	LAST.get_or_init(|| Mutex::new(String::new()))
}

/// `times` (start, end, label) sorted by start: "label ms +gap | ...".
fn order_line(times: &[(u64, u64, &'static str)], period: f64) -> String {
	let ms = |a: u64, b: u64| b.saturating_sub(a) as f64 * period * 1e-6;
	let row = |(i, t): (usize, &(u64, u64, &'static str))| {
		let gap = times.get(i + 1).map_or(0.0, |n| ms(t.1, n.0));
		format!("{} {:.3} +{:.3}", t.2, ms(t.0, t.1), gap)
	};
	times.iter().enumerate().map(row).collect::<Vec<_>>().join(" | ")
}

/// End of the frame's encoding (before `finish`): resolves the stamps taken this frame into a readback slot.
pub(super) fn frame_end(enc: &mut wgpu::CommandEncoder) {
	super::levelstats::frame_end(enc); // [probe] RAC_LEVEL_STATS (levelstats.rs)
	let Some(t) = timer() else { return };
	let n = t.next.load(Ordering::Relaxed).min(PAIRS);
	if n == 0 {
		return;
	}
	let labels = t.labels.lock().map(|l| l.clone()).unwrap_or_default();
	let Ok(mut st) = t.state.lock() else { return };
	let Some(k) = st.slots.iter().position(|s| *s == Slot::Free) else { return };
	enc.resolve_query_set(&t.qs, 0..2 * n, &t.resolve[k], 0);
	enc.copy_buffer_to_buffer(&t.resolve[k], 0, &t.read[k], 0, n as u64 * 16);
	let frame = st.frame;
	st.slots[k] = Slot::Encoded(frame, n, labels);
}

#[cfg(test)]
mod tests {
	use super::*;

	fn state() -> State {
		State { slots: Vec::new(), ready: Vec::new(), frame: 0, every: 120, frames: 0, sum: BTreeMap::new(), span: 0.0, total: 0.0, gaps: BTreeMap::new(), prev_lo: 0, step: (0.0, 0), diag: DiagAcc::default() }
	}

	#[test]
	fn take_sums_passes_and_spans_the_frame() {
		let mut st = state();
		// Period 1000 ns per tick: pass "a" 1..3 ms, "b" 4..9 ms, one unwritten pair (0) and one reversed pair are skipped.
		let v = [1000, 3000, 4000, 9000, 0, 0, 7000, 6000];
		take(&mut st, 1000.0, &v, 4, &["a", "b", "c", "d"]);
		assert_eq!(st.frames, 1);
		assert!((st.sum["a"].0 - 2.0).abs() < 1e-9 && (st.sum["b"].0 - 5.0).abs() < 1e-9);
		assert!(!st.sum.contains_key("c") && !st.sum.contains_key("d"));
		assert!(st.diag.frames == 1 && (st.diag.sum["b"] - 5.0).abs() < 1e-9 && (st.diag.span - 8.0).abs() < 1e-9);
		assert!((st.total - 7.0).abs() < 1e-9 && (st.span - 8.0).abs() < 1e-9);
		// An empty frame counts for nothing.
		take(&mut st, 1000.0, &[0, 0], 1, &["a"]);
		assert_eq!(st.frames, 1);
	}

	#[test]
	fn diag_only_fills_just_the_diag_sums() {
		let mut st = State { every: DIAG_ONLY, ..state() };
		take(&mut st, 1000.0, &[1000, 3000, 4000, 9000], 2, &["a", "b"]);
		assert!(st.sum.is_empty() && st.gaps.is_empty() && st.frames == 0);
		assert!(st.diag.frames == 1 && (st.diag.total - 7.0).abs() < 1e-9 && (st.diag.span - 8.0).abs() < 1e-9);
	}

	#[test]
	fn print_table_resets_the_counters() {
		let mut st = state();
		take(&mut st, 1000.0, &[1000, 3000], 1, &["a"]);
		print_table(&mut st);
		assert!(st.sum.is_empty() && st.frames == 0 && st.total == 0.0 && st.span == 0.0);
	}
}
