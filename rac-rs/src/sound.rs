//! Sound effects: Sony 989snd "SBlk" sound banks and the game's sound-id tables.
//!
//! A bank section (the level data header's `sound_bank`, or the ToC's global bank at ToC+0x14e0)
//! is a small container `{u32 3, u32 2, (offset, size) x 2}` holding two chunks:
//!   chunk 0: the SBlk metadata (sounds, grains), chunk 1: raw PS-ADPCM sample data.
//! SBlk header (offsets from the "SBlk" magic): +0x0 magic, +0x4 version 1, +0x8 flags, +0xc bank
//! id, +0x16 u16 sound count, +0x18 u16 grain count, +0x1a u16 vag count, +0x1c u32 sound table,
//! +0x20 u32 grain table (both relative to the magic), +0x28 u32 sample data size.
//! Sound = 12 bytes {u32 volume, u32 grain count (bit 31 = flag), u32 byte offset into the grain
//! table}. Grain = 0x28 bytes {u32 type, s32 delay, 32 bytes of parameters}. Type 1 (tone) plays a
//! sample: the parameters are the PlayStation tone attributes (see `Tone`).
//!
//! Game side (all verified in the v1.00 overlay code, see decomp/notes/sound.md): the level core
//! index +0x70 holds a "sound remap" block with the level's sound definitions (0x20 bytes each) and
//! per-moby-class tables that turn each class sound-def's raw index into a bank sound index.

use crate::bytes::{span, u16_at, u32_at, R};
use crate::iso::Disc;
use crate::level::Level;
use crate::particles::Rng;
use crate::physics::{add, length, scale, sub, Hit, MobySolid, Scene, World, V3};
use crate::toc;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

/// The SPU2 plays a voice at pitch 0x1000 at this rate; tones are played at their centre note.
pub const NATIVE_RATE: u32 = 48000;

pub struct Sample {
	pub rate: u32,
	pub pcm: Vec<i16>,
	/// Index of the first sample of the loop; the sample loops from there to its end when set.
	pub loop_start: Option<usize>,
	/// Feeds the reverb unit: the tone's flag bit 0 (989snd sets the voice's VMIXEL/R from it) [C].
	pub reverb: bool,
}

pub(crate) const FILTERS: [(i32, i32); 5] = [(0, 0), (60, 0), (115, -52), (98, -55), (122, -60)];

/// Decode PS-ADPCM frames starting at `data[0]` until a frame with the end flag (bit 0) is
/// decoded. Returns the samples and the loop start (a frame with bit 2 set, when the end frame
/// has the loop-repeat bit 1).
pub fn decode_adpcm(data: &[u8]) -> R<(Vec<i16>, Option<usize>)> {
	let mut pcm = Vec::new();
	let (mut h1, mut h2) = (0i32, 0i32);
	let mut loop_start = None;
	let looped;
	let mut pos = 0;
	loop {
		let f = span(data, pos, 16).map_err(|_| "ADPCM sample runs off the end of the data".to_string())?;
		pos += 16;
		let (shift, filter) = ((f[0] & 15) as u32, (f[0] >> 4) as usize);
		if filter >= FILTERS.len() {
			return Err(format!("bad ADPCM filter {filter}"));
		}
		let flags = f[1];
		if flags & 4 != 0 && loop_start.is_none() {
			loop_start = Some(pcm.len());
		}
		let (c1, c2) = FILTERS[filter];
		// Shifts above 12 behave as 9 on the hardware.
		let shift = if shift > 12 { 9 } else { shift };
		for i in 0..28 {
			let b = f[2 + i / 2];
			let nib = if i & 1 == 0 { b & 15 } else { b >> 4 } as i32;
			let mut s = (((nib << 12) as i16) as i32) >> shift;
			s += (h1 * c1 + h2 * c2 + 32) >> 6;
			let s = s.clamp(-32768, 32767);
			h2 = h1;
			h1 = s;
			pcm.push(s as i16);
		}
		if flags & 1 != 0 {
			looped = flags & 2 != 0;
			break;
		}
	}
	Ok((pcm, if looped { Some(loop_start.unwrap_or(0)) } else { None }))
}

/// Tone attributes of a type-1 grain (PlayStation VAB tone layout, 32 bytes of parameters).
#[derive(Debug, Clone)]
pub struct Tone {
	/// 0..127.
	pub volume: u8,
	pub centre_note: u8,
	pub centre_fine: u8,
	/// The centre byte was non-negative: 989snd scales that pitch by 44100/48000 (IOP FUN_0001600c) [C].
	pub native44: bool,
	/// Pitch-bend ranges in semitones (signed tone bytes 8 / 9) for bends below / above zero.
	pub bend_down: i8,
	pub bend_up: i8,
	/// Reverb send (flag bit 0).
	pub reverb: bool,
	/// Byte offset of the sample inside the bank's sample data.
	pub vag_offset: usize,
}

#[derive(Debug, Clone)]
pub struct Grain {
	pub kind: u32,
	pub delay: i32,
	pub params: [u8; 32],
}

pub const GRAIN_TONE: u32 = 1;

/// Sound effects play at note 60 (989snd plays SFX tones at middle C; a tone's centre note is the
/// note its sample sounds at its recorded rate).
const SFX_NOTE: f32 = 60.0;

impl Tone {
	/// The rate the sample plays at: the SPU's 48 kHz shifted from the centre note (and fine, in
	/// 1/128 semitones) to the SFX note. The centre byte is already |signed byte| (see `Grain::tone`).
	pub fn rate(&self) -> u32 {
		// Exact driver pitch (IOP FUN_0001600c -> libsd sceSdNote2Pitch at the SFX note 60, fine 0), then the SPU2
		// PITCH register (cap 0x3fff, unity 0x1000 = 48000 Hz) in Hz. The centre fine is ADDED (was subtracted).
		let centre = if self.native44 { self.centre_note as i8 } else { (self.centre_note as i8).wrapping_neg() };
		let pitch = tone_pitch(centre, self.centre_fine as i8, SFX_NOTE as i32, 0).min(0x3fff) as u64;
		((pitch * 48000 + 0x800) >> 12) as u32
	}

}

/// libsd's pitch tables: T1[k] = floor(0x8000 * 2^(k/12)) (k 0..12), T2[k] = floor(0x8000 * 2^(k/1536)) (k 0..128).
fn pitch_tables() -> &'static ([u16; 12], [u16; 128]) {
	static T: std::sync::OnceLock<([u16; 12], [u16; 128])> = std::sync::OnceLock::new();
	T.get_or_init(|| {
		let mut t1 = [0u16; 12];
		let mut t2 = [0u16; 128];
		for (k, v) in t1.iter_mut().enumerate() {
			*v = (32768.0 * (k as f64 / 12.0).exp2()).floor() as u16;
		}
		for (k, v) in t2.iter_mut().enumerate() {
			*v = (32768.0 * (k as f64 / 1536.0).exp2()).floor() as u16;
		}
		(t1, t2)
	})
}

/// libsd sceSdNote2Pitch(centre, centre_fine, note, fine) (libsd 0x38a0), step by step as the IOP code runs it;
/// the sign-extended centre fine is masked to 16 bits and ADDED to the fine. Returns the low 16 bits.
fn note2pitch(centre: i32, centre_fine: i32, note: i32, fine: i32) -> u16 {
	let (t1, t2) = pitch_tables();
	let mut a3 = (fine as i16 as i32) + (centre_fine & 0xffff);
	let v1 = (if a3 < 0 { a3 + 0x7f } else { a3 }) >> 7;
	let n = (note + v1 - centre) as i16 as i32;
	let q = n / 12;
	a3 -= v1 << 7;
	let mut t0 = q - 2;
	let rem = n - 12 * q;
	let mut r = rem;
	let mut fix = true;
	if rem < 0 {
		r = rem + 12;
		t0 = q - 3;
	} else if rem == 0 {
		if a3 >= 0 {
			fix = false;
		} else {
			r = 12;
			t0 = q - 3;
		}
	}
	if fix && a3 < 0 {
		r = (r - 1) + v1;
		a3 += (v1 + 1) << 7;
	}
	let prod = (t1[(r as i16 as i32) as usize] as u32).wrapping_mul(t2[a3 as usize] as u32) as i32;
	let p = prod >> 16;
	let t0 = t0 as i16 as i32;
	let p = if t0 < 0 {
		let sh = (-t0) as u32;
		(p as u32).wrapping_add(1u32 << ((sh - 1) & 31)) >> (sh & 31)
	} else {
		p as u32
	};
	(p & 0xffff) as u16
}

/// IOP FUN_0001600c: the tone's centre note byte (negative = native 48 kHz sample, non-negative = 44.1 kHz sample,
/// scaled by 44100/48000) and centre fine -> SPU2 PITCH value (unity 0x1000).
pub fn tone_pitch(centre_byte: i8, centre_fine: i8, note: i32, fine: i32) -> u16 {
	let non_negative = centre_byte >= 0;
	let centre = if non_negative { centre_byte } else { centre_byte.wrapping_neg() };
	let p = note2pitch(centre as i32, centre_fine as i32, note, fine) as u32;
	let p = if non_negative { (p * 0xac44) / 48000 } else { p };
	(p & 0xffff) as u16
}

/// IOP FUN_000160b4's bend term (1/128 semitone) for a (down, up) range pair.
pub fn bend_term(range: (i8, i8), bend: i32) -> i32 {
	if bend < 0 {
		(range.0 as i32 * bend * 128) >> 15
	} else {
		range.1 as i32 * bend * 128 / 0x7fff
	}
}

impl Grain {
	pub fn tone(&self) -> Option<Tone> {
		if self.kind != GRAIN_TONE {
			return None;
		}
		let p = &self.params;
		Some(Tone {
			volume: p[1],
			// 989snd (IOP FUN_0001600c) takes |signed byte| before sceSdNote2Pitch [C].
			centre_note: (p[2] as i8).unsigned_abs(),
			centre_fine: p[3],
			native44: (p[2] as i8) >= 0,
			bend_down: p[8] as i8,
			bend_up: p[9] as i8,
			reverb: p[14] & 1 != 0,
			vag_offset: u32::from_le_bytes([p[16], p[17], p[18], p[19]]) as usize,
		})
	}
}

