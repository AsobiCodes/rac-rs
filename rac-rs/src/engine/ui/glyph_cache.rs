//! The glyph atlas on the CPU side: rasterised glyphs (and the line shapes of `shapes.rs`) packed by `atlas.rs` into one R8 bitmap
//! that `gpu.rs` uploads. A glyph is rasterised at `px * scale` with the pen at one of four sub-pixel x positions, so text keeps
//! its even spacing without blurring; the key is (face, character, size in quarter pixels, sub-pixel). When the UI scale changes
//! (a resize) every old size is dead, so the cache starts over. Growth (1024 -> 2048) keeps all slots; a full 2048 atlas is
//! cleared and `epoch` counts the clears so a frame that already looked slots up can run again.
use super::atlas::Atlas;
use super::font::{role_index, FontSet, ROLES};
use super::shapes::{self, Shape};
use ab_glyph::{point, Font, Glyph};
use std::collections::HashMap;

/// Sub-pixel x positions per pixel.
pub const SUBS: u32 = 4;
/// `GlyphKey::face` of a line shape (`ch` is then `Shape::id`).
pub const FACE_SHAPE: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct GlyphKey {
	pub face: u8,
	pub ch: u32,
	/// Size in quarter pixels (a shape: its bitmap side in pixels).
	pub size_q: u16,
	pub sub: u8,
}

impl GlyphKey {
	pub fn glyph(face: u8, ch: char, size_q: u16, sub: u8) -> GlyphKey {
		GlyphKey { face, ch: ch as u32, size_q, sub }
	}
	/// A shape's bitmap of side `px`, at most `SHAPE_MAX_PX` (a bigger rect draws the biggest bitmap; `shapes::raster` allocates px^2).
	pub fn shape(s: Shape, px: u32) -> GlyphKey {
		GlyphKey { face: FACE_SHAPE, ch: s.id(), size_q: px.min(SHAPE_MAX_PX) as u16, sub: 0 }
	}
}

/// The biggest shape bitmap side in pixels (half the biggest atlas side).
pub const SHAPE_MAX_PX: u32 = Atlas::MAX / 2;

/// Where a glyph sits in the atlas and how its bitmap hangs off the pen: the bitmap's top-left is at (pen x floor + dx, baseline + dy).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Slot {
	pub x: u32,
	pub y: u32,
	pub w: u32,
	pub h: u32,
	pub dx: i32,
	pub dy: i32,
}

pub struct GlyphCache {
	atlas: Atlas,
	/// R8 bitmap, `size` x `size`.
	pub pixels: Vec<u8>,
	map: HashMap<GlyphKey, Slot>,
	/// Rows `[a, b)` changed since the last upload.
	dirty: Option<(u32, u32)>,
	resized: bool,
	epoch: u32,
	scale_bits: u32,
}

impl Default for GlyphCache {
	fn default() -> Self {
		GlyphCache::with_limits(Atlas::START, Atlas::MAX)
	}
}

impl GlyphCache {
	pub fn with_limits(size: u32, max: u32) -> GlyphCache {
		GlyphCache { atlas: Atlas::new(size, max), pixels: vec![0; (size * size) as usize], map: HashMap::new(), dirty: None, resized: true, epoch: 0, scale_bits: 0 }
	}

	pub fn size(&self) -> u32 {
		self.atlas.size()
	}

	pub fn len(&self) -> usize {
		self.map.len()
	}

	pub fn is_empty(&self) -> bool {
		self.map.is_empty()
	}

	pub fn epoch(&self) -> u32 {
		self.epoch
	}

	/// Start of a frame at UI scale `s`: a changed scale drops every slot (sizes are re-rasterised on demand).
	pub fn begin_frame(&mut self, s: f32) {
		if self.scale_bits != s.to_bits() {
			self.scale_bits = s.to_bits();
			self.clear();
		}
	}

	fn clear(&mut self) {
		self.atlas.clear();
		self.map.clear();
		self.pixels.fill(0);
		self.dirty = Some((0, self.atlas.size()));
		self.epoch += 1;
	}

