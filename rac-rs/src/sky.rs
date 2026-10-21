//! The sky: shells of triangles around the camera.
//!
//! Layout: rac1-decomp tools/extract/sky.py (field meanings and the 1/1024 scale follow Wrench).
//! Header: +0 background RGB, +6 shell count, +0xc texture count, +0x10 texture defs and texture
//! data offsets, +0x20 shell offsets. A shell is (cluster count, flags) then 0x20-byte clusters,
//! each pointing at vertices, UVs or colours, and faces. Flag bit 0 marks an untextured shell whose
//! vertices carry RGBA colours instead of coordinates. The sky texture palettes live in the sky
//! block itself. The game centres the sky on the camera. Shells keep a fixed orientation, except on
//! Novalis, where shells 2, 3 and 4 turn about z with the frame counter (`shell_angle`, NOV-31 in
//! decomp/notes/novalis_environment.md).

use crate::bytes::{i16_at, span, u16_at, u32_at, R};
use crate::level::{Level, Texture, SKY_OFFSET};
use crate::mesh::Mesh;
use std::collections::HashMap;

/// Face key of an untextured face (the shell's vertex colours apply).
pub const UNTEXTURED: u32 = 4 << 16 | 0xffff;

pub struct Sky {
	/// One mesh per shell, to be drawn in order, each blended over the last.
	pub meshes: Vec<Mesh>,
	/// Per shell, per vertex RGBA (0..1). Alpha is the blend factor: PS2 alpha 0x80 is 1.0.
	/// Textured shells have white colours; the texel alpha multiplies this alpha.
	pub colours: Vec<Vec<[f32; 4]>>,
	/// Per shell: true when it has no textures and uses `colours` for its RGB.
	pub untextured: Vec<bool>,
	/// By key `4 << 16 | index`.
	pub textures: HashMap<u32, Texture>,
	/// RGBA shown where no shell covers; alpha is 255.
	pub colour: [u8; 4],
	/// ARI-4: header +8, the live sky sprite count (0 on disc; the Aridia sky pass sets it to 256 on its first frame).
	pub sprite_count: i16,
	/// ARI-4: header +0xa, the sprite buffer capacity (256 on Aridia, 0 on Veldin and Novalis).
	pub sprite_max: u16,
}

/// The sky of a level, or None when its core index has no sky block.
pub fn sky(level: &Level) -> R<Option<Sky>> {
	let start = u32_at(&level.index, if level.title { 0x14 } else { SKY_OFFSET })? as usize;
	if start == 0 {
		return Ok(None);
	}
	sky_block(level.block(start)?).map(Some)
}

/// An 8-bit texture with a CSM1 RGBA32 palette (index bits 3 and 4 swapped, alpha doubled).
fn texture(w: usize, h: usize, pixels: &[u8], palette: &[u8]) -> R<Texture> {
	if !(1..=4096).contains(&w) || !(1..=4096).contains(&h) {
		return Err("unsupported sky texture dimensions".into());
	}
	let colours: Vec<[u8; 4]> = (0..256)
		.map(|i| {
			let j = (i & !0x18) | ((i & 8) << 1) | ((i & 0x10) >> 1);
			let c = &palette[j * 4..j * 4 + 4];
			[c[0], c[1], c[2], (c[3] as u16 * 2).min(255) as u8]
		})
		.collect();
	Ok(Texture { w, h, rgba: pixels.iter().map(|&p| colours[p as usize]).collect() })
}

