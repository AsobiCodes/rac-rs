//! Tests of the BC7 decoder (against texconv's output, stored in `fixtures/`) and of the load rule for BC7 files (`bc7alpha`, `dds::load`).

use super::super::bc7alpha::bc7_may_decode;
use super::super::chain::{assemble, Mip, ReplTex};
use super::super::dds::{self, Fmt, Image};
use super::*;

const BLOCKS: &[u8] = include_bytes!("fixtures/bc7_blocks.bin");
const ODD: &[u8] = include_bytes!("fixtures/bc7_odd.dds");
const ODD_RGBA: &[u8] = include_bytes!("fixtures/bc7_odd_rgba.bin");

#[test]
fn every_fixture_block_matches_texconv() {
	// 80 byte records: the block and the 64 bytes texconv decodes it to (R8G8B8A8_UNORM). Two or three blocks of modes 0, 1, 2, 3, 6, 7
	// (different partitions), mode 4 with every rotation / index selection, mode 5 with every rotation, and a reserved block.
	assert_eq!(BLOCKS.len() % 80, 0);
	let mut modes = [0usize; 9];
	for (n, rec) in BLOCKS.chunks_exact(80).enumerate() {
		let mut px = [0xAAu8; 64];
		decode_block(rec[..16].try_into().unwrap(), &mut px);
		assert_eq!(&px[..], &rec[16..], "fixture block {n}: {:02x?}", &rec[..16]);
		modes[if rec[0] == 0 { 8 } else { rec[0].trailing_zeros() as usize }] += 1;
	}
	assert!(modes.iter().all(|&n| n > 0), "every mode and the reserved block are covered: {modes:?}");
}

#[test]
fn a_cropped_mip_chain_matches_texconv() {
	// 21 x 10 with levels 21x10, 10x5, 5x2: edge blocks are cut to the size.
	let mut at = 148;
	let mut want = 0;
	for (w, h) in [(21u32, 10u32), (10, 5), (5, 2)] {
		let n = (w.div_ceil(4) * h.div_ceil(4)) as usize * 16;
		let got = decode_level(w, h, &ODD[at..at + n]).unwrap();
		assert_eq!(got.len(), (w * h * 4) as usize);
		assert_eq!(&got[..], &ODD_RGBA[want..want + got.len()], "level {w}x{h}");
		at += n;
		want += got.len();
	}
	assert_eq!((at, want), (ODD.len(), ODD_RGBA.len()));
}

#[test]
fn the_reserved_block_is_all_zero_and_short_data_is_an_error() {
	let mut px = [7u8; 64];
	decode_block(&[0u8; 16], &mut px);
	assert_eq!(px, [0u8; 64]);
	assert!(decode_level(8, 8, &[0u8; 63]).is_err());
	assert!(level_is_opaque(8, 8, &[0u8; 63]).is_err());
	assert_eq!(decode_level(8, 8, &[0u8; 64]).unwrap(), vec![0u8; 256]);
	// Any 16 bytes decode without a panic.
	let mut seed = 0x1234_5678_9abc_def0u64;
	for _ in 0..2000 {
		let mut b = [0u8; 16];
		for x in &mut b {
			seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
			*x = (seed >> 56) as u8;
		}
		decode_block(&b, &mut px);
		let _ = block_is_opaque(&b);
	}
}

#[test]
fn opaque_check_agrees_with_the_decode() {
	for rec in BLOCKS.chunks_exact(80) {
		let b: &[u8; 16] = rec[..16].try_into().unwrap();
		assert_eq!(block_is_opaque(b), rec[16..].chunks_exact(4).all(|p| p[3] == 255), "{:02x?}", b);
	}
}

// ---- the load rule ----

struct W {
	v: u128,
	at: u32,
}
impl W {
	fn put(&mut self, v: u32, n: u32) {
		self.v |= ((v as u128) & ((1u128 << n) - 1)) << self.at;
		self.at += n;
	}
}

/// A mode 6 block (RGBA 7.7.7.7 + p-bits): endpoint 0 / 1 colours and 8 bit alphas (the alpha's lowest bit is the endpoint's p-bit),
/// and a 4 bit index per texel.
fn mode6(c0: [u8; 3], c1: [u8; 3], a0: u8, a1: u8, idx: [u8; 16]) -> [u8; 16] {
	let mut w = W { v: 0, at: 0 };
	w.put(1 << 6, 7);
	for ch in 0..3 {
		w.put((c0[ch] >> 1) as u32, 7);
		w.put((c1[ch] >> 1) as u32, 7);
	}
	w.put((a0 >> 1) as u32, 7);
	w.put((a1 >> 1) as u32, 7);
	w.put((a0 & 1) as u32, 1);
	w.put((a1 & 1) as u32, 1);
	for (i, x) in idx.iter().enumerate() {
		w.put(*x as u32, if i == 0 { 3 } else { 4 });
	}
	w.v.to_le_bytes()
}