#[derive(Debug, Clone)]
pub struct Sound {
	pub volume: u32,
	pub grains: Vec<Grain>,
}

impl Sound {
	/// The tones this sound starts (a sound may layer several samples).
	pub fn tones(&self) -> Vec<Tone> {
		self.grains.iter().filter_map(Grain::tone).collect()
	}

	/// Does a loop-start grain (type 21) appear, i.e. is this a looping sound?
	pub fn loops(&self) -> bool {
		self.grains.iter().any(|g| g.kind == 21)
	}
}

pub struct Bank {
	pub sounds: Vec<Sound>,
	data: Vec<u8>,
}

impl Bank {
	pub fn parse(section: &[u8]) -> R<Bank> {
		if u32_at(section, 4)? != 2 {
			return Err("sound bank container: expected two chunks".into());
		}
		let meta = span(section, u32_at(section, 8)? as usize, u32_at(section, 12)? as usize)?;
		let data = span(section, u32_at(section, 16)? as usize, u32_at(section, 20)? as usize)?;
		if span(meta, 0, 4)? != b"SBlk" || u32_at(meta, 4)? != 1 {
			return Err("sound bank: not an SBlk version 1 block".into());
		}
		let id = u32_at(meta, 0xc)?;
		let nsounds = u16_at(meta, 0x16)? as usize;
		let ngrains = u16_at(meta, 0x18)? as usize;
		let vag_count = u16_at(meta, 0x1a)? as usize;
		let sound_table = u32_at(meta, 0x1c)? as usize;
		let grain_table = u32_at(meta, 0x20)? as usize;
		let grains = span(meta, grain_table, ngrains * 0x28)?;
		let mut sounds = Vec::with_capacity(nsounds);
		for i in 0..nsounds {
			let s = span(meta, sound_table + i * 12, 12)?;
			let count_word = u32_at(s, 4)?;
			let (count, off) = ((count_word & 0xffff) as usize, u32_at(s, 8)? as usize);
			let mut list = Vec::with_capacity(count);
			for g in 0..count {
				let b = span(grains, off + g * 0x28, 0x28).map_err(|e| format!("sound {i}: cw={count_word:#x} off={off:#x} g={g} {e}"))?;
				let mut params = [0u8; 32];
				params.copy_from_slice(&b[8..]);
				list.push(Grain { kind: u32_at(b, 0)?, delay: u32_at(b, 4)? as i32, params });
			}
			sounds.push(Sound { volume: u32_at(s, 0)?,  grains: list });
		}
		Ok(Bank {  sounds,  data: data.to_vec() })
	}

	/// Decode the sample at a byte offset of the sample data.
	pub fn sample_at(&self, offset: usize) -> R<Sample> {
		let (pcm, loop_start) = decode_adpcm(self.data.get(offset..).ok_or("sample offset outside the bank")?)?;
		Ok(Sample { rate: NATIVE_RATE, pcm, loop_start, reverb: false })
	}

	/// The sample of sound `index`: its first tone grain. None when the sound has no tone.
	pub fn sound_sample(&self, index: usize) -> R<Option<Sample>> {
		let sound = self.sounds.get(index).ok_or("no such sound")?;
		match sound.tones().first() {
			Some(t) => Ok(Some(Sample { rate: t.rate(), reverb: t.reverb, ..self.sample_at(t.vag_offset)? })),
			None => Ok(None),
		}
	}

	/// Every sound's first sample, by sound index (an empty sample for sounds without a tone).
	pub fn samples(&self) -> R<Vec<Sample>> {
		(0..self.sounds.len())
			.map(|i| Ok(self.sound_sample(i)?.unwrap_or(Sample { rate: NATIVE_RATE, pcm: Vec::new(), loop_start: None, reverb: false })))
			.collect()
	}
}

/// The level's own bank (level data section 1).
pub fn level_bank(level: &Level) -> R<Bank> {
	Bank::parse(level.sections[1].as_deref().ok_or("level has no sound bank")?)
}

/// Every sound of the level's bank decoded, indexed by bank sound index.
pub fn level_sounds(level: &Level) -> R<Vec<Sample>> {
	level_bank(level)?.samples()
}

/// The global bank (ToC + 0x14e0: sector, sector count).
pub fn global_bank(disc: &mut Disc) -> R<Bank> {
	let t = toc::read_toc(disc).map_err(|e| e.to_string())?;
	let (sector, sectors) = (u32_at(&t, 0x14e0)? as u64, u32_at(&t, 0x14e4)? as u64);
	let raw = disc.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
	Bank::parse(&raw)
}

/// HelpDesk voice `voice` (help message entry +8) for menu language `lang` (EN FR DE ES IT): ToC +
/// 0x1ab8 holds 6 x 150 sector numbers (language table 1 is empty on this disc), each a VAGp file
/// (big-endian data size +0x0c, rate +0x10, PS-ADPCM from +0x30). FUN_002151d0 reads the same
/// table from the boot ELF (decomp/notes/ingame_menu_leftovers.md P1-a). Returns (rate, pcm).
pub fn help_voice(disc: &mut Disc, lang: usize, voice: usize) -> R<Option<(u32, Vec<i16>)>> {
	const TABLE: usize = 0x1ab8;
	const LANG: [usize; 5] = [0, 2, 3, 4, 5];
	if voice >= 150 {
		return Ok(None);
	}
	let t = toc::read_toc(disc).map_err(|e| e.to_string())?;
	let lba = u32_at(&t, TABLE + LANG[lang.min(4)] * 600 + voice * 4)? as u64;
	if lba == 0 {
		return Ok(None);
	}
	let head = disc.read_bytes(lba, 0x30).map_err(|e| e.to_string())?;
	if &head[..4] != b"VAGp" {
		return Err(format!("help voice {voice}: no VAGp at sector {lba}"));
	}
	let be = |o: usize| u32::from_be_bytes([head[o], head[o + 1], head[o + 2], head[o + 3]]);
	let (size, rate) = (be(0x0c) as usize, be(0x10));
	let data = disc.read_bytes(lba, 0x30 + size).map_err(|e| e.to_string())?;
	Ok(Some((rate, decode_adpcm(&data[0x30..])?.0)))
}

/// Every sound of the global bank decoded.
pub fn global_sounds(disc: &mut Disc) -> R<Vec<Sample>> {
	global_bank(disc)?.samples()
}

/// One sound definition: what game code plays by id.
#[derive(Debug, Clone)]
pub struct SoundDef {
	pub min_range: f32,
	pub max_range: f32,
	/// Volume range in 1/1024 units (0x400 = full).
	pub min_vol: i32,
	pub max_vol: i32,
	pub min_pitch: i32,
	pub max_pitch: i32,
	pub looped: bool,
	/// Flag byte values: 1 quadratic falloff, 2 no occlusion test, 4 no underwater halving, 8 no underwater pitch drop
	/// (FUN_0029ff68; notes/sound.md).
	pub flags: u8,
	/// Sound index in the level bank after remapping (None = unmapped, 0xffff).
	pub bank_sound: Option<u16>,
}

/// The parts of a sound def the retail voice code reads each frame (FUN_0029fc10 / FUN_002a0cd0).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Falloff {
	pub min_range: f32,
	pub max_range: f32,
	pub min_vol: i32,
	pub max_vol: i32,
	pub min_pitch: i32,
	pub max_pitch: i32,
	/// Def flag bits (`DEF_NO_OCCLUSION` and the others): 1 quadratic falloff, 2 no occlusion test, 4 no
	/// underwater halving, 8 no underwater pitch drop.
	pub flags: u8,
}

impl Falloff {
	/// Distance gain in 1/1024 units (FUN_0029fc10), 0 when below the start threshold.
	pub fn level(&self, dist: f32) -> i32 {
		let a = attenuate(dist, self.min_range, self.max_range, self.max_vol, self.min_vol, self.flags & 1 != 0);
		if a < MIN_START_VOLUME { 0 } else { a }
	}

	/// Random bend (FUN_002a0cd0): `min + rand % (max - min)`, `r` being the game's `(rand >> 16) & 0x7fff`
	/// draw (`Rng::small` with the span); a def without a range stays at min.
	pub fn bend(&self, r: impl FnOnce(u32) -> u32) -> i32 {
		let span = self.max_pitch - self.min_pitch;
		self.min_pitch + if span > 0 { r(span as u32) as i32 } else { 0 }
	}

	pub fn parse(d: &[u8]) -> R<Falloff> {
		let f = |o: usize| -> R<f32> { Ok(f32::from_bits(u32_at(d, o)?)) };
		Ok(Falloff {
			min_range: f(0)?,
			max_range: f(4)?,
			min_vol: u32_at(d, 8)? as i32,
			max_vol: u32_at(d, 12)? as i32,
			min_pitch: u32_at(d, 16)? as i32,
			max_pitch: u32_at(d, 20)? as i32,
			flags: *d.get(0x19).ok_or("short sound def")?,
		})
	}
}

impl SoundDef {
	pub fn falloff(&self) -> Falloff {
		Falloff { min_range: self.min_range, max_range: self.max_range, min_vol: self.min_vol, max_vol: self.max_vol, min_pitch: self.min_pitch, max_pitch: self.max_pitch, flags: self.flags }
	}

	pub fn parse(d: &[u8], remap: impl Fn(u16) -> Option<u16>) -> R<SoundDef> {
		let f = |o: usize| -> R<f32> { Ok(f32::from_bits(u32_at(d, o)?)) };
		let raw = u16_at(d, 0x1a)?;
		Ok(SoundDef {
			min_range: f(0)?,
			max_range: f(4)?,
			min_vol: u32_at(d, 8)? as i32,
			max_vol: u32_at(d, 12)? as i32,
			min_pitch: u32_at(d, 16)? as i32,
			max_pitch: u32_at(d, 20)? as i32,
			looped: d[0x18] != 0,
			flags: d[0x19],
			bank_sound: remap(raw),
		})
	}
}

