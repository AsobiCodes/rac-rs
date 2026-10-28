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
		Fmt::Bc7 => super::bc7dec::decode_level(w, h, data),
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
		plain_from(self.fmt, m0.w, m0.h, &m0.data)
	}
}

/// Level 0 (`data`, `w` x `h` in `fmt`) as RGBA8 with a box-filtered chain down to 1x1.
fn plain_from(fmt: Fmt, w: u32, h: u32, data: &[u8]) -> Result<ReplTex, String> {
	let mut mips = vec![Mip { w, h, data: rgba(fmt, w, h, data)? }];
	while let Some(&Mip { w, h, .. }) = mips.last().filter(|m| m.w > 1 || m.h > 1) {
		let (nw, nh, px) = box_down(&mips[mips.len() - 1].data, w, h);
		mips.push(Mip { w: nw, h: nh, data: px });
	}
	Ok(ReplTex { fmt: Fmt::Rgba8, mips })
}

/// Whether a file of `file_levels` levels (`fmt`, `w` x `h`) is used as it is for a slot without an authored chain: it carries every level
/// down to 1x1 and a compressed level 0 is made of whole blocks. `plain_of` and `plain_bytes` share this rule.
fn file_chain_is_plain(fmt: Fmt, w: u32, h: u32, file_levels: usize) -> bool {
	(!fmt.compressed() || (w % 4 == 0 && h % 4 == 0)) && file_levels == 1 + w.max(h).ilog2() as usize
}

/// Bytes of the chain `plain_of` makes for a file (`fmt`, `w` x `h`, `file_levels` levels), before `repl::for_device` (which decodes BC on
/// a device without BC). The one sizing function for the plain chain: `LevelRepl` checks it against the budget before building one.
pub fn plain_bytes(fmt: Fmt, w: u32, h: u32, file_levels: usize) -> usize {
	let levels = 1 + w.max(h).ilog2();
	let fmt = plain_fmt(fmt, w, h, file_levels);
	(0..levels).map(|k| fmt.level_bytes((w >> k).max(1), (h >> k).max(1))).sum()
}

/// `plain_bytes` for a device: without BC (`bc` false) `repl::for_device` decodes a BC chain (BC7 included) to RGBA8, so it is sized as
/// RGBA8; the early-out that decides whether to build a plain chain never under-counts what `plain_of` + `for_device` make.
pub fn plain_bytes_on(fmt: Fmt, w: u32, h: u32, file_levels: usize, bc: bool) -> usize {
	let fmt = if bc || !plain_fmt(fmt, w, h, file_levels).compressed() { fmt } else { Fmt::Rgba8 };
	plain_bytes(fmt, w, h, file_levels)
}

/// The format of the chain `plain_of` makes for a file: its own when it carries every level (see `file_chain_is_plain`), else RGBA8.
pub fn plain_fmt(fmt: Fmt, w: u32, h: u32, file_levels: usize) -> Fmt {
	if file_chain_is_plain(fmt, w, h, file_levels) { fmt } else { Fmt::Rgba8 }
}

