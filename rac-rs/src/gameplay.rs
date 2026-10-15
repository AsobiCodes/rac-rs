//! PAL gameplay data: the per-level instance tables. Pointer table layout: Wrench's R&C1 gameplay
//! table, checked on v1.00 (decomp/notes/collision_and_gameplay.md).

use crate::bytes::{f32_at, i32_at, span, u16_at, u32_at, R};
use crate::level::Level;
use crate::physics::V3;

/// FUN_00214468: PAL authored-tick scale.
pub const PAL_SCALE: f32 = f32::from_bits(0x3f555555);
pub const PAL_DT: f32 = f32::from_bits(0x3ca3d70b);
/// 0x15ed70, the per-tick-squared word D2 (not PAL_DT * PAL_DT, which is 0x39d1b719).
pub const PAL_DT2: f32 = f32::from_bits(0x39d1b718);


/// Retail scale(n) (`FUN_00220960`): ACC = 0.5 then madd with k, truncating; exact in f64 (EE FPU rounds toward zero).
pub fn pal_ticks(n: i32) -> i32 {
	debug_assert!(n >= 0);
	((n as f64) * (PAL_SCALE as f64) + 0.5) as i32
}




/// The EE's truncating f32 multiply (rounds toward zero, as `pal_scale_f`).
pub(crate) fn ee_mul(a: f32, b: f32) -> f32 {
	ee_trunc(a as f64 * b as f64)
}

/// The EE's truncating f32 add (rounds toward zero, as `ee_mul`).
pub(crate) fn ee_add(a: f32, b: f32) -> f32 {
	ee_trunc(a as f64 + b as f64)
}



/// An exact f64 result rounded toward zero to f32 (the EE FPU rounding mode).
pub(crate) fn ee_trunc(p: f64) -> f32 {
	let r = p as f32;
	if (r as f64).abs() > p.abs() { f32::from_bits(r.to_bits() - 1) } else { r }
}

/// Gameplay +0x44: (static count, spawnable count, pad) then 0x78-byte moby instance records.
const MOBY_INSTANCES: usize = 0x44;

#[derive(Clone)]
pub struct MobyInstance {
	pub class_id: u32,
	pub pos: V3,
	/// Euler angles in radians; [2] is yaw around Z.
	pub rot: V3,
	/// Record +0x10: the bolt drop for crates and enemies, class-specific otherwise.
	pub param: u32,
	/// The instance's class-specific variables (record +0x58 indexes the pvar table), empty if none.
	pub pvars: Vec<u8>,
	/// Record +0x08: spawn flags (bits 0/1 = mission phases, 0x4/0x8 = suppressed once destroyed) and
	/// +0x0c the UID those bits are kept under (decomp/notes/level_transition.md).
	pub flags: u32,
	pub uid: u32,
	/// Index in the level's placement table: the unique identity (UIDs repeat, e.g. instances 528 and 913
	/// share 376, and some are negative, e.g. 857 = -16).
	pub index: u32,
	/// Record +0x04: the mission slot (0..16) whose byte selects the phase (-1 or out of range: none).
	pub slot: i32,
	/// Record +0x14: the bolt drop once the slot is complete (+0x10 `param` before).
	pub reward_done: u32,
	/// Records +0x64/+0x68/+0x6c: the placed colour (moby +0x80, copied to the ambient +0x3c; Wrench `col`).
	pub col: [u8; 3],
	/// Record +0x70: the light word (moby +0x38): byte 0 = directional slot A, byte 1 = slot B, byte 2 = blend
	/// (decomp/notes/lighting_fog.md "Moby lighting").
	pub light: u32,
	/// Scheduler inputs (FUN_002650e0): update distance, group, loaded class index (game/sched.rs).
	pub place: Placement,
	/// Record +0x1c: the instance scale (the moby's draw / collision scale is class +0x24 times this).
	pub scale: f32,
	/// Moby bytes +0x36 / +0x37 as one u16 (low byte = bit mask, high byte = mask index): the occlusion pair of the record
	/// (REN-CULL-OCC-MOBY); 0x7f80 passes every mask.
	pub occ: u16,
}

/// The placement fields the actor scheduler reads: record +0x24 as the byte actor+0x30, signed byte +0x48
/// as actor+0x21, and the class's index in the level's class table (actor+0x22; decomp/notes/novalis_flow.md).
#[derive(Clone, Copy, Debug, Default)]
pub struct Placement {
	pub dist: u8,
	pub group: i8,
	pub class_rank: u32,
	/// Record +0x20 as a signed short: the moby draw distance in metres (actor +0x32).
	pub draw: i16,
}

impl MobyInstance {
	/// The slot's mission byte. The loader indexes the Veldin buffer 0x15fc48 by the signed slot unchecked; six
	/// Veldin placements (102, 118-122) use slot -1, which reads 0x15fc47: padding after the newline printf
	/// literal at 0x15fc40, always 0, so not complete [C] (decomp/notes/novalis_flow.md). Other out-of-range
	/// slots never occur and also read 0.
	fn slot_byte(&self, mission: &[u8; 16]) -> u8 {
		usize::try_from(self.slot).ok().and_then(|s| mission.get(s)).copied().unwrap_or(0)
	}

	/// Whether the loader spawns this instance (overlay.elf.c:46155-46195; the bitmap flags 0x4/0x8 of
	/// placements without phase bits are the port's `saved` set). With a phase bit set the actor's OWN
	/// slot selects bit 0 (not complete) or bit 1 (complete: the byte is exactly 0xff).
	pub fn in_phase(&self, mission: &[u8; 16]) -> bool {
		self.flags & 3 == 0 || self.flags & if self.slot_byte(mission) == 0xff { 2 } else { 1 } != 0
	}



