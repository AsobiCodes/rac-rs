//! Sound output: a small software mixer on the default output device. Clips are mono 16-bit PCM
//! at their own rate (the game's samples), resampled linearly to the device rate.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};

/// [PC] Diagnostics counters (RAC_AUDIO_STATS, `Probe`), read by the probe thread; plain relaxed atomics.
pub static ST_STREAM_UNDER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static ST_STREAM_OPEN: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static ST_SPU_PUSH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static ST_SPU_REANCHOR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static ST_SPU_LATE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Stream declick fades started (gain left 1.0), by cause, and output frames mixed with a gain below 1.0.
pub static ST_FADE_UNDER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static ST_FADE_PAUSE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static ST_FADE_CLOSE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static ST_FADE_IDLE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub static ST_FADE_FRAMES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// [PC] RAC_AUDIO_STATS=<prefix>: records every device callback (time, size, mix time, the device's own callback and playback
/// timestamps, the counters above) to <prefix>.csv and the exact samples handed to the device to <prefix>.f32 (interleaved f32
/// LE, before the test mute), written by a side thread. Off unless the variable is set.
struct Probe {
	tx: std::sync::mpsc::Sender<(String, Vec<f32>)>,
	t0: std::time::Instant,
	epoch: Option<cpal::StreamInstant>,
	n: u64,
}

impl Probe {
	fn from_env(rate: f64, channels: usize) -> Option<Probe> {
		use std::io::Write;
		let prefix = std::env::var("RAC_AUDIO_STATS").ok()?;
		let (tx, rx) = std::sync::mpsc::channel::<(String, Vec<f32>)>();
		std::thread::spawn(move || {
			let (Ok(csv), Ok(raw)) = (std::fs::File::create(format!("{prefix}.csv")), std::fs::File::create(format!("{prefix}.f32"))) else { return };
			let (mut csv, mut raw) = (std::io::BufWriter::new(csv), std::io::BufWriter::new(raw));
			let _ = writeln!(csv, "# rate {rate} channels {channels}");
			let _ = writeln!(csv, "n,t_ns,frames,mix_ns,cb_ns,play_ns,stream_under,stream_open,spu_push,spu_reanchor,spu_late,fade_under,fade_pause,fade_close,fade_idle,fade_frames");
			let mut k = 0u64;
			for (line, pcm) in rx {
				let _ = writeln!(csv, "{line}");
				let _ = raw.write_all(&pcm.iter().flat_map(|x| x.to_le_bytes()).collect::<Vec<u8>>());
				k += 1;
				if k % 50 == 0 {
					let _ = csv.flush();
					let _ = raw.flush();
				}
			}
		});
		Some(Probe { tx, t0: std::time::Instant::now(), epoch: None, n: 0 })
	}

	fn record(&mut self, data: &[f32], channels: usize, start: std::time::Instant, mix_ns: u64, info: &cpal::OutputCallbackInfo) {
		use std::sync::atomic::Ordering::Relaxed;
		let ts = info.timestamp();
		let epoch = *self.epoch.get_or_insert(ts.callback);
		let ns = |t: cpal::StreamInstant| (t.as_nanos() as i128 - epoch.as_nanos() as i128) as i64;
		let line = format!(
			"{},{},{},{},{},{},{},{},{},{},{},{},{},{},{},{}",
			self.n,
			start.duration_since(self.t0).as_nanos(),
			data.len() / channels.max(1),
			mix_ns,
			ns(ts.callback),
			ns(ts.playback),
			ST_STREAM_UNDER.load(Relaxed),
			ST_STREAM_OPEN.load(Relaxed),
			ST_SPU_PUSH.load(Relaxed),
			ST_SPU_REANCHOR.load(Relaxed),
			ST_SPU_LATE.load(Relaxed),
			ST_FADE_UNDER.load(Relaxed),
			ST_FADE_PAUSE.load(Relaxed),
			ST_FADE_CLOSE.load(Relaxed),
			ST_FADE_IDLE.load(Relaxed),
			ST_FADE_FRAMES.load(Relaxed)
		);
		self.n += 1;
		let _ = self.tx.send((line, data.to_vec()));
	}
}

#[derive(Clone)]
pub struct Clip {
	pub rate: u32,
	pub pcm: Arc<Vec<i16>>,
	/// Sample index the clip loops back to, if it loops.
	pub loop_start: Option<usize>,
	/// Feeds the reverb unit (the tone's send flag; music, speech and help-voice streams are dry: play-stream flags 0x20/0x21 have bit 1 clear [C], sound.md).
	pub reverb: bool,
}

