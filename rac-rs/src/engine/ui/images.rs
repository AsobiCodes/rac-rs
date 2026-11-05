//! Pictures for the modern UI (v5i): a game pack hands over small RGBA pictures under its own ids (`set`), the UI packs them into ONE
//! atlas that the GPU uploads when the set changes (`gpu.rs`, drawn through `Prim::Image`). Game-neutral: the engine never names
//! a game or an id scheme. The twin of the achievement icon atlas (`achievements/icons.rs`) for the menus. A tile is stored with one
//! extruded texel around it, so the linear filter never reads a neighbour; a picture drawn at the window scale looks smooth.
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

/// Atlas widths in pixels, tried in order (the height grows with the pictures): the narrowest one that holds every picture within
/// `MAX_H` rows is used, so the usual small set stays 512 wide and a set with bigger pictures (a texture pack's) gets a wider atlas.
const WIDTHS: [u32; 4] = [512, 1024, 2048, 4096];
/// Most rows of the atlas (under every GPU's texture limit). A picture that does not fit even the widest atlas is left out: the screen
/// draws its fallback shape for it.
const MAX_H: u32 = 4096;

/// One picture to pack: id, size and straight RGBA bytes (`w * h * 4`).
pub struct Pic<'a> {
	pub id: u32,
	pub w: u32,
	pub h: u32,
	pub rgba: &'a [u8],
}

/// The packed atlas: its pixels and where every id sits.
#[derive(Debug, Default)]
pub struct ImageSet {
	pub w: u32,
	pub h: u32,
	pub rgba: Vec<u8>,
	/// id -> (x, y, w, h) in atlas pixels.
	tiles: HashMap<u32, [u32; 4]>,
}

/// Where each picture of `order` (tallest first) goes in an atlas `w` wide: (picture, x, y) and the atlas height. A picture wider than
/// the atlas or past `MAX_H` rows is not placed; a shorter one later may still fit the current shelf.
fn shelf<'a, 'b>(order: &[&'b Pic<'a>], w: u32) -> (Vec<(&'b Pic<'a>, u32, u32)>, u32) {
	let (mut x, mut y, mut row_h) = (0u32, 0u32, 0u32);
	let mut place = Vec::new();
	for &p in order {
		if p.w + 2 > w {
			continue;
		}
		let (nx, ny, nh) = if x + p.w + 2 > w { (0, y + row_h, 0) } else { (x, y, row_h) };
		if ny + p.h + 2 > MAX_H {
			continue;
		}
		(x, y, row_h) = (nx, ny, nh);
		place.push((p, x + 1, y + 1));
		x += p.w + 2;
		row_h = row_h.max(p.h + 2);
	}
	(place, (y + row_h).max(1))
}

impl ImageSet {
	/// Shelf-packs `pics` (tallest first) into the narrowest atlas of `WIDTHS` that holds them all within `MAX_H` rows (else the widest,
	/// without the pictures past its last row, printed once). A picture with a wrong byte count, a zero side or too big for the widest
	/// atlas is left out.
	pub fn pack(pics: &[Pic]) -> ImageSet {
		let widest = WIDTHS[WIDTHS.len() - 1];
		let mut order: Vec<&Pic> = pics.iter().filter(|p| p.w > 0 && p.h > 0 && p.w <= widest - 2 && p.h <= MAX_H - 2 && p.rgba.len() as u64 == p.w as u64 * p.h as u64 * 4).collect();
		order.sort_by_key(|p| (std::cmp::Reverse(p.h), p.id));
		let mut fit = WIDTHS.iter().map(|&w| (w, shelf(&order, w)));
		let (w, (place, h)) = fit.find(|(_, (place, _))| place.len() == order.len()).unwrap_or_else(|| (widest, shelf(&order, widest)));
		if place.len() < order.len() {
			eprintln!("ui pictures: {} picture(s) left out (the atlas is full at {w} x {MAX_H})", order.len() - place.len());
		}
		let mut set = ImageSet { w, h, rgba: vec![0; w as usize * h as usize * 4], tiles: HashMap::new() };
		for (p, tx, ty) in place {
			set.blit(p, tx, ty);
			set.tiles.insert(p.id, [tx, ty, p.w, p.h]);
		}
		set
	}