/// The level's "sound remap" block (core index +0x70).
pub struct SoundMap {
	/// Sound definitions played by id: ids 0..global_count are shared, then per-level ones. The
	/// footstep table (`footstep_id`) lives in this array too.
	pub defs: Vec<SoundDef>,
	/// For each moby class (in core-index class-table order), the bank sound index of each of
	/// the class's sound definitions.
	pub class_tables: Vec<Vec<u16>>,
}

pub fn sound_map(level: &Level) -> R<SoundMap> {
	let start = u32_at(&level.index, 0x70)? as usize;
	let block = level.index.get(start..).ok_or("sound remap outside the core index")?;
	let defs_off = u16_at(block, 0)? as usize;
	let ndefs = u16_at(block, 2)? as usize;
	let remap_off = u16_at(block, 4)? as usize;
	let nremap = u16_at(block, 6)? as usize;
	let remap = |raw: u16| -> Option<u16> {
		if (raw as usize) < nremap {
			let v = u16_at(block, remap_off + raw as usize * 4).ok()?;
			if v != 0xffff { Some(v) } else { None }
		} else {
			None
		}
	};
	let mut defs = Vec::new();
	for i in 0..ndefs {
		defs.push(SoundDef::parse(span(block, defs_off + i * 0x20, 0x20)?, remap)?);
	}
	let nclasses = level.table(0x18, 32)?.len();
	let mut class_tables = Vec::new();
	for k in 0..nclasses {
		let off = u16_at(block, 8 + k * 4)? as usize;
		let count = u16_at(block, 10 + k * 4)? as usize;
		let mut t = Vec::new();
		for j in 0..count {
			t.push(u16_at(block, off + j * 4)?);
		}
		class_tables.push(t);
	}
	Ok(SoundMap { defs, class_tables })
}

/// Bank sound index of sound definition `idx` of moby class `class_id` (e.g. `class_sound(level,
/// 0, 9)` is Ratchet's class sound 9), None if the class or definition is unmapped.
pub fn class_sound(level: &Level, class_id: u32, idx: usize) -> R<Option<u16>> {
	let map = sound_map(level)?;
	for (k, (id, _)) in class_sound_counts(level)?.into_iter().enumerate() {
		if id == class_id {
			return Ok(map.class_tables[k].get(idx).copied().filter(|&v| v != 0xffff));
		}
	}
	Ok(None)
}

/// Number of sound definitions every moby class declares (class block +0x0d) in class-table order.
pub fn class_sound_counts(level: &Level) -> R<Vec<(u32, Option<usize>)>> {
	let mut out = Vec::new();
	for e in level.table(0x18, 32)? {
		let start = u32_at(e, 0)? as usize;
		if start == 0 {
			out.push((u32_at(e, 4)?, None));
			continue;
		}
		let block = level.block(start)?;
		out.push((u32_at(e, 4)?, Some(*block.get(0x0d).ok_or("short moby class")? as usize)));
	}
	Ok(out)
}


/// One playable cue: a bank sound plus the game's volume for it (1.0 = full).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Cue {
	pub sound: usize,
	pub volume: f32,
	pub looped: bool,
	/// The def's falloff, volumes and flags (every cue comes from a def).
	pub fall: Falloff,
	/// Bend ranges (down, up) of the sound's first tone, for the def's random pitch.
	pub bend: (i8, i8),
}

impl Cue {
	/// The random pitch shift of one start in 1/128 semitones (def pitch range through the tone's bend
	/// range; only the footstep defs have one). `r(n)` is the game's `rand % n` draw.
	pub fn random_shift(&self, r: impl FnOnce(u32) -> u32) -> i32 {
		bend_term(self.bend, self.fall.bend(r))
	}

	/// Gain (1.0 = full) of the cue heard `dist` away: the retail curve of its def (FUN_0029fd00).
	pub fn level(&self, dist: f32) -> f32 {
		self.fall.level(dist) as f32 / 1024.0
	}
}

/// Which bank sound the game plays for each gameplay event (see notes/sound.md "Event table").
/// Pitch is always unshifted (the game only adds doppler), so no pitch field.
#[derive(Debug, Clone, Default)]
pub struct Events {
	/// Hero jump grunt (class sound 26; trigger of sequences 7-9).
	pub jump: Option<Cue>,
	/// Hero landing (class sounds 30 and 31, sequence 12).
	pub land: Option<Cue>,
	/// Hero hurt grunt (class sound 33, sequence 16).
	pub hurt: Option<Cue>,
	/// Wrench swoosh per combo hit (class sounds 27 for hits 1-2, 54 for hit 3).
	pub swing: Vec<Cue>,
	/// Bolt magnet pickup by bolt class (13 = 1 bolt .. 16 = 50), class sound 0 of the bolt class.
	pub bolt: HashMap<u32, Cue>,
	/// Crate break by crate class (class sound 0 of the crate).
	pub crate_break: HashMap<u32, Cue>,
	/// Nanotech crate health pickup (class 501 sound 1).
	pub nanotech: Option<Cue>,
	/// 16 footstep cues indexed `surface_class * 4 + foot * 2 + variant`.
	pub footsteps: Vec<Option<Cue>>,
	/// First looping level sound (emitter ambience).
	pub ambient: Option<Cue>,
	/// NOV-5.8a: level sound-map def 1 (FUN_002a10a0(1, 0, 0), the Take Aim cue), played flat.
	pub global1: Option<Cue>,
	/// Enemy sounds by (moby class, event name): class sounds named by animation role.
	pub enemy: HashMap<(u32, &'static str), Cue>,
	/// Every class sound of the toad (0..=8) and robot (0..=10) by (class, index), for animation cues and
	/// loop sounds (sound.md "Animation sound cues").
	pub enemy_cs: HashMap<(u32, usize), Cue>,
	/// Loop sound (class sound index) of skeleton-less cue classes (the fan), played from sequence 0's loop byte.
	pub static_loops: HashMap<u32, usize>,
	pub glove: GloveCues,
	/// Wrench throw and catch (hero cs27 / cs5, banks 47 / 25; decomp/notes/sound.md task 2).
	pub wrench_throw: Option<Cue>,
	pub wrench_catch: Option<Cue>,
	/// Thrown-wrench impacts: class 71 cs0, cs1 (moby hit) and cs2 (wall), banks 95..97.
	pub wrench_impact: [Option<Cue>; 3],
	/// Pause-menu cues: class 0x472 (the menu panel mobys) cs0..7 through the level's class remap (Veldin banks
	/// 213..220, Novalis 272..279), indexed by `frontend::Cue` (decomp/notes/ingame_pause_menu.md s2).
	pub menu: Vec<Option<Cue>>,
	/// Hyper-Strike seq 43 triggers: hero cs53 and cs27 (veldin_enemies.md 7.2).
	pub slam: [Option<Cue>; 2],
	/// Hero health death (states 0x7b / 0x7c): hero cs9, bank 29.
	pub death: Option<Cue>,
	/// Every hero (class 0) cue 0..80 by class sound index, for the sequence triggers (death sequences
	/// 69 / 113 / 17; decomp/notes/sound.md).
	pub hero_cs: HashMap<usize, Cue>,
	/// Level mechanism sounds by (moby class, class sound index) (game::Event::Sound).
	pub class: HashMap<(u32, usize), Cue>,
}

/// Bomb Glove sounds, all in the level bank (decomp/notes/glove_prereq_leftovers.md, sound table).
#[derive(Debug, Clone, Default)]
pub struct GloveCues {
	/// Throw whoosh (hero cs26, bank 46).
	pub throw: Option<Cue>,
	/// Fire with no ammo (class 192 cs0, bank 154).
	pub empty: Option<Cue>,
	/// Glove drawn (class 192 cs1, bank 155).
	pub draw: Option<Cue>,
	/// Ammo pickup (class 213 cs0, bank 164).
	pub pickup: Option<Cue>,
	/// Small explosion (class 121 cs0 / cs1 gold: bank 104 / 105) and the big one (hero cs22, bank 42).
	pub small: [Option<Cue>; 2],
	pub big: Option<Cue>,
}

/// Number of global defs preceding the footstep defs on Veldin.
const FOOTSTEP_BASE: usize = 2;

/// First footstep def of a level: the global base 2 plus the per-level byte of the retail table at 0x1bdc20
/// (FUN_002a11c8; decomp/notes/sound.md AUD-11g): Veldin 2, Novalis 4.
pub fn footstep_base(level_id: usize) -> usize {
	const TABLE: [usize; 8] = [0, 2, 6, 1, 0, 2, 2, 0];
	FOOTSTEP_BASE + TABLE.get(level_id).copied().unwrap_or(0)
}

/// The class block holding a class's sound defs (class +0x0d count, +0x28 defs): the moby class table's block, or
/// for a class with no core block (the hero's gadgets: wrench 71, glove 192, ...) the WAD-compressed block of the
/// gadget table (core index 0x80; moby_format.md, wrench.md).
fn class_block(level: &Level, class_id: u32) -> R<Option<std::borrow::Cow<'_, [u8]>>> {
	for e in level.table(0x18, 32)? {
		if u32_at(e, 4)? == class_id {
			let start = u32_at(e, 0)? as usize;
			if start != 0 {
				return Ok(Some(std::borrow::Cow::Borrowed(level.block(start)?)));
			}
			break;
		}
	}
	Ok(crate::mobies::gadget_block(level, class_id)?.map(std::borrow::Cow::Owned))
}

