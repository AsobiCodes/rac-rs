//! The mods' draw list (OmniLoader ABI 1.2 `draw_submit`, src/omni/draw.rs) as the last 2D layer of the HUD pass: over the GS overlay (retail's
//! HUD and menus), the prompt model and the item previews. Both layers ("hud" and "menu") sit here: the overlay is one picture, so the renderer
//! cannot put a mod's HUD between retail's HUD and retail's menus.
//!
//! Zero cost without mods: `crate::omni::draw::current` answers `None` from one atomic load until a mod has submitted a non-empty list; no
//! buffer, no pipeline (the HUD alpha pipeline and the built-in font texture already exist) and no draw call are made. Both backends (Vulkan, DX12)
//! take the same path: wgpu quads in window pixels, mapped from the game's 512 x 448 space by `omni::draw_map::Layout`.
//!
//! Sharp HUD (`retail::ui_sharp`: Graphics row HUD = Sharp, window >= 896 px tall): the game-art quads (the game font's glyphs, the icons;
//! every texture past slot 0) take the edge-aware sampler of viewer/hudsharp.rs, the one the front end's art and the retail HUD's second,
//! window-resolution picture use, so a mod's text and icons are as crisp as the game's own. Solid rectangles and bars are painted into small
//! game-pixel pictures (omni/draw_mask.rs, exact coverage, in submit order) that take the same sampler, so a shape a mod builds from stacked
//! rects (a cut corner) is smoothed like retail art; the pictures are kept between frames while the rects stay the same (`MASKS`). A picture
//! too large for that falls back to plain quads. HUD = Original (or a window below 896 px): plain quads, nearest sampling, the classic look.

use super::*;
use crate::omni::draw_mask::{self, Piece, Run, Solid};
use crate::omni::draw_quads::build as plain_build;
use crate::overlay::{Quad, FONT};
use std::sync::Mutex;

/// Slot marker of a mask picture's quad (past every HUD texture slot).
const MASK: usize = usize::MAX;

/// A kept solids piece: where it sits in game pixels and its texture, or the rects to draw as plain quads.
enum Kept {
	Mask { x: f32, y: f32, w: f32, h: f32, bind: wgpu::BindGroup },
	Plain(Vec<Solid>),
}

/// The pieces of one solids run of the last frame, valid while the same rects and the same visible game area come again.
struct Cached {
	solids: Vec<Solid>,
	clip: [i32; 4],
	pieces: Vec<Kept>,
}

/// Pictures of the mods' solids by run number (MAIN only; see STATE.md).
static MASKS: Mutex<Vec<Cached>> = Mutex::new(Vec::new());

/// What one draw call binds.
#[derive(Clone, Copy)]
enum Src {
	Slot(usize),
	/// Piece `.1` of solids run `.0` in `MASKS`.
	Mask(usize, usize),
}

/// A draw call: the quads up to `to` (from the end of the previous item).
struct Item {
	to: usize,
	src: Src,
}

/// Records that the last quad of `quads` is drawn with `src`: joins the previous call when it binds the same slot.
fn add_item(items: &mut Vec<Item>, quads: &[Quad], src: Src) {
	let same = |a: Src, b: Src| matches!((a, b), (Src::Slot(x), Src::Slot(y)) if x == y);
	match items.last_mut() {
		Some(last) if same(last.src, src) => last.to = quads.len(),
		_ => items.push(Item { to: quads.len(), src }),
	}
}

impl Gpu {
	/// The previews, then the mods' layer (called in place of `pass_previews` by `draw_passes`).
	pub(super) fn pass_previews_omni(&self, pass: &mut wgpu::RenderPass<'_>, scene: &Scene, sw: f32, sh: f32) {
		self.pass_previews(pass, scene, sw, sh);
		self.pass_omni(pass, sw, sh);
	}

