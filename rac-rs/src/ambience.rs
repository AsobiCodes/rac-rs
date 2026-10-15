//! Positional ambient sound emitters (gameplay header +0x0c). Decoded in decomp/notes/ambience.md.
//!
//! Each record is a 0x90-byte moby-shaped struct; the game spawns it as a sound-emitter moby whose
//! update (overlay FUN_002ed2f8 for spheres, FUN_002ed480 for boxes) starts a level sound def
//! (FUN_0028d780) when the listener is inside its volume.

use crate::bytes::{f32_at, span, u16_at, u32_at, R};
use crate::level::Level;
use crate::sound::{sound_map, Falloff};

/// Gameplay +0x0c: (count, pad x3) then 0x90-byte records.
const EMITTERS: usize = 0x0c;
/// Pvar table entry layout used by emitters: i32 def, i32 min interval s, i32 max interval s, i32 timer, i32 handle.
const PVAR_TABLE: usize = 0x54;
const PVAR_DATA: usize = 0x58;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Shape {
	/// Record type 0: audible while the listener is within `trigger_radius` of `pos` (stops beyond +1).
	Sphere,
	/// Record type 2: oriented box; the sound is started at a random point inside it.
	Cuboid,
}

#[derive(Debug, Clone)]
pub struct Emitter {
	/// Emitter centre (record +0x40).
	pub pos: [f32; 3],
	/// Bank sound the def resolves to (None = unmapped).
	pub bank_sound: Option<usize>,
	/// Def falloff ranges: full volume inside `min_range`, silent (min_volume) beyond `max_range`.
	pub min_range: f32,
	pub max_range: f32,
	/// Def max volume (full volume inside min_range), 1.0 = 0x400.
	pub volume: f32,
	/// Def min volume (beyond max_range), 1.0 = 0x400.
	pub min_volume: f32,
	/// Def flag bit 0: falloff is quadratic instead of linear.
	pub quadratic: bool,
	/// The def's flag byte (values 1 quadratic, 2 no occlusion test, 4 no underwater halving, 8 no underwater
	/// pitch drop; sound.md).
	pub flags: u8,
	/// Never set: emitters are panned (play flag 0x1 clear); def flag 2 only exempts the occlusion test.
	pub no_pan: bool,
	pub shape: Shape,
	/// Record +0xc: sphere radius / box extent scale.
	pub trigger_radius: f32,
	pub inverse: [[f32; 3]; 3],
}

fn row(r: &[u8], o: usize) -> R<[f32; 3]> {
	Ok([f32_at(r, o)?, f32_at(r, o + 4)?, f32_at(r, o + 8)?])
}

/// Sound-instance record types (dispatch table 0x20c480, selected by FUN_002a12d8 from the record's
/// first u16): 0 sphere 0x3190d0, 1 volume box 0x319258, 2 random-point box 0x3195f8, 3 reverb box
/// 0x319848, 5 underwater 0x3199a8, 6 music zone 0x319a58 (ambience.md Novalis section).
pub const TYPE_SPHERE: u16 = 0;
pub const TYPE_VOLUME: u16 = 1;
pub const TYPE_RANDOM: u16 = 2;
pub const TYPE_REVERB: u16 = 3;
pub const TYPE_UNDERWATER: u16 = 5;
pub const TYPE_MUSIC: u16 = 6;

/// The sound-instance records (type, record, pvar bytes) of a level.
fn records(level: &Level) -> R<Vec<(u16, &[u8], &[u8])>> {
	let g = &level.gameplay[..];
	let start = u32_at(g, EMITTERS)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	let table = u32_at(g, PVAR_TABLE)? as usize;
	let data = u32_at(g, PVAR_DATA)? as usize;
	let mut out = Vec::new();
	for r in span(g, start + 16, count * 0x90)?.chunks_exact(0x90) {
		let pv = u32_at(r, 8)? as usize;
		let off = u32_at(g, table + pv * 8)? as usize;
		let size = u32_at(g, table + pv * 8 + 4)? as usize;
		out.push((u16_at(r, 0)?, r, span(g, data + off, size)?));
	}
	Ok(out)
}

