//! The R&C1 table of contents at sector 1500 (0x2960 bytes, read once at boot).
//!
//! Layout: header (1, 0x2960), then fixed-size groups of (sector, size) slots for global assets
//! (empty slots are zero), then at 0x28c8 a table of 19 (header_sector, size_sectors) pairs, one
//! per level. Each level header starts with (level id, 0x2434) and four sector ranges.
//! Offsets cross-checked against the rac1-decomp project's notes (PAL v2.00) and verified on v1.00.

use crate::iso::{u32_le, Disc};
use std::io;

pub const TOC_SECTOR: u64 = 1500;
pub const TOC_SIZE: usize = 0x2960;
const LEVEL_TABLE: usize = 0x28c8;
pub const LEVEL_COUNT: usize = 19;
const LEVEL_HEADER_SIZE: usize = 0x2434;

pub const LEVEL_NAMES: [&str; LEVEL_COUNT] = [
	"Veldin", "Novalis", "Aridia", "Kerwan", "Eudora", "Rilgar", "Blarg Station (Nebula G34)",
	"Umbris", "Batalia", "Gaspar", "Orxon", "Pokitaru", "Hoven", "Gemlik Base", "Oltanis",
	"Quartu", "Kalebo III", "Drek's Fleet", "Veldin (return)",
];

#[derive(Debug, Clone, Copy)]
pub struct Range {
	pub sector: u64,
	pub sectors: u64,
}

#[derive(Debug, Clone)]
pub struct Level {
	pub id: usize,
	pub header_sector: u64,
	pub data: Range,
	pub gameplay_pal: Range,
	pub occlusion: Range,
}

fn bad(msg: String) -> io::Error {
	io::Error::new(io::ErrorKind::InvalidData, msg)
}

pub fn read_toc(disc: &mut Disc) -> io::Result<Vec<u8>> {
	let toc = disc.read_bytes(TOC_SECTOR, TOC_SIZE)?;
	if u32_le(&toc, 0) != 1 || u32_le(&toc, 4) as usize != TOC_SIZE {
		return Err(bad("unexpected table of contents header".into()));
	}
	Ok(toc)
}

pub fn read_levels(disc: &mut Disc) -> io::Result<Vec<Level>> {
	let toc = read_toc(disc)?;
	let mut levels = Vec::with_capacity(LEVEL_COUNT);
	for id in 0..LEVEL_COUNT {
		let header_sector = u32_le(&toc, LEVEL_TABLE + id * 8) as u64;
		let h = disc.read_bytes(header_sector, LEVEL_HEADER_SIZE)?;
		if u32_le(&h, 0) as usize != id || u32_le(&h, 4) as usize != LEVEL_HEADER_SIZE {
			return Err(bad(format!("level {id}: unexpected header at sector {header_sector}")));
		}
		let range = |i: usize| -> io::Result<Range> {
			let r = Range { sector: u32_le(&h, 8 + i * 8) as u64, sectors: u32_le(&h, 12 + i * 8) as u64 };
			if r.sector + r.sectors > disc.sector_count {
				return Err(bad(format!("level {id}: range {i} outside the disc")));
			}
			Ok(r)
		};
		levels.push(Level {
			id,
			header_sector,
			data: range(0)?,
			gameplay_pal: range(2)?,
			occlusion: range(3)?,
		});
	}
	Ok(levels)
}
