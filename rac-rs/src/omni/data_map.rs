//! The map of the overlay's global data for native mods (`relocate`, `relocate_level`).
//!
//! Mods name a global of a level's program by its address in the format's reference numbering: the NTSC-U disc's level-0 program. Level programs keep their
//! globals at other addresses (every level, and the PAL disc against the USA one), so `rac1_data.txt` (written by tools/make_data_map.py from pairs of
//! copies of the same function in two levels and from the PAL -> USA disc map; addresses only) lists, for each disc and each loaded level, the runs
//! `lo hi delta` of the reference address range that move by `delta`. The executable's globals (below 0x15EF00) do not move and are not in the table.
//! An address outside every run is unmapped: `relocate` says so (and the mod gets 0).

use std::sync::OnceLock;

/// One run: a reference address in `lo..hi` is `address + delta` in the loaded program.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Run {
	pub lo: u32,
	pub hi: u32,
	pub delta: i32,
}

/// The runs of one disc and one loaded level (`level` -1: none loaded, only the head of the overlay data).
#[derive(Debug, Default)]
pub struct DataMap {
	sections: Vec<(bool, i32, Vec<Run>)>,
}

fn num(t: &str) -> Result<i64, String> {
	let r = match t.strip_prefix("0x") {
		Some(h) => i64::from_str_radix(h, 16),
		None => t.parse::<i64>(),
	};
	r.map_err(|_| format!("`{t}` is not a number"))
}

impl DataMap {
	/// Parses the table text: `#` comment lines, section lines `pal L` / `usa L`, run lines `lo hi delta`.
	pub fn parse(text: &str) -> Result<DataMap, String> {
		let mut m = DataMap::default();
		for (n, line) in text.lines().enumerate() {
			let line = line.trim();
			if line.is_empty() || line.starts_with('#') {
				continue;
			}
			let at = |e: String| format!("line {}: {e}", n + 1);
			let f: Vec<&str> = line.split_whitespace().collect();
			match f.as_slice() {
				[disc @ ("pal" | "usa"), level] => {
					let l = i32::try_from(num(level).map_err(at)?).map_err(|_| at("bad level".into()))?;
					m.sections.push((*disc == "usa", l, Vec::new()));
				}
				[lo, hi, delta] => {
					let (lo, hi, delta) = (num(lo).map_err(at)?, num(hi).map_err(at)?, num(delta).map_err(at)?);
					let run = Run { lo: u32::try_from(lo).map_err(|_| at("bad lo".into()))?, hi: u32::try_from(hi).map_err(|_| at("bad hi".into()))?, delta: i32::try_from(delta).map_err(|_| at("bad delta".into()))? };
					if run.hi <= run.lo {
						return Err(at("hi is not above lo".into()));
					}
					m.sections.last_mut().ok_or_else(|| at("a run before any `pal L` / `usa L` line".into()))?.2.push(run);
				}
				_ => return Err(at(format!("cannot read `{line}`"))),
			}
		}
		Ok(m)
	}

	/// The address of the global at reference address `addr` in the program of `level` (-1 none) on the PAL (`usa` false) or USA disc.
	pub fn lookup(&self, usa: bool, level: i32, addr: u32) -> Option<u32> {
		let (_, _, runs) = self.sections.iter().find(|(u, l, _)| *u == usa && *l == level)?;
		let r = runs.iter().find(|r| (r.lo..r.hi).contains(&addr))?;
		Some((i64::from(addr) + i64::from(r.delta)) as u32)
	}

	/// Number of runs of a section (tests, the run log).
	pub fn runs(&self, usa: bool, level: i32) -> usize {
		self.sections.iter().find(|(u, l, _)| *u == usa && *l == level).map_or(0, |s| s.2.len())
	}
}

/// The table built into the exe (`rac1_data.txt`).
pub fn builtin() -> &'static DataMap {
	static MAP: OnceLock<DataMap> = OnceLock::new();
	MAP.get_or_init(|| DataMap::parse(include_str!("rac1_data.txt")).unwrap_or_else(|e| {
		eprintln!("omni: rac1_data.txt is damaged ({e}); overlay data stays unmapped");
		DataMap::default()
	}))
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn parse_reads_sections_and_runs() {
		let m = DataMap::parse("# c\npal 3\n0x100 0x200 16\n0x300 0x400 -32\nusa -1\n0x10 0x20 0\n").unwrap();
		assert_eq!(m.lookup(false, 3, 0x100), Some(0x110));
		assert_eq!(m.lookup(false, 3, 0x1ff), Some(0x20f));
		assert_eq!(m.lookup(false, 3, 0x200), None);
		assert_eq!(m.lookup(false, 3, 0x310), Some(0x2f0));
		assert_eq!(m.lookup(true, -1, 0x10), Some(0x10));
		assert_eq!(m.lookup(true, 3, 0x100), None);
		assert_eq!(m.lookup(false, 4, 0x100), None);
	}

	#[test]
	fn parse_refuses_damage() {
		assert!(DataMap::parse("0x1 0x2 0\n").is_err());
		assert!(DataMap::parse("pal 1\n0x5 0x5 0\n").is_err());
		assert!(DataMap::parse("pal 1\nnonsense\n").is_err());
		assert!(DataMap::parse("pal x\n").is_err());
	}

	/// The camera yaw and the other lock-strafe globals, level by level (PAL v1.00): found by the same code in two levels, checked against the
	/// decomp's reference lists (the hero code of Kerwan reads 0x166e58, Eudora 0x166ed8, Novalis 0x1671d8, Veldin finale 0x167758).
	#[test]
	fn lock_strafe_globals() {
		let m = builtin();
		let yaw = |l| m.lookup(false, l, 0x16_6DD8);
		assert_eq!((yaw(0), yaw(1), yaw(3), yaw(4), yaw(18)), (Some(0x16_6DD8), Some(0x16_71D8), Some(0x16_6E58), Some(0x16_6ED8), Some(0x16_7758)));
		for l in 0..19 {
			assert!(yaw(l).is_some(), "camera yaw unmapped in level {l}");
			assert_eq!(m.lookup(false, l, 0x15_F5C4), Some(0x15_F5A4), "level mode, level {l}");
			assert_eq!(m.lookup(false, l, 0x15_EF40), Some(0x15_EF40), "camera pan, level {l}");
			assert_eq!(m.lookup(true, l, 0x15_F5C4), Some(0x15_F5C4), "USA level mode, level {l}");
		}
	}

	#[test]
	fn no_level_loaded_maps_only_the_head() {
		let m = builtin();
		assert_eq!(m.lookup(false, -1, 0x15_F5C4), Some(0x15_F5A4));
		assert_eq!(m.lookup(false, -1, 0x16_6DD8), None);
		assert!(m.runs(false, 0) > 10 && m.runs(true, 3) > 10);
	}
}
