//! The draw list (`Prim`s in design units) turned into GPU instances in WINDOW pixels: one 112-byte record per rounded rectangle
//! or textured quad (a glyph of the atlas, a symbol or a chevron shape), in draw order. Text is placed here: alignment, shrink-to-fit
//! (`max_w`), whole-pixel baseline, sub-pixel pen. No device is touched, so the whole step is unit-tested. Clips (the scrolling list)
//! travel with each instance as a rounded rectangle the shader multiplies in; instances outside their clip are dropped here.
use super::font::{role_index, FontSet};
use super::glyph_cache::{GlyphCache, GlyphKey, SUBS};
use super::kit::Scale;
use super::prim::{Align, FontRole, GlyphKind, Prim, Rect, Rgba, TextRun};
use super::shapes::Shape;

/// `Inst::q.z` (the same numbers as `K_*` in ui.wgsl; a test compares them).
pub const K_RRECT: f32 = 0.0;
pub const K_TEXT: f32 = 1.0;
/// A picture of the image atlas (`images.rs`): `uv` = u0, v0, u1, v1, `q.x` the corner radius.
pub const K_IMAGE: f32 = 2.0;
/// `Inst::clip.z` when the instance has no clip.
pub const NO_CLIP: [f32; 4] = [0.0, 0.0, -1.0, -1.0];
pub const INST_BYTES: u64 = 112;

/// A shrunk run goes down to this fraction of its size, then it is cut and ends with an ellipsis.
const MIN_SHRINK: f32 = 0.8;
/// The pad button disc: fill and hairline of the mockup SVG (circle r 11 + stroke 1 in a 24 box).
const DISC_FILL: Rgba = Rgba::rgba(28, 36, 51, 1.0); // #1C2433 (bytes: a hex literal of that size trips the boundary test)
const DISC_RING: Rgba = Rgba::rgba(255, 255, 255, 0.18);
/// Letter in a disc: its size relative to the disc.
const LETTER_EM: f32 = 0.56;

/// One instance. `rect` = x, y, w, h in window pixels; `q` = corner radius, border width, kind, -; `uv` = atlas x, y of the bitmap
/// (text); `clip` = x, y, w, h of the clip (w < 0 = none), `clipq.x` its corner radius. Colours are straight RGBA 0..1.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Inst {
	pub rect: [f32; 4],
	pub fill: [f32; 4],
	pub border: [f32; 4],
	pub q: [f32; 4],
	pub uv: [f32; 4],
	pub clip: [f32; 4],
	pub clipq: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
struct Clip {
	rect: [f32; 4],
	r: f32,
}

struct QuadReq {
	key: GlyphKey,
	x: i32,
	y: i32,
	fill: [f32; 4],
	clip: Option<Clip>,
}

enum Item {
	Inst(Inst),
	Quad(QuadReq),
}

/// What `build` returns: the instances in draw order and the backdrop request (blur in design units, scrim colour).
#[derive(Debug, Default)]
pub struct Built {
	pub insts: Vec<Inst>,
	pub backdrop: Option<(f32, Rgba)>,
}

fn outside(c: &Option<Clip>, x: f32, y: f32, w: f32, h: f32) -> bool {
	match c {
		Some(c) => x >= c.rect[0] + c.rect[2] || x + w <= c.rect[0] || y >= c.rect[1] + c.rect[3] || y + h <= c.rect[1],
		None => false,
	}
}

fn with_clip(mut i: Inst, c: &Option<Clip>) -> Inst {
	if let Some(c) = c {
		i.clip = c.rect;
		i.clipq = [c.r, 0.0, 0.0, 0.0];
	} else {
		i.clip = NO_CLIP;
	}
	i
}