	/// The UID's bit in a level bitmap (0x100 bytes); negative or out-of-range UIDs have none.
	pub fn uid_bit(&self) -> Option<usize> {
		Some(self.uid as i32).filter(|&u| (0..0x800).contains(&u)).map(|u| u as usize)
	}
}


/// Gameplay +0x54: (offset, size) per pvar index, offsets into the pvar data at gameplay +0x58.
const PVAR_TABLE: usize = 0x54;
const PVAR_DATA: usize = 0x58;
/// Gameplay +0x10: US English help messages (+0x14 UK English, a stub on PAL; then French, German,
/// Spanish, Italian, Japanese, Korean).
const HELP_US_ENGLISH: usize = 0x10;
/// Gameplay +0x60: (count, pad x3) then 0x80-byte cuboids.
const CUBOIDS: usize = 0x60;

fn pvars(g: &[u8], index: i32) -> R<Vec<u8>> {
	if index < 0 {
		return Ok(Vec::new());
	}
	let table = u32_at(g, PVAR_TABLE)? as usize;
	let data = u32_at(g, PVAR_DATA)? as usize;
	let off = u32_at(g, table + index as usize * 8)? as usize;
	let size = u32_at(g, table + index as usize * 8 + 4)? as usize;
	Ok(span(g, data + off, size)?.to_vec())
}

/// NOV-3.4: the moby group section (header word 0x12, byte 0x48; FUN_002554c8 OV:46340-46405): N groups, then N i32
/// offsets at +0x10 (negative = no list) into the list block at +0x10 + 4N; each list is the raw u16 source words
/// (low 15 bits = blob moby index) up to and including the one with bit 15.
pub fn group_lists(g: &[u8]) -> R<Vec<Option<Vec<u16>>>> {
	let s = u32_at(g, 0x48)? as usize;
	if s == 0 {
		return Ok(Vec::new());
	}
	let n = u32_at(g, s)? as usize;
	let block = s + 0x10 + 4 * n;
	(0..n)
		.map(|i| {
			let off = i32_at(g, s + 0x10 + 4 * i)?;
			if off < 0 {
				return Ok(None);
			}
			let mut list = Vec::new();
			let mut at = block + off as usize;
			loop {
				let w = u16_at(g, at)?;
				list.push(w);
				at += 2;
				if w & 0x8000 != 0 {
					break;
				}
			}
			Ok(Some(list))
		})
		.collect()
}

/// Actor header word 1: the dynamic slot count D (256 on Novalis, NOV-9.10); the allocator scans D - 1 slots.
pub fn moby_slot_count(level: &Level) -> R<usize> {
	let g = &level.gameplay;
	let start = u32_at(g, MOBY_INSTANCES)? as usize;
	Ok(u32_at(g, start + 4)? as usize)
}

/// Loaded class index of every class: position in the class table (model-less entries are not loaded). The scheduler's
/// rank of a placed or spawned actor; also the effect mobys' (SCH-6b).
pub fn class_ranks(level: &Level) -> R<std::collections::HashMap<u32, u32>> {
	let mut ranks = std::collections::HashMap::new();
	for e in level.table(0x18, 32)?.into_iter().filter(|e| u32_at(e, 0).is_ok_and(|s| s != 0)) {
		let n = ranks.len() as u32;
		ranks.entry(u32_at(e, 4)?).or_insert(n);
	}
	Ok(ranks)
}

/// ARI-6: class-table row of every class, model-less rows included: the rank a creator-spawned actor carries (the
/// `sched::spawned` Veldin / Novalis columns are these rows), read from any level's own table.
pub fn class_rows(level: &Level) -> R<std::collections::HashMap<u32, u32>> {
	let mut rows = std::collections::HashMap::new();
	for (n, e) in level.table(0x18, 32)?.into_iter().enumerate() {
		rows.entry(u32_at(e, 4)?).or_insert(n as u32);
	}
	Ok(rows)
}

pub fn moby_instances(level: &Level) -> R<Vec<MobyInstance>> {
	let g = &level.gameplay;
	let start = u32_at(g, MOBY_INSTANCES)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	let ranks = class_ranks(level)?;
	// Occlusion is on when the level has both the grid (core index +0x0c) and the mapping block (REN-CULL-OCC).
	let occ_map = if !level.title && u32_at(&level.index, 0x0c).is_ok_and(|s| s != 0) { crate::occlusion::OccMap::new(g) } else { None };
	span(g, start + 16, count * 0x78)?
		.chunks_exact(0x78)
		.enumerate()
		.map(|(index, r)| {
			let v = |o: usize| -> R<V3> { Ok([f32_at(r, o)?, f32_at(r, o + 4)?, f32_at(r, o + 8)?]) };
			Ok(MobyInstance {
				class_id: u32_at(r, 0x18)?,
				pos: v(0x30)?,
				rot: v(0x3c)?,
				param: u32_at(r, 0x10)?,
				pvars: pvars(g, u32_at(r, 0x58)? as i32)?,
				flags: u32_at(r, 0x08)?,
				uid: u32_at(r, 0x0c)?,
				index: index as u32,
				slot: u32_at(r, 0x04)? as i32,
				reward_done: u32_at(r, 0x14)?,
				col: [u32_at(r, 0x64)? as u8, u32_at(r, 0x68)? as u8, u32_at(r, 0x6c)? as u8],
				light: u32_at(r, 0x70)?,
				place: Placement { dist: u32_at(r, 0x24)? as u8, group: r[0x48] as i8, class_rank: ranks.get(&u32_at(r, 0x18)?).copied().unwrap_or(u32::MAX), draw: u32_at(r, 0x20)? as i16 },
				scale: f32_at(r, 0x1c)?,
				// Record +0x5c 0 = mapped, else always drawn.
				occ: match &occ_map {
					Some(m) if u32_at(r, 0x5c)? == 0 => crate::occlusion::moby_occ(m, u32_at(r, 0x0c)?),
					_ => crate::occlusion::MOBY_UNMAPPED,
				},
			})
		})
		.collect()
}

