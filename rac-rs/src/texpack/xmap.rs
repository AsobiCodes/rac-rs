//! [PC] PAL <-> USA texture-name map (lane tex2, stage B1). The PCSX2 packs are named for the USA disc (SCUS-97199); the PAL disc has a
//! few hundred textures whose name differs from the USA twin (a different CLUT or a re-encoded picture), so the pack has no file for
//! the PAL name. `assets/texmap.tsv` pairs those names (the status "ok" rows of the lane's `tools/map.py`: pictures that match within
//! one grey level; hashes only, no game data). `twins` is symmetric, so one table serves both directions: a USA pack on the PAL disc
//! (PAL name -> USA twin) and a PAL-named pack on the USA disc (USA name -> its PAL twins). Identical names need no row. The PAL-side
//! and USA-side names of the table are disjoint (unit test), so the lookup does not need to know which disc is loaded. One USA name
//! can have several PAL twins (4 of the 92 rows).

use super::name::{parse_file_name, TexName};
use std::collections::HashMap;
use std::sync::OnceLock;

const TSV: &str = include_str!("../../assets/texmap.tsv");

/// The twin names of every mapped name, both directions.
#[derive(Default)]
pub struct XMap {
	twins: HashMap<TexName, Vec<TexName>>,
	rows: usize,
}

/// A file stem (`tex0-clut-bits` or `tex0-bits`, hex) as a name; None when it is in no PCSX2 format.
fn stem_name(stem: &str) -> Option<TexName> {
	parse_file_name(&format!("{stem}.png")).map(|(n, _)| n)
}

impl XMap {
	/// Reads tab-separated rows `pal_stem  usa_stem  ...` (more columns are ignored). Blank lines, `#` lines, the header and rows whose
	/// first two fields are no names are skipped.
	pub fn parse(text: &str) -> XMap {
		let mut m = XMap::default();
		for line in text.lines().map(str::trim).filter(|l| !l.is_empty() && !l.starts_with('#')) {
			let mut f = line.split('\t');
			let (Some(Some(pal)), Some(Some(usa))) = (f.next().map(stem_name), f.next().map(stem_name)) else { continue };
			m.add(pal, usa);
		}
		m
	}

	/// One pair, both directions; a pair or name seen twice adds nothing.
	pub fn add(&mut self, pal: TexName, usa: TexName) {
		if pal == usa {
			return;
		}
		for (a, b) in [(pal, usa), (usa, pal)] {
			let v = self.twins.entry(a).or_default();
			if !v.contains(&b) {
				v.push(b);
			}
		}
		self.rows += 1;
	}

	/// The other disc's names for `n` (empty when `n` is not mapped).
	pub fn twins(&self, n: &TexName) -> &[TexName] {
		self.twins.get(n).map_or(&[], Vec::as_slice)
	}

	/// Pairs read.
	pub fn rows(&self) -> usize {
		self.rows
	}
}

/// The shipped table, parsed once.
pub fn shipped() -> &'static XMap {
	static MAP: OnceLock<XMap> = OnceLock::new();
	MAP.get_or_init(|| XMap::parse(TSV))
}

#[cfg(test)]
mod tests {
	use super::*;
	use std::collections::HashSet;

	fn n(stem: &str) -> TexName {
		stem_name(stem).unwrap_or_else(|| panic!("{stem} should parse"))
	}

	#[test]
	fn parse_reads_pairs_both_ways_and_skips_the_rest() {
		let text = "pal_stem\tusa_stem\tkind\tmip\n# comment\n\n1a-2b-00001dd3\t3c-4d-00001dd3\tentry\t0\n5e-00002a80\t6f-00002a80\tgrab\t0\nnot a name\tx\n";
		let m = XMap::parse(text);
		assert_eq!(m.rows(), 2, "the header and the broken row are skipped");
		assert_eq!(m.twins(&n("1a-2b-00001dd3")), &[n("3c-4d-00001dd3")]);
		assert_eq!(m.twins(&n("3c-4d-00001dd3")), &[n("1a-2b-00001dd3")], "the other direction");
		assert_eq!(m.twins(&n("5e-00002a80")), &[n("6f-00002a80")], "names without a CLUT part work too");
		assert!(m.twins(&n("7-8-00001dd3")).is_empty());
	}

	#[test]
	fn one_usa_name_with_several_pal_twins_and_repeats() {
		let mut m = XMap::default();
		m.add(n("1-2-00001dd3"), n("9-9-00001dd3"));
		m.add(n("3-2-00001dd3"), n("9-9-00001dd3"));
		m.add(n("3-2-00001dd3"), n("9-9-00001dd3"));
		m.add(n("4-4-00001dd3"), n("4-4-00001dd3"));
		assert_eq!(m.twins(&n("9-9-00001dd3")), &[n("1-2-00001dd3"), n("3-2-00001dd3")]);
		assert_eq!(m.twins(&n("3-2-00001dd3")), &[n("9-9-00001dd3")]);
		assert!(m.twins(&n("4-4-00001dd3")).is_empty(), "identical names need no row");
	}

	#[test]
	fn the_shipped_table_is_the_92_ok_rows_with_disjoint_sides() {
		let m = shipped();
		assert_eq!(m.rows(), 92, "status ok rows of texmap.tsv; the 11 review rows are not shipped");
		let (mut pal, mut usa) = (HashSet::new(), HashSet::new());
		for line in TSV.lines().skip(1) {
			let mut f = line.split('\t');
			pal.insert(n(f.next().unwrap()));
			usa.insert(n(f.next().unwrap()));
		}
		assert_eq!((pal.len(), usa.len()), (92, 88));
		assert!(pal.is_disjoint(&usa), "no name is on both sides, so the lookup needs no disc id");
		for p in &pal {
			assert!(m.twins(p).iter().all(|t| usa.contains(t)));
		}
	}
}
