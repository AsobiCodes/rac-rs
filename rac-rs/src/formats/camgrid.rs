//! Camera collision grid (gameplay header +0x84) and the shape tables it indexes
//! (decomp/notes/camera.md "Camera collision grid"). Ports FUN_001ee130 (grid query),
//! FUN_001ee298 (narrow phase) and FUN_002e3b28 (sphere push). Separate from the render occlusion grid.
//!
//! Shape tables in the gameplay header: +0x60 cuboids, +0x64 spheres, +0x68 cylinders (0x80-byte
//! records), +0x6c pills (0x90-byte records); count at the section start, records from +0x10.

use crate::bytes::{f32_at, span, u32_at, R};
use crate::physics::V3;

const GRID: usize = 0x84;
const CYLINDERS: usize = 0x68;


/// One shape record. `center` is +0x30, `inverse` the world-to-unit matrix at +0x40 (rows), `axes` the
/// local-to-world rows at +0 (pills use row 2 for the cap offsets, spheres use |row 0| as radius).
#[derive(Clone, Debug)]
struct Shape {
	axes: [V3; 3],
	center: V3,
	inverse: [V3; 3],
}


#[derive(Clone, Default)]
pub struct CamGrid {
	/// Section data from +0x10: 0x4000 bytes of cell offsets, then the lists.
	data: Vec<u8>,
	cylinders: Vec<Shape>,
}

fn table(g: &[u8], at: usize, stride: usize) -> R<Vec<Shape>> {
	let start = u32_at(g, at)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let count = u32_at(g, start)? as usize;
	let rows = span(g, start + 16, count * stride)?;
	rows.chunks_exact(stride)
		.map(|r| {
			let row = |o: usize| -> R<V3> { Ok([f32_at(r, o)?, f32_at(r, o + 4)?, f32_at(r, o + 8)?]) };
			Ok(Shape {
				axes: [row(0)?, row(0x10)?, row(0x20)?],
				center: row(0x30)?,
				inverse: [row(0x40)?, row(0x50)?, row(0x60)?],
			})
		})
		.collect()
}




impl CamGrid {
	/// Parses the grid and shape tables from the gameplay section. A level without a grid gives an
	/// empty one (no hits).
	pub fn parse(g: &[u8]) -> R<CamGrid> {
		let start = u32_at(g, GRID)? as usize;
		let mut out = CamGrid {
			cylinders: table(g, CYLINDERS, 0x80)?,
			data: Vec::new(),
		};
		if start != 0 {
			let size = u32_at(g, start)? as usize;
			out.data = span(g, start + 16, size + 0x4000)?.to_vec();
		}
		Ok(out)
	}

	/// ARI-22 / BRIDGE-59: cylinder record `k` as (centre +0x30, axes rows +0/+0x10/+0x20, inverse rows +0x40/+0x50/+0x60).
	pub fn cylinder(&self, k: usize) -> Option<(V3, [V3; 3], [V3; 3])> {
		self.cylinders.get(k).map(|s| (s.center, s.axes, s.inverse))
	}

	/// The number of cylinder records.
	pub fn cylinder_count(&self) -> usize {
		self.cylinders.len()
	}




}

#[cfg(test)]
mod tests {
	use super::*;

	fn put(v: &mut Vec<u8>, at: usize, f: &[f32]) {
		for (i, x) in f.iter().enumerate() {
			v[at + i * 4..at + i * 4 + 4].copy_from_slice(&x.to_le_bytes());
		}
	}

}
