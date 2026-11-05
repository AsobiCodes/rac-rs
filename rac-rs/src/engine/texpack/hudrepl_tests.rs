//! Tests of `hudrepl`: name resolution (index, xmap twin), missing / refused files, the memory budget, decoded pixels, and that
//! nothing runs without a pack. Fixtures are tiny PNG / DDS files made here in a temp folder.

use super::*;
use crate::texpack::dds::tests::dds;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub(in crate::texpack) fn tmp(tag: &str) -> PathBuf {
	let d = std::env::temp_dir().join(format!("texpack_hud_{tag}_{}", std::process::id()));
	let _ = std::fs::remove_dir_all(&d);
	std::fs::create_dir_all(&d).unwrap();
	d
}

/// An 8-bit RGBA PNG (stored deflate blocks, no CRCs: the decoder does not check them).
pub(in crate::texpack) fn png(w: u32, h: u32, px: &[u8]) -> Vec<u8> {
	assert_eq!(px.len(), (w * h * 4) as usize);
	let mut raw = Vec::new();
	for row in px.chunks((w * 4) as usize) {
		raw.push(0);
		raw.extend_from_slice(row);
	}
	let mut out = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
	let mut chunk = |k: &[u8], d: &[u8]| {
		out.extend((d.len() as u32).to_be_bytes());
		out.extend(k);
		out.extend(d);
		out.extend([0u8; 4]);
	};
	let mut ih = Vec::new();
	ih.extend(w.to_be_bytes());
	ih.extend(h.to_be_bytes());
	ih.extend([8, 6, 0, 0, 0]);
	chunk(b"IHDR", &ih);
	let mut z = vec![0x78, 0x01, 1];
	z.extend((raw.len() as u16).to_le_bytes());
	z.extend((!(raw.len() as u16)).to_le_bytes());
	z.extend(&raw);
	z.extend([0u8; 4]);
	chunk(b"IDAT", &z);
	chunk(b"IEND", &[]);
	out
}

pub(in crate::texpack) fn name(n: u64) -> TexName {
	TexName::palette(n, n + 1, 0x1dd3)
}

/// Writes `bytes` as the replacement file of `n` into `dir` and returns an index over the folder.
pub(in crate::texpack) fn pack(dir: &Path, files: &[(TexName, &str, Vec<u8>)]) -> PackIndex {
	for (n, ext, bytes) in files {
		std::fs::write(dir.join(format!("{}.{ext}", n.stem(true))), bytes).unwrap();
	}
	let mut ix = PackIndex::new();
	ix.scan_dir(dir, 0);
	assert_eq!(ix.len(), files.len(), "every fixture file parses to a name");
	ix
}

pub(in crate::texpack) fn settle(s: &Slot) -> State {
	let t = Instant::now();
	while s.state() == State::Pending && t.elapsed() < Duration::from_secs(20) {
		std::thread::sleep(Duration::from_millis(2));
	}
	s.state()
}

