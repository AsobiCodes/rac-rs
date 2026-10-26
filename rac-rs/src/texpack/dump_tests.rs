//! Gated dump of the PCSX2 names of every level table texture of a disc (lane tex2, stage A: the world-class name set per level and the
//! 8 x 8 colour signature that pairs a PAL texture with its USA twin). Runs only with RAC_TEXDUMP_ISO=<iso> and RAC_TEXDUMP_OUT=<tsv>:
//!   `level  group  index  mip  w  h  stem  sig`  (names and signatures only; no pixels).

use super::hash::Level as Lv;
use super::mods::MipNames;
use super::{names, source};
use crate::level::Level;
use std::fmt::Write as _;
use std::path::Path;

/// The 8 x 8 mean-RGB grid (192 bytes) of an index image under a CT32 CSM1 palette, the same grid `gsname::signature` makes from local memory.
fn table_signature(l: &Lv, pal: &[u8]) -> Vec<u8> {
	let mut out = vec![0u8; 192];
	let (tw, th) = (l.w as i32, l.h as i32);
	for cy in 0..8i32 {
		for cx in 0..8i32 {
			let (x0, y0) = (cx * tw / 8, cy * th / 8);
			let (x1, y1) = (((cx + 1) * tw / 8).max(x0 + 1).min(tw), ((cy + 1) * th / 8).max(y0 + 1).min(th));
			let (mut sum, mut n) = ([0u32; 3], 0u32);
			for y in y0..y1 {
				for x in x0..x1 {
					let i = l.px[y as usize * l.w + x as usize] as usize;
					let j = (i & !0x18) | ((i & 8) << 1) | ((i & 0x10) >> 1);
					for (k, s) in sum.iter_mut().enumerate() {
						*s += pal[j * 4 + k] as u32;
					}
					n += 1;
				}
			}
			for k in 0..3 {
				out[(cy * 8 + cx) as usize * 3 + k] = (sum[k] / n.max(1)) as u8;
			}
		}
	}
	out
}

#[test]
fn dump_level_tables() {
	let (Some(iso), Some(out)) = (std::env::var_os("RAC_TEXDUMP_ISO"), std::env::var_os("RAC_TEXDUMP_OUT")) else { return };
	let mut d = crate::iso::Disc::open(Path::new(&iso)).unwrap();
	let levels = crate::toc::read_levels(&mut d).unwrap();
	eprintln!("dump: {} levels on {}", levels.len(), Path::new(&iso).display());
	let mut s = String::from("level\tgroup\tindex\tmip\tw\th\tstem\tsig\n");
	for (i, lv) in levels.iter().enumerate() {
		let l = Level::load(&mut d, lv).unwrap();
		let (srcs, skipped) = source::level_sources(&l).unwrap();
		let all = names::level_names(&srcs, MipNames::Single);
		let distinct: std::collections::HashSet<_> = all.iter().map(|n| n.name).collect();
		eprintln!("dump: level {i}: {} textures, {skipped} skipped, {} names, {} distinct", srcs.len(), all.len(), distinct.len());
		for t in &srcs {
			for ln in names::texture_names(t, MipNames::Single) {
				let lvl = &t.levels[ln.level as usize];
				let _ = write!(s, "{i}\t{}\t{}\t{}\t{}\t{}\t{}\t", t.group as usize, t.index, ln.level, lvl.w, lvl.h, ln.name.stem(true));
				for b in table_signature(lvl, t.pal) {
					let _ = write!(s, "{b:02x}");
				}
				s.push('\n');
			}
		}
	}
	std::fs::write(&out, s).unwrap();
}

