//! PCSX2 replacement-texture file names (DESIGN.md 1.1): the `TexName` key, its formatting and a parser that accepts exactly the
//! formats `GSTextureReplacements::ParseReplacementName` accepts (plain, palette, region, old region), in the same order, with
//! bit 14 of `bits` cleared. Files with a `-mipN` suffix do not parse (PCSX2 never loads them: mips come from DDS levels).

/// A replacement texture's identity: hashes, optional region size and the `bits` field (PSM, TW, TH, TEXA). `clut` is 0 for
/// non-palette names. Equal names replace each other whatever the folder or extension.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TexName {
	pub tex0: u64,
	pub clut: u64,
	pub region_w: u32,
	pub region_h: u32,
	pub bits: u32,
}

/// Image file types PCSX2 loads (case-insensitive extension).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ext {
	Png,
	Dds,
}

/// Bit 14 of `bits` (the old TCC flag): present in the real pack's names, cleared by PCSX2 on load.
const UNUSED_BIT: u32 = 1 << 14;

impl TexName {
	/// A plain palette name (PSMT8, no region): `<tex0>-<clut>-<bits>`.
	pub fn palette(tex0: u64, clut: u64, bits: u32) -> TexName {
		TexName { tex0, clut, region_w: 0, region_h: 0, bits: bits & !UNUSED_BIT }
	}

	/// The file stem PCSX2 dumps this name under (`%llx-%llx-[rWxH-]%08x`); `palette` says whether the PSM has a CLUT part.
	pub fn stem(&self, palette: bool) -> String {
		let mut s = format!("{:x}", self.tex0);
		if palette {
			s += &format!("-{:x}", self.clut);
		}
		if self.region_w != 0 || self.region_h != 0 {
			s += &format!("-r{}x{}", self.region_w, self.region_h);
		}
		s + &format!("-{:08x}", self.bits)
	}

	/// PSM code (bits 0..5).
	pub fn psm(&self) -> u32 {
		self.bits & 0x3f
	}
}

/// Whether a PSM code carries a CLUT (T8 0x13, T4 0x14, T8H 0x1b, T4HL 0x24, T4HH 0x2c).
pub fn psm_has_palette(psm: u32) -> bool {
	matches!(psm, 0x13 | 0x14 | 0x1b | 0x24 | 0x2c)
}

/// A scanf-like cursor over a file name.
struct Cur<'a> {
	s: &'a [u8],
	i: usize,
}

impl Cur<'_> {
	/// `%llx`: one or more hex digits (more than 16 are not read here; scanf would wrap).
	fn hex64(&mut self) -> Option<u64> {
		self.hex(16)
	}

	/// `%08x`: one to eight hex digits (the 8 is a scanf field width).
	fn bits(&mut self) -> Option<u32> {
		self.hex(8).map(|v| v as u32)
	}

	fn hex(&mut self, max: usize) -> Option<u64> {
		let start = self.i;
		let mut v = 0u64;
		while self.i < self.s.len() && self.s[self.i].is_ascii_hexdigit() && self.i - start < max {
			v = v << 4 | (self.s[self.i] as char).to_digit(16)? as u64;
			self.i += 1;
		}
		(self.i > start).then_some(v)
	}

	/// `%u`: decimal digits.
	fn dec(&mut self) -> Option<u32> {
		let start = self.i;
		let mut v = 0u64;
		while self.i < self.s.len() && self.s[self.i].is_ascii_digit() && self.i - start < 10 {
			v = v * 10 + (self.s[self.i] - b'0') as u64;
			self.i += 1;
		}
		(self.i > start && v <= u32::MAX as u64).then_some(v as u32)
	}

	fn lit(&mut self, c: u8) -> Option<()> {
		(self.s.get(self.i) == Some(&c)).then(|| self.i += 1)
	}
}

/// Region part of a name pattern.
#[derive(Clone, Copy)]
enum Region {
	None,
	/// `rWxH` (decimal width and height).
	New,
	/// `r<hex>`: the 64-bit `SourceRegion` of older dumps.
	Old,
}

/// Width and height of an old-format region: x min/max in the low 32 bits, y min/max in the high 32 (u16 each, read as i16).
fn old_region_size(bits: u64) -> (u32, u32) {
	let f = |shift: u32| (bits >> shift) as u16 as i16 as i32;
	((f(16) - f(0)) as u32, (f(48) - f(32)) as u32)
}

/// One scanf pattern: `T[-C][-r..]-bits` followed by a '.'.
fn parse_pattern(file: &str, clut: bool, region: Region) -> Option<TexName> {
	let mut c = Cur { s: file.as_bytes(), i: 0 };
	let tex0 = c.hex64()?;
	let mut clut_hash = 0;
	if clut {
		c.lit(b'-')?;
		clut_hash = c.hex64()?;
	}
	let (mut region_w, mut region_h) = (0, 0);
	match region {
		Region::None => {}
		Region::New => {
			c.lit(b'-')?;
			c.lit(b'r')?;
			region_w = c.dec()?;
			c.lit(b'x')?;
			region_h = c.dec()?;
		}
		Region::Old => {
			c.lit(b'-')?;
			c.lit(b'r')?;
			(region_w, region_h) = old_region_size(c.hex64()?);
		}
	}
	c.lit(b'-')?;
	let bits = c.bits()?;
	c.lit(b'.')?;
	Some(TexName { tex0, clut: clut_hash, region_w, region_h, bits: bits & !UNUSED_BIT })
}

