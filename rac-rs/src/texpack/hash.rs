//! PCSX2 texture hashes for PSMT8 level textures (DESIGN.md sections 1.2, 1.3): the TEX0 hash (XXH3_64 over the GS-swizzled 16x16
//! blocks, or the linear rows of a level smaller than one block), the CLUT hash (palette in logical index order) and the `bits`
//! field. Pure functions on byte slices; no game data in this file. Golden values come from the research script texhash.py.

use xxhash_rust::xxh3::xxh3_64;

/// PCSX2 GSTables `columnTable8` (GPL-3 source): byte index inside a 16x16 PSMT8 GS block of the pixel at row y, column x.
const COL8: [[u8; 16]; 16] = [
	[0, 4, 16, 20, 32, 36, 48, 52, 2, 6, 18, 22, 34, 38, 50, 54], [8, 12, 24, 28, 40, 44, 56, 60, 10, 14, 26, 30, 42, 46, 58, 62],
	[33, 37, 49, 53, 1, 5, 17, 21, 35, 39, 51, 55, 3, 7, 19, 23], [41, 45, 57, 61, 9, 13, 25, 29, 43, 47, 59, 63, 11, 15, 27, 31],
	[96, 100, 112, 116, 64, 68, 80, 84, 98, 102, 114, 118, 66, 70, 82, 86], [104, 108, 120, 124, 72, 76, 88, 92, 106, 110, 122, 126, 74, 78, 90, 94],
	[65, 69, 81, 85, 97, 101, 113, 117, 67, 71, 83, 87, 99, 103, 115, 119], [73, 77, 89, 93, 105, 109, 121, 125, 75, 79, 91, 95, 107, 111, 123, 127],
	[128, 132, 144, 148, 160, 164, 176, 180, 130, 134, 146, 150, 162, 166, 178, 182], [136, 140, 152, 156, 168, 172, 184, 188, 138, 142, 154, 158, 170, 174, 186, 190],
	[161, 165, 177, 181, 129, 133, 145, 149, 163, 167, 179, 183, 131, 135, 147, 151], [169, 173, 185, 189, 137, 141, 153, 157, 171, 175, 187, 191, 139, 143, 155, 159],
	[224, 228, 240, 244, 192, 196, 208, 212, 226, 230, 242, 246, 194, 198, 210, 214], [232, 236, 248, 252, 200, 204, 216, 220, 234, 238, 250, 254, 202, 206, 218, 222],
	[193, 197, 209, 213, 225, 229, 241, 245, 195, 199, 211, 215, 227, 231, 243, 247], [201, 205, 217, 221, 233, 237, 249, 253, 203, 207, 219, 223, 235, 239, 251, 255],
];

/// One mip level of a PSMT8 texture: its index bytes (row-major, `w * h` long) and size in pixels.
#[derive(Clone, Copy, Debug)]
pub struct Level<'a> {
	pub px: &'a [u8],
	pub w: usize,
	pub h: usize,
}

/// Appends the byte stream PCSX2's `HashTextureLevel` feeds to XXH3 for one PSMT8 level without a region: block rows top to
/// bottom, block columns left to right, each block's 256 raw GS bytes (`block[COL8[y][x]] = pixel(x, y)`) when the level is at
/// least one block (16x16) in both directions; the linear rows (the "expanded" path) otherwise.
pub fn t8_level_stream(l: Level, out: &mut Vec<u8>) {
	let n = l.w * l.h;
	let px = &l.px[..n.min(l.px.len())];
	if l.w < 16 || l.h < 16 || l.w % 16 != 0 || l.h % 16 != 0 || px.len() != n {
		out.extend_from_slice(px);
		return;
	}
	let mut block = [0u8; 256];
	for by in 0..l.h / 16 {
		for bx in 0..l.w / 16 {
			for y in 0..16 {
				let row = &px[(by * 16 + y) * l.w + bx * 16..][..16];
				for x in 0..16 {
					block[COL8[y][x] as usize] = row[x];
				}
			}
			out.extend_from_slice(&block);
		}
	}
}

/// TEX0 hash of the single level `l` (what the real packs use: every mip level has its own name).
pub fn tex_hash_single(l: Level) -> u64 {
	let mut s = Vec::with_capacity(l.w * l.h);
	t8_level_stream(l, &mut s);
	xxh3_64(&s)
}