/// Loop sound of a class with no skeleton (joint count 0): the loop byte of sequence 0. None for skeleton
/// classes (their loop comes from the animation), classes without sequences, or a loop byte of 0xff.
fn static_loop_sound(level: &Level, class_id: u32) -> R<Option<usize>> {
	let Some(block) = class_block(level, class_id)? else { return Ok(None) };
	if block.len() < 0x4c || block[0x08] != 0 || block[0x0c] == 0 {
		return Ok(None);
	}
	let s = u32_at(&block, 0x48)? as usize;
	if s == 0 {
		return Ok(None);
	}
	let lp = *block.get(s + 0x11).ok_or("short class block")?;
	Ok((lp != 0xff).then_some(lp as usize))
}

/// A class sound as a cue (weapon class sounds use the level bank, sound.md AUD-8b): remapped bank sound plus the class's own def. None when the class sound is unmapped
/// (0xffff) or the class has no such def: PlayClassSound (Novalis FUN_002a0f48) returns -1 for an index at or
/// above the class's sound count or a missing def array, and the game then plays nothing (sound.md).
pub fn class_cue(level: &Level, class_id: u32, idx: usize) -> R<Option<Cue>> {
	let Some(sound) = class_sound(level, class_id, idx)? else { return Ok(None) };
	let Some(block) = class_block(level, class_id)? else { return Ok(None) };
	let n = block.get(0x0d).copied().unwrap_or(0) as usize;
	let defs = u32_at(&block, 0x28).unwrap_or(0) as usize;
	if idx >= n || defs == 0 {
		return Ok(None);
	}
	let Ok(d) = span(&block, defs + idx * 0x20, 0x20) else { return Ok(None) };
	let fall = Falloff::parse(d)?;
	let v = u32_at(d, 12)? as i32;
	let volume = if v > 0 { (v as f32 / 1024.0).min(1.0) } else { 1.0 };
	let bank = level_bank(level)?;
	let looped = bank.sounds.get(sound as usize).map_or(false, |s| s.loops());
	Ok(Some(Cue { sound: sound as usize, volume, looped, fall, bend: bend_range(&bank, sound as usize) }))
}

fn bend_range(bank: &Bank, sound: usize) -> (i8, i8) {
	bank.sounds.get(sound).and_then(|s| s.tones().first().map(|t| (t.bend_down, t.bend_up))).unwrap_or((0, 0))
}

pub fn events(level: &Level) -> R<Events> {
	let mut ev = Events::default();
	let cs = |class: u32, i: usize| class_cue(level, class, i);
	ev.jump = cs(0, 26)?;
	ev.land = cs(0, 30)?;
	ev.hurt = cs(0, 33)?;
	for i in [27, 54] {
		ev.swing.extend(cs(0, i)?);
	}
	for class in 13..=16 {
		if let Some(c) = cs(class, 0)? {
			ev.bolt.insert(class, c);
		}
	}
	for class in [500, 501, 502, 505, 511] {
		if let Some(c) = cs(class, 0)? {
			ev.crate_break.insert(class, c);
		}
	}
	ev.nanotech = cs(501, 1)?;
	// Every cue is a class sound with its own def (gadget classes through the gadget table); an unmapped
	// index plays nothing, as in the game (Novalis maps hero cs9 to 0xffff: no death cue there).
	ev.glove = GloveCues {
		throw: cs(0, 26)?,
		empty: cs(192, 0)?,
		draw: cs(192, 1)?,
		pickup: cs(213, 0)?,
		small: [cs(121, 0)?, cs(121, 1)?],
		big: cs(0, 22)?,
	};
	ev.wrench_throw = cs(0, 27)?;
	ev.wrench_catch = cs(0, 5)?;
	ev.wrench_impact = [cs(71, 0)?, cs(71, 1)?, cs(71, 2)?];
	ev.death = cs(0, 9)?;
	for i in 0..80 {
		if let Some(c) = cs(0, i)? {
			ev.hero_cs.insert(i, c);
		}
	}
	// NOV-13.2: every def the class block declares (+0x0d), not only the first four.
	let counts: HashMap<u32, usize> = class_sound_counts(level)?.into_iter().filter_map(|(c, n)| n.map(|n| (c, n))).collect();
	for &class in crate::game::SOUND_CLASSES {
		// Gadget classes (the held Suck Cannon 849) have no level-table block: their count is the gadget block's.
		let n = match counts.get(&class) {
			Some(&n) => n,
			None => class_block(level, class)?.map_or(4, |b| b.get(0x0d).copied().unwrap_or(0) as usize),
		};
		for i in 0..n {
			if let Some(c) = cs(class, i)? {
				ev.class.insert((class, i), c);
			}
		}
	}
	ev.slam = [cs(0, 53)?, cs(0, 27)?];
	let map = sound_map(level)?;
	let bank = level_bank(level)?;
	for i in 0..16 {
		let d = map.defs.get(footstep_base(level.id) + i).filter(|d| d.bank_sound.is_some());
		ev.footsteps.push(d.map(|d| Cue {
			sound: d.bank_sound.unwrap() as usize,
			volume: (d.max_vol.max(0) as f32 / 1024.0).min(1.0),
			looped: false,
			fall: d.falloff(),
			bend: bend_range(&bank, d.bank_sound.unwrap() as usize),
		}));
	}
	ev.global1 = map.defs.get(1).filter(|d| d.bank_sound.is_some()).map(|d| Cue {
		sound: d.bank_sound.unwrap() as usize,
		volume: (d.max_vol.max(0) as f32 / 1024.0).min(1.0),
		looped: false,
		fall: d.falloff(),
		bend: bend_range(&bank, d.bank_sound.unwrap() as usize),
	});
	ev.ambient = map.defs.iter().skip(FOOTSTEP_BASE + 16).find(|d| d.looped && d.bank_sound.is_some()).map(|d| Cue {
		sound: d.bank_sound.unwrap() as usize,
		volume: (d.max_vol.max(0) as f32 / 1024.0).min(1.0),
		looped: true,
		fall: d.falloff(),
		bend: bend_range(&bank, d.bank_sound.unwrap() as usize),
	});
	// Enemy roles (class sound per animation role; roles from sequence triggers, see notes).
	let toad = [("hit", 0), ("death", 1), ("attack", 2), ("step", 3), ("attack_end", 4), ("land", 6), ("idle", 8)];
	let robot = [("idle", 0), ("death", 1), ("death2", 2), ("hurt", 4), ("chase_loop", 5), ("zap", 6), ("zap2", 8), ("patrol_loop", 10)];
	for &class in crate::game::CUE_CLASSES {
		let n = counts.get(&class).copied().unwrap_or(0);
		for i in 0..n {
			if let Some(c) = cs(class, i)? {
				ev.enemy_cs.insert((class, i), c);
			}
		}
		if let Some(w) = static_loop_sound(level, class)? {
			ev.static_loops.insert(class, w);
		}
	}
	for (class, list) in [(749u32, &toad[..]), (1440, &robot[..])] {
		for &(name, i) in list {
			if let Some(c) = cs(class, i)? {
				ev.enemy.insert((class, name), c);
			}
		}
	}
	ev.menu = (0..8).map(|i| cs(MENU_CLASS, i)).collect::<R<_>>()?;
	Ok(ev)
}

/// The menu panel moby class (14 spawned per page menu); its class sounds are the menu cues.
pub const MENU_CLASS: u32 = 0x472;

/// Retail distance attenuation (Novalis FUN_0029fc10, notes/sound.md): volume (1/1024 units) between
/// the near and far values of the def, linear or quadratic in the distance between `min_range` and
/// `max_range`, truncated.
pub fn attenuate(dist: f32, min_range: f32, max_range: f32, near: i32, far: i32, quadratic: bool) -> i32 {
	if dist <= min_range {
		return near;
	}
	if dist >= max_range {
		return far;
	}
	let (mut a, mut b) = (max_range - dist, max_range - min_range);
	if quadratic {
		a *= a;
		b *= b;
	}
	far + ((near - far) as f32 * a / b) as i32
}

/// Retail pan (FUN_0029fd48): the listener-space offset (x right, y forward) as an angle in degrees,
/// scaled by `clamp(len - 1, 0, 1)` so sounds within one unit stay centred. Returned as -1 (left) to
/// 1 (right), full at 90 degrees and beyond.
pub fn pan_of(x: f32, y: f32) -> f32 {
	let deg = x.atan2(y).to_degrees().trunc() * (x.hypot(y) - 1.0).clamp(0.0, 1.0);
	(deg / 90.0).clamp(-1.0, 1.0)
}

/// Sounds quieter than this (1/1024 units) are not started (FUN_002a0cd0 [C]).
pub const MIN_START_VOLUME: i32 = 0x20;

/// Voice slots (FUN_002a0cd0): 26 normally, 30 for the hero, the camera-attached owner and class 0x472
/// mobys. A start with no free slot is dropped, never evicting a playing voice [C].
pub const VOICE_CAP: usize = 26;
pub const VOICE_CAP_PRIVILEGED: usize = 30;

/// Pitch drop of the underwater listener, 1/128 semitones (about one octave), for defs without flag 8.
pub const UNDERWATER_PITCH: i32 = 0x5f4;

/// Listener velocity for Doppler (FUN_0029ff68): a ring of the last four listener positions; the
/// velocity is the mean of up to three consecutive per-frame deltas, ignoring jumps (teleports) of
/// 1.2 units or more.
#[derive(Debug, Clone, Default)]
pub struct ListenerVelocity {
	/// Newest first.
	ring: [[f32; 3]; 4],
	count: usize,
	/// The listener is below the water surface (camera probe, DAT_00167414) and the hero's water surface z
	/// (0x13f640, the "above the water" reference of the volume halving).
	pub underwater: bool,
	pub water_z: f32,
	/// Ticks pushed so far: the frame counter (DAT_0015f5ac) of the occlusion schedule.
	frame: u32,
	/// Collision geometry of the occlusion and water probes.
	pub scene: OccScene,
	occ: RefCell<OccState>,
}

/// Def flag bits (def byte +0x19): 1 quadratic falloff, 2 no occlusion test, 4 no underwater volume halving,
/// 8 no underwater pitch drop (FUN_0029ff68; sound.md "Voice start, update").
pub const DEF_NO_OCCLUSION: u8 = 2;

