//! In-engine cutscenes ("scenes"): per-level scene chunks, camera track, actor tracks, subtitles,
//! speech and the FMV movie table. Format and evidence: decomp/notes/cutscenes.md.
//!
//! A scene is a list of WAD-compressed chunks on the disc (listed in the level header). Each chunk
//! covers 80 ticks (PAL, 50 ticks/s) and holds a camera record per tick, per-actor position tracks
//! and ratchet-style animation sequences at half rate (one keyframe per 2 ticks), and the subtitle
//! records that overlap it. Speech is one VAG stream per language.

use crate::anim::{self, Frame};
use crate::bytes::{f32_at, i16_at, span, u16_at, u32_at, R};
use crate::iso::Disc;
use crate::level::Level;
use crate::{sound, toc, wad};

/// Scene ticks per second on PAL (one tick per video frame).
pub const TICKS_PER_SECOND: f32 = 50.0;
/// Ticks covered by one chunk on PAL (NTSC uses 96).
pub const CHUNK_TICKS: usize = 80;
/// Language slots of the speech streams and of the subtitle texts.
pub const LANG_ENGLISH: usize = 0;
pub const LANG_FRENCH: usize = 1;
pub const LANG_GERMAN: usize = 2;
pub const LANG_SPANISH: usize = 3;
pub const LANG_ITALIAN: usize = 4;

/// Level header +0x148: 15 music LBAs, then 15 scenes of 148 words each.
const HEADER_SIZE: usize = 0x2434;
const SCENE_TABLE: usize = 0x148 + 15 * 4;
const SCENE_WORDS: usize = 148;
/// Within a scene: 6 speech LBAs, 68 NTSC chunk LBAs (+3 pad), 68 PAL chunk LBAs (+3 pad).
const PAL_LIST: usize = 77;
const LIST_LEN: usize = 68;

#[derive(Debug, Clone)]
pub struct CameraKey {
	pub pos: [f32; 3],
	/// Euler angles (radians) applied X, then Y, then Z: [pitch-ish, roll (always ~0), yaw].
	pub rot: [f32; 3],
	/// Record +0x1c: the retail horizontal projection scalar 0x16cef0 (decomp/notes/camera.md "REN-7b port
	/// spec"); the vertical half-angle tangent is this times the PAL ratio 0.756.
	pub tan_half_fov: f32,
	/// Camera cut: actors snap instead of interpolating on this tick.
	pub cut: bool,
}

impl CameraKey {
	/// Look direction (unit, game axes, Z up): Rz * Ry * Rx * (0, 0, -1). [I] checked against the
	/// scene actors: the camera looks straight at Ratchet on scenes 1 and 4.
	pub fn forward(&self) -> [f32; 3] {
		self.rotate([0.0, 0.0, -1.0])
	}

	pub fn up(&self) -> [f32; 3] {
		self.rotate([0.0, 1.0, 0.0])
	}

	fn rotate(&self, v: [f32; 3]) -> [f32; 3] {
		let [rx, ry, rz] = self.rot;
		let (sx, cx) = rx.sin_cos();
		let v = [v[0], cx * v[1] - sx * v[2], sx * v[1] + cx * v[2]];
		let (sy, cy) = ry.sin_cos();
		let v = [cy * v[0] + sy * v[2], v[1], -sy * v[0] + cy * v[2]];
		let (sz, cz) = rz.sin_cos();
		[cz * v[0] - sz * v[1], sz * v[0] + cz * v[1], v[2]]
	}

	/// Linear blend of two keys (position, angles, zoom); the cut flag of `b` wins.
	pub fn lerp(&self, b: &CameraKey, t: f32) -> CameraKey {
		let mix = |x: f32, y: f32| x + (y - x) * t;
		CameraKey {
			pos: [0, 1, 2].map(|i| mix(self.pos[i], b.pos[i])),
			rot: [0, 1, 2].map(|i| mix(self.rot[i], b.rot[i])),
			tan_half_fov: mix(self.tan_half_fov, b.tan_half_fov),
			cut: b.cut,
		}
	}
}

#[derive(Debug, Clone)]
pub struct Subtitle {
	/// Start and end tick (inclusive range) from the start of the scene.
	pub start: u32,
	pub end: u32,
	/// Text per language slot: English, French, German, Spanish, Italian, slot 5 (unknown, short
	/// placeholder). Accented characters are mapped to Unicode.
	pub text: [String; 6],
}

#[derive(Debug, Clone)]
pub struct ActorKey {
	/// World position of the moby (game units).
	pub pos: [f32; 3],
	/// Ratchet-style animation frame (joint quaternions etc.), parsed by `anim::parse_sequence`.
	pub frame: Frame,
	/// The chunk's bounding sphere `(x, y, z, r)` of this actor (first qword of its sequence header, raw units).
	pub sphere: [f32; 4],
}

/// One scripted moby (record index in the chunk header; the same index is the same actor in every chunk).
#[derive(Debug, Clone)]
pub struct Actor {
	pub index: usize,
	/// Moby class id (0 = Ratchet; 530 = the ship; 1365/0x555 at the crash site...).
	pub class_id: u32,
	/// One key per 2 ticks: key `k` is tick `2k`. None where no chunk carries this actor.
	pub keys: Vec<Option<ActorKey>>,
	/// Joint count of the frames (rotation entries per frame).
	pub joints: usize,
}

impl Actor {
	/// Interpolated position at a (fractional) tick; holds the nearest key at the ends.
	pub fn pos_at(&self, tick: f32) -> Option<[f32; 3]> {
		let (a, b, t) = self.pair(tick)?;
		Some([0, 1, 2].map(|i| a.pos[i] + (b.pos[i] - a.pos[i]) * t))
	}

	/// The two keys around `tick` and the blend factor, as the game does (`frame>>1`, `(frame&1)*0.5`).
	/// Feed the frames to `anim::Animated::pose_frames(&a.frame, &b.frame, t)`.
	pub fn pair(&self, tick: f32) -> Option<(&ActorKey, &ActorKey, f32)> {
		let tick = tick.max(0.0);
		let k = (tick / 2.0) as usize;
		let t = (tick / 2.0).fract();
		let at = |i: usize| self.keys.get(i.min(self.keys.len().saturating_sub(1))).and_then(|k| k.as_ref());
		let a = at(k).or_else(|| self.keys.iter().rev().flatten().next())?;
		let b = at(k + 1).unwrap_or(a);
		Some((a, b, t))
	}
}