/// SPU2 reverb presets, libsd modes 1..=9 (SD_REV_MODE_ROOM, STUDIO_A/B/C, HALL, SPACE, ECHO, DELAY,
/// PIPE): (work area size in 8-byte units, the 32 effect registers in SPU2 order: dAPF1, dAPF2, vIIR,
/// vCOMB1..4, vWALL, vAPF1, vAPF2, then the buffer addresses mLSAME mRSAME mLCOMB1 mRCOMB1 mLCOMB2
/// mRCOMB2 dLSAME dRSAME mLDIFF mRDIFF mLCOMB3 mRCOMB3 mLCOMB4 mRCOMB4 dLDIFF dRDIFF mLAPF1 mRAPF1
/// mLAPF2 mRAPF2, then vLIN, vRIN). The sound library keeps the same table (the test
/// `reverb_presets_match_libsd` compares it with the IOP image when it is extracted).
pub(crate) const PRESETS: [(usize, [u16; 32]); 9] = [
	// 1 ROOM
	(0x4d8, [0x007d, 0x005b, 0x6d80, 0x54b8, 0xbed0, 0x0000, 0x0000, 0xba80, 0x5800, 0x5300, 0x04d6, 0x0333, 0x03f0, 0x0227, 0x0374, 0x01ef, 0x0336, 0x01b7, 0x0335, 0x01b6, 0x0334, 0x01b5, 0x0334, 0x01b5, 0x0334, 0x01b5, 0x01b4, 0x0136, 0x00b8, 0x005c, 0x8000, 0x8000]),
	// 2 STUDIO_A
	(0x3e8, [0x0033, 0x0025, 0x70f0, 0x4fa8, 0xbce0, 0x4410, 0xc0f0, 0x9c00, 0x5280, 0x4ec0, 0x03e4, 0x031b, 0x03a4, 0x02af, 0x0372, 0x0266, 0x031c, 0x025d, 0x025c, 0x018e, 0x022f, 0x0135, 0x01d2, 0x00b7, 0x018f, 0x00b5, 0x00b4, 0x0080, 0x004c, 0x0026, 0x8000, 0x8000]),
	// 3 STUDIO_B
	(0x908, [0x00b1, 0x007f, 0x70f0, 0x4fa8, 0xbce0, 0x4510, 0xbef0, 0xb4c0, 0x5280, 0x4ec0, 0x0904, 0x076b, 0x0824, 0x065f, 0x07a2, 0x0616, 0x076c, 0x05ed, 0x05ec, 0x042e, 0x050f, 0x0305, 0x0462, 0x02b7, 0x042f, 0x0265, 0x0264, 0x01b2, 0x0100, 0x0080, 0x8000, 0x8000]),
	// 4 STUDIO_C
	(0xdfc, [0x00e3, 0x00a9, 0x6f60, 0x4fa8, 0xbce0, 0x4510, 0xbef0, 0xa680, 0x5680, 0x52c0, 0x0dfb, 0x0b58, 0x0d09, 0x0a3c, 0x0bd9, 0x0973, 0x0b59, 0x08da, 0x08d9, 0x05e9, 0x07ec, 0x04b0, 0x06ef, 0x03d2, 0x05ea, 0x031d, 0x031c, 0x0238, 0x0154, 0x00aa, 0x8000, 0x8000]),
	// 5 HALL
	(0x15bc, [0x01a5, 0x0139, 0x6000, 0x5000, 0x4c00, 0xb800, 0xbc00, 0xc000, 0x6000, 0x5c00, 0x15ba, 0x11bb, 0x14c2, 0x10bd, 0x11bc, 0x0dc1, 0x11c0, 0x0dc3, 0x0dc0, 0x09c1, 0x0bc4, 0x07c1, 0x0a00, 0x06cd, 0x09c2, 0x05c1, 0x05c0, 0x041a, 0x0274, 0x013a, 0x8000, 0x8000]),
	// 6 SPACE
	(0x1ed8, [0x033d, 0x0231, 0x7e00, 0x5000, 0xb400, 0xb000, 0x4c00, 0xb000, 0x6000, 0x5400, 0x1ed6, 0x1a31, 0x1d14, 0x183b, 0x1bc2, 0x16b2, 0x1a32, 0x15ef, 0x15ee, 0x1055, 0x1334, 0x0f2d, 0x11f6, 0x0c5d, 0x1056, 0x0ae1, 0x0ae0, 0x07a2, 0x0464, 0x0232, 0x8000, 0x8000]),
	// 7 ECHO
	(0x3008, [0x0003, 0x0003, 0x7fff, 0x7fff, 0x0000, 0x0000, 0x0000, 0x8100, 0x0000, 0x0000, 0x1ffd, 0x0ffd, 0x1009, 0x0009, 0x0000, 0x0000, 0x1009, 0x0009, 0x1fff, 0x1fff, 0x1ffe, 0x1ffe, 0x1ffe, 0x1ffe, 0x1ffe, 0x1ffe, 0x1008, 0x1004, 0x0008, 0x0004, 0x8000, 0x8000]),
	// 8 DELAY
	(0x3008, [0x0003, 0x0003, 0x7fff, 0x7fff, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x0000, 0x1ffd, 0x0ffd, 0x1009, 0x0009, 0x0000, 0x0000, 0x1009, 0x0009, 0x1fff, 0x1fff, 0x1ffe, 0x1ffe, 0x1ffe, 0x1ffe, 0x1ffe, 0x1ffe, 0x1008, 0x1004, 0x0008, 0x0004, 0x8000, 0x8000]),
	// 9 PIPE
	(0x780, [0x0017, 0x0013, 0x70f0, 0x4fa8, 0xbce0, 0x4510, 0xbef0, 0x8500, 0x5f80, 0x54c0, 0x0371, 0x02af, 0x02e5, 0x01df, 0x02b0, 0x01d7, 0x0358, 0x026a, 0x01d6, 0x011e, 0x012d, 0x00b1, 0x011f, 0x0059, 0x01a0, 0x00e3, 0x0058, 0x0040, 0x0028, 0x0014, 0x8000, 0x8000]),
];

/// Reverb effect rate: the SPU2 effect unit runs at half the 48 kHz output [I].
const REVERB_RATE: f64 = 24000.0;

