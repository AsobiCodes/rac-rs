//! Streaming FMV player (decomp/notes/cutscenes.md): a worker thread demuxes the PSS from the disc,
//! decodes MPEG-2 video and the language's ADPCM track into bounded queues; the owner polls once per
//! rendered frame and gets the frame due at the audio clock (90 kHz PTS), or at the wall clock when
//! there is no audio device. Dropping the player stops the worker and closes the audio stream.
//! The queue and clock rules are in `feed.rs`: the audio is the master clock and the video never stops it.

use super::adpcm_stream::{parse_header, AdpcmDecoder};
use super::feed::{self, Fill, HARD_FRAMES, PRIME};
use super::mpeg2::{Decoder, Frame};
use super::pss::{track_for_language, DiscReader, Packet, PssDemuxer};
use super::synclog::{Knobs, Row, SyncLog};
use crate::audio::{Audio, StreamSource};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The audio clock holds (nothing played) this long while the output device keeps pulling: the stream is starved, the decoder or the disc
/// is behind. The movie waits for it, video and audio held together, and goes on in sync (v0.4.1, GitHub #7: it used to give the audio up
/// after 0.5 s here and play the rest of the movie silent).
const STARVED: Duration = Duration::from_millis(250);
/// ... and gives the audio up after this long (something is broken: the disc or the worker is gone).
const STARVE_MAX: Duration = Duration::from_secs(8);
/// An output device that pulled nothing for this long is gone or asleep (its clock would hold the movie, and the game thread parked in
/// fmv_bridge, forever): the movie continues on the wall clock, video only.
const DEAD_AFTER: Duration = Duration::from_millis(1000);
/// A movie waits for 0.25 s of audio before it starts. When the demuxer has read this much video (90 kHz ticks of video PTS, from the
/// first video packet to the latest) and not one audio packet of the chosen language track came with it, that track is not in the stream:
/// the movie starts video only on the wall clock. Without this it stayed black with the game thread parked for good: the worker stops
/// reading at the spill cap (`feed::SPILL_MAX`) long before the end of the stream, so `done` never came. The verdict is the stream's, not
/// the clock's: a slow disc (an HDD spinning up, a network drive) delays the reading but not the span, so a movie whose audio is merely
/// late keeps its sound. The tracks are interleaved within a fraction of a second, so 3 s without a packet of the track is conclusive.
const NO_AUDIO_SPAN: u64 = 3 * 90_000;

/// The audio clock watchdog's memory: frames played, device pulls, and when each last changed.
type Seen = (u64, u64, Instant, Instant);

/// What the watchdog decides at one poll.
#[derive(Debug, PartialEq, Eq)]
enum Watch {
	Fine,
	/// The clock is held by a starved stream (device alive); nothing to do but wait.
	Starved,
	/// The audio track is over (nothing more will come): the rest of the video runs on the wall clock. Not a fault.
	Ended,
	GiveUp(&'static str),
}

/// One watchdog step: `played` / `pulls` are `StreamSource::played` / `pulls` now, `over` that the audio track has ended and its queue is
/// drained. `played` standing still alone is a starved stream and is waited out; it is the device that stopped (no pulls) or a stall past
/// `STARVE_MAX` that gives the audio up. The video queue never gives the audio up: the worker keeps reading past a full video queue
/// (`feed::must_wait`), so a starved stream always finds its audio packets (GitHub #7: a full video queue used to end the audio here, and
/// the rest of the movie played without sound).
fn watch(seen: &mut Option<Seen>, played: u64, pulls: u64, over: bool, now: Instant) -> Watch {
	if over {
		return Watch::Ended;
	}
	let (p, n, tp, tn) = seen.unwrap_or((played, pulls, now, now));
	let tp = if p == played { tp } else { now };
	let tn = if n == pulls { tn } else { now };
	*seen = Some((played, pulls, tp, tn));
	if now.saturating_duration_since(tn) > DEAD_AFTER {
		Watch::GiveUp("device stopped pulling")
	} else if now.saturating_duration_since(tp) > STARVE_MAX {
		Watch::GiveUp("stream starved too long")
	} else if now.saturating_duration_since(tp) > STARVED {
		Watch::Starved
	} else {
		Watch::Fine
	}
}

/// The start verdict from the stream alone: `audio_seen` = an audio packet of the language track arrived (it gave the track's first PTS),
/// `video_span` = video PTS ticks the demuxer has read since the first video packet. True = the track is not in this stream.
fn no_audio_in_stream(audio_seen: bool, video_span: u64) -> bool {
	!audio_seen && video_span >= NO_AUDIO_SPAN
}

struct Shared {
	video: Mutex<VecDeque<Frame>>,
	/// PTS of the first audio packet of the track (90 kHz), once seen.
	audio_pts0: Mutex<Option<u64>>,
	done: AtomicBool,
	stop: AtomicBool,
	/// The output device stopped pulling the stream: decode video only, the wall clock paces the movie.
	audio_off: AtomicBool,
	/// The audio track decoded its END frame: no more audio will come (a movie's audio often ends before its last video frames).
	audio_end: AtomicBool,
	/// The output latency (ms, f64 bits) the owner's clock subtracts; the worker drops late frames against the same clock.
	lat_bits: AtomicU64,
	/// Video PTS ticks the demuxer has read so far, from the first video packet with a PTS to the latest (`no_audio_in_stream`).
	video_span: AtomicU64,
}

impl Shared {
	fn new(video: VecDeque<Frame>, audio_pts0: Option<u64>) -> Shared {
		Shared { video: Mutex::new(video), audio_pts0: Mutex::new(audio_pts0), done: AtomicBool::new(false), stop: AtomicBool::new(false), audio_off: AtomicBool::new(false), audio_end: AtomicBool::new(false), lat_bits: AtomicU64::new(0), video_span: AtomicU64::new(0) }
	}

