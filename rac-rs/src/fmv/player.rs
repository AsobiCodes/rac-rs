//! Streaming FMV player (decomp/notes/cutscenes.md): a worker thread demuxes the PSS from the disc,
//! decodes MPEG-2 video and the language's ADPCM track into bounded queues; the owner polls once per
//! rendered frame and gets the frame due at the audio clock (90 kHz PTS), or at the wall clock when
//! there is no audio device. Dropping the player stops the worker and closes the audio stream.

use super::adpcm_stream::{parse_header, AdpcmDecoder};
use super::mpeg2::{Decoder, Frame};
use super::pss::{track_for_language, DiscReader, Packet, PssDemuxer};
use crate::audio::{Audio, StreamSource};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Decoded frames held ahead of the clock (0.5 s at 25 fps).
const MAX_FRAMES: usize = 12;
/// Hard cap while audio is starved, so the demux can reach the audio packets.
const HARD_FRAMES: usize = 90;
/// Seconds of audio the worker keeps queued.
const MAX_AUDIO: f64 = 2.0;
/// Seconds of audio queued before the clock starts.
const PRIME: f64 = 0.25;
/// The audio clock holds (nothing played) this long while the output device keeps pulling: the stream is starved, the decoder or the disc
/// is behind. The movie waits for it, video and audio held together, and goes on in sync (v0.4.1, GitHub #7: it used to give the audio up
/// after 0.5 s here and play the rest of the movie silent).
const STARVED: Duration = Duration::from_millis(250);
/// ... and gives the audio up after this long (something is broken: the disc or the worker is gone).
const STARVE_MAX: Duration = Duration::from_secs(8);
/// An output device that pulled nothing for this long is gone or asleep (its clock would hold the movie, and the game thread parked in
/// fmv_bridge, forever): the movie continues on the wall clock, video only.
const DEAD_AFTER: Duration = Duration::from_millis(1000);

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

/// One watchdog step: `played` / `pulls` are `StreamSource::played` / `pulls` now, `queued` the decoded video frames waiting, `over` that
/// the audio track has ended and its queue is drained. `played` standing still alone is a starved stream and is waited out; it is the
/// device that stopped (no pulls) or a stall past `STARVE_MAX` that gives the audio up, and so is a starved stream with the video queue
/// full (`HARD_FRAMES`): the worker sleeps there and the clock holds the movie, so the audio packets could never be reached.
fn watch(seen: &mut Option<Seen>, played: u64, pulls: u64, queued: usize, over: bool, now: Instant) -> Watch {
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
		if queued >= HARD_FRAMES {
			Watch::GiveUp("video queue full while the audio is starved")
		} else {
			Watch::Starved
		}
	} else {
		Watch::Fine
	}
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
}

fn run(iso: String, lba: u32, bytes: u64, track: u8, fps: f32, shared: Arc<Shared>, audio: Option<Arc<StreamSource>>) {
	let Ok(mut disc) = crate::iso::Disc::open(std::path::Path::new(&iso)) else {
		shared.done.store(true, Ordering::Relaxed);
		return;
	};
	let mut dm = PssDemuxer::new(DiscReader::new(&mut disc, lba, bytes));
	let mut dec = Decoder::new();
	let mut adpcm: Option<AdpcmDecoder> = None;
	let mut pcm = Vec::new();
	let frame_ticks = (90000.0 / fps.max(1.0)) as u64;
	let mut last_pts: Option<u64> = None;
	let mut drain = |dec: &mut Decoder| {
		while let Some(mut f) = dec.next_frame() {
			let p = f.pts.or(last_pts.map(|p| p + frame_ticks)).unwrap_or(0);
			f.pts = Some(p);
			last_pts = Some(p);
			shared.video.lock().unwrap().push_back(f);
		}
	};
	loop {
		if shared.stop.load(Ordering::Relaxed) {
			return;
		}
		// Throttle on the video queue (and the audio lead).
		let frames = shared.video.lock().unwrap().len();
		let aon = audio.is_some() && !shared.audio_off.load(Ordering::Relaxed);
		let lead = if aon { audio.as_ref().map_or(0.0, |a| a.buffered() as f64 / a.rate as f64) } else { 0.0 };
		let starved = aon && lead < PRIME;
		if (frames >= MAX_FRAMES && !starved) || frames >= HARD_FRAMES || lead > MAX_AUDIO {
			std::thread::sleep(Duration::from_millis(2));
			continue;
		}
		match dm.next_packet() {
			Ok(Some(Packet::Video { pts, data, .. })) => {
				dec.push(&data, pts);
				drain(&mut dec);
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
						}
				}
			}
			Ok(Some(_)) => {}
			Ok(None) | Err(_) => {
				dec.flush();
				drain(&mut dec);
				shared.done.store(true, Ordering::Relaxed);
				return;
			}
		}
	}
}

