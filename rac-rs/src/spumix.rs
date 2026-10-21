//! The SPU2 voice mixer: plays the register writes of the exact 989snd driver (ee-oracle `snd989`) 1:1 with the hardware.
//!
//! The oracle's driver port issues the SPU2 register writes the IOP module makes (`SpuWrite`: VOLL/VOLR/PITCH/SSA/ADSR1/ADSR2
//! per voice, VMIXL/R/EL/ER, KON/KOFF, core 1 master volume), each tagged with the driver tick (240 Hz) it happened in. This
//! module replays them on 48 SPU2 voices and renders the 48 kHz output the SPU2 would produce:
//! - per output sample the envelope and sample cursor are the oracle's own (`ee_oracle::spu2::Spu2Voice`: PCSX2
//!   `V_ADSR::Calculate`, SP / NextA / loop flags), so voice lifetimes agree with the driver's end poll to the sample;
//! - the PS-ADPCM blocks are decoded as the cursor enters them with the decoder history carried across blocks and loop jumps
//!   (the filter table and rounding of `sound::decode_adpcm`);
//! - the 4-tap Gaussian interpolation (`gauss.rs`, index = SP bits 4-11), `(ENVX * s) >> 15`, `(VOL * s) >> 15` with the
//!   signed 15-bit VOLL/VOLR (phase invert in the register);
//! - the core mix: VMIXL/R pick the dry voices, VMIXEL/ER the reverb send, MMIX gates the voice and external inputs, core 0
//!   feeds core 1's external input through AVOL, MVOL scales the core output (notes/sound_grains.md "Output effect").
//!
//! The reverb unit itself stays in `audio.rs` (`ReverbUnit`: mode and depth come from the game's reverb commands). Events of
//! driver tick T are applied after the SPU2 has run `200 * (T - T0)` samples, T0 = the tick the driver state was taken at
//! [C oracle snd989 driver.rs `tick_once`: the end poll steps the SPU2 200 samples, then the iteration increments the tick
//! counter to T and makes tick T's writes; commands between wakes are tagged with the last tick T and land at the same
//! point]. (corrected 2026-10-13, stage 9b: was `200 * (T + 1 - T0)`, which put every write one tick late.)

use crate::gauss::GAUSS;
use ee_oracle::snd989::bank::Bank;
use ee_oracle::snd989::coreinit::CoreMix;
use ee_oracle::snd989::voices::{Reg, SpuWrite, StreamData};
use ee_oracle::spu2::{volume_of, Spu2Voice, Step};
use std::collections::VecDeque;
use std::sync::Arc;

pub const VOICES: usize = 48;
/// 48 kHz output samples per 240 Hz driver tick.
pub const SAMPLES_PER_TICK: u64 = 200;
/// Lead of the live timeline over the mixer clock: two PAL fields (the oracle hands the events over once per field).
const LIVE_LEAD: i64 = 1920;
/// A live timeline that drifts from the lead by more than this (late or far ahead) is re-anchored.
const LIVE_SLACK: i64 = 4 * 960;

/// One register write with the bank its SSA points into (`Reg::Ssa` only) or, for the host event `Reg::StreamSrc`, the ADPCM
/// data of the stream the voice plays from now on (shared with the oracle's voice, the oracle appends queued parts to it).
#[derive(Clone)]
pub struct Ev {
	pub w: SpuWrite,
	pub bank: Option<Arc<Bank>>,
	pub stream: Option<StreamData>,
}

/// The oracle driver's SPU2 state at the moment the mixer attaches (an adopted savestate has running voices).
#[derive(Clone)]
pub struct Attach {
	/// Driver tick the state is at (T0).
	pub tick: u32,
	pub voices: Vec<(Spu2Voice, Option<Arc<Bank>>, Option<StreamData>)>,
	/// VMIXL, VMIXR, VMIXEL, VMIXER per core.
	pub vmix: [[u32; 4]; 2],
	pub core: CoreMix,
	/// The reverb registers of the driver state (`Reverb::snapshot`: EEA, preset, EVOL, EFFECT_ENABLE), applied at once.
	pub fx: Vec<SpuWrite>,
}

#[derive(Clone)]
struct Voice {
	spu: Spu2Voice,
	bank: Option<Arc<Bank>>,
	/// A stream voice reads its data from here (offset 0), not from a bank (stage 11).
	stream: Option<StreamData>,
}

/// One 48 kHz output sample of the SPU2 (core 1 output, before the master volume and the reverb return).
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub struct Frame {
	/// Dry L/R: core 1's clamped voice sum and the external input (core 0) as MMIX selects them, before the final clamp.
	pub dry: [i32; 2],
	/// Reverb send L/R (core 1's reverb input TW).
	pub wet: [i32; 2],
	/// Core 1 MVOL L/R as signed multipliers of 1/0x8000.
	pub master: [i32; 2],
}

