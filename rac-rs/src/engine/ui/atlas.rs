//! Shelf packer for the glyph atlas (an R8 texture): rectangles go onto horizontal shelves, a new shelf starts below the last one.
//! The atlas starts at 1024 and doubles once to 2048 when it is full; growing keeps every rectangle where it is (the texture only
//! gets more room at the right and the bottom), so nothing has to be re-rasterised. At the maximum a full atlas is cleared by its
//! owner and the frame's glyphs are packed again (a frame needs a few thousand pixels, an atlas four million).
//! Pure CPU, no device: the packing is unit-tested (`tests_render.rs`).

/// Empty pixels kept between two rectangles (and at the right / bottom of each), so a sampler never reaches a neighbour.
const GAP: u32 = 1;

#[derive(Clone, Copy, Debug)]
struct Shelf {
	y: u32,
	h: u32,
	x: u32,
}

#[derive(Clone, Debug)]
pub struct Atlas {
	size: u32,
	max: u32,
	shelves: Vec<Shelf>,
	next_y: u32,
}

impl Atlas {
	/// Where the atlas starts and where it stops growing.
	pub const START: u32 = 1024;
	pub const MAX: u32 = 2048;

	pub fn new(size: u32, max: u32) -> Atlas {
		Atlas { size, max: max.max(size), shelves: Vec::new(), next_y: 0 }
	}

	/// The size it stops growing at.
	pub fn max(&self) -> u32 {
		self.max
	}

	pub fn size(&self) -> u32 {
		self.size
	}

	/// The top-left corner of a free `w` x `h` rectangle, or None when it does not fit (grow or clear, then try again).
	pub fn alloc(&mut self, w: u32, h: u32) -> Option<(u32, u32)> {
		let (bw, bh) = (w + GAP, h + GAP);
		if bw > self.size || bh > self.size {
			return None;
		}
		let size = self.size;
		// The shelf with the least wasted height that still has room at its end.
		let best = self.shelves.iter_mut().filter(|s| s.h >= bh && s.x + bw <= size).min_by_key(|s| s.h);
		if let Some(s) = best {
			let at = (s.x, s.y);
			s.x += bw;
			return Some(at);
		}
		if self.next_y + bh > self.size {
			return None;
		}
		let y = self.next_y;
		self.shelves.push(Shelf { y, h: bh, x: bw });
		self.next_y += bh;
		Some((0, y))
	}

	/// Doubles the size (up to the maximum); false when it is at the maximum already. Existing rectangles stay valid.
	pub fn grow(&mut self) -> bool {
		if self.size >= self.max {
			return false;
		}
		self.size = (self.size * 2).min(self.max);
		true
	}

	/// Forgets every rectangle (the size stays).
	pub fn clear(&mut self) {
		self.shelves.clear();
		self.next_y = 0;
	}

	/// Number of shelves in use (tests).
	pub fn shelf_count(&self) -> usize {
		self.shelves.len()
	}
}