/// Software model of the SPU2 reverb network (the sound IOP selects a libsd mode, depth and two
/// options; ambience.md reverb section). Input is the mix of the voices that send to it.
struct ReverbUnit {
	mode: u8,
	/// Output depth (EVOL) as a fraction of 0x8000.
	depth: f32,
	regs: [u16; 32],
	buf: Vec<f32>,
	cur: usize,
	phase: f64,
	acc: (f32, f32),
	acc_n: u32,
	prev: (f32, f32),
	next: (f32, f32),
}

impl ReverbUnit {
	/// Mixes one output frame: collects the send, runs the effect at its own rate and returns the
	/// (linearly interpolated) wet output.
	fn frame(&mut self, input: (f32, f32), out_rate: f64) -> (f32, f32) {
		self.acc.0 += input.0;
		self.acc.1 += input.1;
		self.acc_n += 1;
		self.phase += REVERB_RATE / out_rate;
		if self.phase >= 1.0 {
			self.phase -= 1.0;
			let k = 1.0 / self.acc_n as f32;
			let x = (self.acc.0 * k, self.acc.1 * k);
			self.acc = (0.0, 0.0);
			self.acc_n = 0;
			self.prev = self.next;
			self.next = self.tick(x);
		}
		let t = self.phase as f32;
		let (a, b) = (self.prev, self.next);
		((a.0 + (b.0 - a.0) * t) * self.depth, (a.1 + (b.1 - a.1) * t) * self.depth)
	}

	fn new() -> ReverbUnit {
		ReverbUnit { mode: 0, depth: 0.0, regs: [0; 32], buf: Vec::new(), cur: 0, phase: 0.0, acc: (0.0, 0.0), acc_n: 0, prev: (0.0, 0.0), next: (0.0, 0.0) }
	}

	/// libsd mode (0 = off, 1..=9 = `PRESETS`) and depth (the authored amount, 0..=0x7fff).
	fn set(&mut self, mode: u8, amount: i32) {
		if mode != self.mode {
			self.mode = mode;
			match PRESETS.get((mode as usize).wrapping_sub(1)) {
				Some(&(units, regs)) => {
					self.regs = regs;
					self.buf = vec![0.0; units * 4];
				}
				None => {
					self.mode = 0;
					self.buf = Vec::new();
				}
			}
			self.cur = 0;
			self.acc = (0.0, 0.0);
			self.acc_n = 0;
			self.prev = (0.0, 0.0);
			self.next = (0.0, 0.0);
		}
		self.depth = amount.clamp(0, 0x7fff) as f32 / 32768.0;
	}

	fn active(&self) -> bool {
		self.mode != 0 && self.depth > 0.0
	}

	/// One effect tick (the classic SPU reverb network: two same-side and two cross-side
	/// feedback delay lines, four comb taps and two all-pass filters per channel).
	fn tick(&mut self, input: (f32, f32)) -> (f32, f32) {
		let n = self.buf.len();
		let v = |r: &[u16; 32], i: usize| r[i] as i16 as f32 / 32768.0;
		let r = self.regs;
		let at = |i: usize, off: i32| (r[i] as i32 * 4 + off + self.cur as i32).rem_euclid(n as i32) as usize;
		let sat = |x: f32| x.clamp(-1.0, 1.0);
		let (lin, rin) = (v(&r, 30) * input.0, v(&r, 31) * input.1);
		let (iir, wall) = (v(&r, 2), v(&r, 7));
		// (destination, tap, input, which wall source): same side then the crossed diff lines.
		for (dst, tap, x) in [(10, 16, lin), (11, 17, rin), (18, 25, lin), (19, 24, rin)] {
			let (a, p) = (at(dst, 0), at(dst, -1));
			let s = self.buf[at(tap, 0)] * wall;
			self.buf[a] = sat((x + s - self.buf[p]) * iir + self.buf[p]);
		}
		// Comb taps (volumes vCOMB1..4 against the mL/mRCOMB1..4 addresses), then two all-pass stages.
		let combs = [12, 14, 20, 22];
		let mut out = [0f32; 2];
		for (ch, o) in out.iter_mut().enumerate() {
			let mut y = 0.0;
			for (k, &c) in combs.iter().enumerate() {
				y += v(&r, 3 + k) * self.buf[at(c + ch, 0)];
			}
			for (apf, vol, size) in [(26, 8, 0), (28, 9, 1)] {
				let (w, back) = (at(apf + ch, 0), at(apf + ch, -(r[size] as i32 * 4)));
				y = sat(y - v(&r, vol) * self.buf[back]);
				self.buf[w] = y;
				y = sat(y * v(&r, vol) + self.buf[back]);
			}
			*o = y;
		}
		self.cur = (self.cur + 1) % n;
		(out[0], out[1])
	}
}

struct Voice {
	clip: Clip,
	/// Position in clip samples.
	at: f64,
	volume: f32,
	pan: f32,
	id: u64,
	/// Held in place and silent (the pause menu's music pause).
	paused: bool,
	/// Feeds the reverb unit (the clip's send flag).
	send: bool,
	/// Rate multiplier: random def pitch, Doppler and underwater shift (`sound::pitch_ratio`).
	pitch: f32,
	/// A music voice: not a sound-effect slot, scaled down underwater.
	music: bool,
}


/// May another sound-effect voice start? The retail allocator (FUN_002a0cd0) has 26 slots, 30 for the
/// hero and the other privileged owners, and drops the new sound when none is free: nothing is evicted
/// (decomp/notes/sound.md).
/// RAC_RETAIL_SNDLOG: log every voice start with its call site (retail/snd.rs voice ledger).
pub fn voice_log() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var_os("RAC_RETAIL_SNDLOG").is_some())
}

