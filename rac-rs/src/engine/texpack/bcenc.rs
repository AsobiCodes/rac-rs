//! [PC] RGBA8 -> BC3 encoder for pack textures that would not fit the memory budget (v0.4.1, GitHub #32). A PNG pack (the second public
//! R&C1 pack, 6.4 GB) decodes to RGBA8, 4x the size of the DDS files of the first pack: 1.3 to 1.6 GB per level, so the default
//! `RAC_TEXPACK_MB` (1536) ran out part-way through a level and the remaining textures kept the original. A texture that does not fit
//! is now stored as BC3 (4x smaller, 1 byte per texel) instead of being dropped.
//!
//! Fast "range fit" encoder, one pass per block, no search: colour endpoints are the bounding box of the 16 texels pulled in by 1/16
//! (the classic inset), alpha endpoints are the min / max; indices are the nearest palette entry. It runs on the loader threads.

use super::chain::{Mip, ReplTex};
use super::dds::Fmt;

fn to565(c: [i32; 3]) -> u16 {
	let q = |v: i32, bits: u32| ((v.clamp(0, 255) * ((1 << bits) - 1) + 127) / 255) as u16;
	q(c[0], 5) << 11 | q(c[1], 6) << 5 | q(c[2], 5)
}

fn from565(c: u16) -> [i32; 3] {
	let (r, g, b) = ((c >> 11) as i32 & 31, (c >> 5) as i32 & 63, c as i32 & 31);
	[r << 3 | r >> 2, g << 2 | g >> 4, b << 3 | b >> 2]
}

/// One 4x4 block (16 RGBA texels, row-major) as 16 BC3 bytes.
fn block(px: &[[u8; 4]; 16]) -> [u8; 16] {
	let mut out = [0u8; 16];
	// Alpha: endpoints max, min (a0 > a1 selects the 8-level mode); index 0 = a0, 1 = a1, k >= 2 = (( 8 - k) a0 + (k - 1) a1) / 7.
	let (amin, amax) = px.iter().fold((255u8, 0u8), |(lo, hi), p| (lo.min(p[3]), hi.max(p[3])));
	out[0] = amax;
	out[1] = amin;
	if amax > amin {
		let t: [i32; 8] = std::array::from_fn(|i| match i {
			0 => amax as i32,
			1 => amin as i32,
			k => ((8 - k as i32) * amax as i32 + (k as i32 - 1) * amin as i32) / 7,
		});
		let mut bits = 0u64;
		for (i, p) in px.iter().enumerate() {
			let best = (0..8).min_by_key(|&j| (t[j] - p[3] as i32).abs()).unwrap_or(0);
			bits |= (best as u64) << (3 * i);
		}
		out[2..8].copy_from_slice(&bits.to_le_bytes()[..6]);
	}
	// Colour: bounding box inset by 1/16, c0 >= c1 (BC3 always decodes the 4-colour way).
	let (mut lo, mut hi) = ([255i32; 3], [0i32; 3]);
	for p in px {
		for c in 0..3 {
			lo[c] = lo[c].min(p[c] as i32);
			hi[c] = hi[c].max(p[c] as i32);
		}
	}
	let inset = |c: usize| (hi[c] - lo[c]) / 16;
	let (mut e0, mut e1) = (0u16, 0u16);
	if hi != lo {
		e0 = to565([hi[0] - inset(0), hi[1] - inset(1), hi[2] - inset(2)]);
		e1 = to565([lo[0] + inset(0), lo[1] + inset(1), lo[2] + inset(2)]);
	} else {
		e0 = to565(hi);
	}
	if e0 < e1 {
		std::mem::swap(&mut e0, &mut e1);
	}
	let (c0, c1) = (from565(e0), from565(e1));
	let pal: [[i32; 3]; 4] = [c0, c1, std::array::from_fn(|i| (2 * c0[i] + c1[i]) / 3), std::array::from_fn(|i| (c0[i] + 2 * c1[i]) / 3)];
	let mut bits = 0u32;
	if e0 != e1 {
		for (i, p) in px.iter().enumerate() {
			let d = |q: &[i32; 3]| (0..3).map(|c| (q[c] - p[c] as i32).pow(2)).sum::<i32>();
			let best = (0..4).min_by_key(|&j| d(&pal[j])).unwrap_or(0);
			bits |= (best as u32) << (2 * i);
		}
	}
	out[8..10].copy_from_slice(&e0.to_le_bytes());
	out[10..12].copy_from_slice(&e1.to_le_bytes());
	out[12..16].copy_from_slice(&bits.to_le_bytes());
	out
}

