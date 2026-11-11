//! [PC] RAC_DIAG (diag.rs): the pure part of the session record: one second's aggregate of the display frames (fps, 1% low,
//! worst frame, CPU phases), the spike rule, the perf.csv row and the options diff. No clock, no file, no global: unit tested.

use std::fmt::Write;

/// The game's facts for a row (`Pack::diag_facts`; a game without the hook gives the defaults: no level, no hero).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Facts {
	/// Level name ("" = unknown) and index (-1 = none).
	pub level: &'static str,
	pub level_id: i32,
	/// The game is paused (a pause menu is up).
	pub paused: bool,
	/// The game's front end (title, boot, load menus) runs instead of a level.
	pub menu: bool,
	/// The hero's world position, if there is one.
	pub hero: Option<[f32; 3]>,
	/// The camera mode's name ("" = unknown).
	pub camera: &'static str,
}

/// Number of CPU phases (viewer/profile.rs `PHASES`).
pub const PHASES: usize = 13;

/// Cost counters (`diag::count`), in perf.csv column order.
pub const COUNT_NAMES: [&str; 5] = ["lights", "fires", "fire_maps", "particles", "smoke_fires"];

/// Causes (`diag::cause` bits), in bit order.
pub const CAUSE_NAMES: [&str; 6] = ["load", "pipeline", "texture", "swapchain", "screenshot", "options"];

/// The names of the causes in `bits`, joined by '+' ("" = none).
pub fn cause_text(bits: u32) -> String {
	CAUSE_NAMES.iter().enumerate().filter(|(i, _)| bits & (1 << i) != 0).map(|(_, n)| *n).collect::<Vec<_>>().join("+")
}

/// The `q` quantile (0..1) of `v` by nearest rank (0 for an empty list); sorts `v`.
pub fn percentile(v: &mut [f32], q: f32) -> f32 {
	if v.is_empty() {
		return 0.0;
	}
	v.sort_by(f32::total_cmp);
	v[((q * v.len() as f32).ceil() as usize).clamp(1, v.len()) - 1]
}

/// The spike rule: a frame interval over twice the running median (the last second without a load step), at least 25 ms.
pub fn spike_limit(median_ms: f32) -> f32 {
	(2.0 * median_ms).max(25.0)
}

/// The spike rule's median after a second with median `med` and causes `causes`: a load second keeps the old one (its long
/// load frames would raise the limit for the next second).
pub fn next_median(old: f32, med: f32, causes: u32) -> f32 {
	if causes & 1 != 0 { old } else { med }
}

/// Seconds in the 1% low's rolling window (`Rolling`).
pub const LOW_WINDOW: usize = 10;

/// The frame intervals of the last `LOW_WINDOW` seconds: the perf.csv `low1_10s_fps` (a per-second 1% low is no 1% low: under
/// 100 frames it is just the worst frame).
#[derive(Clone, Debug, Default)]
pub struct Rolling {
	secs: std::collections::VecDeque<Vec<f32>>,
}

impl Rolling {
	/// Adds a finished second's intervals (the oldest second drops out once the window is full).
	pub fn push(&mut self, intervals: &[f32]) {
		let mut v = if self.secs.len() >= LOW_WINDOW { self.secs.pop_front().unwrap_or_default() } else { Vec::new() };
		v.clear();
		v.extend_from_slice(intervals);
		self.secs.push_back(v);
	}

	/// The 1% low fps of the window plus `current` (the running second): 1000 / the 99th percentile interval (0 = no frames).
	pub fn low1(&self, current: &[f32]) -> f32 {
		let mut all: Vec<f32> = self.secs.iter().flatten().chain(current).copied().collect();
		let p = percentile(&mut all, 0.99);
		if p > 0.0 { 1000.0 / p } else { 0.0 }
	}
}

/// One second of display frames.
#[derive(Clone, Debug, Default)]
pub struct Second {
	/// Frame-to-frame intervals (ms).
	pub intervals: Vec<f32>,
	pub ticks: u32,
	/// Summed CPU phase times of the redraws (ms).
	pub phase: [f32; PHASES],
	/// Redraws added (the phases' divisor).
	pub redraws: u32,
	/// Causes seen this second (bits).
	pub causes: u32,
	/// Spikes found / written in full this second.
	pub spikes: u32,
}

impl Second {
	pub fn add(&mut self, interval_ms: Option<f32>, ticks: u32, phase_ms: &[f32; PHASES], causes: u32) {
		if let Some(i) = interval_ms {
			self.intervals.push(i);
		}
		self.ticks += ticks;
		self.redraws += 1;
		for (a, b) in self.phase.iter_mut().zip(phase_ms) {
			*a += *b;
		}
		self.causes |= causes;
	}

	/// (fps over `secs`, average interval ms, 1% low fps (the 99th percentile interval), worst interval ms, median interval ms).
	pub fn rates(&self, secs: f32) -> (f32, f32, f32, f32, f32) {
		let mut v = self.intervals.clone();
		let n = v.len().max(1) as f32;
		let avg = v.iter().sum::<f32>() / n;
		let worst = v.iter().cloned().fold(0.0, f32::max);
		let p99 = percentile(&mut v, 0.99);
		let med = percentile(&mut v, 0.5);
		(self.intervals.len() as f32 / secs.max(1e-3), avg, if p99 > 0.0 { 1000.0 / p99 } else { 0.0 }, worst, med)
	}
}

