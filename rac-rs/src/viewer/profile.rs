//! RAC_FRAME_PROFILE=1 (PC debug aid, no retail counterpart): the CPU time of each display frame split by phase, printed
//! to stderr once per second. Off by default; when off every call is one relaxed atomic load.
//!
//! The top-level phases partition the redraw (`lap` closes the running phase); `span` times a nested piece (game update,
//! particle step, ...) into its own bucket on top of that. Each line also carries the 99th-percentile frame time of the
//! window and the phase / span breakdown of its slowest frame (the 1% lows), and the time spent between frames (outside
//! the redraw: event loop, paint delivery, window events), which `fps x frame ms` alone cannot show.

use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum Phase {
	/// Frame start up to the game ticks (input, listener water, movie poll, camera input).
	Input,
	/// The fixed 50 Hz game ticks of this frame (actors, hero, particles, camera rig).
	Ticks,
	/// Event drain, sounds per event, display camera, blob shadows.
	Post,
	/// Hero / held-object / scene actor skinning, sprite and glow lists.
	Hero,
	/// Prop posing and skinning (`posed` / `fx_posed`) and the pose smoothing.
	Props,
	/// HUD, menus, previews.
	Hud,
	/// Relight, clip view, marker / props / particles vertex uploads.
	Upload,
	/// Voices, loops, music, ambience.
	Audio,
	/// `Gpu::draw` before the swapchain image: shrub / tie / tfrag culls, uniforms.
	Cull,
	/// `get_current_texture` (blocks on vsync / frame latency).
	Acquire,
	/// Command encoding (all passes).
	Encode,
	/// Submit and present.
	Present,
	/// After the last lap up to the frame end (window requests, shot bookkeeping, anything a frame without a draw skips).
	End,
}

const PHASES: [Phase; 13] = [Phase::Input, Phase::Ticks, Phase::Post, Phase::Hero, Phase::Props, Phase::Hud, Phase::Upload, Phase::Audio, Phase::Cull, Phase::Acquire, Phase::Encode, Phase::Present, Phase::End];

fn name(p: Phase) -> &'static str {
	match p {
		Phase::Input => "input",
		Phase::Ticks => "ticks",
		Phase::Post => "post",
		Phase::Hero => "hero",
		Phase::Props => "props",
		Phase::Hud => "hud",
		Phase::Upload => "upload",
		Phase::Audio => "audio",
		Phase::Cull => "cull",
		Phase::Acquire => "acquire",
		Phase::Encode => "encode",
		Phase::Present => "present",
		Phase::End => "end",
	}
}

static ON: AtomicBool = AtomicBool::new(false);