#[derive(Debug, Clone, Copy)]
pub struct SpeechRef {
	pub lba: u32,
}

#[derive(Debug, Clone)]
pub struct Scene {
	pub id: u32,
	/// Total length in ticks and seconds (50 ticks/s).
	pub ticks: u32,
	pub duration: f32,
	/// One camera key per tick (`ticks` entries; the chunk overlap tick is dropped).
	pub camera: Vec<CameraKey>,
	pub subtitles: Vec<Subtitle>,
	pub actors: Vec<Actor>,
	/// VAG stream LBA per language slot (slot 1 is unused in the PAL level header, so French is slot 2 there;
	/// this array is indexed by LANG_*).
	pub speech: [Option<SpeechRef>; 5],
}

/// Decoded speech: mono PCM.
pub struct Speech {
	pub sample_rate: u32,
	pub pcm: Vec<i16>,
}

impl Scene {
	pub fn camera_at(&self, tick: f32) -> Option<CameraKey> {
		let last = self.camera.len().checked_sub(1)?;
		let t = tick.max(0.0).min(last as f32);
		let (i, f) = (t as usize, t.fract());
		Some(self.camera[i].lerp(&self.camera[(i + 1).min(last)], f))
	}

	/// Subtitles visible at `tick`.
	pub fn subtitles_at(&self, tick: u32) -> impl Iterator<Item = &Subtitle> {
		self.subtitles.iter().filter(move |s| s.start <= tick && tick <= s.end)
	}

