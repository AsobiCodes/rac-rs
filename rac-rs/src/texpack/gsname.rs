//! PCSX2 texture names computed from GS local memory, for every texture format a draw can use (DESIGN.md 1.1 to 1.4): the TEX0
//! hash (raw 256-byte blocks for CT32 / T8 / T4, the expanded rows for CT24 / CT16 / T8H / T4HL / T4HH, for textures smaller than a
//! block and for region textures), the CLUT hash (CSM1 and CSM2, CT32 / CT24 / CT16), the clamp region and the `bits` field with
//! TEXA. Pure functions on the 4 MB of local memory plus the TEX0 / CLAMP / TEXA / TEXCLUT register values of one draw; the
//! hook sites (retail/gscap/texname.rs, gsfull/texname.rs) and the collector (discover.rs) call them.
//! Not modelled (PCSX2 decides it from the draw, not from registers): the region that `CreateSource` shrinks to a texture upload of
//! the last 10 draws, `UserHacks_EstimateTextureRegion` (off by default) and the mip chain hash (see `Name::mxl`).

use super::name::TexName;
use crate::gsfull::gs_mem::{a16, a32, a4, a8, luts, Luts};
use xxhash_rust::xxh3::xxh3_64;

/// The fields of a TEX0 register this module reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Tex {
	pub tbp: u32,
	pub tbw: u32,
	pub psm: u32,
	pub tw: u32,
	pub th: u32,
	pub cbp: u32,
	pub cpsm: u32,
	pub csm: u32,
	pub csa: u32,
}

impl Tex {
	pub fn from_tex0(t: u64) -> Tex {
		Tex {
			tbp: (t & 0x3fff) as u32,
			tbw: ((t >> 14) & 0x3f) as u32,
			psm: ((t >> 20) & 0x3f) as u32,
			tw: ((t >> 26) & 0xf) as u32,
			th: ((t >> 30) & 0xf) as u32,
			cbp: ((t >> 37) & 0x3fff) as u32,
			cpsm: ((t >> 51) & 0xf) as u32,
			csm: ((t >> 55) & 1) as u32,
			csa: ((t >> 56) & 0x1f) as u32,
		}
	}
}

/// PCSX2's `SourceRegion` (X and Y ranges; a side is "set" when its u16 pair is not 0, 0).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Region {
	pub minx: i32,
	pub maxx: i32,
	pub miny: i32,
	pub maxy: i32,
}

impl Region {
	pub fn has_x(&self) -> bool {
		self.minx as u16 != 0 || self.maxx as u16 != 0
	}
	pub fn has_y(&self) -> bool {
		self.miny as u16 != 0 || self.maxy as u16 != 0
	}
	/// `SourceRegion::Create(TEX0, CLAMP)`.
	pub fn create(t: &Tex, clamp: u64) -> Region {
		let (wms, wmt) = (clamp & 3, (clamp >> 2) & 3);
		let (minu, maxu) = (((clamp >> 4) & 0x3ff) as i32, ((clamp >> 14) & 0x3ff) as i32);
		let (minv, maxv) = (((clamp >> 24) & 0x3ff) as i32, ((clamp >> 34) & 0x3ff) as i32);
		let (sw, sh) = (1i32 << t.tw.min(10), 1i32 << t.th.min(10));
		let mut r = Region::default();
		if wms == 2 && maxu >= minu {
			if maxu - minu + 1 < sw || maxu >= sw {
				(r.minx, r.maxx) = (minu, maxu + 1);
			}
		} else if wms == 3 && minu != 0 {
			let rw = ((minu | maxu) - maxu) + 1;
			if rw < sw || (maxu != 0 && rw <= sw) {
				(r.minx, r.maxx) = (maxu, (minu | maxu) + 1);
			}
		}
		if wmt == 2 && maxv >= minv {
			if maxv - minv + 1 < sh || maxv >= sh {
				(r.miny, r.maxy) = (minv, maxv + 1);
			}
		} else if wmt == 3 && minv != 0 {
			let rh = ((minv | maxv) - maxv) + 1;
			if rh < sh || (maxv != 0 && rh <= sh) {
				(r.miny, r.maxy) = (maxv, (minv | maxv) + 1);
			}
		}
		r
	}
}