	/// The atlas grew since the last call (the texture must be made again and uploaded whole).
	pub fn take_resized(&mut self) -> bool {
		std::mem::take(&mut self.resized)
	}

	/// The rows changed since the last call.
	pub fn take_dirty(&mut self) -> Option<(u32, u32)> {
		self.dirty.take()
	}

	pub fn get(&self, k: &GlyphKey) -> Option<Slot> {
		self.map.get(k).copied()
	}

	fn grow(&mut self) -> bool {
		let old = self.atlas.size();
		if !self.atlas.grow() {
			return false;
		}
		let new = self.atlas.size();
		let mut px = vec![0u8; (new * new) as usize];
		for y in 0..old {
			px[(y * new) as usize..(y * new + old) as usize].copy_from_slice(&self.pixels[(y * old) as usize..((y + 1) * old) as usize]);
		}
		self.pixels = px;
		self.resized = true;
		self.dirty = Some((0, new));
		true
	}

	/// Room for `w` x `h`: grows when the atlas is full, clears it when it cannot grow (then `epoch` changed). A bitmap that cannot fit
	/// even the biggest empty atlas is refused at once (clearing would not make room and would wipe every glyph, every frame).
	fn reserve(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
		if w.saturating_add(1) > self.atlas.max() || h.saturating_add(1) > self.atlas.max() {
			return None;
		}
		for _ in 0..3 {
			if let Some(at) = self.atlas.alloc(w, h) {
				return Some(at);
			}
			if !self.grow() {
				self.clear();
			}
		}
		None
	}

	fn put(&mut self, k: GlyphKey, w: u32, h: u32, dx: i32, dy: i32, bits: impl FnOnce(&mut dyn FnMut(u32, u32, u8))) {
		let Some((x, y)) = (if w == 0 || h == 0 { Some((0, 0)) } else { self.reserve(w, h) }) else { return };
		let size = self.atlas.size();
		let pixels = &mut self.pixels;
		bits(&mut |i, j, v| {
			if i < w && j < h {
				pixels[((y + j) * size + x + i) as usize] = v;
			}
		});
		if w > 0 && h > 0 {
			self.dirty = Some(match self.dirty {
				Some((a, b)) => (a.min(y), b.max(y + h)),
				None => (y, y + h),
			});
		}
		self.map.insert(k, Slot { x, y, w, h, dx, dy });
	}

	/// Makes sure `k` is in the atlas (rasterises it if not). Returns false if it could not be stored.
	/// (A glyph refused by `reserve` is not stored: it simply does not draw.)
	pub fn ensure(&mut self, fonts: &FontSet, k: GlyphKey) -> bool {
		if self.map.contains_key(&k) {
			return true;
		}
		if k.face == FACE_SHAPE {
			let Some(shape) = Shape::from_id(k.ch) else { return false };
			let px = k.size_q as u32;
			let bits = shapes::raster(shape, px);
			self.put(k, px, px, 0, 0, |set| {
				for (n, v) in bits.iter().enumerate() {
					set(n as u32 % px, n as u32 / px, *v);
				}
			});
			return self.map.contains_key(&k);
		}
		let Some(role) = ROLES.get(k.face as usize).copied() else { return false };
		debug_assert_eq!(role_index(role), k.face as usize);
		let Some(c) = char::from_u32(k.ch) else { return false };
		let font = fonts.font(role);
		let glyph = Glyph { id: font.glyph_id(c), scale: fonts.px_scale(role, k.size_q as f32 / 4.0), position: point(k.sub as f32 / SUBS as f32, 0.0) };
		match font.outline_glyph(glyph) {
			None => self.put(k, 0, 0, 0, 0, |_| {}),
			Some(og) => {
				let b = og.px_bounds();
				let (w, h) = ((b.max.x - b.min.x).ceil().max(0.0) as u32, (b.max.y - b.min.y).ceil().max(0.0) as u32);
				self.put(k, w, h, b.min.x.floor() as i32, b.min.y.floor() as i32, |set| {
					og.draw(|x, y, c| set(x, y, (c.clamp(0.0, 1.0) * 255.0 + 0.5) as u8));
				});
			}
		}
		self.map.contains_key(&k)
	}
}
