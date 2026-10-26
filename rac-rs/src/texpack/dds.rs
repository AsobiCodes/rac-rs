//! DDS replacement textures (PCSX2's `DDSLoader`): BC1 / BC2 / BC3 by FourCC or DX10 header, BC7 (DX10, opaque files only: there is no
//! CPU decoder, so a BC7 file whose alpha matters is refused), and uncompressed 32 / 24 bit RGB(A) by channel masks. Mip levels
//! stored in the file are kept. Alpha: a file's alpha is on the PS2 scale (0x80 = opaque, `tfx.glsl`), the engine's on
//! `min(a * 2, 255)` (`Texture::new`); `load` converts, for BC2 / BC3 by re-coding the alpha blocks (`bc`), so the colour data stays
//! compressed and no shader changes.

use super::bc;

/// Pixel formats of a loaded replacement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fmt {
	/// RGBA bytes.
	Rgba8,
	Bc1,
	Bc2,
	Bc3,
	Bc7,
}

impl Fmt {
	/// Bytes of one 4x4 block (compressed formats).
	pub fn block_bytes(self) -> usize {
		if self == Fmt::Bc1 { 8 } else { 16 }
	}

	/// Bytes of a `w` x `h` level.
	pub fn level_bytes(self, w: u32, h: u32) -> usize {
		match self {
			Fmt::Rgba8 => w as usize * h as usize * 4,
			_ => w.div_ceil(4) as usize * h.div_ceil(4) as usize * self.block_bytes(),
		}
	}

	pub fn compressed(self) -> bool {
		self != Fmt::Rgba8
	}
}

/// A decoded file: `levels[0]` is `w` x `h`, every next level half of that (at least 1).
pub struct Image {
	pub fmt: Fmt,
	pub w: u32,
	pub h: u32,
	pub levels: Vec<Vec<u8>>,
}

/// PS2 alpha (0x80 = opaque) to the engine's scale.
pub fn engine_alpha(a: u8) -> u8 {
	a.saturating_mul(2)
}