/// Ratchet's start: on every level, moby instance 0 is the only class-0 (Ratchet) instance.
/// Inferred from data, not yet traced in the game's hero placement code.
pub fn spawn(level: &Level) -> R<(V3, f32)> {
	let m = moby_instances(level)?;
	let hero = m.first().filter(|m| m.class_id == 0).ok_or("moby instance 0 is not Ratchet")?;
	Ok((hero.pos, hero.rot[2]))
}

/// Gameplay +0x00 points at the level settings; +0x28 there is the kill plane height (Wrench's
/// death_height; Veldin 27.0). Falling below it with no ground near is death.
pub fn death_height(level: &Level) -> R<f32> {
	let settings = u32_at(&level.gameplay, 0)? as usize;
	f32_at(&level.gameplay, settings + 0x28)
}

/// The level header's ship transform (settings +0x2c position, +0x38 yaw): where the loader spawns the
/// landed ship with the pad callback FUN_002a1570, when x is positive (decomp/notes/novalis_flow.md).
pub fn ship(level: &Level) -> R<Option<(V3, f32)>> {
	let s = u32_at(&level.gameplay, 0)? as usize;
	let pos = [f32_at(&level.gameplay, s + 0x2c)?, f32_at(&level.gameplay, s + 0x30)?, f32_at(&level.gameplay, s + 0x34)?];
	Ok((pos[0] > 0.0).then_some((pos, f32_at(&level.gameplay, s + 0x38)?)))
}

/// The level header's ship flight settings (words +0x3c, +0x40, +0x44 of the settings block): the departure
/// path id and the two camera cuboid ids of the glide; -1 means none.
pub fn ship_flight(level: &Level) -> R<(i32, i32, i32)> {
	let s = u32_at(&level.gameplay, 0)? as usize;
	Ok((i32_at(&level.gameplay, s + 0x3c)?, i32_at(&level.gameplay, s + 0x40)?, i32_at(&level.gameplay, s + 0x44)?))
}

/// What the departure flight needs from a level: the path points (path id in range) and the two camera poses
/// (both cuboid ids in range).
pub type FlightData = (Option<Vec<V3>>, Option<(crate::game::transit::CamPose, crate::game::transit::CamPose)>);

/// Resolve the settings words against the level's polygons and cuboids.
pub fn flight_data(words: (i32, i32, i32), polygons: &[Polygon], cuboids: &[Cuboid]) -> FlightData {
	let (path, a, b) = words;
	let path = usize::try_from(path).ok().and_then(|i| polygons.get(i)).map(|p| p.points.clone());
	let pose = |i: i32| usize::try_from(i).ok().and_then(|i| cuboids.get(i)).map(|c| crate::game::transit::CamPose { pos: c.pos, y: c.rot[1], z: c.rot[2] });
	let cams = pose(a).zip(pose(b));
	(path, cams)
}

/// A trigger volume: a unit cube mapped into the world by a scaled rotation (rows are the box's
/// half-axes) at `pos`. The game tests a point with the stored inverse (decomp/notes/veldin_flow.md).
#[derive(Clone, Copy)]
pub struct Cuboid {
	pub pos: V3,
	pub inverse: [[f32; 3]; 3],
	/// Euler angles at +0x70 (x, y, z); the camera cuts read them (decomp/notes/camera.md).
	pub rot: V3,
}

impl Cuboid {
	pub fn contains(&self, p: V3) -> bool {
		let d = [p[0] - self.pos[0], p[1] - self.pos[1], p[2] - self.pos[2]];
		(0..3).all(|j| (d[0] * self.inverse[0][j] + d[1] * self.inverse[1][j] + d[2] * self.inverse[2][j]).abs() <= 1.0)
	}
}

pub fn cuboids(level: &Level) -> R<Vec<Cuboid>> {
	let g = &level.gameplay;
	let start = u32_at(g, CUBOIDS)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	span(g, start + 16, count * 0x80)?
		.chunks_exact(0x80)
		.map(|r| {
			let f = |o: usize| f32_at(r, o);
			let row = |o: usize| -> R<[f32; 3]> { Ok([f(o)?, f(o + 4)?, f(o + 8)?]) };
			Ok(Cuboid {  pos: row(0x30)?, inverse: [row(0x40)?, row(0x50)?, row(0x60)?], rot: row(0x70)? })
		})
		.collect()
}

/// KER-5: each cuboid record's forward basis rows (+0x00, +0x10, +0x20; `Cuboid` keeps only row 0 and the inverse),
/// same record walk as `cuboids` (decomp/notes/kerwan_enemies.md "KER-5 port spec").
pub fn cuboid_basis(level: &Level) -> R<Vec<[V3; 3]>> {
	let g = &level.gameplay;
	let start = u32_at(g, CUBOIDS)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	span(g, start + 16, count * 0x80)?
		.chunks_exact(0x80)
		.map(|r| {
			let f = |o: usize| f32_at(r, o);
			let row = |o: usize| -> R<[f32; 3]> { Ok([f(o)?, f(o + 4)?, f(o + 8)?]) };
			Ok([row(0x00)?, row(0x10)?, row(0x20)?])
		})
		.collect()
}