/// TEX0 hash over the chain `levels[start..]` in one XXH3 stream (current PCSX2 with Hardware Mipmapping: the drawn level up to MXL).
pub fn tex_hash_chain(levels: &[Level], start: usize) -> u64 {
	let mut s = Vec::new();
	for l in &levels[start..] {
		t8_level_stream(*l, &mut s);
	}
	xxh3_64(&s)
}

/// CLUT hash of a 1024-byte CT32 CSM1 palette as the GS reads it: entry i of the logical order is stored at j(i) = i with bits 3
/// and 4 swapped; the four raw RGBA bytes (alpha 0x80 = opaque) are hashed as they are.
pub fn clut_hash(pal: &[u8]) -> u64 {
	let mut out = [0u8; 1024];
	for i in 0..256 {
		let j = (i & !0x18) | ((i & 8) << 1) | ((i & 0x10) >> 1);
		out[i * 4..i * 4 + 4].copy_from_slice(&pal[j * 4..j * 4 + 4]);
	}
	xxh3_64(&out)
}

/// ceil(log2(n)) for a texture side (TW / TH of the TEX0 register).
fn log2_side(n: usize) -> u32 {
	n.max(1).next_power_of_two().trailing_zeros()
}

/// The `bits` field of a PSMT8 (0x13) texture name: PSM bits 0..5, TW bits 6..9, TH bits 10..13; bit 14 (old TCC) is masked and
/// TEXA is 0 for palette formats.
pub fn bits_t8(w: usize, h: usize) -> u32 {
	0x13 | log2_side(w) << 6 | log2_side(h) << 10
}

#[cfg(test)]
mod tests {
	use super::*;

	/// `(i * mul + add) & 255`, the synthetic pattern the golden values were made with (texhash.py).
	fn pat(n: usize, mul: usize, add: usize) -> Vec<u8> {
		(0..n).map(|i| ((i * mul + add) & 255) as u8).collect()
	}

	fn single(w: usize, h: usize) -> u64 {
		let px = pat(w * h, 7, 3);
		tex_hash_single(Level { px: &px, w, h })
	}

	#[test]
	fn single_level_golden_values() {
		// 16x16 and larger: swizzled block stream; 8x8, 4x2, 1x1: linear rows.
		for (w, h, want) in [
			(16, 16, 0x2530f7e8f17c7e16u64), (32, 16, 0x27573efde2baafef), (64, 32, 0x9697b6ef995b8ba1), (8, 8, 0x287eb1fa9e4be2c1),
			(4, 2, 0x60539db630471163), (1, 1, 0x13e608bc156defed), (128, 128, 0x94e26d48cb36795e), (16, 64, 0x148e5e704629f7b1),
		] {
			assert_eq!(single(w, h), want, "{w}x{h}");
		}
	}

	#[test]
	fn clut_hash_golden() {
		assert_eq!(clut_hash(&pat(1024, 13, 5)), 0x67092c9a05cb2660);
	}

	#[test]
	fn chain_hash_golden_and_bits() {
		let (a, b, c, d) = (pat(64 * 64, 7, 3), pat(32 * 32, 5, 1), pat(16 * 16, 3, 9), pat(8 * 8, 11, 2));
		let lv = [Level { px: &a, w: 64, h: 64 }, Level { px: &b, w: 32, h: 32 }, Level { px: &c, w: 16, h: 16 }, Level { px: &d, w: 8, h: 8 }];
		for (s, want) in [0xbe7f4b07be8ae043u64, 0xe2e6f832fa1cf7f0, 0xa788e0d7d5530027, 0xc99f4334ae5d0a2c].into_iter().enumerate() {
			assert_eq!(tex_hash_chain(&lv, s), want, "chain from {s}");
		}
		assert_eq!(tex_hash_single(lv[0]), 0xfcebcc02d3f221f0);
		assert_eq!(bits_t8(64, 64), 0x1993);
		assert_eq!(bits_t8(128, 128), 0x1dd3);
		assert_eq!(bits_t8(8, 8) , 0x13 | 3 << 6 | 3 << 10);
	}

	#[test]
	fn col8_is_a_permutation() {
		let mut seen = [false; 256];
		for r in COL8 {
			for v in r {
				assert!(!std::mem::replace(&mut seen[v as usize], true));
			}
		}
	}
}
