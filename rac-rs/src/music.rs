//! Level music: the VAG streams listed in the level header (decomp/notes/music.md).
//!
//! The level header (the sector in the ToC's level table, 0x2434 bytes) holds 15 absolute LBAs at
//! +0x148 (RAM 0x13a628 + 4*slot, 0 = unused). Each LBA is a plain Sony `VAGp` file (0x30-byte
//! big-endian header: +0 "VAGp", +4 version 0x20, +0xc data size, +0x10 sample rate, +0x20 name)
//! followed by PS-ADPCM frames. The IOP streams them straight from disc (EE side:
//! FUN_00215660 / FUN_00215810 in the boot ELF, mode word 1 = music, 2 = speech).
//!
//! Slots come in (intro, loop) pairs: music id n plays slot n (the "Start" part, one shot) and the
//! game then queues slot n+1 (the "Loop" part, repeats). Slots 6.. are one-shot transition
//! stingers. All streams measured are 44100 Hz; no interleave or second channel was found (a mono
//! stream decodes with no discontinuity at any block boundary), so they are mono [I].
//!
//! Veldin (level 0, tutorial) has one pair: slot 0 is a 7.68 s intro, slot 1 a 122.88 s loop. The
//! game plays music id 0 when the level starts (FUN_00264da8(0, flags 1, volume 0x400 = 1.0) in
//! the level-start function, overlay OV:54220, and again on death/world restart; the pause menu only
//! pauses and resumes it, corrected 2026-10-04: was "after every pause/menu return") and never
//! switches it on Veldin: the "music zone" table that could change the id (gameplay +0x88) has no
//! entries there. So: play slot 0 once, then slot 1 forever, at full volume.

use crate::bytes::{span, u32_at, R};
use crate::iso::Disc;
use crate::level::Level;
use crate::sound::decode_adpcm;
use crate::toc;

/// Level header offset of the music LBA list.
const MUSIC_LBAS: usize = 0x148;
const SLOTS: usize = 15;
const HEADER_SIZE: usize = 0x2434;
/// Slots below this form (intro, loop) pairs; the rest are one-shot stingers.
const PAIR_SLOTS: usize = 6;

pub struct Music {
	pub rate: u32,
	/// Interleaved samples.
	pub pcm: Vec<i16>,
	/// Frame the stream repeats from when it reaches its end (None: plays once).
	pub loop_start: Option<usize>,
	/// Slot in the level header's music list (music id n = slot n, then n + 1).
	pub slot: usize,
}

/// Decode every music stream of a level, in slot order. Odd slots below 6 (the "Loop" parts) get
/// `loop_start = Some(0)`; see the module comment for how Veldin uses them.
pub fn level_music(disc: &mut Disc, level: &Level) -> R<Vec<Music>> {
	let levels = toc::read_levels(disc).map_err(|e| e.to_string())?;
	let info = levels.get(level.id).ok_or("no such level")?;
	let h = disc.read_bytes(info.header_sector, HEADER_SIZE).map_err(|e| e.to_string())?;
	let mut out = Vec::new();
	for slot in 0..SLOTS {
		let lba = u32_at(&h, MUSIC_LBAS + slot * 4)? as u64;
		if lba == 0 {
			continue;
		}
		out.push(read_vag(disc, lba, slot)?);
	}
	Ok(out)
}

pub(crate) fn read_vag(disc: &mut Disc, lba: u64, slot: usize) -> R<Music> {
	let head = disc.read_bytes(lba, 0x30).map_err(|e| e.to_string())?;
	if &head[0..4] != b"VAGp" {
		return Err(format!("music slot {slot} (LBA {lba}) is not a VAG"));
	}
	let be = |o: usize| u32::from_be_bytes([head[o], head[o + 1], head[o + 2], head[o + 3]]);
	let (size, rate) = (be(12) as usize, be(16));
	let data = disc.read_bytes(lba, 0x30 + size).map_err(|e| e.to_string())?;
	let (pcm, _) = decode_adpcm(span(&data, 0x30, size)?)?;
	let name = String::from_utf8_lossy(&head[0x20..0x30]).trim_end_matches('\0').to_string();
	let loops = slot < PAIR_SLOTS && slot % 2 == 1;
	Ok(Music { rate,  pcm, loop_start: loops.then_some(0), slot, })
}