/// Diagnostic (lane tex2): are the level's table textures, as 256-byte GS blocks, present anywhere in the GS local memory of a savestate?
/// RAC_TEXDUMP_ISO + RAC_TEXDUMP_STATE=<state dir> + RAC_TEXDUMP_LEVEL=<n>; prints counts only.
#[test]
fn table_blocks_in_state_vram() {
	let (Some(iso), Some(state), Some(lv)) = (std::env::var_os("RAC_TEXDUMP_ISO"), std::env::var("RAC_TEXDUMP_STATE").ok(), std::env::var("RAC_TEXDUMP_LEVEL").ok().and_then(|v| v.parse::<usize>().ok())) else { return };
	let Some(vram) = ee_oracle::gs::Gs::load_vram(&format!("{state}/GS.bin")) else { panic!("no GS.bin") };
	let mut d = crate::iso::Disc::open(Path::new(&iso)).unwrap();
	let levels = crate::toc::read_levels(&mut d).unwrap();
	let l = Level::load(&mut d, &levels[lv]).unwrap();
	let (srcs, _) = source::level_sources(&l).unwrap();
	let mut blocks: std::collections::HashMap<&[u8], Vec<usize>> = std::collections::HashMap::new();
	for (i, b) in vram.chunks_exact(256).enumerate() {
		blocks.entry(b).or_default().push(i);
	}
	let (mut tex, mut first, mut all, mut seq) = (0, 0, 0, 0);
	for t in &srcs {
		let l0 = &t.levels[0];
		if l0.w < 16 || l0.h < 16 || l0.w % 16 != 0 || l0.h % 16 != 0 {
			continue;
		}
		let mut s = Vec::new();
		super::hash::t8_level_stream(*l0, &mut s);
		let bl: Vec<&[u8]> = s.chunks_exact(256).collect();
		tex += 1;
		let Some(a0) = blocks.get(bl[0]) else { continue };
		first += 1;
		if bl.iter().all(|b| blocks.contains_key(*b)) {
			all += 1;
		}
		// Same blocks at consecutive block numbers (a linear upload) from some start?
		if a0.iter().any(|&a| bl.iter().enumerate().all(|(k, b)| vram.get((a + k) * 256..(a + k + 1) * 256) == Some(*b))) {
			seq += 1;
		}
	}
	eprintln!("blocks: level {lv}: {tex} textures >= 16x16 level 0; first block in vram {first}; every block in vram {all}; stored as consecutive blocks {seq}");
}

/// Diagnostic (lane tex2): the raw block streams a gsfull run dumped (RAC_TEXNAMES_RAW, `stem<TAB>len<LF>bytes<LF>` records) against the blocks of the
/// level's table textures (every mip level). RAC_TEXDUMP_ISO + RAC_TEXDUMP_RAW=<file> + RAC_TEXDUMP_LEVEL=<n>; prints counts only.
#[test]
fn raw_dump_vs_table_blocks() {
	let (Some(iso), Some(raw), Some(lv)) = (std::env::var_os("RAC_TEXDUMP_ISO"), std::env::var_os("RAC_TEXDUMP_RAW"), std::env::var("RAC_TEXDUMP_LEVEL").ok().and_then(|v| v.parse::<usize>().ok())) else { return };
	let mut d = crate::iso::Disc::open(Path::new(&iso)).unwrap();
	let levels = crate::toc::read_levels(&mut d).unwrap();
	let l = Level::load(&mut d, &levels[lv]).unwrap();
	let (srcs, _) = source::level_sources(&l).unwrap();
	let mut set: std::collections::HashSet<Vec<u8>> = std::collections::HashSet::new();
	for t in &srcs {
		for lvl in &t.levels {
			if lvl.w >= 16 && lvl.h >= 16 && lvl.w % 16 == 0 && lvl.h % 16 == 0 {
				let mut s = Vec::new();
				super::hash::t8_level_stream(*lvl, &mut s);
				set.extend(s.chunks_exact(256).map(|b| b.to_vec()));
			}
		}
	}
	let data = std::fs::read(raw).unwrap();
	let (mut at, mut n, mut full, mut part, mut none, mut blocks, mut hit) = (0usize, 0, 0, 0, 0, 0usize, 0usize);
	while at < data.len() {
		let nl = data[at..].iter().position(|&b| b == b'\n').unwrap();
		let head = std::str::from_utf8(&data[at..at + nl]).unwrap();
		let len: usize = head.split('\t').nth(1).unwrap().parse().unwrap();
		let body = &data[at + nl + 1..at + nl + 1 + len];
		at += nl + 1 + len + 1;
		let h = body.chunks_exact(256).filter(|b| set.contains(*b)).count();
		let t = len / 256;
		(n, blocks, hit) = (n + 1, blocks + t, hit + h);
		if h == t {
			full += 1;
		} else if h == 0 {
			none += 1;
		} else {
			part += 1;
		}
	}
	eprintln!("rawcmp: level {lv}: {n} dumped textures, {blocks} blocks, {hit} in the level's table blocks; textures all-blocks {full}, some {part}, none {none}; table blocks {}", set.len());
}