	/// The demuxer read a video packet stamped `pts`; `first` is the PTS of the first stamped one.
	fn note_video_pts(&self, first: &mut Option<u64>, pts: Option<u64>) {
		if let Some(p) = pts {
			let f = *first.get_or_insert(p);
			self.video_span.fetch_max(p.saturating_sub(f), Ordering::Relaxed);
		}
	}

	/// The clock of what is heard (`feed::heard_ticks`); None until the first audio packet gave the stream's start.
	fn audio_clock(&self, a: &StreamSource) -> Option<u64> {
		let t0 = (*self.audio_pts0.lock().unwrap())?;
		Some(feed::heard_ticks(t0, a.played(), a.rate, f64::from_bits(self.lat_bits.load(Ordering::Relaxed))))
	}
}

pub struct Player {
	shared: Arc<Shared>,
	audio: Option<Arc<StreamSource>>,
	worker: Option<std::thread::JoinHandle<()>>,
	started: bool,
	/// Wall clock fallback: (instant the clock last ran from, 90 kHz ticks at that instant).
	wall: Option<(Instant, u64)>,
	first_pts: Option<u64>,
	current: Option<Frame>,
	pub finished: bool,
	/// The audio clock watchdog (`watch`).
	moved: Option<Seen>,
	/// "audio starved" was logged for the current stall.
	starved: bool,
	/// The smoothed output latency in ms (`feed::smooth_latency`), None until the device reported one.
	lat: Option<f64>,
	/// RAC_FMV_SYNC_LOG: the audio / video offset over the movie (`synclog.rs`).
	log: Option<SyncLog>,
	/// Test-only delays (`Knobs`).
	knobs: Knobs,
	/// When the player started (the knobs' time base).
	t0: Instant,
}

/// The worker's video side: packets to the decoder, decoded frames to the queue.
struct VideoSide {
	dec: Decoder,
	last_pts: Option<u64>,
	frame_ticks: u64,
	/// Test-only: ms added to every packet decoded (RAC_FMV_TEST_DECODE_MS).
	decode_ms: u64,
	/// Packets the decoded queue had no room for (`feed::spill`), in stream order, and their bytes.
	spilled: VecDeque<(Option<u64>, Vec<u8>)>,
	spilled_bytes: usize,
}

impl VideoSide {
	fn decode(&mut self, shared: &Shared, pts: Option<u64>, data: &[u8]) {
		if self.decode_ms > 0 {
			std::thread::sleep(Duration::from_millis(self.decode_ms));
		}
		self.dec.push(data, pts);
		self.drain(shared);
	}

	fn drain(&mut self, shared: &Shared) {
		while let Some(mut f) = self.dec.next_frame() {
			let p = f.pts.or(self.last_pts.map(|p| p + self.frame_ticks)).unwrap_or(0);
			f.pts = Some(p);
			self.last_pts = Some(p);
			shared.video.lock().unwrap().push_back(f);
		}
	}

