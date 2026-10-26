//! Shop portrait pose and the item panel render targets.

use super::*;

impl Gpu {
	/// BRIDGE-37 (b): poses the shop portrait (class 12) for the next frame: skinned vertices placed at `pos` / `rot` into the
	/// one-prop batch (alpha 0x80, light word colour 0x20; vendor.md UI-5a). `None` hides it (the target is still cleared).
	pub(in crate::viewer) fn pose_portrait(&mut self, pose: Option<(&Posed, [f32; 3], [f32; 3])>) {
		self.portrait_shown = false;
		let (Some(scene), Some(((ps, ns), pos, rot))) = (self.scene.as_ref(), pose) else { return };
		let Some(batch) = scene.portrait.as_ref().filter(|_| ps.len() == scene.portrait_model.len()) else { return };
		let m = place(pos, rot);
		let light = Self::moby_light(Some(([0x20; 3], 0)));
		let id = dyn_id(&self.table, &mut self.moby_dyn, pos);
		let mut verts = scene.portrait_model.clone();
		for (v, (q, n)) in verts.iter_mut().zip(ps.iter().zip(ns)) {
			v.colour[3] = moby_alpha_encoding(id, 0x80).0;
			v.light = light;
			for i in 0..3 {
				v.pos[i] = m[i] * q[0] + m[4 + i] * q[1] + m[8 + i] * q[2] + m[12 + i];
				v.normal[i] = m[i] * n[0] + m[4 + i] * n[1] + m[8 + i] * n[2];
			}
		}
		self.queue.write_buffer(&batch.vbuf, 0, bytemuck::cast_slice(&verts));
		self.portrait_shown = true;
	}

	/// Draws one frame; with `capture`, also returns it as an image.
	/// REN-9b: the shop item-panel pass. Clears the 512x128 target (colour 0, depth far) and draws the panel-1 item with the
	/// main camera's view and the target projection (tan 0.756, horizontal scalar 1.0, near 0.03125, far 512), then resolves it.
	/// The projection leaves the world projection alone: it is only this pass's own uniform slot (4).
	pub(super) fn draw_item_panel(&self, enc: &mut wgpu::CommandEncoder, cam: &Camera, scene: &Scene) {
		if !(self.shop_panel_models.is_empty() && !self.hud.iter().any(|q| q.tex == ITEM_PANEL_TEX)) {
			// NOV-30d: one clear, then the item (slot 4) and the slot-0 bolt icon (slot 7), also when no item model is posed.
			let item = self.shop_panel_models.first().and_then(|(c, m)| Some((4, &scene.previews.get(c)?.0, *m, None)));
			// NOV-35b: the icon bolt's class-13 metal (glass) faces.
			let icon_metal = self.icon_bolt.and_then(|m| self.panel_metal(scene, &m, self.icon_bolt_sphere));
			let icon = self.icon_bolt.and_then(|m| Some((7, &scene.previews.get(&13)?.0, m, icon_metal.as_ref())));
			let models: Vec<_> = item.into_iter().chain(icon).collect();
			self.draw_panel_target(enc, cam, scene, &self.item_panel, &models);
		}
		// BRIDGE-36 (b): the purchase popup's 512x128 target, cleared, plus the class-13 bolt when one is posed (UI-5d).
		if self.hud.iter().any(|q| q.tex == POPUP_PANEL_TEX) {
			let bolt_metal = self.popup_bolt.and_then(|m| self.panel_metal(scene, &m, self.popup_bolt_sphere));
			let bolt = self.popup_bolt.and_then(|m| Some((5, &scene.previews.get(&13)?.0, m, bolt_metal.as_ref())));
			self.draw_panel_target(enc, cam, scene, &self.popup_panel, bolt.as_slice());
		}
		// BRIDGE-37 (b): the shop portrait target, cleared every shop frame, the posed actor (world-space vertices) when present.
		if self.hud.iter().any(|q| q.tex == PORTRAIT_PANEL_TEX) {
			let ident = [1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0];
			let actor = scene.portrait.as_ref().filter(|_| self.portrait_shown).map(|b| (6, b, ident, None));
			self.draw_panel_target(enc, cam, scene, &self.portrait_panel, actor.as_slice());
		}
	}