/// Gameplay +0x70: the path/polygon table (decomp/notes/veldin_enemies.md). Section header: (count,
/// data offset from the section, data size, pad), then `count` u32 offsets into the data.
const POLYGONS: usize = 0x70;

/// A closed XY polygon (or an open path: the same data serves both). Record: i32 point count,
/// 12 pad bytes, then 16-byte points (x, y, z, w = -1.0).
#[derive(Clone, Debug)]
pub struct Polygon {
	pub points: Vec<[f32; 3]>,
	/// Each point's fourth word (+0xc of the 16-byte record); the birdbot path code reads it as a 1.0 / -1.0 mark.
	pub marks: Vec<f32>,
}


pub fn polygons(level: &Level) -> R<Vec<Polygon>> {
	let g = &level.gameplay;
	let sec = u32_at(g, POLYGONS)? as usize;
	if sec == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, sec)? as usize;
	let data = sec + u32_at(g, sec + 4)? as usize;
	(0..count).map(|i| polygon_at(g, data + u32_at(g, sec + 16 + i * 4)? as usize)).collect()
}

/// One path/polygon record at byte `at`: u32 point count, 12 pad bytes, then 16-byte points.
fn polygon_at(g: &[u8], at: usize) -> R<Polygon> {
	let n = u32_at(g, at)? as usize;
	let pts = span(g, at + 16, n * 16)?;
	let points = pts
		.chunks_exact(16)
		.map(|r| Ok([f32_at(r, 0)?, f32_at(r, 4)?, f32_at(r, 8)?]))
		.collect::<R<Vec<_>>>()?;
	let marks = pts.chunks_exact(16).map(|r| f32_at(r, 12)).collect::<R<Vec<_>>>()?;
	Ok(Polygon { points, marks })
}

/// KER-60: gameplay +0x74, the grind-rail section (decomp/notes/kerwan_gadgets.md). Section header: (count, blob
/// offset from the section, blob size, pad), then `count` 0x20-byte rail records, then `count` i32 path offsets into
/// the blob (each a path record in the `polygons` format).
const GRIND_RAILS: usize = 0x74;

/// One grind rail (runtime record of the list 0x15f6ec): bounding sphere (+0x0 centre, +0xc radius), its path
/// (+0x10) and the closed-path flag (+0x14).
#[derive(Clone, Debug)]
pub struct GrindRail {
}

/// KER-60: the level's grind rails (empty when the header word +0x74 is 0).
pub fn grind_rails(level: &Level) -> R<Vec<GrindRail>> {
	grind_rails_in(&level.gameplay)
}

pub(crate) fn grind_rails_in(g: &[u8]) -> R<Vec<GrindRail>> {
	let sec = u32_at(g, GRIND_RAILS)? as usize;
	if sec == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, sec)? as usize;
	let data = sec + u32_at(g, sec + 4)? as usize;
	(0..count)
		.map(|i| {
			let rec = sec + 0x10 + i * 0x20;
			let off = i32_at(g, sec + 0x10 + count * 0x20 + i * 4)?;
			Ok(GrindRail {
			})
		})
		.collect()
}







/// One help message: its text, and the help slot that remembers it was shown (-1 for plain strings).
pub struct Help {
	pub text: String,
	pub slot: i16,
}

/// The US English help and caption strings by id. File: (count, size) then 16-byte entries
/// (text offset, id, slot, s16, 0); text is NUL-terminated ASCII with control bytes for highlights
/// (0x0c on, 0x08 off) and pad buttons (0x10..).
pub fn help_messages(level: &Level) -> R<std::collections::HashMap<u32, Help>> {
	let g = &level.gameplay;
	let start = u32_at(g, HELP_US_ENGLISH)? as usize;
	let mut out = std::collections::HashMap::new();
	if start == 0 {
		return Ok(out);
	}
	let count = u32_at(g, start)? as usize;
	let size = u32_at(g, start + 4)? as usize;
	let file = span(g, start, size)?;
	for e in span(file, 8, count * 16)?.chunks_exact(16) {
		let off = u32_at(e, 0)? as usize;
		let raw = file.get(off..).ok_or("help text outside the file")?;
		let raw = &raw[..raw.iter().position(|&b| b == 0).unwrap_or(raw.len())];
		let mut text = String::new();
		for &b in raw {
			match b {
				// Pad glyphs (0x10 Cross, 0x11 Circle, 0x12 Triangle, 0x13 Square, 0x14 R1..), the highlight
				// marks 0x0c/0x08 and the 0x01 line break stay as control chars; the game font draws the glyph
				// sprites inline (decomp/notes/novalis_flow.md "Prompt and dialogue box").
				0x08 | 0x0c | 0x10..=0x1f | 0x01 => text.push(b as char),
				b'\n' | b'\r' => text.push(' '),
				0x20..=0x7e => text.push(b as char),
				_ => {}
			}
		}
		out.insert(u32_at(e, 4)?, Help { text, slot: i16::from_le_bytes([e[8], e[9]]) });
	}
	Ok(out)
}