/// What PCSX2 calls a texture name plus what the hook logs next to it.
#[derive(Clone, Debug)]
pub struct Name {
	pub name: TexName,
	pub psm: u32,
	/// Size in pixels of the hashed rectangle (the region when there is one).
	pub w: u32,
	pub h: u32,
	/// The CLUT part is in the name (T8, T4, T8H, T4HL, T4HH).
	pub palette: bool,
	/// True when the hash took the raw GS block path (CT32, T8, T4 without a region and at least one block big).
	pub raw: bool,
	/// 8 x 8 grid of mean RGB (192 bytes) of the hashed rectangle, when asked for.
	pub sig: Option<Vec<u8>>,
}

/// PCSX2 `psm.bs`, block size in pixels (0 for formats that are not textures).
pub fn block_size(psm: u32) -> (u32, u32) {
	match psm {
		0x00 | 0x01 | 0x1b | 0x24 | 0x2c => (8, 8),
		0x02 | 0x0a => (16, 8),
		0x13 => (16, 16),
		0x14 => (32, 16),
		_ => (0, 0),
	}
}

pub fn has_palette(psm: u32) -> bool {
	super::name::psm_has_palette(psm)
}

/// `(TA0, AEM, TA1)` of a TEXA register value.
pub fn texa_parts(texa: u64) -> (u32, u32, u32) {
	((texa & 0xff) as u32, ((texa >> 15) & 1) as u32, ((texa >> 32) & 0xff) as u32)
}

/// Byte address of the first byte of block (`bx`, `by`) of the texture (blocks of `block_size` pixels).
fn block_addr(l: &Luts, t: &Tex, bx: u32, by: u32) -> usize {
	let (bw, bh) = block_size(t.psm);
	let (x, y) = (bx * bw, by * bh);
	let a = match t.psm {
		0x00 | 0x01 => a32(l, t.tbp, t.tbw, x, y),
		0x02 | 0x0a => a16(l, t.tbp, t.tbw, x, y),
		0x13 => a8(l, t.tbp, t.tbw, x, y),
		_ => a4(l, t.tbp, t.tbw, x, y) >> 1,
	};
	a & !255
}

fn rd32(v: &[u8], a: usize) -> u32 {
	u32::from_le_bytes([v[a], v[a + 1], v[a + 2], v[a + 3]])
}

fn rd16(v: &[u8], a: usize) -> u32 {
	u16::from_le_bytes([v[a], v[a + 1]]) as u32
}

/// `Expand16To32`: PS2 16-bit colour to RGBA with the TEXA alpha rule.
pub fn expand16(c: u32, texa: u64) -> u32 {
	let (ta0, aem, ta1) = texa_parts(texa);
	let a = if c & 0x8000 != 0 {
		ta1
	} else if aem == 0 || c != 0 {
		ta0
	} else {
		0
	};
	(a << 24) | ((c & 0x7c00) << 9) | ((c & 0x03e0) << 6) | ((c & 0x001f) << 3)
}

/// `Expand24To32`.
pub fn expand24(c: u32, texa: u64) -> u32 {
	let (ta0, aem, _) = texa_parts(texa);
	let c = c & 0xff_ffff;
	((if aem == 0 || c != 0 { ta0 } else { 0 }) << 24) | c
}

/// One pixel of the texture as `rtx` / `rtxP` give it: the palette index for palette formats, the expanded RGBA word otherwise.
fn texel(v: &[u8], l: &Luts, t: &Tex, x: u32, y: u32, texa: u64) -> u32 {
	match t.psm {
		0x00 => rd32(v, a32(l, t.tbp, t.tbw, x, y)),
		0x01 => expand24(rd32(v, a32(l, t.tbp, t.tbw, x, y)), texa),
		0x02 | 0x0a => expand16(rd16(v, a16(l, t.tbp, t.tbw, x, y)), texa),
		0x13 => v[a8(l, t.tbp, t.tbw, x, y)] as u32,
		0x14 => {
			let n = a4(l, t.tbp, t.tbw, x, y);
			(v[n >> 1] >> ((n & 1) * 4)) as u32 & 0xf
		}
		0x1b => v[a32(l, t.tbp, t.tbw, x, y) + 3] as u32,
		0x24 => v[a32(l, t.tbp, t.tbw, x, y) + 3] as u32 & 0xf,
		_ => (v[a32(l, t.tbp, t.tbw, x, y) + 3] >> 4) as u32,
	}
}