fn u32_at(b: &[u8], at: usize) -> u32 {
	u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

/// How the pixel data is stored.
enum Layout {
	Block(Fmt),
	/// Uncompressed: bytes per pixel and the (r, g, b, a) byte positions (a: None = no alpha channel).
	Raw { bpp: usize, pos: [usize; 3], a: Option<usize> },
}

fn mask_pos(mask: u32) -> Result<usize, String> {
	let s = mask.trailing_zeros() as usize;
	if mask == 0xffu32 << s && s % 8 == 0 { Ok(s / 8) } else { Err(format!("dds: channel mask {mask:#x} is not one byte")) }
}

fn dx10_layout(file: &[u8]) -> Result<Layout, String> {
	if file.len() < 148 {
		return Err("dds: DX10 header ends early".into());
	}
	if u32_at(file, 140) > 1 {
		return Err("dds: texture arrays are not supported".into());
	}
	Ok(match u32_at(file, 128) {
		71 | 72 => Layout::Block(Fmt::Bc1),
		74 | 75 => Layout::Block(Fmt::Bc2),
		77 | 78 => Layout::Block(Fmt::Bc3),
		98 | 99 if u32_at(file, 144) & 7 == 3 => Layout::Block(Fmt::Bc7),
		98 | 99 => return Err("dds: BC7 with alpha is not supported (no CPU decoder)".into()),
		28 | 29 => Layout::Raw { bpp: 4, pos: [0, 1, 2], a: Some(3) },
		87 | 91 => Layout::Raw { bpp: 4, pos: [2, 1, 0], a: Some(3) },
		88 => Layout::Raw { bpp: 4, pos: [2, 1, 0], a: None },
		f => return Err(format!("dds: DXGI format {f} is not supported")),
	})
}

fn layout(file: &[u8]) -> Result<(Layout, usize), String> {
	let (flags, four, bits) = (u32_at(file, 80), &file[84..88], u32_at(file, 88) as usize);
	if flags & 4 != 0 {
		return match four {
			b"DXT1" => Ok((Layout::Block(Fmt::Bc1), 128)),
			b"DXT2" | b"DXT3" => Ok((Layout::Block(Fmt::Bc2), 128)),
			b"DXT4" | b"DXT5" => Ok((Layout::Block(Fmt::Bc3), 128)),
			b"DX10" => Ok((dx10_layout(file)?, 148)),
			f => Err(format!("dds: FourCC {:?} is not supported", String::from_utf8_lossy(f))),
		};
	}
	if flags & 0x40 == 0 || !matches!(bits, 24 | 32) {
		return Err(format!("dds: pixel format flags {flags:#x} / {bits} bit is not supported"));
	}
	let pos = [mask_pos(u32_at(file, 92))?, mask_pos(u32_at(file, 96))?, mask_pos(u32_at(file, 100))?];
	let a = if flags & 1 != 0 { Some(mask_pos(u32_at(file, 104))?) } else { None };
	Ok((Layout::Raw { bpp: bits / 8, pos, a }, 128))
}

fn to_rgba(raw: &[u8], bpp: usize, pos: [usize; 3], a: Option<usize>) -> Vec<u8> {
	let mut out = Vec::with_capacity(raw.len() / bpp * 4);
	for p in raw.chunks_exact(bpp) {
		out.extend_from_slice(&[p[pos[0]], p[pos[1]], p[pos[2]], a.map_or(255, |i| engine_alpha(p[i]))]);
	}
	out
}

/// Parses a DDS file.
pub fn load(file: &[u8]) -> Result<Image, String> {
	if file.len() < 128 || &file[..4] != b"DDS " || u32_at(file, 4) != 124 {
		return Err("dds: not a DDS file".into());
	}
	let (h, w, mips) = (u32_at(file, 12), u32_at(file, 16), if u32_at(file, 8) & 0x20000 != 0 { u32_at(file, 28).max(1) } else { 1 });
	if w == 0 || h == 0 || w > 8192 || h > 8192 || u32_at(file, 24) > 1 && u32_at(file, 112) & 0x200000 != 0 {
		return Err(format!("dds: unsupported size {w}x{h}"));
	}
	let (lay, mut at) = layout(file)?;
	let fmt = match lay {
		Layout::Block(f) => f,
		Layout::Raw { .. } => Fmt::Rgba8,
	};
	let mut levels = Vec::new();
	for k in 0..mips.min(16) {
		let (lw, lh) = ((w >> k).max(1), (h >> k).max(1));
		let size = match lay {
			Layout::Block(f) => f.level_bytes(lw, lh),
			Layout::Raw { bpp, .. } => lw as usize * lh as usize * bpp,
		};
		let Some(data) = file.get(at..at + size) else {
			if k == 0 {
				return Err("dds: pixel data ends early".into());
			}
			break;
		};
		at += size;
		levels.push(match lay {
			Layout::Raw { bpp, pos, a } => to_rgba(data, bpp, pos, a),
			Layout::Block(Fmt::Bc2) => bc::alpha_x2_bc2(data),
			Layout::Block(Fmt::Bc3) => bc::alpha_x2_bc3(data),
			Layout::Block(_) => data.to_vec(),
		});
	}
	Ok(Image { fmt, w, h, levels })
}

#[cfg(test)]
pub(super) mod tests {
	use super::*;

	/// A DDS file: `four` = FourCC (or "" for uncompressed with `masks`), `levels` the raw level data.
	pub(crate) fn dds(w: u32, h: u32, four: &[u8; 4], masks: [u32; 4], bits: u32, levels: &[&[u8]]) -> Vec<u8> {
		let mut f = vec![0u8; 128];
		f[..4].copy_from_slice(b"DDS ");
		let put = |f: &mut Vec<u8>, at: usize, v: u32| f[at..at + 4].copy_from_slice(&v.to_le_bytes());
		put(&mut f, 4, 124);
		put(&mut f, 8, 0x1007 | if levels.len() > 1 { 0x20000 } else { 0 });
		put(&mut f, 12, h);
		put(&mut f, 16, w);
		put(&mut f, 28, levels.len() as u32);
		put(&mut f, 76, 32);
		if four[0] != 0 {
			put(&mut f, 80, 4);
			f[84..88].copy_from_slice(four);
		} else {
			put(&mut f, 80, 0x40 | if masks[3] != 0 { 1 } else { 0 });
			put(&mut f, 88, bits);
			for (i, m) in masks.iter().enumerate() {
				put(&mut f, 92 + 4 * i, *m);
			}
		}
		for l in levels {
			f.extend_from_slice(l);
		}
		f
	}

	#[test]
	fn bc3_alpha_is_rescaled_and_size_checked() {
		// One block: colour endpoints white, alpha endpoints 128 / 128 (PS2 opaque) -> 255.
		let mut blk = [0u8; 16];
		blk[0] = 128;
		blk[1] = 128;
		blk[8..12].copy_from_slice(&[0xff, 0xff, 0xff, 0xff]);
		let f = dds(4, 4, b"DXT5", [0; 4], 0, &[&blk]);
		let i = load(&f).unwrap();
		assert_eq!((i.fmt, i.w, i.h, i.levels.len()), (Fmt::Bc3, 4, 4, 1));
		assert_eq!(&i.levels[0][..2], &[255, 255]);
		assert!(load(&f[..140]).is_err(), "pixel data ends early");
		assert!(load(b"DDS").is_err());
	}

	#[test]
	fn uncompressed_bgra_and_bgr() {
		// A8R8G8B8: one pixel, bytes B G R A in the file.
		let f = dds(1, 1, &[0; 4], [0xff0000, 0xff00, 0xff, 0xff000000], 32, &[&[10, 20, 30, 64]]);
		let i = load(&f).unwrap();
		assert_eq!((i.fmt, &i.levels[0][..]), (Fmt::Rgba8, &[30, 20, 10, 128][..]));
		let f = dds(1, 1, &[0; 4], [0xff0000, 0xff00, 0xff, 0], 24, &[&[1, 2, 3]]);
		assert_eq!(load(&f).unwrap().levels[0], vec![3, 2, 1, 255], "no alpha channel = opaque");
	}

	#[test]
	fn file_mips_are_kept() {
		let f = dds(8, 8, b"DXT1", [0; 4], 0, &[&[0u8; 32], &[0u8; 8], &[0u8; 8]]);
		let i = load(&f).unwrap();
		assert_eq!(i.levels.iter().map(|l| l.len()).collect::<Vec<_>>(), vec![32, 8, 8]);
	}

	#[test]
	fn bc7_needs_the_opaque_alpha_mode() {
		let mut f = dds(4, 4, b"DX10", [0; 4], 0, &[&[0u8; 20 + 16]]);
		f[128..132].copy_from_slice(&98u32.to_le_bytes());
		f[140..144].copy_from_slice(&1u32.to_le_bytes());
		assert!(load(&f).is_err());
		f[144..148].copy_from_slice(&3u32.to_le_bytes());
		assert_eq!(load(&f).unwrap().fmt, Fmt::Bc7);
	}
}
