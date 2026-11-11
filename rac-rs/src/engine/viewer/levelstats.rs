//! RAC_LEVEL_STATS=N (PC debug aid, no retail counterpart; lane fshade): the level pass's overdraw. Pipeline-statistics queries
//! (wgpu PIPELINE_STATISTICS_QUERY: vertex shader invocations, clipper primitives out, fragment shader invocations) cover the level
//! pass from `open` to `close`, cut into segments at the RAC_GPU_PASSES lap points (passtime.rs `lap`: a lap NAME ends the segment
//! NAME). Read back `LAG` frames later; every N frames (default 120) one line goes to stderr:
//! `levelstats: F frames WxH | NAME frag/px (vert k, prim k) | ... | total X frag/px, V k vert`. Fragment invocations are counted
//! after the early depth test where the driver runs it early (so hidden fragments the test rejects are not counted; helper lanes
//! of 2x2 quads may be). Off (unset, or the device lacks the feature): every call is one `OnceLock` read.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// Segments per frame (later laps join the last one).
const SEGS: u32 = 64;
const RING: usize = 8;
const LAG: u64 = 4;
/// u64 values per query: vertex invocations, clipper primitives out, fragment invocations (flag bit order).
const VALS: usize = 3;
const TYPES: wgpu::PipelineStatisticsTypes = wgpu::PipelineStatisticsTypes::VERTEX_SHADER_INVOCATIONS.union(wgpu::PipelineStatisticsTypes::CLIPPER_PRIMITIVES_OUT).union(wgpu::PipelineStatisticsTypes::FRAGMENT_SHADER_INVOCATIONS);

struct Stats {
	qs: wgpu::QuerySet,
	resolve: Vec<wgpu::Buffer>,
	read: Vec<wgpu::Buffer>,
	every: u64,
	st: Mutex<State>,
}

#[derive(Default)]
struct State {
	/// This frame: a query is open (inside the level pass), the next query index, the labels of the closed ones, the render size.
	open: bool,
	next: u32,
	labels: Vec<&'static str>,
	size: (u32, u32),
	/// Per slot: (frame, labels, size) once encoded; mapping flag.
	slots: Vec<Option<(u64, Vec<&'static str>, (u32, u32))>>,
	mapping: Vec<bool>,
	/// Per slot: the frame its map was asked for.
	asked: Vec<u64>,
	ready: Vec<Arc<AtomicBool>>,
	frame: u64,
	frames: u64,
	size_seen: (u32, u32),
	/// Per label, in first-seen order: (vertex, prims, fragments) summed over the frames.
	sum: Vec<(&'static str, [u64; VALS])>,
}

static STATS: OnceLock<Option<Stats>> = OnceLock::new();

fn wanted() -> Option<u64> {
	static W: OnceLock<Option<u64>> = OnceLock::new();
	*W.get_or_init(|| std::env::var("RAC_LEVEL_STATS").ok().map(|v| v.parse().ok().filter(|&n| n > 0).unwrap_or(120)))
}

/// The device feature the probe needs, when RAC_LEVEL_STATS is set and the adapter has it.
pub(crate) fn features(adapter: &wgpu::Adapter) -> wgpu::Features {
	if wanted().is_none() {
		return wgpu::Features::empty();
	}
	let f = wgpu::Features::PIPELINE_STATISTICS_QUERY & adapter.features();
	if f.is_empty() {
		eprintln!("levelstats: RAC_LEVEL_STATS is set but the adapter has no pipeline statistics queries");
	}
	f
}

/// After the device exists (passtime.rs `init`).
pub(crate) fn init(device: &wgpu::Device) {
	let Some(every) = wanted() else { return };
	let s = device.features().contains(wgpu::Features::PIPELINE_STATISTICS_QUERY).then(|| {
		let bytes = (SEGS as usize * VALS * 8) as u64;
		let mk = |label, usage| device.create_buffer(&wgpu::BufferDescriptor { label: Some(label), size: bytes, usage, mapped_at_creation: false });
		Stats {
			qs: device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("level stats"), ty: wgpu::QueryType::PipelineStatistics(TYPES), count: SEGS }),
			resolve: (0..RING).map(|_| mk("level stats resolve", wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC)).collect(),
			read: (0..RING).map(|_| mk("level stats read", wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST)).collect(),
			every,
			st: Mutex::new(State { slots: (0..RING).map(|_| None).collect(), mapping: vec![false; RING], asked: vec![0; RING], ready: (0..RING).map(|_| Default::default()).collect(), ..Default::default() }),
		}
	});
	let _ = STATS.set(s);
}

fn stats() -> Option<&'static Stats> {
	STATS.get().and_then(|s| s.as_ref())
}

fn begin(s: &Stats, st: &mut State, pass: &mut wgpu::RenderPass<'_>) {
	if st.next < SEGS {
		pass.begin_pipeline_statistics_query(&s.qs, st.next);
		st.next += 1;
		st.open = true;
	}
}

/// Start of the level pass (render size w x h): opens the first segment.
pub(crate) fn open(pass: &mut wgpu::RenderPass<'_>, w: u32, h: u32) {
	let Some(s) = stats() else { return };
	let Ok(mut st) = s.st.lock() else { return };
	st.next = 0;
	st.labels.clear();
	st.size = (w, h);
	begin(s, &mut st, pass);
}

/// A lap point inside the level pass: the open segment ends as `name`, the next one starts.
pub(crate) fn lap(pass: &mut wgpu::RenderPass<'_>, name: &str) {
	let Some(s) = stats() else { return };
	let Ok(mut st) = s.st.lock() else { return };
	if !st.open || st.next >= SEGS {
		return;
	}
	pass.end_pipeline_statistics_query();
	st.open = false;
	st.labels.push(intern(name));
	begin(s, &mut st, pass);
}

