//! Block-compressed texture helpers: CPU decode of BC1 / BC2 / BC3 to RGBA8 (the path for GPUs without
//! `TEXTURE_COMPRESSION_BC`, and the source of generated mip levels) and the alpha re-coding that moves a pack texture's PS2 alpha
//! scale (0x80 = opaque) to the engine's (`min(a * 2, 255)`) without touching the colour blocks.

fn rgb565(c: u16) -> [u8; 3] {
	let (r, g, b) = ((c >> 11) as u8 & 31, (c >> 5) as u8 & 63, c as u8 & 31);
	[r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2]
}

/// The 16 texels of a colour block (8 bytes) as RGBA; alpha 0 only for a transparent texel of BC1's 3 colour mode (BC2 / BC3 blocks
/// always use the 4 colour mode).
fn bc1_block(b: &[u8], dxt1: bool) -> [[u8; 4]; 16] {
	let (c0, c1) = (u16::from_le_bytes([b[0], b[1]]), u16::from_le_bytes([b[2], b[3]]));
	let (p0, p1) = (rgb565(c0), rgb565(c1));
	let mix = |a: u32, x: u32, y: u32, d: u32| -> [u8; 4] {
		let f = |i: usize| ((a * p0[i] as u32 + x * p1[i] as u32 + y) / d) as u8;
		[f(0), f(1), f(2), 255]
	};
	let pal = if c0 > c1 || !dxt1 { [mix(1, 0, 0, 1), [p1[0], p1[1], p1[2], 255], mix(2, 1, 0, 3), mix(1, 2, 0, 3)] } else { [[p0[0], p0[1], p0[2], 255], [p1[0], p1[1], p1[2], 255], mix(1, 1, 0, 2), [0, 0, 0, 0]] };
	// mix(1, 0, 0, 1) is p0; the second term of `mix` weights p1, so (2, 1) = (2 p0 + p1) / 3.
	let bits = u32::from_le_bytes([b[4], b[5], b[6], b[7]]);
	std::array::from_fn(|i| pal[(bits >> (2 * i)) as usize & 3])
}

/// The 8 alpha values a BC3 alpha block (a0, a1 and 16 3-bit indices) can take.
fn bc3_table(a0: u8, a1: u8) -> [u8; 8] {
	let (x, y) = (a0 as u32, a1 as u32);
	let mut t = [a0, a1, 0, 0, 0, 0, 0, 0];
	if a0 > a1 {
		for k in 1..7 {
			t[k + 1] = (((7 - k as u32) * x + k as u32 * y) / 7) as u8;
		}
	} else {
		for k in 1..5 {
			t[k + 1] = (((5 - k as u32) * x + k as u32 * y) / 5) as u8;
		}
		t[6] = 0;
		t[7] = 255;
	}
	t
}

fn bc3_bits(b: &[u8]) -> u64 {
	b[2..8].iter().enumerate().fold(0u64, |a, (i, &v)| a | (v as u64) << (8 * i))
}

/// Decodes a `w` x `h` BC1 / BC2 / BC3 level to RGBA8 (rows top to bottom; the last block row / column is cut to the size).
pub fn decode(fmt: super::dds::Fmt, w: u32, h: u32, data: &[u8]) -> Vec<u8> {
	use super::dds::Fmt;
	let (bw, bh) = (w.div_ceil(4) as usize, h.div_ceil(4) as usize);
	let mut out = vec![0u8; w as usize * h as usize * 4];
	for by in 0..bh {
		for bx in 0..bw {
			let at = (by * bw + bx) * fmt.block_bytes();
			let Some(b) = data.get(at..at + fmt.block_bytes()) else { return out };
			let mut px = bc1_block(if fmt == Fmt::Bc1 { b } else { &b[8..] }, fmt == Fmt::Bc1);
			match fmt {
				Fmt::Bc2 => {
					for (i, p) in px.iter_mut().enumerate() {
						p[3] = (b[i / 2] >> (4 * (i & 1)) & 15) * 17;
					}
				}
				Fmt::Bc3 => {
					let (t, bits) = (bc3_table(b[0], b[1]), bc3_bits(b));
					for (i, p) in px.iter_mut().enumerate() {
						p[3] = t[(bits >> (3 * i)) as usize & 7];
					}
				}
				_ => {}
			}
			for (i, p) in px.iter().enumerate() {
				let (x, y) = (bx * 4 + i % 4, by * 4 + i / 4);
				if x < w as usize && y < h as usize {
					out[(y * w as usize + x) * 4..][..4].copy_from_slice(p);
				}
			}
		}
	}
	out
}

