//! A replacement texture with its mip chain, built from the pack's files. The real pack has one file per ORIGINAL mip level, each 4x
//! that level, so level k of the replacement is exactly the file named for level k of the original; a missing level is box-filtered
//! from the one above (the engine's `mip_chain` rounding), a level inside a DDS file is used when no file is named for it. The level
//! count is the original's, so the engine's level choice (`gs_level`, K / MXL) works unchanged.

use super::bc;
use super::dds::{Fmt, Image};

/// One mip level.
pub struct Mip {
	pub w: u32,
	pub h: u32,
	pub data: Vec<u8>,
}

/// A chain in one format (a GPU texture has one format for all its levels).
pub struct ReplTex {
	pub fmt: Fmt,
	pub mips: Vec<Mip>,
}

/// `px` (RGBA8, `w` x `h`) halved with a 2x2 box filter, alpha averaged too: the rounding of `viewer::targets::mip_chain`.
pub fn box_down(px: &[u8], w: u32, h: u32) -> (u32, u32, Vec<u8>) {
	let (nw, nh) = ((w / 2).max(1), (h / 2).max(1));
	let mut next = Vec::with_capacity((nw * nh * 4) as usize);
	for y in 0..nh {
		for x in 0..nw {
			let mut acc = [0u32; 4];
			for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
				let (sx, sy) = ((x * 2 + dx).min(w - 1), (y * 2 + dy).min(h - 1));
				let c = &px[((sy * w + sx) * 4) as usize..][..4];
				for i in 0..4 {
					acc[i] += c[i] as u32;
				}
			}
			next.extend(acc.map(|v| ((v + 2) / 4) as u8));
		}
	}
	(nw, nh, next)
}

fn rgba(fmt: Fmt, w: u32, h: u32, data: &[u8]) -> Result<Vec<u8>, String> {
	match fmt {
		Fmt::Rgba8 => Ok(data.to_vec()),
		Fmt::Bc7 => Err("BC7 cannot be converted (no CPU decoder)".into()),
		f => Ok(bc::decode(f, w, h, data)),
	}
}

impl ReplTex {
	/// Bytes of all levels.
	pub fn bytes(&self) -> usize {
		self.mips.iter().map(|m| m.data.len()).sum()
	}

	/// The same chain decoded to RGBA8.
	pub fn to_rgba8(&self) -> Result<ReplTex, String> {
		let mips = self.mips.iter().map(|m| Ok(Mip { w: m.w, h: m.h, data: rgba(self.fmt, m.w, m.h, &m.data)? })).collect::<Result<_, String>>()?;
		Ok(ReplTex { fmt: Fmt::Rgba8, mips })
	}

	/// Level 0 alone with a generated chain down to 1x1, RGBA8: what a texture without authored levels gets (like `mip_chain`).
	pub fn plain(&self) -> Result<ReplTex, String> {
		let m0 = &self.mips[0];
		let (mut w, mut h, mut px) = (m0.w, m0.h, rgba(self.fmt, m0.w, m0.h, &m0.data)?);
		let mut mips = vec![Mip { w, h, data: px.clone() }];
		while w > 1 || h > 1 {
			(w, h, px) = box_down(&px, w, h);
			mips.push(Mip { w, h, data: px.clone() });
		}
		Ok(ReplTex { fmt: Fmt::Rgba8, mips })
	}
}

