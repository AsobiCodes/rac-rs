//! The pack index: every replacement file of the enabled textures mods by `TexName`. Folders are scanned recursively (hidden
//! files too, like PCSX2); the first file for a name wins, mods in priority order. Only png and dds files whose name parses are
//! indexed; `-mipN` dumps and other files are counted, not loaded.

use super::name::{self, Ext, TexName};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Where a replacement image is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackFile {
	pub path: PathBuf,
	pub ext: Ext,
	/// Position of the mod in priority order (0 = highest).
	pub mod_idx: usize,
}

/// What a scan found.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanStats {
	/// Image files that parsed to a name and were added.
	pub added: usize,
	/// Parsed names already provided by an earlier file (kept the first).
	pub duplicates: usize,
	/// `-mipN` dump files (never loaded).
	pub mip_dumps: usize,
	/// Everything else (other extensions, names in no format).
	pub other: usize,
}

#[derive(Default)]
pub struct PackIndex {
	map: HashMap<TexName, PackFile>,
	/// The names of each TEX0 (texel) hash, for textures whose CLUT changes (`names_of_tex0`).
	by_tex0: HashMap<u64, Vec<TexName>>,
}

impl PackIndex {
	pub fn new() -> PackIndex {
		PackIndex::default()
	}

	pub fn len(&self) -> usize {
		self.map.len()
	}

	pub fn is_empty(&self) -> bool {
		self.map.is_empty()
	}

	pub fn get(&self, n: &TexName) -> Option<&PackFile> {
		self.map.get(n)
	}

	/// Every name in the index with this TEX0 (texel) hash: the same image under different CLUTs / sizes.
	pub fn names_of_tex0(&self, tex0: u64) -> &[TexName] {
		self.by_tex0.get(&tex0).map_or(&[], Vec::as_slice)
	}

	/// Adds a file under a name unless an earlier one has it; true when added.
	pub fn insert(&mut self, n: TexName, f: PackFile) -> bool {
		match self.map.entry(n) {
			std::collections::hash_map::Entry::Occupied(_) => false,
			std::collections::hash_map::Entry::Vacant(v) => {
				v.insert(f);
				self.by_tex0.entry(n.tex0).or_default().push(n);
				true
			}
		}
	}

	/// Scans `dir` recursively for replacement files of the mod at priority `mod_idx`. Entries are visited in sorted order so the
	/// result does not depend on the file system.
	pub fn scan_dir(&mut self, dir: &Path, mod_idx: usize) -> ScanStats {
		let mut st = ScanStats::default();
		self.scan(dir, mod_idx, &mut st, 0);
		st
	}

	fn scan(&mut self, dir: &Path, mod_idx: usize, st: &mut ScanStats, depth: u32) {
		let Ok(rd) = std::fs::read_dir(dir) else { return };
		let mut entries: Vec<_> = rd.flatten().collect();
		entries.sort_by_key(|e| e.file_name());
		for e in entries {
			let Ok(ft) = e.file_type() else { continue };
			let path = e.path();
			if ft.is_dir() {
				if depth < 16 {
					self.scan(&path, mod_idx, st, depth + 1);
				}
				continue;
			}
			let fname = e.file_name().to_string_lossy().into_owned();
			match name::parse_file_name(&fname) {
				Some((n, ext)) => {
					if self.insert(n, PackFile { path, ext, mod_idx }) {
						st.added += 1;
					} else {
						st.duplicates += 1;
					}
				}
				None if name::ext_of(&fname).is_some() && name::mip_dump_level(&fname).is_some() => st.mip_dumps += 1,
				None => st.other += 1,
			}
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn tmp(tag: &str) -> PathBuf {
		let d = std::env::temp_dir().join(format!("texpack_idx_{tag}_{}", std::process::id()));
		let _ = std::fs::remove_dir_all(&d);
		std::fs::create_dir_all(&d).unwrap();
		d
	}

	#[test]
	fn scans_recursively_first_file_wins() {
		let root = tmp("scan");
		std::fs::create_dir_all(root.join("UI")).unwrap();
		std::fs::write(root.join("1-2-00005dd3.dds"), b"x").unwrap();
		std::fs::write(root.join("UI").join("3-4-r31002000610050-00005e13.PNG"), b"x").unwrap();
		std::fs::write(root.join("UI").join("1-2-00001dd3.png"), b"x").unwrap(); // same name as the first (bit 14 masked)
		std::fs::write(root.join("5-6-00001193-mip1.png"), b"x").unwrap();
		std::fs::write(root.join("readme.txt"), b"x").unwrap();
		let mut ix = PackIndex::new();
		let st = ix.scan_dir(&root, 0);
		assert_eq!(st, ScanStats { added: 2, duplicates: 1, mip_dumps: 1, other: 1 });
		assert_eq!(ix.len(), 2);
		let hit = ix.get(&TexName::palette(1, 2, 0x1dd3)).unwrap();
		assert!(hit.path.ends_with("1-2-00005dd3.dds") || hit.path.ends_with("UI/1-2-00001dd3.png") || hit.path.ends_with("UI\\1-2-00001dd3.png"));
		assert_eq!(hit.ext, if hit.path.extension().unwrap() == "dds" { Ext::Dds } else { Ext::Png });
		let r = ix.get(&TexName { tex0: 3, clut: 4, region_w: 17, region_h: 17, bits: 0x1e13 }).unwrap();
		assert_eq!(r.ext, Ext::Png);
		let _ = std::fs::remove_dir_all(&root);
	}

	#[test]
	fn earlier_mod_wins_over_later_mod() {
		let (a, b) = (tmp("moda"), tmp("modb"));
		std::fs::write(a.join("1-2-00001dd3.png"), b"x").unwrap();
		std::fs::write(b.join("1-2-00001dd3.dds"), b"x").unwrap();
		std::fs::write(b.join("9-9-00001dd3.dds"), b"x").unwrap();
		let mut ix = PackIndex::new();
		ix.scan_dir(&a, 0);
		let st = ix.scan_dir(&b, 1);
		assert_eq!((st.added, st.duplicates), (1, 1));
		assert_eq!(ix.get(&TexName::palette(1, 2, 0x1dd3)).unwrap().mod_idx, 0);
		assert_eq!(ix.get(&TexName::palette(9, 9, 0x1dd3)).unwrap().mod_idx, 1);
		let _ = (std::fs::remove_dir_all(&a), std::fs::remove_dir_all(&b));
	}
}