/// The loader for a file name by extension (case-insensitive, after the last dot).
pub fn ext_of(file: &str) -> Option<Ext> {
	let e = file.rsplit_once('.')?.1;
	if e.eq_ignore_ascii_case("png") {
		Some(Ext::Png)
	} else if e.eq_ignore_ascii_case("dds") {
		Some(Ext::Dds)
	} else {
		None
	}
}

/// The name and loader of a replacement file `file` (a bare file name), or None when PCSX2 would ignore it: an extension other
/// than png / dds, a name in none of the six formats (tried in PCSX2's order: region+CLUT, region, old region+CLUT, old region,
/// CLUT, plain), or a `-mipN` dump.
pub fn parse_file_name(file: &str) -> Option<(TexName, Ext)> {
	let ext = ext_of(file)?;
	const ORDER: [(bool, Region); 6] = [(true, Region::New), (false, Region::New), (true, Region::Old), (false, Region::Old), (true, Region::None), (false, Region::None)];
	ORDER.iter().find_map(|&(clut, region)| parse_pattern(file, clut, region)).map(|n| (n, ext))
}

/// For tools: the `-mipN` level of a PCSX2 dump file name (`<stem>-mip3.png` -> 3), which `parse_file_name` refuses.
pub fn mip_dump_level(file: &str) -> Option<u32> {
	let stem = file.rsplit_once('.')?.0;
	let (head, lvl) = stem.rsplit_once("-mip")?;
	let level = lvl.parse().ok()?;
	parse_file_name(&format!("{head}.png")).map(|_| level)
}

#[cfg(test)]
mod tests {
	use super::*;

	fn name(f: &str) -> TexName {
		parse_file_name(f).unwrap_or_else(|| panic!("{f} should parse")).0
	}

	// Cases are taken from the real USA pack's listing (scratch_texpack/ia_list.html); only names, no pack data.
	#[test]
	fn plain_palette_name_masks_bit_14() {
		let n = name("1a077a665f7258d-bddb5bfecca73b34-00005dd3.dds");
		assert_eq!((n.tex0, n.clut, n.region_w, n.region_h), (0x1a077a665f7258d, 0xbddb5bfecca73b34, 0, 0));
		assert_eq!(n.bits, 0x1dd3, "bit 14 cleared");
		assert_eq!(n.psm(), 0x13);
		assert_eq!(n, TexName::palette(0x1a077a665f7258d, 0xbddb5bfecca73b34, 0x5dd3));
	}

	#[test]
	fn old_region_names_give_the_region_size() {
		// 0x0031002000610050: x 0x50..0x61, y 0x20..0x31 -> 17 x 17.
		let n = name("1a7f2b6b0fd3037-5d46340bd88b677e-r31002000610050-00005e13.dds");
		assert_eq!((n.tex0, n.clut, n.region_w, n.region_h, n.bits), (0x1a7f2b6b0fd3037, 0x5d46340bd88b677e, 17, 17, 0x1e13));
		let n = name("1c75d8b383dcc2ff-5d46340bd88b677e-r51004000210010-00005e13.dds");
		assert_eq!((n.region_w, n.region_h), (0x21 - 0x10, 0x51 - 0x40));
		// Without a CLUT part.
		let n = name("1c75d8b383dcc2ff-r31002000610050-00005e13.dds");
		assert_eq!((n.clut, n.region_w, n.region_h), (0, 17, 17));
	}

	#[test]
	fn new_region_formats() {
		let n = name("abc-def-r64x32-00001993.png");
		assert_eq!((n.tex0, n.clut, n.region_w, n.region_h, n.bits), (0xabc, 0xdef, 64, 32, 0x1993));
		let n = name("abc-r64x32-00001993.PNG");
		assert_eq!((n.tex0, n.clut, n.region_w, n.region_h), (0xabc, 0, 64, 32));
	}

	#[test]
	fn no_palette_name_and_ext_rules() {
		let n = name("123456789abcdef0-00001993.dds");
		assert_eq!((n.tex0, n.clut, n.bits), (0x123456789abcdef0, 0, 0x1993));
		assert!(parse_file_name("1-2-00001993.jpg").is_none(), "only png and dds load");
		assert!(parse_file_name("1-2-00001993").is_none());
		assert_eq!(parse_file_name("1-2-00001993.DdS").unwrap().1, Ext::Dds);
		assert!(parse_file_name("1-2-0000199300.dds").is_none(), "bits are at most 8 digits: the char after them must be a dot");
		assert!(parse_file_name("readme.txt").is_none());
		assert!(parse_file_name("zz-00001993.dds").is_none());
	}

	#[test]
	fn mip_dumps_do_not_load() {
		assert!(parse_file_name("1a077a665f7258d-bddb5bfecca73b34-00005dd3-mip1.png").is_none());
		assert_eq!(mip_dump_level("1a077a665f7258d-bddb5bfecca73b34-00005dd3-mip1.png"), Some(1));
		assert_eq!(mip_dump_level("1a077a665f7258d-bddb5bfecca73b34-00005dd3.png"), None);
	}

	#[test]
	fn stem_round_trip() {
		let n = TexName::palette(0x1a077a665f7258d, 0xbddb5bfecca73b34, 0x1dd3);
		assert_eq!(n.stem(true), "1a077a665f7258d-bddb5bfecca73b34-00001dd3");
		assert_eq!(name(&format!("{}.png", n.stem(true))), n);
		let r = TexName { tex0: 5, clut: 6, region_w: 17, region_h: 9, bits: 0x1e13 };
		assert_eq!(r.stem(true), "5-6-r17x9-00001e13");
		assert_eq!(name(&format!("{}.dds", r.stem(true))), r);
		assert!(psm_has_palette(0x13) && !psm_has_palette(0x00));
	}
}