pub struct SpuVoices {
	voices: Vec<Voice>,
	/// VMIXL, VMIXR, VMIXEL, VMIXER per core.
	vmix: [[u32; 4]; 2],
	mmix: [u16; 2],
	mvol: [[u16; 2]; 2],
	avol: [[u16; 2]; 2],
	/// Core 1's effect unit (core 0's EVOL is 0 [C coreinit INIT] and its effect stays off) and its EVOL. The driver's reverb
	/// commands arrive as events (`Reg::EvolL` ... `Reg::FxClear`, ee-oracle snd989 reverb.rs).
	rev: crate::spurev::Reverb,
	evol: [i16; 2],
	/// EEA (end of the work area, the 128 KiB page kept by the register | 0x1ffff) and ESA of the last SetEffectAttr.
	eea: u32,
	esa: u32,
	pending: VecDeque<(i64, Ev)>,
	/// Samples rendered since the clock started.
	pos: i64,
	/// Timeline offset: event tick T lands at `base + 200 * T`.
	base: i64,
	live: bool,
	anchored: bool,
	/// Highest sample position an event was scheduled at.
	last_at: i64,
	/// Applied key-ons (sample position, voice) when logging (verification).
	kons: Option<Vec<(i64, usize)>>,
}

/// The preset libsd sceSdSetEffectAttr programs for `mode` (1..=9; 0 = off has no work area): (size in bytes, 32 registers).
/// ECHO (7) and DELAY (8) are patched with the delay and feedback (libsd 0xe50-0xf6c, register numbers in `audio::PRESETS` order):
/// d = delay + 1; r10 = (d << 6) - r0; r11 = (d << 5) - r1; r12 = r13 + (d << 5); r16 = r17 + (d << 5); r26 = r28 + (d << 5);
/// r27 = r29 + (d << 5); r7 (vWALL) = feedback * 0x102 (16 bit wrap).
fn effect_preset(mode: u32, delay: u32, feedback: u32) -> Option<(u32, [u16; 32])> {
	let &(units, mut r) = crate::audio::PRESETS.get((mode as usize).wrapping_sub(1))?;
	if mode == 7 || mode == 8 {
		let d = delay.wrapping_add(1);
		r[10] = ((d << 6) as u16).wrapping_sub(r[0]);
		r[11] = ((d << 5) as u16).wrapping_sub(r[1]);
		r[12] = r[13].wrapping_add((d << 5) as u16);
		r[16] = r[17].wrapping_add((d << 5) as u16);
		r[26] = r[28].wrapping_add((d << 5) as u16);
		r[27] = r[29].wrapping_add((d << 5) as u16);
		r[7] = (feedback.wrapping_mul(0x102)) as u16;
	}
	Some((units as u32 * 8, r))
}

fn clamp16(x: i32) -> i32 {
	x.clamp(-32768, 32767)
}

impl SpuVoices {
	/// An idle mixer whose clock starts at the beginning of driver tick `t0`. `live` re-anchors the timeline to the audio
	/// clock when it drifts (device pacing); an offline render keeps the exact tick * 200 positions.
	pub fn new(t0: u32, live: bool) -> SpuVoices {
		// The boot state: sceSdInit + the driver's init writes + cid 13 (voices to the output) that every boot sends.
		let (mut core, _) = CoreMix::boot_init();
		core.set_voice_mode(1);
		let mut s = SpuVoices {
			voices: (0..VOICES).map(|_| Voice { spu: Spu2Voice::default(), bank: None, stream: None }).collect(),
			vmix: [[0; 4]; 2],
			mmix: [0; 2],
			mvol: [[0; 2]; 2],
			avol: [[0; 2]; 2],
			rev: crate::spurev::Reverb::new(),
			evol: [0; 2],
			eea: 0,
			esa: 0,
			pending: VecDeque::new(),
			pos: 0,
			base: -(SAMPLES_PER_TICK as i64) * (t0 as i64),
			live,
			anchored: !live,
			last_at: 0,
			kons: None,
		};
		s.sync_core(&core, true);
		s
	}

	/// Core registers from the oracle's image (`CoreMix`). MVOL of core 1 also arrives as events (cid 9 group 8), so it is
	/// only taken from the image when `all`.
	pub fn sync_core(&mut self, c: &CoreMix, all: bool) {
		for k in 0..2 {
			self.mmix[k] = c.core[k].mmix;
			self.avol[k] = c.core[k].avol;
			if all || k == 0 {
				self.mvol[k] = c.core[k].mvol;
			}
		}
	}

