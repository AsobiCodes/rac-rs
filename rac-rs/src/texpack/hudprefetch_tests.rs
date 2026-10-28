//! Tests of `hudprefetch`: the images of the asked names are decoded when `run` returns, missing names cost nothing, the cap stops it.

use super::*;
use crate::texpack::hudrepl::tests::{name, pack, png, tmp};
use crate::texpack::hudrepl::Registry;
use crate::texpack::xmap::XMap;

#[test]
fn every_asked_name_is_ready_when_run_returns() {
	let dir = tmp("pre_ready");
	let names: Vec<TexName> = (0..20).map(|i| name(0x1000 + i)).collect();
	let files: Vec<_> = names.iter().take(18).map(|n| (*n, "png", png(2, 2, &[9; 16]))).collect();
	let ix = pack(&dir, &files);
	let reg = Registry::new(1 << 20);
	// the list repeats two names (the HUD of two levels): asked once
	let mut asked = names.clone();
	asked.extend([names[0], names[1]]);
	let d = run(&asked, |n| reg.lookup(n, &ix, &XMap::default()), || reg.stats().bytes, 1 << 20);
	assert_eq!(d, Done { asked: 20, ready: 18, other: 2, capped: false });
	for n in &names[..18] {
		assert_eq!(reg.lookup(n, &ix, &XMap::default()).state(), State::Ready, "no pending left for the first frame");
	}
	assert_eq!(reg.stats().names, 20);
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_cap_stops_the_prefetch_before_the_next_batch() {
	let dir = tmp("pre_cap");
	let names: Vec<TexName> = (0..40).map(|i| name(0x2000 + i)).collect();
	let files: Vec<_> = names.iter().map(|n| (*n, "png", png(4, 4, &[7; 64]))).collect();
	let ix = pack(&dir, &files);
	let reg = Registry::new(1 << 20);
	// 64 bytes per image: a cap of 600 bytes is reached after the second batch of 8 (512 after the first)
	let d = run(&names, |n| reg.lookup(n, &ix, &XMap::default()), || reg.stats().bytes, 600);
	assert!(d.capped);
	assert_eq!((d.asked, d.ready), (16, 16));
	assert_eq!(reg.stats().names, 16, "the rest was never asked for");
	// images the atlas took leave the count: the same list goes on from where it stopped
	for n in &names[..16] {
		reg.lookup(n, &ix, &XMap::default()).release();
	}
	let d = run(&names, |n| reg.lookup(n, &ix, &XMap::default()), || reg.stats().bytes, 600);
	assert!(d.asked >= 24, "{d:?}");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_empty_list_and_no_pack_do_nothing() {
	let reg = Registry::new(1 << 20);
	assert_eq!(run(&[], |n| reg.lookup(n, &crate::texpack::index::PackIndex::new(), &XMap::default()), || 0, 1), Done::default());
	// start() returns at once: no pack is enabled in the unit tests' working folder, or the list is empty
	start(Vec::new());
}

#[test]
fn an_older_generation_stops_asking_when_a_newer_level_load_began() {
	let g1 = begin();
	assert_ne!(used_unless_superseded(g1), usize::MAX, "the newest generation counts the registry as usual");
	let g2 = begin();
	assert_eq!(used_unless_superseded(g1), usize::MAX, "the older thread sees the cap at its next batch and stops");
	assert_ne!(used_unless_superseded(g2), usize::MAX);
}
