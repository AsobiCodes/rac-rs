//! REN-2: the Veldin ship's environment-mapped reflection (class 530 update, 74 quads over 102 vertices). The three
//! source tables come from the level's overlay image (`Level::sections[0]`); the UV table is recomputed while the
//! camera is inside the 16 m box and blended back over 50 ticks when it leaves.

use super::*;

/// Veldin's overlay section holds the loaded image from virtual address 0x15ef70 (byte comparison of the table at
/// 0x1de730 against the extracted overlay.elf; Novalis's image starts at 0x15ef38).
const IMAGE_BASE: usize = 0x15ef70;
/// Overlay virtual addresses of the Veldin tables.
const POSITIONS: usize = 0x1de730;
const NORMALS: usize = 0x1df920;
const QUADS: usize = 0x1ded90;
pub(super) const VERTS: usize = 102;
pub(super) const QUAD_COUNT: usize = 74;
const QUAD_STRIDE: usize = 0x18;

/// The mesh tables: positions and normals (xyzw), the quads' four vertex indices.
#[derive(Clone)]
pub(super) struct ShipMesh {
}

impl ShipMesh {
	/// Reads the tables from a Veldin overlay image; `None` when the image is too short.
	pub(super) fn from_overlay(ov: &[u8]) -> Option<ShipMesh> {
		let at = |va: usize, len: usize| ov.get(va.checked_sub(IMAGE_BASE)?..)?.get(..len);
		let vec4 = |b: &[u8]| -> Vec<[f32; 4]> { b.chunks_exact(16).map(|c| [0, 4, 8, 12].map(|o| f32::from_le_bytes(c[o..o + 4].try_into().unwrap()))).collect() };
		let pos = vec4(at(POSITIONS, VERTS * 16)?);
		let nrm = vec4(at(NORMALS, VERTS * 16)?);
		let q = at(QUADS, QUAD_COUNT * QUAD_STRIDE)?;
		let quads: Vec<[usize; 4]> = q
			.chunks_exact(QUAD_STRIDE)
			.map(|r| [0, 6, 12, 18].map(|o| i16::from_le_bytes([r[o], r[o + 1]]).max(0) as usize % VERTS))
			.collect();
		Some(ShipMesh {   })
	}
}








impl Game {
	/// Gives the game the Veldin ship tables from the level overlay image (viewer load).
	pub fn set_ship_mesh(&mut self, overlay: &[u8]) {
		self.ship_mesh = ShipMesh::from_overlay(overlay);
	}


}