/// A directional light pair (gameplay +0x04, 0x40-byte records): every lit object gets both
/// lights of its pair (decomp/notes/lighting_fog.md section 2).
#[derive(Clone, Copy, Default)]
pub struct DirLight {
	pub colour: [[f32; 3]; 2],
	/// The direction the light travels in (to-light vector is the negation) [I sign].
	pub dir: [[f32; 3]; 2],
	/// Colour W of each light: the wrap factor of the baked-colour equation (vertex_colours.md).
	pub wrap: [f32; 2],
}

/// The light pair a packed selector picks (decomp/notes/vertex_colours.md): low nibble index when
/// the high byte is zero, else two nibbles blended by the high byte / 256, directions renormalized.
/// Bit positions of the two-light form [I].
pub fn select_light(lights: &[DirLight], selector: u32) -> DirLight {
	let get = |i: u32| lights.get(i as usize).copied().unwrap_or_default();
	let blend = (selector >> 8) & 0xff;
	if blend == 0 {
		return get(selector & 0xf);
	}
	let (a, b, t) = (get(selector & 0xf), get((selector >> 4) & 0xf), blend as f32 / 256.0);
	let mix = |x: [f32; 3], y: [f32; 3]| -> [f32; 3] { std::array::from_fn(|j| x[j] + (y[j] - x[j]) * t) };
	let unit = |v: [f32; 3]| -> [f32; 3] {
		let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
		v.map(|c| c / l)
	};
	DirLight {
		colour: [mix(a.colour[0], b.colour[0]), mix(a.colour[1], b.colour[1])],
		dir: [unit(mix(a.dir[0], b.dir[0])), unit(mix(a.dir[1], b.dir[1]))],
		wrap: [a.wrap[0] + (b.wrap[0] - a.wrap[0]) * t, a.wrap[1] + (b.wrap[1] - a.wrap[1]) * t],
	}
}

/// Baked vertex colour in PS2 bytes (0x80 = 1.0): `base + 128 * sum(colour * max(q, q * w))` with
/// `q = dot(n, -dir)`, RGB capped at `cap`, alpha kept (decomp/notes/vertex_colours.md "Lighting and port recipe").
pub fn baked_colour(light: &DirLight, base: [f32; 4], n: [f32; 3], cap: f32) -> [f32; 4] {
	let mut out = baked_raw(light, base, n);
	for c in &mut out[..3] {
		*c = c.clamp(0.0, cap);
	}
	out
}

/// `baked_colour` before the cap: the dynamic third light is added to this sum and the cap applies to the total
/// (LightTies / LightShrubs, decomp/notes/lighting_fog.md 5b).
pub fn baked_raw(light: &DirLight, base: [f32; 4], n: [f32; 3]) -> [f32; 4] {
	let mut out = base;
	for k in 0..2 {
		let d = light.dir[k];
		let q = -(n[0] * d[0] + n[1] * d[1] + n[2] * d[2]);
		let q = q.max(q * light.wrap[k]);
		for j in 0..3 {
			out[j] += 128.0 * light.colour[k][j] * q;
		}
	}
	out
}

/// Tie and shrub colour cap: prog1 `MINIi` with I = 0x478000f3.
pub const TIE_CAP: f32 = 243.0;
/// Terrain colour cap: LightTfrags clamps with vf22 = 0x478000ff.
pub const TERRAIN_CAP: f32 = 255.0;





/// RGBA5551 as PEXT5 expands it: each 5-bit channel shifted left 3, no bit replication; alpha 0x80.
pub fn pext5(c: u16) -> [f32; 4] {
	[(c & 0x1f) << 3, ((c >> 5) & 0x1f) << 3, ((c >> 10) & 0x1f) << 3, 0x80].map(|x| x as f32)
}

/// An environment zone (gameplay +0x80): a bounding sphere, then a world-to-unit-cube box whose
/// local X blends A -> B. Flag bit 0: moby ambient and lights; bit 1: camera fog (`Lighting::zone_fog`).
#[derive(Clone, Copy)]
pub struct Zone {
	pub sphere: [f32; 4],
	pub matrix: [[f32; 4]; 4],
	pub ambient: [[f32; 3]; 2],
	pub light: [usize; 2],
	pub flags: u32,
	/// Fog colour A and B (record +0x54/+0x58, byte 0 = red).
	pub fog_colour: [[u8; 3]; 2],
	/// Fog A and B (record +0x5c/+0x6c): near m, near intensity, far m, far intensity (0..1 fogging amount).
	pub fog: [[f32; 4]; 2],
}

impl Zone {
	/// FUN_002569f0's pre-test: HORIZONTAL, dx^2 + dy^2 < w (decomp/notes/lighting_fog.md "Zones").
	pub fn near(&self, p: V3) -> bool {
		let d = [p[0] - self.sphere[0], p[1] - self.sphere[1]];
		d[0] * d[0] + d[1] * d[1] < self.sphere[3]
	}

	/// The blend along the box (0 at A, 1 at B), None outside the pre-test or the box; FUN_002569f0. The blend
	/// runs along the box's local X [C].
	pub fn blend(&self, p: V3) -> Option<f32> {
		if !self.near(p) {
			return None;
		}
		let m = &self.matrix;
		let local: [f32; 3] = std::array::from_fn(|j| p[0] * m[0][j] + p[1] * m[1][j] + p[2] * m[2][j] + m[3][j]);
		local.iter().all(|c| c.abs() <= 1.0).then(|| (local[0] + 1.0) / 2.0)
	}
}