/// Accumulated times of one reporting window (or of the whole run, `Totals`).
#[derive(Clone, Default, Debug)]
pub(super) struct Totals {
	pub frames: u32,
	pub ticks: u32,
	pub phase: [Duration; 13],
	/// Nested spans by name: (time, count).
	pub spans: Vec<(&'static str, Duration, u32)>,
	/// Whole redraw (first lap to frame end) and the slowest one.
	pub frame: Duration,
	pub worst: Duration,
	/// Every frame time of the window (ms), for the percentiles.
	pub times: Vec<f32>,
	/// The slowest frame's own breakdown.
	pub worst_frame: Option<FrameRec>,
	/// Time between a frame end and the next frame start (event loop, OS paint delivery, other events) and its longest.
	pub gap: Duration,
	pub worst_gap: Duration,
}

/// One frame's phases and spans (kept for the slowest frame of a window).
#[derive(Clone, Default, Debug)]
pub(super) struct FrameRec {
	pub ms: f32,
	pub ticks: u32,
	pub phase: [Duration; 13],
	pub spans: Vec<(&'static str, Duration, u32)>,
}

impl FrameRec {
	fn report(&self) -> String {
		let mut s = format!("worst {:.2} ms, {} ticks:", self.ms, self.ticks);
		for (k, p) in PHASES.iter().enumerate() {
			s += &format!(" {} {:.2}", name(*p), self.phase[k].as_secs_f32() * 1000.0);
		}
		let mut spans = self.spans.clone();
		spans.sort_by(|a, b| b.1.cmp(&a.1));
		s += " |";
		for &(name, d, c) in spans.iter().take(16) {
			s += &format!(" {name} {:.0}us x{c}", d.as_secs_f32() * 1e6);
		}
		s
	}
}

fn add_span(spans: &mut Vec<(&'static str, Duration, u32)>, name: &'static str, d: Duration, c: u32) {
	match spans.iter_mut().find(|s| s.0 == name) {
		Some(s) => {
			s.1 += d;
			s.2 += c;
		}
		None => spans.push((name, d, c)),
	}
}

impl Totals {
	fn merge(&mut self, o: &Totals) {
		self.frames += o.frames;
		self.ticks += o.ticks;
		for k in 0..self.phase.len() {
			self.phase[k] += o.phase[k];
		}
		for &(n, d, c) in &o.spans {
			add_span(&mut self.spans, n, d, c);
		}
		self.frame += o.frame;
		self.worst = self.worst.max(o.worst);
		self.gap += o.gap;
		self.worst_gap = self.worst_gap.max(o.worst_gap);
		self.times.extend_from_slice(&o.times);
		if o.worst_frame.as_ref().is_some_and(|w| self.worst_frame.as_ref().is_none_or(|s| w.ms > s.ms)) {
			self.worst_frame = o.worst_frame.clone();
		}
	}

	/// The `q` quantile (0..1) of the frame times in ms (nearest rank).
	pub fn percentile(&self, q: f32) -> f32 {
		if self.times.is_empty() {
			return 0.0;
		}
		let mut t = self.times.clone();
		t.sort_by(f32::total_cmp);
		t[((q * t.len() as f32).ceil() as usize).clamp(1, t.len()) - 1]
	}

	/// One line: per-frame milliseconds by phase, then the nested spans (microseconds per call x calls per frame).
	pub fn report(&self, secs: f32) -> String {
		let n = self.frames.max(1) as f32;
		let ms = |d: Duration| d.as_secs_f32() * 1000.0 / n;
		let mut s = format!("frame-profile: {:.0} fps, {} ticks | frame {:.2} ms (p99 {:.2}, worst {:.2}), between frames {:.2} ms (worst {:.2}) |", self.frames as f32 / secs.max(1e-6), self.ticks, ms(self.frame), self.percentile(0.99), self.worst.as_secs_f32() * 1000.0, ms(self.gap), self.worst_gap.as_secs_f32() * 1000.0);
		for (k, p) in PHASES.iter().enumerate() {
			s += &format!(" {} {:.2}", name(*p), ms(self.phase[k]));
		}
		if !self.spans.is_empty() {
			s += " |";
			for &(name, d, c) in &self.spans {
				s += &format!(" {name} {:.0}us x{:.1}", d.as_secs_f32() * 1e6 / c.max(1) as f32, c as f32 / n);
			}
		}
		if let Some(w) = &self.worst_frame {
			s += &format!("\n  {}", w.report());
		}
		s
	}
}

struct State {
	mark: Instant,
	start: Instant,
	window_start: Instant,
	window: Totals,
	all: Totals,
	/// The running frame's phases and spans.
	cur: FrameRec,
	/// The last frame end (for the between-frames time).
	last_end: Option<Instant>,
	print: bool,
}

thread_local! {
	static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// Reads RAC_FRAME_PROFILE once at start-up.
pub(super) fn init_from_env() {
	if std::env::var("RAC_FRAME_PROFILE").is_ok_and(|v| v == "1") {
		enable(true);
	}
}

/// Turns profiling on for this thread (`print`: the once-per-second stderr line).
pub(super) fn enable(print: bool) {
	let now = Instant::now();
	STATE.with(|s| *s.borrow_mut() = Some(State { mark: now, start: now, window_start: now, window: Totals::default(), all: Totals::default(), cur: FrameRec::default(), last_end: None, print }));
	ON.store(true, Ordering::Relaxed);
}

#[inline]
pub(super) fn on() -> bool {
	ON.load(Ordering::Relaxed)
}

/// Start of a redraw.
#[inline]
pub(super) fn frame_start() {
	if !on() {
		return;
	}
	STATE.with(|s| {
		if let Some(s) = s.borrow_mut().as_mut() {
			let now = Instant::now();
			s.mark = now;
			s.start = now;
			if let Some(e) = s.last_end.take() {
				s.window.gap += now - e;
				s.window.worst_gap = s.window.worst_gap.max(now - e);
			}
		}
	});
}

/// Closes the running phase as `p`.
#[inline]
pub(super) fn lap(p: Phase) {
	if !on() {
		return;
	}
	STATE.with(|s| {
		if let Some(s) = s.borrow_mut().as_mut() {
			let now = Instant::now();
			let k = PHASES.iter().position(|&q| q == p).unwrap_or(0);
			s.cur.phase[k] += now - s.mark;
			s.mark = now;
		}
	});
}

/// Times a nested piece into its own bucket until the guard drops.
pub(super) struct Span(Option<(&'static str, Instant)>);

#[inline]
pub(super) fn span(name: &'static str) -> Span {
	Span(on().then(|| (name, Instant::now())))
}

impl Drop for Span {
	fn drop(&mut self) {
		if let Some((name, t)) = self.0 {
			let d = t.elapsed();
			STATE.with(|s| {
				if let Some(s) = s.borrow_mut().as_mut() {
					add_span(&mut s.cur.spans, name, d, 1);
				}
			});
		}
	}
}

/// [PC] The on-screen meter (RAC_FPS_METER=1, test launchers; no retail counterpart): over half-second windows, the display
/// frames per second, the average and worst time from one frame to the next, and the game steps per second as a share of the
/// disc's field rate (100% = full speed). `frame_end` counts, `meter_draw` puts the two lines in the top right corner.
struct Meter {
	since: Instant,
	last: Option<Instant>,
	frames: u32,
	ticks: u32,
	sum: Duration,
	worst: Duration,
	lines: Option<(String, String, [f32; 4])>,
}

thread_local! {
	static METER: RefCell<Option<Meter>> = const { RefCell::new(None) };
}

fn meter_on() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_FPS_METER").is_ok_and(|v| v == "1"))
}

fn meter_frame(ticks: u32) {
	METER.with(|m| {
		let now = Instant::now();
		let mut b = m.borrow_mut();
		let m = b.get_or_insert_with(|| Meter { since: now, last: None, frames: 0, ticks: 0, sum: Duration::ZERO, worst: Duration::ZERO, lines: None });
		if let Some(l) = m.last {
			m.sum += now - l;
			m.worst = m.worst.max(now - l);
		}
		m.last = Some(now);
		m.frames += 1;
		m.ticks += ticks;
		let secs = (now - m.since).as_secs_f32();
		if secs >= 0.5 && m.frames > 1 {
			let ms = |d: Duration| d.as_secs_f32() * 1000.0;
			let steps = m.ticks as f32 / secs;
			let pct = 100.0 * steps / ee_oracle::ver::field_hz() as f32;
			let colour = if pct >= 98.0 { [0.55, 1.0, 0.55, 1.0] } else if pct >= 80.0 { [1.0, 0.9, 0.4, 1.0] } else { [1.0, 0.45, 0.4, 1.0] };
			let a = format!("{:.0} FPS  {:.1} MS  MAX {:.1}", m.frames as f32 / secs, ms(m.sum) / (m.frames - 1).max(1) as f32, ms(m.worst));
			let b = format!("GAME SPEED {pct:.0}%  {steps:.0} STEPS/S");
			*m = Meter { since: now, last: Some(now), frames: 0, ticks: 0, sum: Duration::ZERO, worst: Duration::ZERO, lines: Some((a, b, colour)) };
		}
	});
}

/// Draws the meter's lines (window pixels) when RAC_FPS_METER=1.
pub(super) fn meter_draw(out: &mut Vec<crate::overlay::Quad>, w: f32, h: f32) {
	if !meter_on() {
		return;
	}
	METER.with(|m| {
		let Some((a, b, colour)) = m.borrow().as_ref().and_then(|m| m.lines.clone()) else { return };
		let k = (h / 720.0).max(0.6);
		let size = 14.0 * k;
		let tw = crate::overlay::text_width(&a, size).max(crate::overlay::text_width(&b, size));
		let x = w - tw - 14.0 * k;
		crate::overlay::rect(out, [x - 6.0 * k, 6.0 * k, tw + 12.0 * k, 2.0 * size + 12.0 * k], [0.0, 0.0, 0.0, 0.55]);
		crate::overlay::text(out, &a, x, 10.0 * k, size, [1.0, 1.0, 1.0, 1.0]);
		crate::overlay::text(out, &b, x, 14.0 * k + size, size, colour);
	});
}

/// End of a redraw that ran `ticks` game ticks; prints the window once a second.
pub(super) fn frame_end(ticks: u32) {
	if meter_on() {
		meter_frame(ticks);
	}
	if !on() {
		return;
	}
	STATE.with(|s| {
		let mut b = s.borrow_mut();
		let Some(s) = b.as_mut() else { return };
		let now = Instant::now();
		let d = now - s.start;
		s.cur.phase[PHASES.len() - 1] += now - s.mark;
		let mut cur = std::mem::take(&mut s.cur);
		cur.ms = d.as_secs_f32() * 1000.0;
		cur.ticks = ticks;
		let w = &mut s.window;
		w.frames += 1;
		w.ticks += ticks;
		w.frame += d;
		w.worst = w.worst.max(d);
		w.times.push(cur.ms);
		for k in 0..w.phase.len() {
			w.phase[k] += cur.phase[k];
		}
		for &(n, d, c) in &cur.spans {
			add_span(&mut w.spans, n, d, c);
		}
		if w.worst_frame.as_ref().is_none_or(|x| cur.ms > x.ms) {
			w.worst_frame = Some(cur);
		} else {
			// Keep the span list's allocation for the next frame.
			cur.spans.clear();
			cur.phase = [Duration::ZERO; 13];
			s.cur = cur;
		}
		s.last_end = Some(now);
		let secs = (now - s.window_start).as_secs_f32();
		if secs >= 1.0 {
			if s.print {
				eprintln!("{}", s.window.report(secs));
			}
			let w = std::mem::take(&mut s.window);
			s.all.merge(&w);
			s.window_start = now;
		}
	});
}


#[cfg(test)]
mod tests {
	use super::*;

}