/// A mode 1 block (RGB only, two subsets): flat colour, every texel index 0.
fn mode1() -> [u8; 16] {
	let mut w = W { v: 0, at: 0 };
	w.put(0b10, 2);
	w.v.to_le_bytes()
}

/// A BC7 DDS file (DXGI 98) with DX10 alpha mode `alpha`; `levels` are the raw block data of each level.
fn file(w: u32, h: u32, alpha: u32, levels: &[&[u8]]) -> Vec<u8> {
	let mut first = vec![0u8; 20];
	first.extend_from_slice(levels[0]);
	let mut all: Vec<&[u8]> = vec![&first];
	all.extend_from_slice(&levels[1..]);
	let mut f = dds::tests::dds(w, h, b"DX10", [0; 4], 0, &all);
	f[128..132].copy_from_slice(&98u32.to_le_bytes());
	f[140..144].copy_from_slice(&1u32.to_le_bytes());
	f[144..148].copy_from_slice(&alpha.to_le_bytes());
	f
}

fn flat(a: u8) -> [u8; 16] {
	mode6([10, 20, 30], [40, 50, 60], a, a, [0; 16])
}

#[test]
fn an_opaque_file_stays_bc7_unchanged_in_every_alpha_mode() {
	let blk = flat(255);
	for mode in [0, 1, 2, 3, 4] {
		let i = dds::load(&file(4, 4, mode, &[&blk])).unwrap();
		assert_eq!((i.fmt, i.levels.len(), &i.levels[0][..]), (Fmt::Bc7, 1, &blk[..]), "alpha mode {mode}");
	}
	// RGB-only blocks are opaque too (the v0.4.0 case: alpha mode unknown, modes 0..3).
	let i = dds::load(&file(4, 4, 0, &[&mode1()])).unwrap();
	assert_eq!((i.fmt, &i.levels[0][..]), (Fmt::Bc7, &mode1()[..]));
}

#[test]
fn ps2_scale_alpha_is_doubled_when_the_alpha_mode_is_not_opaque() {
	for mode in [0, 1, 2, 4] {
		let i = dds::load(&file(4, 4, mode, &[&flat(0x80)])).unwrap();
		assert_eq!((i.fmt, i.levels[0].len()), (Fmt::Rgba8, 64), "alpha mode {mode}");
		assert!(i.levels[0].chunks_exact(4).all(|p| p[3] == 255), "0x80 = opaque on the PS2 scale");
		let i = dds::load(&file(4, 4, mode, &[&flat(0x40)])).unwrap();
		assert!(i.levels[0].chunks_exact(4).all(|p| p[3] == 0x80), "0x40 -> 0x80, alpha mode {mode}");
		let i = dds::load(&file(4, 4, mode, &[&flat(0x00)])).unwrap();
		assert!(i.levels[0].chunks_exact(4).all(|p| p[3] == 0));
		// 254 (what texconv leaves in "opaque" images) is not 255: decoded, and 2 * 254 saturates to 255.
		let i = dds::load(&file(4, 4, mode, &[&flat(254)])).unwrap();
		assert_eq!(i.fmt, Fmt::Rgba8);
		assert!(i.levels[0].chunks_exact(4).all(|p| p[3] == 255));
	}
	// The colour is the decoder's, untouched by the alpha doubling.
	let blk = flat(0x40);
	let i = dds::load(&file(4, 4, 0, &[&blk])).unwrap();
	let mut px = [0u8; 64];
	decode_block(&blk, &mut px);
	assert!(i.levels[0].chunks_exact(4).zip(px.chunks_exact(4)).all(|(a, b)| a[..3] == b[..3]));
}

#[test]
fn alpha_mode_opaque_is_never_touched() {
	let blk = flat(0x40);
	let i = dds::load(&file(4, 4, 3, &[&blk])).unwrap();
	assert_eq!((i.fmt, &i.levels[0][..]), (Fmt::Bc7, &blk[..]));
}

#[test]
fn one_translucent_level_makes_the_whole_chain_rgba8() {
	// 8 x 8: levels 8x8 (4 blocks), 4x4, 2x2, 1x1; only the 2x2 level has alpha 0x40.
	let (o, t) = (flat(255), flat(0x40));
	let l0 = [o, o, o, o].concat();
	let f = file(8, 8, 0, &[&l0, &o, &t, &o]);
	let i = dds::load(&f).unwrap();
	assert_eq!((i.fmt, i.levels.iter().map(|l| l.len()).collect::<Vec<_>>()), (Fmt::Rgba8, vec![256, 64, 16, 4]));
	assert!(i.levels[0].chunks_exact(4).all(|p| p[3] == 255));
	assert!(i.levels[2].chunks_exact(4).all(|p| p[3] == 0x80));
	assert!(i.levels[3].chunks_exact(4).all(|p| p[3] == 255));
	// All levels opaque: stays BC7 with every level.
	let f = file(8, 8, 0, &[&l0, &o, &o, &o]);
	let i = dds::load(&f).unwrap();
	assert_eq!((i.fmt, i.levels.iter().map(|l| l.len()).collect::<Vec<_>>()), (Fmt::Bc7, vec![64, 16, 16, 16]));
}