/// What the viewer should have playing: the main stream (slot, gain 0..1, paused) and the stinger.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct MainOut {
	pub slot: usize,
	pub gain: f32,
	pub paused: bool,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Main {
	Idle,
	Playing,
	/// New intro loaded but held until the stinger's last quarter.
	Preloaded,
}

/// Cooldown between queued-request attempts (7 s, FUN_00279fa0).
const COOLDOWN: f32 = 7.0;

/// Music/stinger controller of Novalis (decomp/notes/music.md): the pending setter 0x279b60, the
/// controller 0x279fa0 and its transition states 0..5, restart 0x279bb8. Stream lengths and the
/// stinger clock are in the IOP's integer 1/1024 s units (RPC 0x31/0x32/0x33, `stream_units`); the
/// remaining-time polls and the fade arithmetic are integer as in FUN_00279fa0 / callback 0x27a8d0.
pub struct Player {
	/// Length of each slot's stream in 1/1024 s (0 = no stream).
	lens: [i32; SLOTS],
	/// Current track id (0x151708): slot `cur` is the intro, `cur + 1` the loop.
	pub cur: i32,
	/// Queued request track (0x1516f2, -1 none) and its stinger slot (0x1516f3, 0xff none).
	queued: i32,
	cue: u8,
	cooldown: f32,
	/// Transition state (0x1516f0): 0 idle, 2 fading old, 3 preload, 4 hold, 5 wait for the stinger.
	trans: u8,
	vol: f32,
	started: bool,
	main: Main,
	main_slot: usize,
	main_t: f32,
	fade: f32,
	/// Stinger slot and its elapsed time in 1/1024 s.
	sting: Option<(usize, f32)>,
	/// The quarter of the stinger's remaining time at its first poll (fade length, 1/1024 s).
	fade_len: i32,
	/// NOV-34b: music and stinger paused in place (channel words 0x8000, FUN_00279d20(0)) from the ship
	/// boarding until the landing end (FUN_00279d58).
	held: bool,
}

/// The IOP's sample count to 1/1024 s conversion (989snd FUN_0001070c / FUN_00010860):
/// `(n / rate) * 1024 + ((n % rate) << 10) / rate`. A VAG's total is `(bytes >> 4) * 28` samples.
pub fn stream_units(samples: u64, rate: u32) -> i32 {
	let rate = rate.max(1) as u64;
	((samples / rate) * 0x400 + ((samples % rate) << 10) / rate) as i32
}

impl Player {


	/// From the stream length in 1/1024 s of each slot (0 = no stream).
	pub fn from_units(l: &[i32]) -> Self {
		let mut lens = [0; SLOTS];
		for (d, s) in lens.iter_mut().zip(l) {
			*d = *s;
		}
		Player { lens, cur: -1, queued: -1, cue: 0xff, cooldown: 0.0, trans: 0, vol: 1.0, started: false, main: Main::Idle, main_slot: 0, main_t: 0.0, fade: 1.0, sting: None, fade_len: 0, held: false }
	}

	fn has(&self, slot: i32) -> bool {
		usize::try_from(slot).ok().and_then(|s| self.lens.get(s)).is_some_and(|&l| l > 0)
	}

	/// FUN_002798c0: start `cur` from its intro (needs the stream slot to exist and no stream running).
	fn start(&mut self) {
		if self.main == Main::Idle && self.has(self.cur) {
			self.main = Main::Playing;
			self.main_slot = self.cur as usize;
			self.main_t = 0.0;
			self.fade = 1.0;
		}
	}

	pub fn restart(&mut self, track: Option<i16>) {
		if self.queued != -1 {
			self.cur = self.queued;
		}
		self.queued = -1;
		self.cue = 0xff;
		self.trans = 0;
		self.sting = None;
		self.main = Main::Idle;
		if let Some(t) = track.filter(|&t| t >= 0) {
			self.cur = t as i32;
		} else if self.cur < 0 {
			self.cur = 0;
		}
		self.vol = 1.0;
		self.started = true;
		self.start();
	}

