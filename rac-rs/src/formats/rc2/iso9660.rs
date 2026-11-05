//! ISO9660 file reads from the player's own disc image (R&C2 names its WADs, unlike R&C1's sector table).

use crate::bytes::{span, u32_at, R};
use std::io::{Read, Seek, SeekFrom};

fn sectors(f: &mut std::fs::File, lba: u32, len: usize) -> R<Vec<u8>> {
	let mut buf = vec![0u8; len];
	f.seek(SeekFrom::Start(lba as u64 * 0x800)).map_err(|e| e.to_string())?;
	f.read_exact(&mut buf).map_err(|e| e.to_string())?;
	Ok(buf)
}

/// (lba, size) of `path` ("G/LEVEL0.WAD"), case-insensitive, version suffix ignored.
pub fn locate(f: &mut std::fs::File, path: &str) -> R<(u32, u32)> {
	let pvd = sectors(f, 16, 0x800)?;
	if &pvd[1..6] != b"CD001" {
		return Err("not an ISO9660 image".into());
	}
	let (mut lba, mut size) = (u32_at(&pvd, 158)?, u32_at(&pvd, 166)?);
	for part in path.split('/') {
		let dir = sectors(f, lba, size as usize)?;
		let mut o = 0;
		let mut found = None;
		while o < dir.len() {
			let l = dir[o] as usize;
			if l == 0 {
				o = (o / 0x800 + 1) * 0x800;
				continue;
			}
			let rec = span(&dir, o, l)?;
			let nl = rec[32] as usize;
			let name = String::from_utf8_lossy(span(rec, 33, nl)?).to_string();
			if name.split(';').next().unwrap_or("").eq_ignore_ascii_case(part) {
				found = Some((u32_at(rec, 2)?, u32_at(rec, 10)?));
				break;
			}
			o += l;
		}
		(lba, size) = found.ok_or_else(|| format!("{path}: {part} not on the disc"))?;
	}
	Ok((lba, size))
}

pub fn read_file(iso: &str, path: &str) -> R<Vec<u8>> {
	let mut f = std::fs::File::open(iso).map_err(|e| format!("{iso}: {e}"))?;
	let (lba, size) = locate(&mut f, path)?;
	sectors(&mut f, lba, size as usize)
}