fn has_free_slot(voices: &[Voice], privileged: bool) -> bool {
	let cap = if privileged { crate::sound::VOICE_CAP_PRIVILEGED } else { crate::sound::VOICE_CAP };
	voices.iter().filter(|v| !v.music).count() < cap
}

/// A stereo streaming source (FMV audio): interleaved i16 frames fed by another thread, resampled
/// linearly to the output rate and mixed after the mono voices. `played` is the audio clock.
pub struct StreamSource {
	pub rate: u32,
	state: Mutex<StreamState>,
	closed: std::sync::atomic::AtomicBool,
}

struct StreamState {
	queue: std::collections::VecDeque<i16>,
	/// Fraction of a source frame consumed (resampler phase).
	phase: f64,
	played: u64,
	volume: f32,
	/// Silent and not consuming until set (priming) or while paused.
	playing: bool,
	paused: bool,
	/// [PC] Declick: gain 0..1 that ramps down over `FADE_FRAMES` when the queue runs dry, the stream pauses or closes (the last
	/// frame is held and faded instead of cutting to zero) and back up when data plays again. 1.0 = untouched samples.
	gain: f32,
	hold: [f32; 2],
}

/// Length of the stream declick ramp in output frames (2 ms at 48 kHz).
const FADE_FRAMES: f32 = 96.0;

impl StreamSource {
	/// Appends interleaved L,R samples.
	pub fn push(&self, pcm: &[i16]) {
		if let Ok(mut s) = self.state.lock() {
			s.queue.extend(pcm.iter().copied());
		}
	}

	/// Source frames queued and not yet played.
	pub fn buffered(&self) -> usize {
		self.state.lock().map_or(0, |s| s.queue.len() / 2)
	}

	/// Source frames consumed so far (the audio clock, in 1/rate seconds).
	pub fn played(&self) -> u64 {
		self.state.lock().map_or(0, |s| s.played)
	}

	pub fn set_playing(&self, playing: bool) {
		if let Ok(mut s) = self.state.lock() {
			s.playing = playing;
		}
	}

	pub fn close(&self) {
		self.closed.store(true, std::sync::atomic::Ordering::Relaxed);
	}
}

pub struct Audio {
	streams: Arc<Mutex<Vec<Arc<StreamSource>>>>,
	voices: Arc<Mutex<Vec<Voice>>>,
	next_id: u64,
	/// Requested reverb (libsd mode, depth) for the callback's effect unit.
	reverb: Arc<Mutex<(u8, i32)>>,
	/// Master volume (the front end's sound option), applied as voices start or change.
	pub master: f32,
	/// Music volume (the pause menu's music option), for `play_music` / `set_music`.
	pub music: f32,
	/// Sound Options MONO: every voice centred.
	pub mono: bool,
	/// Listener underwater: music voices play at 3/5 (group-1 volume, notes/ambience.md).
	underwater: Arc<std::sync::atomic::AtomicBool>,
	/// In-engine scene (DAT_0015f5a4 == 2): group 1 (music) is 0 (`master_groups`).
	scene: Arc<std::sync::atomic::AtomicBool>,
	/// The retail driver's SPU2 voices (spumix.rs), fed by `spu_push`; None until the first batch.
	spu: SpuShared,
	_stream: Option<cpal::Stream>,
}

/// The retail driver's SPU2 voices (spumix.rs), shared with the mixer.
type SpuShared = Arc<Mutex<Option<crate::spumix::SpuVoices>>>;