/// `px` (RGBA8, `w` x `h`) as BC3 data (rows of blocks; the edge texels repeat into a partial block).
pub fn encode_bc3(px: &[u8], w: u32, h: u32) -> Vec<u8> {
	let (bw, bh) = (w.div_ceil(4), h.div_ceil(4));
	let mut out = Vec::with_capacity((bw * bh * 16) as usize);
	for by in 0..bh {
		for bx in 0..bw {
			let t: [[u8; 4]; 16] = std::array::from_fn(|i| {
				let (x, y) = ((bx * 4 + i as u32 % 4).min(w - 1), (by * 4 + i as u32 / 4).min(h - 1));
				let o = ((y * w + x) * 4) as usize;
				[px[o], px[o + 1], px[o + 2], px[o + 3]]
			});
			out.extend_from_slice(&block(&t));
		}
	}
	out
}

/// An RGBA8 chain as BC3, level by level; None when it is not RGBA8 or level 0 is not a whole number of blocks (the GPU needs that).
pub fn compress(t: &ReplTex) -> Option<ReplTex> {
	let m0 = t.mips.first()?;
	if t.fmt != Fmt::Rgba8 || m0.w % 4 != 0 || m0.h % 4 != 0 {
		return None;
	}
	Some(ReplTex { fmt: Fmt::Bc3, mips: t.mips.iter().map(|m| Mip { w: m.w, h: m.h, data: encode_bc3(&m.data, m.w, m.h) }).collect() })
}

#[cfg(test)]
mod tests {
	use super::super::bc;
	use super::*;

	fn psnr(a: &[u8], b: &[u8]) -> f64 {
		let se: f64 = a.iter().zip(b).map(|(&x, &y)| (x as f64 - y as f64).powi(2)).sum();
		10.0 * (255.0f64 * 255.0 * a.len() as f64 / se.max(1.0)).log10()
	}

	fn picture(w: u32, h: u32) -> Vec<u8> {
		let mut v = Vec::new();
		for y in 0..h {
			for x in 0..w {
				let n = ((x * 7 + y * 13) % 11) as u8;
				v.extend_from_slice(&[(x * 255 / w) as u8 / 2 + n, (y * 255 / h) as u8 / 2 + n, ((x + y) * 255 / (w + h)) as u8, if (x / 8 + y / 8) % 2 == 0 { 255 } else { 128 + n }]);
			}
		}
		v
	}

	#[test]
	fn a_smooth_picture_survives_the_round_trip() {
		let (w, h) = (64, 32);
		let px = picture(w, h);
		let data = encode_bc3(&px, w, h);
		assert_eq!(data.len(), Fmt::Bc3.level_bytes(w, h));
		let back = bc::decode(Fmt::Bc3, w, h, &data);
		assert!(psnr(&px, &back) > 32.0, "psnr {}", psnr(&px, &back));
	}

	#[test]
	fn flat_blocks_are_exact_and_alpha_edges_stay_put() {
		let flat: Vec<u8> = (0..16).flat_map(|_| [200u8, 100, 50, 255]).collect();
		let back = bc::decode(Fmt::Bc3, 4, 4, &encode_bc3(&flat, 4, 4));
		for (a, b) in flat.chunks(4).zip(back.chunks(4)) {
			assert!(a.iter().zip(b).all(|(&x, &y)| (x as i32 - y as i32).abs() <= 8), "{a:?} {b:?}");
			assert_eq!(a[3], b[3]);
		}
		let cut: Vec<u8> = (0..16).flat_map(|i| [10u8, 20, 30, if i % 4 < 2 { 0 } else { 255 }]).collect();
		let back = bc::decode(Fmt::Bc3, 4, 4, &encode_bc3(&cut, 4, 4));
		for i in 0..16 {
			assert_eq!(back[i * 4 + 3], cut[i * 4 + 3], "texel {i}");
		}
	}

	#[test]
	fn a_chain_shrinks_to_a_quarter_and_small_levels_pad() {
		let (w, h) = (16u32, 8u32);
		let mut mips = Vec::new();
		let (mut mw, mut mh, mut px) = (w, h, picture(w, h));
		loop {
			mips.push(Mip { w: mw, h: mh, data: px.clone() });
			if mw == 1 && mh == 1 {
				break;
			}
			(mw, mh, px) = super::super::chain::box_down(&px, mw, mh);
		}
		let rgba = ReplTex { fmt: Fmt::Rgba8, mips };
		let c = compress(&rgba).unwrap();
		assert_eq!(c.fmt, Fmt::Bc3);
		assert_eq!(c.mips.len(), rgba.mips.len());
		for m in &c.mips {
			assert_eq!(m.data.len(), Fmt::Bc3.level_bytes(m.w, m.h), "{}x{}", m.w, m.h);
		}
		assert!(c.bytes() * 3 < rgba.bytes(), "{} vs {}", c.bytes(), rgba.bytes());
		let odd = ReplTex { fmt: Fmt::Rgba8, mips: vec![Mip { w: 6, h: 4, data: vec![0; 96] }] };
		assert!(compress(&odd).is_none(), "level 0 must be whole blocks");
	}
}
