//! Turns the pack hits of a level into replacement jobs (one per world texture key) for `repl::LevelRepl`.

use super::index::PackFile;
use super::levelrule;
use super::names::LevelHits;
use super::repl::{Fp, Job};
use super::source::TexSource;
use crate::level::{texture_key, Texture};
use std::collections::HashMap;

/// One job per texture key that has a pack file for at least one level and still holds its original in `textures`. `exclude` keys keep
/// their original. Per level the first hit wins (hits list the single-level name before the chain name); `levelrule::pick` then
/// decides which files the replacement is built from (authored chain, or the lowest named level with a generated chain).
pub fn make_jobs(srcs: &[TexSource], hits: &LevelHits, textures: &HashMap<u32, Texture>, exclude: &[u32]) -> Vec<Job> {
	let mut by_tex: HashMap<u32, [Option<PackFile>; 4]> = HashMap::new();
	for (ln, f) in &hits.hits {
		let key = texture_key(ln.group, ln.index as u32);
		let slot = &mut by_tex.entry(key).or_default()[(ln.level as usize).min(3)];
		if slot.is_none() {
			*slot = Some(f.clone());
		}
	}
	let mut jobs = Vec::new();
	for t in srcs {
		let key = texture_key(t.group, t.index as u32);
		let (Some(files), Some(orig)) = (by_tex.remove(&key), textures.get(&key)) else { continue };
		let files = levelrule::pick(&files, t.levels.len());
		if files[0].is_none() || exclude.contains(&key) {
			continue;
		}
		jobs.push(Job { key, fp: Fp::of(orig), n: t.levels.len(), files });
	}
	jobs
}

/// Loader threads: `RAC_TEXPACK_THREADS`, else a third of the logical CPUs (1 to 3); read once.
pub fn loader_threads() -> usize {
	static N: std::sync::OnceLock<usize> = std::sync::OnceLock::new();
	*N.get_or_init(|| std::env::var("RAC_TEXPACK_THREADS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|&n| n > 0).unwrap_or_else(|| (std::thread::available_parallelism().map_or(2, |n| n.get()) / 3).clamp(1, 3)))
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::level::TextureGroup;
	use crate::texpack::hash::Level as Lv;
	use crate::texpack::index::PackIndex;
	use crate::texpack::mods::MipNames;
	use crate::texpack::name::Ext;
	use crate::texpack::names::{resolve_with, texture_names};
	use crate::texpack::xmap::XMap;

	fn pat(n: usize, mul: usize) -> Vec<u8> {
		(0..n).map(|i| ((i * mul + 3) & 255) as u8).collect()
	}

	/// Job files for a 4-level table entry when the pack names the levels in `named`.
	fn files_for(named: &[usize]) -> Option<[Option<String>; 4]> {
		let (a, b, c, d, pal) = (pat(64 * 64, 7), pat(32 * 32, 5), pat(16 * 16, 3), pat(8 * 8, 11), pat(1024, 13));
		let t = TexSource { group: TextureGroup::Tie, index: 7, w: 64, h: 64, pal: &pal, levels: vec![Lv { px: &a, w: 64, h: 64 }, Lv { px: &b, w: 32, h: 32 }, Lv { px: &c, w: 16, h: 16 }, Lv { px: &d, w: 8, h: 8 }] };
		let mut ix = PackIndex::new();
		for ln in texture_names(&t, MipNames::Single).into_iter().filter(|l| named.contains(&(l.level as usize))) {
			ix.insert(ln.name, PackFile { path: format!("l{}.dds", ln.level).into(), ext: Ext::Dds, mod_idx: 0 });
		}
		let srcs = [t];
		let hits = resolve_with(&srcs, MipNames::Single, &ix, &XMap::default());
		let key = texture_key(TextureGroup::Tie, 7);
		let orig = Texture { w: 64, h: 64, rgba: vec![[1, 2, 3, 4]; 64 * 64] };
		let jobs = make_jobs(&srcs, &hits, &HashMap::from([(key, orig)]), &[]);
		assert!(jobs.len() <= 1);
		jobs.into_iter().next().map(|j| {
			assert_eq!((j.key, j.n), (key, 4));
			j.files.map(|f| f.map(|f| f.path.to_string_lossy().into_owned()))
		})
	}

	#[test]
	fn pack_a_keeps_the_authored_chain_pack_b_generates_from_the_base() {
		let s = |v: &[&str]| -> [Option<String>; 4] { std::array::from_fn(|i| v.get(i).filter(|n| !n.is_empty()).map(|n| n.to_string())) };
		assert_eq!(files_for(&[0, 1, 2, 3]), Some(s(&["l0.dds", "l1.dds", "l2.dds", "l3.dds"])), "every level named: authored");
		assert_eq!(files_for(&[0, 3]), Some(s(&["l0.dds"])), "pack B: level 0 is the base");
		assert_eq!(files_for(&[0, 2, 3]), Some(s(&["l0.dds"])));
		assert_eq!(files_for(&[3]), Some(s(&["l3.dds"])), "only a smaller level: it is the base");
		assert_eq!(files_for(&[2, 3]), Some(s(&["l2.dds"])));
		assert_eq!(files_for(&[]), None, "nothing named: no job");
	}
}
