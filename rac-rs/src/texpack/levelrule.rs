//! [PC] Which pack files a world texture is built from (lane tex2, stage B1). A table entry has up to 4 original mip levels and the
//! pack names some of them: pack A (HD Remaster folder) names every level and keeps its authored chain, pack B (SCUS-97199.zip) names
//! mip 0 and 3 only (a few 2), and some entries have no file for level 0 at all (the name of the other disc, or a picture shared with a
//! smaller texture). A replacement therefore is built from the LOWEST named level of the entry: its file is the base and the chain
//! below it is generated (2x2 box filter, `chain::assemble`), unless every level of the entry has its own file (authored chain).

/// `files[k]` = the pack file named for original level `k` (`n` levels in the original, 1..=4). Returns the files the replacement is
/// built from: all of them when each of the `n` levels is named, else only the lowest named level in slot 0 (None when nothing is named).
pub fn pick<T: Clone>(files: &[Option<T>; 4], n: usize) -> [Option<T>; 4] {
	let n = n.clamp(1, 4);
	if files[..n].iter().all(Option::is_some) {
		return files.clone();
	}
	[files.iter().flatten().next().cloned(), None, None, None]
}

#[cfg(test)]
mod tests {
	use super::pick;

	#[test]
	fn all_levels_named_keeps_the_authored_chain() {
		let f = [Some('a'), Some('b'), Some('c'), Some('d')];
		assert_eq!(pick(&f, 4), f);
		let f = [Some('a'), Some('b'), None, None];
		assert_eq!(pick(&f, 2), f, "a 2-level original is fully named by 2 files");
		assert_eq!(pick(&[Some('a'), None, None, None], 1), [Some('a'), None, None, None]);
	}

	#[test]
	fn pack_b_mip_0_and_3_generates_the_chain_from_level_0() {
		assert_eq!(pick(&[Some('a'), None, None, Some('d')], 4), [Some('a'), None, None, None]);
		assert_eq!(pick(&[Some('a'), None, Some('c'), Some('d')], 4), [Some('a'), None, None, None]);
	}

	#[test]
	fn the_lowest_named_level_is_the_base() {
		assert_eq!(pick(&[None, None, None, Some('d')], 4), [Some('d'), None, None, None]);
		assert_eq!(pick(&[None, None, Some('c'), Some('d')], 4), [Some('c'), None, None, None]);
		assert_eq!(pick(&[None, Some('b'), Some('c'), Some('d')], 4), [Some('b'), None, None, None]);
		assert_eq!(pick(&[None, Some('b'), None, None], 3), [Some('b'), None, None, None]);
	}

	#[test]
	fn nothing_named_gives_nothing() {
		assert_eq!(pick::<char>(&[None, None, None, None], 4), [None, None, None, None]);
	}
}
