//! Galactic Map bundle decoder and explored-mask compositor.
//! Owner: decomp/notes/ingame_menu_leftovers.md P1-c (and ingame_other_screens.md s1).
//! ToC +0x820 holds one {sector, sizeSectors} per level; the bundle is WAD-LZ. After decompression a
//! header of 7 offsets points at sections: sec1 = save region spans, sec2 = revealed 512x512 image, sec3 = fogged 512x512
//! image (both raw 8bpp indices, no header), sec4..6 = 128x128 PIF2 images (0x20 header, 256x4 CLUT at
//! +0x20, pixels at +0x420). The 512x512 images use the CLUT from the sec4 PIF2 [I].
//! Every planet uses the retail anchor affine (ingame_other_screens.md s1 [C]). The reveal stamp is the retail 32x32 template, d^2 <= 256 about (15.5, 15.5) [C].

use crate::bytes::{span, u32_at, R};
use crate::hud::palette_rgba;
use crate::iso::Disc;
use crate::level::Texture;

const MAP_TABLE: usize = 0x820;
const SIDE: usize = 512;
const STAMP_RADIUS: f32 = 16.0;

/// The authored two-anchor table V 0x182810 / N 0x182c10 (decomp/notes/ingame_other_screens.md s1): per
/// planet id, world anchors A, B then map anchors a, b (map pixels of 512).
const ANCHORS: [[f32; 8]; 19] = [
	[140.9, 136.1, 361.6, 303.3, 81.0, 346.0, 393.0, 109.0],
	[61.2, 202.6, 291.0, 59.5, 77.5, 189.0, 454.5, 420.0],
	[262.0, 96.0, 51.7, 265.75, 391.5, 421.0, 29.0, 127.0],
	[119.6, 87.5, 336.8, 239.8, 109.5, 489.5, 388.5, 293.5],
	[336.3, 314.6, 161.88, 93.8, 447.0, 62.0, 132.0, 453.0],
	[334.25, 343.0, 176.0, 205.25, 320.0, 86.0, 54.0, 316.5],
	[135.2, 166.3, 317.9, 383.6, 35.5, 390.5, 289.5, 88.5],
	[252.7, 81.24, 151.5, 485.1, 250.0, 469.0, 136.5, 14.5],
	[314.5, 92.4, 135.7, 303.8, 336.0, 390.0, 55.0, 60.0],
	[388.8, 215.3, 167.0, 381.0, 449.0, 363.0, 90.0, 90.0],
	[171.9, 345.7, 341.0, 120.4, 125.0, 71.0, 407.0, 452.0],
	[446.4, 592.9, 588.7, 404.6, 142.0, 116.0, 385.0, 445.0],
	[205.6, 268.5, 427.7, 457.5, 128.5, 472.5, 486.5, 168.5],
	[391.6, 399.6, 605.7, 636.0, 55.0, 437.5, 396.5, 60.0],
	[115.5, 340.7, 311.6, 101.8, 93.5, 56.0, 378.0, 403.0],
	[120.8, 49.1, 312.5, 381.9, 59.0, 497.0, 335.0, 23.5],
	[123.4, 351.5, 363.0, 37.0, 109.5, 38.5, 462.0, 500.5],
	[695.3, 169.0, 306.6, 541.5, 443.5, 491.0, 50.0, 114.0],
	[340.6, 648.4, 561.3, 815.6, 81.0, 346.0, 393.0, 109.0],
];

/// Map pixel = (ax + bx*x, ay + by*y) from the planet's anchor pairs, in f32 as the game computes it:
/// bx = (a.x - b.x) / (A.x - B.x), ax = a.x - bx * A.x (same for y). Nebula G34's alternate swapped-axis
/// orientation in FUN_0024a3d0 (u = 1053 + by*y, v = 740 - bx*x, marker yaw + pi/2) is switched on only by a
/// Nebula updater, so it waits for that level's port.
pub fn retail_affine(level: usize) -> Option<[f32; 4]> {
	let [wx0, wy0, wx1, wy1, mx0, my0, mx1, my1] = *ANCHORS.get(level)?;
	let bx = (mx0 - mx1) / (wx0 - wx1);
	let by = (my0 - my1) / (wy0 - wy1);
	Some([mx0 - bx * wx0, bx, my0 - by * wy0, by])
}