/// The mixer: fills interleaved f32 frames (`channels` per frame at `rate` Hz) from the voices, the reverb unit and the
/// streams. It is the output device's callback; `Audio::offline` renders with it into memory (tests, WAV renders).
fn mixer(
	mix: Arc<Mutex<Vec<Voice>>>,
	smix: Arc<Mutex<Vec<Arc<StreamSource>>>>,
	rev_req: Arc<Mutex<(u8, i32)>>,
	uw: Arc<std::sync::atomic::AtomicBool>,
	sc: Arc<std::sync::atomic::AtomicBool>,
	spu: SpuShared,
	rate: f64,
	channels: usize,
) -> impl FnMut(&mut [f32]) + Send + 'static {
	let mut unit = ReverbUnit::new();
	let mut resample = crate::spumix::Resampler::new(rate);
	let mut wet: Vec<f32> = Vec::new();
	move |data: &mut [f32]| {
		data.fill(0.0);
		let Ok(mut voices) = mix.lock() else { return };
		if let Ok(r) = rev_req.try_lock() {
			unit.set(r.0, r.1);
		}
		wet.clear();
		wet.resize(data.len(), 0.0);
		let uw = uw.load(std::sync::atomic::Ordering::Relaxed);
			let scene = sc.load(std::sync::atomic::Ordering::Relaxed);
		for v in voices.iter_mut().filter(|v| !v.paused) {
			// The SPU2 caps the pitch register at 0x3fff, i.e. 4x the 48 kHz base.
			let step = (v.clip.rate as f64 * v.pitch as f64).min(192000.0) / rate;
			let vol = match (v.music, uw, scene) {
					(true, _, true) => 0.0,
					(true, true, _) => v.volume * 0.6,
					_ => v.volume,
				};
			let (l, r) = (vol * (1.0 - v.pan.max(0.0)), vol * (1.0 + v.pan.min(0.0)));
			for (fi, frame) in data.chunks_exact_mut(channels).enumerate() {
				let n = v.clip.pcm.len();
				if v.at >= n as f64 {
					match v.clip.loop_start {
						Some(s) if s < n => v.at = s as f64 + (v.at - n as f64),
						_ => break,
					}
				}
				let i = v.at as usize;
				let f = (v.at - i as f64) as f32;
				let a = v.clip.pcm[i] as f32;
				let b = *v.clip.pcm.get(i + 1).unwrap_or(&v.clip.pcm[i]) as f32;
				let s = (a + (b - a) * f) / 32768.0;
				frame[0] += s * l;
				if channels > 1 {
					frame[1] += s * r;
				}
				if v.send {
					wet[fi * channels] += s * l;
					if channels > 1 {
						wet[fi * channels + 1] += s * r;
					}
				}
				v.at += step;
			}
		}
		voices.retain(|v| v.clip.loop_start.is_some() || v.at < v.clip.pcm.len() as f64);
		// The SPU2 voices of the retail driver: the 48 kHz mix, resampled to the device. Its reverb send joins `wet`; the
		// core 1 master volume scales the reverb return too.
			let mut master = [1.0f32; 2];
			if let Ok(mut g) = spu.lock() {
			if let Some(s) = g.as_mut() {
				// the effect type, area and depth come from the driver's own reverb events (spumix, stage 10e)
				for (fi, frame) in data.chunks_exact_mut(channels).enumerate() {
					let (d, w, m) = resample.pull(s);
					master = m;
					if channels > 1 {
						frame[0] += d[0] * m[0];
						frame[1] += d[1] * m[1];
						wet[fi * channels] += w[0];
						wet[fi * channels + 1] += w[1];
					} else {
						frame[0] += (d[0] * m[0] + d[1] * m[1]) * 0.5;
						wet[fi * channels] += (w[0] + w[1]) * 0.5;
					}
				}
			}
		}
		// [PC] Release the voice list before the reverb and the streams so the frame thread's voice
		// queries (playing / cap / stop) wait for the voice mix only.
		drop(voices);
		if unit.active() {
			for (fi, frame) in data.chunks_exact_mut(channels).enumerate() {
				let (wl, wr) = unit.frame((wet[fi * channels], if channels > 1 { wet[fi * channels + 1] } else { wet[fi * channels] }), rate);
							let (wl, wr) = (wl * master[0], wr * master[1]);
				frame[0] += wl;
				if channels > 1 {
					frame[1] += wr;
				}
			}
		}
		if let Ok(mut streams) = smix.lock() {
			// A closed stream stays until its fade-out has run (a skipped movie must not cut off in mid-wave).
			streams.retain(|s| !s.closed.load(std::sync::atomic::Ordering::Relaxed) || s.state.lock().is_ok_and(|st| st.gain > 0.0));
			for src in streams.iter() {
				let Ok(mut st) = src.state.lock() else { continue };
				let step = src.rate as f64 / rate;
				let vol = st.volume;
				let closed = src.closed.load(std::sync::atomic::Ordering::Relaxed);
				let wants = st.playing && !st.paused && !closed;
				for frame in data.chunks_exact_mut(channels) {
					let (l, r);
					if wants && st.queue.len() >= 4 {
						let f = st.phase as f32;
						let mix = |c: usize| {
							let (a, b) = (st.queue[c] as f32, st.queue[2 + c] as f32);
							(a + (b - a) * f) / 32768.0 * vol
						};
						(l, r) = (mix(0), mix(1));
						st.hold = [l, r];
						st.phase += step;
						while st.phase >= 1.0 && st.queue.len() >= 4 {
							st.queue.drain(..2);
							st.phase -= 1.0;
							st.played += 1;
						}
						// Back up after a fade (not before: gain 1.0 leaves the samples exactly as they were).
						if st.gain < 1.0 {
							st.gain = (st.gain + 1.0 / FADE_FRAMES).min(1.0);
						}
					} else {
						if wants {
							ST_STREAM_UNDER.fetch_add(1, std::sync::atomic::Ordering::Relaxed); // underrun: the clock holds
						}
						if st.gain <= 0.0 {
							break;
						}
						// No data (or paused / closed): hold the last frame and fade it out, no step to zero.
						if st.gain >= 1.0 {
							let c = if closed { &ST_FADE_CLOSE } else if wants { &ST_FADE_UNDER } else if st.paused { &ST_FADE_PAUSE } else { &ST_FADE_IDLE };
							c.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
						}
						st.gain = (st.gain - 1.0 / FADE_FRAMES).max(0.0);
						(l, r) = (st.hold[0], st.hold[1]);
					}
					let g = st.gain;
					if g < 1.0 {
						ST_FADE_FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
					}
					let (l, r) = (l * g, r * g);
					if channels > 1 {
						frame[0] += l;
						frame[1] += r;
					} else {
						frame[0] += (l + r) * 0.5;
					}
				}
			}
		}
		for x in data.iter_mut() {
			*x = x.clamp(-1.0, 1.0);
		}
	}
}

