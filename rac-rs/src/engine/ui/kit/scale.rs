//! Design space to window: one uniform scale and one offset. The 1280x720 design is letterboxed into the window (16:9 safe
//! area, centred): 1280x720 -> 1, 1280x800 (Steam Deck) -> 1 with 40 px bands top and bottom, 1920x1080 -> 1.5, 3440x1440 -> 2
//! with side bands. The backdrop covers the whole window; everything else lives inside the safe area.
use crate::ui::prim::Rect;
use crate::ui::theme::{DESIGN_H, DESIGN_W};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Scale {
	pub s: f32,
	pub ox: f32,
	pub oy: f32,
	pub win_w: f32,
	pub win_h: f32,
}

impl Scale {
	pub fn fit(win_w: u32, win_h: u32) -> Scale {
		let (w, h) = (win_w.max(1) as f32, win_h.max(1) as f32);
		let s = (w / DESIGN_W).min(h / DESIGN_H);
		Scale { s, ox: ((w - DESIGN_W * s) * 0.5).round(), oy: ((h - DESIGN_H * s) * 0.5).round(), win_w: w, win_h: h }
	}
	/// Window pixel -> design units.
	pub fn to_design(&self, px: f32, py: f32) -> (f32, f32) {
		((px - self.ox) / self.s, (py - self.oy) / self.s)
	}
	pub fn to_window(&self, r: Rect) -> Rect {
		Rect::new(self.ox + r.x * self.s, self.oy + r.y * self.s, r.w * self.s, r.h * self.s)
	}
	/// The safe area in window pixels.
	pub fn safe_area(&self) -> Rect {
		Rect::new(self.ox, self.oy, DESIGN_W * self.s, DESIGN_H * self.s)
	}
	/// A length in design units as window pixels (font sizes: text is rasterised at `px * s`).
	pub fn len(&self, design: f32) -> f32 {
		design * self.s
	}
}
