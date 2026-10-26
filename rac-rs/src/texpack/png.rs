//! PNG replacement textures. PCSX2 reads 8-bit RGB (alpha 0x80, the PS2 "opaque") and RGBA (alpha as is, PS2 scale 0..0x80); this
//! also reads grey, grey + alpha, palette and 16-bit files, not interlaced. The result is RGBA8 in the engine's alpha scale
//! (`min(a * 2, 255)`, like `Texture::new`), see `dds::engine_alpha`.

use super::dds::{engine_alpha, Fmt, Image};
use super::inflate;

fn be32(b: &[u8]) -> u32 {
	u32::from_be_bytes([b[0], b[1], b[2], b[3]])
}

struct Header {
	w: usize,
	h: usize,
	depth: usize,
	kind: u8,
}

impl Header {
	/// Samples per pixel.
	fn channels(&self) -> usize {
		match self.kind {
			0 | 3 => 1,
			2 => 3,
			4 => 2,
			_ => 4,
		}
	}

	fn row_bytes(&self) -> usize {
		(self.w * self.channels() * self.depth).div_ceil(8)
	}
}

fn unfilter(h: &Header, raw: &[u8]) -> Result<Vec<u8>, String> {
	let (row, bpp) = (h.row_bytes(), (h.channels() * h.depth).div_ceil(8));
	if raw.len() < (row + 1) * h.h {
		return Err("png: image data too short".into());
	}
	let mut out = vec![0u8; row * h.h];
	for y in 0..h.h {
		let f = raw[y * (row + 1)];
		let src = &raw[y * (row + 1) + 1..(y + 1) * (row + 1)];
		let (before, cur) = out.split_at_mut(y * row);
		let prev = if y > 0 { Some(&before[(y - 1) * row..]) } else { None };
		for x in 0..row {
			let a = if x >= bpp { cur[x - bpp] as i32 } else { 0 };
			let b = prev.map_or(0, |p| p[x] as i32);
			let c = if x >= bpp { prev.map_or(0, |p| p[x - bpp] as i32) } else { 0 };
			let add = match f {
				0 => 0,
				1 => a,
				2 => b,
				3 => (a + b) / 2,
				4 => {
					let p = a + b - c;
					let (pa, pb, pc) = ((p - a).abs(), (p - b).abs(), (p - c).abs());
					if pa <= pb && pa <= pc { a } else if pb <= pc { b } else { c }
				}
				_ => return Err("png: bad filter".into()),
			};
			cur[x] = src[x].wrapping_add(add as u8);
		}
	}
	Ok(out)
}

/// The sample `i` of a row (1, 2, 4, 8 or 16 bits) as 0..255 (palette index for colour type 3).
fn sample(row: &[u8], i: usize, depth: usize, index: bool) -> u8 {
	match depth {
		8 => row[i],
		16 => row[2 * i],
		d => {
			let bit = i * d;
			let v = (row[bit / 8] >> (8 - d - bit % 8)) & ((1 << d) - 1);
			if index { v } else { (v as u32 * 255 / ((1 << d) - 1)) as u8 }
		}
	}
}