	/// sceSdSetEffectAttr (event `Reg::FxAttr`, value = mode, bank = delay << 16 | feedback): loads the preset and puts the work
	/// area at ESA = EEA - size + 1 (libsd reads the end back from the register: its 128 KiB page | 0x1ffff, so the area starts
	/// `size` bytes below the page end, size NOT rounded). EFFECT_ENABLE is left as it is (libsd clears it while programming and
	/// restores it); mode 0 loads silence and keeps the area.
	fn fx_attr(&mut self, mode: u32, bank: u32) {
		match effect_preset(mode, bank >> 16, bank & 0xffff) {
			Some((size, regs)) => {
				self.rev.set_regs(&regs);
				self.esa = self.eea.wrapping_sub(size).wrapping_add(1);
				self.rev.set_area_bytes(self.esa, self.eea);
			}
			None => self.rev.set_regs(&[0; 32]),
		}
	}

	/// Takes over the oracle driver's voices (running voices of an adopted state keep playing).
	pub fn attach(&mut self, a: &Attach) {
		for (v, (spu, bank, stream)) in a.voices.iter().enumerate().take(VOICES) {
			self.voices[v] = Voice { spu: spu.clone(), bank: bank.clone(), stream: stream.clone() };
		}
		self.vmix = a.vmix;
		self.sync_core(&a.core, true);
		for w in &a.fx {
			self.apply(&Ev { w: *w, bank: None, stream: None });
		}
	}

	/// Queues the writes of one batch (oracle order). Offline, event tick T lands at sample `200 * (T - T0)`. Live, the
	/// first batch anchors the timeline `LIVE_LEAD` samples ahead of the audio clock and later drift re-anchors it.
	pub fn push(&mut self, ev: Vec<Ev>) {
		let Some(first) = ev.first() else { return };
		crate::audio::ST_SPU_PUSH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
		if self.live {
			let lead = |s: &SpuVoices| s.base + SAMPLES_PER_TICK as i64 * first.w.tick as i64 - s.pos;
			if lead(self) < 0 {
				crate::audio::ST_SPU_LATE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
			}
			if !self.anchored || (lead(self) - LIVE_LEAD).abs() > LIVE_SLACK {
				crate::audio::ST_SPU_REANCHOR.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
				// Keep the events behind the new anchor after the ones already queued.
				self.base += LIVE_LEAD - lead(self);
				self.anchored = true;
			}
		}
		for e in ev {
			// A write may not land before an earlier one (re-anchoring can pull the timeline back).
			let at = (self.base + SAMPLES_PER_TICK as i64 * e.w.tick as i64).max(self.last_at);
			self.last_at = at;
			self.pending.push_back((at, e));
		}
	}

	/// Applies the queued writes that are due at the current position (the oracle applies a tick's writes after the tick's
	/// samples; this settles the mixer at that point without running a sample).
	pub fn flush_due(&mut self) {
		while self.pending.front().is_some_and(|p| p.0 <= self.pos) {
			let (_, e) = self.pending.pop_front().unwrap();
			self.apply(&e);
		}
	}

	/// Voices still sounding (envelope not stopped).
	#[cfg(test)]
	pub fn active(&self) -> usize {
		self.voices.iter().filter(|v| v.spu.active()).count()
	}

	/// ENVX of a voice (tests, diagnostics).
	#[cfg(test)]
	pub fn envx(&self, v: usize) -> u16 {
		self.voices[v].spu.envx()
	}

	/// Starts logging the applied key-ons (verification).
	#[cfg(test)]
	pub fn log_kons(&mut self) {
		self.kons = Some(Vec::new());
	}

	/// The key-ons applied since `log_kons` (sample position, voice).
	#[cfg(test)]
	pub fn kon_log(&self) -> &[(i64, usize)] {
		self.kons.as_deref().unwrap_or(&[])
	}

	/// (ESA, EEA, EVOL L/R) of core 1's effect unit (tests, diagnostics).
	#[cfg(test)]
	pub fn effect_regs(&self) -> (u32, u32, [i16; 2]) {
		(self.esa, self.eea, self.evol)
	}

