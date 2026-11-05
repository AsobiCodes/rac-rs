//! The gadget-buy cinematic at the vendor shop close (decomp/notes/vendor.md "NOV-13.5", "NOV-13.5b"): the
//! fanfare voice on a black screen, then the pack's camera and two actors (hero, gadget model) in the frame the
//! vendor sets up, one camera record a tick, three WAD-compressed chunks of 0x50 ticks (PAL).

use crate::anim::{parse_sequence, Sequence};
use crate::bytes::{f32_at, span, u16_at, u32_at, R};

/// The port targets the PAL release (`iRam0015ed80` = PAL flag: TOC +0x15a0 and 0x50-tick chunks).
pub const PAL: bool = true;
/// "Fanfare_Vendor" (FUN_002ae128 OV:110502: `0x1516ec = 0x2734`).
pub const FANFARE_VOICE: u16 = 10036;

/// The pack's TOC entry (lba, sectors) offset: PAL +0x15a0, NTSC +0x1530, 8 bytes per entry (FUN_00259050).
pub fn pack_toc_offset(entry: usize, pal: bool) -> usize {
	(if pal { 0x15a0 } else { 0x1530 }) + 8 * entry
}




/// One 0x20-byte camera record: position, cut flag (+0x0c), angles (+0x10..), projection scale (+0x1c).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CamKey {
	pub pos: [f32; 3],
	pub cut: bool,
	pub ang: [f32; 3],
	pub scale: f32,
}

/// One actor record: class id (+0x00), its animation sequence (+0x10) and the translation per frame (+0x0c).
#[derive(Clone, Debug)]
pub struct ActorTrack {
	pub seq: Sequence,
}

/// One decoded chunk (`wad::decompress` output).
#[derive(Clone, Debug)]
pub struct Chunk {
	pub actors: Vec<ActorTrack>,
}

fn i32_at(d: &[u8], o: usize) -> R<i32> {
	Ok(u32_at(d, o)? as i32)
}

impl Chunk {
	/// `joints` = the joint counts of the two actors' classes (class 0 and the gadget model).
	pub fn parse(raw: &[u8], joints: [usize; 2]) -> R<Chunk> {
		let total = u16_at(raw, 0)?;
		let count = u16_at(raw, 0x0c)? as i16;
		if count < 0 || count > 3 {
			return Err(format!("gadget scene: {count} actors"));
		}
		let cam_off = i32_at(raw, 0x10)? as usize;
		let offs: Vec<usize> = (0..count as usize).map(|i| i32_at(raw, 0x14 + 4 * i).map(|o| o as usize)).collect::<R<_>>()?;
		// The camera track runs up to the first actor record.
		let end = offs.iter().copied().min().unwrap_or(raw.len());
		let cam = (0..end.saturating_sub(cam_off) / 0x20)
			.map(|k| {
				let r = span(raw, cam_off + 0x20 * k, 0x20)?;
				Ok(CamKey { pos: [f32_at(r, 0)?, f32_at(r, 4)?, f32_at(r, 8)?], cut: r[0x0c] != 0, ang: [f32_at(r, 0x10)?, f32_at(r, 0x14)?, f32_at(r, 0x18)?], scale: f32_at(r, 0x1c)? })
			})
			.collect::<R<Vec<_>>>()?;
		let actors = offs
			.iter()
			.enumerate()
			.map(|(i, &a)| {
				let class = u32_at(raw, a)?;
				let block = raw.get(a + 0x10..).ok_or("gadget scene: actor record past the chunk")?;
				let seq = parse_sequence(block, block, joints[i.min(1)]).map_err(|e| format!("gadget scene actor {i}: {e}"))?;
				let t = i32_at(raw, a + 0x0c)? as usize;
				let trans = (0..seq.frames.len())
					.map(|k| {
						let v = span(raw, t + 16 * k, 16)?;
						Ok([f32_at(v, 0)?, f32_at(v, 4)?, f32_at(v, 8)?, f32_at(v, 12)?])
					})
					.collect::<R<Vec<_>>>()?;
				Ok(ActorTrack {  seq, })
			})
			.collect::<R<Vec<_>>>()?;
		Ok(Chunk {   actors })
	}
}

/// One actor this tick: frames `frame_a` -> `frame_b` of its sequence at `blend`, at `pos` facing `yaw`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ActorPose {
	pub class: u32,
	pub frame_a: usize,
	pub frame_b: usize,
	pub blend: f32,
	pub pos: [f32; 3],
	pub yaw: f32,
}

/// One rendered tick: the camera (world position, angles applied x, y, z), projection scale, the black fade (0..1)
/// and the two actors.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SceneFrame {
	pub cam_pos: [f32; 3],
	pub cam_ang: [f32; 3],
	pub scale: f32,
	pub fade: f32,
	pub actors: [ActorPose; 2],
}

/// The pack and the scene clock (`cc94` scene tick, `cc98` chunk tick, `cc9c` chunk, fade `0x15f3fc`).
#[derive(Clone, Debug)]
pub struct GadgetScene {
	pub chunk: Chunk,
}

/// Chunk `k` of a pack: (offset, size) pairs at +0, data at 0x800 + offset, WAD-compressed.
pub fn chunk_raw(pack: &[u8], k: usize) -> R<Vec<u8>> {
	let (off, size) = (u32_at(pack, 8 * k)? as usize, u32_at(pack, 8 * k + 4)? as usize);
	let data = span(pack, 0x800 + off, size)?;
	crate::wad::decompress(data).map_err(|e| format!("gadget scene chunk {k}: {e:?}"))
}

impl GadgetScene {
	/// `origin` (`0x1ca900`) and `yaw` (`0x1ca8f8`) from the shop close (NOV-13.5 Order A); chunk 0 loaded
	/// (`FUN_00259148(0)`), every clock at 0.
	pub fn new(pack: &[u8], pal: bool, origin: [f32; 3], yaw: f32, joints: [usize; 2]) -> R<GadgetScene> {
		let chunk = Chunk::parse(&chunk_raw(pack, 0)?, joints)?;
		Ok(GadgetScene {      chunk,    })
	}

	/// The current chunk's sequence of actor `i` (the frames `ActorPose` indexes).
	pub fn sequence(&self, i: usize) -> Option<&Sequence> {
		self.chunk.actors.get(i).map(|a| &a.seq)
	}

}

/// The cinematic run (`Game.gadget`): the fanfare on a black screen until the voice ends and the pack is read,
/// then the scene. `offer` (NOV-14.1) selects the world gold-offer end (FUN_002ae620) instead of FUN_002adf90(1).
#[derive(Clone, Debug)]
pub enum GadgetRun {
	/// `prop`: the vendor (shop path) whose exit placement ends the scene.
	Fanfare { entry: usize, prop: usize, origin: [f32; 3], yaw: f32, scene: Option<GadgetScene>, voice_done: bool, offer: Option<usize> },
	Scene { prop: usize, scene: GadgetScene, offer: Option<usize> },
}