#[test]
fn texels_outside_the_cropped_size_do_not_count() {
	// A 2 x 2 level is one block; texels 0, 1, 4, 5 (index 0) are visible with alpha 255, the other twelve (index 15) have alpha 0x40.
	let mut idx = [15u8; 16];
	for t in [0, 1, 4, 5] {
		idx[t] = 0;
	}
	let blk = mode6([1, 2, 3], [4, 5, 6], 255, 0x40, idx);
	let i = dds::load(&file(2, 2, 0, &[&blk])).unwrap();
	assert_eq!((i.fmt, &i.levels[0][..]), (Fmt::Bc7, &blk[..]));
	// Seen from a 4 x 4 level the same block has translucent texels.
	assert_eq!(dds::load(&file(4, 4, 0, &[&blk])).unwrap().fmt, Fmt::Rgba8);
}

#[test]
fn short_and_broken_files_are_errors_not_panics() {
	let f = file(8, 8, 0, &[&[flat(0x40), flat(0x40), flat(0x40), flat(0x40)].concat()]);
	assert!(dds::load(&f[..f.len() - 1]).is_err(), "level 0 ends early");
	assert!(dds::load(&f[..147]).is_err(), "DX10 header ends early");
	// A missing later level is dropped, as for every format.
	let (o, t) = (flat(255), flat(0x40));
	let f = file(8, 8, 0, &[&[o, o, o, o].concat(), &t]);
	let i = dds::load(&f[..f.len() - 3]).unwrap();
	assert_eq!((i.fmt, i.levels.len()), (Fmt::Bc7, 1));
	// Garbage block data is a valid BC7 file (every bit pattern decodes), whatever the alpha.
	let junk: Vec<u8> = (0..64u32).map(|i| (i * 37 + 11) as u8).collect();
	assert!(dds::load(&file(8, 8, 0, &[&junk])).is_ok());
	// The decoder rejects a short level directly.
	assert!(bc7alpha_apply_short().is_err());
}

fn bc7alpha_apply_short() -> Result<Image, String> {
	super::super::bc7alpha::apply(Image { fmt: Fmt::Bc7, w: 8, h: 8, levels: vec![vec![0x40; 48]] })
}

#[test]
fn a_device_without_bc_decodes_bc7() {
	// A native BC7 chain (opaque file, alpha 255) decoded to RGBA8, as `repl::for_device` does for BC1..3.
	let blk = flat(255);
	let i = dds::load(&file(4, 4, 0, &[&blk])).unwrap();
	assert_eq!(i.fmt, Fmt::Bc7);
	let c = assemble(&[Some(i)], 1).unwrap();
	assert_eq!(c.fmt, Fmt::Bc7);
	let d = c.to_rgba8().unwrap();
	let mut px = [0u8; 64];
	decode_block(&blk, &mut px);
	assert_eq!((d.fmt, &d.mips[0].data[..]), (Fmt::Rgba8, &px[..]));
	// A size that is not a whole number of blocks cannot stay BC7: assemble decodes it.
	let img = Image { fmt: Fmt::Bc7, w: 5, h: 5, levels: vec![[flat(255); 4].concat()] };
	let c = assemble(&[Some(img)], 1).unwrap();
	assert_eq!((c.fmt, c.mips[0].data.len()), (Fmt::Rgba8, 5 * 5 * 4));
	// A broken chain (too little data) is an error.
	let bad = ReplTex { fmt: Fmt::Bc7, mips: vec![Mip { w: 8, h: 8, data: vec![0u8; 16] }] };
	assert!(bad.to_rgba8().is_err());
}

#[test]
fn the_header_alone_says_whether_a_bc7_file_may_be_decoded() {
	let blk = flat(255);
	for (mode, may) in [(0, true), (1, true), (2, true), (3, false), (4, true)] {
		assert_eq!(bc7_may_decode(&file(4, 4, mode, &[&blk])[..148]), may, "alpha mode {mode}");
	}
	let mut f = file(4, 4, 0, &[&blk]);
	f[128..132].copy_from_slice(&99u32.to_le_bytes());
	assert!(bc7_may_decode(&f[..148]), "DXGI 99 (sRGB) is BC7 too");
	f[128..132].copy_from_slice(&77u32.to_le_bytes());
	assert!(!bc7_may_decode(&f[..148]), "BC3 header");
	assert!(!bc7_may_decode(&f[..100]) && !bc7_may_decode(b"DDS ") && !bc7_may_decode(&[]), "short or foreign data");
	let bc3 = dds::tests::dds(4, 4, b"DXT5", [0; 4], 0, &[&[0u8; 16]]);
	assert!(!bc7_may_decode(&bc3[..128]));
}