	/// Copies a picture to (tx, ty) and extrudes its edge texels one pixel outwards.
	fn blit(&mut self, p: &Pic, tx: u32, ty: u32) {
		for y in -1i64..=p.h as i64 {
			for x in -1i64..=p.w as i64 {
				let (sx, sy) = (x.clamp(0, p.w as i64 - 1) as u32, y.clamp(0, p.h as i64 - 1) as u32);
				let s = ((sy * p.w + sx) * 4) as usize;
				let d = ((((ty as i64 + y) as u32) * self.w + (tx as i64 + x) as u32) * 4) as usize;
				self.rgba[d..d + 4].copy_from_slice(&p.rgba[s..s + 4]);
			}
		}
	}

	/// The picture's UV rectangle (u0, v0, u1, v1) in the atlas.
	pub fn uv(&self, id: u32) -> Option<[f32; 4]> {
		let [x, y, w, h] = *self.tiles.get(&id)?;
		let (aw, ah) = (self.w as f32, self.h as f32);
		Some([x as f32 / aw, y as f32 / ah, (x + w) as f32 / aw, (y + h) as f32 / ah])
	}

	/// The picture's own size in pixels.
	pub fn size_of(&self, id: u32) -> Option<(u32, u32)> {
		self.tiles.get(&id).map(|t| (t[2], t[3]))
	}

	pub fn len(&self) -> usize {
		self.tiles.len()
	}

	pub fn is_empty(&self) -> bool {
		self.tiles.is_empty()
	}
}

/// The registered set and its generation (bumped by every `set`; the GPU re-uploads when it changed).
static SET: RwLock<Option<(u64, Arc<ImageSet>)>> = RwLock::new(None);

/// Registers the set the pack's pictures are drawn from (a level load decodes it again: the same pictures, so nothing visibly changes).
pub fn set(s: ImageSet) {
	let mut g = SET.write().unwrap_or_else(|e| e.into_inner());
	let generation = g.as_ref().map_or(1, |(n, _)| n + 1);
	*g = Some((generation, Arc::new(s)));
}

/// The registered set with its generation.
pub fn current() -> Option<(u64, Arc<ImageSet>)> {
	SET.read().unwrap_or_else(|e| e.into_inner()).clone()
}

/// Is picture `id` registered? (A screen draws its fallback shape when not.)
pub fn has(id: u32) -> bool {
	SET.read().unwrap_or_else(|e| e.into_inner()).as_ref().is_some_and(|(_, s)| s.tiles.contains_key(&id))
}

/// The UV rectangle of a registered picture.
pub fn uv(id: u32) -> Option<[f32; 4]> {
	SET.read().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|(_, s)| s.uv(id))
}

/// A picture's own size in pixels (the aspect a screen draws it at).
pub fn size_of(id: u32) -> Option<(u32, u32)> {
	SET.read().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|(_, s)| s.size_of(id))
}

#[cfg(test)]
mod tests {
	use super::*;

	type Owned = (u32, u32, u32, Vec<u8>);

	fn pic(id: u32, w: u32, h: u32, c: [u8; 4]) -> Owned {
		(id, w, h, (0..w * h).flat_map(|_| c).collect())
	}

	fn pack(v: &[Owned]) -> ImageSet {
		let pics: Vec<Pic> = v.iter().map(|(id, w, h, b)| Pic { id: *id, w: *w, h: *h, rgba: b }).collect();
		ImageSet::pack(&pics)
	}

	#[test]
	fn tiles_do_not_overlap_and_keep_their_pixels() {
		let v: Vec<Owned> = (0..40u32).map(|i| pic(i, 30 + i % 7, 20 + i % 5, [i as u8, 2 * i as u8, 9, 255])).collect();
		let s = pack(&v);
		assert_eq!(s.len(), 40);
		let mut seen = vec![0u8; (s.w * s.h) as usize];
		for (id, w, h, _) in &v {
			let [x, y, tw, th] = s.tiles[id];
			assert_eq!((tw, th), (*w, *h));
			for yy in y..y + h {
				for xx in x..x + w {
					assert_eq!(seen[(yy * s.w + xx) as usize], 0, "tile {id} overlaps");
					seen[(yy * s.w + xx) as usize] = 1;
					let p = ((yy * s.w + xx) * 4) as usize;
					assert_eq!(&s.rgba[p..p + 4], &[*id as u8, 2 * *id as u8, 9, 255]);
				}
			}
		}
	}