/// Level settings fog and clear colour plus the light and zone tables.
pub struct Lighting {
	pub fog_colour: [u8; 3],
	/// Fog near/far in world units (stored x1024) and the GS fog coefficient at each (255 = clear).
	pub fog: [f32; 4],
	pub lights: Vec<DirLight>,
	pub zones: Vec<Zone>,
	/// Static point lights (gameplay +0x7c): they brighten only the hero's ambient.
	pub points: Vec<PointLight>,
	/// Point light lookup grid (gameplay +0x78, after its 0x10 header): 64x64 cells of 16 units, each a u32
	/// offset into this block (0 = empty) of a list (count, light indices).
	pub point_grid: Vec<u8>,
}

/// A static level point light (gameplay +0x7c, 0x20-byte records; decomp/notes/lighting_fog.md "Point lights").
#[derive(Clone, Copy, Debug)]
pub struct PointLight {
	pub pos: V3,
	/// Record +0x0c: compared with the squared distance (Wrench's `radius` field).
	pub r2: f32,
	/// Record +0x10 colour bytes.
	pub col: [u8; 3],
}

/// Spawned mobies (FUN_0024e540): ambient (64, 64, 64), light word 0.
pub const SPAWN_AMBIENT: [u8; 3] = [64, 64, 64];

/// The hero's environment before any zone: its placed colour and light word.
pub fn placed_env(col: [u8; 3], light: u32) -> (usize, usize, f32, [f32; 3]) {
	let (a, b, t) = light_word(light);
	(a, b, t, col.map(|c| c as f32 / 128.0))
}

impl Lighting {
	/// The hero's light pair, ambient (GS scale, 0x80 = 1.0) and blend toward light B: HeroEnvLighting
	/// FUN_00256c34 (decomp/notes/lighting_fog.md "Zones"). The first zone whose horizontal pre-test passes
	/// decides: inside its box with flags&1 it writes the zone's pair and ambient into `env` (hero +0x38/+0x80);
	/// outside the box, or without the flag, nothing is written and no later zone is tried. `env` is sticky:
	/// nothing resets it outside zones, so before any zone it holds the placed colour and light word
	/// (`placed_env`). The first point light in range then adds to the ambient (FUN_00256b24).
	pub fn hero(&self, p: V3, env: &mut (usize, usize, f32, [f32; 3])) -> (usize, usize, f32, [f32; 3]) {
		if let Some(z) = self.zones.iter().find(|z| z.near(p)) {
			if let Some(t) = z.blend(p).filter(|_| z.flags & 1 != 0) {
				let a = std::array::from_fn(|i| z.ambient[0][i] + (z.ambient[1][i] - z.ambient[0][i]) * t);
				*env = (z.light[0], z.light[1], t, a);
			}
		}
		let (a, b, t, amb) = *env;
		(a, b, t, self.point_ambient(p, amb))
	}

	/// NOV-28 camera fog (FUN_002101b8, decomp/notes/water.md): the first zone whose horizontal pre-test passes
	/// decides; inside its box with flags&2 it writes the blended colour and fog [near, far, near int, far int]
	/// into `live`, otherwise nothing is written (sticky, like `hero`).
	pub fn zone_fog(&self, cam: V3, live: &mut ([u8; 3], [f32; 4])) {
		let Some(z) = self.zones.iter().find(|z| z.near(cam)) else { return };
		let Some(t) = z.blend(cam).filter(|_| z.flags & 2 != 0) else { return };
		let i = (t * 255.0) as i32;
		let [a, b] = z.fog_colour;
		let colour = std::array::from_fn(|k| ((b[k] as i32 * i + a[k] as i32 * (255 - i)) >> 8) as u8);
		let [fa, fb] = z.fog;
		let mix = |k: usize| fb[k] * t + fa[k] * (1.0 - t);
		*live = (colour, [mix(0), mix(2), 255.0 - mix(1) * 255.0, 255.0 - mix(3) * 255.0]);
	}

	/// FUN_00256b24 (decomp/notes/lighting_fog.md "Point lights"): the grid cell (y>>4)*64 + (x>>4) lists
	/// lights; the first with d^2 < r2 is the only one used. a = 255 - int(d^2 / r2 * 255), ambient += col * a
	/// >> 8 (bytes, saturated at 255).
	pub fn point_ambient(&self, p: V3, amb: [f32; 3]) -> [f32; 3] {
		let Some((l, d2)) = self.point_at(p) else { return amb };
		let a = 255 - (d2 / l.r2 * 255.0) as u32;
		let bytes = amb.map(|c| (c * 128.0).round().clamp(0.0, 255.0) as u32);
		std::array::from_fn(|i| (bytes[i] + (l.col[i] as u32 * a >> 8)).min(255) as f32 / 128.0)
	}

	/// The first light of p's grid cell with p inside its range, and the squared distance.
	fn point_at(&self, p: V3) -> Option<(PointLight, f32)> {
		let g = &self.point_grid;
		let cell = ((p[1] as i32 as u32 >> 4) as usize).checked_mul(64)?.checked_add(p[0] as i32 as u32 as usize >> 4)?;
		let off = u32_at(g, cell.checked_mul(4)?).ok()? as usize;
		if off == 0 {
			return None;
		}
		let n = u32_at(g, off).ok()? as usize;
		(0..n).find_map(|i| {
			let l = *self.points.get(u32_at(g, off + 4 + i * 4).ok()? as usize)?;
			let d = [p[0] - l.pos[0], p[1] - l.pos[1], p[2] - l.pos[2]];
			let d2 = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
			(d2 < l.r2).then_some((l, d2))
		})
	}
}