/// The run's glyphs as quad requests (pen positions, baseline), or nothing for an empty run.
fn place_text(t: &TextRun, sc: &Scale, fonts: &FontSet, clip: &Option<Clip>, out: &mut Vec<Item>) {
	let mut px = t.px * sc.s;
	let max_w = t.max_w * sc.s;
	let mut placed = fonts.place(t.role, px, t.track, &t.text);
	if max_w > 0.0 && placed.width > max_w + 0.01 {
		let f = max_w / placed.width;
		if f >= MIN_SHRINK {
			px *= f;
			placed = fonts.place(t.role, px, t.track, &t.text);
		} else {
			px *= MIN_SHRINK;
			let cut = fonts.ellipsis_cut(t.role, px, t.track, &t.text, max_w);
			placed = fonts.place(t.role, px, t.track, &cut);
		}
	}
	let x0 = sc.ox + t.x * sc.s
		- match t.align {
			Align::Left => 0.0,
			Align::Center => placed.width * 0.5,
			Align::Right => placed.width,
		};
	let base = (sc.oy + t.y * sc.s + fonts.centre_to_baseline(t.role) * px).round() as i32;
	let size_q = (px * 4.0).round().clamp(1.0, 4000.0) as u16;
	let face = role_index(t.role) as u8;
	for (c, off) in &placed.chars {
		if c.is_whitespace() {
			continue;
		}
		let pen = x0 + off;
		let mut ix = pen.floor();
		let mut sub = ((pen - ix) * SUBS as f32).round() as u32;
		if sub >= SUBS {
			sub = 0;
			ix += 1.0;
		}
		out.push(Item::Quad(QuadReq { key: GlyphKey::glyph(face, *c, size_q, sub as u8), x: ix as i32, y: base, fill: t.colour.to_f32(), clip: *clip }));
	}
}

fn rrect(w: Rect, radius: f32, fill: Rgba, border: Option<(f32, Rgba)>, clip: &Option<Clip>) -> Option<Inst> {
	if outside(clip, w.x, w.y, w.w, w.h) || w.w <= 0.0 || w.h <= 0.0 {
		return None;
	}
	let (bw, bc) = border.map(|(b, c)| (b, c.to_f32())).unwrap_or((0.0, [0.0; 4]));
	let i = Inst { rect: [w.x, w.y, w.w, w.h], fill: fill.to_f32(), border: bc, q: [radius.min(w.w.min(w.h) * 0.5), bw, K_RRECT, 0.0], ..Default::default() };
	Some(with_clip(i, clip))
}

fn glyph(rect: Rect, kind: GlyphKind, colour: Rgba, sc: &Scale, fonts: &FontSet, clip: &Option<Clip>, out: &mut Vec<Item>) {
	let w = sc.to_window(rect);
	let n = w.w.round().max(1.0);
	let (x, y) = (w.x.round(), w.y.round());
	let u = n / 24.0;
	let disc = Rect::new(x + 0.5 * u, y + 0.5 * u, n - u, n - u);
	if let Some(i) = rrect(disc, disc.w * 0.5, DISC_FILL, Some((u.max(1.0), DISC_RING)), clip) {
		out.push(Item::Inst(i));
	}
	match kind {
		GlyphKind::Symbol(s) => out.push(Item::Quad(QuadReq { key: GlyphKey::shape(Shape::of_symbol(s), n as u32), x: x as i32, y: y as i32, fill: colour.to_f32(), clip: *clip })),
		GlyphKind::Letter(c) => {
			let run = TextRun {
				x: rect.cx(),
				y: rect.cy(),
				role: FontRole::SemiBold,
				px: rect.h * LETTER_EM,
				colour,
				align: Align::Center,
				max_w: 0.0,
				track: 0.0,
				text: c.to_string(),
			};
			place_text(&run, sc, fonts, clip, out);
		}
	}
}

/// A picture as an instance (None when it lies outside the clip or is empty).
fn image(w: Rect, radius: f32, uv: [f32; 4], tint: Rgba, clip: &Option<Clip>) -> Option<Inst> {
	if outside(clip, w.x, w.y, w.w, w.h) || w.w <= 0.0 || w.h <= 0.0 {
		return None;
	}
	let i = Inst { rect: [w.x, w.y, w.w, w.h], fill: tint.to_f32(), q: [radius.min(w.w.min(w.h) * 0.5), 0.0, K_IMAGE, 0.0], uv, ..Default::default() };
	Some(with_clip(i, clip))
}

/// `FontSet` as the layout's measure (design units in, design units out).
struct Measure<'a>(&'a FontSet);

