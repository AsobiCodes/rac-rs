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

struct Shared {
	video: Mutex<VecDeque<Frame>>,
	/// PTS of the first audio packet of the track (90 kHz), once seen.
	audio_pts0: Mutex<Option<u64>>,
	done: AtomicBool,
	stop: AtomicBool,
	/// The output device stopped pulling the stream: decode video only, the wall clock paces the movie.
	audio_off: AtomicBool,
}

pub struct Player {
	shared: Arc<Shared>,
	audio: Option<Arc<StreamSource>>,
	worker: Option<std::thread::JoinHandle<()>>,
	started: bool,
	paused: bool,
	/// Wall clock fallback: (instant the clock last ran from, 90 kHz ticks at that instant).
	wall: Option<(Instant, u64)>,
	first_pts: Option<u64>,
	current: Option<Frame>,
	pub finished: bool,
	/// (played, when it last moved): the audio clock watchdog.
	moved: Option<(u64, Instant)>,
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
		let shared = Arc::new(Shared { video: Mutex::new(VecDeque::new()), audio_pts0: Mutex::new(None), done: AtomicBool::new(false), stop: AtomicBool::new(false), audio_off: AtomicBool::new(false) });
		let stream = audio.map(|a| a.open_stream(48000));
		let (s2, a2, iso) = (shared.clone(), stream.clone(), iso.to_string());
		let (lba, bytes, fps) = (movie.lba, movie.bytes, movie.fps);
		let track = track_for_language(lang);
		let worker = std::thread::spawn(move || run(iso, lba, bytes, track, fps, s2, a2));
		Player { shared, audio: stream, worker: Some(worker), started: false, paused: false, wall: None, first_pts: None, current: None, finished: false, moved: None }
	}

	/// Clock in 90 kHz ticks of the stream's PTS timeline.
	fn clock(&self) -> u64 {
		if let Some(a) = &self.audio {
			let t0 = self.shared.audio_pts0.lock().unwrap().or(self.first_pts).unwrap_or(0);
			return t0 + a.played() * 90000 / a.rate as u64;
		}
		match self.wall {
			Some((t, b)) if !self.paused => b + (t.elapsed().as_secs_f64() * 90000.0) as u64,
			Some((_, b)) => b,
			None => 0,
		}
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
		if self.paused {
			return None;
		}
		// The device stopped pulling the stream (no output, endpoint asleep or switched): its clock would hold the movie
		// (and the game thread parked in fmv_bridge) forever. Continue on the wall clock from here, video only.
		if let Some(a) = &self.audio {
			let p = a.played();
			match self.moved {
				Some((q, t)) if q == p && t.elapsed() > Duration::from_millis(500) => {
					let c = self.clock();
					eprintln!("fmv: audio clock stalled at {p} frames; movie continues on the wall clock");
					a.close();
					self.shared.audio_off.store(true, Ordering::Relaxed);
					self.audio = None;
					self.wall = Some((Instant::now(), c));
				}
				Some((q, _)) if q == p => {}
				_ => self.moved = Some((p, Instant::now())),
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