pub fn sky_block(data: &[u8]) -> R<Sky> {
	crate::viewer::gs_state::guard_path("sky");
	let shell_count = i16_at(data, 6)?;
	let texture_count = i16_at(data, 0xc)?;
	if !(0..=8).contains(&shell_count) || !(0..=256).contains(&texture_count) {
		return Err("invalid sky shell or texture count".into());
	}
	let (shell_count, texture_count) = (shell_count as usize, texture_count as usize);
	let defs = u32_at(data, 0x10)? as usize;
	let texture_data = u32_at(data, 0x14)? as usize;
	let mut textures = HashMap::new();
	for i in 0..texture_count {
		let at = defs.checked_add(i * 16).ok_or("invalid sky texture table")?;
		let palette = u32_at(data, at)? as usize;
		let pixels = u32_at(data, at + 4)? as usize;
		let (w, h) = (u32_at(data, at + 8)? as usize, u32_at(data, at + 12)? as usize);
		if !(1..=4096).contains(&w) || !(1..=4096).contains(&h) {
			return Err("unsupported sky texture dimensions".into());
		}
		let pixel_data = span(data, texture_data.checked_add(pixels).ok_or("invalid sky texture")?, w * h)?;
		let palette_data = span(data, texture_data.checked_add(palette).ok_or("invalid sky palette")?, 1024)?;
		textures.insert(4 << 16 | i as u32, texture(w, h, pixel_data, palette_data)?);
	}
	let head = span(data, 0, 3)?;
	let mut result = Sky {
		meshes: Vec::new(),
		colours: Vec::new(),
		untextured: Vec::new(),
		textures,
		colour: [head[0], head[1], head[2], 255],
		sprite_count: i16_at(data, 8)?,
		sprite_max: u16_at(data, 0xa)?,
	};
	for shell_id in 0..shell_count {
		let offset = u32_at(data, 0x20 + shell_id * 4)? as usize;
		let clusters = u32_at(data, offset)? as usize;
		let flags = u32_at(data, offset + 4)?;
		if clusters > 0x10000 {
			return Err("invalid sky cluster count".into());
		}
		let flat = flags & 1 != 0;
		let mut mesh = Mesh::new(format!("Sky_{shell_id}"));
		let mut colours = Vec::new();
		for cluster in 0..clusters {
			let at = offset + 0x20 + cluster * 0x20;
			let base = u32_at(data, at)? as usize;
			let nv = u16_at(data, at + 4)? as usize;
			let nf = u16_at(data, at + 6)? as usize;
			let vertices = u16_at(data, at + 8)? as usize;
			let extra = u16_at(data, at + 10)? as usize;
			let faces = u16_at(data, at + 12)? as usize;
			let size = u16_at(data, at + 14)? as usize;
			let block = span(data, base, size)?;
			let first = mesh.positions.len() as u32;
			let verts = span(block, vertices, nv * 8)?;
			let extras = span(block, extra, nv * 4)?;
			for (v, e) in verts.chunks_exact(8).zip(extras.chunks_exact(4)) {
				let alpha = i16_at(v, 6)?;
				if !(0..=0x80).contains(&alpha) {
					return Err("invalid sky vertex alpha".into());
				}
				mesh.positions.push([i16_at(v, 0)?, i16_at(v, 2)?, i16_at(v, 4)?].map(|c| c as f32 / 1024.0));
				if flat {
					mesh.uvs.push([0.0, 0.0]);
					colours.push([e[0] as f32 / 255.0, e[1] as f32 / 255.0, e[2] as f32 / 255.0, (e[3] as f32 / 128.0).min(1.0)]);
				} else {
					mesh.uvs.push([i16_at(e, 0)? as f32 / 4096.0, i16_at(e, 2)? as f32 / 4096.0]);
					colours.push([1.0, 1.0, 1.0, alpha as f32 / 128.0]);
				}
			}
			for f in span(block, faces, nf * 4)?.chunks_exact(4) {
				let (a, b, c, tex) = (f[0] as usize, f[1] as usize, f[2] as usize, f[3]);
				if a.max(b).max(c) >= nv || (tex != 0xff && tex as usize >= texture_count) {
					return Err("invalid sky face index or texture".into());
				}
				if tex != 0xff && flat {
					return Err("textured face on an untextured sky shell".into());
				}
				let key = if tex == 0xff { UNTEXTURED } else { 4 << 16 | tex as u32 };
				// Winding is reversed.
				mesh.faces.push((key, [first + c as u32, first + b as u32, first + a as u32]));
			}
		}
		result.meshes.push(mesh);
		result.colours.push(colours);
		result.untextured.push(flat);
	}
	Ok(result)
}