	/// FUN_00279b60: a zone's exit request. Idle streams just take the track; otherwise the request
	/// queues with its stinger slot for the controller.
	pub fn request(&mut self, track: i16, cue: u8) {
		let track = track as i32;
		if self.cur == track && self.queued == -1 {
			return;
		}
		if self.main == Main::Idle && self.sting.is_none() {
			self.cur = track;
		} else {
			self.queued = track;
			self.cue = cue;
		}
	}


	/// Advance by `dt` seconds (call only while the game runs; pausing just stops calling).
	pub fn update(&mut self, dt: f32) {
		self.cooldown = (self.cooldown - dt).max(0.0);
		// Held: both streams frozen, no intro->loop hand-over and no transition step (FUN_00279fa0 gates).
		if self.held {
			return;
		}
		let dt = dt * 1024.0;
		if let Some((slot, t)) = &mut self.sting {
			*t += dt;
			if *t as i32 >= self.lens[*slot] {
				self.sting = None;
			}
		}
		if self.main == Main::Playing {
			self.main_t += dt;
			// Intro done: the loop slot follows (FUN_00279998); a loop slot repeats on its own.
			if self.cur >= 0 && self.main_slot == self.cur as usize && self.cur % 2 == 0 && (self.cur as usize) < PAIR_SLOTS && self.main_t as i32 >= self.lens[self.main_slot] {
				if self.has(self.cur + 1) {
					self.main_slot = self.cur as usize + 1;
					self.main_t = 0.0;
				} else {
					self.main = Main::Idle;
				}
			}
		}
		if self.main == Main::Idle && self.queued == -1 && self.started && self.trans == 0 && self.sting.is_none() {
			self.start();
		}
		// Queued request: after the cooldown, skip it if already current, else start its stinger.
		if self.cooldown <= 0.0 && self.queued != -1 && self.trans == 0 {
			if self.cur == self.queued {
				self.queued = -1;
			} else {
				let cue = self.cue as usize;
				let ok = self.cue != 0xff && self.sting.is_none() && cue < SLOTS && self.lens[cue] > 0;
				if self.cue == 0xff || ok {
					if ok {
						self.sting = Some((cue, 0.0));
						// Remaining-time callback 0x27a8d0: the first poll's remaining time (the whole
						// stream; the CD start latency is not modelled [P]) quartered, integer.
						self.fade_len = self.lens[cue] / 4;
						self.trans = 2;
					}
					self.cooldown = COOLDOWN;
				}
			}
		}
		match self.trans {
			2 => {
				// gain = 1024 * (q - elapsed) / q in integer units; the state ends below 1 (FUN_00279fa0).
				let d = self.sting.map_or(self.fade_len, |s| s.1 as i32);
				let gain = if self.fade_len > 0 { 1024 * (self.fade_len - d) / self.fade_len } else { 0 };
				self.fade = gain as f32 / 1024.0;
				if gain < 1 || self.sting.is_none() || self.main == Main::Idle {
					self.main = Main::Idle;
					self.trans = 3;
				}
			}
			3 => {
				// FUN_002797e8: the new intro, loaded paused.
				if self.main == Main::Idle {
					if self.has(self.queued) {
						self.cur = self.queued;
						self.main = Main::Preloaded;
						self.main_slot = self.cur as usize;
						self.main_t = 0.0;
						self.fade = 1.0;
					}
					self.queued = -1;
					self.cue = 0xff;
					self.trans = 4;
				}
			}
			4 => {
				let last_quarter = self.sting.map_or(true, |(slot, t)| self.lens[slot] - (t as i32) < self.fade_len);
				// Resume (rem < q, or the stinger stopped); with nothing preloaded the state still
				// advances to 5 (decomp/notes/music.md, FUN_00279fa0).
				if last_quarter {
					if self.main == Main::Preloaded {
						self.main = Main::Playing;
					}
					self.trans = 5;
				}
			}
			5 => {
				if self.sting.is_none() {
					self.trans = 0;
				}
			}
			_ => {}
		}
	}