/// The CLUT PCSX2 holds for this TEX0 (`m_buff32`): 256 entries for T8 / T8H, 16 for the 4-bit formats, as RGBA words.
pub fn clut_entries(v: &[u8], t: &Tex, texa: u64, texclut: u64) -> Option<Vec<u32>> {
	let n: u32 = match t.psm {
		0x13 | 0x1b => 256,
		0x14 | 0x24 | 0x2c => 16,
		_ => return None,
	};
	let l = luts();
	let ct16 = matches!(t.cpsm, 2 | 10);
	let (cbw, cou, cov) = ((texclut & 0x3f) as u32, ((texclut >> 6) & 0x3f) as u32, ((texclut >> 12) & 0x7ff) as u32);
	Some(
		(0..n)
			.map(|i| {
				let (bw, x, y) = if t.csm == 0 {
					let p = if n == 256 { (i & 0xe7) | ((i & 8) << 1) | ((i & 0x10) >> 1) } else { i };
					(1, if n == 256 { p % 16 } else { p % 8 }, if n == 256 { p / 16 } else { p / 8 })
				} else {
					(cbw, cou * 16 + i, cov)
				};
				if ct16 {
					expand16(rd16(v, a16(l, t.cbp, bw, x, y)), texa)
				} else {
					rd32(v, a32(l, t.cbp, bw, x, y))
				}
			})
			.collect(),
	)
}

/// `PaletteKeyHash`: XXH3_64 over the entries as little-endian words.
pub fn clut_hash(e: &[u32]) -> u64 {
	let mut b = Vec::with_capacity(e.len() * 4);
	for w in e {
		b.extend_from_slice(&w.to_le_bytes());
	}
	xxh3_64(&b)
}

/// The rectangle `(left, top, right, bottom)` the hash covers and its size, `SourceRegion::GetRect`.
fn rect_of(t: &Tex, r: &Region) -> ((i32, i32, i32, i32), i32, i32) {
	let (sw, sh) = (1i32 << t.tw.min(10), 1i32 << t.th.min(10));
	let (tw, th) = (if r.has_x() { r.maxx - r.minx } else { sw }, if r.has_y() { r.maxy - r.miny } else { sh });
	((if r.has_x() { r.minx } else { 0 }, if r.has_y() { r.miny } else { 0 }, if r.has_x() { r.maxx } else { tw }, if r.has_y() { r.maxy } else { th }), tw, th)
}

/// The raw GS block stream PCSX2 hashes for CT32 / T8 / T4 without a region and at least one block big: the 256 bytes of every block of
/// the rectangle aligned outwards, block rows top to bottom. None when the expanded path applies.
pub fn raw_stream(v: &[u8], t: &Tex, r: &Region) -> Option<Vec<u8>> {
	let l = luts();
	let (bx, by) = block_size(t.psm);
	let ((rl, rt, rr, rb), tw, th) = rect_of(t, r);
	let fmsk_full = matches!(t.psm, 0x00 | 0x13 | 0x14);
	if !(tw >= bx as i32 && th >= by as i32 && fmsk_full && r.maxx <= 0 && r.miny <= 0) {
		return None;
	}
	let (bxs, bys) = (bx as i32, by as i32);
	let (c0, c1, r0, r1) = (rl.div_euclid(bxs), (rr + bxs - 1).div_euclid(bxs), rt.div_euclid(bys), (rb + bys - 1).div_euclid(bys));
	let mut s = Vec::with_capacity(((c1 - c0) * (r1 - r0) * 256).max(0) as usize);
	for y in r0..r1 {
		for x in c0..c1 {
			let a = block_addr(l, t, x as u32, y as u32);
			s.extend_from_slice(&v[a..a + 256]);
		}
	}
	Some(s)
}

