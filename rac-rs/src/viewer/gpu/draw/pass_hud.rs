//! HUD-pass steps: the overlay quad groups with the prompt model between them, and the pause-menu / vendor item previews.

use super::*;

impl Gpu {
	pub(super) fn pass_hud(&self, mut pass: &mut wgpu::RenderPass<'_>, scene: &Scene, warp: bool, db: &DrawBufs) {
		let (hd, split_group, hud_buffers, sharp_pipes) = (&db.hd, db.split_group, &db.hud_buffers, db.sharp_pipes);
			// BRIDGE-36 (c5): the HUD groups before the split, then the prompt model (world depth, world uniforms), then the rest.
			let draw_hud = |pass: &mut wgpu::RenderPass<'_>, groups: &[(Draw, bool)]| {
				let Some((vb, ib)) = &hud_buffers else { return };
				pass.set_vertex_buffer(0, vb.slice(..));
				pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
				for (d, add) in groups {
					if d.texture == crate::retail::splitview::SPLIT_TEX {
						// [PC] Split view (split.rs): the Classic picture over the left part, opaque.
						let Some(bind) = super::split::bind() else { continue };
						pass.set_pipeline(&self.hud_pipeline[2]);
						pass.set_bind_group(1, &bind, &[]);
					} else if d.texture == RETAIL_OV_TEX {
						let Some((_, bind, _, _)) = &self.retail_ov else { continue };
						pass.set_pipeline(&self.hud_pipeline[3]);
						pass.set_bind_group(1, bind, &[]);
					} else if d.texture == RETAIL_FILTER_TEX {
						let Some((_, bind, _, _)) = &self.retail_ov else { continue };
						pass.set_pipeline(&self.hud_pipeline[4]);
						pass.set_bind_group(1, bind, &[]);
					} else if d.texture == ITEM_PANEL_TEX || d.texture == POPUP_PANEL_TEX || d.texture == PORTRAIT_PANEL_TEX {
						// Retail mode: added onto the GS overlay's black panel interior (the overlay holds retail's panel text).
						pass.set_pipeline(&self.hud_pipeline[if self.retail_ov.is_some() { 1 } else { 2 }]);
						pass.set_bind_group(1, if d.texture == ITEM_PANEL_TEX { &self.item_panel.bind } else if d.texture == POPUP_PANEL_TEX { &self.popup_panel.bind } else { &self.portrait_panel.bind }, &[]);
					} else {
						pass.set_pipeline(match sharp_pipes.filter(|_| d.inst != 0) {
							Some(p) => &p[*add as usize],
							None => &self.hud_pipeline[*add as usize],
						});
						pass.set_bind_group(1, &self.hud_textures[d.texture.min(self.hud_textures.len() - 1)], &[]);
					}
					pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
				}
			};
			let g = split_group.unwrap_or(hd.len());
			draw_hud(&mut pass, &hd[..g]);
			if let (Some(_), Some(prompt), false) = (split_group, scene.prompt.as_ref(), warp) {
				pass.set_bind_group(0, &self.ubind, &[]);
				pass.set_vertex_buffer(0, prompt.vbuf.slice(..));
				pass.set_index_buffer(prompt.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				pass.set_pipeline(&self.pipeline);
				for d in &prompt.draws {
					if d.cull != 0 {
						pass.set_pipeline(&self.pipeline_cull[d.cull as usize - 1]);
					}
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
					if d.cull != 0 {
						pass.set_pipeline(&self.pipeline);
					}
				}
				for pipeline in &self.moby_pipelines {
					pass.set_pipeline(pipeline);
					for d in &prompt.draws {
						pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
						pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
					}
				}
			}
			draw_hud(&mut pass, &hd[g..]);
	}