	#[test]
	fn the_gutter_repeats_the_edge_texel() {
		let mut b = vec![0u8; 3 * 2 * 4];
		for (i, px) in b.chunks_mut(4).enumerate() {
			px.copy_from_slice(&[i as u8 * 10, 0, 0, 255]);
		}
		let s = pack(&[(7, 3, 2, b)]);
		let [x, y, ..] = s.tiles[&7];
		let px = |xx: u32, yy: u32| s.rgba[((yy * s.w + xx) * 4) as usize];
		assert_eq!(px(x - 1, y), px(x, y), "left gutter");
		assert_eq!(px(x + 3, y + 1), px(x + 2, y + 1), "right gutter");
		assert_eq!(px(x, y - 1), px(x, y), "top gutter");
		assert_eq!(px(x + 1, y + 2), px(x + 1, y + 1), "bottom gutter");
	}

	/// The usual small set stays 512 wide; bigger pictures (a texture pack's, #66) get the narrowest wider atlas that holds them all;
	/// a set too big for the widest atlas keeps within `MAX_H` rows and leaves the rest out.
	#[test]
	fn the_atlas_widens_for_big_pictures_and_never_passes_its_height() {
		let small: Vec<Owned> = (0..113u32).map(|i| pic(i, 64, 64, [1, 2, 3, 255])).collect();
		let s = pack(&small);
		assert_eq!((s.w, s.len()), (512, 113));
		let big: Vec<Owned> = (0..113u32).map(|i| pic(i, 256, 256, [4, 5, 6, 255])).collect();
		let s = pack(&big);
		assert_eq!(s.len(), 113, "every big picture placed");
		assert!(s.w > 512 && s.h <= MAX_H, "{} x {}", s.w, s.h);
		assert_eq!(s.size_of(112), Some((256, 256)));
		let wide = pack(&[pic(1, 600, 8, [7, 7, 7, 255])]);
		assert_eq!((wide.w, wide.len()), (1024, 1));
		// 1 x 2100 strips: one shelf of 4096 / 3 fits under MAX_H, a second would not
		let huge: Vec<Owned> = (0..1400u32).map(|i| pic(i, 1, 2100, [8, 8, 8, 255])).collect();
		let s = pack(&huge);
		assert_eq!((s.w, s.h, s.len()), (4096, 2102, 4096 / 3), "the widest atlas, one shelf, the rest left out");
	}

	#[test]
	fn the_widest_atlas_stops_at_its_height_cap_and_huge_sizes_do_not_overflow() {
		// 2046 wide = two pictures per shelf of the widest atlas; 20 shelves of 202 px = 4040 rows, a 21st would pass MAX_H: the 41st
		// picture (and on) is left out.
		let v: Vec<Owned> = (0..44u32).map(|i| pic(i, 2046, 200, [1, 2, 3, 255])).collect();
		let s = pack(&v);
		assert!(s.w == 4096 && s.h <= MAX_H && s.len() == 40, "{} pictures, {} x {} px", s.len(), s.w, s.h);
		let short = pic(99, 8, 4, [9, 9, 9, 255]);
		let mut v2 = v.clone();
		v2.push(short);
		assert!(pack(&v2).uv(99).is_some(), "a small picture still finds room on the last shelf");
		let huge = [Pic { id: 1, w: 0x8000_0000, h: 0x8000_0000, rgba: &[] }, Pic { id: 2, w: 4, h: u32::MAX, rgba: &[] }];
		assert!(ImageSet::pack(&huge).is_empty(), "sizes whose byte count overflows u32 are skipped, not wrapped");
	}

	#[test]
	fn uv_matches_the_tile_and_bad_pictures_are_skipped() {
		let s = pack(&[pic(1, 8, 4, [1, 2, 3, 4]), (2, 8, 4, vec![0u8; 5])]);
		assert!(s.uv(2).is_none() && s.size_of(2).is_none());
		let [u0, v0, u1, v1] = s.uv(1).unwrap();
		assert!(((u1 - u0) * s.w as f32 - 8.0).abs() < 1e-3 && ((v1 - v0) * s.h as f32 - 4.0).abs() < 1e-3);
		assert_eq!(s.size_of(1), Some((8, 4)));
	}
}