	fn apply(&mut self, e: &Ev) {
		let (idx, val) = (e.w.index as usize, e.w.value);
		// The host event before a stream key-on: the voice plays the stream's data from offset 0.
		if e.w.reg == Reg::StreamSrc {
			if let Some(v) = self.voices.get_mut(idx) {
				v.stream = e.stream.clone();
				v.bank = None;
			}
			return;
		}
		if e.w.reg.per_voice() {
			let Some(v) = self.voices.get_mut(idx) else { return };
			let s = &mut v.spu;
			match e.w.reg {
				Reg::VolL => s.vol_l = val as u16,
				Reg::VolR => s.vol_r = val as u16,
				Reg::Pitch => s.pitch = val as u16,
				Reg::Adsr1 => {
					s.adsr1 = val as u16;
					s.env.set_adsr(s.adsr1, s.adsr2);
				}
				Reg::Adsr2 => {
					s.adsr2 = val as u16;
					s.env.set_adsr(s.adsr1, s.adsr2);
				}
				Reg::Ssa => {
					// A stream voice's SSA is a ring address (bank handle 0): its data stays the stream. A tone's SSA (a bank
					// handle, even one the driver no longer holds) drops the stream (oracle `write_bank`).
					if v.stream.is_some() && e.w.bank == 0 {
						s.ssa = 0;
					} else {
						s.ssa = val;
						v.stream = None;
						v.bank = e.bank.clone();
					}
				}
				_ => {}
			}
			return;
		}
		let core = idx & 1;
		match e.w.reg {
			Reg::VmixL => self.vmix[core][0] = val,
			Reg::VmixR => self.vmix[core][1] = val,
			Reg::VmixEL => self.vmix[core][2] = val,
			Reg::VmixER => self.vmix[core][3] = val,
			Reg::Kon => {
				for b in 0..24 {
					if val & (1 << b) != 0 {
						let v = &mut self.voices[core * 24 + b];
						let g = v.stream.as_ref().map(|s| s.read().unwrap());
						let data: &[u8] = match &g {
							Some(g) => &g[..],
							None => v.bank.as_ref().map_or(&[], |b| &b.data[..]),
						};
						v.spu.key_on(data);
						if let Some(k) = self.kons.as_mut() {
							k.push((self.pos, core * 24 + b));
						}
					}
				}
			}
			Reg::Koff => {
				for b in 0..24 {
					if val & (1 << b) != 0 {
						self.voices[core * 24 + b].spu.key_off();
					}
				}
			}
			Reg::MvolL => self.mvol[core][0] = val as u16,
			Reg::MvolR => self.mvol[core][1] = val as u16,
			// the reverb of core 0 is never raised (EVOL 0 at init, the game only drives core 1 [C census: 140 states])
			Reg::EvolL if core == 1 => self.evol[0] = val as u16 as i16,
			Reg::EvolR if core == 1 => self.evol[1] = val as u16 as i16,
			Reg::FxOn if core == 1 => self.rev.fx_enable = val != 0,
			Reg::Eea if core == 1 => self.eea = ((val >> 17) << 17) | 0x1ffff,
			Reg::FxAttr if core == 1 => self.fx_attr(val, e.w.bank),
			Reg::FxClear if core == 1 => self.rev.clear(self.esa, self.eea),
			_ => {}
		}
	}

	/// The next 48 kHz output sample: queued writes that are due are applied first, then every voice runs one sample in
	/// PCSX2 `MixVoice` order [C]: interpolate at the cursor, envelope step (`V_ADSR::Calculate`), `(ENVX * s) >> 15`,
	/// `(VOL * s) >> 15`, then the pitch step (`UpdatePitch` + `ConsumeSamples`). A voice whose sample ends at this step
	/// still outputs this sample (the oracle's `Spu2Voice::tick` = envelope then cursor, split here around the output).
	pub fn frame(&mut self) -> Frame {
		self.flush_due();
		self.pos += 1;
		// Per core: dry L/R and wet L/R sums over the voices (VMIXL/R/EL/ER select the voices per side).
		let mut sum = [[0i32; 4]; 2];
		for (n, v) in self.voices.iter_mut().enumerate() {
			if !v.spu.active() {
				continue;
			}
			let g = v.stream.as_ref().map(|s| s.read().unwrap());
			let data: &[u8] = match &g {
				Some(g) => &g[..],
				None => v.bank.as_ref().map_or(&[], |b| &b.data[..]),
			};
			// PCSX2 MixVoice order: DecodeSamples (the oracle cursor decodes ahead into its FIFO; an end block without repeat
			// stops the voice here, while the last decoded samples are still unplayed), voice value, ADSR, pitch step.
			if v.spu.cur.decode(data) == Step::Stop {
				v.spu.env.stop();
				continue;
			}
			// The window STARTS at the read position: DecodeFifo[DecPosRead + 0..=3] [C].
			let w = v.spu.cur.window();
			let i = v.spu.cur.gauss_index();
			// PCSX2 GetVoiceValues: every tap is shifted separately, then summed (no clamp: the weights sum to 0x7f81
			// at most) [C].
			let interp = ((GAUSS[0xff - i] as i32 * w[0]) >> 15)
				+ ((GAUSS[0x1ff - i] as i32 * w[1]) >> 15)
				+ ((GAUSS[0x100 + i] as i32 * w[2]) >> 15)
				+ ((GAUSS[i] as i32 * w[3]) >> 15);
			// CalculateADSR: a voice whose envelope runs out is stopped and outputs 0 (ADSR.Value = 0).
			if !v.spu.env.tick() {
				continue;
			}
			let (l, r) = v.spu.mix(interp);
			// UpdatePitch + ConsumeSamples.
			v.spu.cur.consume(v.spu.pitch as u32);
			let (core, bit) = (n / 24, 1u32 << (n % 24));
			let m = &self.vmix[core];
			let s = &mut sum[core];
			if m[0] & bit != 0 {
				s[0] += l;
			}
			if m[1] & bit != 0 {
				s[1] += r;
			}
			if m[2] & bit != 0 {
				s[2] += l;
			}
			if m[3] & bit != 0 {
				s[3] += r;
			}
		}
		// MMIX (PCSX2 spu2sys.cpp REG_P_MMIX = libsd SD_MMIX_*) [C]: bit 0/1 ext wet R/L, 2/3 ext dry R/L, 4-7 sound data
		// input (ADMA, not used by the driver), 8/9 voice wet R/L, 10/11 voice dry R/L; core 0 ignores bits 0-3.
		let gate = |core: usize, bit: u32| self.mmix[core] & (1 << bit) != 0;
		let g = |on: bool, x: i32| if on { x } else { 0 };
		// MixCore: the voice sums are clamped first (clamp_mix), the gated parts are added.
		let v = sum.map(|c| c.map(clamp16));
		// Core 0 (no input, no external, its reverb return EVOL0 = 0: the driver never raises it [C coreinit INIT]) -> clamp
		// -> MVOL0 -> core 1 external input at AVOL1 (spu2Mix).
		let c0 = [g(gate(0, 11), v[0][0]), g(gate(0, 10), v[0][1])];
		let c0 = [(clamp16(c0[0]) * volume_of(self.mvol[0][0])) >> 15, (clamp16(c0[1]) * volume_of(self.mvol[0][1])) >> 15];
		let ext = [(c0[0] * self.avol[1][0] as i16 as i32) >> 15, (c0[1] * self.avol[1][1] as i16 as i32) >> 15];
		let wet = [g(gate(1, 9), v[1][2]) + g(gate(1, 1), ext[0]), g(gate(1, 8), v[1][3]) + g(gate(1, 0), ext[1])];
		// MixCore: TD + ApplyVolume(DoReverb(TW), EVOL) [C PCSX2 Mixer.cpp]; the return is part of the dry mix, nothing is left
		// for the device-rate effect.
		let rv = self.rev.process(wet);
		let ret = [(rv[0] * self.evol[0] as i32) >> 15, (rv[1] * self.evol[1] as i32) >> 15];
		Frame {
			dry: [g(gate(1, 11), v[1][0]) + g(gate(1, 3), ext[0]) + ret[0], g(gate(1, 10), v[1][1]) + g(gate(1, 2), ext[1]) + ret[1]],
			wet: [0, 0],
			master: [volume_of(self.mvol[1][0]), volume_of(self.mvol[1][1])],
		}

	}