/// [PC] Puts the calling thread (the device callback) in the Windows multimedia scheduler class "Pro Audio" (MMCSS), else at
/// time-critical priority. A normal-priority callback thread loses the CPU to the busy game threads (above normal, `retail/hot.rs`)
/// for longer than the 10 ms device period on a loaded or weak PC, and the device then plays a gap: crackle. cpal's own `realtime`
/// feature would do the same but is not built in. Returns what was set (for the log). RAC_AUDIO_NO_PROMOTE=1 skips it (A/B runs).
#[cfg(windows)]
fn promote_audio_thread() -> &'static str {
	use std::ffi::c_void;
	if std::env::var_os("RAC_AUDIO_NO_PROMOTE").is_some() {
		return "normal (RAC_AUDIO_NO_PROMOTE)";
	}
	#[link(name = "kernel32")]
	unsafe extern "system" {
		fn LoadLibraryA(name: *const u8) -> *mut c_void;
		fn GetProcAddress(module: *mut c_void, name: *const u8) -> *mut c_void;
		fn GetCurrentThread() -> *mut c_void;
		fn SetThreadPriority(thread: *mut c_void, priority: i32) -> i32;
	}
	// SAFETY: plain Win32 calls on the current thread; avrt.dll is a system library and the export has this signature.
	unsafe {
		let m = LoadLibraryA(c"avrt.dll".as_ptr().cast());
		let f = if m.is_null() { std::ptr::null_mut() } else { GetProcAddress(m, c"AvSetMmThreadCharacteristicsW".as_ptr().cast()) };
		if !f.is_null() {
			type SetCharacteristics = unsafe extern "system" fn(*const u16, *mut u32) -> *mut c_void;
			let f = std::mem::transmute::<*mut c_void, SetCharacteristics>(f);
			let task: Vec<u16> = "Pro Audio\0".encode_utf16().collect();
			let mut index = 0u32;
			if !f(task.as_ptr(), &mut index).is_null() {
				return "MMCSS Pro Audio";
			}
		}
		// THREAD_PRIORITY_TIME_CRITICAL
		if SetThreadPriority(GetCurrentThread(), 15) != 0 {
			return "time critical";
		}
	}
	"normal (could not raise)"
}

#[cfg(not(windows))]
fn promote_audio_thread() -> &'static str {
	"normal"
}

/// Agent test runs play silent so the user does not hear them: RAC_MUTE, the hidden test window (RAC_TEST_WINDOW=hidden) or
/// any smoke check variable (RAC_RETAIL_SMOKE*). Players never set these.
fn test_mute() -> bool {
	std::env::vars_os().any(|(k, v)| {
		let k = k.to_string_lossy();
		k == "RAC_MUTE" || (k == "RAC_TEST_WINDOW" && v == "hidden") || k.starts_with("RAC_RETAIL_SMOKE")
	})
}

impl Audio {
	/// None when there is no usable output device; the game then runs silent.
	pub fn new() -> Option<Audio> {
		let device = cpal::default_host().default_output_device()?;
		let config = device.default_output_config().ok()?;
		if config.sample_format() != cpal::SampleFormat::F32 {
			eprintln!("audio: unsupported output format {:?}", config.sample_format());
			return None;
		}
		let config: cpal::StreamConfig = config.into();
		let (rate, channels) = (config.sample_rate as f64, config.channels as usize);
		let mut a = Audio::detached();
		let mut mix = mixer(a.voices.clone(), a.streams.clone(), a.reverb.clone(), a.underwater.clone(), a.scene.clone(), a.spu.clone(), rate, channels);
		// Agent test runs are silent: the mixer runs as usual (same voice state), the device gets zeros.
		let mute = test_mute();
		let mut probe = Probe::from_env(rate, channels);
		let mut promoted = false;
		let stream = device
			.build_output_stream(
				config,
				move |data: &mut [f32], info: &cpal::OutputCallbackInfo| {
					if !promoted {
						promoted = true;
						eprintln!("audio: device callback thread priority: {}", promote_audio_thread());
					}
					let t = std::time::Instant::now();
					mix(data);
					if let Some(p) = probe.as_mut() {
						p.record(data, channels, t, t.elapsed().as_nanos() as u64, info);
					}
					if mute {
						data.fill(0.0);
					}
				},
				|e| eprintln!("audio: {e}"),
				None,
			)
			.ok()?;
		stream.play().ok()?;
		a._stream = Some(stream);
		Some(a)
	}

	/// The voice state with no output attached.
	fn detached() -> Audio {
		Audio {
			streams: Default::default(),
			voices: Default::default(),
			reverb: Arc::new(Mutex::new((0, 0))),
			next_id: 1,
			master: 1.0,
			music: 1.0,
			mono: false,
			underwater: Default::default(),
			scene: Default::default(),
			spu: Default::default(),
			_stream: None,
		}
	}



	/// The mixer with no output device: the caller pulls `rate` Hz frames (tests, WAV renders).
	#[cfg(test)]
	pub fn offline(rate: f64, channels: usize) -> (Audio, impl FnMut(&mut [f32]) + Send + 'static) {
		let a = Audio::detached();
		let mix = mixer(a.voices.clone(), a.streams.clone(), a.reverb.clone(), a.underwater.clone(), a.scene.clone(), a.spu.clone(), rate, channels);
		(a, mix)
	}

	/// The retail driver's SPU2 state at the first batch (an adopted savestate has running voices; `None` = a cold start).
	/// `live` = the timeline follows the audio clock (the device), else it is the exact tick * 200 grid.
	pub fn spu_attach(&mut self, t0: u32, a: Option<&crate::spumix::Attach>, live: bool) {
		let mut s = crate::spumix::SpuVoices::new(t0, live);
		if let Some(a) = a {
			s.attach(a);
		}
		if let Ok(mut g) = self.spu.lock() {
			*g = Some(s);
		}
	}