/// Builds the chain of `n` levels from `files[k]` (the image named for level k, if any). Level 0 is required.
pub fn assemble(files: &[Option<Image>], n: usize) -> Result<ReplTex, String> {
	let base = files.first().and_then(|f| f.as_ref()).ok_or("no file for level 0")?;
	let (w0, h0) = (base.w, base.h);
	let n = n.clamp(1, 1 + w0.max(h0).ilog2() as usize);
	let mut lv: Vec<(Fmt, Mip)> = Vec::new();
	for k in 0..n {
		let (w, h) = ((w0 >> k).max(1), (h0 >> k).max(1));
		let named = files.get(k).and_then(|f| f.as_ref()).filter(|i| i.w == w && i.h == h);
		let found = named.map(|i| (i.fmt, i.levels[0].clone())).or_else(|| base.levels.get(k).map(|l| (base.fmt, l.clone())));
		let (fmt, data) = match found {
			Some(f) => f,
			None => {
				let (pf, pm) = &lv[k - 1];
				let (nw, nh, px) = box_down(&rgba(*pf, pm.w, pm.h, &pm.data)?, pm.w, pm.h);
				debug_assert_eq!((nw, nh), (w, h));
				(Fmt::Rgba8, px)
			}
		};
		lv.push((fmt, Mip { w, h, data }));
	}
	let fmt = lv[0].0;
	let uniform = lv.iter().all(|(f, _)| *f == fmt);
	let blocks_ok = !fmt.compressed() || (w0 % 4 == 0 && h0 % 4 == 0);
	if uniform && blocks_ok {
		return Ok(ReplTex { fmt, mips: lv.into_iter().map(|(_, m)| m).collect() });
	}
	let mips = lv.into_iter().map(|(f, m)| Ok(Mip { w: m.w, h: m.h, data: rgba(f, m.w, m.h, &m.data)? })).collect::<Result<_, String>>()?;
	Ok(ReplTex { fmt: Fmt::Rgba8, mips })
}

#[cfg(test)]
mod tests {
	use super::*;

	fn solid(w: u32, h: u32, v: u8) -> Image {
		Image { fmt: Fmt::Rgba8, w, h, levels: vec![vec![v; (w * h * 4) as usize]] }
	}

	#[test]
	fn missing_levels_are_filtered_from_the_one_above() {
		let c = assemble(&[Some(solid(8, 8, 100)), None, Some(solid(2, 2, 7)), None], 4).unwrap();
		assert_eq!(c.mips.iter().map(|m| (m.w, m.h)).collect::<Vec<_>>(), vec![(8, 8), (4, 4), (2, 2), (1, 1)]);
		assert_eq!(c.mips[1].data[0], 100, "level 1 = box of level 0");
		assert_eq!(c.mips[2].data[0], 7, "the named file wins");
		assert_eq!(c.mips[3].data[0], 7);
	}

	#[test]
	fn a_wrong_sized_file_is_ignored_and_level_0_is_required() {
		let c = assemble(&[Some(solid(8, 8, 1)), Some(solid(8, 8, 9))], 2).unwrap();
		assert_eq!(c.mips[1].data[0], 1);
		assert!(assemble(&[None, Some(solid(4, 4, 1))], 2).is_err());
	}

	#[test]
	fn a_full_bc_chain_stays_compressed_a_mixed_one_does_not() {
		let bc = |w: u32, h: u32| Image { fmt: Fmt::Bc3, w, h, levels: vec![vec![0u8; Fmt::Bc3.level_bytes(w, h)]] };
		let c = assemble(&[Some(bc(32, 32)), Some(bc(16, 16)), Some(bc(8, 8)), Some(bc(4, 4))], 4).unwrap();
		assert_eq!((c.fmt, c.bytes()), (Fmt::Bc3, 16 * (64 + 16 + 4 + 1)));
		let c = assemble(&[Some(bc(32, 32)), Some(bc(16, 16)), None, None], 4).unwrap();
		assert_eq!(c.fmt, Fmt::Rgba8, "generated levels are RGBA8, so the whole chain is");
		assert_eq!(c.bytes(), 4 * (32 * 32 + 16 * 16 + 8 * 8 + 4 * 4));
	}

	#[test]
	fn plain_chain_reaches_1x1() {
		let c = assemble(&[Some(solid(4, 2, 50))], 1).unwrap().plain().unwrap();
		assert_eq!(c.mips.iter().map(|m| (m.w, m.h)).collect::<Vec<_>>(), vec![(4, 2), (2, 1), (1, 1)]);
	}
}