/// The GPU side of a second (viewer/passtime.rs `diag_take`): frames read, average pass sum and span (ms), and the average ms
/// per pass label, biggest first.
#[derive(Clone, Debug, Default)]
pub struct GpuSecond {
	pub frames: u64,
	pub sum: f64,
	pub span: f64,
	pub passes: Vec<(&'static str, f64)>,
}

/// The perf.csv header (one `S` row per second, `M` rows for the mark key).
pub fn header() -> String {
	let mut s = String::from("kind,t,level,state,fps,avg_ms,low1_10s_fps,max_ms,steps_s");
	for p in ["input", "ticks", "post", "hero", "props", "hud", "upload", "audio", "cull", "acquire", "encode", "present", "end"] {
		let _ = write!(s, ",cpu_{p}");
	}
	s += ",gpu_ms,gpu_span_ms,gpu_frames";
	for c in COUNT_NAMES {
		let _ = write!(s, ",{c}");
	}
	s += ",causes,spikes,hero_x,hero_y,hero_z,camera,window,render,upscaler,fg,fg_presented_fps,gpu_passes";
	s
}

/// Everything one row holds besides the second itself.
pub struct RowIn<'a> {
	pub kind: &'a str,
	pub t: f32,
	pub secs: f32,
	pub facts: &'a Facts,
	pub gpu: &'a GpuSecond,
	pub counts: [u32; 5],
	/// "WxH" of the window and of the render target, the upscaler and frame generation names.
	pub window: &'a str,
	pub render: &'a str,
	pub upscaler: &'a str,
	pub fg: &'a str,
	/// Frames frame generation presented per second (rendered + generated; None = no frame generation swapchain counts).
	pub presented: Option<f32>,
	/// The 1% low fps of the last `LOW_WINDOW` seconds (`Rolling::low1`).
	pub low1: f32,
}

/// The game's state word of a row: menu (front end), paused, load (a load step this second) or play.
pub fn state(f: &Facts, causes: u32) -> &'static str {
	if causes & 1 != 0 {
		"load"
	} else if f.menu {
		"menu"
	} else if f.paused {
		"paused"
	} else {
		"play"
	}
}

/// One perf.csv row (no newline). Text fields never hold a comma (pass names keep theirs out: ';' and '=' separate them).
pub fn row(s: &Second, r: &RowIn) -> String {
	let (fps, avg, _, worst, _) = s.rates(r.secs);
	let n = s.redraws.max(1) as f32;
	let mut o = format!("{},{:.2},{},{},{:.1},{:.2},{:.1},{:.2},{:.1}", r.kind, r.t, clean(r.facts.level), state(r.facts, s.causes), fps, avg, r.low1, worst, s.ticks as f32 / r.secs.max(1e-3));
	for p in s.phase {
		let _ = write!(o, ",{:.3}", p / n);
	}
	let _ = write!(o, ",{:.3},{:.3},{}", r.gpu.sum, r.gpu.span, r.gpu.frames);
	for c in r.counts {
		let _ = write!(o, ",{c}");
	}
	let _ = write!(o, ",{},{}", cause_text(s.causes), s.spikes);
	match r.facts.hero {
		Some(h) => {
			let _ = write!(o, ",{:.2},{:.2},{:.2}", h[0], h[1], h[2]);
		}
		None => o += ",,,",
	}
	let passes: Vec<String> = r.gpu.passes.iter().map(|(l, ms)| format!("{}={:.3}", clean(l), ms)).collect();
	let pres = r.presented.map_or(String::new(), |p| format!("{p:.1}"));
	let _ = write!(o, ",{},{},{},{},{},{pres},{}", clean(r.facts.camera), clean(r.window), clean(r.render), clean(r.upscaler), clean(r.fg), passes.join(";"));
	o
}

/// A text for a csv field: commas, ';', '=' and line breaks become spaces.
pub fn clean(s: &str) -> String {
	s.chars().map(|c| if matches!(c, ',' | ';' | '=' | '\n' | '\r') { ' ' } else { c }).collect()
}

/// The `key=value` lines of an options file text (pcopts.rs `to_text`), comments and blanks left out.
fn pairs(text: &str) -> Vec<(&str, &str)> {
	text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')).filter_map(|l| l.split_once('=')).map(|(k, v)| (k.trim(), v.trim())).collect()
}

