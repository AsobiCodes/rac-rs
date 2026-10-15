//! Raw sector access to the user's own disc image. R&C1 reads almost everything by sector number,
//! so the ISO9660 filesystem is only needed for SYSTEM.CNF and the boot ELF.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

pub const SECTOR_SIZE: u64 = 0x800;

pub struct Disc {
	file: File,
	pub sector_count: u64,
}

impl Disc {
	pub fn open(path: &Path) -> io::Result<Disc> {
		let file = File::open(path)?;
		let sector_count = file.metadata()?.len() / SECTOR_SIZE;
		Ok(Disc { file, sector_count })
	}

	/// Read `count` whole sectors starting at `sector`.
	pub fn read_sectors(&mut self, sector: u64, count: u64) -> io::Result<Vec<u8>> {
		let mut buf = vec![0u8; (count * SECTOR_SIZE) as usize];
		self.file.seek(SeekFrom::Start(sector * SECTOR_SIZE))?;
		self.file.read_exact(&mut buf)?;
		Ok(buf)
	}

	/// Read `len` bytes starting at the beginning of `sector`.
	pub fn read_bytes(&mut self, sector: u64, len: usize) -> io::Result<Vec<u8>> {
		let mut buf = vec![0u8; len];
		self.file.seek(SeekFrom::Start(sector * SECTOR_SIZE))?;
		self.file.read_exact(&mut buf)?;
		Ok(buf)
	}
}

pub fn u32_le(b: &[u8], off: usize) -> u32 {
	u32::from_le_bytes([b[off], b[off + 1], b[off + 2], b[off + 3]])
}
