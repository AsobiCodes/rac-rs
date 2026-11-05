//! The line shapes of the UI: the four pad symbols drawn on the hint-bar button discs (the mockup's SVG paths) and the two list
//! chevrons. Each is a stroke of width 2 along a centre line, so the shape is a signed distance (`distance`) and the renderer
//! rasterises it once per pixel size into the glyph atlas (`raster`, anti-aliased by the exact distance), then draws it tinted like
//! a letter. The disc behind a symbol is an ordinary rounded rectangle. Geometry is in the mockup's view boxes: 24 x 24 for the
//! symbols, 14 x 14 for the chevrons.
use super::prim::Symbol;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
	Cross,
	Circle,
	Square,
	Triangle,
	ChevronLeft,
	ChevronRight,
}

pub const ALL: [Shape; 6] = [Shape::Cross, Shape::Circle, Shape::Square, Shape::Triangle, Shape::ChevronLeft, Shape::ChevronRight];

/// Half of the stroke width, in view-box units.
const HALF: f32 = 1.0;

impl Shape {
	pub fn id(self) -> u32 {
		self as u32
	}

	pub fn from_id(id: u32) -> Option<Shape> {
		ALL.get(id as usize).copied()
	}

	pub fn of_symbol(s: Symbol) -> Shape {
		match s {
			Symbol::Cross => Shape::Cross,
			Symbol::Circle => Shape::Circle,
			Symbol::Square => Shape::Square,
			Symbol::Triangle => Shape::Triangle,
		}
	}

	/// Side of the view box the shape is drawn in.
	pub fn view_box(self) -> f32 {
		match self {
			Shape::ChevronLeft | Shape::ChevronRight => 14.0,
			_ => 24.0,
		}
	}
}

fn seg(px: f32, py: f32, a: (f32, f32), b: (f32, f32)) -> f32 {
	let (dx, dy) = (b.0 - a.0, b.1 - a.1);
	let t = (((px - a.0) * dx + (py - a.1) * dy) / (dx * dx + dy * dy)).clamp(0.0, 1.0);
	let (cx, cy) = (a.0 + dx * t - px, a.1 + dy * t - py);
	(cx * cx + cy * cy).sqrt()
}

/// Signed distance (view-box units, negative inside the stroke) from `(x, y)` (view-box coordinates, origin top-left) to the shape.
pub fn distance(shape: Shape, x: f32, y: f32) -> f32 {
	let (dx, dy) = (x - 12.0, y - 12.0);
	match shape {
		// M8 8l8 8M16 8l-8 8, round caps
		Shape::Cross => seg(x, y, (8.0, 8.0), (16.0, 16.0)).min(seg(x, y, (16.0, 8.0), (8.0, 16.0))) - HALF,
		// circle r 4.8
		Shape::Circle => ((dx * dx + dy * dy).sqrt() - 4.8).abs() - HALF,
		// rect 7.6..16.4, sharp corners (the distance of the square, not the round one)
		Shape::Square => (dx.abs().max(dy.abs()) - 4.4).abs() - HALF,
		// M12 7.5 l4.8 8.5 H7.2 z, round joins
		Shape::Triangle => {
			let (a, b, c) = ((12.0, 7.5), (16.8, 16.0), (7.2, 16.0));
			seg(x, y, a, b).min(seg(x, y, b, c)).min(seg(x, y, c, a)) - HALF
		}
		// M9 2L4 7l5 5 and M5 2l5 5-5 5, round caps and joins
		Shape::ChevronLeft => seg(x, y, (9.0, 2.0), (4.0, 7.0)).min(seg(x, y, (4.0, 7.0), (9.0, 12.0))) - HALF,
		Shape::ChevronRight => seg(x, y, (5.0, 2.0), (10.0, 7.0)).min(seg(x, y, (10.0, 7.0), (5.0, 12.0))) - HALF,
	}
}

/// Coverage bitmap (`px` x `px`, 0..255) of the shape scaled so its view box fills the bitmap.
pub fn raster(shape: Shape, px: u32) -> Vec<u8> {
	let px = px.max(1);
	let unit = shape.view_box() / px as f32;
	let mut out = Vec::with_capacity((px * px) as usize);
	for j in 0..px {
		for i in 0..px {
			let d = distance(shape, (i as f32 + 0.5) * unit, (j as f32 + 0.5) * unit) / unit; // in output pixels
			out.push(((0.5 - d).clamp(0.0, 1.0) * 255.0 + 0.5) as u8);
		}
	}
	out
}