	/// Read and decode the speech stream of a language (LANG_*). None if the scene has no speech in it.
	pub fn speech(&self, disc: &mut Disc, lang: usize) -> R<Option<Speech>> {
		let Some(r) = self.speech.get(lang).copied().flatten() else { return Ok(None) };
		let head = disc.read_bytes(r.lba as u64, 0x30).map_err(|e| e.to_string())?;
		if &head[0..4] != b"VAGp" {
			return Err("speech stream is not a VAG".into());
		}
		// VAG header is big-endian: +0x0c data size, +0x10 sample rate.
		let size = u32::from_be_bytes([head[12], head[13], head[14], head[15]]) as usize;
		let rate = u32::from_be_bytes([head[16], head[17], head[18], head[19]]);
		let data = disc.read_bytes(r.lba as u64, 0x30 + size).map_err(|e| e.to_string())?;
		let (pcm, _) = sound::decode_adpcm(&data[0x30..]).or_else(|_| {
			// Streams that run to the end of the size without an end flag: pad one end frame.
			let mut d = data[0x30..].to_vec();
			d.extend_from_slice(&[0, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
			sound::decode_adpcm(&d)
		})?;
		Ok(Some(Speech { sample_rate: rate, pcm }))
	}
}

/// Game font bytes above 0x7f that the subtitle strings use (French: e acute 0x8c, e circumflex
/// 0x8e, e grave 0x8d, a grave 0x85, u grave 0xa5, C cedilla 0xaa, c cedilla 0xab, A grave 0x81, em dash 0xad). [I]
fn decode_text(b: &[u8]) -> String {
	b.iter()
		.map(|&c| match c {
			0x81 => '\u{c0}',
			0x85 => '\u{e0}',
			0x8c => '\u{e9}',
			0x8d => '\u{e8}',
			0x8e => '\u{ea}',
			0xa5 => '\u{f9}',
			0xaa => '\u{c7}',
			0xab => '\u{e7}',
			0xad => '\u{2014}',
			c if c < 0x80 => c as char,
			c => char::from_u32(0xe000 + c as u32).unwrap(),
		})
		.collect()
}

struct Chunk {
	data: Vec<u8>,
	total_ticks: u32,
	actor_count: usize,
}

fn read_chunk(disc: &mut Disc, lba: u32) -> R<Vec<u8>> {
	let first = disc.read_sectors(lba as u64, 1).map_err(|e| e.to_string())?;
	if !wad::is_wad(&first) {
		return Err(format!("scene chunk at sector {lba} is not a WAD"));
	}
	let size = wad::compressed_size(&first);
	let raw = disc.read_bytes(lba as u64, size).map_err(|e| e.to_string())?;
	wad::decompress(&raw).map_err(|e| format!("scene chunk WAD: {e}"))
}

fn parse_chunk_header(data: Vec<u8>) -> R<Chunk> {
	let total_ticks = u16_at(&data, 0)? as u32;
	let actor_count = i16_at(&data, 0x0c)?.max(0) as usize;
	Ok(Chunk { data, total_ticks, actor_count })
}

/// Per-tick camera records of a chunk: 0x20 bytes each, 81 of them (the last repeats the next chunk's first).
fn chunk_cameras(c: &Chunk, chunk_ticks: usize) -> R<Vec<CameraKey>> {
	let table = u32_at(&c.data, 0x10)? as usize;
	let first_actor = if c.actor_count > 0 { u32_at(&c.data, 0x14)? as usize } else { c.data.len() };
	let n = (first_actor.saturating_sub(table) / 0x20).min(chunk_ticks + 1);
	(0..n)
		.map(|i| {
			let r = span(&c.data, table + i * 0x20, 0x20)?;
			Ok(CameraKey {
				pos: [f32_at(r, 0)?, f32_at(r, 4)?, f32_at(r, 8)?],
				rot: [f32_at(r, 0x10)?, f32_at(r, 0x14)?, f32_at(r, 0x18)?],
				tan_half_fov: f32_at(r, 0x1c)?,
				cut: r[0x0c] != 0,
			})
		})
		.collect()
}

/// Subtitle records at header +4 (an offset when >= 0x400): 16 bytes each, (start, end, 6 text offsets), s16 -1 ends.
fn chunk_subtitles(c: &Chunk, out: &mut Vec<Subtitle>) -> R<()> {
	let off = u32_at(&c.data, 4)? as usize;
	if off < 0x400 {
		return Ok(());
	}
	let mut at = off;
	loop {
		let start = i16_at(&c.data, at)?;
		if start < 0 {
			return Ok(());
		}
		let end = i16_at(&c.data, at + 2)?;
		let mut text: [String; 6] = Default::default();
		for (l, t) in text.iter_mut().enumerate() {
			let o = off + u16_at(&c.data, at + 4 + l * 2)? as usize;
			let s = c.data.get(o..).ok_or("subtitle text outside the chunk")?;
			*t = decode_text(&s[..s.iter().position(|&b| b == 0).unwrap_or(s.len())]);
		}
		if !out.iter().any(|s| s.start == start as u32 && s.end == end as u32) {
			out.push(Subtitle {
				start: start as u32,
				end: end as u32,
				text,
			});
		}
		at += 16;
	}
}

/// One actor record of a chunk: (class id, 41 positions, 41 frames).
fn chunk_actor(c: &Chunk, k: usize) -> R<(u32, Vec<[f32; 3]>, Vec<Frame>, [f32; 4])> {
	let rec = u32_at(&c.data, 0x14 + k * 4)? as usize;
	let class_id = u32_at(&c.data, rec)?;
	let track = u32_at(&c.data, rec + 0x0c)? as usize;
	// Record +0x10 is a sequence header (bounding sphere, frame count at +0x10, offsets from +0x1c).
	let seq = span(&c.data, rec + 0x10, c.data.len() - rec - 0x10)?;
	let count = seq[0x10] as usize;
	let first = u32_at(seq, 0x1c)? as usize;
	let joints = u16_at(seq, first + 8)? as usize / 8;
	// The scene engine reads no sequence triggers; the count byte +0x12 is 0xff in every scene record, and the last chunk
	// of Aridia scene 2 ends before 255 trigger words would (ARI-53), so they are not parsed.
	let sequence = anim::parse_sequence_frames(seq, seq, joints)?;
	let positions = (0..count)
		.map(|i| {
			let p = span(&c.data, track + i * 16, 16)?;
			Ok([f32_at(p, 0)?, f32_at(p, 4)?, f32_at(p, 8)?])
		})
		.collect::<R<Vec<_>>>()?;
	Ok((class_id, positions, sequence.frames, sequence.sphere))
}

fn scene_words(disc: &mut Disc, level_id: usize, id: u32) -> R<Vec<u32>> {
	if id as usize >= 15 {
		return Err(format!("scene id {id} out of range"));
	}
	let levels = toc::read_levels(disc).map_err(|e| e.to_string())?;
	let info = levels.get(level_id).ok_or("no such level")?;
	let h = disc.read_bytes(info.header_sector, HEADER_SIZE).map_err(|e| e.to_string())?;
	(0..SCENE_WORDS).map(|i| u32_at(&h, SCENE_TABLE + (id as usize * SCENE_WORDS + i) * 4)).collect()
}

/// Joins a scene's decompressed chunks (`chunk_ticks` ticks each, plus one overlap key).
fn assemble(chunks: Vec<Vec<u8>>, chunk_ticks: usize) -> R<(u32, Vec<CameraKey>, Vec<Subtitle>, Vec<Actor>)> {
	let n_chunks = chunks.len();
	let mut camera = Vec::new();
	let mut subtitles = Vec::new();
	let mut actors: Vec<Actor> = Vec::new();
	let mut ticks = 0;
	for (ci, data) in chunks.into_iter().enumerate() {
		let chunk = parse_chunk_header(data)?;
		ticks = chunk.total_ticks;
		let cams = chunk_cameras(&chunk, chunk_ticks)?;
		let keep = if ci + 1 < n_chunks { chunk_ticks } else { cams.len() };
		camera.extend(cams.into_iter().take(keep));
		chunk_subtitles(&chunk, &mut subtitles)?;
		for k in 0..chunk.actor_count {
			let (class_id, pos, frames, sphere) = chunk_actor(&chunk, k).map_err(|e| format!("chunk {ci} actor {k}: {e}"))?;
			while actors.len() <= k {
				actors.push(Actor { index: actors.len(), class_id, keys: Vec::new(), joints: 0 });
			}
			let a = &mut actors[k];
			if a.keys.is_empty() && a.joints == 0 {
				a.class_id = class_id;
			}
			a.joints = frames.first().map_or(a.joints, |f| f.rotation.len());
			// Key k of this chunk is global key ci*chunk_ticks/2 + k; the chunk's last key repeats the next chunk's first.
			a.keys.resize(ci * (chunk_ticks / 2), None);
			let keep = if ci + 1 < n_chunks { chunk_ticks / 2 } else { pos.len() };
			for (p, f) in pos.into_iter().zip(frames).take(keep) {
				a.keys.push(Some(ActorKey { pos: p, frame: f, sphere }));
			}
		}
	}
	camera.truncate(ticks as usize);
	Ok((ticks, camera, subtitles, actors))
}

/// Title screen ticks per chunk (FUN_001eb698 loads the next chunk when the in-chunk tick passes 0x5f).
pub const TITLE_CHUNK_TICKS: usize = 96;

/// The title screen's looping scene: chunk table at core index[0x80], u32 (offset, size) pairs
/// relative to table + 0x800, ended by size 0 (decomp/notes/title_scene_data.md).
pub fn title_scene(level: &Level) -> R<Scene> {
	let table = u32_at(&level.index, 0x80)? as usize;
	let mut chunks = Vec::new();
	for i in 0.. {
		let (off, size) = (u32_at(&level.core, table + i * 8)? as usize, u32_at(&level.core, table + i * 8 + 4)? as usize);
		if size == 0 {
			break;
		}
		let raw = span(&level.core, table + 0x800 + off, size)?;
		chunks.push(wad::decompress(raw).map_err(|e| format!("title chunk {i}: {e}"))?);
	}
	let n = chunks.len();
	let (ticks, camera, subtitles, actors) = assemble(chunks, TITLE_CHUNK_TICKS)?;
	Ok(Scene { id: u32::MAX, ticks, duration: ticks as f32 / TICKS_PER_SECOND,  camera, subtitles, actors, speech: [None; 5] })
}

/// Load scene `id` of any level (ids as passed to the engine's start-scene function, 0..14).
pub fn load_scene(disc: &mut Disc, level_id: usize, id: u32) -> R<Scene> {
	let words = scene_words(disc, level_id, id)?;
	let lbas: Vec<u32> = words[PAL_LIST..PAL_LIST + LIST_LEN].iter().copied().take_while(|&w| w != 0).collect();
	// The last non-zero entry is the end marker, not a chunk.
	if lbas.len() < 2 {
		return Err(format!("scene {id} has no chunks"));
	}
	let n_chunks = lbas.len() - 1;
	let chunks = lbas[..n_chunks].iter().map(|&lba| read_chunk(disc, lba)).collect::<R<Vec<_>>>()?;
	let (ticks, camera, mut subtitles, actors) = assemble(chunks, CHUNK_TICKS)?;
	let mut speech = [None; 5];
	// Header slots: 0 English, 1 unused, 2 French, 3 German, 4 Spanish, 5 Italian.
	for (lang, slot) in [(LANG_ENGLISH, 0), (LANG_FRENCH, 2), (LANG_GERMAN, 3), (LANG_SPANISH, 4), (LANG_ITALIAN, 5)] {
		if words[slot] != 0 {
			speech[lang] = Some(SpeechRef { lba: words[slot] });
		}
	}
	subtitles.sort_by_key(|s| s.start);
	Ok(Scene { id, ticks, duration: ticks as f32 / TICKS_PER_SECOND,  camera, subtitles, actors, speech })
}

/// Ship stream `idx` (decomp/notes/level_transition.md "Entry and takeoff"): {sector, sectors} at ToC +0x1338 +
/// 8*idx (PAL table 0x138eb8; FUN_00259198), a directory of (offset, size) pairs ended by size 0 with each chunk
/// at file +0x800 + offset (at most 0x46), 80 ticks per chunk like the level scenes (FUN_002592a8 -> FUN_00258df8).
/// Positions and cameras are relative to the ship pad. Index ship+1 is the takeoff, ship+5 the landing.
pub fn ship_stream(disc: &mut Disc, idx: usize) -> R<Scene> {
	let t = toc::read_toc(disc).map_err(|e| e.to_string())?;
	let (sector, sectors) = (u32_at(&t, 0x1338 + idx * 8)? as u64, u32_at(&t, 0x133c + idx * 8)? as u64);
	if sectors == 0 {
		return Err(format!("ship stream {idx} is empty"));
	}
	let file = disc.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
	let mut chunks = Vec::new();
	for i in 0..0x46 {
		let (off, size) = (u32_at(&file, i * 8)? as usize, u32_at(&file, i * 8 + 4)? as usize);
		if size == 0 {
			break;
		}
		let raw = span(&file, 0x800 + off, size)?;
		chunks.push(wad::decompress(raw).map_err(|e| format!("ship stream {idx} chunk {i}: {e}"))?);
	}
	let n = chunks.len();
	let (ticks, camera, subtitles, actors) = assemble(chunks, CHUNK_TICKS)?;
	Ok(Scene { id: u32::MAX - 1, ticks, duration: ticks as f32 / TICKS_PER_SECOND,  camera, subtitles, actors, speech: [None; 5] })
}

/// Arrival warp ticks per chunk (`0x5f < cc98`, FUN_002a2ce0), the same as the title screen.
pub const WARP_CHUNK_TICKS: usize = 96;

/// The warp bundle (ToC entry 16 = table 0x138eb8 index 16, 509 sectors on PAL; FUN_0027a438) decompressed whole
/// (FUN_00261480): header words, class table, textures and the five stream directories (level_transition.md SCN-4.1).
pub fn warp_bundle(disc: &mut Disc) -> R<Vec<u8>> {
	let t = toc::read_toc(disc).map_err(|e| e.to_string())?;
	let (sector, sectors) = (u32_at(&t, 0x1338 + 16 * 8)? as u64, u32_at(&t, 0x133c + 16 * 8)? as u64);
	if sectors == 0 {
		return Err("warp bundle is empty".into());
	}
	let file = disc.read_sectors(sector, sectors).map_err(|e| e.to_string())?;
	wad::decompress(&file).map_err(|e| format!("warp bundle: {e}"))
}

/// Warp stream of type `ty` (0..4) from the decompressed bundle: header word `0x14 + ty` is the directory offset
/// from the base `w[1]`, a directory of (offset, size) pairs ended by size 0 (at most 0x46), each chunk at directory
/// + 0x800 + offset, WAD-compressed like the title chunks. 96 ticks per chunk.
pub fn warp_stream_from(bundle: &[u8], ty: usize) -> R<Scene> {
	if ty > 4 {
		return Err(format!("warp stream type {ty} out of range"));
	}
	let base = u32_at(bundle, 4)? as usize;
	let dir = base + u32_at(bundle, (0x14 + ty) * 4)? as usize;
	let mut chunks = Vec::new();
	for i in 0..0x46 {
		let (off, size) = (u32_at(bundle, dir + i * 8)? as usize, u32_at(bundle, dir + i * 8 + 4)? as usize);
		if size == 0 {
			break;
		}
		let raw = span(bundle, dir + 0x800 + off, size)?;
		chunks.push(wad::decompress(raw).map_err(|e| format!("warp stream {ty} chunk {i}: {e}"))?);
	}
	let n = chunks.len();
	let (ticks, camera, subtitles, actors) = assemble(chunks, WARP_CHUNK_TICKS)?;
	Ok(Scene { id: u32::MAX - 2, ticks, duration: ticks as f32 / TICKS_PER_SECOND,  camera, subtitles, actors, speech: [None; 5] })
}

/// Everything the warp bundle installs besides the streams (level_transition.md SCN-4.1b, loader `FUN_002a5198`
/// OV:103758-103809): the three ship classes with their own textures, the star-fan sky, the HUD (fx) textures, the
/// particle textures and defs, and the chrome texture.
pub struct WarpAssets {
	/// (class id, decoded class, core block kept for the skeleton) in table order; entries with no core are skipped.
	pub classes: Vec<(u32, crate::mobies::MobyClass, Vec<u8>)>,
	/// The bundle's moby texture table (the classes' remaps index it).
	pub moby_textures: Vec<Option<crate::level::Texture>>,
	pub sky: crate::sky::Sky,
	/// fx slot order: slot 0 the ribbon texture, 2 and 3 the emblem.
	pub fx_textures: Vec<crate::level::Texture>,
}

/// Decode the warp bundle's assets (the header words and tables are listed in the SCN-4.1b spec).
pub fn warp_assets(bundle: &[u8]) -> R<WarpAssets> {
	let w = |i: usize| u32_at(bundle, 4 * i).map(|v| v as usize);
	let base = w(1)?;
	let gs = bundle.get(w(0)?..).ok_or("warp gs image out of range")?;
	let texture = |pixels: &[u8], (tw, th): (usize, usize), palette: &[u8]| crate::level::Texture::new(tw, th, pixels, palette);
	// Moby textures: (pixel offset, w, h, palette) from B + w[0xc].
	let mut moby_textures = Vec::new();
	for i in 0..w(6)? {
		let e = span(bundle, w(7)? + 16 * i, 16)?;
		let (tw, th) = (u16_at(e, 4)? as usize, u16_at(e, 6)? as usize);
		if tw == 0 && th == 0 {
			moby_textures.push(None);
			continue;
		}
		let pal = u16_at(e, 10)? as usize * 256;
		let px = span(bundle, base + w(0xc)? + u32_at(e, 0)? as usize, tw * th)?;
		moby_textures.push(Some(texture(px, (tw, th), span(gs, pal, 1024)?)?));
	}
	let sky = crate::sky::sky_block(span(bundle, base + w(0x12)?, w(0x13)? - w(0x12)?)?)?;
	// Classes: 32-byte entries (core offset from B, id, _, _, 16-byte remap); a core runs to the next start.
	let table = span(bundle, w(5)?, 32 * w(4)?)?;
	let starts: Vec<usize> = table.chunks_exact(32).map(|e| u32_at(e, 0).map(|o| o as usize)).collect::<R<_>>()?;
	let mut classes = Vec::new();
	for e in table.chunks_exact(32) {
		let (off, id) = (u32_at(e, 0)? as usize, u32_at(e, 4)?);
		if off == 0 {
			continue;
		}
		let end = starts.iter().copied().filter(|&s| s > off).min().map_or(bundle.len(), |s| base + s);
		let block = span(bundle, base + off, end.saturating_sub(base + off))?;
		classes.push((id, crate::mobies::moby_class(block, &e[0x10..0x20], id)?, block.to_vec()));
	}
	// fx textures: (palette offset, pixel offset, w, h) from B + w[0xe].
	let fx_base = base + w(0xe)?;
	let mut fx_textures = Vec::new();
	for i in 0..w(0xa)? {
		let e = span(bundle, w(0xb)? + 16 * i, 16)?;
		let (pal, pix, tw, th) = (u32_at(e, 0)? as usize, u32_at(e, 4)? as usize, u32_at(e, 8)? as usize, u32_at(e, 12)? as usize);
		let colours = crate::hud::palette_rgba(span(bundle, fx_base + pal, 1024)?);
		let px = span(bundle, fx_base + pix, tw * th)?;
		fx_textures.push(crate::level::Texture { w: tw, h: th, rgba: px.iter().map(|&p| colours[p as usize]).collect() });
	}
	// Particle textures: (palette offset, 0, pixel offset, side) from B + w[0xd].
	let pt_base = base + w(0xd)?;
	let mut particle_textures = Vec::new();
	for i in 0..w(8)? {
		let e = span(bundle, w(9)? + 16 * i, 16)?;
		let (pal, pix, side) = (u32_at(e, 0)? as usize, u32_at(e, 8)? as usize, u32_at(e, 12)? as usize);
		particle_textures.push(span(bundle, pt_base + pix, side * side).ok().and_then(|px| texture(px, (side, side), span(bundle, pt_base + pal, 1024).ok()?).ok()));
	}
	// Particle defs: count, _, defs offset, size, then per-type offsets (0 = the first def).
	let blob = w(0xf)?;
	let (count, defs) = (u32_at(bundle, blob)? as usize, u32_at(bundle, blob + 8)? as usize);
	let mut particle_defs = Vec::new();
	for k in 0..count {
		let off = u32_at(bundle, blob + 16 + 4 * k)? as usize;
		particle_defs.push(*bundle.get(blob + if off == 0 { defs } else { off }).ok_or("particle def out of range")?);
	}
	let chrome = texture(span(gs, w(0x10)?, 128 * 128)?, (128, 128), span(gs, w(0x11)?, 1024)?)?;
	Ok(WarpAssets { classes, moby_textures, sky, fx_textures,   })
}

/// Ship class ids of core 0 / core 1 for ship 0 (level_transition.md NOV-34c.1).
pub const SHIP_BUNDLE_CLASSES: [u32; 2] = [531, 535];

/// The ship bundle of ship type `ship`: ToC {lba, sectors} at 0x12c8 + 8 * (ship + 1), read raw (no WAD), every
/// gameplay level load (level_transition.md NOV-34c.1).
pub fn ship_bundle(disc: &mut Disc, ship: usize) -> R<Vec<u8>> {
	let t = toc::read_toc(disc).map_err(|e| e.to_string())?;
	let at = 0x12c8 + 8 * (ship + 1);
	let (sector, sectors) = (u32_at(&t, at)? as u64, u32_at(&t, at + 4)? as u64);
	if sectors == 0 {
		return Err(format!("ship bundle {ship} is empty"));
	}
	disc.read_sectors(sector, sectors).map_err(|e| e.to_string())
}

/// What the ship bundle installs: two base-mip textures (moby texture indices `first` and `first + 1`) and its two
/// classes.
pub struct ShipAssets {
	/// 256 x 256 (section 0) and 128 x 128 (section 1).
	pub textures: [crate::level::Texture; 2],
	/// (class id, decoded class, core block kept for the skeleton): core 0 then core 1.
	pub classes: Vec<(u32, crate::mobies::MobyClass, Vec<u8>)>,
}


/// `ship_assets` of ship type `ship`: core 0 is class 531 + type, core 1 class 535 + type (FUN_00253588 OV:44473 /
/// 44490 read the ids from dat 0x1604c8 / 0x1604d8 indexed by the type; level_transition.md NOV-34c). [C]
pub fn ship_assets_of(bundle: &[u8], first: u32, ship: usize) -> R<ShipAssets> {
	if first > 0xfe {
		return Err(format!("ship bundle texture index {first} does not fit a remap byte"));
	}
	let h = |i: usize| u32_at(bundle, 4 * i).map(|v| v as usize);
	let (c0, t0, c1, t1) = (h(0)?, h(1)?, h(2)?, h(3)?);
	let tex = |base: usize, side: usize| {
		crate::level::Texture::new(side, side, span(bundle, base + 0x430, side * side)?, span(bundle, base + 0x30, 0x400)?)
	};
	let textures = [tex(t0, 256)?, tex(t1, 128)?];
	let mut classes = Vec::new();
	for (k, (s, e)) in [(c0, c1), (c1, t0)].into_iter().enumerate() {
		let block = span(bundle, s, e.checked_sub(s).ok_or("ship bundle core out of order")?)?;
		let mut remap = [0xffu8; 16];
		remap[0] = (first + k as u32) as u8;
		let id = SHIP_BUNDLE_CLASSES[k] + ship as u32;
		classes.push((id, crate::mobies::moby_class(block, &remap, id)?, block.to_vec()));
	}
	Ok(ShipAssets { textures, classes })
}

/// Decode a GS image record as the warp bundle stores its per-pair textures (level_transition.md SCN-4.2): header
/// +8 width, +0xc height, +0x10 format, +0x14 CLUT-size flag, then the CLUT and pixels from +0x20. Format 0x13 is
/// 8-bit indexed (CLUT 0x400 bytes of RGBA32, or 0x200 of 16-bit when the flag is set; index bits 3/4 swapped),
/// 0x14 is 4-bit indexed (CLUT 0x40 or 0x20 bytes, low nibble first), 0 is direct RGBA32 and 2 is direct 16-bit.
/// Pixel rows are taken as linear raster order.
pub fn gs_image(desc: &[u8]) -> R<crate::level::Texture> {
	let (w, h) = (u32_at(desc, 8)? as usize, u32_at(desc, 0xc)? as usize);
	let (fmt, flag) = (u32_at(desc, 0x10)?, u32_at(desc, 0x14)? != 0);
	if !(1..=4096).contains(&w) || !(1..=4096).contains(&h) {
		return Err("gs image dimensions out of range".into());
	}
	let c32 = |c: &[u8]| [c[0], c[1], c[2], (c[3] as u16 * 2).min(255) as u8];
	let c16 = |c: &[u8]| {
		let v = u16::from_le_bytes([c[0], c[1]]);
		[((v & 31) << 3) as u8, (((v >> 5) & 31) << 3) as u8, (((v >> 10) & 31) << 3) as u8, if v & 0x8000 != 0 { 255 } else { 0 }]
	};
	let rgba = match fmt {
		0x13 => {
			let (clut, wide) = if flag { (0x200, false) } else { (0x400, true) };
			let table = span(desc, 0x20, clut)?;
			let colours: Vec<[u8; 4]> = (0..256)
				.map(|i| {
					let j = (i & !0x18) | ((i & 8) << 1) | ((i & 0x10) >> 1);
					if wide { c32(&table[j * 4..j * 4 + 4]) } else { c16(&table[j * 2..j * 2 + 2]) }
				})
				.collect();
			span(desc, 0x20 + clut, w * h)?.iter().map(|&p| colours[p as usize]).collect()
		}
		0x14 => {
			let (clut, wide) = if flag { (0x20, false) } else { (0x40, true) };
			let table = span(desc, 0x20, clut)?;
			let colours: Vec<[u8; 4]> = (0..16).map(|i| if wide { c32(&table[i * 4..i * 4 + 4]) } else { c16(&table[i * 2..i * 2 + 2]) }).collect();
			let px = span(desc, 0x20 + clut, (w * h).div_ceil(2))?;
			(0..w * h).map(|i| colours[((px[i / 2] >> (4 * (i & 1))) & 15) as usize]).collect()
		}
		0 => span(desc, 0x20, w * h * 4)?.chunks_exact(4).map(c32).collect(),
		2 => span(desc, 0x20, w * h * 2)?.chunks_exact(2).map(c16).collect(),
		_ => return Err(format!("gs image format {fmt:#x} is not uploaded")),
	};
	Ok(crate::level::Texture { w, h, rgba })
}

/// The big quad's and the banner's textures for an arrival at `to` coming from `from` (SCN-4.2 Inputs): the table at the
/// bundle's `w[0x13]` block holds the big-quad descriptor offset at `+4 + 4 * to` and the banner descriptor offset at
/// `+0x50 + 4 * (max(from - 1, 0) * 0x13 + to)`, both relative to the block.
pub fn warp_pair_textures(bundle: &[u8], from: usize, to: usize) -> R<(crate::level::Texture, Option<crate::level::Texture>)> {
	let r = u32_at(bundle, 4)? as usize + u32_at(bundle, 0x13 * 4)? as usize;
	let big = r + u32_at(bundle, r + 4 + 4 * to)? as usize;
	// A (from, to) pair past the table (the table holds 114 banners; the slots after it are zero or image headers) has no banner.
	let i = from.saturating_sub(1) * 0x13 + to;
	let banner = if i >= 114 { None } else { u32_at(bundle, r + 0x50 + 4 * i).ok().and_then(|o| gs_image(bundle.get(r + o as usize..)?).ok()) };
	Ok((gs_image(bundle.get(big..).ok_or("big quad texture out of range")?)?, banner))
}


/// Veldin scene `id` (1..4 are the ones the level plays: 4 opening, 1, 2, 3).
pub fn veldin_scene(disc: &mut Disc, level: &Level, id: u32) -> R<Scene> {
	load_scene(disc, level.id, id)
}

/// An FMV movie (a PSS file on the disc).
#[derive(Debug, Clone)]
pub struct Movie {
	pub lba: u32,
	pub bytes: u64,
	pub fps: f32,
}

/// TOC mpeg group: 88 (lba, bytes) slots at 0x17f8; the PAL movie table (RAM 0x139420) starts at slot 21,
/// so scene id `0x4000 | n` is slot 21 + n.
const MPEG_GROUP: usize = 0x17f8;
const PAL_MOVIE_BASE: usize = 21;

/// Movie for a scene id with the 0x4000 bit (e.g. 0x4001) or a bare movie number.
pub fn movie(disc: &mut Disc, scene_id: u32) -> R<Movie> {
	let id = scene_id & 0x3fff;
	let slot = PAL_MOVIE_BASE + id as usize;
	let toc = toc::read_toc(disc).map_err(|e| e.to_string())?;
	let lba = u32_at(&toc, MPEG_GROUP + slot * 8)?;
	let bytes = u32_at(&toc, MPEG_GROUP + slot * 8 + 4)? as u64;
	if lba == 0 || bytes == 0 {
		return Err(format!("movie {id} is empty"));
	}
	let head = disc.read_bytes(lba as u64, 0x10000).map_err(|e| e.to_string())?;
	let at = head.windows(4).position(|w| w == [0, 0, 1, 0xb3]).ok_or("no MPEG sequence header")?;
	let (w, h, rate) = (
		((head[at + 4] as u32) << 4) | (head[at + 5] as u32 >> 4),
		(((head[at + 5] & 15) as u32) << 8) | head[at + 6] as u32,
		head[at + 7] & 15,
	);
	let fps = match rate {
		1 => 23.976,
		2 => 24.0,
		3 => 25.0,
		4 => 29.97,
		5 => 30.0,
		_ => 25.0,
	};
	// Last pack header: 00 00 01 BA + 6 bytes of SCR (MPEG-2), 90 kHz base.
	let tail_len = (bytes as usize).min(0x40000);
	let tail_sector = lba as u64 + (bytes - tail_len as u64) / 0x800;
	let tail = disc.read_bytes(tail_sector, tail_len).map_err(|e| e.to_string())?;
	let p = tail.windows(4).rposition(|w| w == [0, 0, 1, 0xba]).ok_or("no pack header")?;
	let x = tail[p + 4..p + 10].iter().fold(0u64, |a, &b| a << 8 | b as u64);
	let base = (((x >> 43) & 7) << 30) | (((x >> 27) & 0x7fff) << 15) | ((x >> 3) & 0x7fff);
	Ok(Movie {   lba, bytes,   fps, })
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::path::PathBuf;

	/// A one-actor, one-key chunk whose sequence header carries `sphere` (class 0x213).
	fn sphere_chunk(sphere: [f32; 4]) -> Vec<u8> {
		let mut d = vec![0u8; 0x200];
		d[0..2].copy_from_slice(&2u16.to_le_bytes());
		d[0x0c..0x0e].copy_from_slice(&1i16.to_le_bytes());
		d[0x10..0x14].copy_from_slice(&0x40u32.to_le_bytes());
		d[0x14..0x18].copy_from_slice(&0x80u32.to_le_bytes());
		d[0x80..0x84].copy_from_slice(&0x213u32.to_le_bytes());
		d[0x8c..0x90].copy_from_slice(&0x100u32.to_le_bytes());
		for (i, v) in sphere.iter().enumerate() {
			d[0x90 + 4 * i..0x94 + 4 * i].copy_from_slice(&v.to_le_bytes());
		}
		d[0xa0] = 1; // one frame
		d[0xac..0xb0].copy_from_slice(&0x20u32.to_le_bytes()); // frame at seq + 0x20
		d[0xb8..0xba].copy_from_slice(&8u16.to_le_bytes()); // scale_off = 8: one joint
		d[0xb6..0xb8].copy_from_slice(&1u16.to_le_bytes()); // body 16 bytes
		d
	}

	/// NOV-14.5d test 7: each actor key carries the sphere of the chunk it came from.
	#[test]
	fn actor_key_sphere_follows_the_chunk() {
		let (_, _, _, actors) = assemble(vec![sphere_chunk([1.0, 2.0, 3.0, 4.0]), sphere_chunk([5.0, 6.0, 7.0, 8.0])], 2).unwrap();
		let keys = &actors[0].keys;
		assert_eq!(keys[0].as_ref().unwrap().sphere, [1.0, 2.0, 3.0, 4.0]);
		assert_eq!(keys[1].as_ref().unwrap().sphere, [5.0, 6.0, 7.0, 8.0]);
	}

	#[test]
	#[ignore]
	fn warp_ship_canopy_probe() {
		let (mut disc, _) = open();
		let bundle = warp_bundle(&mut disc).unwrap();
		let assets = warp_assets(&bundle).unwrap();
		let s = warp_stream_from(&bundle, 4).unwrap();
		let act = &s.actors[0];
		let k0 = act.keys.iter().flatten().next().unwrap();
		eprintln!("stream ty4 actor0 class {:#x} joints {} keys {} rot {} tr {:?} sc {:?}", act.class_id, act.joints, act.keys.len(), k0.frame.rotation.len(), k0.frame.translation, k0.frame.scale);
		for (id, _, block) in &assets.classes {
			let a = crate::anim::animation_of_class_block(block, *id).unwrap().unwrap();
			let n = a.skeleton.parents.len();
			eprintln!("class {id:#x} joints {n} parents {:?} seqs {} attach {:?}", a.skeleton.parents, a.sequences.len(), &a.attach_points[..a.attach_points.len().min(6)]);
			eprintln!("  bind tr {:?}", a.skeleton.bind_translation);
			for (q, sq) in a.sequences.iter().enumerate() {
				if let Some(f) = sq.frames.first() {
					eprintln!("  seq {q} frames {} f0 rot {:?}", sq.frames.len(), f.rotation.iter().map(|r| r.map(|x| (x * 1000.0).round() / 1000.0)).collect::<Vec<_>>());
				}
			}
			if n == k0.frame.rotation.len() {
				eprintln!("  stream f0 rot {:?}", k0.frame.rotation.iter().map(|r| r.map(|x| (x * 1000.0).round() / 1000.0)).collect::<Vec<_>>());
			}
		}
	}

	fn open() -> (Disc, Vec<toc::Level>) {
		let path = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = Disc::open(&path).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		(disc, levels)
	}

	fn gs_desc(w: u32, h: u32, fmt: u32, flag: u32, body: &[u8]) -> Vec<u8> {
		let mut d = vec![0u8; 0x20];
		d[8..12].copy_from_slice(&w.to_le_bytes());
		d[12..16].copy_from_slice(&h.to_le_bytes());
		d[16..20].copy_from_slice(&fmt.to_le_bytes());
		d[20..24].copy_from_slice(&flag.to_le_bytes());
		d.extend_from_slice(body);
		d
	}

	#[test]
	fn gs_image_decodes_the_four_formats() {
		// 0x13 with an RGBA32 CLUT: entry 0 and entry 0x10 (index bits 3/4 swap: index 8 reads CLUT slot 0x10).
		let mut body = vec![0u8; 0x400];
		body[0..4].copy_from_slice(&[1, 2, 3, 0x40]);
		body[0x40..0x44].copy_from_slice(&[9, 8, 7, 0x80]);
		body.extend_from_slice(&[0, 8]);
		let t = gs_image(&gs_desc(2, 1, 0x13, 0, &body)).unwrap();
		assert_eq!((t.w, t.h, t.rgba), (2, 1, vec![[1, 2, 3, 0x80], [9, 8, 7, 255]]));
		// 0x13 with a 16-bit CLUT: 0x7c1f = r 31, b 31, bit 15 clear; 0xfc1f with bit 15 set.
		let mut body = vec![0u8; 0x200];
		body[0..2].copy_from_slice(&0x7c1fu16.to_le_bytes());
		body[2..4].copy_from_slice(&0xfc1fu16.to_le_bytes());
		body.extend_from_slice(&[0, 1]);
		let t = gs_image(&gs_desc(2, 1, 0x13, 1, &body)).unwrap();
		assert_eq!(t.rgba, vec![[248, 0, 248, 0], [248, 0, 248, 255]]);
		// 0x14: two nibbles per byte, low first; CLUT 0x40 bytes of RGBA32.
		let mut body = vec![0u8; 0x40];
		body[4..8].copy_from_slice(&[5, 6, 7, 1]);
		body[8..12].copy_from_slice(&[10, 11, 12, 2]);
		body.push(0x21);
		let t = gs_image(&gs_desc(2, 1, 0x14, 0, &body)).unwrap();
		assert_eq!(t.rgba, vec![[5, 6, 7, 2], [10, 11, 12, 4]]);
		// 0x14 with the 0x20-byte 16-bit CLUT.
		let mut body = vec![0u8; 0x20];
		body[2..4].copy_from_slice(&0x8001u16.to_le_bytes());
		body.push(0x01);
		let t = gs_image(&gs_desc(2, 1, 0x14, 1, &body)).unwrap();
		assert_eq!(t.rgba, vec![[8, 0, 0, 255], [0, 0, 0, 0]]);
		// 0: RGBA32 direct, alpha doubled and clamped; 2: 16-bit direct.
		let t = gs_image(&gs_desc(1, 1, 0, 0, &[1, 2, 3, 0x7f])).unwrap();
		assert_eq!(t.rgba, vec![[1, 2, 3, 254]]);
		let t = gs_image(&gs_desc(1, 1, 2, 0, &0x83e0u16.to_le_bytes())).unwrap();
		assert_eq!(t.rgba, vec![[0, 248, 0, 255]]);
		assert!(gs_image(&gs_desc(1, 1, 7, 0, &[0; 8])).is_err());
	}

	#[test]
	fn warp_pair_textures_are_128_square_and_a_256_by_32_banner() {
		let (mut disc, _) = open();
		let b = warp_bundle(&mut disc).unwrap();
		for (from, to) in [(1, 0), (2, 5), (5, 18)] {
			let (big, banner) = warp_pair_textures(&b, from, to).unwrap();
			assert_eq!((big.w, big.h), (128, 128));
			let banner = banner.unwrap();
			assert_eq!((banner.w, banner.h), (256, 32));
		}
		// Rows 0..5 are populated; from 7 (index 114+) has no banner.
		assert!(warp_pair_textures(&b, 6, 18).unwrap().1.is_some());
		assert!(warp_pair_textures(&b, 7, 0).unwrap().1.is_none());
		assert!(warp_pair_textures(&b, 8, 3).unwrap().1.is_none());
	}

	#[test]
	fn novalis_ship_bundle_registration() {
		// NOV-34c.2: Novalis has 237 moby textures and its own class 531 (remap [90]); 535 comes from the bundle.
		let (mut disc, levels) = open();
		let l = Level::load(&mut disc, &levels[1]).unwrap();
		assert_eq!(l.table(crate::level::TextureGroup::Moby as usize, 16).unwrap().len(), 237);
		let remaps = crate::mobies::moby_remaps(&l).unwrap();
		assert_eq!(remaps[&531][0], 90);
		assert!(remaps.get(&535).is_none());
	}

	#[test]
	fn ship_streams() {
		// Ship 0: takeoff idx 1 (498 ticks), landing idx 5 (614 ticks); both carry Ratchet 0, Clank 10 and ship 531.
		let (mut disc, _) = open();
		for (idx, ticks) in [(1, 498), (5, 614)] {
			let s = ship_stream(&mut disc, idx).unwrap();
			assert_eq!(s.ticks, ticks, "stream {idx}");
			for c in [0, 10, 531] {
				assert!(s.actors.iter().any(|a| a.class_id == c), "stream {idx} class {c}");
			}
		}
	}

	#[test]
	fn novalis_scenes() {
		// Scene ids 0-5 (decomp/notes/novalis_flow.md): 0 the Plumber's offer, 5 the arrival.
		let (mut disc, _) = open();
		for id in 0..=5 {
			let s = load_scene(&mut disc, 1, id).unwrap();
			assert!(s.ticks > 0, "scene {id}");
		}
		assert!(load_scene(&mut disc, 1, 5).unwrap().subtitles[0].text[0].contains("Ooomph"));
		assert!(load_scene(&mut disc, 1, 6).is_err());
	}

	#[test]
	fn subtitles_and_speech() {
		let (mut disc, levels) = open();
		let level = Level::load(&mut disc, &levels[0]).unwrap();
		let s2 = veldin_scene(&mut disc, &level, 2).unwrap();
		assert!(s2.subtitles.iter().any(|s| s.text[LANG_ENGLISH].contains("quite handy with your wrench")));
		let fr = s2.subtitles.iter().find(|s| s.text[LANG_ENGLISH].contains("quite handy")).unwrap();
		assert!(fr.text[LANG_FRENCH].starts_with("Vous \u{ea}tes dou\u{e9}"));
		assert!(s2.subtitles.iter().all(|s| s.end > s.start && s.end <= s2.ticks + 50));
		let s3 = veldin_scene(&mut disc, &level, 3).unwrap();
		assert!(s3.subtitles.iter().any(|s| s.text[0] == "Deal!"));
		assert!(veldin_scene(&mut disc, &level, 1).unwrap().subtitles.is_empty());
		let sp = s3.speech(&mut disc, LANG_ENGLISH).unwrap().unwrap();
		assert_eq!(sp.sample_rate, 44056);
		let secs = sp.pcm.len() as f32 / sp.sample_rate as f32;
		assert!((secs - 45.2).abs() < 1.0, "speech {secs}s");
	}

}