/// The chain of a texture slot that has no authored mip chain (the engine's `upload` makes a full chain down to 1x1 for those). A file
/// that carries every level down to 1x1 is used as it is, in its own format (BC stays BC); otherwise level 0 gets a generated RGBA8
/// chain. Takes the decoded file (its levels are moved, not copied). Runs on the loader threads (`repl::LevelRepl`): the render thread
/// only uploads.
pub fn plain_of(base: Image) -> Result<ReplTex, String> {
	if file_chain_is_plain(base.fmt, base.w, base.h, base.levels.len()) {
		let (fmt, w, h) = (base.fmt, base.w, base.h);
		let mips = base.levels.into_iter().enumerate().map(|(k, data)| Mip { w: (w >> k).max(1), h: (h >> k).max(1), data }).collect();
		return Ok(ReplTex { fmt, mips });
	}
	plain_from(base.fmt, base.w, base.h, base.levels.first().ok_or("no pixel data")?)
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
	fn a_file_with_every_level_is_the_plain_chain_as_it_is_others_get_a_generated_one() {
		let bc = |w: u32, h: u32, n: usize| Image { fmt: Fmt::Bc3, w, h, levels: (0..n).map(|k| vec![k as u8; Fmt::Bc3.level_bytes((w >> k).max(1), (h >> k).max(1))]).collect() };
		let full = plain_of(bc(8, 4, 4)).unwrap();
		assert_eq!((full.fmt, full.mips.iter().map(|m| (m.w, m.h, m.data.len())).collect::<Vec<_>>()), (Fmt::Bc3, vec![(8, 4, 32), (4, 2, 16), (2, 1, 16), (1, 1, 16)]));
		assert_eq!(full.mips[2].data[0], 2, "the file's own level, not a regenerated one");
		let short = plain_of(bc(8, 4, 2)).unwrap();
		assert_eq!((short.fmt, short.mips.len()), (Fmt::Rgba8, 4), "levels are generated, in RGBA8");
		// The sizing function the loader checks against the budget agrees with what is built.
		for (w, h, n) in [(8, 4, 4), (8, 4, 2), (8, 4, 1), (4, 4, 3), (2, 2, 1), (16, 16, 5)] {
			assert_eq!(plain_bytes(Fmt::Bc3, w, h, n), plain_of(bc(w, h, n)).unwrap().bytes(), "BC3 {w}x{h} with {n} levels");
		}
		let rgba = |w: u32, h: u32| Image { fmt: Fmt::Rgba8, w, h, levels: vec![vec![0u8; (w * h * 4) as usize]] };
		assert_eq!(plain_bytes(Fmt::Rgba8, 4, 2, 1), plain_of(rgba(4, 2)).unwrap().bytes());
		// BC7: a complete chain stays BC7, a short one is decoded (reserved blocks decode to zero) and gets a generated RGBA8 chain.
		let mut b7 = bc(4, 4, 3);
		b7.fmt = Fmt::Bc7;
		assert_eq!(plain_of(Image { fmt: Fmt::Bc7, w: 4, h: 4, levels: b7.levels.clone() }).unwrap().fmt, Fmt::Bc7);
		b7.levels.truncate(1);
		assert_eq!(plain_of(b7).unwrap().fmt, Fmt::Rgba8);
	}

	/// `plain_bytes` / `plain_bytes_on` never count less than `plain_of` + `repl::for_device` build, for BC7 as for the other formats.
	#[test]
	fn the_plain_size_covers_what_a_device_with_or_without_bc_builds() {
		for (fmt, w, h, n) in [(Fmt::Bc7, 8, 8, 4), (Fmt::Bc7, 8, 8, 2), (Fmt::Bc7, 6, 6, 3), (Fmt::Bc3, 8, 4, 4), (Fmt::Rgba8, 4, 4, 3)] {
			let levels: Vec<Vec<u8>> = (0..n).map(|k| vec![0u8; fmt.level_bytes((w >> k).max(1), (h >> k).max(1))]).collect();
			for bc in [true, false] {
				let p = plain_of(Image { fmt, w, h, levels: levels.clone() }).unwrap();
				let built = if p.fmt.compressed() && !bc { p.to_rgba8().unwrap().bytes() } else { p.bytes() };
				assert_eq!(plain_bytes_on(fmt, w, h, n, bc), built, "{fmt:?} {w}x{h} with {n} levels, device with BC: {bc}");
			}
		}
	}

	#[test]
	fn plain_chain_reaches_1x1() {
		let c = assemble(&[Some(solid(4, 2, 50))], 1).unwrap().plain().unwrap();
		assert_eq!(c.mips.iter().map(|m| (m.w, m.h)).collect::<Vec<_>>(), vec![(4, 2), (2, 1), (1, 1)]);
	}
}