	/// The SPU2 register writes the retail driver made in one field (oracle order), and the oracle's core mix image.
	pub fn spu_push(&mut self, ev: Vec<crate::spumix::Ev>, core: Option<&ee_oracle::snd989::coreinit::CoreMix>) {
		if let Ok(mut g) = self.spu.lock() {
			if let Some(s) = g.as_mut() {
				if let Some(c) = core {
					s.sync_core(c, false);
				}
				s.push(ev);
			}
		}
	}

	/// Whether the retail driver's SPU2 voices are attached.
	pub fn spu_on(&self) -> bool {
		self.spu.lock().is_ok_and(|g| g.is_some())
	}

	/// A stereo streaming source at `rate` Hz (scaled by the master volume); feed it with `push`,
	/// start it with `set_playing`, drop it with `close`.
	pub fn open_stream(&mut self, rate: u32) -> Arc<StreamSource> {
		ST_STREAM_OPEN.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		let s = Arc::new(StreamSource {
			rate,
			state: Mutex::new(StreamState { queue: Default::default(), phase: 0.0, played: 0, volume: self.master, playing: false, paused: false, gain: 1.0, hold: [0.0; 2] }),
			closed: Default::default(),
		});
		if let Ok(mut v) = self.streams.lock() {
			v.push(s.clone());
		}
		s
	}

	/// Starts a clip; pan -1 (left) .. 1 (right). Returns an id for `stop`.
	#[track_caller]
	pub fn play(&mut self, clip: &Clip, volume: f32, pan: f32) -> u64 {
		self.play_with(clip, volume, pan, 1.0, false)
	}

	/// `play` with a rate multiplier; `privileged` owners (hero) may use the last four voice slots.
	/// Returns 0 (never a live id) when no slot is free and the sound is dropped.
	#[track_caller]
	pub fn play_with(&mut self, clip: &Clip, volume: f32, pan: f32, pitch: f32, privileged: bool) -> u64 {
		self.start(clip, volume * self.master, pan, pitch, false, privileged, true)
	}

	/// Changes a live voice's rate multiplier (Doppler is re-evaluated every frame).
	pub fn set_pitch(&mut self, id: u64, pitch: f32) {
		if let Ok(mut v) = self.voices.lock() {
			if let Some(v) = v.iter_mut().find(|v| v.id == id) {
				v.pitch = pitch;
			}
		}
	}

	/// The listener is underwater (music at 3/5).
	pub fn set_underwater(&mut self, on: bool) {
		self.underwater.store(on, std::sync::atomic::Ordering::Relaxed);
	}

	/// An in-engine scene plays (music group 1 = 0).
	pub fn set_scene(&mut self, on: bool) {
		self.scene.store(on, std::sync::atomic::Ordering::Relaxed);
	}

	/// A music voice: scaled by `music` instead of `master`.
	#[track_caller]
	pub fn play_music(&mut self, clip: &Clip, volume: f32) -> u64 {
		self.start(clip, volume * self.music, 0.0, 1.0, true, true, true)
	}

	pub fn set_music(&mut self, id: u64, volume: f32) {
		self.change(id, volume * self.music, 0.0);
	}

	/// Selects the reverb effect (libsd mode 0..=9) and depth, as the game's per-frame consumer
	/// does after the zone setters (ambience.md); the unit reconfigures only when the mode changes.
	pub fn set_reverb(&mut self, mode: u8, amount: i32) {
		if let Ok(mut r) = self.reverb.lock() {
			*r = (mode, amount);
		}
	}

	#[track_caller]
	fn start(&mut self, clip: &Clip, volume: f32, pan: f32, pitch: f32, music: bool, privileged: bool, capped: bool) -> u64 {
		let pan = if self.mono { 0.0 } else { pan };
		let Ok(mut v) = self.voices.lock() else { return 0 };
		if capped && !music && !has_free_slot(&v, privileged) {
			return 0;
		}
		let id = self.next_id;
		self.next_id += 1;
		v.push(Voice { clip: clip.clone(), at: 0.0, volume, pan: pan.clamp(-1.0, 1.0), id, paused: false, send: clip.reverb && !music, pitch, music });
		if voice_log() {
			eprintln!("audio: start {id} frames {} loop {:?} live {} by {}", clip.pcm.len(), clip.loop_start, v.len(), std::panic::Location::caller());
		}
		id
	}

	pub fn set(&mut self, id: u64, volume: f32, pan: f32) {
		self.change(id, volume * self.master, pan);
	}

	fn change(&mut self, id: u64, volume: f32, pan: f32) {
		let pan = if self.mono { 0.0 } else { pan };
		if let Ok(mut v) = self.voices.lock() {
			if let Some(v) = v.iter_mut().find(|v| v.id == id) {
				v.volume = volume;
				v.pan = pan.clamp(-1.0, 1.0);
			}
		}
	}

	/// Freezes or resumes a voice where it is.
	pub fn pause(&mut self, id: u64, paused: bool) {
		if let Ok(mut v) = self.voices.lock() {
			if let Some(v) = v.iter_mut().find(|v| v.id == id) {
				v.paused = paused;
			}
		}
	}

	/// Freezes or resumes every voice where it is: the pause menu's music pause plus IOP sound group
	/// 0x1d pause/resume (opcodes 0x16 / 0x17; decomp/notes/music.md). Voices started while paused
	/// (menu sounds) play normally.
	pub fn pause_all(&mut self, paused: bool) {
		if let Ok(mut v) = self.voices.lock() {
			for v in v.iter_mut() {
				v.paused = paused;
			}
		}
	}


	pub fn playing(&self, id: u64) -> bool {
		self.voices.lock().is_ok_and(|v| v.iter().any(|v| v.id == id))
	}

