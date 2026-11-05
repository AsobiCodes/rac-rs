//! R&C2 gameplay-block tables and the R&C2 header translations the shared decoders use (from lane rc2's `rc2/world/rc2.rs`).
//! Layout facts of the gameplay block: lights at +0x04, fog through the settings pointer at +0, tie instances at +0x34 (0x60-byte
//! records), shrub instances at +0x40 (`game::RC2.shrub_instances`).

use crate::bytes::{f32_at, span, u16_at, u32_at, R};
use crate::gamelayout::{layout, Game};
use crate::gameplay::DirLight;
use crate::ties::placement;

/// Directional light pairs (gameplay +0x04, 0x40-byte records, the R&C1 layout [C: Aranos, 5 records]).
pub fn lights(g: &[u8]) -> R<Vec<DirLight>> {
	let start = u32_at(g, 0x04)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let n = u32_at(g, start)? as usize;
	span(g, start + 16, n * 0x40)?;
	let v3 = |o: usize| -> R<[f32; 3]> { Ok([f32_at(g, o)?, f32_at(g, o + 4)?, f32_at(g, o + 8)?]) };
	(0..n.min(16))
		.map(|i| {
			let r = start + 16 + i * 0x40;
			Ok(DirLight { colour: [v3(r)?, v3(r + 0x20)?], dir: [v3(r + 0x10)?, v3(r + 0x30)?], wrap: [f32_at(g, r + 0x0c)?, f32_at(g, r + 0x2c)?] })
		})
		.collect()
}

/// A tie class header (0x70 bytes) in the layout `ties::tie_mesh_full` reads (R&C1's). R&C2 keeps the three LOD packet tables at
/// +0/4/8, their packet counts and the material count as bytes at +0xc..0xf (R&C1: +0x20..0x23) and the material records' offset at
/// +0x1c (R&C1: +0x2c); the translation moves them. R&C1 headers are returned as they are. [I] fields not listed are copied unchanged.
pub fn tie_header(game: Game, h: &[u8]) -> [u8; 0x70] {
	let mut o = [0u8; 0x70];
	let n = h.len().min(0x70);
	o[..n].copy_from_slice(&h[..n]);
	if game == Game::Rc2 && n >= 0x20 {
		o[0xc..0x10].fill(0);
		o[0x20..0x24].copy_from_slice(&h[0xc..0x10]);
		o[0x2c..0x30].copy_from_slice(&h[0x1c..0x20]);
	}
	o
}

/// Tie instances: gameplay +0x34 points to a count, then 0x60-byte records from +16 {class, draw distance, colour offset (0 on disc),
/// occlusion id, column-major matrix at +0x10 (w 0.01), +0x50 light selector [I]}: (class, matrix) per record.
pub fn tie_placements(g: &[u8]) -> R<Vec<(u32, [f32; 16])>> {
	let start = u32_at(g, 0x34)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	let size = layout(Game::Rc2).tie_record;
	span(g, start + 16, count * size)?.chunks_exact(size).map(|r| Ok((u32_at(r, 0)?, placement(r, "tie instance")?))).collect()
}

/// An R&C2 moby packet's vertex table header in R&C1's field order (transfers, two-joint, three-joint, main, duplicates,
/// transfer count, array offset, trailer offset): R&C2 stores the first seven as u16 [C: transfer = packet entry +15 on
/// every Aranos class] and no trailer. The epilogue after the array holds the cache ids of the last seven vertices (each
/// qword's low u16, then u16s from +4 of the last one) and is e qwords, the least e >= 1 with (n + e) % 4 == 3 [C: packets
/// with n = 49, 66, 72, 89, 93, 97 have e = 2, 1, 3, 2, 2, 2], so the trailer is array + (n + e) * 16.
pub fn moby_vertex_header(h: &[u8]) -> R<[usize; 8]> {
	let f = |i: usize| u16_at(h, i * 2).map(|v| v as usize);
	let (transfers, two, three, main, dupes, transfer, array) = (f(0)?, f(1)?, f(2)?, f(3)?, f(4)?, f(5)?, f(6)?);
	let n = two + three + main;
	let e = (3 + 4 - n % 4) % 4;
	let e = if e == 0 { 4 } else { e };
	Ok([transfers, two, three, main, dupes, transfer, array, array + (n + e) * 16])
}

/// Size of an R&C2 moby packet's vertex table header (R&C1: 0x20); the transfer list follows it.
pub const MOBY_VERTEX_HEADER: usize = 0x10;

/// Level fog from the level settings block (gameplay +0 pointer; R&C1's layout [C: Aranos +0x0c..0x24 = fog rgb 0x66 0x66
/// 0x81, near 61440, far 358400 (x1024), GS fog 255 at near, 140.25 at far]): (rgb 0..255, near, far in units, GS fog at
/// near, at far).
pub fn fog(g: &[u8]) -> R<([f32; 3], [f32; 4])> {
	let s = u32_at(g, 0)? as usize;
	let c = |o: usize| -> R<f32> { Ok(u32_at(g, s + o)? as u8 as f32) };
	Ok(([c(0x0c)?, c(0x10)?, c(0x14)?], [f32_at(g, s + 0x18)? / 1024.0, f32_at(g, s + 0x1c)? / 1024.0, f32_at(g, s + 0x20)?, f32_at(g, s + 0x24)?]))
}

/// The level's background colour (rgb 0..255) from the level settings block (gameplay +0 pointer, +0x00..0x0c, before the fog
/// colour): what the game clears the picture to where no sky shell or world covers it [I: level 12's abyss].
pub fn background(g: &[u8]) -> R<[u8; 3]> {
	let s = u32_at(g, 0)? as usize;
	let c = |o: usize| -> R<u8> { Ok(u32_at(g, s + o)? as u8) };
	Ok([c(0)?, c(4)?, c(8)?])
}