	/// Sends spilled packets to the decoder while the decoded queue has fewer than `below` frames.
	fn replay(&mut self, shared: &Shared, below: usize) {
		while shared.video.lock().unwrap().len() < below {
			let Some((pts, data)) = self.spilled.pop_front() else { break };
			self.spilled_bytes -= data.len();
			self.decode(shared, pts, &data);
		}
	}

	/// The stream is over: the rest of the spill and the decoder's own tail go out (waiting for the owner to make room).
	fn finish(&mut self, shared: &Shared) {
		while !self.spilled.is_empty() && !shared.stop.load(Ordering::Relaxed) {
			self.replay(shared, HARD_FRAMES);
			if !self.spilled.is_empty() {
				std::thread::sleep(Duration::from_millis(2));
			}
		}
		self.dec.flush();
		self.drain(shared);
	}
}

fn run(iso: String, lba: u32, bytes: u64, track: u8, fps: f32, shared: Arc<Shared>, audio: Option<Arc<StreamSource>>) {
	let Ok(mut disc) = crate::iso::Disc::open(std::path::Path::new(&iso)) else {
		shared.done.store(true, Ordering::Relaxed);
		return;
	};
	let mut dm = PssDemuxer::new(DiscReader::new(&mut disc, lba, bytes));
	let mut video = VideoSide { dec: Decoder::new(), last_pts: None, frame_ticks: (90000.0 / fps.max(1.0)) as u64, decode_ms: super::synclog::test_decode_ms(), spilled: VecDeque::new(), spilled_bytes: 0 };
	let mut adpcm: Option<AdpcmDecoder> = None;
	let mut pcm = Vec::new();
	let mut first_vpts = None;
	loop {
		if shared.stop.load(Ordering::Relaxed) {
			return;
		}
		let aon = audio.is_some() && !shared.audio_off.load(Ordering::Relaxed);
		let lead = if aon { audio.as_ref().map_or(0.0, |a| a.buffered() as f64 / a.rate as f64) } else { 0.0 };
		// Frames the clock has passed are of no use to the owner but the newest of them: drop them first, so an owner that stopped polling
		// (a render hitch, a dragged window) cannot fill the queue and hold the stream with it.
		let mut frames = {
			let mut q = shared.video.lock().unwrap();
			if aon && q.len() >= feed::MAX_FRAMES {
				if let Some(c) = audio.as_ref().and_then(|a| shared.audio_clock(a)) {
					feed::drop_late(&mut q, |f| f.pts.unwrap_or(0), c);
				}
			}
			q.len()
		};
		if !video.spilled.is_empty() {
			video.replay(&shared, feed::MAX_FRAMES);
			frames = shared.video.lock().unwrap().len();
		}
		if feed::must_wait(&Fill { frames, spilled: video.spilled_bytes, lead, audio_on: aon }) {
			std::thread::sleep(Duration::from_millis(2));
			continue;
		}
		match dm.next_packet() {
			Ok(Some(Packet::Video { pts, data, .. })) => {
				shared.note_video_pts(&mut first_vpts, pts);
				if feed::spill(frames, video.spilled.len()) {
					video.spilled_bytes += data.len();
					video.spilled.push_back((pts, data));
				} else {
					video.decode(&shared, pts, &data);
				}
			}
			Ok(Some(Packet::Audio { track: t, pts, data })) if t == track => {
				let mut body = &data[..];
				if adpcm.is_none() {
					if let Some((h, n)) = parse_header(body) {
						body = &body[n..];
						adpcm = Some(AdpcmDecoder::new(h.channels as usize, h.interleave));
						*shared.audio_pts0.lock().unwrap() = Some(pts.unwrap_or(0));
					}
				}
				if let (Some(d), Some(a), false) = (&mut adpcm, &audio, shared.audio_off.load(Ordering::Relaxed)) {
					pcm.clear();
					d.push(body, &mut pcm);
					a.push(&pcm);
					if d.all_ended() {
						shared.audio_end.store(true, Ordering::Relaxed);
						a.finish();
					}
				}
			}
			Ok(Some(_)) => {}
			Ok(None) | Err(_) => {
				video.finish(&shared);
				shared.done.store(true, Ordering::Relaxed);
				return;
			}
		}
	}
}

impl Player {
	/// Starts streaming `movie` (a scene::Movie) in front-end language `lang` (settings.rs).
	pub fn start(iso: &str, movie: &crate::scene::Movie, lang: u8, audio: Option<&mut Audio>) -> Player {
		let shared = Arc::new(Shared::new(VecDeque::new(), None));
		let stream = audio.map(|a| a.open_stream(48000));
		let (s2, a2, iso) = (shared.clone(), stream.clone(), iso.to_string());
		let (lba, bytes, fps) = (movie.lba, movie.bytes, movie.fps);
		let track = track_for_language(lang);
		let worker = std::thread::spawn(move || run(iso, lba, bytes, track, fps, s2, a2));
		Player { shared, audio: stream, worker: Some(worker), started: false, wall: None, first_pts: None, current: None, finished: false, moved: None, starved: false, lat: None, log: SyncLog::open(&format!("lba {} bytes {} fps {}", movie.lba, movie.bytes, movie.fps)), knobs: Knobs::from_env(), t0: Instant::now() }
	}

