//! The fur table against both discs (review A12): the hero's body texture is found by its content hash (`hero_fur.rs`), and the
//! hash was only ever measured on the PAL disc. This decodes class 0 (Ratchet) and its textures from each disc present, on a few
//! levels, and checks that one of its textures has a row. A failure prints every hero texture's size and hash, which is the
//! `RAC_FUR_DUMP` list without running the game. Without a disc the test returns early.

use crate::hero_fur::spec;
use crate::level::Level;
use std::path::Path;

// Next to the dev tree (rac-rs/crates/x -> ../../../) or, from a lane folder one level deeper, in the project root.
const PAL: [&str; 2] = [concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"), concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso")];
const USA: [&str; 2] = [
	concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (USA) (En,Fr,De,Es,It)/Ratchet & Clank (USA) (En,Fr,De,Es,It).iso"),
	concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../Ratchet & Clank (USA) (En,Fr,De,Es,It)/Ratchet & Clank (USA) (En,Fr,De,Es,It).iso"),
];

/// (width, height, hash) of every texture Ratchet's faces use on `level`.
fn hero_textures(d: &mut crate::iso::Disc, level: usize) -> Vec<(usize, usize, u64)> {
	let levels = crate::toc::read_levels(d).unwrap();
	let l = Level::load(d, &levels[level]).unwrap();
	let hero = crate::mobies::moby_classes(&l).unwrap().remove(&0).expect("class 0 is Ratchet");
	let textures = l.all_textures().unwrap();
	let keys: std::collections::BTreeSet<u32> = hero.mesh.faces.iter().map(|(k, _)| k & crate::mobies::KEY_MASK).collect();
	keys.iter().filter_map(|k| textures.get(k)).map(|t| (t.w, t.h, crate::furspec::texhash(t.w, t.h, &t.rgba))).collect()
}

#[test]
fn the_body_texture_has_a_fur_row_on_every_disc() {
	let rows = spec().unwrap().rows;
	for (name, isos) in [("PAL", PAL), ("USA", USA)] {
		let Some(iso) = isos.iter().find(|p| Path::new(p).is_file()) else { continue };
		let mut d = crate::iso::Disc::open(Path::new(iso)).unwrap();
		for level in [0, 1, 3] {
			let found = hero_textures(&mut d, level);
			let hit = found.iter().any(|&(_, _, h)| rows.iter().any(|r| r.hash == h));
			assert!(hit, "{name} level {level}: no fur row matches; hero textures (w, h, hash) {:x?}", found);
		}
	}
}
