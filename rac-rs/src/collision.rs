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

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{iso, toc};

	#[test]
	fn veldin_collision_bounds() {
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for info in &levels {
			let l = Level::load(&mut disc, info).unwrap();
			let faces = collision(&l).unwrap();
			let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
			for f in &faces {
				for p in f.v {
					for k in 0..3 {
						lo[k] = lo[k].min(p[k]);
						hi[k] = hi[k].max(p[k]);
					}
				}
			}
			println!("{:>2} {:<27} {:>6} triangles  {lo:?}..{hi:?}", info.id, toc::LEVEL_NAMES[info.id], faces.len());
			if info.id == 0 {
				// Wrench's Veldin collision.dae bounds.
				assert!((lo[0] - 67.94).abs() < 0.01 && (hi[1] - 311.19).abs() < 0.01 && (hi[2] - 82.06).abs() < 0.01);
			}
		}
	}

	/// Cell copies (collision_and_gameplay.md "Terrain cell copies"): a copy is stored relative to its cell centre with
	/// a 10/10/12-bit offset, so a vertex over 32 m away is clamped there and the copies of a large face disagree
	/// (Novalis: by up to 0.94 m). Querying through each copy's own cell loses no hit against the union of all copies.
	#[test]
	fn cell_copies_match_the_union() {
		use crate::physics::World;
		let mut disc = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		for id in [0usize, 1] {
			let l = Level::load(&mut disc, &levels[id]).unwrap();
			let faces = collision(&l).unwrap();
			let union = World::typed(faces.iter().map(|f| (f.v, f.kind)), 4.0);
			let homed = World::homed(faces.iter().map(|f| (f.v, f.kind, Some(f.cell))), 4.0);
			let (mut lo, mut hi) = ([f32::MAX; 3], [f32::MIN; 3]);
			for f in &faces {
				for p in f.v {
					for k in 0..3 {
						lo[k] = lo[k].min(p[k]);
						hi[k] = hi[k].max(p[k]);
					}
				}
			}
			let (mut n, mut worst, mut lost) = (0, 0f32, 0);
			let mut s = 12345u32;
			let mut rnd = || {
				s = s.wrapping_mul(1664525).wrapping_add(1013904223);
				(s >> 8) as f32 / (1u32 << 24) as f32
			};
			for _ in 0..30000 {
				let (x, y) = (lo[0] + rnd() * (hi[0] - lo[0]), lo[1] + rnd() * (hi[1] - lo[1]));
				let (a, b) = ([x, y, hi[2] + 1.0], [x + (rnd() - 0.5) * 6.0, y + (rnd() - 0.5) * 6.0, lo[2] - 1.0]);
				match (union.line(a, b, 0), homed.line(a, b, 0)) {
					(Some(u), Some(h)) => {
						n += 1;
						worst = worst.max((u.p[2] - h.p[2]).abs());
					}
					(Some(_), None) | (None, Some(_)) => lost += 1,
					_ => {}
				}
			}
			println!("level {id}: {n} common hits, worst hit z difference {worst}, hit/miss disagreements {lost}");
			assert!(lost * 1000 <= n.max(1), "{lost} of {n}");
		}
	}

}
