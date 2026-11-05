//! The cutscene / stream ship "gloss" overlay (retail callback FUN_002a69d8, decomp/notes/novalis_checklist.md
//! "NOV-14.5b port spec" and "NOV-14.5c"): an environment-mapped quad overlay on the four ship classes 0x212..0x215.
//! Data comes from the level overlay image (Novalis); one `ShipGloss` is the retail global state shared by all ships.

use crate::particles::WorldQuad;

/// The overlay image starts 0x38 bytes into the ELF load address (same rule as `water.rs`).
const IMAGE_BASE: usize = 0x15ef38;
const MAX_VERTS: usize = 102;
/// Per-class tables (pointer arrays in the overlay): colour words, vertex counts, quad counts, reflector (B),
/// position (A) and quad index (D) table pointers.
const COLOURS: usize = 0x160560;
const COUNTS: usize = 0x160570;
const QUADS: usize = 0x160580;
const TABLE_B: usize = 0x160590;
const TABLE_A: usize = 0x1605a0;
const TABLE_D: usize = 0x1605b0;

fn at(img: &[u8], va: usize, len: usize) -> Option<&[u8]> {
	if crate::retail::disc::usa() {
		return crate::retail::disc::ov_bytes(img, va, len);
	}
	img.get(va.checked_sub(IMAGE_BASE)?..)?.get(..len)
}

fn u32_at(img: &[u8], va: usize) -> Option<u32> {
	// `va` = a PAL table address (Novalis, level 1); `at` also reads the USA pointers stored in them.
	Some(u32::from_le_bytes(at(img, crate::retail::disc::va_level(1, va as u32) as usize, 4)?.try_into().ok()?))
}

struct Ship {
	/// Ship-local positions and reflectors (`x, y, z`; w is 1).
	a: Vec<[f32; 3]>,
	b: Vec<[f32; 3]>,
	/// Vertex index of each of the four corners of each quad.
	quads: Vec<[usize; 4]>,
	colour: [f32; 4],
}

pub struct ShipGloss {
	ships: Vec<Ship>,
	first: u8,
	armed: u8,
	ctr: i32,
	cur: [[f32; 2]; MAX_VERTS],
	prev: [[f32; 2]; MAX_VERTS],
	vtx: [[f32; 3]; MAX_VERTS],
}

impl ShipGloss {
	/// Read the four ships' tables from the level overlay image (None when the image does not hold them).
	pub fn new(overlay: &[u8]) -> Option<ShipGloss> {
		let mut ships = Vec::new();
		for c in 0..4 {
			let n = u32_at(overlay, COUNTS + 4 * c)? as usize;
			let nq = u32_at(overlay, QUADS + 4 * c)? as usize;
			if n > MAX_VERTS {
				return None;
			}
			let vecs = |table: usize| -> Option<Vec<[f32; 3]>> {
				let base = u32_at(overlay, table + 4 * c)? as usize;
				let raw = at(overlay, base, 16 * n)?;
				Some((0..n).map(|i| std::array::from_fn(|k| f32::from_le_bytes(raw[16 * i + 4 * k..16 * i + 4 * k + 4].try_into().unwrap()))).collect())
			};
			let d = u32_at(overlay, TABLE_D + 4 * c)? as usize;
			let raw = at(overlay, d, 16 * nq)?;
			let quads = (0..nq).map(|q| std::array::from_fn(|k| i16::from_le_bytes(raw[16 * q + 4 * k..16 * q + 4 * k + 2].try_into().unwrap()) as usize)).collect();
			let w = u32_at(overlay, COLOURS + 4 * c)?;
			let colour = std::array::from_fn(|k| ((w >> (8 * k)) & 0xff) as f32 / 128.0);
			ships.push(Ship { a: vecs(TABLE_A)?, b: vecs(TABLE_B)?, quads, colour });
		}
		Some(ShipGloss { ships, first: 1, armed: 1, ctr: 0, cur: [[0.0; 2]; MAX_VERTS], prev: [[0.0; 2]; MAX_VERTS], vtx: [[0.0; 3]; MAX_VERTS] })
	}

}