/// What changed from options text `old` to `new`: one "key: old -> new" per changed, added or removed key, in `new`'s order.
pub fn options_diff(old: &str, new: &str) -> Vec<String> {
	let (a, b) = (pairs(old), pairs(new));
	let get = |v: &[(&str, &str)], k: &str| v.iter().find(|e| e.0 == k).map(|e| e.1.to_string());
	let mut out = Vec::new();
	for (k, v) in &b {
		match get(&a, k) {
			Some(o) if o == *v => {}
			Some(o) => out.push(format!("{k}: {o} -> {v}")),
			None => out.push(format!("{k}: (unset) -> {v}")),
		}
	}
	for (k, v) in &a {
		if get(&b, k).is_none() {
			out.push(format!("{k}: {v} -> (unset)"));
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn percentile_takes_the_nearest_rank() {
		let mut v = vec![5.0, 1.0, 3.0, 2.0, 4.0];
		assert_eq!(percentile(&mut v, 0.5), 3.0);
		assert_eq!(percentile(&mut v, 0.99), 5.0);
		assert_eq!(percentile(&mut v, 0.0), 1.0);
		assert_eq!(percentile(&mut [], 0.5), 0.0);
	}

	#[test]
	fn spikes_need_twice_the_median_and_25_ms() {
		assert_eq!(spike_limit(5.0), 25.0);
		assert_eq!(spike_limit(16.7), 33.4);
		// a load second keeps the old median
		assert_eq!(next_median(8.0, 200.0, 1), 8.0);
		assert_eq!(next_median(8.0, 9.0, 2), 9.0);
	}

	#[test]
	fn the_rolling_low_spans_ten_seconds() {
		let mut r = Rolling::default();
		// one 100 ms hitch in the first second, then 11 clean seconds of 100 frames at 10 ms
		let mut first = vec![10.0; 99];
		first.push(100.0);
		r.push(&first);
		for _ in 0..8 {
			r.push(&[10.0; 100]);
		}
		// 1000 frames, 1 hitch: the 99th percentile is a clean frame
		assert_eq!(r.low1(&[10.0; 100]), 100.0);
		// 10 hitches in 1000 frames reach the 1% rank
		r.push(&[50.0; 10]);
		assert_eq!(r.low1(&[]), 1000.0 / 50.0);
		// the first second drops out after the window is full
		assert_eq!(r.secs.len(), LOW_WINDOW);
		assert_eq!(Rolling::default().low1(&[]), 0.0);
	}

	#[test]
	fn a_second_gives_fps_low_and_worst() {
		let mut s = Second::default();
		let ph = [1.0; PHASES];
		for k in 0..100 {
			s.add(Some(if k == 0 { 50.0 } else { 10.0 }), 1, &ph, if k == 3 { 4 } else { 0 });
		}
		let (fps, avg, low, worst, med) = s.rates(1.0);
		assert_eq!(fps, 100.0);
		assert!((avg - 10.4).abs() < 1e-3);
		assert_eq!((low, worst, med), (100.0, 50.0, 10.0));
		assert_eq!(s.causes, 4);
		assert_eq!(cause_text(s.causes), "texture");
		assert_eq!(cause_text(1 | 8), "load+swapchain");
	}

	#[test]
	fn the_row_matches_the_header() {
		let mut s = Second::default();
		s.add(Some(16.0), 1, &[0.5; PHASES], 0);
		let f = Facts { level: "Veldin, 1", level_id: 0, hero: Some([1.0, 2.0, 3.0]), camera: "modern", ..Facts::default() };
		let gpu = GpuSecond { frames: 60, sum: 4.0, span: 5.0, passes: vec![("level", 2.5), ("post", 1.5)] };
		let r = row(&s, &RowIn { kind: "S", t: 1.0, secs: 1.0, facts: &f, gpu: &gpu, counts: [1, 2, 3, 4, 5], window: "1920x1080", render: "1920x1080", upscaler: "off", fg: "off", presented: None, low1: 62.5 });
		assert_eq!(r.split(',').count(), header().split(',').count());
		assert!(r.starts_with("S,1.00,Veldin  1,play,"));
		assert!(r.ends_with(",modern,1920x1080,1920x1080,off,off,,level=2.500;post=1.500"));
		let r2 = row(&s, &RowIn { kind: "S", t: 1.0, secs: 1.0, facts: &f, gpu: &gpu, counts: [0; 5], window: "", render: "", upscaler: "fsr", fg: "fsr", presented: Some(120.0), low1: 0.0 });
		assert_eq!(r2.split(',').count(), header().split(',').count());
		assert!(r2.contains(",fsr,fsr,120.0,level="));
	}

	#[test]
	fn state_prefers_load_then_menu_then_paused() {
		let f = Facts { paused: true, ..Facts::default() };
		assert_eq!(state(&f, 1), "load");
		assert_eq!(state(&f, 0), "paused");
		assert_eq!(state(&Facts { menu: true, ..f.clone() }, 0), "menu");
		assert_eq!(state(&Facts::default(), 2), "play");
	}

	#[test]
	fn options_diff_lists_changes_additions_and_removals() {
		let old = "# comment\nfps=60\nupscaler=off\nold=1\n";
		let new = "fps=120\nupscaler=off\nframegen=fsr\n";
		assert_eq!(options_diff(old, new), vec!["fps: 60 -> 120", "framegen: (unset) -> fsr", "old: 1 -> (unset)"]);
		assert!(options_diff(new, new).is_empty());
	}
}