/// `HashTextureLevel` for one level: the XXH3_64 of the byte stream PCSX2 feeds (raw GS blocks or expanded rows).
/// Returns the hash and whether the raw block path was taken.
pub fn tex_hash(v: &[u8], t: &Tex, r: &Region, texa: u64) -> (u64, bool) {
	if let Some(s) = raw_stream(v, t, r) {
		return (xxh3_64(&s), true);
	}
	let l = luts();
	let (bx, _) = block_size(t.psm);
	let ((rl, rt, rr, _), tw, th) = rect_of(t, r);
	// Expanded path: one byte per pixel for palette formats, one RGBA word otherwise, the rows of the rectangle. PCSX2 reads the
	// block-aligned rectangle into a buffer and starts each row `rect.left - block_rect.left` BYTES in (not pixels): same here.
	let pal = has_palette(t.psm);
	let bxs = bx.max(1) as i32;
	let (left, right) = (rl.div_euclid(bxs) * bxs, (rr + bxs - 1).div_euclid(bxs) * bxs);
	let dx = (rl - left) as usize;
	let row_size = (tw as usize) << if pal { 0 } else { 2 };
	let mut s = Vec::with_capacity(row_size * th as usize);
	let mut row: Vec<u8> = Vec::new();
	for y in 0..th {
		row.clear();
		for x in left..right {
			let p = texel(v, l, t, x as u32, (rt + y) as u32, texa);
			if pal {
				row.push(p as u8);
			} else {
				row.extend_from_slice(&p.to_le_bytes());
			}
		}
		let end = dx + row_size;
		if row.len() < end {
			row.resize(end, 0);
		}
		s.extend_from_slice(&row[dx..end]);
	}
	(xxh3_64(&s), false)
}

/// The 8 x 8 mean-colour grid of the hashed rectangle (192 bytes), for pairing textures of two game versions by look.
fn signature(v: &[u8], t: &Tex, r: &Region, texa: u64, clut: &Option<Vec<u32>>) -> Vec<u8> {
	let l = luts();
	let ((rl, rt, _, _), tw, th) = rect_of(t, r);
	let mut out = vec![0u8; 192];
	for cy in 0..8i32 {
		for cx in 0..8i32 {
			let (x0, y0) = (cx * tw / 8, cy * th / 8);
			let (x1, y1) = (((cx + 1) * tw / 8).max(x0 + 1).min(tw), ((cy + 1) * th / 8).max(y0 + 1).min(th));
			let (mut sum, mut n) = ([0u32; 3], 0u32);
			for y in y0..y1 {
				for x in x0..x1 {
					let p = texel(v, l, t, (rl + x) as u32, (rt + y) as u32, texa);
					let c = clut.as_ref().map_or(p, |c| c[p as usize & (c.len() - 1)]);
					for (k, s) in sum.iter_mut().enumerate() {
						*s += (c >> (8 * k)) & 255;
					}
					n += 1;
				}
			}
			for k in 0..3 {
				out[(cy * 8 + cx) as usize * 3 + k] = (sum[k] / n.max(1)) as u8;
			}
		}
	}
	out
}

/// `GSState::GetTex0Layer`: the texture of mip level `k` (1..=6): base and width from MIPTBP1 (levels 1..3) or MIPTBP2 (4..6), 20 bits each
/// (TBP 14, TBW 6), the size shrunk by `k` (not below 1).
pub fn mip_tex(t: &Tex, k: u32, mip1: u64, mip2: u64) -> Tex {
	let (reg, i) = if k <= 3 { (mip1, k - 1) } else { (mip2, k - 4) };
	let f = (reg >> (i * 20)) & 0xf_ffff;
	Tex { tbp: (f & 0x3fff) as u32, tbw: ((f >> 14) & 0x3f) as u32, tw: t.tw.saturating_sub(k), th: t.th.saturating_sub(k), ..*t }
}

/// The names of mip levels 1..=`mxl` of the draw's texture (single-level hashes at the MIPTBP bases, no region), as dump file stems.
pub fn mip_stems(v: &[u8], tex0: u64, texa: u64, texclut: u64, mxl: u32, mip1: u64, mip2: u64) -> Vec<String> {
	let t = Tex::from_tex0(tex0);
	(1..=mxl.min(6))
		.filter_map(|k| {
			let m = mip_tex(&t, k, mip1, mip2);
			let t0 = (tex0 & !0x3_ffff_ffff) | u64::from(m.tbp) | u64::from(m.tbw) << 14 | u64::from(m.psm) << 20 | u64::from(m.tw) << 26 | u64::from(m.th) << 30;
			name_for_draw(v, t0, 0, texa, texclut, false).map(|n| n.name.stem(n.palette))
		})
		.collect()
}