	/// Clock in 90 kHz ticks of the stream's PTS timeline: what is HEARD at the speaker (the device clock less the output latency) while
	/// the audio plays, the wall clock after it.
	fn clock(&self) -> u64 {
		if let Some(a) = &self.audio {
			let t0 = self.shared.audio_pts0.lock().unwrap().or(self.first_pts).unwrap_or(0);
			return feed::heard_ticks(t0, a.played(), a.rate, self.lat.unwrap_or(0.0));
		}
		match self.wall {
			Some((t, b)) => b + (t.elapsed().as_secs_f64() * 90000.0) as u64,
			None => 0,
		}
	}

	/// Takes the output device's latest latency report into the smoothed value the clock subtracts (a device that reports none, 0, is not
	/// compensated).
	fn update_latency(&mut self) {
		let r = crate::audio::out_latency();
		if !r.is_zero() {
			self.lat = Some(feed::smooth_latency(self.lat, r));
		}
		self.shared.lat_bits.store(self.lat.unwrap_or(0.0).to_bits(), Ordering::Relaxed);
	}

	/// Closes the audio stream; the rest of the movie runs on the wall clock from where the audio clock stood, video only.
	fn leave_audio(&mut self) {
		let c = self.clock();
		if let Some(a) = self.audio.take() {
			a.close();
		}
		self.shared.audio_off.store(true, Ordering::Relaxed);
		self.wall = Some((Instant::now(), c));
	}