pub fn affine(a: [f32; 4], p: [f32; 2]) -> [f32; 2] {
	[a[0] + a[1] * p[0], a[2] + a[3] * p[1]]
}

pub struct MapBundle {
	pub revealed: Texture,
	pub fogged: Texture,
	/// The save codec's region spans (save.rs `explore_encode`, retail DAT_00184604): section 1, byte pairs
	/// {skip, count} ending where the pixel total reaches 512 * 512, plus a {0, 0} guard.
	pub spans: Vec<u8>,
}

/// Section 1 cut at the pair whose pixel total reaches 0x40000.
fn region_spans(data: &[u8], off: usize) -> R<Vec<u8>> {
	let mut out = Vec::new();
	let mut sum = 0usize;
	let mut k = off;
	while sum < SIDE * SIDE {
		let b = *data.get(k).ok_or("map section 1 ends early")?;
		sum += b as usize;
		out.push(b);
		k += 1;
	}
	if out.len() % 2 == 1 {
		out.push(0);
	}
	out.extend([0, 0]);
	Ok(out)
}

/// Sections 2 and 3 are headerless 512x512 8bpp index images; the shared CLUT lives in the PIF2
/// header of the 128x128 images (sections 4..6, identical palettes).
fn indexed(data: &[u8], off: usize, pal: &[[u8; 4]]) -> R<Texture> {
	let px = span(data, off, SIDE * SIDE)?;
	Ok(Texture { w: SIDE, h: SIDE, rgba: px.iter().map(|&i| pal[i as usize]).collect() })
}

pub fn load(disc: &mut Disc, level: usize) -> R<Option<MapBundle>> {
	let toc = crate::toc::read_toc(disc).map_err(|e| e.to_string())?;
	let sector = u32_at(&toc, MAP_TABLE + level * 8)? as u64;
	let size = u32_at(&toc, MAP_TABLE + level * 8 + 4)? as u64;
	if size == 0 {
		return Ok(None);
	}
	let raw = disc.read_sectors(sector, size).map_err(|e| e.to_string())?;
	let data = if crate::wad::is_wad(&raw) { crate::wad::decompress(&raw).map_err(|e| e.to_string())? } else { raw };
	let sec1 = u32_at(&data, 4)? as usize;
	let sec2 = u32_at(&data, 8)? as usize;
	let sec3 = u32_at(&data, 12)? as usize;
	let sec4 = u32_at(&data, 16)? as usize;
	if span(&data, sec4, 4)? != b"2FIP" {
		return Err("map section 4 is not a PIF2 image".into());
	}
	let pal = palette_rgba(span(&data, sec4 + 0x20, 0x400)?);
	Ok(Some(MapBundle { revealed: indexed(&data, sec2, &pal)?, fogged: indexed(&data, sec3, &pal)?, spans: region_spans(&data, sec1)? }))
}

/// Per-planet inset of the globe inside its 256x256 render target, in 1/16 px (table at overlay
/// 0x1c2068, FUN_0027de30; decomp/notes/ingame_pause_menu.md "Pause presentation").
pub const GLOBE_INSET: [i32; LEVEL_COUNT] = [1200, 1000, 1300, 400, 1100, 900, 0, 1200, 700, 190, 800, 1550, 300, 0, 600, 900, 500, 0, 1200];
const LEVEL_COUNT: usize = crate::toc::LEVEL_COUNT;