/// Decodes a PNG file to RGBA8 (engine alpha scale).
pub fn load(file: &[u8]) -> Result<Image, String> {
	if file.get(..8) != Some(&[0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a]) {
		return Err("png: bad signature".into());
	}
	let (mut at, mut head, mut pal, mut trns, mut idat) = (8, None, Vec::new(), Vec::new(), Vec::new());
	while at + 8 <= file.len() {
		let (len, kind) = (be32(&file[at..]) as usize, &file[at + 4..at + 8]);
		let body = file.get(at + 8..at + 8 + len).ok_or("png: chunk ends early")?;
		match kind {
			b"IHDR" if len >= 13 => {
				head = Some(Header { w: be32(body) as usize, h: be32(&body[4..]) as usize, depth: body[8] as usize, kind: body[9] });
				if body[12] != 0 {
					return Err("png: interlaced files are not supported".into());
				}
			}
			b"PLTE" => pal = body.to_vec(),
			b"tRNS" => trns = body.to_vec(),
			b"IDAT" => idat.extend_from_slice(body),
			b"IEND" => break,
			_ => {}
		}
		at += 12 + len;
	}
	let h = head.ok_or("png: no IHDR")?;
	if h.w == 0 || h.h == 0 || h.w > 8192 || h.h > 8192 || !matches!(h.depth, 1 | 2 | 4 | 8 | 16) || !matches!(h.kind, 0 | 2 | 3 | 4 | 6) {
		return Err(format!("png: unsupported {}x{} depth {} colour type {}", h.w, h.h, h.depth, h.kind));
	}
	if h.kind == 3 && pal.len() < 3 {
		return Err("png: no palette".into());
	}
	let raw = inflate::zlib(&idat, (h.row_bytes() + 1) * h.h)?;
	let px = unfilter(&h, &raw)?;
	let row = h.row_bytes();
	let mut rgba = Vec::with_capacity(h.w * h.h * 4);
	for y in 0..h.h {
		let r = &px[y * row..(y + 1) * row];
		for x in 0..h.w {
			let s = |c: usize| sample(r, x * h.channels() + c, h.depth, false);
			let (rgb, a) = match h.kind {
				0 => ([s(0); 3], None),
				2 => ([s(0), s(1), s(2)], None),
				3 => {
					let i = sample(r, x, h.depth, true) as usize;
					let c = pal.get(i * 3..i * 3 + 3).ok_or("png: palette index out of range")?;
					([c[0], c[1], c[2]], Some(trns.get(i).copied().unwrap_or(255)))
				}
				4 => ([s(0); 3], Some(s(1))),
				_ => ([s(0), s(1), s(2)], Some(s(3))),
			};
			// RGB files carry no alpha: PCSX2 gives them 0x80; palette alpha (tRNS) is on the 0..255 scale and is taken as the PS2 value too.
			rgba.extend_from_slice(&[rgb[0], rgb[1], rgb[2], engine_alpha(a.unwrap_or(0x80))]);
		}
	}
	Ok(Image { fmt: Fmt::Rgba8, w: h.w as u32, h: h.h as u32, levels: vec![rgba] })
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A 2x2 RGBA PNG made with Python (zlib.compress of the filtered rows), filters None / Sub / Up / Paeth.
	fn png(w: u32, h: u32, kind: u8, raw: &[u8]) -> Vec<u8> {
		let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
		let mut chunk = |k: &[u8], d: &[u8]| {
			out.extend((d.len() as u32).to_be_bytes());
			out.extend(k);
			out.extend(d);
			out.extend([0u8; 4]);
		};
		let mut ih = Vec::new();
		ih.extend(w.to_be_bytes());
		ih.extend(h.to_be_bytes());
		ih.extend([8, kind, 0, 0, 0]);
		chunk(b"IHDR", &ih);
		// A stored deflate block: the pixel data is the test, not the compressor.
		let mut z = vec![0x78, 0x01, 1];
		z.extend((raw.len() as u16).to_le_bytes());
		z.extend((!(raw.len() as u16)).to_le_bytes());
		z.extend(raw);
		z.extend([0u8; 4]);
		chunk(b"IDAT", &z);
		chunk(b"IEND", &[]);
		out
	}

	#[test]
	fn rgba_and_rgb_alpha_rules() {
		// Row 0 filter 0, row 1 filter 2 (Up): second row = first row + deltas.
		let raw = [0, 10, 20, 30, 64, 40, 50, 60, 128, 2, 1, 1, 1, 0, 1, 1, 1, 0];
		let im = png(2, 2, 6, &raw);
		let i = load(&im).unwrap();
		assert_eq!((i.w, i.h, i.fmt), (2, 2, Fmt::Rgba8));
		// PS2 alpha 64 -> 128, 128 -> 255; the second row (Up) adds 1,1,1,0 to the first.
		assert_eq!(&i.levels[0][..8], &[10, 20, 30, 128, 40, 50, 60, 255]);
		assert_eq!(&i.levels[0][8..12], &[11, 21, 31, 128]);
		let rgb = png(1, 1, 2, &[0, 7, 8, 9]);
		assert_eq!(load(&rgb).unwrap().levels[0], vec![7, 8, 9, 255], "RGB gets the opaque alpha");
	}

	#[test]
	fn refuses_bad_files() {
		assert!(load(b"not a png").is_err());
		let mut im = png(1, 1, 6, &[0, 1, 2, 3, 4]);
		im[28] = 1; // interlaced flag in IHDR
		assert!(load(&im).is_err());
	}
}
