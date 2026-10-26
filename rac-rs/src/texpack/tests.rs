//! Tests gated on the player's disc (and, for the pack numbers, on a local pack folder): per-level name counts against the numbers
//! of the research script texhash.py / full_match.py. Without the disc or the pack the tests return early.

use super::index::PackIndex;
use super::mods::MipNames;
use super::{names, source};
use crate::level::Level;
use std::path::Path;

const ISO: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");
const PACK: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scratch_texpack/pack/Ratchet & Clank (USA) [SCUS-97199] HD Remaster/SCUS-97199/replacements");

/// texhash.py per level 0..18: (textures, single-level names with repeats, distinct names, distinct names in the real USA pack,
/// textures with at least one name in it). Counts only, no game or pack data.
const GOLDEN: [(usize, usize, usize, usize, usize); 19] = [
	(402, 1472, 1159, 1061, 397), (616, 2310, 1830, 1390, 594), (439, 1625, 1215, 1051, 426), (434, 1566, 1266, 966, 374), (481, 1771, 1364, 1074, 466),
	(496, 1823, 1500, 1034, 450), (585, 2161, 1346, 993, 563), (384, 1398, 1088, 776, 369), (417, 1513, 1320, 1137, 403), (389, 1413, 1075, 800, 352),
	(422, 1546, 1107, 950, 410), (436, 1594, 1317, 1065, 419), (560, 2046, 1594, 1256, 537), (501, 1859, 1420, 1120, 461), (401, 1460, 1227, 983, 388),
	(578, 2151, 1489, 1182, 545), (459, 1688, 1287, 1092, 449), (417, 1526, 1165, 931, 392), (605, 2263, 1488, 1307, 589),
];

fn disc() -> Option<crate::iso::Disc> {
	Path::new(ISO).is_file().then(|| crate::iso::Disc::open(Path::new(ISO)).unwrap())
}

#[test]
fn level_name_counts_match_texhash_py() {
	let Some(mut d) = disc() else { return };
	let levels = crate::toc::read_levels(&mut d).unwrap();
	let pack = Path::new(PACK).is_dir().then(|| {
		let mut ix = PackIndex::new();
		let st = ix.scan_dir(Path::new(PACK), 0);
		eprintln!("pack scan: {st:?}");
		ix
	});
	for (i, g) in GOLDEN.iter().enumerate() {
		let l = Level::load(&mut d, &levels[i]).unwrap();
		let (srcs, skipped) = source::level_sources(&l).unwrap();
		assert_eq!((srcs.len(), skipped), (g.0, 0), "level {i} textures");
		let all = names::level_names(&srcs, MipNames::Single);
		assert_eq!(all.len(), g.1, "level {i} names");
		let distinct: std::collections::HashSet<_> = all.iter().map(|n| n.name).collect();
		assert_eq!(distinct.len(), g.2, "level {i} distinct names");
		if let Some(ix) = &pack {
			// GOLDEN counts the names as they are (texhash.py has no PAL <-> USA name map).
			let r = names::resolve_with(&srcs, MipNames::Single, ix, &super::xmap::XMap::default());
			assert_eq!((r.hit_names, r.textures_hit), (g.3, g.4), "level {i} pack hits");
		}
	}
}

#[test]
fn hook_is_a_no_op_without_mods() {
	// No mods folder in the test run: nothing is read, nothing returned.
	let Some(mut d) = disc() else { return };
	if std::env::var_os("RAC_MODS_DIR").is_some() {
		return;
	}
	let levels = crate::toc::read_levels(&mut d).unwrap();
	let l = Level::load(&mut d, &levels[0]).unwrap();
	assert!(super::level_loaded(&l, &std::collections::HashMap::new(), &[], false).is_none());
}

/// The number of table entries of all 19 PAL levels that get a replacement (any level named), summed: `tools\cov.py` of lane tex2
/// (COVERAGE.md: pack A 8,584 direct / 8,624 with the name map, pack B 8,769 / 8,801; 9,022 entries). Pack A is `PACK`, pack B is the
/// folder named by `RAC_TEST_PACK_B` (the extracted SCUS-97199.zip `replacements` folder); a missing pack skips its part.
#[test]
fn world_entries_with_a_file_match_cov_py() {
	let Some(mut d) = disc() else { return };
	let levels = crate::toc::read_levels(&mut d).unwrap();
	let packs: [(&str, Option<std::path::PathBuf>, [usize; 2]); 2] = [
		("A", Path::new(PACK).is_dir().then(|| PACK.into()), [8584, 8624]),
		("B", std::env::var_os("RAC_TEST_PACK_B").map(Into::into).filter(|p: &std::path::PathBuf| p.is_dir()), [8769, 8801]),
	];
	for (tag, dir, want) in packs {
		let Some(dir) = dir else { continue };
		let mut ix = PackIndex::new();
		ix.scan_dir(&dir, 0);
		let (mut direct, mut mapped, mut total, mut via) = (0, 0, 0, 0);
		for lv in levels.iter().take(19) {
			let l = Level::load(&mut d, lv).unwrap();
			let (srcs, _) = source::level_sources(&l).unwrap();
			total += srcs.len();
			direct += names::resolve_with(&srcs, MipNames::Single, &ix, &super::xmap::XMap::default()).textures_hit;
			let r = names::resolve(&srcs, MipNames::Single, &ix);
			(mapped, via) = (mapped + r.textures_hit, via + r.mapped_names);
		}
		eprintln!("pack {tag}: {total} entries, {direct} with a file direct, {mapped} with the name map ({via} names through it)");
		assert_eq!((direct, mapped), (want[0], want[1]), "pack {tag}");
	}
}