/// The three 128x128 globe layers (map bundle sections 4..6, FUN_0027de30's DAT_00184448/50/58,
/// all with the section-4 CLUT). Layer 0 is opaque (it scrolls); layers 1 and 2 are alpha tested
/// (GS TEST 0x360b: alpha >= 0x60 of 0x80) [I].
pub fn load_globe(disc: &mut Disc, level: usize) -> R<Option<[Texture; 3]>> {
	if level >= LEVEL_COUNT {
		return Ok(None);
	}
	let toc = crate::toc::read_toc(disc).map_err(|e| e.to_string())?;
	let sector = u32_at(&toc, MAP_TABLE + level * 8)? as u64;
	let size = u32_at(&toc, MAP_TABLE + level * 8 + 4)? as u64;
	if size == 0 {
		return Ok(None);
	}
	let raw = disc.read_sectors(sector, size).map_err(|e| e.to_string())?;
	let data = if crate::wad::is_wad(&raw) { crate::wad::decompress(&raw).map_err(|e| e.to_string())? } else { raw };
	let sec4 = u32_at(&data, 16)? as usize;
	let pal = palette_rgba(span(&data, sec4 + 0x20, 0x400)?);
	let mut out = Vec::new();
	for (n, at) in [16usize, 20, 24].into_iter().enumerate() {
		let off = u32_at(&data, at)? as usize;
		let px = span(&data, off + 0x420, 128 * 128)?;
		let rgba = px
			.iter()
			.map(|&i| {
				let c = pal[i as usize];
				if n == 0 { [c[0], c[1], c[2], 255] } else if c[3] >= 0xc0 { c } else { [c[0], c[1], c[2], 0] }
			})
			.collect();
		out.push(Texture { w: 128, h: 128, rgba });
	}
	Ok(out.try_into().ok())
}

/// 1 bit per map pixel, LSB first, row-major; set = revealed.
pub struct Explored(pub Vec<u8>);

impl Explored {


	/// The 32x32 template placed with its top-left at (x-16, y-16): pixel (i, j) is revealed when
	/// (i-15.5)^2 + (j-15.5)^2 <= 256 [C]; the integer placement of the template is [I].
	pub fn stamp(&mut self, x: f32, y: f32) {
		let (x0, y0) = ((x - STAMP_RADIUS).round() as i32, (y - STAMP_RADIUS).round() as i32);
		for j in 0..32 {
			for i in 0..32 {
				let (dx, dy) = (i as f32 - 15.5, j as f32 - 15.5);
				let (px, py) = (x0 + i, y0 + j);
				if dx * dx + dy * dy <= STAMP_RADIUS * STAMP_RADIUS && (0..SIDE as i32).contains(&px) && (0..SIDE as i32).contains(&py) {
					let k = py as usize * SIDE + px as usize;
					self.0[k >> 3] |= 1 << (k & 7);
				}
			}
		}
	}
}

pub fn composite(b: &MapBundle, m: &Explored) -> Texture {
	let rgba = (0..SIDE * SIDE)
		.map(|i| if m.0[i >> 3] >> (i & 7) & 1 != 0 { b.revealed.rgba[i] } else { b.fogged.rgba[i] })
		.collect();
	Texture { w: SIDE, h: SIDE, rgba }
}

/// Fit lo..hi (world xy) onto 0..512 keeping aspect, centred; map y grows downward [P].
pub fn world_to_map(lo: [f32; 2], hi: [f32; 2], p: [f32; 2]) -> [f32; 2] {
	let (w, h) = ((hi[0] - lo[0]).max(1e-6), (hi[1] - lo[1]).max(1e-6));
	let s = SIDE as f32 / w.max(h);
	let (ox, oy) = ((SIDE as f32 - w * s) * 0.5, (SIDE as f32 - h * s) * 0.5);
	[ox + (p[0] - lo[0]) * s, oy + (hi[1] - p[1]) * s]
}

#[cfg(test)]
mod tests {
	use super::*;

}