	/// NOV-35b / BRIDGE-45: the class-13 metal faces of a shop icon or popup bolt placed at `m` with draw sphere `sphere`
	/// (`Game::shop_bolt_spheres`): model space and the stored (bind) vertices like its LOD preview, alpha from metal byte 0x18
	/// (no bolt_update on these slots) and the frame view's depth, env matrix from that view, colour from the preview's light
	/// word (menu slot 14, no third light). None: no class-13 metal, no view or sphere, or the alpha skips the metal part.
	fn panel_metal(&self, scene: &Scene, m: &[f32; 16], sphere: Option<([f32; 3], f32)>) -> Option<Batch> {
		let (packets, view, (c, w)) = (self.metal.classes.get(&13)?, scene.clip_view?, sphere?);
		let d: [f32; 3] = std::array::from_fn(|k| c[k] - view.e[k] * 1024.0);
		let alpha = metal_alpha(0x18, view.f[0] * d[0] + view.f[1] * d[1] + view.f[2] * d[2], w)?;
		let e = mobies::metal_env(view.e, metal_view_rows(&view), c, axes(m));
		// Slot 14 as `draw_panel_target` lights the LOD faces (retail level-light row in retail mode, else the menu row).
		let a = match self.reserved_lights {
			Some(r) => {
				let r = r[MENU_LIGHT_SLOT - 12];
				([r[1][0], r[1][1], r[1][2]], [r[0][0], r[0][1], r[0][2]])
			}
			None => (MENU_LIGHT_DIR.map(|x| -x), MENU_LIGHT_COLOUR),
		};
		let rows = [a, ([0.0; 3], [0.0; 3]), ([0.0; 3], [0.0; 3])];
		let (mut verts, mut runs) = (Vec::new(), BTreeMap::new());
		push_metal(&mut verts, &mut runs, packets, None, &rows, e, m, 1.0, alpha, false);
		let mut batch = None;
		upload_metal(&self.device, &self.queue, &mut batch, &mut (0, 0), &verts, runs);
		batch
	}

	/// One 512x128 target pass: clear (colour 0, depth far), then each `(uniform slot, batch, model, metal batch)` in order,
	/// depth-tested, with the main projection centred on the target (NOV-30a); a model's metal faces follow its LOD faces.
	fn draw_panel_target(&self, enc: &mut wgpu::CommandEncoder, cam: &Camera, scene: &Scene, target: &ItemPanel, models: &[(usize, &Batch, [f32; 16], Option<&Batch>)]) {
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("target panel"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment {
				view: &target.msaa,
				resolve_target: Some(&target.resolved),
				depth_slice: None,
				ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Discard },
			})],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
				view: &target.depth,
				depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Discard }),
				stencil_ops: None,
			}),
			timestamp_writes: crate::viewer::passtime::rw("target panel"),
			occlusion_query_set: None,
			multiview_mask: None,
		});
		for &(slot, batch, ref m, metal) in models {
			// NOV-30a: the main projection (scalar 0.63, PAL 0.756) centred on the target, not a target projection.
			let pcam = Camera { tan_half: cam.tan_half * PANEL_TAN_SCALE, ..*cam };
			let mut pu = self.uniforms_fog(&pcam, &Camera { pos: [0.0; 3], ..pcam }, PANEL_ASPECT, (RETAIL_NEAR, RETAIL_FAR), false);
			let vp = pu.view_proj;
			for (c, col) in pu.view_proj.iter_mut().enumerate() {
				*col = [0, 1, 2, 3].map(|row| (0..4).map(|j| vp[j][row] * m[4 * c + j]).sum());
			}
			let local = |p: [f32; 4]| {
				let d = [p[0] - m[12], p[1] - m[13], p[2] - m[14]];
				let l = [0, 1, 2].map(|c| m[4 * c] * d[0] + m[4 * c + 1] * d[1] + m[4 * c + 2] * d[2]);
				[l[0], l[1], l[2], p[3]]
			};
			pu.eye = local(pu.eye);
			for i in (0..pu.points.len()).step_by(2) {
				pu.points[i] = local(pu.points[i]);
			}
			// Light slot 14 (the panel models' light word 0x0e0e): retail mode reads the live level-light row (retail.rs
			// `reserved_lights`, Eudora (0.9, 0.9, 0.6) towards (-0.80, -0.14, -0.58)) as the moby draw does; else the menu row.
			// To-light directions go to model space through the unscaled rotation (m = s R: R^T d = m^T d / s).
			let s2 = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).max(1e-12);
			let local = |d: [f32; 4]| -> [f32; 4] {
				let v = [0, 1, 2].map(|k| (m[4 * k] * d[0] + m[4 * k + 1] * d[1] + m[4 * k + 2] * d[2]) / s2.sqrt());
				[v[0], v[1], v[2], 0.0]
			};
			pu.lights[MENU_LIGHT_SLOT] = match self.reserved_lights {
				Some(r) => {
					let r = r[MENU_LIGHT_SLOT - 12];
					[r[0], local(r[1]), r[2], local(r[3])]
				}
				None => {
					let (l, c) = (MENU_LIGHT_DIR, MENU_LIGHT_COLOUR);
					[[c[0], c[1], c[2], -0.25], local([-l[0], -l[1], -l[2], 0.0]), [0.0; 4], [0.0; 4]]
				}
			};
			self.queue.write_buffer(&self.pbuf[slot], 0, bytemuck::bytes_of(&pu));
			pass.set_bind_group(0, &self.pbind[slot], &[]);
			pass.set_vertex_buffer(0, batch.vbuf.slice(..));
			pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
			for pipeline in std::iter::once(&self.pipeline).chain(&self.moby_pipelines) {
				pass.set_pipeline(pipeline);
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				for d in &batch.draws {
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
				}
				if let (Some(mb), Some(tex)) = (metal, &self.metal.tex) {
					pass.set_vertex_buffer(0, mb.vbuf.slice(..));
					pass.set_index_buffer(mb.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					for d in &mb.draws {
						pass.set_bind_group(1, &tex[d.texture], &[]);
						pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
					}
				}
			}
		}
	}
}