/// A light word (moby +0x38): (slot A, slot B, blend toward B). Slots past the level's lights (15 = none)
/// light nothing; the shader zeroes them.
pub fn light_word(light: u32) -> (usize, usize, f32) {
	((light & 0xff) as usize, ((light >> 8) & 0xff) as usize, ((light >> 16) & 0xff) as f32 / 256.0)
}

/// Pointer table entry -> (count, records after the 0x10-byte header).
fn table(g: &[u8], entry: usize, size: usize) -> R<(usize, usize)> {
	let start = u32_at(g, entry)? as usize;
	if start == 0 {
		return Ok((0, 0));
	}
	let n = u32_at(g, start)? as usize;
	span(g, start + 16, n * size)?;
	Ok((n, start + 16))
}

pub fn lighting(level: &Level) -> R<Lighting> {
	let g = &level.gameplay;
	let s = u32_at(g, 0)? as usize;
	let byte = |o: usize| -> R<u8> { Ok(u32_at(g, s + o)? as u8) };
	let clear = [byte(0)?, byte(4)?, byte(8)?];
	let fog_colour = [byte(0x0c)?, byte(0x10)?, byte(0x14)?];
	let fog = [f32_at(g, s + 0x18)? / 1024.0, f32_at(g, s + 0x1c)? / 1024.0, f32_at(g, s + 0x20)?, f32_at(g, s + 0x24)?];
	let vec3 = |o: usize| -> R<[f32; 3]> { Ok([f32_at(g, o)?, f32_at(g, o + 4)?, f32_at(g, o + 8)?]) };
	let (n, at) = table(g, 0x04, 0x40)?;
	let lights = (0..n.min(12))
		.map(|i| {
			let r = at + i * 0x40;
			Ok(DirLight { colour: [vec3(r)?, vec3(r + 0x20)?], dir: [vec3(r + 0x10)?, vec3(r + 0x30)?], wrap: [f32_at(g, r + 0x0c)?, f32_at(g, r + 0x2c)?] })
		})
		.collect::<R<Vec<_>>>()?;
	let (n, at) = table(g, 0x80, 0x10)?;
	let records = at + n * 0x10;
	span(g, records, n * 0x80)?;
	let rgb = |o: usize| -> R<[f32; 3]> {
		let c = u32_at(g, o)?;
		Ok([c as u8, (c >> 8) as u8, (c >> 16) as u8].map(|x| x as f32 / 128.0)) // GS colour: 0x80 = 1.0
	};
	let byte3 = |o: usize| -> R<[u8; 3]> {
		let c = u32_at(g, o)?;
		Ok([c as u8, (c >> 8) as u8, (c >> 16) as u8])
	};
	let vec4 = |o: usize| -> R<[f32; 4]> { Ok([f32_at(g, o)?, f32_at(g, o + 4)?, f32_at(g, o + 8)?, f32_at(g, o + 12)?]) };
	let light = |o: usize| -> R<usize> { Ok(match crate::bytes::i32_at(g, o)? { x if x < 0 => 11, x => x as usize }) };
	let zones = (0..n)
		.map(|i| {
			let r = records + i * 0x80;
			let sp = at + i * 0x10;
			Ok(Zone {
				sphere: [f32_at(g, sp)?, f32_at(g, sp + 4)?, f32_at(g, sp + 8)?, f32_at(g, sp + 12)?],
				matrix: std::array::from_fn(|row| std::array::from_fn(|c| f32_at(g, r + row * 16 + c * 4).unwrap_or(0.0))),
				ambient: [rgb(r + 0x40)?, rgb(r + 0x44)?],
				light: [light(r + 0x48)?, light(r + 0x4c)?],
				flags: u32_at(g, r + 0x50)?,
				fog_colour: [byte3(r + 0x54)?, byte3(r + 0x58)?],
				fog: [vec4(r + 0x5c)?, vec4(r + 0x6c)?],
			})
		})
		.collect::<R<Vec<_>>>()?;
	let (n, at) = table(g, 0x7c, 0x20)?;
	let points = (0..n)
		.map(|i| {
			let r = at + i * 0x20;
			let c = u32_at(g, r + 0x10)?;
			Ok(PointLight { pos: vec3(r)?, r2: f32_at(g, r + 0x0c)?, col: [c as u8, (c >> 8) as u8, (c >> 16) as u8] })
		})
		.collect::<R<Vec<_>>>()?;
	// Grid block: u32 size, 0x10 header, then `size` bytes: 0x4000 of cell offsets and the lists (the loader
	// copies size + 0x4000, past the end of the data).
	let point_grid = match u32_at(g, 0x78)? as usize {
		0 => Vec::new(),
		at => span(g, at + 16, u32_at(g, at)? as usize)?.to_vec(),
	};
	Ok(Lighting {  fog_colour, fog, lights, zones, points, point_grid })
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{iso, toc};

	/// SHR-PATH1 test (2): a path record keeps each vertex's w word bit-exact in `marks` (Kerwan rail 0 vertex 0
	/// w 0x426bc94c, one ulp off its f32 chord), so the grind sample walk reads the authored length.
	#[test]
	fn polygon_keeps_authored_w_bits() {
		let mut g = vec![0u8; 16 + 32];
		g[0..4].copy_from_slice(&2u32.to_le_bytes());
		g[16 + 12..16 + 16].copy_from_slice(&0x426bc94cu32.to_le_bytes());
		g[32 + 12..32 + 16].copy_from_slice(&(-1.0f32).to_bits().to_le_bytes());
		let p = polygon_at(&g, 0).unwrap();
		assert_eq!(p.points.len(), 2);
		assert_eq!((p.marks[0].to_bits(), p.marks[1]), (0x426bc94c, -1.0));
	}

	// NOV-28 tests 1-3: zone fog blend, truncation, stickiness and the flag gate.
	// GLOBAL-DT
	#[test]
	fn pal_half_tick_rounding() {
		assert_eq!(PAL_SCALE.to_bits(), 0x3f555555);
		for (n, want) in [(3, 2), (9, 7), (15, 12), (21, 17), (27, 22), (33, 27), (39, 32), (45, 37), (51, 42), (100, 83)] {
			assert_eq!(pal_ticks(n), want);
		}
		assert!((0..20000).all(|n| pal_ticks(n) == (5 * n + 2) / 6));
	}

	#[test]
	fn novalis_ship_flight_words() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let s = u32_at(&l.gameplay, 0).unwrap() as usize;
		let w = ship_flight(&l).unwrap();
		assert_eq!(w, (i32_at(&l.gameplay, s + 0x3c).unwrap(), i32_at(&l.gameplay, s + 0x40).unwrap(), i32_at(&l.gameplay, s + 0x44).unwrap()));
		eprintln!("ship flight words {w:?}");
		let d = flight_data(w, &polygons(&l).unwrap(), &cuboids(&l).unwrap());
		assert_eq!(d.0.is_some(), w.0 >= 0);
	}

	#[test]
	fn novalis_moby_slot_count_is_256() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		assert_eq!(moby_slot_count(&l).unwrap(), 256);
	}

	#[test]
	fn veldin_moby_slot_count_is_256() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[0]).unwrap();
		assert_eq!(moby_slot_count(&l).unwrap(), 256);
	}

	#[test]
	fn novalis_birdbots_and_peckbots_by_mission_slots() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let all = moby_instances(&l).unwrap();
		let count = |m: &[u8; 16]| {
			let n = |c: u32| all.iter().filter(|i| i.class_id == c && i.in_phase(m)).count();
			(n(459), n(577))
		};
		assert_eq!(count(&[0; 16]), (25, 14));
		let mut done = [0u8; 16];
		for s in [0, 1, 2, 6] {
			done[s] = 0xff;
		}
		assert_eq!(count(&done), (5, 58));
		// 528 and 913 share UID 376: identity is the placement index.
		assert_eq!(all[528].uid, all[913].uid);
		assert!(all.iter().enumerate().all(|(i, m)| m.index as usize == i));
	}

	#[test]
	fn novalis_point_lights() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		let lt = lighting(&l).unwrap();
		assert_eq!(lt.points.len(), 17);
		let p = lt.points[0];
		assert!((p.pos[0] - 222.418).abs() < 0.01 && (p.r2 - 6.5025).abs() < 0.001, "{p:?}");
		assert_eq!(p.col, [49, 46, 36]);
		// At the light, the full colour is added (light 0 must be the first match in its cell).
		let a = lt.point_ambient(p.pos, [0.0; 3]);
		assert!((a[0] - (49 * 255 >> 8) as f32 / 128.0).abs() < 1e-6);
		// The grid reaches every light from its own position.
		assert!(lt.points.iter().all(|l| lt.point_at(l.pos).is_some()));
		eprintln!("wraps {:?}", lt.lights.iter().map(|d| d.wrap).collect::<Vec<_>>());
	}

	/// ARI-3 (decomp/notes/aridia_flow.md): hero spawn, death height and header ship on Aridia.
	#[test]
	fn aridia_spawn() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[2]).unwrap();
		assert_eq!(spawn(&l).unwrap(), ([210.41f32, 170.35, 25.35], 2.3099957f32));
		assert_eq!(death_height(&l).unwrap(), 0.0);
		let (pos, yaw) = ship(&l).unwrap().unwrap();
		let want = [205.35262f32, 153.97421, 26.614683];
		assert!((0..3).all(|k| (pos[k] - want[k]).abs() < 1e-5) && (yaw - 2.0943952).abs() < 1e-5, "{pos:?} {yaw}");
		assert_eq!(ship_flight(&l).unwrap(), (23, 70, 71));
	}
}