	/// Call once per rendered frame. Returns the new frame to show when one became due; None keeps the
	/// previous one on screen. Sets `finished` once the stream and its queues are exhausted.
	pub fn poll(&mut self) -> Option<Frame> {
		self.knobs.stall(self.t0);
		if self.audio.is_some() {
			self.update_latency();
		}
		let done = self.shared.done.load(Ordering::Relaxed);
		let (queued, front_pts) = {
			let q = self.shared.video.lock().unwrap();
			(q.len(), q.front().and_then(|f| f.pts))
		};
		if self.first_pts.is_none() {
			self.first_pts = front_pts;
		}
		if !self.started {
			let mut primed = match &self.audio {
				Some(a) => a.buffered() as f64 / a.rate as f64 >= PRIME || (done && a.buffered() > 0) || (done && self.shared.audio_pts0.lock().unwrap().is_none()),
				None => true,
			};
			let (audio_seen, span) = (self.shared.audio_pts0.lock().unwrap().is_some(), self.shared.video_span.load(Ordering::Relaxed));
			if !primed && queued > 0 && no_audio_in_stream(audio_seen, span) {
				eprintln!("fmv: no audio packet of the language track in the first {} s of the stream; the movie starts without sound", span / 90_000);
				self.leave_audio();
				primed = true;
			}
			if !(queued >= 3 || (done && queued > 0)) || !primed {
				if done && queued == 0 {
					self.finished = true;
				}
				return None;
			}
			self.started = true;
			if let Some(a) = &self.audio {
				a.set_playing(true);
			} else {
				self.wall = Some((Instant::now(), self.first_pts.unwrap_or(0)));
			}
		}
		// The device stopped pulling the stream (no output, endpoint asleep or switched): its clock would hold the movie
		// (and the game thread parked in fmv_bridge) forever. Continue on the wall clock from here, video only. A stream that is only
		// starved (the device still pulls, the decoder is behind) keeps the audio: the clock holds and the movie goes on in sync.
		if let Some(a) = self.audio.clone() {
			let p = a.played();
			// The audio is over (its END frame played out, or the stream ended) and the queue is drained: nothing more is coming, the
			// rest of the video runs on the wall clock at once (no wait for audio that will never arrive).
			let over = (done || self.shared.audio_end.load(Ordering::Relaxed)) && a.buffered() < 4;
			match watch(&mut self.moved, p, a.pulls(), over, Instant::now()) {
				Watch::Fine => self.starved = false,
				Watch::Starved => {
					if !std::mem::replace(&mut self.starved, true) {
						eprintln!("fmv: audio starved at {p} frames (decoder or disc behind); the movie waits for it");
					}
				}
				Watch::Ended => {
					eprintln!("fmv: audio track ended at {p} frames");
					self.leave_audio();
				}
				Watch::GiveUp(why) => {
					eprintln!("fmv: audio clock stalled at {p} frames ({why}); movie continues on the wall clock");
					self.leave_audio();
				}
			}
		}
		let now = self.clock();
		let mut out = None;
		let mut q = self.shared.video.lock().unwrap();
		while q.front().is_some_and(|f| f.pts.unwrap_or(0) <= now) {
			out = q.pop_front();
		}
		let left = q.len();
		drop(q);
		if let Some(l) = self.log.as_mut() {
			let (played, pulls, abuf) = self.audio.as_ref().map_or((0, 0, 0), |a| (a.played(), a.pulls(), a.buffered()));
			let mode = if self.audio.is_none() { 'w' } else if self.starved { 'p' } else { 'a' };
			let frame = out.as_ref().map(|f| (f.temporal_reference, f.coding_type, super::synclog::thumb(&f.y, f.width as usize, f.height as usize), f.repeat_first_field));
			let comp_ms = if self.audio.is_some() { self.lat.unwrap_or(0.0) } else { 0.0 };
			l.row(&Row { played, pulls, abuf, vq: left, clock: now, shown: out.as_ref().and_then(|f| f.pts), frame, dev_ms: crate::audio::out_latency().as_secs_f64() * 1000.0, comp_ms, mode });
		}
		if out.is_some() {
			self.current = out.clone();
		}
		let audio_left = self.audio.as_ref().map_or(0, |a| a.buffered());
		if done && left == 0 && audio_left < 4 {
			self.finished = true;
		}
		out
	}

}

/// MPEG-2 aspect_ratio_information: 1 square pixels, 2 4:3, 3 16:9, 4 2.21:1 (display aspect).
pub fn display_aspect(code: u8, w: u32, h: u32) -> f32 {
	match code {
		2 => 4.0 / 3.0,
		3 => 16.0 / 9.0,
		4 => 2.21,
		_ => w as f32 / h.max(1) as f32,
	}
}

impl Drop for Player {
	fn drop(&mut self) {
		self.shared.stop.store(true, Ordering::Relaxed);
		if let Some(a) = &self.audio {
			a.close();
		}
		if let Some(w) = self.worker.take() {
			let _ = w.join();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The audio clock watchdog: a starved stream (device alive) is waited for, a dead device or a very long stall gives the audio up.
	#[test]
	fn the_watchdog_waits_for_a_starved_stream_and_gives_up_on_a_dead_device() {
		let t0 = Instant::now();
		let at = |ms: u64| t0 + Duration::from_millis(ms);
		// Starved: `played` stuck at 7, the device keeps pulling (the count goes up every 100 ms).
		let mut seen = None;
		assert_eq!(watch(&mut seen, 7, 1, false, at(0)), Watch::Fine);
		assert_eq!(watch(&mut seen, 7, 2, false, at(100)), Watch::Fine);
		for k in 3..=79u64 {
			let v = watch(&mut seen, 7, k, false, at((k - 1) * 100));
			assert!(matches!(v, Watch::Fine | Watch::Starved), "starved stream at {} ms: {v:?}", (k - 1) * 100);
		}
		assert_eq!(watch(&mut seen, 7, 80, false, at(7900)), Watch::Starved, "still waiting at 7.9 s");
		// The decoder catches up: the clock moves, the audio stays.
		assert_eq!(watch(&mut seen, 9, 81, false, at(8000)), Watch::Fine);
		assert_eq!(watch(&mut seen, 9, 82, false, at(8100)), Watch::Fine);
		// A stall past STARVE_MAX gives up.
		for k in 1..=80u64 {
			assert!(matches!(watch(&mut seen, 9, 82 + k, false, at(8000 + k * 100)), Watch::Fine | Watch::Starved));
		}
		assert!(matches!(watch(&mut seen, 9, 163, false, at(16100)), Watch::GiveUp("stream starved too long")));
		// Dead device: nothing pulls, nothing plays.
		let mut seen = None;
		assert_eq!(watch(&mut seen, 5, 9, false, at(0)), Watch::Fine);
		assert_eq!(watch(&mut seen, 5, 9, false, at(900)), Watch::Starved);
		assert!(matches!(watch(&mut seen, 5, 9, false, at(1100)), Watch::GiveUp(_)));
		// A healthy stream never trips it.
		let mut seen = None;
		for k in 0..500u64 {
			assert_eq!(watch(&mut seen, k * 480, k, false, at(k * 10)), Watch::Fine);
		}
	}

	/// The normal end of the audio track is its own verdict (not a stall), whatever else is going on.
	#[test]
	fn the_end_of_the_audio_is_not_a_fault() {
		let t0 = Instant::now();
		let at = |ms: u64| t0 + Duration::from_millis(ms);
		let mut seen = None;
		assert_eq!(watch(&mut seen, 7, 1, true, at(0)), Watch::Ended);
		assert_eq!(watch(&mut seen, 7, 1, true, at(5000)), Watch::Ended, "also with a stuck clock");
	}

	/// A player on a detached mixer, with `queued` frames due far in the future, started, its audio clock at 0.
	fn rig(queued: usize) -> (Player, impl FnMut(&mut [f32])) {
		let (mut audio, mix) = Audio::offline(48000.0, 2);
		let stream = audio.open_stream(48000);
		stream.set_playing(true);
		let frame = Frame { width: 2, height: 2, y: vec![0; 4], cb: vec![0], cr: vec![0], pts: Some(90_000_000), picture_structure: 3, top_field_first: true, progressive: true, repeat_first_field: false, aspect_ratio_information: 2, frame_rate_code: 3, temporal_reference: 0, coding_type: 1 };
		let shared = Arc::new(Shared::new(VecDeque::from(vec![frame; queued]), Some(0)));
		let p = Player { shared, audio: Some(stream), worker: None, started: true, wall: None, first_pts: None, current: None, finished: false, moved: None, starved: false, lat: None, log: None, knobs: Knobs::off(), t0: Instant::now() };
		(p, mix)
	}

	/// The start verdict is the stream's: no audio packet of the track in `NO_AUDIO_SPAN` of video gives the audio up, whatever the clock
	/// says; an audio packet seen, or less video read, never does.
	#[test]
	fn the_start_gives_up_on_the_stream_evidence_only() {
		assert!(!no_audio_in_stream(false, 0), "nothing read yet (a disc that is slow to spin up)");
		assert!(!no_audio_in_stream(false, NO_AUDIO_SPAN - 1));
		assert!(no_audio_in_stream(false, NO_AUDIO_SPAN), "3 s of video, no packet of the track: it is not there");
		assert!(!no_audio_in_stream(true, 100 * NO_AUDIO_SPAN), "the track's packets arrived: wait for the buffer, however slowly it fills");
		// The worker's side: the span runs from the first stamped video packet to the latest, never back, and unstamped packets do not count.
		let shared = Shared::new(VecDeque::new(), None);
		let mut first = None;
		shared.note_video_pts(&mut first, None);
		assert_eq!((first, shared.video_span.load(Ordering::Relaxed)), (None, 0));
		shared.note_video_pts(&mut first, Some(1_000_000));
		shared.note_video_pts(&mut first, Some(1_000_000 + 45_000));
		shared.note_video_pts(&mut first, Some(1_000_000 + 20_000)); // out of order: the span does not shrink
		shared.note_video_pts(&mut first, Some(500)); // before the first: saturates
		assert_eq!((first, shared.video_span.load(Ordering::Relaxed)), (Some(1_000_000), 45_000));
	}

	/// poll(): a movie whose language track is not in the stream starts video only once the demuxer has read `NO_AUDIO_SPAN` of video (it
	/// used to stay black for good); a slow disc (a long wall time, little read, or the track's packets there) is waited for, and audio that
	/// arrives starts the movie with it.
	#[test]
	fn poll_starts_a_movie_without_audio_by_the_stream_and_waits_out_a_slow_disc() {
		let (mut p, _mix) = rig(3);
		p.started = false;
		*p.shared.audio_pts0.lock().unwrap() = None;
		// A slow read: the owner polls for longer than the old 3 s wait would have, the demuxer got through 0.5 s of video only.
		p.shared.video_span.store(45_000, Ordering::Relaxed);
		for _ in 0..4 {
			assert!(p.poll().is_none() && !p.started && p.audio.is_some(), "little read, no audio yet: waiting");
			std::thread::sleep(Duration::from_millis(20));
		}
		p.shared.video_span.store(NO_AUDIO_SPAN - 1, Ordering::Relaxed);
		assert!(p.poll().is_none() && !p.started && p.audio.is_some(), "still short of the span");
		// The span is read and not one packet of the track came with it: video only.
		p.shared.video_span.store(NO_AUDIO_SPAN, Ordering::Relaxed);
		p.poll();
		assert!(p.started && p.audio.is_none() && p.wall.is_some() && p.shared.audio_off.load(Ordering::Relaxed), "started video only");
		// The track's packets are in the stream (its first PTS is known) but the buffer fills slowly: wait, however much video was read.
		let (mut p, _mix) = rig(3);
		p.started = false;
		p.shared.video_span.store(50 * NO_AUDIO_SPAN, Ordering::Relaxed);
		assert!(p.poll().is_none() && !p.started && p.audio.is_some() && p.wall.is_none(), "the track exists: waiting for its audio");
		// Audio that arrived (0.3 s) starts the movie with the audio.
		p.audio.as_ref().unwrap().push(&vec![0i16; 2 * 14400]);
		p.poll();
		assert!(p.started && p.audio.is_some() && p.wall.is_none(), "audio primed: starts with it");
	}

	/// poll(): the audio track's end moves the movie to the wall clock; a starved stream waits for its audio, whatever the video queue holds
	/// (a full queue used to give the audio up and silence the rest of the movie).
	#[test]
	fn poll_leaves_the_audio_at_its_end_and_never_because_of_the_video_queue() {
		let pull = |mix: &mut dyn FnMut(&mut [f32])| mix(&mut [0.0f32; 96]);
		// The audio track ended (END frame decoded, nothing buffered): wall clock at once.
		let (mut p, mut mix) = rig(3);
		pull(&mut mix);
		p.shared.audio_end.store(true, Ordering::Relaxed);
		p.poll();
		assert!(p.audio.is_none() && p.wall.is_some() && p.shared.audio_off.load(Ordering::Relaxed));
		// Starved (device pulls, nothing plays): the movie waits for the audio with the video queue nearly full, full, or overfull.
		for queued in [HARD_FRAMES - 1, HARD_FRAMES, HARD_FRAMES + 20] {
			let (mut p, mut mix) = rig(queued);
			pull(&mut mix);
			p.poll();
			std::thread::sleep(STARVED + Duration::from_millis(100));
			pull(&mut mix);
			p.poll();
			assert!(p.audio.is_some() && p.starved && p.wall.is_none(), "{queued} frames queued: the audio stays");
		}
	}

	/// The clock is the device clock less the smoothed output latency: the picture follows what is heard.
	#[test]
	fn the_clock_follows_what_is_heard() {
		let (mut p, mut mix) = rig(0);
		// 4800 source frames played at 48 kHz = 0.1 s = 9000 ticks.
		p.audio.as_ref().unwrap().push(&vec![0i16; 2 * 5000]);
		let mut buf = vec![0.0f32; 2 * 4800];
		mix(buf.as_mut_slice());
		assert_eq!(p.audio.as_ref().unwrap().played(), 4800);
		assert_eq!(p.clock(), 9000);
		p.lat = Some(31.5);
		assert_eq!(p.clock(), 9000 - 2835);
		p.lat = Some(1000.0);
		assert_eq!(p.clock(), 0, "never below 0");
		// update_latency takes the device's report (0 in the test process: no change) and publishes the value for the worker.
		let (mut p, _mix) = rig(0);
		p.lat = Some(20.0);
		p.update_latency();
		assert_eq!(f64::from_bits(p.shared.lat_bits.load(Ordering::Relaxed)), 20.0);
	}
}