	/// The sharp layer: quads and draw calls in submit order (game-art quads as they are, solids as kept pictures).
	fn omni_sharp(&self, runs: Vec<Run>, layout: &crate::omni::draw_map::Layout, clip: [i32; 4], cache: &mut Vec<Cached>) -> (Vec<Quad>, Vec<Item>) {
		let (mut quads, mut items, mut ordinal) = (Vec::new(), Vec::new(), 0);
		for run in runs {
			match run {
				Run::Quads(v) => {
					for q in v {
						let tex = q.tex;
						quads.push(q);
						add_item(&mut items, &quads, Src::Slot(tex));
					}
				}
				Run::Solids(v) => {
					if cache.get(ordinal).is_none_or(|c| c.clip != clip || c.solids != v) {
						let pieces = draw_mask::pieces(&v, clip).into_iter().map(|p| self.keep(p)).collect();
						let fresh = Cached { solids: v, clip, pieces };
						if ordinal < cache.len() {
							cache[ordinal] = fresh;
						} else {
							cache.push(fresh);
						}
					}
					for (j, k) in cache[ordinal].pieces.iter().enumerate() {
						match k {
							Kept::Mask { x, y, w, h, .. } => {
								let [x0, y0] = layout.to_window(*x, *y);
								let [x1, y1] = layout.to_window(x + w, y + h);
								quads.push(Quad { tex: MASK, rect: [x0, y0, x1 - x0, y1 - y0], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
								items.push(Item { to: quads.len(), src: Src::Mask(ordinal, j) });
							}
							Kept::Plain(s) => {
								draw_mask::plain_quads(&mut quads, layout, s);
								if let Some(last) = items.last_mut().filter(|l| matches!(l.src, Src::Slot(FONT))) {
									last.to = quads.len();
								} else {
									items.push(Item { to: quads.len(), src: Src::Slot(FONT) });
								}
							}
						}
					}
					ordinal += 1;
				}
			}
		}
		cache.truncate(ordinal);
		(quads, items)
	}

	/// A piece as kept state: the picture uploaded (straight alpha, nearest sampler; the sharp shader does its own reconstruction).
	fn keep(&self, p: Piece) -> Kept {
		match p {
			Piece::Mask(m) => {
				let t = crate::level::Texture { w: m.w, h: m.h, rgba: m.rgba };
				Kept::Mask { x: m.x as f32, y: m.y as f32, w: m.w as f32, h: m.h as f32, bind: self.upload(Some(&t), &self.nearest) }
			}
			Piece::Plain(s) => Kept::Plain(s),
		}
	}

	fn pass_omni(&self, pass: &mut wgpu::RenderPass<'_>, sw: f32, sh: f32) {
		let Some(frame) = crate::omni::draw::current() else { return };
		let Some((_, _, layout)) = crate::omni::draw::view() else { return };
		let art = crate::omni::draw_art::get();
		let sharp = crate::retail::ui_sharp(sh as u32);
		let mut cache = MASKS.lock().unwrap_or_else(|e| e.into_inner());
		let (quads, items) = if sharp {
			let [gx0, gy0] = layout.to_game(0.0, 0.0);
			let [gx1, gy1] = layout.to_game(sw, sh);
			let clip = [gx0.min(gx1).floor() as i32, gy0.min(gy1).floor() as i32, gx0.max(gx1).ceil() as i32, gy0.max(gy1).ceil() as i32];
			self.omni_sharp(draw_mask::build(&frame, &layout, art.as_deref()), &layout, clip, &mut cache)
		} else {
			cache.clear();
			let quads = plain_build(&frame, &layout, art.as_deref());
			let mut items = Vec::new();
			for i in 0..quads.len() {
				add_item(&mut items, &quads[..=i], Src::Slot(quads[i].tex));
			}
			(quads, items)
		};
		if quads.is_empty() {
			return;
		}
		let mut verts = Vec::with_capacity(quads.len() * 4);
		let mut idx = Vec::with_capacity(quads.len() * 6);
		for q in &quads {
			let [x, y, w, h] = q.rect;
			let base = verts.len() as u32;
			for (px, py, u, v) in [(x, y, q.uv[0], q.uv[1]), (x + w, y, q.uv[2], q.uv[1]), (x + w, y + h, q.uv[2], q.uv[3]), (x, y + h, q.uv[0], q.uv[3])] {
				verts.push(Vertex { pos: [px / sw * 2.0 - 1.0, 1.0 - py / sh * 2.0, 0.0], uv: [u, v], normal: [0.0; 3], colour: q.colour, light: if sharp && q.tex != FONT { q.uv } else { [0.0; 4] } });
			}
			idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
		}
		let vb = buffer(&self.device, "omni vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
		self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
		let ib = buffer(&self.device, "omni indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
		self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
		// The previews left their own viewport, scissor and uniforms bound.
		pass.set_viewport(0.0, 0.0, sw, sh, 0.0, 1.0);
		pass.set_scissor_rect(0, 0, sw as u32, sh as u32);
		pass.set_bind_group(0, &self.ubind, &[]);
		let sharp_pipe = sharp.then(|| &super::hudsharp::pipelines(&self.device, &self.pipeline, &self.tex_layout, self.config.format, self.depth_format)[0]);
		pass.set_vertex_buffer(0, vb.slice(..));
		pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
		// One draw per run of quads that share a texture (the built-in font, the game font, icons, a solids picture), in list order.
		let mut from = 0;
		for it in &items {
			let (bind, sharp_here) = match it.src {
				Src::Slot(tex) => (self.hud_textures.get(tex), tex != FONT),
				Src::Mask(n, j) => (cache.get(n).and_then(|c| c.pieces.get(j)).and_then(|k| if let Kept::Mask { bind, .. } = k { Some(bind) } else { None }), true),
			};
			if let Some(bind) = bind {
				pass.set_pipeline(sharp_pipe.filter(|_| sharp_here).unwrap_or(&self.hud_pipeline[0]));
				pass.set_bind_group(1, bind, &[]);
				pass.draw_indexed(from as u32 * 6..it.to as u32 * 6, 0, 0..1);
			}
			from = it.to;
		}
	}
}