#[cfg(test)]
mod dump {
	#[test]
	#[ignore]
	fn dump_instances() {
		use crate::{iso, level::Level, toc};
		let lv: usize = std::env::var("DUMP_LEVEL").ok().and_then(|v| v.parse().ok()).unwrap_or(1);
		let classes: Vec<u32> = std::env::var("DUMP_CLASSES").unwrap_or_default().split(',').filter_map(|v| v.parse().ok()).collect();
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let l = Level::load(&mut disc, &levels[lv]).unwrap();
		for (i, m) in super::moby_instances(&l).unwrap().iter().enumerate() {
			if !classes.is_empty() && !classes.contains(&m.class_id) {
				continue;
			}
			let words: Vec<String> = m.pvars.chunks_exact(4).map(|c| {
				let u = u32::from_le_bytes([c[0], c[1], c[2], c[3]]);
				let f = f32::from_bits(u);
				if u == 0 { "0".into() } else if u == 0xffff_ffff { "-1".into() } else if (u as i32).abs() < 100000 { format!("{}", u as i32) } else if f.abs() > 1e-4 && f.abs() < 1e6 { format!("{f:.3}") } else { format!("{u:#x}") }
			}).collect();
			println!("#{i} c{} pos {:.2?} rot {:.2?} param {} flags {:#x} uid {} pv[{}]: {}", m.class_id, m.pos, m.rot, m.param, m.flags, m.uid, m.pvars.len(), words.join(" "));
		}
	}
}