/// BC3 data with every block's alpha re-coded to `min(2 a, 255)`: the 16 doubled values get the endpoints (max, min) and the nearest
/// of the 8 levels (a constant block becomes two equal endpoints, which is exact). Colour blocks are copied.
pub fn alpha_x2_bc3(data: &[u8]) -> Vec<u8> {
	let mut out = data.to_vec();
	for b in out.chunks_exact_mut(16) {
		let (t, bits) = (bc3_table(b[0], b[1]), bc3_bits(b));
		let d: [u8; 16] = std::array::from_fn(|i| t[(bits >> (3 * i)) as usize & 7].saturating_mul(2));
		let (lo, hi) = (*d.iter().min().unwrap(), *d.iter().max().unwrap());
		if lo == hi {
			b[..8].copy_from_slice(&[hi, hi, 0, 0, 0, 0, 0, 0]);
			continue;
		}
		let nt = bc3_table(hi, lo);
		let mut nb = 0u64;
		for (i, &v) in d.iter().enumerate() {
			let best = (0..8).min_by_key(|&j| (nt[j] as i32 - v as i32).abs()).unwrap();
			nb |= (best as u64) << (3 * i);
		}
		b[0] = hi;
		b[1] = lo;
		b[2..8].copy_from_slice(&nb.to_le_bytes()[..6]);
	}
	out
}

/// BC2 data with every 4-bit alpha doubled (`min(2 n, 15)`).
pub fn alpha_x2_bc2(data: &[u8]) -> Vec<u8> {
	let mut out = data.to_vec();
	for b in out.chunks_exact_mut(16) {
		for v in &mut b[..8] {
			let dbl = |n: u8| (n * 2).min(15);
			*v = dbl(*v >> 4) << 4 | dbl(*v & 15);
		}
	}
	out
}

#[cfg(test)]
mod tests {
	use super::super::dds::Fmt;
	use super::*;

	#[test]
	fn bc1_four_and_three_colour_modes() {
		// c0 = pure red (0xf800) > c1 = pure blue (0x001f): 4 colours; indices 0,1,2,3 in the first row.
		let blk = [0x00, 0xf8, 0x1f, 0x00, 0b11_10_01_00, 0, 0, 0];
		let px = bc1_block(&blk, true);
		assert_eq!((px[0], px[1]), ([255, 0, 0, 255], [0, 0, 255, 255]));
		assert_eq!((px[2], px[3]), ([170, 0, 85, 255], [85, 0, 170, 255]));
		// c0 < c1: index 3 is transparent, index 2 the midpoint.
		let blk = [0x1f, 0x00, 0x00, 0xf8, 0b11_10_00_00, 0, 0, 0];
		let px = bc1_block(&blk, true);
		assert_eq!((px[2], px[3]), ([127, 0, 127, 255], [0, 0, 0, 0]));
	}

	#[test]
	fn bc3_alpha_recode() {
		// Opaque at the PS2 scale (128 / 128) and a 128 -> 0 ramp (mode a0 > a1).
		let mut b = [0u8; 16];
		b[0] = 128;
		b[1] = 128;
		assert_eq!(&alpha_x2_bc3(&b)[..2], &[255, 255]);
		b[0] = 128;
		b[1] = 0;
		// texel 0 -> index 0 (128), texel 1 -> index 1 (0), texel 2 -> index 2 (6/7 of 128 = 109).
		let bits: u64 = 0 | 1 << 3 | 2 << 6;
		b[2..8].copy_from_slice(&bits.to_le_bytes()[..6]);
		let out = alpha_x2_bc3(&b);
		let (t, nb) = (bc3_table(out[0], out[1]), bc3_bits(&out));
		let a = |i: usize| t[(nb >> (3 * i)) as usize & 7];
		assert_eq!((a(0), a(1)), (255, 0));
		assert!((a(2) as i32 - 218).abs() <= 20, "{}", a(2));
	}

	#[test]
	fn bc2_alpha_doubles() {
		let mut b = [0u8; 16];
		b[0] = 0x84; // texels 0 and 1: 4 and 8
		b[1] = 0x31; // 1 and 3
		let o = alpha_x2_bc2(&b);
		assert_eq!((o[0], o[1]), (0xf8, 0x62));
	}

	#[test]
	fn decode_cuts_to_size() {
		// A 2x2 BC3 level is one block; alpha 255 everywhere (a0 = a1 = 255), colour red.
		let mut b = [0u8; 16];
		b[0] = 255;
		b[1] = 255;
		b[8..10].copy_from_slice(&[0x00, 0xf8]);
		b[10..12].copy_from_slice(&[0x00, 0xf8]);
		let px = decode(Fmt::Bc3, 2, 2, &b);
		assert_eq!(px, [255, 0, 0, 255].repeat(4));
	}
}
