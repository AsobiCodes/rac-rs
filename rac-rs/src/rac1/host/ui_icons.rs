//! Registers the game's own item pictures for the modern menus (v5i): once per level load, from the level's HUD sheets (the same data
//! `hudart.rs` turns into overlay textures; every level's HUD set holds the item sheets 0xea60 + item id and the Items page's sheets
//! [C: format_tests/hud.rs `item_icons_every_level`]). The pictures go into the engine's picture atlas (`rac_engine::ui::images`) under
//! the ids of `modern_menu/icons.rs`; the pause grids and the vendor list draw them as `Prim::Image`. Works for the PAL and the USA disc:
//! the sheets are level data, not code.
//! Texture packs (#66): these pictures do not go through the GPU capture, whose run-time lookup replaces the classic menus' icons, so
//! with a textures mod on each sprite's PCSX2 names (`Sprite::name` / `name_h`: as PSMT8 and as PSMT8H, the two ways the game keeps
//! HUD banks in GS memory, so the names PCSX2 dumps) are looked up here at the level load (`texpack::front_hd::apply`, the lookup of
//! the front end's own pictures; the first name the pack has wins) and the pack's image replaces the disc's; a pack image is halved
//! down to `PACK_SIDE` so an HD pack's ~110 pictures fit the atlas. No pack: no names, nothing looked up.
use crate::hud::Hud;
use crate::level::Texture;
use crate::modern_menu::icons;
use crate::texpack::name::TexName;
use rac_engine::ui::images::{self, ImageSet, Pic};

/// Largest side of a pack's picture in the menu atlas (the biggest menu cell is ~120 design px: 256 is sharp at 4k).
const PACK_SIDE: usize = 256;

/// The pictures `register` would register: (id, width, height, RGBA bytes), a texture pack's image where it has one.
pub(super) fn pictures(h: &Hud) -> Vec<(u32, u32, u32, Vec<u8>)> {
	pictures_with(h, |tex, names| {
		crate::texpack::front_hd::apply(tex, names);
	})
}

/// `pictures` with the pack lookup handed in: `swap` replaces `tex[i]` for the `(i, name)` it has an image for.
fn pictures_with(h: &Hud, swap: impl FnOnce(&mut [Texture], &[(usize, TexName)])) -> Vec<(u32, u32, u32, Vec<u8>)> {
	let (mut ids, mut tex, mut names) = (Vec::new(), Vec::new(), Vec::new());
	let mut take = |sheet: u16, frame: usize, id: u32| {
		let Some(i) = h.icons.iter().find(|i| i.id == sheet).filter(|i| frame < i.count) else { return };
		let Some(sp) = h.frames.get(i.first + frame).and_then(|f| f.as_ref()) else { return };
		names.extend([sp.name, sp.name_h].into_iter().flatten().map(|n| (tex.len(), n)));
		ids.push(id);
		tex.push(Texture { w: sp.tex.w, h: sp.tex.h, rgba: sp.tex.rgba.clone() });
	};
	for item in 0..crate::inventory::GADGETS as u8 {
		for f in icons::FRAMES {
			take(0xea60 + item as u16, f as usize, icons::item(item, f));
		}
	}
	for (j, e) in crate::frontend::ITEM_GRID.iter().enumerate() {
		take(e.0, e.1 as usize, icons::grid(j));
	}
	if !names.is_empty() {
		swap(&mut tex, &names);
	}
	ids.into_iter()
		.zip(tex)
		.map(|(id, t)| {
			let t = shrink(t, PACK_SIDE);
			(id, t.w as u32, t.h as u32, t.rgba.into_iter().flatten().collect())
		})
		.collect()
}

/// `t` halved (2 x 2 box, odd last row / column dropped) until its larger side is at most `side`; the disc's pictures are smaller and
/// come back as they are.
fn shrink(mut t: Texture, side: usize) -> Texture {
	while t.w.max(t.h) > side && t.w >= 2 && t.h >= 2 {
		let (w, h) = (t.w / 2, t.h / 2);
		let mut rgba = Vec::with_capacity(w * h);
		for y in 0..h {
			for x in 0..w {
				let px = [(2 * x, 2 * y), (2 * x + 1, 2 * y), (2 * x, 2 * y + 1), (2 * x + 1, 2 * y + 1)].map(|(sx, sy)| t.rgba[sy * t.w + sx]);
				rgba.push(std::array::from_fn(|c| ((px.iter().map(|p| p[c] as u32).sum::<u32>() + 2) / 4) as u8));
			}
		}
		t = Texture { w, h, rgba };
	}
	t
}