/// The positional emitters the viewer runs: sphere loops and random-point boxes (types 0 and 2).
/// Volume boxes, reverb, underwater and music zones are other record types (see `zones`).
pub fn emitters(level: &Level) -> R<Vec<Emitter>> {
	let map = sound_map(level)?;
	let mut out = Vec::new();
	for (ty, r, p) in records(level)? {
		let shape = match ty {
			TYPE_SPHERE => Shape::Sphere,
			TYPE_RANDOM => Shape::Cuboid,
			_ => continue,
		};
		let def = u32_at(p, 0)? as usize;
		let d = map.defs.get(def).ok_or("emitter sound def out of range")?;
		out.push(Emitter {
			pos: row(r, 0x40)?,
			bank_sound: d.bank_sound.map(|b| b as usize),
			min_range: d.min_range,
			max_range: d.max_range,
			volume: d.max_vol as f32 / 1024.0,
			min_volume: d.min_vol as f32 / 1024.0,
			quadratic: d.flags & 1 != 0,
			flags: d.flags,
			// Pan is gated by the voice's play flag bit 0 (clear for emitters), not the def: def flag
			// bit 1 only enables the occlusion test (corrected 2026-10-04: was no_pan = def flag bit 1).
			no_pan: false,
			shape,
			trigger_radius: f32_at(r, 0xc)?,
			inverse: [row(r, 0x50)?, row(r, 0x60)?, row(r, 0x70)?],
		});
	}
	Ok(out)
}

/// Reverb request/state shared by every reverb source (setter 0x2a13c0): SPU2 mode (libsd
/// SD_REV_MODE_*), depth ("amount", the authored integer) and the two effect options (delay,
/// feedback). Mode 0 = off. The IOP side applies it (per-frame consumer at overlay 0x2a... via boot
/// RPC 0x50 / ef68; ambience.md reverb section).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Reverb {
	pub mode: u8,
	pub amount: i32,
	pub opt: [u8; 2],
}

/// Reverb box (type 3, updater 0x319848): pvar bytes mode, option 1, option 2, inside latch, then
/// the i32 amount.
#[derive(Debug, Clone)]
pub struct ReverbZone {
	pub pos: [f32; 3],
	pub inverse: [[f32; 3]; 3],
	pub settings: Reverb,
	pub inside: bool,
}

impl ReverbZone {
	/// One update (FUN_00319848) with the hero position; returns the setter request, if any. Inside
	/// the unit cube the amount ramps with local X, `trunc(amount * (x + 1) / 2)` capped at the
	/// amount (FUN_00221c80/90 are int->float and truncating float->int). On the first exit the
	/// positive-X side keeps the full authored settings, the other side requests mode, amount and
	/// options all 0.
	pub fn update(&mut self, hero: [f32; 3]) -> Option<Reverb> {
		let d = [0, 1, 2].map(|i| hero[i] - self.pos[i]);
		let l = vec_mat(d, &self.inverse);
		if l.iter().all(|c| c.abs() <= 1.0) {
			let amount = ((self.settings.amount as f32 * (l[0] + 1.0) * 0.5) as i32).min(self.settings.amount);
			self.inside = true;
			return Some(Reverb { amount, ..self.settings });
		}
		if !self.inside {
			return None;
		}
		self.inside = false;
		Some(if l[0] > 0.0 { self.settings } else { Reverb::default() })
	}
}

/// A music request from a zone exit: (track, cue) for `music::Player::request`.
pub type MusicRequest = (i16, u8);

/// Music zone (type 6, updater 0x319a58): pvar words track for the negative-X exit, track for the
/// positive-X exit, inside latch, then two u16 cues (negative, positive) = the stinger slot played
/// on the way. Entering does nothing; leaving requests the side's music.
#[derive(Debug, Clone)]
pub struct MusicZone {
	pub pos: [f32; 3],
	pub inverse: [[f32; 3]; 3],
	pub track: [i32; 2],
	pub cue: [u16; 2],
	pub inside: bool,
}

impl MusicZone {
	/// One update (FUN_00319a58) with the hero position. A zone whose cue word is -1 (both halves)
	/// is dead; otherwise a missing half is copied from the other when it fires.
	pub fn update(&mut self, hero: [f32; 3]) -> Option<MusicRequest> {
		if self.cue == [0xffff, 0xffff] {
			return None;
		}
		let d = [0, 1, 2].map(|i| hero[i] - self.pos[i]);
		let l = vec_mat(d, &self.inverse);
		if l.iter().all(|c| c.abs() <= 1.0) {
			self.inside = true;
			return None;
		}
		if !self.inside {
			return None;
		}
		if self.cue[1] == 0xffff {
			self.cue[1] = self.cue[0];
		}
		if self.cue[0] == 0xffff {
			self.cue[0] = self.cue[1];
		}
		self.inside = false;
		let side = usize::from(l[0] > 0.0);
		Some((self.track[side] as i16, self.cue[side] as u8))
	}
}

/// Volume box (type 1, updater FUN_00319258): a looping or repeating level sound that follows the
/// listener to the nearest point of an oriented box and fades out toward the box's outer shell. pvars:
/// def, min interval, max interval, countdown, voice handle.
#[derive(Debug, Clone)]
pub struct VolumeBox {
	pub bank_sound: Option<usize>,
	pub fall: Falloff,
	pub looped: bool,
	pub pos: [f32; 3],
	pub matrix: [[f32; 3]; 3],
	pub inverse: [[f32; 3]; 3],
	/// Record +0xc: the scalar of the outer shell, E = (r, r, r) in box space.
	pub radius: f32,
}