	/// Renders `n` samples with no reverb (tests, the offline WAV render): `(dry * MVOL) >> 15` per side.
	#[cfg(test)]
	pub fn render(&mut self, n: usize) -> Vec<[i16; 2]> {
		(0..n)
			.map(|_| {
				let f = self.frame();
				// spu2Mix: Out = MVOL1 * clamp_mix(TD + reverb return).
				[((clamp16(f.dry[0]) * f.master[0]) >> 15) as i16, ((clamp16(f.dry[1]) * f.master[1]) >> 15) as i16]
			})
			.collect()
	}
}

/// Resamples the 48 kHz mix to the device rate (linear), one output frame per call.
pub struct Resampler {
	step: f64,
	phase: f64,
	prev: Frame,
	next: Frame,
}

impl Resampler {
	pub fn new(out_rate: f64) -> Resampler {
		Resampler { step: 48000.0 / out_rate, phase: 0.0, prev: Frame::default(), next: Frame::default() }
	}

	/// The device-rate frame: dry and wet as f32 (full scale 1.0) and the master multipliers.
	pub fn pull(&mut self, spu: &mut SpuVoices) -> ([f32; 2], [f32; 2], [f32; 2]) {
		let t = self.phase as f32;
		let mix = |a: [i32; 2], b: [i32; 2]| [(a[0] as f32 + (b[0] - a[0]) as f32 * t) / 32768.0, (a[1] as f32 + (b[1] - a[1]) as f32 * t) / 32768.0];
		// The dry part is clamped before the master volume (spu2Mix `clamp_mix(Out)`).
		let c = |f: [i32; 2]| f.map(clamp16);
		let out = (mix(c(self.prev.dry), c(self.next.dry)), mix(self.prev.wet, self.next.wet), [self.next.master[0] as f32 / 32768.0, self.next.master[1] as f32 / 32768.0]);
		self.phase += self.step;
		while self.phase >= 1.0 {
			self.phase -= 1.0;
			self.prev = self.next;
			self.next = spu.frame();
		}
		out
	}
}

// ---------------------------------------------------------------- the oracle side

/// One field of the oracle's sound output: the driver's SPU2 writes, its core mix image, and (once) the state to attach to.
#[derive(Clone, Default)]
pub struct Batch {
	pub attach: Option<Attach>,
	pub ev: Vec<Ev>,
	pub core: CoreMix,
	/// The oracle driver's tick after the field and each voice's ENVX there (verification: the mixer must agree).
	pub tick: u32,
	pub envx: Vec<u16>,
	/// The stream (VAG) events of the field: start / queued part / end ticks (verification and listening logs).
	pub streams: Vec<ee_oracle::snd989::streams::StreamEv>,
}