	pub fn main_out(&self) -> Option<MainOut> {
		match self.main {
			Main::Idle => None,
			m => Some(MainOut { slot: self.main_slot, gain: self.vol * self.fade, paused: m == Main::Preloaded || self.held }),
		}
	}

	pub fn held(&self) -> bool {
		self.held
	}

	pub fn stinger_slot(&self) -> Option<usize> {
		self.sting.map(|s| s.0)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// NOV-34b

	// SAV-1c
	use std::path::{Path, PathBuf};

	const ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");
	const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test_out/music");

	fn write_wav(path: &Path, rate: u32, pcm: &[i16]) {
		let mut b = Vec::new();
		b.extend_from_slice(b"RIFF");
		b.extend_from_slice(&(36 + pcm.len() as u32 * 2).to_le_bytes());
		b.extend_from_slice(b"WAVEfmt ");
		for v in [16u32, 0x0001_0001, rate, rate * 2, 0x0010_0002] {
			b.extend_from_slice(&v.to_le_bytes());
		}
		b.extend_from_slice(b"data");
		b.extend_from_slice(&(pcm.len() as u32 * 2).to_le_bytes());
		for s in pcm {
			b.extend_from_slice(&s.to_le_bytes());
		}
		std::fs::write(path, b).unwrap();
	}

	/// Magnitude spectrum (Hann window) of the first `n` (a power of two) samples.
	fn spectrum(x: &[i16], n: usize) -> Vec<f64> {
		let tau = 2.0 * std::f64::consts::PI;
		let mut re: Vec<f64> = (0..n).map(|i| x[i] as f64 * (0.5 - 0.5 * (tau * i as f64 / n as f64).cos())).collect();
		let mut im = vec![0.0; n];
		let mut j = 0;
		for i in 1..n {
			let mut bit = n >> 1;
			while j & bit != 0 {
				j ^= bit;
				bit >>= 1;
			}
			j |= bit;
			if i < j {
				re.swap(i, j);
			}
		}
		let mut len = 2;
		while len <= n {
			let ang = -tau / len as f64;
			for s in (0..n).step_by(len) {
				for k in 0..len / 2 {
					let (wr, wi) = ((ang * k as f64).cos(), (ang * k as f64).sin());
					let (a, b) = (s + k, s + k + len / 2);
					let (tr, ti) = (re[b] * wr - im[b] * wi, re[b] * wi + im[b] * wr);
					re[b] = re[a] - tr;
					im[b] = im[a] - ti;
					re[a] += tr;
					im[a] += ti;
				}
			}
			len <<= 1;
		}
		(0..n / 2).map(|i| re[i].hypot(im[i])).collect()
	}

	/// (spectral flatness, zero-crossing rate, lag-1 autocorrelation, power share below 1/8 Nyquist)
	/// of the first 8192 samples.
	fn stats(x: &[i16]) -> (f64, f64, f64, f64) {
		let n = 8192;
		let sp: Vec<f64> = spectrum(x, n).iter().skip(1).map(|m| m * m + 1e-9).collect();
		let flat = (sp.iter().map(|v| v.ln()).sum::<f64>() / sp.len() as f64).exp() / (sp.iter().sum::<f64>() / sp.len() as f64);
		let low = sp[..sp.len() / 8].iter().sum::<f64>() / sp.iter().sum::<f64>();
		let zcr = x[..n].windows(2).filter(|w| (w[0] < 0) != (w[1] < 0)).count() as f64 / n as f64;
		let m = x[..n].iter().map(|&v| v as f64).sum::<f64>() / n as f64;
		let (mut num, mut den) = (0.0, 0.0);
		for i in 0..n - 1 {
			num += (x[i] as f64 - m) * (x[i + 1] as f64 - m);
			den += (x[i] as f64 - m).powi(2);
		}
		(flat, zcr, num / den, low)
	}

	#[test]
	fn stream_unit_formula() {
		// 44100 samples at 44100 Hz are exactly 1024 units; half a second is 512.
		assert_eq!(stream_units(44100, 44100), 1024);
		assert_eq!(stream_units(22050, 44100), 512);
		assert_eq!(stream_units(44100 * 3 + 100, 44100), 3 * 1024 + 2);
	}

}
