//! Disc tests of formats/hud.rs (engine split stage E2, 2026-10-26): they read the R&C1 disc through `toc`, so they live in
//! the R&C1 pack; moved unchanged from the formats file's test modules.
#![allow(unused_imports)]
mod tests {
	use crate::hud::*;
	use crate::render::{png, Image};
	use crate::{iso, toc};
	use crate::bytes::span;
	use crate::bytes::u16_at;
	use crate::bytes::u32_at;
	use crate::bytes::R;
	use crate::iso::Disc;
	use crate::level::Level;
	use crate::level::Texture;
	use crate::wad;

	// UI-TEXT-1 test (1): bytes / 128, and the on-screen value times the 0x80 texel (clamped).
	const DISC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso");

	const OUT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../target/test_out/hud");

	fn flat(t: &Texture) -> Image {
		let mut rgb = Vec::new();
		for (i, p) in t.rgba.iter().enumerate() {
			let (x, y) = (i % t.w, i / t.w);
			let bg = if (x / 4 + y / 4) % 2 == 0 { 60.0 } else { 100.0 };
			let a = p[3] as f32 / 255.0;
			for k in 0..3 {
				rgb.push((p[k] as f32 * a + bg * (1.0 - a)) as u8);
			}
		}
		Image { w: t.w, h: t.h, rgb }
	}

	/// WPN-6.7e4 (8): the Novalis level HUD bank holds the Morph-o-Ray meter sheet 0x7533 (tank, bubble).
	#[test]
	fn novalis_hud_has_morph_sheet() {
		let mut disc = iso::Disc::open(std::path::Path::new(DISC)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let info = levels.iter().find(|i| toc::LEVEL_NAMES[i.id] == "Novalis").unwrap();
		let hud = hud_sprites(&Level::load(&mut disc, info).unwrap(), false).unwrap();
		let icon = hud.icons.iter().find(|i| i.id == 0x7533).expect("sheet 0x7533");
		println!("0x7533: {} frames", icon.count);
		assert!(icon.count >= 2);
		for k in 0..2 {
			let t = &hud.frames[icon.first + k].as_ref().unwrap().tex;
			println!("frame {k}: {}x{}", t.w, t.h);
		}
	}

	/// v5i: the item icons (sheet 0xea60 + item id, frames 0 = blue / green menu icon, 1 = worn, 2 = shop ammo, 3 = ammo HUD, 4 = gold) and
	/// the Items grid sheets are in EVERY level's HUD set: the modern pause grids and the vendor list draw them from there.
	#[test]
	fn item_icons_every_level() {
		let mut disc = iso::Disc::open(std::path::Path::new(DISC)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let mut sheet = Vec::new();
		for info in &levels {
			let hud = hud_sprites(&Level::load(&mut disc, info).unwrap(), false).unwrap();
			let mut have = Vec::new();
			for id in 0..crate::inventory::GADGETS as u16 {
				let Some(i) = hud.icons.iter().find(|i| i.id == 0xea60 + id) else { continue };
				let f0 = hud.frames.get(i.first).and_then(|f| f.as_ref()).map(|s| (s.tex.w, s.tex.h));
				have.push((id, i.count, f0));
				if info.id == 0 {
					if let Some(s) = hud.frames.get(i.first).and_then(|f| f.as_ref()) {
						sheet.push((id, flat(&s.tex)));
					}
				}
			}
			let grid: Vec<_> = [0xea7eu16, 0xeac9, 0xeac8].iter().map(|&g| hud.icons.iter().find(|i| i.id == g).map(|i| i.count)).collect();
			println!("level {}: {} item sheets {:?}; items grid sheets {:?}", info.id, have.len(), have, grid);
		}
		std::fs::create_dir_all(OUT).unwrap();
		for (id, img) in sheet {
			std::fs::write(format!("{OUT}/icon_{id:02}.png"), png(&img)).unwrap();
		}
	}

	#[test]
	fn fonts_decode() {
		let mut disc = iso::Disc::open(std::path::Path::new(DISC)).unwrap();
		let levels = toc::read_levels(&mut disc).unwrap();
		let glyphs = toc::font_glyph_tables(&mut disc).unwrap();
		for (n, g) in glyphs.iter().enumerate() {
			for d in 0..10usize {
				let e = g[0x30 + d];
				assert_eq!((e.u, e.v), (d as u16 * 16, 64), "font {} digit {d}", n + 1);
			}
		}
		assert_eq!(glyphs[0][0x30].adv, 13);
		for info in &levels {
			let l = Level::load(&mut disc, info).unwrap();
			let fonts = level_fonts(&l, &glyphs).unwrap();
			assert_eq!(fonts.len(), 3);
			if info.id == 0 {
				std::fs::create_dir_all(OUT).unwrap();
				for (i, f) in fonts.iter().enumerate() {
					std::fs::write(format!("{OUT}/font{}.png", i + 1), png(&flat(&f.tex))).unwrap();
				}
			}
		}
	}
}
