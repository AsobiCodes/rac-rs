//! The modern planet map names every planet on both discs (lane rows, 2026-11-05): `ship_marks::name_text` gives the text id of each
//! level 0..=18 (0x4ecc + id; Veldin's two ids 0x4ede); each must be a real name in every language of each disc present, never empty
//! and never the game's "????" (0x4f54, shown for a planet the player has not visited). Without a disc the test returns early.

use crate::frontend::Text;
use crate::modern_menu::ship_marks::name_text;
use std::path::Path;

const PAL: [&str; 2] = [concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"), concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso")];
const USA: [&str; 2] = [
	concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (USA) (En,Fr,De,Es,It)/Ratchet & Clank (USA) (En,Fr,De,Es,It).iso"),
	concat!(env!("CARGO_MANIFEST_DIR"), "/../../../../Ratchet & Clank (USA) (En,Fr,De,Es,It)/Ratchet & Clank (USA) (En,Fr,De,Es,It).iso"),
];

#[test]
fn every_planet_has_a_name_on_every_disc() {
	for (disc, isos) in [("PAL", PAL), ("USA", USA)] {
		let Some(iso) = isos.iter().find(|p| Path::new(p).is_file()) else { continue };
		let mut d = crate::iso::Disc::open(Path::new(iso)).unwrap();
		for lang in 0..5 {
			let text = Text::load(&mut d, lang).unwrap();
			let unknown = text.get(0x4f54).to_string();
			for id in 0..=18u32 {
				let t = name_text(id).unwrap_or_else(|| panic!("{disc}: no name text for level {id}"));
				let name = text.get(t);
				assert!(!name.trim().is_empty(), "{disc} lang {lang}: level {id} (text {t:#x}) has no name");
				assert!(name != unknown && !name.contains("????"), "{disc} lang {lang}: level {id} (text {t:#x}) reads {name:?}");
			}
		}
	}
}