impl Batch {
}

/// The driver state taken before the first field (`attach_state`), handed to the viewer with the first batch.
pub static ATTACH: std::sync::Mutex<Option<Attach>> = std::sync::Mutex::new(None);

/// The oracle driver as the machine was loaded (the machine must not have run a field yet): running voices, masks, core mix.
pub fn attach_state(m: &mut ee_oracle::Machine) -> Attach {
	let vb = m.rd32(0x15ed48);
	let c: &mut ee_oracle::cpu::Cpu = &mut m.cpu;
	let inner = c.snd.ensure(&c.iop, &c.mem.ram, c.cold.is_some(), vb);
	let v = &inner.drv.voices;
	Attach {
		tick: inner.tick(),
		voices: (0..VOICES).map(|n| (v.spu[n].clone(), v.bank_of(n).cloned(), v.stream_of(n).cloned())).collect(),
		vmix: [0, 1].map(|c| [v.vmix_l[c], v.vmix_r[c], v.reverb[c], v.reverb[c]]),
		core: inner.core.clone(),
		fx: inner.rev.snapshot().into_iter().map(|o| SpuWrite { tick: inner.tick(), reg: o.reg, index: o.index, value: o.value, bank: o.bank }).collect(),
	}
}

/// The SPU2 writes the driver made while the machine ran its last field, with the bank of every sample start.
pub fn drain(m: &mut ee_oracle::Machine) -> Batch {
	let w = m.take_spu_events();
	let streams = m.take_stream_events();
	let c: &ee_oracle::cpu::Cpu = &m.cpu;
	let inner = c.snd.inner.as_ref();
	let ev = w
		.into_iter()
		.map(|w| {
			let bank = if w.reg == Reg::Ssa { inner.and_then(|i| i.banks.get(w.bank).cloned()) } else { None };
			let stream = if w.reg == Reg::StreamSrc { inner.and_then(|i| i.drv.voices.stream_by_id(w.value)) } else { None };
			Ev { w, bank, stream }
		})
		.collect();
	Batch {
		attach: None,
		ev,
		core: inner.map(|i| i.core.clone()).unwrap_or_default(),
		tick: inner.map_or(0, |i| i.tick()),
		envx: inner.map(|i| (0..VOICES).map(|n| i.drv.voices.envx(n)).collect()).unwrap_or_default(),
		streams,
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use ee_oracle::snd989::voices::SpuWrite;

	fn w(tick: u32, reg: Reg, index: u8, value: u32) -> Ev {
		Ev { w: SpuWrite { tick, reg, index, value, bank: 0 }, bank: None, stream: None }
	}

	/// A bank with only sample data (the mixer reads nothing else).
	fn bank(data: Vec<u8>) -> Arc<Bank> {
		Arc::new(Bank { signature: ee_oracle::snd989::bank::Signature { id: 0, sounds: 0, grains: 0, vags: 0 }, sounds: Vec::new(), data })
	}

	fn blocks(n: usize, seed: u32, end: bool, looped: bool) -> Vec<u8> {
		let mut x = seed;
		let mut d = Vec::new();
		for b in 0..n {
			let mut blk = [0u8; 16];
			for v in blk.iter_mut() {
				x = x.wrapping_mul(1664525).wrapping_add(1013904223);
				*v = (x >> 24) as u8;
			}
			blk[0] = ((x >> 8) as u8 % 5) << 4 | ((x >> 16) as u8 % 13);
			blk[1] = if b == 0 { 4 } else { 0 };
			if b == n - 1 && end {
				blk[1] |= 1 | if looped { 2 } else { 0 };
			}
			d.extend_from_slice(&blk);
		}
		d
	}

	/// The oracle cursor's stream (FIFO window at the read position) equals the whole-sample decoder sound::decode_adpcm
	/// (history carried over the blocks; the disc has no shift above 12, where the two rules differ).
	#[test]
	fn cursor_stream_matches_decode_adpcm() {
		let data = blocks(9, 7, true, true);
		let (pcm, _) = crate::sound::decode_adpcm(&data).unwrap();
		assert_eq!(pcm.len(), 9 * 28);
		let mut c = ee_oracle::spu2::Cursor::start(0, &data);
		for k in 0..pcm.len() - 3 {
			assert_eq!(c.decode(&data), Step::Run);
			let w = c.window();
			for j in 0..4 {
				assert_eq!(w[j], pcm[k + j] as i32, "k {k} tap {j}");
			}
			c.consume(0x1000);
		}
	}

	/// A looping one-block sample: every lap is decoded again with the history of the lap before (PCSX2 Prev1/Prev2).
	#[test]
	fn loop_lap_carries_history() {
		let mut data = blocks(1, 9, true, true);
		data[0] = 0x10; // filter 1 (60/64 of the previous sample), shift 0
		let mut once = data.clone();
		once.extend_from_slice(&data);
		once[1] = 4;
		once[17] = 1;
		let (pcm, _) = crate::sound::decode_adpcm(&once).unwrap();
		let mut c = ee_oracle::spu2::Cursor::start(0, &data);
		for k in 0..52 {
			assert_eq!(c.decode(&data), Step::Run);
			let w = c.window();
			for j in 0..4 {
				assert_eq!(w[j], pcm[k + j] as i32, "k {k} tap {j}");
			}
			c.consume(0x1000);
		}
	}

	/// gauss.rs equals PCSX2's interpTable rows (spot rows; the whole table was compared once, 256/256 rows equal).
	#[test]
	fn gauss_rows_equal_pcsx2_interp_table() {
		let row = |i: usize| [GAUSS[0xff - i], GAUSS[0x1ff - i], GAUSS[0x100 + i], GAUSS[i]];
		assert_eq!(row(0), [0x12C7, 0x59B3, 0x1307, -0x0001]);
		assert_eq!(row(1), [0x1288, 0x59B2, 0x1347, -0x0001]);
		assert_eq!(row(128), [412, 15855, 15948, 424]);
		assert_eq!(row(255), [-0x0001, 0x1307, 0x59B3, 0x12C7]);
	}

	/// MMIX: core 0 voices reach the output only through core 1's external dry gate (bits 2/3) after MVOL0 and AVOL1.
	#[test]
	fn core0_voice_through_external_input() {
		let mut data = blocks(4, 11, true, true);
		for b in 0..4 {
			data[b * 16] = 0;
			for i in 2..16 {
				data[b * 16 + i] = 0x44;
			}
		}
		let run = |mmix1: u16| {
			let mut s = SpuVoices::new(0, false);
			let mut ssa = w(0, Reg::Ssa, 3, 0);
			ssa.bank = Some(bank(data.clone()));
			s.push(vec![
				w(0, Reg::VolL, 3, 0x3fff),
				w(0, Reg::VolR, 3, 0x3fff),
				w(0, Reg::VmixL, 0, 8),
				w(0, Reg::VmixR, 0, 8),
				w(0, Reg::Pitch, 3, 0x1000),
				ssa,
				w(0, Reg::Adsr1, 3, 0x00ff),
				w(0, Reg::Adsr2, 3, 0x1fc0),
				w(0, Reg::Kon, 0, 8),
			]);
			s.mmix[1] = mmix1;
			s.render(400)[350]
		};
		assert_eq!(run(0xf00), [0, 0], "ext gates closed");
		let on = run(0xf0c);
		assert!(on[0] > 1000 && on[0] == on[1], "boot MMIX 0xf0c passes core 0: {on:?}");
	}

	/// A looping voice: key-on through events, steady output while the envelope sustains, key-off ends it, and the 48 kHz
	/// output obeys the pan law of the register volumes.
	#[test]
	fn keyed_voice_plays_and_releases() {
		let mut data = blocks(4, 11, true, true);
		// A constant positive wave: filter 0, shift 0, nibbles 4 (4 << 12 = 0x4000).
		for b in 0..4 {
			data[b * 16] = 0;
			for i in 2..16 {
				data[b * 16 + i] = 0x44;
			}
		}
		let mut s = SpuVoices::new(0, false);
		let bk = bank(data);
		let mut e = |t, r, i, v| w(t, r, i, v);
		let mut ssa = e(1, Reg::Ssa, 24, 0);
		ssa.bank = Some(bk);
		// Attack rate fastest, sustain at max; release 0x1f (slowest) is not needed: key off with a fast release.
		let ev = vec![
			e(1, Reg::VolL, 24, 0x3fff),
			e(1, Reg::VolR, 24, 0x1fff),
			e(1, Reg::VmixL, 1, 1),
			e(1, Reg::VmixR, 1, 1),
			e(1, Reg::Pitch, 24, 0x1000),
			ssa,
			e(1, Reg::Adsr1, 24, 0x00ff),
			e(1, Reg::Adsr2, 24, 0x1fc0),
			e(1, Reg::Kon, 1, 1),
			e(3, Reg::Koff, 1, 1),
		];
		s.push(ev);
		let out = s.render(200 * 2);
		assert!(out[..200].iter().all(|f| *f == [0, 0]), "events of tick 1 land after the first tick's samples");
		let out = s.render(200 * 1);
		let mid = out[150];
		assert!(mid[0] > 1000, "voice sounds: {mid:?}");
		assert!((mid[0] as i32 - 2 * mid[1] as i32).abs() <= 64, "L is twice R: {mid:?}");
		assert!(s.active() == 1);
		s.render(200 * 40);
		assert_eq!(s.active(), 0, "key off released the voice");
	}

	/// A one-shot sample stops by itself at its end block, the envelope reads 0.
	#[test]
	fn one_shot_ends_by_itself() {
		let mut data = blocks(2, 5, true, false);
		for b in 0..2 {
			data[b * 16] = 0;
			for i in 2..16 {
				data[b * 16 + i] = 0x44;
			}
		}
		let mut s = SpuVoices::new(0, false);
		let mut ssa = w(1, Reg::Ssa, 25, 0);
		ssa.bank = Some(bank(data));
		s.push(vec![
			w(1, Reg::VolL, 25, 0x3fff),
			w(1, Reg::VolR, 25, 0x3fff),
			w(1, Reg::VmixL, 1, 2),
			w(1, Reg::VmixR, 1, 2),
			w(1, Reg::Pitch, 25, 0x1000),
			ssa,
			w(1, Reg::Adsr1, 25, 0x00ff),
			w(1, Reg::Adsr2, 25, 0x1fc0),
			w(1, Reg::Kon, 1, 2),
		]);
		s.render(200);
		assert_eq!(s.active(), 0, "keyed at the end of the first tick: voice is idle before");
		s.render(10);
		assert_eq!(s.active(), 1);
		s.render(25);
		assert_eq!(s.active(), 1);
		s.render(10);
		assert_eq!(s.active(), 0, "2 blocks = 14 groups: the decoder reaches the end block at sample 41 (4 * 14 - 15), 16 samples early");
		assert_eq!(s.envx(25), 0);
	}

	/// The driver's cid 80 as events (ee-oracle reverb.rs): EEA, SetEffectAttr (EVOL 0), clear, then 10 ticks later EVOL and
	/// EFFECT_ENABLE; the work area starts at EEA page end - size + 1 (PROVED on PCSX2 SPU2 RAM: mode 3 -> 0x1fb7c0, mode 4 ->
	/// 0x1f9020, the fixed block is 0x1f9000 and the size is not rounded).
	#[test]
	fn driver_events_program_the_effect() {
		for (mode, arg, esa) in [(3u32, 0x1f_9000u32 - 1 + 0x4840, 0x1f_b7c0u32), (4, 0x1f_9000 - 1 + 0x6fe0, 0x1f_9020)] {
			let mut s = SpuVoices::new(0, false);
			assert_eq!((s.evol, s.rev.fx_enable), ([0, 0], false));
			s.push(vec![
				w(0, Reg::Eea, 1, arg),
				w(0, Reg::FxAttr, 1, mode),
				w(0, Reg::EvolL, 1, 0),
				w(0, Reg::EvolR, 1, 0),
				w(0, Reg::FxClear, 1, mode),
				w(10, Reg::EvolL, 1, 6000),
				w(10, Reg::EvolR, 1, 6000),
				w(10, Reg::FxOn, 1, 1),
			]);
			s.render(1);
			assert_eq!((s.esa, s.eea), (esa, 0x1f_ffff), "mode {mode}");
			assert!(s.rev.has_area());
			assert_eq!((s.evol, s.rev.fx_enable), ([0, 0], false));
			s.render(10 * SAMPLES_PER_TICK as usize - 2);
			assert_eq!((s.evol, s.rev.fx_enable), ([0, 0], false));
			s.render(2);
			assert_eq!((s.evol, s.rev.fx_enable), ([6000, 6000], true));
			// core 0 events are ignored: its reverb is never raised
			s.push(vec![w(11, Reg::EvolL, 0, 123), w(11, Reg::FxOn, 0, 0)]);
			s.render(SAMPLES_PER_TICK as usize);
			assert_eq!((s.evol, s.rev.fx_enable), ([6000, 6000], true));
			s.push(vec![w(12, Reg::FxOn, 1, 0), w(12, Reg::EvolL, 1, 0), w(12, Reg::EvolR, 1, 0)]);
			s.render(SAMPLES_PER_TICK as usize);
			assert_eq!((s.evol, s.rev.fx_enable), ([0, 0], false));
		}
	}

	/// ECHO / DELAY presets are patched by delay and feedback (libsd 0xe50-0xf6c); the other modes load the table unchanged.
	#[test]
	fn echo_and_delay_presets_take_delay_and_feedback() {
		let (size, plain) = effect_preset(5, 9, 9).unwrap();
		assert_eq!((size, plain), (0xade0, crate::audio::PRESETS[4].1));
		let base = crate::audio::PRESETS[6].1;
		let (size, r) = effect_preset(7, 15, 0x40).unwrap();
		assert_eq!(size, 0x18040);
		let d = 16u32;
		assert_eq!(r[10], ((d << 6) as u16).wrapping_sub(base[0]));
		assert_eq!(r[11], ((d << 5) as u16).wrapping_sub(base[1]));
		assert_eq!((r[12], r[16], r[26], r[27]), (base[13] + 0x200, base[17] + 0x200, base[28] + 0x200, base[29] + 0x200));
		assert_eq!(r[7], 0x40 * 0x102);
		assert!(effect_preset(0, 0, 0).is_none() && effect_preset(10, 0, 0).is_none());
	}
}