/// Packs the level's item pictures into the UI atlas and registers it (nothing is registered when the level holds none).
pub(super) fn register(h: &Hud) {
	let owned = pictures(h);
	let pics: Vec<Pic> = owned.iter().map(|(id, w, h, b)| Pic { id: *id, w: *w, h: *h, rgba: b }).collect();
	let set = ImageSet::pack(&pics);
	if !set.is_empty() {
		images::set(set);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::hud::{Icon, Sprite};

	fn solid(w: usize, h: usize, c: [u8; 4]) -> Texture {
		Texture { w, h, rgba: vec![c; w * h] }
	}

	fn name(k: u64) -> TexName {
		TexName::palette(0x1000 + k, 0x2000 + k, 0x4008)
	}

	/// Item 0's sheet (0xea60) with frames 0..4: frames 0, 2, 3 and 4 named (a pack is on: PSMT8 name `name(f)`, PSMT8H name
	/// `name(f + 10)`), frame 1 not.
	fn hud() -> Hud {
		let frames = (0..5u64)
			.map(|f| Some(Sprite { tex: solid(32, 32, [f as u8, 0, 0, 255]), name: (f != 1).then(|| name(f)), name_h: (f != 1).then(|| name(f + 10)) }))
			.collect();
		Hud { icons: vec![Icon { id: 0xea60, first: 0, count: 5 }], frames }
	}

	fn find(v: &[(u32, u32, u32, Vec<u8>)], id: u32) -> (u32, u32, [u8; 4]) {
		let p = v.iter().find(|p| p.0 == id).unwrap();
		(p.1, p.2, p.3[..4].try_into().unwrap())
	}

	/// #66: the pack's image replaces the sprite whose PCSX2 name (PSMT8 or PSMT8H) it has (an HD one halved to `PACK_SIDE`); a sprite
	/// without names or whose names the pack lacks keeps the disc's picture. Frame 3 is not one the menus use.
	#[test]
	fn a_pack_image_replaces_the_named_item_picture() {
		let v = pictures_with(&hud(), |tex, names| {
			let asked: Vec<(usize, TexName)> = names.to_vec();
			assert_eq!(asked, [(0, name(0)), (0, name(10)), (2, name(2)), (2, name(12)), (3, name(4)), (3, name(14))], "both names of every named frame");
			for (i, n) in names {
				if *n == name(0) {
					tex[*i] = solid(1024, 512, [9, 200, 9, 255]);
				} else if *n == name(14) {
					tex[*i] = solid(64, 64, [7, 7, 250, 255]);
				}
			}
		});
		assert_eq!(find(&v, icons::item(0, 0)), (256, 128, [9, 200, 9, 255]), "the pack's image, halved twice");
		assert_eq!(find(&v, icons::item(0, 1)), (32, 32, [1, 0, 0, 255]), "no name: the disc's");
		assert_eq!(find(&v, icons::item(0, 2)), (32, 32, [2, 0, 0, 255]), "not in the pack: the disc's");
		assert_eq!(find(&v, icons::item(0, 4)), (64, 64, [7, 7, 250, 255]), "found under its PSMT8H name");
	}

	/// No pack: the sprites carry no names and the lookup is never asked.
	#[test]
	fn no_names_no_lookup() {
		let mut h = hud();
		h.frames.iter_mut().flatten().for_each(|s| (s.name, s.name_h) = (None, None));
		let v = pictures_with(&h, |_, _| panic!("asked without a pack"));
		assert_eq!(v.len(), 4);
		assert_eq!(find(&v, icons::item(0, 0)), (32, 32, [0, 0, 0, 255]));
	}

	#[test]
	fn shrink_averages_2x2_and_leaves_small_pictures() {
		let t = Texture { w: 4, h: 2, rgba: vec![[0, 0, 0, 0], [4, 8, 12, 255], [1, 1, 1, 1], [1, 1, 1, 1], [0, 0, 0, 0], [0, 0, 0, 255], [1, 1, 1, 1], [1, 1, 1, 1]] };
		let s = shrink(t, 2);
		assert_eq!((s.w, s.h), (2, 1));
		assert_eq!(s.rgba, vec![[1, 2, 3, 128], [1, 1, 1, 1]]);
		let small = shrink(solid(64, 32, [5, 5, 5, 5]), PACK_SIDE);
		assert_eq!((small.w, small.h), (64, 32));
	}
}
