//! GPU side of the arrival warp's draw pass (decomp/notes/level_transition.md SCN-4.2): the star fan (the warp
//! bundle's sky shells, re-posed every frame), the big quad / ribbon / gloss world quads through the warp's own fx
//! texture table, and the banner and fade quads over the screen. The ship itself goes through BRIDGE-35
//! (`load_warp_actors` / `set_warp_actors`).

use super::*;
use crate::game::transit::{WarpFrame, WARP_BIG_TEX};

/// Everything `load_warp` uploads; dropped by `clear_warp`.
pub(super) struct WarpFx {
	/// The sky shells as one ordered batch whose vertices are rewritten every frame.
	fan: Batch,
	fan_verts: Vec<Vertex>,
	fan_ranges: Vec<std::ops::Range<usize>>,
	fan_binds: Vec<wgpu::BindGroup>,
	/// The shells' untransformed positions.
	base: Vec<Vec<[f32; 3]>>,
	/// Warp fx slot (and `WARP_BIG_TEX`) -> texture for the world quads.
	pub(super) quad_tex: HashMap<usize, wgpu::BindGroup>,
	/// This frame's world quads (big quad, ribbons, gloss) in draw order.
	pub(super) quads: Vec<particles::WorldQuad>,
	/// HUD slot of the banner texture.
	banner_slot: Option<usize>,
	banner: Option<u32>,
	fade: u8,
	/// Projection scalar of the fan (times the PAL ratio at draw time).
	fan_tan: f32,
	/// Background colour (the bundle sky's, as the level's sky is).
	pub(super) clear: wgpu::Color,
}

impl Gpu {
	/// Uploads the warp bundle's sky, fx textures and the pair's big-quad and banner textures.
	pub(super) fn load_warp(&mut self, assets: &scene::WarpAssets, big: &Texture, banner: Option<&Texture>) {
		let mut t: HashMap<u32, Texture> = assets.sky.textures.iter().map(|(k, v)| (*k, Texture { w: v.w, h: v.h, rgba: v.rgba.clone() })).collect();
		t.insert(sky::UNTEXTURED, Texture { w: 1, h: 1, rgba: vec![[255; 4]] });
		let (mut binds, mut slots) = (Vec::new(), HashMap::new());
		let refs: Vec<&Mesh> = assets.sky.meshes.iter().collect();
		let (fan, _, _, fan_verts) = self.batch(&refs, Some(&assets.sky.colours), true, &t, &mut binds, &mut slots);
		let mut start = 0;
		let fan_ranges = refs
			.iter()
			.map(|m| {
				let r = start..start + m.positions.len();
				start = r.end;
				r
			})
			.collect();
		let mut quad_tex: HashMap<usize, wgpu::BindGroup> = assets.fx_textures.iter().enumerate().map(|(i, x)| (i, self.upload_texture(Some(x)))).collect();
		quad_tex.insert(WARP_BIG_TEX, self.upload_texture(Some(big)));
		let banner_slot = banner.map(|b| self.hud_texture(None, b));
		let base = assets.sky.meshes.iter().map(|m| m.positions.clone()).collect();
		self.warp_fx = Some(WarpFx { fan, fan_verts, fan_ranges, fan_binds: binds, base, quad_tex, quads: Vec::new(), banner_slot, banner: None, fade: 0, fan_tan: 0.0, clear: wgpu::Color::BLACK });
	}

	/// This frame's pass: re-poses the fan shells and stores the quads, banner and fade.
	pub(super) fn set_warp_frame(&mut self, f: &WarpFrame, gloss: &[particles::WorldQuad]) {
		let Some(w) = &mut self.warp_fx else { return };
		for (k, m, t) in &f.fan {
			let (Some(range), Some(base)) = (w.fan_ranges.get(*k), w.base.get(*k)) else { continue };
			for (v, p) in w.fan_verts[range.clone()].iter_mut().zip(sky::fan_positions(base, *m, *t)) {
				v.pos = p;
			}
		}
		self.queue.write_buffer(&w.fan.vbuf, 0, bytemuck::cast_slice(&w.fan_verts));
		w.quads = f.quads.iter().chain(gloss).cloned().collect();
		w.banner = f.banner;
		w.fade = f.fade;
		w.fan_tan = f.fan_tan;
	}

	/// The warp is over: drops the fan, the quad textures and the banner slot.
	pub(super) fn clear_warp(&mut self) {
		if let Some(w) = self.warp_fx.take() {
			if let Some(s) = w.banner_slot {
				self.hud_textures.truncate(s);
			}
		}
	}

	/// The warp's screen quads for a `sw` x `sh` window: the banner at (32, H - 88, 256, 32) of the 512 x 448 frame
	/// (centred, scaled by the height) and the black fade over everything.
	pub(super) fn warp_hud(&self, sw: f32, sh: f32) -> Vec<Quad> {
		let mut out = Vec::new();
		let Some(w) = &self.warp_fx else { return out };
		if let (Some(slot), Some(word)) = (w.banner_slot, w.banner) {
			let (k, ox) = (sh / 448.0, (sw - 512.0 * sh / 448.0) / 2.0);
			out.push(Quad { tex: slot, rect: [ox + 32.0 * k, 360.0 * k, 256.0 * k, 32.0 * k], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0, 1.0, 1.0, (word >> 24) as f32 / 128.0], angle: 0.0, corners: None });
		}
		if w.fade > 0 {
			overlay::rect(&mut out, [0.0, 0.0, sw, sh], [0.0, 0.0, 0.0, w.fade as f32 / 128.0]);
		}
		out
	}

	/// The fan's draw: the shells in order with the sky pipeline and the fan's own projection (preview slot 3 holds
	/// the uniforms). Returns false when no warp is loaded.
	pub(super) fn draw_warp_fan<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) -> bool {
		let Some(w) = &self.warp_fx else { return false };
		pass.set_bind_group(0, &self.pbind[3], &[]);
		pass.set_pipeline(&self.sky_pipeline);
		pass.set_vertex_buffer(0, w.fan.vbuf.slice(..));
		pass.set_index_buffer(w.fan.ibuf.slice(..), wgpu::IndexFormat::Uint32);
		for d in &w.fan.draws {
			pass.set_bind_group(1, &w.fan_binds[d.texture], &[]);
			pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
		}
		pass.set_bind_group(0, &self.ubind, &[]);
		true
	}

	/// Writes the fan's uniforms (the camera's rotation with the fan's projection scalar).
	pub(super) fn write_warp_uniforms(&self, cam: &Camera, aspect: f32) {
		let Some(w) = &self.warp_fx else { return };
		let fan_eye = Camera { pos: [0.0; 3], yaw: cam.yaw, pitch: cam.pitch, tan_half: w.fan_tan * PROJ_Y_RATIO, up: cam.up };
		let u = self.uniforms(cam, &fan_eye, aspect, (RETAIL_NEAR, self.world_far()));
		self.queue.write_buffer(&self.pbuf[3], 0, bytemuck::bytes_of(&u));
	}
}