/// The PCSX2 name of the texture a draw with these registers samples (level 0, single-level hash), or None for a format that is
/// no texture. `v` is the 4 MB of local memory as it is when the draw runs (uploads flushed).
pub fn name_for_draw(v: &[u8], tex0: u64, clamp: u64, texa: u64, texclut: u64, want_sig: bool) -> Option<Name> {
	let t = Tex::from_tex0(tex0);
	if block_size(t.psm).0 == 0 || v.len() < crate::gsfull::gs_mem::VRAM_SIZE {
		return None;
	}
	let region = Region::create(&t, clamp);
	let ((_, _, _, _), w, h) = rect_of(&t, &region);
	if w <= 0 || h <= 0 {
		return None;
	}
	let pal = has_palette(t.psm);
	let clut = if pal { clut_entries(v, &t, texa, texclut) } else { None };
	let (hash, raw) = tex_hash(v, &t, &region, texa);
	let mut bits = t.psm | t.tw << 6 | t.th << 10;
	if !pal && matches!(t.psm, 0x01 | 0x02 | 0x0a) {
		let (ta0, aem, ta1) = texa_parts(texa);
		bits |= ta0 << 15 | aem << 23 | ta1 << 24;
	}
	let (rw, rh) = (if region.has_x() { (region.maxx - region.minx) as u16 as u32 } else { 0 }, if region.has_y() { (region.maxy - region.miny) as u16 as u32 } else { 0 });
	let name = TexName { tex0: hash, clut: clut.as_ref().map_or(0, |c| clut_hash(c)), region_w: rw, region_h: rh, bits };
	let sig = want_sig.then(|| signature(v, &t, &region, texa, &clut));
	Some(Name { name, psm: t.psm, w: w as u32, h: h as u32, palette: pal, raw, sig })
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::gsfull::gs_mem::{wr_px, VRAM_SIZE};
	use crate::texpack::hash::{bits_t8, clut_hash as clut_hash_raw, tex_hash_single, Level};

	fn vram() -> Vec<u8> {
		vec![0u8; VRAM_SIZE]
	}

	fn put(v: &mut [u8], psm: u32, bp: u32, bw: u32, x: u32, y: u32, val: u32) {
		unsafe { wr_px(v.as_mut_ptr(), luts(), psm, bp, bw, x, y, val) }
	}

	/// TEX0 of a texture at `tbp` with a CT32 CSM1 CLUT at `cbp`.
	fn tex0(psm: u64, tbp: u64, tbw: u64, tw: u64, th: u64, cbp: u64) -> u64 {
		tbp | tbw << 14 | psm << 20 | tw << 26 | th << 30 | 1 << 34 | cbp << 37
	}

	fn pat(n: usize, mul: usize, add: usize) -> Vec<u8> {
		(0..n).map(|i| ((i * mul + add) & 255) as u8).collect()
	}

	/// Writes a T8 texture (row-major indices) and a CT32 CSM1 CLUT (logical entry i at its stored place) into local memory.
	fn put_t8(v: &mut [u8], px: &[u8], w: u32, h: u32, tbp: u32, cbp: u32, pal: &[u32]) {
		for y in 0..h {
			for x in 0..w {
				put(v, 0x13, tbp, (w / 64).max(1), x, y, px[(y * w + x) as usize] as u32);
			}
		}
		for (i, c) in pal.iter().enumerate() {
			let i = i as u32;
			let p = (i & 0xe7) | ((i & 8) << 1) | ((i & 0x10) >> 1);
			put(v, 0, cbp, 1, p % 16, p / 16, *c);
		}
	}

	#[test]
	fn t8_names_equal_the_table_names() {
		// A 64x32 level (block path), a 32x32 one and an 8x8 one (linear path): the local-memory hash equals the table hash.
		let pal: Vec<u32> = (0..256u32).map(|i| 0x8000_0000 | (i * 0x010203) & 0xff_ffff).collect();
		let raw_pal: Vec<u8> = {
			// the stored order: entry i of the logical order sits at dword j(i)
			let mut b = vec![0u8; 1024];
			for i in 0..256usize {
				let j = (i & !0x18) | ((i & 8) << 1) | ((i & 0x10) >> 1);
				b[j * 4..j * 4 + 4].copy_from_slice(&pal[i].to_le_bytes());
			}
			b
		};
		for (w, h, tbw) in [(64usize, 32usize, 1u64), (32, 32, 1), (8, 8, 1), (128, 16, 2)] {
			let px = pat(w * h, 7, 3);
			let mut v = vram();
			put_t8(&mut v, &px, w as u32, h as u32, 100, 5000, &pal);
			let tw = (w as f32).log2() as u64;
			let th = (h as f32).log2() as u64;
			let n = name_for_draw(&v, tex0(0x13, 100, tbw, tw, th, 5000), 0, 0, 0, false).unwrap();
			assert_eq!(n.name.tex0, tex_hash_single(Level { px: &px, w, h }), "{w}x{h} texture hash");
			assert_eq!(n.name.clut, clut_hash_raw(&raw_pal), "{w}x{h} clut hash");
			assert_eq!(n.name.bits, bits_t8(w, h), "{w}x{h} bits");
			assert_eq!(n.raw, w >= 16 && h >= 16);
		}
	}

	#[test]
	fn region_from_clamp_matches_the_pack_example() {
		// Pack name ...-r31002000610050: x 0x50..0x61, y 0x20..0x31 of a 256x128 texture = a 17x17 region.
		let t = Tex::from_tex0(tex0(0x13, 0, 4, 8, 7, 0));
		let clamp = 2 | 2 << 2 | 0x50 << 4 | 0x60 << 14 | 0x20u64 << 24 | 0x30u64 << 34;
		let r = Region::create(&t, clamp);
		assert_eq!((r.minx, r.maxx, r.miny, r.maxy), (0x50, 0x61, 0x20, 0x31));
		let ((l, tp, rr, b), w, h) = rect_of(&t, &r);
		assert_eq!((l, tp, rr, b, w, h), (0x50, 0x20, 0x61, 0x31, 17, 17));
		// A clamp region as big as the texture is no region.
		assert_eq!(Region::create(&t, 2 | 2 << 2 | 255 << 14 | 127u64 << 34), Region::default());
	}

	#[test]
	fn region_t8_hash_is_the_expanded_rows() {
		let (w, h) = (64usize, 64usize);
		let px = pat(w * h, 5, 9);
		let mut v = vram();
		put_t8(&mut v, &px, w as u32, h as u32, 0, 4000, &vec![0x80ff_ffff; 256]);
		let clamp = 2 | 2 << 2 | 16 << 4 | 32 << 14 | 8u64 << 24 | 19u64 << 34;
		let n = name_for_draw(&v, tex0(0x13, 0, 1, 6, 6, 4000), clamp, 0, 0, false).unwrap();
		let mut rows = Vec::new();
		for y in 8..20 {
			rows.extend_from_slice(&px[y * w + 16..y * w + 33]);
		}
		assert_eq!((n.w, n.h, n.raw), (17, 12, false));
		assert_eq!(n.name.tex0, xxh3_64(&rows));
		assert_eq!((n.name.region_w, n.name.region_h), (17, 12));
	}

	#[test]
	fn ct24_and_ct16_expand_with_texa() {
		let mut v = vram();
		for y in 0..8u32 {
			for x in 0..8u32 {
				put(&mut v, 0x01, 20, 1, x, y, (y * 8 + x) * 0x010101 & 0xff_ffff);
			}
		}
		let texa = 0x80 | 1 << 15 | 0x7fu64 << 32;
		let n = name_for_draw(&v, tex0(0x01, 20, 1, 3, 3, 0), 0, texa, 0, false).unwrap();
		let mut want = Vec::new();
		for i in 0..64u32 {
			want.extend_from_slice(&(((if i != 0 { 0x80 } else { 0 }) << 24) | (i * 0x010101)).to_le_bytes());
		}
		assert_eq!(n.name.tex0, xxh3_64(&want));
		assert_eq!(n.name.bits, 0x01 | 3 << 6 | 3 << 10 | 0x80 << 15 | 1 << 23 | 0x7f << 24);
		assert_eq!(n.name.clut, 0);
		// CT16: bit 15 set takes TA1, clear takes TA0 (AEM off).
		let mut v = vram();
		for y in 0..8u32 {
			for x in 0..16u32 {
				put(&mut v, 0x02, 30, 1, x, y, (y * 16 + x) * 0x0101 & 0xffff);
			}
		}
		let n = name_for_draw(&v, tex0(0x02, 30, 1, 4, 3, 0), 0, 0x40 | 0x7fu64 << 32, 0, false).unwrap();
		let mut want = Vec::new();
		for i in 0..128u32 {
			want.extend_from_slice(&expand16((i * 0x0101) & 0xffff, 0x40 | 0x7fu64 << 32).to_le_bytes());
		}
		assert_eq!(n.name.tex0, xxh3_64(&want));
	}

	#[test]
	fn t4_block_and_t8h_index() {
		let mut v = vram();
		for y in 0..16u32 {
			for x in 0..32u32 {
				put(&mut v, 0x14, 64, 1, x, y, (x + y) & 15);
			}
		}
		for i in 0..16u32 {
			put(&mut v, 0, 900, 1, i % 8, i / 8, 0x8000_0000 + i);
		}
		let n = name_for_draw(&v, tex0(0x14, 64, 1, 5, 4, 900), 0, 0, 0, false).unwrap();
		assert!(n.raw);
		let a = block_addr(luts(), &Tex::from_tex0(tex0(0x14, 64, 1, 5, 4, 900)), 0, 0);
		assert_eq!(n.name.tex0, xxh3_64(&v[a..a + 256]));
		let e: Vec<u32> = (0..16).map(|i| 0x8000_0000 + i).collect();
		assert_eq!(n.name.clut, clut_hash(&e));
		// T8H: the index is the top byte of a CT32 word, hashed as one byte per pixel.
		let mut v = vram();
		for y in 0..8u32 {
			for x in 0..8u32 {
				put(&mut v, 0x1b, 7, 1, x, y, y * 8 + x);
			}
		}
		for i in 0..256u32 {
			let p = (i & 0xe7) | ((i & 8) << 1) | ((i & 0x10) >> 1);
			put(&mut v, 0, 800, 1, p % 16, p / 16, i);
		}
		let n = name_for_draw(&v, tex0(0x1b, 7, 1, 3, 3, 800), 0, 0, 0, true).unwrap();
		assert_eq!(n.name.tex0, xxh3_64(&(0..64u8).collect::<Vec<_>>()));
		assert_eq!(n.name.bits, 0x1b | 3 << 6 | 3 << 10);
		assert_eq!(n.sig.unwrap().len(), 192);
	}

	#[test]
	fn mip_levels_come_from_miptbp() {
		// MIPTBP1: level 1 at 200 (TBW 2), level 2 at 300 (TBW 1), level 3 at 400 (TBW 1); MIPTBP2: level 4 at 500 (TBW 1).
		let m1 = 200u64 | 2 << 14 | 300 << 20 | 1 << 34 | 400 << 40 | 1 << 54;
		let m2 = 500u64 | 1 << 14;
		let t = Tex::from_tex0(tex0(0x13, 100, 2, 7, 7, 5000));
		let (a, b, c) = (mip_tex(&t, 1, m1, m2), mip_tex(&t, 3, m1, m2), mip_tex(&t, 4, m1, m2));
		assert_eq!((a.tbp, a.tbw, a.tw, a.th, a.psm), (200, 2, 6, 6, 0x13));
		assert_eq!((b.tbp, b.tbw, b.tw), (400, 1, 4));
		assert_eq!((c.tbp, c.tbw, c.tw, c.cbp), (500, 1, 3, 5000));
		// A level named through mip_stems equals the single-level name of a texture stored at that base.
		let mut v = vram();
		let px = pat(64 * 64, 3, 7);
		put_t8(&mut v, &px, 64, 64, 200, 5000, &[0x8000_0000; 256]);
		let want = name_for_draw(&v, tex0(0x13, 200, 2, 6, 6, 5000), 0, 0, 0, false).unwrap().name.stem(true);
		assert_eq!(mip_stems(&v, tex0(0x13, 100, 2, 7, 7, 5000), 0, 0, 1, m1, m2), vec![want]);
	}
}