#[test]
fn a_name_resolves_through_the_index_then_the_xmap_twin() {
	let dir = tmp("twin");
	let (a, b, c) = (name(0x10), name(0x20), name(0x30));
	let ix = pack(&dir, &[(a, "png", png(1, 1, &[1, 2, 3, 0x80])), (c, "png", png(1, 1, &[4, 5, 6, 0x80]))]);
	let mut tw = XMap::default();
	tw.add(b, a); // b has no file, its twin a has
	tw.add(c, a); // c has its own file as well as a twin that has one: its own wins
	assert_eq!(resolve(&ix, &tw, &a).unwrap().path, ix.get(&a).unwrap().path);
	assert_eq!(resolve(&ix, &tw, &b).unwrap().path, ix.get(&a).unwrap().path, "the twin's file");
	assert_eq!(resolve(&ix, &tw, &c).unwrap().path, ix.get(&c).unwrap().path, "the name's own file first");
	assert!(resolve(&ix, &tw, &name(0x99)).is_none());
	let reg = Registry::new(1 << 20);
	let s = reg.lookup(&b, &ix, &tw);
	assert_eq!(settle(&s), State::Ready);
	assert_eq!(s.image().unwrap().rgba, vec![1, 2, 3, 0xff], "b shows a's pixels (PS2 alpha 0x80 -> engine 255)");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn decoded_size_and_pixels_and_one_slot_per_name() {
	let dir = tmp("px");
	let n = name(0x40);
	// 2 x 3 pixels; alpha 0x40 (PS2, half) -> 0x80 on the engine scale.
	let px: Vec<u8> = (0..6u8).flat_map(|i| [i * 10, i * 10 + 1, i * 10 + 2, 0x40]).collect();
	let ix = pack(&dir, &[(n, "png", png(2, 3, &px))]);
	let reg = Registry::new(1 << 20);
	let s = reg.lookup(&n, &ix, &XMap::default());
	assert_eq!(settle(&s), State::Ready);
	let im = s.image().unwrap();
	assert_eq!((im.w, im.h, im.rgba.len()), (2, 3, 24));
	assert_eq!(&im.rgba[..8], &[0, 1, 2, 0x80, 10, 11, 12, 0x80]);
	assert_eq!(&im.rgba[20..], &[50, 51, 52, 0x80]);
	assert!(Arc::ptr_eq(&s, &reg.lookup(&n, &ix, &XMap::default())), "asking again gives the same slot");
	assert_eq!(reg.stats(), Stats { names: 1, ready: 1, missing: 0, refused: 0, bytes: 24 });
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn release_drops_the_cpu_copy_and_gives_the_budget_back() {
	let dir = tmp("rel");
	let n = name(0x41);
	let ix = pack(&dir, &[(n, "png", png(1, 1, &[1, 2, 3, 0x80]))]);
	let reg = Registry::new(1 << 20);
	let s = reg.lookup(&n, &ix, &XMap::default());
	assert_eq!(settle(&s), State::Ready);
	let held = s.image().unwrap();
	assert_eq!(reg.stats().bytes, 4);
	s.release();
	assert_eq!((s.state(), s.image().is_none(), reg.stats().bytes), (State::Released, true, 0), "the budget is free at once");
	assert_eq!(held.rgba, vec![1, 2, 3, 0xff], "a handle taken before keeps its pixels");
	s.release();
	assert_eq!(reg.stats().bytes, 0, "a second release changes nothing");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_name_with_no_file_is_missing_at_once_and_starts_no_thread() {
	let reg = Registry::new(1 << 20);
	let s = reg.lookup(&name(0x50), &PackIndex::new(), &XMap::default());
	assert_eq!(s.state(), State::Missing, "no waiting: decided inside lookup");
	assert!(s.image().is_none());
	assert!(!reg.workers_started());
	assert_eq!(reg.stats(), Stats { names: 1, ready: 0, missing: 1, refused: 0, bytes: 0 });
}

#[test]
fn bc7_is_decoded_and_oversize_and_broken_files_are_refused() {
	let dir = tmp("refuse");
	let (bc7, big, bad) = (name(0x60), name(0x70), name(0x80));
	let mut f = dds(4, 4, b"DX10", [0; 4], 0, &[&[0u8; 20 + 16]]);
	f[128..132].copy_from_slice(&98u32.to_le_bytes());
	f[140..144].copy_from_slice(&1u32.to_le_bytes());
	f[144..148].copy_from_slice(&3u32.to_le_bytes());
	let wide = png(MAX_SIDE + 1, 1, &vec![0x80; (MAX_SIDE as usize + 1) * 4]);
	let ix = pack(&dir, &[(bc7, "dds", f), (big, "png", wide), (bad, "png", b"not a png".to_vec())]);
	let reg = Registry::new(1 << 20);
	let tw = XMap::default();
	let (s1, s2, s3) = (reg.lookup(&bc7, &ix, &tw), reg.lookup(&big, &ix, &tw), reg.lookup(&bad, &ix, &tw));
	assert_eq!((settle(&s1), settle(&s2), settle(&s3)), (State::Ready, State::TooBig, State::Broken));
	// The BC7 file (an all-zero block = reserved, declared opaque) is decoded by `bc7dec` to a 4x4 RGBA8 image; the others are refused.
	assert!(s1.image().is_some_and(|i| (i.w, i.h, i.rgba.len()) == (4, 4, 64)) && s2.image().is_none() && s3.image().is_none());
	let st = reg.stats();
	assert_eq!((st.ready, st.refused, st.bytes), (1, 2, 64));
	assert_eq!(reg.errors().len(), 2, "each refusal says why");
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_budget_refuses_images_past_it() {
	let dir = tmp("budget");
	let (a, b, c) = (name(0x90), name(0xa0), name(0xb0));
	let img = png(8, 8, &[0x80; 8 * 8 * 4]); // 256 bytes decoded
	let ix = pack(&dir, &[(a, "png", img.clone()), (b, "png", img.clone()), (c, "png", img)]);
	let tw = XMap::default();
	// Room for one and a half: the first fits, the second is decoded and refused, then the budget counts as full.
	let reg = Registry::new(384);
	let sa = reg.lookup(&a, &ix, &tw);
	assert_eq!(settle(&sa), State::Ready);
	let sb = reg.lookup(&b, &ix, &tw);
	assert_eq!(settle(&sb), State::OverBudget);
	assert!(sb.image().is_none());
	assert_eq!(reg.stats().bytes, 256, "the refused image is not counted");
	// Exactly full: the next file is refused before it is read (its file is gone, still OverBudget and not Broken).
	let reg = Registry::new(256);
	assert_eq!(settle(&reg.lookup(&a, &ix, &tw)), State::Ready);
	std::fs::remove_file(&ix.get(&c).unwrap().path).unwrap();
	assert_eq!(settle(&reg.lookup(&c, &ix, &tw)), State::OverBudget);
	assert_eq!(reg.errors().len(), 1);
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn nothing_runs_without_a_pack() {
	assert!(!enabled_for(false, None), "no textures mod");
	assert!(!enabled_for(false, Some("1")));
	assert!(!enabled_for(true, Some("0")), "RAC_TEXPACK_HUD=0");
	assert!(enabled_for(true, None) && enabled_for(true, Some("1")));
	let reg = Registry::new(1 << 20);
	assert!(!reg.workers_started(), "a new registry has no thread");
	assert_eq!(reg.stats(), Stats::default());
	assert!(reg.sh.queue.lock().unwrap().is_empty() && reg.map.lock().unwrap().is_empty());
	// The disabled answer is one shared Missing slot; with no lookup the process registry does not exist and has nothing to report.
	assert_eq!(disabled_slot().state(), State::Missing);
	assert!(Arc::ptr_eq(&disabled_slot(), &disabled_slot()));
	assert!(REG.get().is_none() && report().is_none());
}

#[test]
fn a_released_name_is_decoded_again_by_rearm() {
	let dir = tmp("rearm");
	let (n, m) = (name(0xc0), name(0xc1));
	let ix = pack(&dir, &[(n, "png", png(1, 1, &[7, 8, 9, 0x80])), (m, "png", png(1, 1, &[1, 1, 1, 0x80]))]);
	let reg = Registry::new(1 << 20);
	let s = reg.lookup(&n, &ix, &XMap::default());
	assert!(!s.rearm(), "a Pending / Ready slot is not re-armed");
	assert_eq!(settle(&s), State::Ready);
	assert!(!s.rearm(), "a Ready slot is not re-armed");
	s.release();
	assert_eq!((s.state(), reg.stats().bytes), (State::Released, 0));
	assert!(s.rearm(), "a released slot goes back to the loaders");
	assert!(!s.rearm(), "once: it is Pending now");
	assert_eq!(settle(&s), State::Ready, "decoded again from the file");
	assert_eq!(s.image().unwrap().rgba, vec![7, 8, 9, 0xff]);
	assert_eq!(reg.stats().bytes, 4, "the budget counts the new copy once");
	// A slot of a name with no file has nothing to decode.
	let none = reg.lookup(&name(0xc2), &ix, &XMap::default());
	assert!(!none.rearm());
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn drop_unused_frees_ready_images_and_lookup_brings_them_back() {
	let dir = tmp("drop");
	let (a, b) = (name(0xd0), name(0xd1));
	let ix = pack(&dir, &[(a, "png", png(2, 2, &[0x80; 16])), (b, "png", png(1, 1, &[5, 6, 7, 0x80]))]);
	let (tw, reg) = (XMap::default(), Registry::new(1 << 20));
	let (sa, sb) = (reg.lookup(&a, &ix, &tw), reg.lookup(&b, &ix, &tw));
	assert_eq!((settle(&sa), settle(&sb)), (State::Ready, State::Ready));
	sb.release(); // taken by a capture: not Ready any more, drop_unused leaves it alone
	assert_eq!(reg.stats().bytes, 16);
	assert_eq!(reg.drop_unused(), 1, "only the untouched Ready slot");
	assert_eq!((sa.state(), sb.state(), reg.stats().bytes), (State::Dropped, State::Released, 0), "its bytes are back in the budget");
	assert!(sa.image().is_none());
	assert_eq!(reg.drop_unused(), 0, "nothing left to drop");
	// Asking for the dropped name again decodes it again, on the same slot.
	let again = reg.lookup(&a, &ix, &tw);
	assert!(Arc::ptr_eq(&again, &sa));
	assert_eq!(settle(&again), State::Ready);
	assert_eq!((again.image().unwrap().rgba.len(), reg.stats().bytes), (16, 16));
	// A dropped slot can also be re-armed by the capture that wants it.
	assert_eq!(reg.drop_unused(), 1);
	assert!(sa.rearm());
	assert_eq!(settle(&sa), State::Ready);
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_header_past_the_limit_is_too_big_before_any_pixel_is_read() {
	let dir = tmp("hdr");
	let n = name(0xe0);
	// A BC3 header for 8192 x 8192 with no pixel data at all (a truncated file): the header alone decides TooBig, not `Broken`.
	let f = dds(8192, 8192, b"DXT5", [0; 4], 0, &[&[]]);
	let ix = pack(&dir, &[(n, "dds", f)]);
	let reg = Registry::new(1 << 20);
	let s = reg.lookup(&n, &ix, &XMap::default());
	assert_eq!(settle(&s), State::TooBig);
	assert!(reg.errors()[0].contains("8192x8192"), "the message names the size: {:?}", reg.errors());
	let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_over_budget_slot_is_rearmed_once_it_fits_the_budget() {
	let dir = tmp("over_rearm");
	let (a, b) = (name(0xd0), name(0xd1));
	let img = png(8, 8, &[0x80; 8 * 8 * 4]); // 256 bytes decoded
	let ix = pack(&dir, &[(a, "png", img.clone()), (b, "png", img)]);
	let tw = XMap::default();
	// Room for one and a half: `a` fits, `b` is refused (OverBudget, 256 bytes wanted).
	let reg = Registry::new(384);
	// One after the other: the registry has two loader threads, so queueing both would let either one take the room first.
	let sa = reg.lookup(&a, &ix, &tw);
	assert_eq!(settle(&sa), State::Ready);
	let sb = reg.lookup(&b, &ix, &tw);
	assert_eq!(settle(&sb), State::OverBudget);
	assert_eq!(reg.stats().refused, 1);
	assert!(!sb.rearm(), "the budget is still full: 256 used + 256 wanted > 384");
	assert_eq!(sb.state(), State::OverBudget);
	assert_eq!(reg.stats().refused, 1, "a refused re-arm changes nothing");
	// `a` gives its bytes back: now `b` fits, so it goes to the loaders again (once), and the refusal is not counted any more.
	sa.release();
	assert_eq!(reg.stats().bytes, 0);
	assert!(sb.rearm(), "room for it now");
	assert!(!sb.rearm(), "once: it is Pending now");
	assert_eq!(settle(&sb), State::Ready);
	assert_eq!((reg.stats().bytes, reg.stats().refused), (256, 0));
	// A lookup re-arms too: `b` is Ready and holds 256; `a` (released before) is decoded again by rearm and refused (512 > 384)...
	assert!(sa.rearm());
	assert_eq!(settle(&sa), State::OverBudget);
	// ...and once `b` gives its bytes back, looking `a` up again brings it back.
	sb.release();
	assert_eq!(settle(&reg.lookup(&a, &ix, &tw)), State::Ready);
	assert_eq!((reg.stats().bytes, reg.stats().refused), (256, 0));
	let _ = std::fs::remove_dir_all(&dir);
}