/// The collision geometry the occlusion line tests and the camera water probe query: the level's terrain
/// and the moby solids near the hero (refreshed per frame), as `physics::Scene` sees them.
#[derive(Clone, Default)]
pub struct OccScene {
	world: Option<Arc<World>>,
	solids: Arc<Vec<MobySolid>>,
	/// The prop index owning each solid (a voice's owner moby, skipped by its own occlusion lines).
	owners: Arc<Vec<usize>>,
}

impl std::fmt::Debug for OccScene {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		write!(f, "OccScene({} solids)", self.solids.len())
	}
}

impl OccScene {
	#[cfg(test)]
	pub fn set_world(&mut self, w: World) {
		self.world = Some(Arc::new(w));
	}

	/// `set_world`, handing back the previous world (the level switch frees it off the main thread).
	pub fn replace_world(&mut self, w: World) -> Option<Arc<World>> {
		self.world.replace(Arc::new(w))
	}


	/// The solids with the prop index owning each (`Game::solids`).
	pub fn set_solids_owned(&mut self, s: Vec<MobySolid>, owners: Vec<usize>) {
		self.solids = Arc::new(s);
		self.owners = Arc::new(owners);
	}

	/// The solid of prop `prop`, when it has one near the hero.
	pub fn solid_of(&self, prop: usize) -> Option<usize> {
		self.owners.iter().position(|&o| o == prop)
	}

	/// The retail line query FUN_00211770 (no skipped moby).
	pub fn line(&self, from: V3, to: V3, mask: u32) -> Option<Hit> {
		self.line_skip(from, to, mask, None)
	}

	/// The same with the querying moby's own solid skipped (`skip` = solid index).
	pub fn line_skip(&self, from: V3, to: V3, mask: u32, skip: Option<usize>) -> Option<Hit> {
		Scene { world: self.world.as_deref()?, solids: &self.solids }.line(from, to, mask, skip)
	}
}

/// Query mask of every occlusion line: mesh faces only (2), faces with attr bit 7 skipped (0x80).
pub const OCC_MASK: u32 = 0x82;
/// Entries of a voice's line-of-sight history (0x24 bytes at voice +0x44).
pub const OCC_HIST: usize = 36;

/// Per-voice line-of-sight history of the occlusion test (FUN_0029ff68), one byte per entry.
#[derive(Clone, Debug)]
struct OccVoice {
	hist: [u8; OCC_HIST],
	idx: usize,
	frame: u32,
}

#[derive(Clone, Debug, Default)]
struct OccState {
	/// The six start probe points (state-7 start) with the frame they were drawn for; drawn once per frame.
	six: Option<(u32, [V3; 6])>,
	/// The one running-voice probe point, drawn every tick in `push` (FUN_0029ff68 before the voice loop).
	one: V3,
	voices: HashMap<u32, OccVoice>,
}

/// Volume factor of an occlusion history with `blocked` set entries: 0 at 36, `(36 - n) / 18` above 18.
pub fn occlusion_scale(blocked: usize) -> f32 {
	match blocked {
		n if n >= OCC_HIST => 0.0,
		n if n > 18 => (OCC_HIST - n) as f32 / 18.0,
		_ => 1.0,
	}
}

/// FUN_0029fac0: a random point 0.5..6 from the listener (angles uniform over a turn each), pulled back to
/// 0.75 of the way to the first wall it meets.
fn listener_probe(scene: &OccScene, listener: V3, rng: &mut Rng) -> V3 {
	let d = rng.unit_vec_len(0.5, 6.0);
	let p = add(listener, d);
	match scene.line(listener, p, OCC_MASK) {
		Some(h) => add(listener, scale(sub(h.p, listener), 0.75)),
		None => p,
	}
}

/// FUN_0029fb70: whether the line from the listener probe `from` to 0.75 of the way to the voice (the offset
/// clamped to 64 units) meets a face. `skip` is the voice's owner moby solid (*(voice+0x18)); the listener
/// probe skips nothing.
fn voice_blocked(scene: &OccScene, listener: V3, from: V3, at: V3, skip: Option<usize>) -> bool {
	let mut d = scale(sub(at, listener), 0.75);
	let len = length(d);
	if len >= 64.0 {
		d = scale(d, 64.0 / len);
	}
	scene.line_skip(from, add(listener, d), OCC_MASK, skip).is_some()
}

/// What the game state tells the camera water probe (FUN_0020e8f0 and the hero swim exit FUN_00240218).
#[derive(Debug, Clone, Copy, Default)]
pub struct WaterInput {
	/// Normal play (DAT_0015f5a4 == 0): the probe runs; any other mode clears the flag.
	pub gameplay: bool,
	/// The hero is in the underwater group (state 0x11), with its z and its water surface z.
	pub hero_under: bool,
	pub hero_z: f32,
	pub water_z: Option<f32>,
}

impl ListenerVelocity {

	/// The six start probes of this tick (18 draws), once per tick (frame counter), unless already drawn.
	fn fill_six(&self, st: &mut OccState, listener: V3) {
		if st.six.is_none_or(|(f, _)| f != self.frame) {
			let mut rng = Rng::shared();
			let six = std::array::from_fn(|_| listener_probe(&self.scene, listener, &mut rng));
			st.six = Some((self.frame, six));
		}
	}


	/// Volume factor (0..1) of the occlusion test for a voice at `at` heard from `listener` (FUN_0029ff68
	/// with play flag 8, set when the voice is attenuated by distance and its def lacks flag 2). A voice
	/// that has not started (`key` None) takes the six-point start test, six history entries per probe; a
	/// keyed running voice (the key is its slot) shifts one entry per frame: every other frame for a loop,
	/// every fourth for a one-shot, else the previous entry repeats. Without level geometry nothing blocks.
	/// `owner` is the voice's owner prop: its solid is skipped by the voice line (FUN_0029fb70).
	pub fn occlusion(&self, key: Option<u32>, listener: V3, at: V3, def_flags: u8, looped: bool, owner: Option<usize>) -> f32 {
		if def_flags & DEF_NO_OCCLUSION != 0 {
			return 1.0;
		}
		let mut st = self.occ.borrow_mut();
		// The six start probes: once per frame, at the first state-7 (not yet started) voice; a keyed voice that
		// has no history yet also needs them [P: retail never starts one that way].
		let needs_six = key.is_none() || key.is_some_and(|k| !st.voices.contains_key(&k));
		if needs_six {
			self.fill_six(&mut st, listener);
		}
		if self.scene.world.is_none() {
			return 1.0;
		}
		let six = st.six.map_or([[0.0; 3]; 6], |(_, s)| s);
		let one = st.one;
		let skip = owner.and_then(|p| self.scene.solid_of(p));
		let start = || -> [bool; 6] { std::array::from_fn(|i| voice_blocked(&self.scene, listener, six[i], at, skip)) };
		let Some(k) = key else { return occlusion_scale(6 * start().iter().filter(|&&b| b).count()) };
		let v = st.voices.entry(k).or_insert_with(|| {
			let mut hist = [0u8; OCC_HIST];
			for (i, b) in start().iter().enumerate() {
				hist[i * 6..i * 6 + 6].fill(*b as u8);
			}
			OccVoice { hist, idx: 0, frame: self.frame }
		});
		if v.frame != self.frame {
			v.frame = self.frame;
			v.idx = (v.idx + 1) % OCC_HIST;
			let run = if looped { (k ^ self.frame) & 1 != 0 } else { (self.frame ^ k) & 3 == 0 };
			v.hist[v.idx] = if run { voice_blocked(&self.scene, listener, one, at, skip) as u8 } else { v.hist[(v.idx + OCC_HIST - 1) % OCC_HIST] };
		}
		occlusion_scale(v.hist.iter().map(|&b| b as usize).sum())
	}

	/// Forgets the running voices' histories (level load, stop-all).
	pub fn reset_occlusion(&self) {
		*self.occ.borrow_mut() = OccState::default();
	}


	pub fn update_water_with(&mut self, cam: V3, i: WaterInput, grid: &dyn Fn(V3) -> Option<f32>) {
		if !i.gameplay {
			self.underwater = false;
			return;
		}
		if i.hero_under && i.water_z.is_some_and(|w| w + 0.4 < i.hero_z) {
			self.underwater = false;
		}
		let (mut from, to) = ([cam[0], cam[1], cam[2] + 0.75], [cam[0], cam[1], cam[2] - 0.75]);
		for _ in 0..6 {
			let Some(h) = self.scene.line(from, to, 0x12) else { return };
			if h.kind & 0x1f != 0 {
				from = [h.p[0], h.p[1], h.p[2] - 0.01];
				continue;
			}
			self.underwater = grid(h.p).unwrap_or(h.p[2]) + 0.04 > cam[2];
			return;
		}
	}

	/// Volume factor and pitch shift (1/128 semitones) of a positional voice at `at` moving `voice_vel`
	/// per frame (FUN_0029ff68): Doppler, plus underwater: one octave down unless def flag 8, and half
	/// volume for voices above the surface unless def flag 4.
	pub fn effects(&self, listener: [f32; 3], at: [f32; 3], voice_vel: [f32; 3], flags: u8) -> (f32, i32) {
		let mut shift = doppler(listener, self.velocity(), at, voice_vel);
		let mut vol = 1.0;
		if self.underwater {
			if flags & 8 == 0 {
				shift -= UNDERWATER_PITCH;
			}
			if flags & 4 == 0 && at[2] > self.water_z {
				vol = 0.5;
			}
		}
		(vol, shift)
	}