fn len(v: [f32; 3]) -> f32 {
	(v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

impl VolumeBox {
	/// What the box plays this frame for a listener position: None = stop the voice (the listener is
	/// beyond the shell), else (volume in 1/1024 units, world position of the voice). Inside the unit
	/// box the sound is at full max volume at the listener; outside it sits at the nearest box point
	/// (the local offset clamped to [-1, 1]) and the def curve runs from that point's distance to
	/// the shell's.
	pub fn sample(&self, listener: [f32; 3]) -> Option<(i32, [f32; 3])> {
		let l = vec_mat([0, 1, 2].map(|i| listener[i] - self.pos[i]), &self.inverse);
		let e = len(vec_mat([self.radius; 3], &self.inverse));
		let ll = len(l);
		if e <= ll {
			return None;
		}
		let c = l.map(|x| x.clamp(-1.0, 1.0));
		let vol = if l.iter().any(|x| x.abs() > 1.0) {
			let f = &self.fall;
			crate::sound::attenuate(ll, len(c), e, f.max_vol, f.min_vol, f.flags & 1 != 0)
		} else {
			self.fall.max_vol
		};
		let w = vec_mat(c, &self.matrix);
		Some((vol, [w[0] + self.pos[0], w[1] + self.pos[1], w[2] + self.pos[2]]))
	}


}

/// Underwater loop (type 5, updater FUN_003199a8): while the listener is underwater a looping def plays
/// at the listener (no pan, no distance fade, volume 0x400, play flags 0x15).
#[derive(Debug, Clone)]
pub struct Underwater {
	pub bank_sound: Option<usize>,
	pub fall: Falloff,
}

/// The reverb boxes, music zones, volume boxes and underwater loops of a level.
#[derive(Debug, Clone, Default)]
pub struct Zones {
	pub reverb: Vec<ReverbZone>,
	pub music: Vec<MusicZone>,
	pub volume: Vec<VolumeBox>,
	pub underwater: Vec<Underwater>,
}

pub fn zones(level: &Level) -> R<Zones> {
	let mut z = Zones::default();
	let map = sound_map(level)?;
	for (ty, r, p) in records(level)? {
		let (pos, inverse) = (row(r, 0x40)?, [row(r, 0x50)?, row(r, 0x60)?, row(r, 0x70)?]);
		match ty {
			TYPE_REVERB => z.reverb.push(ReverbZone { pos, inverse, settings: Reverb { mode: p[0], opt: [p[1], p[2]], amount: u32_at(p, 4)? as i32 }, inside: false }),
			TYPE_MUSIC => z.music.push(MusicZone { pos, inverse, track: [u32_at(p, 0)? as i32, u32_at(p, 4)? as i32], cue: [u16_at(p, 12)?, u16_at(p, 14)?], inside: false }),
			TYPE_VOLUME | TYPE_UNDERWATER => {
				let def = u32_at(p, 0)? as usize;
				let d = map.defs.get(def).ok_or("zone sound def out of range")?;
				let (bank_sound, fall) = (d.bank_sound.map(|b| b as usize), d.falloff());
				if ty == TYPE_UNDERWATER {
					z.underwater.push(Underwater {  bank_sound, fall });
				} else {
					z.volume.push(VolumeBox {
						bank_sound,
						fall,
						looped: d.looped,
						pos,
						matrix: [row(r, 0x10)?, row(r, 0x20)?, row(r, 0x30)?],
						inverse,
						radius: f32_at(r, 0xc)?,
					});
				}
			}
			_ => {}
		}
	}
	Ok(z)
}

/// Gameplay header +0x88: the area table (u32 count, 12 pad, then 0x30-byte records) the game scans
/// when the hero spawns (FUN_00264a30 overlay 0x264a30): the nearest record within 8 units applies
/// its reverb (+0x20 amount, +0x24 mode, +0x25/+0x26 options, enabled when +0x27 != 0) and, when
/// its music id (+0x28) is >= 0, the music to start. Record +0x00 is the position.
#[derive(Debug, Clone)]
pub struct Area {
	pub pos: [f32; 3],
	pub reverb: Option<Reverb>,
	pub music: i32,
}

const AREAS: usize = 0x88;

pub fn areas(level: &Level) -> R<Vec<Area>> {
	let g = &level.gameplay[..];
	let start = u32_at(g, AREAS)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	let mut out = Vec::new();
	for r in span(g, start + 16, count * 0x30)?.chunks_exact(0x30) {
		let reverb = (r[0x27] != 0).then(|| Reverb { mode: r[0x24], opt: [r[0x25], r[0x26]], amount: i32::from_le_bytes([r[0x20], r[0x21], r[0x22], r[0x23]]) });
		out.push(Area { pos: row(r, 0)?, reverb, music: u32_at(r, 0x28)? as i32 });
	}
	Ok(out)
}

/// The area record the spawn lookup picks for a hero at `pos` (nearest within 8 units).
pub fn area_at(areas: &[Area], pos: [f32; 3]) -> Option<&Area> {
	areas.iter().map(|a| (dist(a.pos, pos), a)).filter(|(d, _)| *d <= 8.0).min_by(|a, b| a.0.total_cmp(&b.0)).map(|(_, a)| a)
}

/// Row vector times 3x3 (the game's FUN_001ff280 with the rotation part).
fn vec_mat(v: [f32; 3], m: &[[f32; 3]; 3]) -> [f32; 3] {
	[0, 1, 2].map(|j| v[0] * m[0][j] + v[1] * m[1][j] + v[2] * m[2][j])
}

impl Emitter {
	/// The game's start/stop test for the listener position (FUN_002ed2f8 / FUN_002ed480). For
	/// spheres this is the start condition; the sound is stopped only beyond `trigger_radius + 1`
	/// (`out_of_range`).
	pub fn in_range(&self, listener: [f32; 3]) -> bool {
		match self.shape {
			Shape::Sphere => dist(self.pos, listener) < self.trigger_radius,
			Shape::Cuboid => {
				let d = [0, 1, 2].map(|i| listener[i] - self.pos[i]);
				let local = vec_mat(d, &self.inverse);
				let r = self.trigger_radius;
				let ext = vec_mat([r, r, r], &self.inverse);
				(0..3).all(|i| local[i].abs() <= ext[i].abs() + 1.0)
			}
		}
	}

	/// Sphere emitters stop their looping sound once the listener is this far.
	pub fn out_of_range(&self, listener: [f32; 3]) -> bool {
		dist(self.pos, listener) > self.trigger_radius + 1.0
	}

}

fn dist(a: [f32; 3], b: [f32; 3]) -> f32 {
	((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Distance falloff of the playing sound (FUN_0028c258, volume sent to the IOP is this times the
/// instance volume 0x400/1024): `max volume` inside min_range, `min volume` beyond max_range,
/// linear (or quadratic, def flag bit 0) in (max_range - d)/(max_range - min_range) between.
/// The game stops a sound whose result is below 0x20/1024.
pub fn gain(e: &Emitter, listener: [f32; 3]) -> f32 {
	let d = dist(e.pos, listener);
	if d <= e.min_range {
		return e.volume;
	}
	if d >= e.max_range {
		return e.min_volume;
	}
	let mut t = (e.max_range - d) / (e.max_range - e.min_range);
	if e.quadratic {
		t *= t;
	}
	e.min_volume + (e.volume - e.min_volume) * t
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{gameplay, iso, toc};

	/// ARI-51 (decomp/notes/aridia_presentation.md): Aridia header streams, music zones, reverb boxes, no spawn areas.
	#[test]
	fn aridia_music() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[2]).unwrap();
		let ms = crate::music::level_music(&mut disc, &l).unwrap();
		let slots: std::collections::BTreeSet<usize> = ms.iter().map(|m| m.slot).collect();
		assert_eq!(slots, (0..9).collect());
		assert!(ms.iter().all(|m| m.rate == 44100));
		assert!(ms.iter().all(|m| m.loop_start.is_some() == matches!(m.slot, 1 | 3 | 5)));
		let secs = [3.84127, 113.27619, 7.11238, 120.89016, 6.40127, 128.00127, 6.00127, 6.00127, 6.00127];
		for m in &ms {
			let d = m.pcm.len() as f32 / 44100.0;
			assert!((d - secs[m.slot]).abs() < 1e-3, "slot {} {d}", m.slot);
		}
		let z = zones(&l).unwrap();
		assert_eq!(z.music.len(), 3);
		let tc: Vec<([i32; 2], [u16; 2])> = z.music.iter().map(|m| (m.track, m.cue)).collect();
		assert_eq!(tc, vec![([0, 4], [8, 7]), ([0, 2], [8, 6]), ([2, 0], [6, 8])]);
		for m in &z.music {
			for (t, c) in m.track.iter().zip(m.cue) {
				assert!(slots.contains(&(*t as usize)) && slots.contains(&(*t as usize + 1)) && slots.contains(&(c as usize)));
			}
		}
		assert_eq!(z.reverb.len(), 2);
		assert!(z.reverb.iter().all(|r| r.settings.mode == 3 && r.settings.amount == 2800));
		assert!(areas(&l).unwrap().is_empty());
	}

}