impl Player {
	/// Starts streaming `movie` (a scene::Movie) in front-end language `lang` (settings.rs).
	pub fn start(iso: &str, movie: &crate::scene::Movie, lang: u8, audio: Option<&mut Audio>) -> Player {
		let shared = Arc::new(Shared { video: Mutex::new(VecDeque::new()), audio_pts0: Mutex::new(None), done: AtomicBool::new(false), stop: AtomicBool::new(false), audio_off: AtomicBool::new(false), audio_end: AtomicBool::new(false) });
		let stream = audio.map(|a| a.open_stream(48000));
		let (s2, a2, iso) = (shared.clone(), stream.clone(), iso.to_string());
		let (lba, bytes, fps) = (movie.lba, movie.bytes, movie.fps);
		let track = track_for_language(lang);
		let worker = std::thread::spawn(move || run(iso, lba, bytes, track, fps, s2, a2));
		Player { shared, audio: stream, worker: Some(worker), started: false, wall: None, first_pts: None, current: None, finished: false, moved: None, starved: false }
	}

	/// Clock in 90 kHz ticks of the stream's PTS timeline.
	fn clock(&self) -> u64 {
		if let Some(a) = &self.audio {
			let t0 = self.shared.audio_pts0.lock().unwrap().or(self.first_pts).unwrap_or(0);
			return t0 + a.played() * 90000 / a.rate as u64;
		}
		match self.wall {
			Some((t, b)) => b + (t.elapsed().as_secs_f64() * 90000.0) as u64,
			None => 0,
		}
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
		let done = self.shared.done.load(Ordering::Relaxed);
		let (queued, front_pts) = {
			let q = self.shared.video.lock().unwrap();
			(q.len(), q.front().and_then(|f| f.pts))
		};
		if self.first_pts.is_none() {
			self.first_pts = front_pts;
		}
		if !self.started {
			let primed = match &self.audio {
				Some(a) => a.buffered() as f64 / a.rate as f64 >= PRIME || (done && a.buffered() > 0) || (done && self.shared.audio_pts0.lock().unwrap().is_none()),
				None => true,
			};
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
			match watch(&mut self.moved, p, a.pulls(), queued, over, Instant::now()) {
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
		assert_eq!(watch(&mut seen, 7, 1, 0, false, at(0)), Watch::Fine);
		assert_eq!(watch(&mut seen, 7, 2, 0, false, at(100)), Watch::Fine);
		for k in 3..=79u64 {
			let v = watch(&mut seen, 7, k, 0, false, at((k - 1) * 100));
			assert!(matches!(v, Watch::Fine | Watch::Starved), "starved stream at {} ms: {v:?}", (k - 1) * 100);
		}
		assert_eq!(watch(&mut seen, 7, 80, 0, false, at(7900)), Watch::Starved, "still waiting at 7.9 s");
		// The decoder catches up: the clock moves, the audio stays.
		assert_eq!(watch(&mut seen, 9, 81, 0, false, at(8000)), Watch::Fine);
		assert_eq!(watch(&mut seen, 9, 82, 0, false, at(8100)), Watch::Fine);
		// A stall past STARVE_MAX gives up.
		for k in 1..=80u64 {
			assert!(matches!(watch(&mut seen, 9, 82 + k, 0, false, at(8000 + k * 100)), Watch::Fine | Watch::Starved));
		}
		assert!(matches!(watch(&mut seen, 9, 163, 0, false, at(16100)), Watch::GiveUp("stream starved too long")));
		// Dead device: nothing pulls, nothing plays.
		let mut seen = None;
		assert_eq!(watch(&mut seen, 5, 9, 0, false, at(0)), Watch::Fine);
		assert_eq!(watch(&mut seen, 5, 9, 0, false, at(900)), Watch::Starved);
		assert!(matches!(watch(&mut seen, 5, 9, 0, false, at(1100)), Watch::GiveUp(_)));
		// A healthy stream never trips it.
		let mut seen = None;
		for k in 0..500u64 {
			assert_eq!(watch(&mut seen, k * 480, k, 0, false, at(k * 10)), Watch::Fine);
		}
	}

	/// The normal end of the audio track is its own verdict (not a stall), whatever else is going on; a starved stream whose video queue is
	/// full can never be healed (the worker sleeps at `HARD_FRAMES`, the clock holds the queue) and gives the audio up at once.
	#[test]
	fn the_end_of_the_audio_is_not_a_fault_and_a_full_queue_ends_a_starve() {
		let t0 = Instant::now();
		let at = |ms: u64| t0 + Duration::from_millis(ms);
		let mut seen = None;
		assert_eq!(watch(&mut seen, 7, 1, 0, true, at(0)), Watch::Ended);
		assert_eq!(watch(&mut seen, 7, 1, 90, true, at(5000)), Watch::Ended, "also with a stuck clock and a full queue");
		for queued in [0, HARD_FRAMES - 1] {
			let mut seen = None;
			assert_eq!(watch(&mut seen, 7, 1, queued, false, at(0)), Watch::Fine);
			assert_eq!(watch(&mut seen, 7, 2, queued, false, at(300)), Watch::Starved, "{queued} frames queued: wait");
		}
		let mut seen = None;
		assert_eq!(watch(&mut seen, 7, 1, HARD_FRAMES, false, at(0)), Watch::Fine, "not starved yet");
		assert!(matches!(watch(&mut seen, 7, 2, HARD_FRAMES, false, at(300)), Watch::GiveUp(_)));
	}

	/// A player on a detached mixer, with `queued` frames due far in the future, started, its audio clock at 0.
	fn rig(queued: usize) -> (Player, impl FnMut(&mut [f32])) {
		let (mut audio, mix) = Audio::offline(48000.0, 2);
		let stream = audio.open_stream(48000);
		stream.set_playing(true);
		let frame = Frame { width: 2, height: 2, y: vec![0; 4], cb: vec![0], cr: vec![0], pts: Some(90_000_000), picture_structure: 3, top_field_first: true, progressive: true, repeat_first_field: false, aspect_ratio_information: 2, frame_rate_code: 3, temporal_reference: 0, coding_type: 1 };
		let shared = Arc::new(Shared { video: Mutex::new(VecDeque::from(vec![frame; queued])), audio_pts0: Mutex::new(Some(0)), done: AtomicBool::new(false), stop: AtomicBool::new(false), audio_off: AtomicBool::new(false), audio_end: AtomicBool::new(false) });
		let p = Player { shared, audio: Some(stream), worker: None, started: true, wall: None, first_pts: None, current: None, finished: false, moved: None, starved: false };
		(p, mix)
	}

	/// poll(): the audio track's end moves the movie to the wall clock; a starved stream waits while the video queue has room and gives up at once when it is full.
	#[test]
	fn poll_leaves_the_audio_at_its_end_and_when_a_starve_cannot_heal() {
		let pull = |mix: &mut dyn FnMut(&mut [f32])| mix(&mut [0.0f32; 96]);
		// The audio track ended (END frame decoded, nothing buffered): wall clock at once.
		let (mut p, mut mix) = rig(3);
		pull(&mut mix);
		p.shared.audio_end.store(true, Ordering::Relaxed);
		p.poll();
		assert!(p.audio.is_none() && p.wall.is_some() && p.shared.audio_off.load(Ordering::Relaxed));
		// Starved (device pulls, nothing plays) with room in the queue: the movie waits for the audio.
		let (mut p, mut mix) = rig(HARD_FRAMES - 1);
		pull(&mut mix);
		p.poll();
		std::thread::sleep(STARVED + Duration::from_millis(100));
		pull(&mut mix);
		p.poll();
		assert!(p.audio.is_some() && p.starved, "waits while the queue has room");
		// The same with the video queue full: the audio is given up at once, no 8 s freeze.
		let (mut p, mut mix) = rig(HARD_FRAMES);
		pull(&mut mix);
		p.poll();
		std::thread::sleep(STARVED + Duration::from_millis(100));
		pull(&mut mix);
		p.poll();
		assert!(p.audio.is_none() && p.wall.is_some(), "full queue: wall clock");
	}
}
