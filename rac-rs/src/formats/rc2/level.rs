//! One R&C2 level from the disc image (G/LEVELn.WAD), in the shape of the shared `Level` (from lane rc2's `rc2/world/level.rs`).
//!
//! LEVELn.WAD [C, Aranos LEVEL0]: header 0x60 bytes {size, level, 0, 3 [?], then from +0x10 (sector, sectors) pairs}: primary,
//! sound bank, gameplay WAD, occlusion map. Primary: 0x80-byte header of (offset, size) pairs: overlay code, asset header (= R&C1
//! core index: same tables at the same offsets), GS RAM, HUD header, five HUD banks, assets (WAD, = R&C1 core data; index +0x8c its
//! decompressed size), and a last WAD block. The fourth pair is the occlusion map (`occlusion::OccMap::from_block`) [C: its
//! tie count equals the number of tie placements on levels 0 and 1].

use crate::bytes::{span, u32_at, R};
use crate::gamelayout::Game;
use crate::gameplay::DirLight;
use crate::level::{decoded, Level, SECTION_NAMES};
use crate::mesh::Mesh;

pub use super::iso9660::read_file;

/// Level `id` from the disc image at `iso` (G/LEVEL<id>.WAD).
pub fn load_level(iso: &str, id: usize) -> R<Level> {
	let raw = read_file(iso, &format!("G/LEVEL{id}.WAD"))?;
	let sec = |k: usize| -> R<&[u8]> {
		let (s, n) = (u32_at(&raw, 0x10 + k * 8)? as usize, u32_at(&raw, 0x14 + k * 8)? as usize);
		// The last block may end inside its last sector: the file is not padded to whole sectors [C: LEVEL10.WAD gameplay
		// sectors 0x1a63 + 0x11f end 0x1d bytes past the file's 0xdc0fe3].
		span(&raw, s * 0x800, (n * 0x800).min(raw.len().saturating_sub(s * 0x800)))
	};
	// Header pairs from +0x10 [C]: (sector, sectors) of the primary, the sound bank, gameplay and the occlusion map.
	let (primary, sound, gameplay) = (sec(0)?, sec(1)?, sec(2)?);
	let part = |k: usize| -> R<&[u8]> { span(primary, u32_at(primary, k * 8)? as usize, u32_at(primary, k * 8 + 4)? as usize) };
	let mut sections: Vec<Option<Vec<u8>>> = vec![None; SECTION_NAMES.len()];
	sections[0] = Some(part(0)?.to_vec());
	sections[1] = Some(sound.to_vec());
	let index = part(1)?.to_vec();
	sections[3] = Some(part(2)?.to_vec());
	for k in 0..6 {
		sections[4 + k] = Some(part(3 + k)?.to_vec());
	}
	let core = decoded(part(9)?)?;
	if core.len() != u32_at(&index, 0x8c)? as usize {
		return Err("core data size differs from its index".into());
	}
	let gameplay = decoded(gameplay)?;
	let occ_block = sec(3).ok().map(|b| b.to_vec());
	Level::from_parts(Game::Rc2, id, sections, index, core, gameplay, occ_block)
}

/// The level's terrain as plain meshes (one per fragment, LOD 0) with the level's directional lights baked into the vertex
/// colours, as lane rc2's world drew it: `terrain_lit(.., Some(lights))`, and the unlit `terrain` when that fails. The R&C2 pack
/// calls this for the terrain; `terrain::terrain_lods` is the shared fragment-LOD form (no baked lights without `lights`).
pub fn terrain_meshes(level: &Level) -> R<Vec<Mesh>> {
	let lights: Vec<DirLight> = super::gameplay::lights(&level.gameplay).unwrap_or_default();
	crate::terrain::terrain_lit(level.terrain_block()?, 0, Some(&lights)).or_else(|_| crate::terrain::terrain(level.terrain_block()?, 0))
}