	pub fn stop(&mut self, id: u64) {
		if let Ok(mut v) = self.voices.lock() {
			v.retain(|v| v.id != id);
		}
	}

	pub fn stop_all(&mut self) {
		if let Ok(mut v) = self.voices.lock() {
			v.clear();
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn voice(music: bool) -> Voice {
		Voice { clip: Clip { rate: 48000, pcm: Arc::new(vec![0; 4]), loop_start: None, reverb: true }, at: 0.0, volume: 1.0, pan: 0.0, id: 1, paused: false, send: true, pitch: 1.0, music }
	}

	/// 26 sound-effect slots (30 for the hero), music not counted, no eviction (FUN_002a0cd0).
	#[test]
	fn voice_cap() {
		let mut v: Vec<Voice> = (0..25).map(|_| voice(false)).collect();
		v.push(voice(true));
		assert!(has_free_slot(&v, false));
		v.push(voice(false));
		assert!(!has_free_slot(&v, false));
		assert!(has_free_slot(&v, true));
		v.extend((0..3).map(|_| voice(false)));
		assert!(has_free_slot(&v, true));
		v.push(voice(false));
		assert!(!has_free_slot(&v, true));
	}

	/// Reverb unit: off is silent, an impulse in a mode leaves a decaying tail.
	#[test]
	fn reverb_tail() {
		let mut off = ReverbUnit::new();
		off.set(0, 0);
		let mut on = ReverbUnit::new();
		on.set(4, 0x7fff);
		let (mut e_off, mut e_on, mut late) = (0.0f32, 0.0f32, 0.0f32);
		for i in 0..96000 {
			let x = if i == 0 { (0.8, 0.8) } else { (0.0, 0.0) };
			let a = if off.active() { off.tick(x).0 } else { 0.0 };
			let (c, _) = on.tick(x);
			e_off += a * a;
			e_on += c * c;
			if i > 48000 {
				late += c * c;
			}
		}
		assert_eq!(e_off, 0.0);
		assert!(e_on > 1e-6, "no wet signal");
		assert!(late < e_on, "tail does not decay");
	}

	/// Left channel of `n` mixed frames, appended to `out`.
	fn pull(mix: &mut impl FnMut(&mut [f32]), n: usize, out: &mut Vec<f32>) {
		let mut b = vec![0.0f32; n * 2];
		mix(&mut b);
		out.extend(b.chunks_exact(2).map(|f| f[0]));
	}

	/// [PC] Stream declick (FMV audio): gain 1.0 leaves the samples exactly as they were; an underrun, `set_playing(false)` (the player's
	/// pause) and `close` fade the held frame to 0 over `FADE_FRAMES` frames instead of cutting to zero; data again fades back in;
	/// a closed stream leaves the list once its fade has ended.
	#[test]
	fn stream_declick() {
		let (mut a, mut mix) = Audio::offline(48000.0, 2);
		let s = a.open_stream(48000);
		let tone = |frames: usize| -> Vec<i16> { (0..frames).flat_map(|_| [16384i16, -16384]).collect() };
		let step = 0.5 / FADE_FRAMES + 1e-6;
		let falls = |x: &[f32]| x.windows(2).all(|w| w[0] >= w[1] && w[0] - w[1] <= step);
		let rises = |x: &[f32]| x.windows(2).all(|w| w[1] >= w[0] && w[1] - w[0] <= step);
		let mut out = Vec::new();
		s.push(&tone(1000));
		s.set_playing(true);
		pull(&mut mix, 500, &mut out);
		assert!(out.iter().all(|&v| v == 0.5), "gain 1.0 must leave the samples untouched");
		// Underrun: 999 frames play (the last one stays as the interpolation partner), then the held frame fades to 0.
		pull(&mut mix, 700, &mut out);
		assert!(out[..999].iter().all(|&v| v == 0.5));
		assert!(falls(&out[998..1100]), "underrun: fade, no step to zero");
		assert!(out[1100..].iter().all(|&v| v == 0.0));
		// Data again: fades back in over FADE_FRAMES, then untouched.
		let n = out.len();
		s.push(&tone(1000));
		pull(&mut mix, 400, &mut out);
		assert!(rises(&out[n - 1..n + 97]), "fade-in");
		assert!(out[n + 100..].iter().all(|&v| v == 0.5));
		// Pause (set_playing(false)) and resume.
		let n = out.len();
		s.set_playing(false);
		pull(&mut mix, 200, &mut out);
		assert!(falls(&out[n - 1..n + 100]), "pause: fade, no step to zero");
		assert!(out[n + 100..].iter().all(|&v| v == 0.0));
		let n = out.len();
		s.set_playing(true);
		pull(&mut mix, 200, &mut out);
		assert!(rises(&out[n - 1..n + 97]), "resume: fade-in");
		assert!(out[n + 100..].iter().all(|&v| v == 0.5));
		// Close: the held frame fades out, then the stream is dropped.
		let n = out.len();
		s.close();
		pull(&mut mix, 200, &mut out);
		assert!(falls(&out[n - 1..n + 100]), "close: fade, no step to zero");
		assert!(out[n + 100..].iter().all(|&v| v == 0.0));
		assert_eq!(a.streams.lock().unwrap().len(), 1, "the stream stays until its fade has run");
		pull(&mut mix, 10, &mut out);
		assert!(a.streams.lock().unwrap().is_empty(), "the faded-out closed stream leaves the list");
	}
}