/// End of the level pass's draws: the open segment ends as `tail`.
pub(crate) fn close(pass: &mut wgpu::RenderPass<'_>) {
	let Some(s) = stats() else { return };
	let Ok(mut st) = s.st.lock() else { return };
	if st.open {
		pass.end_pipeline_statistics_query();
		st.open = false;
		st.labels.push("tail");
	}
}

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

/// End of the frame's encoding (passtime.rs `frame_end`): resolves this frame's queries into a free readback slot.
pub(crate) fn frame_end(enc: &mut wgpu::CommandEncoder) {
	let Some(s) = stats() else { return };
	let Ok(mut st) = s.st.lock() else { return };
	let n = st.labels.len() as u32;
	st.next = 0;
	if n == 0 || st.open {
		st.labels.clear();
		st.open = false;
		return;
	}
	let Some(k) = (0..RING).find(|&k| st.slots[k].is_none() && !st.mapping[k]) else { return };
	enc.resolve_query_set(&s.qs, 0..n, &s.resolve[k], 0);
	enc.copy_buffer_to_buffer(&s.resolve[k], 0, &s.read[k], 0, n as u64 * (VALS as u64 * 8));
	let (frame, labels, size) = (st.frame, std::mem::take(&mut st.labels), st.size);
	st.slots[k] = Some((frame, labels, size));
}

/// Start of a frame's encoding (passtime.rs `frame_begin`): maps the old enough slots, sums the mapped ones, prints when due.
pub(crate) fn frame_begin(device: &wgpu::Device) {
	let Some(s) = stats() else { return };
	let Ok(mut st) = s.st.lock() else { return };
	st.frame += 1;
	let frame = st.frame;
	for k in 0..RING {
		if !st.mapping[k] && st.slots[k].as_ref().is_some_and(|(f, ..)| frame >= f + LAG) {
			let ready = st.ready[k].clone();
			ready.store(false, Ordering::Relaxed);
			s.read[k].slice(..).map_async(wgpu::MapMode::Read, move |r| ready.store(r.is_ok(), Ordering::Relaxed));
			st.mapping[k] = true;
			st.asked[k] = frame;
		}
	}
	device.poll(wgpu::PollType::Poll).ok();
	for k in 0..RING {
		if st.mapping[k] && !st.ready[k].load(Ordering::Relaxed) && frame >= st.asked[k] + 4 * LAG {
			// Review W4 finding 9: a map that failed (or never answered) frees its slot (the reading is dropped).
			s.read[k].unmap();
			st.slots[k] = None;
			st.mapping[k] = false;
			continue;
		}
		if !st.mapping[k] || !st.ready[k].load(Ordering::Relaxed) {
			continue;
		}
		let Some((_, labels, size)) = st.slots[k].take() else { continue };
		{
			let Ok(data) = s.read[k].slice(..).get_mapped_range() else { continue };
			let v: &[u64] = bytemuck::cast_slice(&data);
			add(&mut st, v, &labels, size);
		}
		s.read[k].unmap();
		st.mapping[k] = false;
	}
	if st.frames >= s.every {
		eprintln!("{}", line(&st));
		st.sum.clear();
		st.frames = 0;
	}
}

fn add(st: &mut State, v: &[u64], labels: &[&'static str], size: (u32, u32)) {
	for (i, &l) in labels.iter().enumerate() {
		let q = &v[i * VALS..i * VALS + VALS];
		let e = match st.sum.iter().position(|(n, _)| *n == l) {
			Some(p) => &mut st.sum[p].1,
			None => {
				st.sum.push((l, [0; VALS]));
				&mut st.sum.last_mut().unwrap().1
			}
		};
		for j in 0..VALS {
			e[j] += q[j];
		}
	}
	st.frames += 1;
	st.size_seen = size;
}

fn line(st: &State) -> String {
	let n = st.frames.max(1) as f64;
	let px = (st.size_seen.0 as f64 * st.size_seen.1 as f64).max(1.0);
	let mut tot = [0u64; VALS];
	let mut by: BTreeMap<usize, String> = BTreeMap::new();
	for (i, (l, q)) in st.sum.iter().enumerate() {
		for j in 0..VALS {
			tot[j] += q[j];
		}
		by.insert(i, format!("{l} {:.3} (vert {:.0}k, prim {:.0}k)", q[2] as f64 / n / px, q[0] as f64 / n / 1e3, q[1] as f64 / n / 1e3));
	}
	let body: Vec<String> = by.into_values().collect();
	format!("levelstats: {} frames {}x{} | {} | total {:.3} frag/px, {:.0}k vert, {:.0}k prim", st.frames, st.size_seen.0, st.size_seen.1, body.join(" | "), tot[2] as f64 / n / px, tot[0] as f64 / n / 1e3, tot[1] as f64 / n / 1e3)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn add_sums_per_label_and_line_divides_by_frames_and_pixels() {
		let mut st = State::default();
		// Two segments in a 10x10 frame: "a" 100 fragments, "b" 50; a second frame doubles "a".
		add(&mut st, &[3000, 1000, 100, 6000, 2000, 50], &["a", "b"], (10, 10));
		add(&mut st, &[3000, 1000, 300], &["a"], (10, 10));
		assert_eq!(st.frames, 2);
		assert_eq!(st.sum[0], ("a", [6000, 2000, 400]));
		assert_eq!(st.sum[1], ("b", [6000, 2000, 50]));
		let l = line(&st);
		assert!(l.contains("a 2.000 (vert 3k, prim 1k)") && l.contains("b 0.250") && l.contains("total 2.250 frag/px"), "{l}");
	}
}