	pub(super) fn pass_previews(&self, pass: &mut wgpu::RenderPass<'_>, scene: &Scene, sw: f32, sh: f32) {
			// Pause-menu item preview: a scoped viewport inside the panel, depth reset first. The hero
			// (keys >= HERO_PREVIEW) and the panel-3 item use the frozen menu camera (ingame_pause_menu.md "Menu
			// camera", "Panel 3 item model"); the vendor preview keeps a fitted turntable camera [P].
			for (slot, (class, r, t, item)) in [self.preview, self.preview2.map(|(c, r, t)| (c, r, t, None)), self.preview_pair, self.preview_worn].into_iter().enumerate().filter_map(|(i, p)| Some((i, p?))) {
				if let Some((batch, centre, radius)) = scene.previews.get(&class) {
					let (x0, y0) = (r[0].max(0.0).floor(), r[1].max(0.0).floor());
					let (x1, y1) = ((r[0] + r[2]).min(sw).floor(), (r[1] + r[3]).min(sh).floor());
					if x1 > x0 + 1.0 && y1 > y0 + 1.0 {
						let (vw, vh) = (x1 - x0, y1 - y0);
						let q = |x: f32, y: f32| Vertex { pos: [x / sw * 2.0 - 1.0, 1.0 - y / sh * 2.0, 1.0], uv: [0.0, 0.0], normal: [0.0; 3], colour: [0.0; 4], light: [0.0; 4] };
						let verts = [q(x0, y0), q(x1, y0), q(x1, y1), q(x0, y1)];
						let vb = buffer(&self.device, "preview clear vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
						self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
						let ib = buffer(&self.device, "preview clear indices", bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]), wgpu::BufferUsages::INDEX);
						self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&[0u32, 1, 2, 0, 2, 3]));
						let fixed = class >= HERO_PREVIEW || item.is_some();
						let mut cam = Camera { pos: [0.0; 3], yaw: if fixed { std::f32::consts::PI } else { t * 0.9 }, pitch: if fixed { 0.0 } else { -0.25 }, tan_half: 0.5, up: [0.0, 0.0, 1.0] };
						if fixed {
							// Menu scene (FUN_00275ab8 / FUN_002816a8) [C]: camera at (256,256,64), identity basis
							// (looks +X), projected with the matrix frozen at pause open (zoom 0.63, PAL 0.756; the
							// RT bind does not rebuild it). Ratchet at camera + (4, 0, -0.6), yaw pi: in his frame the
							// camera sits at (4, 0, 0.6) facing -X. Square pixels [PC] port choice.
							cam.pos = [4.0, 0.0, 0.6];
							cam.tan_half = menu_tan_half_y(r[2] * 448.0 / sh, r[3] * 448.0 / sh);
							if item.is_some() {
								// Panel 3 blits with flag 8, a centred crop of the RT at the full-screen scale [I].
								cam = Camera { pos: [0.0; 3], yaw: 0.0, pitch: 0.0, tan_half: r[3] * 448.0 / sh / 2.0 / 224.0 * 0.63 * 0.756, up: [0.0, 0.0, 1.0] };
							}
						} else {
							let f = forward(&cam);
							let dist = radius / (cam.tan_half * (vw / vh).min(1.0)) * 1.15;
							cam.pos = [centre[0] - f[0] * dist, centre[1] - f[1] * dist, centre[2] - f[2] * dist];
						}
						let mut pu = self.uniforms(&cam, &cam, vw / vh, (0.1, (*radius * 4.0).max(100.0))); // [PC] model preview range
						pu.fog = [0.0, 1.0e9, 255.0, 255.0];
						// Slot 14, set at pause open (FUN_0028ba40), in world axes = the menu camera's; turned into
						// the model's frame below (the hero stands at yaw pi; the panel-3 model by its rotation).
						let d = MENU_LIGHT_DIR;
						let mut to_light = match item {
							Some(_) => [0.0; 3],
							None if fixed => [d[0], d[1], -d[2]],
							None => [-d[0], -d[1], -d[2]],
						};
						if let Some(id) = item {
							// FUN_0027bc70: rot (row rx, row ry, t), pos = camera + (d + a cos t - b sin t,
							// y + a sin t + b cos t, z); normals stay unrotated [P].
							let [_, d, y, z, rx, ry, a, b] = PANEL3_VIEW[(id as usize).min(PANEL3_VIEW.len() - 1)];
							let (s, c) = t.sin_cos();
							let m = place([d + a * c - b * s, y + a * s + b * c, z], [rx, ry, t]);
							let l = MENU_LIGHT_DIR;
							to_light = [0, 1, 2].map(|k| -(m[4 * k] * l[0] + m[4 * k + 1] * l[1] + m[4 * k + 2] * l[2]));
							let vp = pu.view_proj;
							for (k, col) in pu.view_proj.iter_mut().enumerate() {
								*col = [0, 1, 2, 3].map(|row| (0..4).map(|j| vp[j][row] * m[4 * k + j]).sum());
							}
						}
						let c = MENU_LIGHT_COLOUR;
						pu.lights[MENU_LIGHT_SLOT] = [[c[0], c[1], c[2], -0.25], [to_light[0], to_light[1], to_light[2], 0.0], [0.0; 4], [0.0; 4]];
						// LIT-7a: the gold weapon's slot-12 light, same direction as slot 14, wrap -0.3, colour B zero.
						let gold = slot == 0 && self.preview_gold && item.is_some();
						let batch = if gold { scene.previews_gold.get(&class).unwrap_or(batch) } else { batch };
						if gold {
							let g = gold_light_colour();
							pu.lights[GOLD_LIGHT_SLOT] = [[g[0], g[1], g[2], GOLD_LIGHT_WRAP], [to_light[0], to_light[1], to_light[2], 0.0], [0.0; 4], [0.0; 4]];
						}
						self.queue.write_buffer(&self.pbuf[slot], 0, bytemuck::bytes_of(&pu));
						pass.set_scissor_rect(x0 as u32, y0 as u32, vw as u32, vh as u32);
						// The worn batch draws over the hero without resetting the depth.
						if class != HERO_WORN {
							pass.set_pipeline(&self.clear_pipeline);
							pass.set_bind_group(0, &self.ubind, &[]);
							pass.set_bind_group(1, &self.hud_textures[0], &[]);
							pass.set_vertex_buffer(0, vb.slice(..));
							pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
							pass.draw_indexed(0..6, 0, 0..1);
						}
						pass.set_viewport(x0, y0, vw, vh, 0.0, 1.0);
						pass.set_pipeline(&self.pipeline);
						pass.set_bind_group(0, &self.pbind[slot], &[]);
						pass.set_vertex_buffer(0, batch.vbuf.slice(..));
						pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
						for d in &batch.draws {
							pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
							pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
						}
						for pipeline in &self.moby_pipelines {
							pass.set_pipeline(pipeline);
							for d in &batch.draws {
								pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
								pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
							}
						}
					}
				}
			}
	}
}