	pub fn velocity(&self) -> [f32; 3] {
		let (mut sum, mut n) = ([0f32; 3], 0u32);
		for i in 0..self.count.saturating_sub(1) {
			let d = [self.ring[i][0] - self.ring[i + 1][0], self.ring[i][1] - self.ring[i + 1][1], self.ring[i][2] - self.ring[i + 1][2]];
			if (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() < crate::gameplay::PAL_DT * 60.0 {
				for k in 0..3 {
					sum[k] += d[k];
				}
				n += 1;
			}
		}
		if n > 1 {
			for v in &mut sum {
				*v /= n as f32;
			}
		}
		sum
	}
}

/// Doppler pitch shift in 1/128 semitones of a voice at `voice` moving `voice_vel` per frame, heard
/// by a listener at `listener` moving `listener_vel` (FUN_0029ff68): `trunc(dot(dir, relvel) * 300)`
/// rescaled by 0x5f4 / 0x2e5.
pub fn doppler(listener: [f32; 3], listener_vel: [f32; 3], voice: [f32; 3], voice_vel: [f32; 3]) -> i32 {
	let mut dir = [listener[0] - voice[0], listener[1] - voice[1], listener[2] - voice[2]];
	let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
	if len < 1e-6 {
		return 0;
	}
	for v in &mut dir {
		*v /= len;
	}
	let rel = [voice_vel[0] - listener_vel[0], voice_vel[1] - listener_vel[1], voice_vel[2] - listener_vel[2]];
	let d = dir[0] * rel[0] + dir[1] * rel[1] + dir[2] * rel[2];
	(d * 300.0) as i32 * 0x5f4 / 0x2e5
}

/// Rate multiplier of a pitch shift in 1/128 semitones. The SPU2 clamps pitch at 0x3fff (4x 48 kHz).
pub fn pitch_ratio(shift_128ths: i32) -> f32 {
	2f32.powf(shift_128ths as f32 / (128.0 * 12.0))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{iso, toc};
	use std::path::{Path, PathBuf};

	const ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");
	const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/target/test_out/sounds");

	fn write_wav(path: &Path, s: &Sample) {
		let mut b = Vec::new();
		let n = (s.pcm.len() * 2) as u32;
		b.extend(b"RIFF");
		b.extend((36 + n).to_le_bytes());
		b.extend(b"WAVEfmt ");
		b.extend(16u32.to_le_bytes());
		b.extend(1u16.to_le_bytes());
		b.extend(1u16.to_le_bytes());
		b.extend(s.rate.to_le_bytes());
		b.extend((s.rate * 2).to_le_bytes());
		b.extend(2u16.to_le_bytes());
		b.extend(16u16.to_le_bytes());
		b.extend(b"data");
		b.extend(n.to_le_bytes());
		for v in &s.pcm {
			b.extend(v.to_le_bytes());
		}
		std::fs::write(path, b).unwrap();
	}

	/// The HelpDesk voice stream decodes for English and is absent past the table (ingame_menu_leftovers.md).
	#[test]
	fn help_voice_decodes() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let (rate, pcm) = (0..150).find_map(|v| help_voice(&mut disc, 0, v).unwrap()).expect("an English help voice");
		assert!((8000..=48000).contains(&rate) && pcm.len() > rate as usize / 2, "rate {rate}, {} samples", pcm.len());
		assert!(help_voice(&mut disc, 0, 150).unwrap().is_none());
	}

