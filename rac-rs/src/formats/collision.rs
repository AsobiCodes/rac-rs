//! Level collision: the mesh the game's physics queries, separate from what is drawn.
//! Core index +0x14 points at a block of (tree offset, hero-groups offset), both block-relative.
//! The tree cuts the world into 4-unit cells, indexed z, then y, then x; every cell repeats the
//! faces that touch it. Layout: decomp/notes/collision_and_gameplay.md (Wrench's reader, the
//! game's CollInit / CollCellLookup), checked against Wrench's Veldin export.

use crate::bytes::{i16_at, span, u16_at, u32_at, R};
use crate::level::Level;
use crate::physics::V3;

pub const COLLISION_OFFSET: usize = 0x14;

pub struct Face {
	pub v: [V3; 3],
	/// Bits 0-4 surface id (31 default, 0 water, 1 lava, 11 death mud...), 5-6 sound class,
	/// bit 7 ignored by camera collision.
	pub kind: u8,
	/// The 4 m collision cell this copy was stored in (tree indices x, y, z).
	pub cell: [i32; 3],
}


fn sext(v: u32, bits: u32) -> i32 {
	((v << (32 - bits)) as i32) >> (32 - bits)
}

/// Every face of every cell, with triangles wound counter-clockwise seen from above.
/// Faces on cell borders appear once per cell, each copy with its own quantised vertices and its cell index
/// (`World::homed` keeps a copy visible only through its cell).
pub fn collision(level: &Level) -> R<Vec<Face>> {
	let start = u32_at(&level.index, COLLISION_OFFSET)? as usize;
	if start == 0 {
		return Ok(Vec::new());
	}
	let block = level.block(start)?;
	let tree = &block[u32_at(block, 0)? as usize..];
	let (z0, zn) = (i16_at(tree, 0)? as i32, u16_at(tree, 2)? as usize);
	let mut out = Vec::new();
	for zi in 0..zn {
		let zo = u16_at(tree, 4 + 2 * zi)? as usize * 4;
		if zo == 0 {
			continue;
		}
		let (y0, yn) = (i16_at(tree, zo)? as i32, u16_at(tree, zo + 2)? as usize);
		for yi in 0..yn {
			let yo = u32_at(tree, zo + 4 + 4 * yi)? as usize;
			if yo == 0 {
				continue;
			}
			let (x0, xn) = (i16_at(tree, yo)? as i32, u16_at(tree, yo + 2)? as usize);
			for xi in 0..xn {
				let off = (u32_at(tree, yo + 4 + 4 * xi)? >> 8) as usize;
				if off == 0 {
					continue;
				}
				let centre = [(x0 + xi as i32) * 4 + 2, (y0 + yi as i32) * 4 + 2, (z0 + zi as i32) * 4 + 2].map(|c| c as f32);
				cell(tree, off, centre, [x0 + xi as i32, y0 + yi as i32, z0 + zi as i32], &mut out)?;
			}
		}
	}
	Ok(out)
}

fn cell(tree: &[u8], off: usize, c: V3, index: [i32; 3], out: &mut Vec<Face>) -> R<()> {
	let (fc, vc, qc) = (u16_at(tree, off)? as usize, tree.get(off + 2).copied().ok_or("truncated cell")? as usize, tree.get(off + 3).copied().ok_or("truncated cell")? as usize);
	if qc > fc {
		return Err("collision cell has more quads than faces".into());
	}
	let verts: Vec<V3> = span(tree, off + 4, vc * 4)?
		.chunks_exact(4)
		.map(|w| {
			let w = u32::from_le_bytes([w[0], w[1], w[2], w[3]]);
			[c[0] + sext(w & 0x3ff, 10) as f32 / 16.0, c[1] + sext((w >> 10) & 0x3ff, 10) as f32 / 16.0, c[2] + sext(w >> 20, 12) as f32 / 64.0]
		})
		.collect();
	let faces = span(tree, off + 4 + vc * 4, fc * 4)?;
	let quads = span(tree, off + 4 + vc * 4 + fc * 4, qc)?;
	let v = |i: u8| verts.get(i as usize).copied().ok_or("collision face index out of range".to_string());
	for (j, f) in faces.chunks_exact(4).enumerate() {
		// Stored clockwise from above: reverse.
		out.push(Face { v: [v(f[2])?, v(f[1])?, v(f[0])?], kind: f[3], cell: index });
		if j < qc {
			out.push(Face { v: [v(quads[j])?, v(f[2])?, v(f[0])?], kind: f[3], cell: index });
		}
	}
	Ok(())
}