/// NOV-31: the z angle of sky shell `shell` at game frame `frames` (`Game.frames`), or None when that shell keeps the
/// fixed orientation. The Novalis sky pass (level 1) turns shells 2, 3 and 4 by 2 pi per 2^18, 2^17 and 2^16 frames
/// (decomp/notes/novalis_environment.md "NOV-31 port spec"); the Kerwan sky pass (level 3, KER-72a in
/// decomp/notes/kerwan_environment.md) turns shells 1 and 2 by 2 pi per 2^16 and 2^17 frames and keeps shell 0 fixed.
/// Every turning shell starts at -pi. All other levels draw their shells unturned.
pub fn shell_angle(level: usize, shell: usize, frames: u32) -> Option<f32> {
	let (mask, step) = match (level, shell) {
		(1, 2) => (0x3ffff, f32::from_bits(0x37c9_0fdb)),
		(1, 3) | (3, 2) => (0x1ffff, f32::from_bits(0x3849_0fdb)),
		(1, 4) | (3, 1) => (0xffff, f32::from_bits(0x38c9_0fdb)),
		_ => return None,
	};
	Some((frames & mask) as f32 * step - std::f32::consts::PI)
}

/// ARI-4 (decomp/notes/aridia_presentation.md "ARI-4 port spec"): one record of the Aridia sky sprite buffer (header +0x1c,
/// 0x20 bytes each). Created by the Aridia sky pass FUN_00289d78 on its first frame, updated each pass, drawn between sky
/// shells 1 and 2 by SkySpriteProc FUN_0028ad68.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SkySprite {
	/// +0: 3 static star, 0 moving light.
	pub kind: u8,
	/// +2: sky texture index (key `4 << 16 | tex`).
	pub tex: u8,
	/// +4: this frame's RGBA (R = low byte).
	pub colour: u32,
	/// +0xc (stars): the base colour the twinkle adds to.
	pub base: u32,
	/// +0xc, +0xe (lights): the two orbit phases.
	pub phase: [u16; 2],
	/// +8: corner rotation (radians).
	pub angle: f32,
	/// +0x10: camera-relative position (sky units, no translation).
	pub pos: [f32; 3],
	/// +0x1c: size.
	pub size: f32,
}

/// Records the Aridia sky pass creates (constant 0x100, not header +0xa); the first 246 are stars.
pub const SKY_SPRITES: usize = 256;
const TAN_Y: f32 = crate::camera::PROJ_SCALAR * f32::from_bits(0x3f41_8937);
const SEC_Y: f32 = 1.107_629_3;
/// View-space corner offset per unit size: 896 * 0.63 / 256 (the VU1 pixel offset `size * 896 / z` back in sky units).
const CORNER: f32 = f32::from_bits(0x400d_1eb8);







/// SkySpriteProc's frustum margins `[cx, cy]` for view-space `v` and size `s` with horizontal tangent / secant `tan_x`, `sec_x`.
pub fn cull_margins(v: [f32; 3], s: f32, tan_x: f32, sec_x: f32) -> [f32; 2] {
	[tan_x * v[2] - (v[0].abs() - sec_x * s), TAN_Y * v[2] - (v[1].abs() - SEC_Y * s)]
}


/// `sprite_drawn` with a horizontal tangent / secant of the port camera ([PC] widescreen).
pub fn sprite_drawn_with(v: [f32; 3], s: f32, tan_x: f32, sec_x: f32) -> bool {
	cull_margins(v, s, tan_x, sec_x).iter().all(|m| !m.is_sign_negative())
}

/// The four corners of a sprite (VU1 prog2 mode 0) in sky space, with ST: (0,0), (1,0), (0,1), (1,1); triangles
/// (0, 1, 2), (1, 2, 3). `right` / `up` are the camera axes; screen y grows downward, so the down axis is `-up`. [PC] the
/// PAL 1.0625 vertical stretch is dropped (square pixels).
pub fn sprite_quad(sp: &SkySprite, right: [f32; 3], up: [f32; 3]) -> [([f32; 3], [f32; 2]); 4] {
	let r = sp.size * CORNER;
	let (c, s) = (crate::camera::fast_cos(sp.angle), crate::camera::fast_sin(sp.angle));
	let d = up.map(|x| -x);
	let p = sp.pos;
	let at = |cr: f32, cd: f32| std::array::from_fn(|k| p[k] + r * (cr * right[k] + cd * d[k]));
	[(at(c, s), [0.0, 0.0]), (at(-s, c), [1.0, 0.0]), (at(s, -c), [0.0, 1.0]), (std::array::from_fn(|k| p[k] - r * (c * right[k] + s * d[k])), [1.0, 1.0])]
}