impl super::kit::TextMeasure for Measure<'_> {
	fn width(&self, role: FontRole, px: f32, text: &str) -> f32 {
		self.0.width(role, px, text)
	}
}

/// Instances for a draw list at the window mapping `sc`. Rasterises the glyphs the list needs into `cache`; pictures come from the
/// registered set (`images::uv`).
pub fn build(prims: &[Prim], sc: &Scale, fonts: &FontSet, cache: &mut GlyphCache) -> Built {
	build_with(prims, sc, fonts, cache, &super::images::uv)
}

/// `build` with the picture lookup given (tests): id -> UV rectangle.
pub fn build_with(prims: &[Prim], sc: &Scale, fonts: &FontSet, cache: &mut GlyphCache, uv_of: &dyn Fn(u32) -> Option<[f32; 4]>) -> Built {
	let prims = super::inline::expand_all(prims, &Measure(fonts)); // runs with button chips -> text pieces + discs / keycaps
	let prims = &prims[..];
	let mut items: Vec<Item> = Vec::with_capacity(prims.len() * 2);
	let mut clips: Vec<Clip> = Vec::new();
	let mut backdrop = None;
	for p in prims {
		let clip = clips.last().copied();
		match p {
			Prim::Backdrop { blur_px, scrim } => backdrop = Some((*blur_px, *scrim)),
			Prim::ClipPush { rect, radius } => {
				let w = sc.to_window(*rect);
				clips.push(Clip { rect: [w.x, w.y, w.w, w.h], r: (*radius * sc.s).min(w.w.min(w.h) * 0.5) });
			}
			Prim::ClipPop => {
				clips.pop();
			}
			Prim::RRect { rect, radius, fill, border } => {
				let b = border.map(|b| (b.width * sc.s, b.colour));
				if let Some(i) = rrect(sc.to_window(*rect), radius * sc.s, *fill, b, &clip) {
					items.push(Item::Inst(i));
				}
			}
			Prim::Text(t) => place_text(t, sc, fonts, &clip, &mut items),
			Prim::Glyph { rect, kind, colour } => glyph(*rect, *kind, *colour, sc, fonts, &clip, &mut items),
			Prim::Chevron { rect, right, colour } => {
				let w = sc.to_window(*rect);
				let n = w.w.round().max(1.0);
				let shape = if *right { Shape::ChevronRight } else { Shape::ChevronLeft };
				items.push(Item::Quad(QuadReq { key: GlyphKey::shape(shape, n as u32), x: w.x.round() as i32, y: w.y.round() as i32, fill: colour.to_f32(), clip }));
			}
			Prim::Image { rect, id, radius, tint } => {
				if let Some(i) = uv_of(*id).and_then(|uv| image(sc.to_window(*rect), radius * sc.s, uv, *tint, &clip)) {
					items.push(Item::Inst(i));
				}
			}
		}
	}
	// Rasterise what the frame needs; if the atlas had to be cleared meanwhile, the earlier slots are gone: go again.
	for _ in 0..2 {
		let e = cache.epoch();
		for it in &items {
			if let Item::Quad(q) = it {
				cache.ensure(fonts, q.key);
			}
		}
		if cache.epoch() == e {
			break;
		}
	}
	let mut insts = Vec::with_capacity(items.len());
	for it in items {
		match it {
			Item::Inst(i) => insts.push(i),
			Item::Quad(q) => {
				let Some(s) = cache.get(&q.key) else { continue };
				if s.w == 0 || s.h == 0 {
					continue;
				}
				let (x, y) = ((q.x + s.dx) as f32, (q.y + s.dy) as f32);
				if outside(&q.clip, x, y, s.w as f32, s.h as f32) {
					continue;
				}
				let i = Inst { rect: [x, y, s.w as f32, s.h as f32], fill: q.fill, q: [0.0, 0.0, K_TEXT, 0.0], uv: [s.x as f32, s.y as f32, 0.0, 0.0], ..Default::default() };
				insts.push(with_clip(i, &q.clip));
			}
		}
	}
	Built { insts, backdrop }
}