	/// AUD-11a.fan: only the skeleton-less fan has a static loop sound.
	#[test]
	fn novalis_fan_static_loop() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let nov = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&nov).unwrap();
		assert_eq!(ev.static_loops.get(&705), Some(&0));
		assert_eq!(ev.enemy_cs[&(705, 0)].sound, 227);
		for c in [459, 577, 660] {
			assert!(!ev.static_loops.contains_key(&c));
		}
		assert_eq!(ev.static_loops.len(), 1);
		assert_eq!(static_loop_sound(&nov, 705).unwrap(), Some(0));
		assert_eq!(static_loop_sound(&nov, 459).unwrap(), None);
		let vel = Level::load(&mut disc, &levels[0]).unwrap();
		let ev = events(&vel).unwrap();
		assert!(ev.static_loops.keys().all(|c| crate::game::CUE_CLASSES.contains(c)));
	}

	/// Aridia enemies (shark 580, constructorbot 612, nest 668) resolve their sequence-trigger and event cues through
	/// `enemy_cs`; before they were in CUE_CLASSES the level was silent for them.
	#[test]
	fn aridia_enemy_cue_classes() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[2]).unwrap();
		let ev = events(&l).unwrap();
		for (c, n) in [(580u32, 7usize), (612, 10), (668, 5)] {
			assert!(crate::game::CUE_CLASSES.contains(&c), "{c}");
			assert!((0..n).any(|i| ev.enemy_cs.contains_key(&(c, i))), "{c}");
		}
		assert!(ev.enemy_cs.contains_key(&(580, 4)));
	}

	/// AUD-11a: Novalis animation cue classes resolve through `enemy_cs`; triggers and loop bytes parse.
	#[test]
	fn novalis_cue_classes() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		for (k, snd) in [((459, 4), 165), ((577, 4), 193), ((572, 0), 183), ((865, 4), 258), ((866, 0), 261), ((705, 0), 227), ((604, 0), 201), ((790, 0), 244)] {
			assert_eq!(ev.enemy_cs[&k].sound, snd, "{k:?}");
		}
		let cues = |class: u32, seq: usize| {
			let a = crate::anim::animation(&l, class).unwrap().unwrap();
			let s = &a.sequences[seq];
			(s.triggers.iter().map(|w| ((w & 0xffff) as usize, (w >> 16) as usize)).collect::<Vec<_>>(), s.loop_sound)
		};
		assert_eq!(cues(459, 1).0, vec![(2, 24), (1, 136)]);
		assert_eq!(cues(459, 8).1, 6);
		assert_eq!(cues(577, 5).0, vec![(6, 56), (6, 128), (1, 176), (0, 184)]);
		assert_eq!(cues(572, 1).1, 4);
	}

	/// Parsed trigger words and loop bytes match the cues read from the decomp (sound.md "Animation sound cues").
	#[test]
	fn enemy_animation_cues() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		let cues = |class: u32, seq: usize| {
			let a = crate::anim::animation(&l, class).unwrap().unwrap();
			let s = &a.sequences[seq];
			(s.triggers.iter().map(|w| ((w & 0xffff) as usize, (w >> 16) as usize)).collect::<Vec<_>>(), s.loop_sound)
		};
		assert_eq!(cues(749, 1).0, vec![(8, 40)]);
		assert_eq!(cues(749, 5).0, vec![(6, 64), (0, 160)]);
		assert_eq!(cues(749, 6).0, vec![(2, 8), (4, 192), (1, 288)]);
		assert_eq!(cues(1440, 2).0, vec![(0, 1776)]);
		assert_eq!(cues(1440, 7).0, vec![(6, 8), (8, 304)]);
		assert_eq!(cues(1440, 10).0, vec![(1, 8), (2, 256), (3, 624)]);
		assert_eq!(cues(1440, 6).1, 5);
		assert_eq!(cues(1440, 9).1, 10);
	}

	#[test]
	fn veldin_events() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		let ev = events(&l).unwrap();
		println!("{ev:#?}");
		assert_eq!(ev.jump.unwrap().sound, 46);
		assert_eq!(ev.hurt.unwrap().sound, 53);
		assert_eq!(ev.swing.iter().map(|c| c.sound).collect::<Vec<_>>(), vec![47, 72]);
		assert_eq!(ev.bolt.get(&13).unwrap().sound, 91);
		assert_eq!(ev.crate_break[&500].sound, 179);
		assert_eq!(ev.crate_break[&501].sound, 180);
		assert_eq!(ev.crate_break[&511].sound, 182);
		assert_eq!(ev.nanotech.unwrap().sound, 181);
		assert!(ev.slam.iter().all(|c| c.is_some()), "{:?}", ev.slam);
		assert_eq!(ev.footsteps.len(), 16);
		assert!(ev.footsteps.iter().all(|c| c.is_some()));
		assert!(ev.ambient.map_or(false, |c| c.looped));
		assert_eq!(ev.enemy[&(749, "idle")].sound, 205);
		assert_eq!(ev.enemy[&(1440, "death")].sound, 224);
		for i in 0..9 {
			assert_eq!(ev.enemy_cs[&(749, i)].sound, 197 + i);
		}
		for i in 0..11 {
			assert_eq!(ev.enemy_cs[&(1440, i)].sound, 223 + i);
		}
		for c in ev.footsteps.iter().flatten().chain(ev.jump.iter()) {
			assert!(c.volume > 0.0 && c.volume <= 1.0, "{c:?}");
		}
		// Ground sound classes under the level's collision (class 3 plays as 0).
		let mut hist = [0usize; 4];
		for f in crate::collision::collision(&l).unwrap() {
			hist[((f.kind >> 5) & 3) as usize] += 1;
		}
		println!("collision sound-class histogram {hist:?}");
		// Write a few decoded events for listening / duration checks.
		let samples = level_sounds(&l).unwrap();
		let dir = PathBuf::from(OUT).parent().unwrap().join("sfx");
		std::fs::create_dir_all(&dir).unwrap();
		for (name, c) in [("jump", ev.jump), ("hurt", ev.hurt), ("bolt", ev.bolt.get(&13).copied()), ("crate", ev.crate_break.get(&500).copied()), ("nanotech", ev.nanotech), ("step0", ev.footsteps[0])] {
			let c = c.unwrap();
			let sm = &samples[c.sound];
			println!("{name}: sound {} {:.2}s", c.sound, sm.pcm.len() as f32 / sm.rate as f32);
			write_wav(&dir.join(format!("{name}.wav")), sm);
		}
	}
	/// Layout-B tones store the centre note as a negative signed byte: 989snd negates it
	/// (IOP FUN_0001600c) before sceSdNote2Pitch. Sound 19 (Veldin ambient loop) is centre 86.
	#[test]
	fn negative_centre_note() {
		let mut p = [0u8; 32];
		p[2] = 0xaa;
		p[3] = 0x42;
		let t = Grain { kind: GRAIN_TONE, delay: 0, params: p }.tone().unwrap();
		assert_eq!((t.centre_note, t.centre_fine), (86, 0x42));
		let r = t.rate();
		assert_eq!(r, 11016, "pitch 940 (centre 86 fine 66 ADDED)");
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		let bank = level_bank(&l).unwrap();
		let s = bank.sound_sample(19).unwrap().unwrap();
		println!("sound 19: rate {} len {} samples = {:.2}s loop {:?}", s.rate, s.pcm.len(), s.pcm.len() as f32 / s.rate as f32, s.loop_start);
		assert_eq!(s.rate, 11016);
	}

	/// `Tone::rate` = the retail driver's pitch: golden (centre byte, centre fine, note, fine, PITCH) rows replayed from the
	/// libsd/IOP MIPS (oracle copy snd989/pitch.rs `TONE_PITCH_GOLDEN`, scratch_grain/pitch_sim.py), plus unity and the
	/// centre-fine-is-ADDED rule.
	#[test]
	fn tone_pitch_matches_the_retail_driver() {
		const GOLDEN: &[(i8, i8, i16, i16, u16)] = &[
		(60, 0, 60, 0, 3763),
		(60, 0, 48, 127, 1992),
		(60, 0, 30, -3, 664),
		(60, 1, 72, -50, 7361),
		(60, 1, 127, 127, 23893),
		(60, 64, 60, -100, 3702),
		(60, 64, 0, 0, 121),
		(60, 125, 60, -1, 3980),
		(60, 125, 59, -127, 3549),
		(48, 0, 60, 0, 7526),
		(48, 0, 48, 127, 3985),
		(48, 0, 30, -3, 1328),
		(48, 1, 72, -50, 14722),
		(48, 1, 127, 127, 23893),
		(48, 64, 60, -100, 7405),
		(48, 64, 0, 0, 242),
		(48, 125, 60, -1, 7959),
		(48, 125, 59, -127, 7097),
		(72, 0, 60, 0, 1881),
		(72, 0, 48, 127, 995),
		(72, 0, 30, -3, 332),
		(72, 1, 72, -50, 3680),
		(72, 1, 127, 127, 23893),
		(72, 64, 60, -100, 1851),
		(72, 64, 0, 0, 60),
		(72, 125, 60, -1, 1990),
		(72, 125, 59, -127, 1774),
		(36, 0, 60, 0, 15052),
		(36, 0, 48, 127, 7970),
		(36, 0, 30, -3, 2657),
		(36, 1, 72, -50, 29445),
		(36, 1, 127, 127, 23893),
		(36, 64, 60, -100, 14810),
		(36, 64, 0, 0, 484),
		(36, 125, 60, -1, 15918),
		(36, 125, 59, -127, 14194),
		(0, 0, 60, 0, 15052),
		(0, 0, 48, 127, 15940),
		(0, 0, 30, -3, 21258),
		(0, 1, 72, -50, 29445),
		(0, 1, 127, 127, 23893),
		(0, 64, 60, -100, 29619),
		(0, 64, 0, 0, 3873),
		(0, 125, 60, -1, 15918),
		(0, 125, 59, -127, 28389),
		(12, 0, 60, 0, 15052),
		(12, 0, 48, 127, 15940),
		(12, 0, 30, -3, 10629),
		(12, 1, 72, -50, 29445),
		(12, 1, 127, 127, 23893),
		(12, 64, 60, -100, 29619),
		(12, 64, 0, 0, 1936),
		(12, 125, 60, -1, 15918),
		(12, 125, 59, -127, 28389),
		(127, 0, 60, 0, 78),
		(127, 0, 48, 127, 41),
		(127, 0, 30, -3, 13),
		(127, 1, 72, -50, 153),
		(127, 1, 127, 127, 3987),
		(127, 64, 60, -100, 77),
		(127, 64, 0, 0, 2),
		(127, 125, 60, -1, 82),
		(127, 125, 59, -127, 74),
		(-60, 0, 60, 0, 4096),
		(-60, 0, 48, 127, 2169),
		(-60, 0, 30, -3, 723),
		(-60, 1, 72, -50, 8013),
		(-60, 1, 127, 127, 26007),
		(-60, 64, 60, -100, 4030),
		(-60, 64, 0, 0, 132),
		(-60, 125, 60, -1, 4332),
		(-60, 125, 59, -127, 3863),
		(-48, 0, 60, 0, 8192),
		(-48, 0, 48, 127, 4338),
		(-48, 0, 30, -3, 1446),
		(-48, 1, 72, -50, 16025),
		(-48, 1, 127, 127, 26007),
		(-48, 64, 60, -100, 8060),
		(-48, 64, 0, 0, 264),
		(-48, 125, 60, -1, 8663),
		(-48, 125, 59, -127, 7725),
		(-1, 0, 60, 0, 30928),
		(-1, 0, 48, 127, 32752),
		(-1, 0, 30, -3, 21840),
		(-1, 1, 72, -50, 30251),
		(-1, 1, 127, 127, 24548),
		(-1, 64, 60, -100, 30429),
		(-1, 64, 0, 0, 3979),
		(-1, 125, 60, -1, 32707),
		(-1, 125, 59, -127, 29165),
		(29, 117, 29, 20, 4002),
		(57, 26, 43, 26, 1716),
		(100, 83, 81, -21, 1291),
		(60, 78, -18, 87, 45),
		(12, 115, 16, 8, 5011),
		(-5, 18, 55, 58, 19031),
		(86, 37, 144, -105, 26008),
		(-13, 83, 52, 48, 20670),
		(-54, 70, 149, 12, 32094),
		(-69, 5, 11, -44, 141),
		(25, 35, 150, -45, 20002),
		(-33, 6, -1, -21, 571),
		(-113, 87, -16, 31, 3),
		(2, 105, 82, -43, 24571),
		(-8, 69, -13, 60, 1291),
		(7, 122, 106, 110, 19876),
		(61, 27, 56, -8, 2843),
		(-15, 19, 76, -85, 16848),
		(34, 64, 46, -11, 7708),
		(77, 99, 62, 28, 1675),
		(-5, 42, 148, 78, 32649),
		(124, 120, 92, 120, 660),
		(42, 110, 110, -104, 23958),
		(36, 115, 137, -122, 20028),
		(-15, 101, 120, -124, 27269),
		(-10, 72, 117, 62, 16428),
		(-104, 42, 75, 94, 816),
		(72, 55, 132, -30, 15222),
		(-60, 0, -15, 68, 55),
		(-112, 108, 25, 58, 29),
		(-57, 46, -17, 32, 59),
		(114, 108, 144, -63, 21723),
		(-17, 118, 76, 31, 16540),
		(-80, 90, 30, 28, 241),
		(-43, 103, 130, -97, 19535),
		(7, 47, 101, -57, 26698),
		(112, 3, 62, -124, 198),
		(-119, 38, 74, -25, 306),
		(-91, 112, 147, 16, 27554),
		(-24, 37, 84, -62, 32399),
		(124, 121, 17, 98, 8),
		(-125, 117, 100, 41, 1038),
		(48, 78, 39, -76, 2239),
		(99, 58, 20, -54, 39),
		(-87, 122, -2, -53, 25),
		(-23, 98, 16, -94, 2739),
		(3, 40, -6, 86, 2368),
		(-13, 35, -12, 52, 1005),
		(13, 10, 131, 89, 28045),
		(-36, 33, 104, 108, 27715),
		(7, 0, 151, 72, 15549),
		(24, 23, 86, -10, 16994),
		(-77, 20, 54, 104, 1147),
		(6, 15, 10, 77, 4941),
		(-38, 2, 17, 7, 1223),
		(110, 35, 114, 63, 4955),
		(92, 6, 70, -55, 1032),
		(-60, 16, 97, 20, 17641),
		(-61, 39, 16, -30, 306),
		(3, 14, 11, 75, 6218),
		(118, 63, 54, -89, 91),
		(-122, 74, 72, 111, 248),
		(8, 38, -19, -63, 781),
		(62, 64, 105, -93, 22259),
		(12, 84, 16, -107, 4692),
		(-122, 106, 3, -84, 4),
		(119, 52, 49, 33, 68),
		(-47, 9, 95, -61, 32007),
		(-64, 30, 36, 41, 839),
		(-77, 30, 23, -11, 183),
		(34, 7, -17, 115, 208),
		(-75, 96, 82, 68, 6608),
		(-19, 72, 115, -84, 32589),
		(-31, 64, 20, -47, 2186),
		(-82, 110, 72, -29, 2384),
		];
		for &(c, cf, note, fine, want) in GOLDEN {
			assert_eq!(tone_pitch(c, cf, note as i32, fine as i32), want, "centre {c} cf {cf} note {note} fine {fine}");
		}
		// a 48 kHz tone centred on the SFX note plays at exactly 48000 Hz; one octave off doubles / halves it
		let tone = |centre: i8, fine: u8| Tone {
			volume: 0x7f,
			centre_note: centre.unsigned_abs(),
			centre_fine: fine,
			native44: centre >= 0,
			bend_down: 2,
			bend_up: 2,
			reverb: false,
			vag_offset: 0,
		};
		assert_eq!(tone(-60, 0).rate(), 48000);
		assert_eq!(tone(-48, 0).rate(), 96000);
		assert_eq!(tone(-72, 0).rate(), 24000);
		assert_eq!(tone(60, 0).rate(), 44098); // pitch 3763 (0x1000 * 44100 / 48000, truncated) * 48000 / 0x1000
		// the centre fine is ADDED: +64/128 semitone is higher, and equals playing 64 fine up
		assert!(tone(-60, 64).rate() > 48000);
		assert_eq!(tone_pitch(-60, 64, 60, 0), tone_pitch(-60, 0, 60, 64));
		// the SPU2 caps PITCH at 0x3fff
		assert_eq!(tone(-1, 0).rate(), 0x3fff * 48000 / 0x1000);
	}

	#[test]
	fn falloff_level_and_bend() {
		let f = Falloff { min_range: 2.0, max_range: 10.0, min_vol: 0, max_vol: 1000, min_pitch: -100, max_pitch: 100, flags: 1 };
		assert_eq!(f.level(1.0), 1000);
		assert_eq!(f.level(6.0), 250);
		assert_eq!(f.level(9.9), 0); // below the 0x20 start threshold
		assert_eq!(f.bend(|n| n - 1), 99);
		assert_eq!(Falloff { max_pitch: -100, ..f }.bend(|_| unreachable!()), -100);
		let cue = Cue { sound: 0, volume: 1.0, looped: false, fall: f, bend: (127, 127) };
		assert_eq!(cue.random_shift(|n| n - 1), bend_term((127, 127), 99));
		assert!((cue.level(6.0) - 250.0 / 1024.0).abs() < 1e-6);
	}

	/// Every gameplay cue is a class sound with its own def (no bank-number fallbacks): the gadget classes take
	/// their defs from the gadget table, and an unmapped sound plays nothing (Novalis hero cs9 = 0xffff).
	#[test]
	fn cues_come_from_defs() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for id in [0usize, 1] {
			let l = Level::load(&mut disc, &levels[id]).unwrap();
			let ev = events(&l).unwrap();
			let g = &ev.glove;
			for (name, c) in [("throw", &g.throw), ("empty", &g.empty), ("draw", &g.draw), ("pickup", &g.pickup), ("small0", &g.small[0]), ("small1", &g.small[1]), ("big", &g.big), ("wthrow", &ev.wrench_throw), ("wcatch", &ev.wrench_catch), ("imp0", &ev.wrench_impact[0]), ("imp1", &ev.wrench_impact[1]), ("imp2", &ev.wrench_impact[2])] {
				let c = c.as_ref().unwrap_or_else(|| panic!("level {id} cue {name} missing"));
				assert!(c.fall.max_range > 0.0, "level {id} cue {name} has an empty def");
			}
			// The glove classes' defs come from the gadget table, not a core class block.
			assert_eq!(ev.death.is_some(), id == 0, "level {id} death cue");
		}
	}

	// AUD-11d test 2: Novalis hero cs 26 / 30 / 33 are bank 42 / 44 / 46.
	#[test]
	fn novalis_hero_trigger_sounds() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		assert_eq!((ev.hero_cs[&26].sound, ev.hero_cs[&30].sound, ev.hero_cs[&33].sound), (42, 44, 46));
	}

	// AUD-11g test 1: the footstep def base per level, and the Novalis def values.
	#[test]
	fn footstep_defs_per_level() {
		assert_eq!((footstep_base(0), footstep_base(1)), (2, 4));
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let nov = events(&Level::load(&mut disc, &levels[1]).unwrap()).unwrap();
		let f = |i: usize| nov.footsteps[i].unwrap();
		assert_eq!(f(0).sound, 4);
		assert!((f(0).volume - 655.0 / 1024.0).abs() < 1e-6);
		assert_eq!(f(12).sound, 16);
		assert_eq!(f(12).volume, 1.0);
		let vel = events(&Level::load(&mut disc, &levels[0]).unwrap()).unwrap();
		assert_eq!(vel.footsteps[0].unwrap().sound, 2);
		// ARI-2 (decomp/notes/aridia_environment.md): Aridia table byte 6, defs 8..23.
		assert_eq!(footstep_base(2), 8);
		let ari = events(&Level::load(&mut disc, &levels[2]).unwrap()).unwrap();
		assert!((0..16).all(|i| ari.footsteps[i].unwrap().sound == 8 + i));
	}

	// AUD-8b test 1: the wrench impact class 71 cs2 resolves to a level-bank sound on both levels.
	#[test]
	fn wrench_impact_uses_level_bank() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for i in [0, 1] {
			let l = Level::load(&mut disc, &levels[i]).unwrap();
			let bank = level_bank(&l).unwrap();
			let c = class_cue(&l, 71, 2).unwrap().unwrap();
			assert!(bank.sounds.get(c.sound).is_some(), "level {i}");
		}
	}

	// Pause-menu cues: class 0x472 cs0..7 through each level's remap (Veldin, Novalis, Aridia), and the fixed
	// pitch of cue 3 (+0xfe) and cue 4 (+0x1fc) through the tone bend range 127 (1/128 semitones).
	#[test]
	fn menu_cue_remap_per_level() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for (i, first) in [(0, 213), (1, 272), (2, 262)] {
			let l = Level::load(&mut disc, &levels[i]).unwrap();
			let ev = events(&l).unwrap();
			let got: Vec<usize> = ev.menu.iter().map(|c| c.as_ref().unwrap().sound).collect();
			assert_eq!(got, (first..first + 8).collect::<Vec<_>>(), "level {i}");
			let shift = |k: usize| ev.menu[k].as_ref().unwrap().random_shift(|_| unreachable!());
			assert_eq!((shift(0), shift(1), shift(3), shift(4)), (0, 0, 126, 252), "level {i}");
		}
	}

	// AUD-11e test 1: Novalis hero remap for the direct cues.
	#[test]
	fn novalis_hero_direct_cue_sounds() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		for (cs, bank) in [(3, 25), (6, 28), (7, 29), (8, 30), (0xc, 32), (0x13, 37), (0x17, 39)] {
			assert_eq!(ev.hero_cs[&cs].sound, bank, "cs {cs}");
		}
	}

	// AUD-11c test 1: the Suck Cannon capture sound (class 849 cs5) is bank 252 on Novalis.
	#[test]
	fn suck_cannon_capture_sound() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		assert_eq!(ev.class[&(849, 5)].sound, 252);
	}

	// WPN-6.9d4 test (3): every held rig class with sequence triggers resolves its cues (gadget block counts, Novalis banks).
	#[test]
	fn held_class_sounds() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		let want: [(u32, &[(usize, usize)]); 8] = [
			(175, &[(6, 114), (7, 115)]),
			(168, &[(2, 102), (3, 103), (4, 104)]),
			(157, &[(0, 93), (1, 94), (2, 95), (3, 96)]),
			(163, &[(0, 97), (1, 98), (2, 99)]),
			(180, &[(2, 127)]),
			(190, &[(1, 138)]),
			(229, &[(0, 151), (1, 152)]),
			(185, &[(1, 129), (2, 130), (3, 131)]),
		];
		for (class, cues) in want {
			assert!(crate::game::SOUND_CLASSES.contains(&class), "{class}");
			for &(cs, bank) in cues {
				assert_eq!(ev.class[&(class, cs)].sound, bank, "{class} cs {cs}");
			}
		}
	}

	// NOV-13.2 test 1: all eight vendor defs are registered (class block +0x0d), cs i -> Novalis bank 68 + i.
	#[test]
	fn vendor_class_sounds_all_eight() {
		let mut disc = iso::Disc::open(Path::new(ISO)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let ev = events(&l).unwrap();
		assert_eq!(ev.class[&(11, 7)].sound, 75);
		assert_eq!(ev.class[&(11, 4)].sound, 72);
		for i in 0..8 {
			assert_eq!(ev.class[&(11, i)].sound, 68 + i);
			assert_eq!(ev.class[&(11, i)].volume, 1.0);
		}
	}

	/// A wall quad (x = `x`, 200 wide) as two triangles, wound one way or the other.
	fn wall(x: f32, flip: bool, kind: u8) -> World {
		let (a, b, c, d) = ([x, -100.0, -100.0], [x, 100.0, -100.0], [x, 100.0, 100.0], [x, -100.0, 100.0]);
		let tris = if flip { [([a, c, b], kind), ([a, d, c], kind)] } else { [([a, b, c], kind), ([a, c, d], kind)] };
		World::typed(tris, 4.0)
	}

	fn scene_with(w: World) -> ListenerVelocity {
		let mut l = ListenerVelocity::default();
		l.scene.set_world(w);
		l
	}

	#[test]
	fn owner_solid_is_skipped() {
		let cam = [0.0, 0.0, 1.0];
		let at = [8.0, 0.0, 1.0];
		let mut l = scene_with(World::new(std::iter::empty::<[V3; 3]>(), 4.0));
		let solid = |flip: bool| {
			let (a, b, c) = ([4.0, -10.0, -10.0], [4.0, 10.0, -10.0], [4.0, 0.0, 10.0]);
			MobySolid { tris: vec![(if flip { [a, c, b] } else { [a, b, c] }, 31)], ..Default::default() }
		};
		let blocked = |l: &ListenerVelocity, owner| (0..8).map(|_| l.occlusion(None, cam, at, 0, false, owner)).fold(1.0f32, f32::min);
		let mut found = false;
		for flip in [false, true] {
			l.scene.set_solids_owned(vec![solid(flip)], vec![5]);
			l.reset_occlusion();
			if blocked(&l, None) < 1.0 {
				found = true;
				l.reset_occlusion();
				assert_eq!(blocked(&l, Some(5)), 1.0, "owner skip");
				assert!(blocked(&l, Some(6)) < 1.0, "other owner still occludes");
			}
		}
		assert!(found);
	}

	/// A horizontal quad at height `z` of face type `kind` (two-sided queries only).
	fn wall_z(z: f32, kind: u8) -> World {
		let (a, b, c, d) = ([-100.0, -100.0, z], [100.0, -100.0, z], [100.0, 100.0, z], [-100.0, 100.0, z]);
		World::typed([([a, b, c], kind), ([a, c, d], kind)], 4.0)
	}

	#[test]
	fn attenuation_and_pan() {
		assert_eq!(attenuate(1.0, 2.0, 10.0, 1000, 0, false), 1000);
		assert_eq!(attenuate(10.0, 2.0, 10.0, 1000, 0, false), 0);
		assert_eq!(attenuate(6.0, 2.0, 10.0, 1000, 0, false), 500);
		assert_eq!(attenuate(6.0, 2.0, 10.0, 1000, 0, true), 250);
		assert_eq!(pan_of(0.5, 0.0), 0.0);
		assert_eq!(pan_of(0.0, 5.0), 0.0);
		assert!((pan_of(5.0, 0.0) - 1.0).abs() < 1e-6);
		assert!(pan_of(-3.0, 3.0) < -0.4);
	}

}