/// The drawn sprites in record order (pure: no rand()): view rows `view` (camera right, up, forward), corners from
/// `sprite_quad`, colour as RGBA / 128.
pub fn sprite_draws(sprites: &[SkySprite], view: [[f32; 3]; 3], right: [f32; 3], up: [f32; 3], tan_x: f32, sec_x: f32) -> Vec<(u8, [([f32; 3], [f32; 2]); 4], [f32; 4])> {
	sprites
		.iter()
		.filter(|sp| {
			let v = view.map(|row| row[0] * sp.pos[0] + row[1] * sp.pos[1] + row[2] * sp.pos[2]);
			(sp.tex as i8) >= 0 && sprite_drawn_with(v, sp.size, tan_x, sec_x)
		})
		.map(|sp| (sp.tex, sprite_quad(sp, right, up), sp.colour.to_le_bytes().map(|b| b as f32 / 128.0)))
		.collect()
}

/// `fan_mesh`'s position map alone.
pub fn fan_positions(positions: &[[f32; 3]], m: [[f32; 3]; 3], t: [f32; 3]) -> Vec<[f32; 3]> {
	positions.iter().map(|p| std::array::from_fn(|k| p[0] * m[0][k] + p[1] * m[1][k] + p[2] * m[2][k] + t[k])).collect()
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::{iso, toc};

	/// NOV-31 spec tests 2-5 (float32 values).
	#[test]
	fn novalis_shell_angles() {
		let pi = -std::f32::consts::PI;
		assert_eq!([2, 3, 4].map(|s| shell_angle(1, s, 0)), [Some(pi); 3]);
		assert_eq!([2, 3, 4].map(|s| shell_angle(1, s, 100000)), [Some(-0.74474764), Some(1.6520975), Some(0.16260195)]);
		assert_eq!([2, 3, 4].map(|s| shell_angle(1, s, 262149)), [Some(-3.1414728), Some(-3.1413531), Some(-3.1411133)]);
		assert!([0, 1, 5].iter().all(|&s| shell_angle(1, s, 12345).is_none()));
		assert!((0..6).all(|s| shell_angle(0, s, 100000).is_none() && shell_angle(2, s, 100000).is_none()));
	}

	/// KER-72a spec tests 1-4 (float32 values).
	#[test]
	fn kerwan_shell_angles() {
		let pi = -std::f32::consts::PI;
		assert_eq!(pi.to_bits(), 0xc049_0fdb);
		assert_eq!([1, 2].map(|s| shell_angle(3, s, 0)), [Some(pi); 2]);
		assert_eq!(shell_angle(3, 1, 32768).map(f32::to_bits), Some(0));
		assert_eq!(shell_angle(3, 1, 98304), shell_angle(3, 1, 32768));
		assert_eq!(shell_angle(3, 2, 98304).map(f32::to_bits), Some(0x3fc9_0fda));
		assert_eq!(shell_angle(3, 1, 65536 + 777), shell_angle(3, 1, 777));
		assert_eq!(shell_angle(3, 2, 131072 + 777), shell_angle(3, 2, 777));
		assert!([0, 3, 4].iter().all(|&s| shell_angle(3, s, 12345).is_none()));
	}

	/// KER-72a: the Kerwan sky has three shells in the retail shell-table order (header +0x20, the table the retail
	/// sky_shell_draw indexes): shell 0 the flat gouraud dome, shells 1 and 2 the textured rings that turn.
	#[test]
	fn kerwan_sky_shell_order() {
		let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = iso::Disc::open(path).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let kerwan = sky(&Level::load(&mut disc, levels.iter().find(|l| l.id == 3).unwrap()).unwrap()).unwrap().unwrap();
		assert_eq!(kerwan.untextured, vec![true, false, false]);
		assert_eq!(kerwan.meshes.iter().map(|m| m.faces.len()).collect::<Vec<_>>(), vec![768, 64, 64]);
		assert!(kerwan.meshes[1].faces.iter().all(|f| f.0 == 4 << 16));
		assert!(kerwan.meshes[2].faces.iter().all(|f| f.0 == 4 << 16 | 1));
	}

	/// ARI-4 spec test 8: the disc headers.
	#[test]
	fn aridia_sky_sprite_header() {
		let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = iso::Disc::open(path).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let get = |disc: &mut iso::Disc, id| sky(&Level::load(disc, levels.iter().find(|l| l.id == id).unwrap()).unwrap()).unwrap().unwrap();
		let aridia = get(&mut disc, 2);
		assert_eq!((aridia.sprite_count, aridia.sprite_max), (0, 256));
		assert!(aridia.textures.contains_key(&(4 << 16 | 2)) && aridia.textures.contains_key(&(4 << 16 | 3)));
		assert_eq!(get(&mut disc, 1).sprite_max, 0);
	}

	#[test]
	fn all_levels() {
		let path = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = iso::Disc::open(path).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let mut failed = 0;
		for info in &levels {
			let result = Level::load(&mut disc, info).and_then(|l| sky(&l));
			match result {
				Ok(Some(s)) => {
					let tris: usize = s.meshes.iter().map(|m| m.faces.len()).sum();
					println!(
						"level {}: shells {} (untextured {:?}), triangles {}, textures {}, colour {:?}",
						info.id, s.meshes.len(), s.untextured, tris, s.textures.len(), s.colour
					);
				}
				Ok(None) => println!("level {}: no sky", info.id),
				Err(e) => {
					failed += 1;
					println!("level {}: error {e}", info.id);
				}
			}
		}
		assert_eq!(failed, 0);
	}

	fn fixture(textured: bool) -> Vec<u8> {
		let mut d = vec![0u8; 0x542];
		d[6] = 1;
		d[0xc] = 1;
		d[0x10..0x14].copy_from_slice(&0x50u32.to_le_bytes());
		d[0x14..0x18].copy_from_slice(&0x140u32.to_le_bytes());
		d[0x20..0x24].copy_from_slice(&0x80u32.to_le_bytes());
		for (i, v) in [0u32, 1024, 2, 1].iter().enumerate() {
			d[0x50 + i * 4..0x54 + i * 4].copy_from_slice(&v.to_le_bytes());
		}
		d[0x80..0x84].copy_from_slice(&1u32.to_le_bytes());
		d[0x84..0x88].copy_from_slice(&(if textured { 0u32 } else { 1 }).to_le_bytes());
		d[0xa0..0xa4].copy_from_slice(&0xc0u32.to_le_bytes());
		for (i, v) in [3u16, 1, 0, 24, 36, 40].iter().enumerate() {
			d[0xa4 + i * 2..0xa6 + i * 2].copy_from_slice(&v.to_le_bytes());
		}
		for (i, v) in [0i16, 0, 0, 128, 1024, 0, 0, 64, 0, 1024, 0, 0].iter().enumerate() {
			d[0xc0 + i * 2..0xc2 + i * 2].copy_from_slice(&v.to_le_bytes());
		}
		if textured {
			for (i, v) in [0i16, 0, 4096, 0, 0, 4096].iter().enumerate() {
				d[0xd8 + i * 2..0xda + i * 2].copy_from_slice(&v.to_le_bytes());
			}
		} else {
			d[0xd8..0xe4].copy_from_slice(&[255, 0, 0, 128, 0, 255, 0, 64, 0, 0, 255, 0]);
		}
		d[0xe4..0xe8].copy_from_slice(&[0, 1, 2, if textured { 0 } else { 255 }]);
		d[0x140..0x148].copy_from_slice(&[255, 0, 0, 128, 0, 255, 0, 0]);
		d[0x541] = 1;
		d
	}

	#[test]
	fn fixtures() {
		let s = sky_block(&fixture(true)).unwrap();
		assert_eq!(s.meshes[0].positions, vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]]);
		assert_eq!(s.meshes[0].faces, vec![(4 << 16, [2, 1, 0])]);
		assert_eq!(s.colours[0].iter().map(|c| c[3]).collect::<Vec<_>>(), vec![1.0, 0.5, 0.0]);
		assert_eq!(s.textures[&(4 << 16)].rgba[1][..2], [0, 255]);
		let flat = sky_block(&fixture(false)).unwrap();
		assert_eq!(flat.meshes[0].faces[0].0, UNTEXTURED);
		for (offset, value) in [(0xe4, 3u8), (0xe7, 1), (0xc6, 0x81)] {
			let mut d = fixture(true);
			d[offset] = value;
			assert!(sky_block(&d).is_err());
		}
		let mut d = fixture(false);
		d[0xe7] = 0;
		assert!(sky_block(&d).is_err());
	}
}
