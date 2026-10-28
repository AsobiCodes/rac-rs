//! The render passes of the frame in order: level pass (sky, world, water, effects), the particle passes, post, the HUD pass.
//! This function owns the pass objects (a pass borrows the encoder and is dropped / re-begun between the stages); the stages
//! that only record draws are the `pass_*` steps in the sibling files.

use super::*;

impl Gpu {
	pub(super) fn draw_passes(&mut self, cam: &Camera, dc: &DrawFrame, dfx: &mut DrawFx, dt: &mut DrawTarget, db: &DrawBufs) -> Option<()> {
		let scene = self.scene.as_ref()?;
		let (aspect, wfar, post_on, gfx) = (dc.aspect, dc.wfar, dc.post_on, dc.gfx);
		let (eye, u, (water_on, strips_water, part_on, rays)) = (&dc.eye, &dc.u, (dfx.water_on, dfx.strips_water, dfx.part_on, dfx.rays));
		let (wrec, strip_flows, river, river_groups, water_frame) = (&mut dfx.wrec, &dfx.strip_flows, &dfx.river, &dfx.river_groups, &dfx.water_frame);
		let ((view, gi_level, rs), (mut enc, mut cuts), (sw, sh, multi)) = ((&dt.view, dt.gi_level, dt.rs), (&mut dt.enc, &mut dt.cuts), (db.sw, db.sh, &db.multi)); self.star_frame(scene, &gfx); /* [PC] the animated Gemlik star (star.rs), decided in every pass */ let clouds_active = post_on && !self.under_flag && !self.under.active() && !scene.warp_shown && !self.ship_fight; if !crate::retail::splitview::classic() { self.cloud_sky(enc, scene, u, &gfx, clouds_active); } /* [PC] Volumetric clouds: the new sky (volclouds/skymodel.rs), marched before the level pass (volclouds.rs) */ match super::volclouds::Input::new(&self.post, &gfx, u, (self.level, self.config.width, self.config.height), (cam, aspect, wfar), clouds_active, &self.sun_rows()) { Some(c) => { self.volclouds.march(&self.device, &self.queue, enc, &c); } None => self.volclouds.idle() }
		{
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("level"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment {
					view: &self.msaa,
					resolve_target: Some(self.post.scene().filter(|_| post_on).unwrap_or(&view)),
					depth_slice: None,
					ops: wgpu::Operations {
						load: wgpu::LoadOp::Clear(self.warp_fx.as_ref().filter(|_| scene.warp_shown).map_or(scene.clear, |w| w.clear)),
						store: if part_on || water_on || rs { wgpu::StoreOp::Store } else { wgpu::StoreOp::Discard },
					},
				})],
				depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
					view: &self.depth,
					depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
					// BRIDGE-64a: the shadow-volume counter starts at 128 every frame.
					stencil_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(super::shadowvol::STENCIL_CLEAR), store: wgpu::StoreOp::Discard }),
				}),
				timestamp_writes: crate::viewer::passtime::rw("level"),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_bind_group(0, &self.ubind, &[]);
			// BRIDGE-35: while the arrival warp plays only its ship batch is drawn.
			let warp = scene.warp_shown;
			if warp {
				self.draw_warp_fan(&mut pass);
			}
			self.pass_sky(&mut pass, scene, warp, self.volclouds.sky.skip(), true); if self.volclouds.apply(&mut pass) { pass.set_bind_group(0, &self.ubind, &[]); } // [PC] Volumetric clouds over the sky, under every world draw, effect and particle
			self.pass_world(&mut pass, scene, warp, dc, db, water_on, wrec);
			self.pass_oceans(&mut pass, scene, warp, water_on, wrec);
			self.pass_world_quads(&mut pass, cam, warp, water_on, wrec, river, river_groups, strips_water, strip_flows); self.pass_kofi(&mut pass, warp); /* [PC] Ko-fi drawings on the garage wall (kofi.rs, draw/pass_kofi.rs) */
			self.pass_shop_models(&mut pass, scene, cam, eye, aspect, wfar, warp);
			// [PC] Particles Enhanced (partfx.rs): the pool (and the pool lines after it) in their own pass over the level's depth.
			// [PC] Particles Enhanced: the additive pool particles' light (partfx.rs `lights`; runs > 0 = the retail pool, so the
			// fixed glows and the ribbons at run 0 stay out) on the smoke and, in post, the scene.
			let fx_lights = match part_on && !warp && super::partfx::FX_ON {
				true => super::partfx::lights(self.part_verts.iter().filter(|((run, additive, key), _)| *run > 0 && *additive && *key != LINE_KEY).map(|((_, _, key), v)| (*key, &v[..])), &self.partfx.tints, cam.pos, super::partfx::light_count(gfx.particles_q)),
				false => super::partfx::FxLights::default(),
			};
			if let (true, Some(resolve), Some(glow)) = (part_on, self.post.scene(), self.post.glow()) {
				drop(pass);
				cuts.cut(&self.device, &mut enc, "f_lvl");
				let generation = self.post.water_inputs().map_or(0, |w| w.3);
				pass = self.partfx.begin(&self.device, &self.queue, &mut enc, &self.msaa, &self.depth, resolve, glow, generation, &fx_lights, wfar);
				pass.set_bind_group(0, &self.ubind, &[]);
			}
			let (all, lall, runs, mut covered) = self.particle_runs(warp);
			if !runs.is_empty() {
				let (tris, lvb, any_tex) = self.particle_buffers(&all, &lall);
				for &(line, additive, key, first, count) in &runs {
					if line {
						let (Some(vb), Some(tex)) = (lvb.as_ref(), any_tex) else { continue };
						pass.set_pipeline(if part_on { &self.partfx.line } else { &self.line_pipeline });
						pass.set_bind_group(1, tex, &[]);
						pass.set_vertex_buffer(0, vb.slice(..));
						pass.draw(first..first + count, 0..1);
						continue;
					}
					let (Some((vb, ib)), Some(tex)) = (tris.as_ref(), self.fx_tex.get(&(key as usize)).or(self.part_tex.as_ref())) else { continue };
					pass.set_pipeline(if part_on { &self.partfx.soft[additive as usize] } else { &self.part_pipelines[additive as usize] });
					pass.set_bind_group(1, tex, &[]);
					pass.set_vertex_buffer(0, vb.slice(..));
					pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
					pass.draw_indexed(first..first + count, 0, 0..1);
				}
				if let (true, false, Some((vb, ib)), Some(resolve), Some(glow)) = (water_on, part_on, tris.as_ref(), self.post.scene(), self.post.glow()) {
					drop(pass);
					let generation = self.post.water_inputs().map_or(0, |w| w.3);
					pass = self.partfx.begin(&self.device, &self.queue, &mut enc, &self.msaa, &self.depth, resolve, glow, generation, &super::partfx::FxLights::default(), wfar);
					pass.set_bind_group(0, &self.ubind, &[]);
					pass.set_vertex_buffer(0, vb.slice(..));
					pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
					for &(_, additive, key, first, count) in runs.iter().filter(|r| !r.0) {
						let Some(tex) = self.fx_tex.get(&(key as usize)).or(self.part_tex.as_ref()) else { continue };
						pass.set_pipeline(&self.partfx.cover[additive as usize]);
						pass.set_bind_group(1, tex, &[]);
						pass.draw_indexed(first..first + count, 0, 0..1);
					}
					covered = true;
				}
			}
			// KER-92: the pool lines (untextured; any texture bind group satisfies the layout).
			if let (false, Some(tex)) = (warp || self.line_verts.is_empty(), self.part_tex.as_ref().or_else(|| self.fx_tex.values().next())) {
				let vb = buffer(&self.device, "particle lines", bytemuck::cast_slice(&self.line_verts), wgpu::BufferUsages::VERTEX);
				self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&self.line_verts));
				pass.set_pipeline(if part_on || covered { &self.partfx.line } else { &self.line_pipeline });
				pass.set_bind_group(1, tex, &[]);
				pass.set_vertex_buffer(0, vb.slice(..));
				pass.draw(0..self.line_verts.len() as u32, 0..1);
			}
			// [PC] Enhanced image (post.rs): the level pass ends here; the post passes grade the picture into the MSAA target, and the
			// HUD pass loads it and draws everything below (HUD, retail overlay, prompt model, item previews) as the level pass would.
			if post_on {
				drop(pass);
				cuts.cut(&self.device, &mut enc, "f_fx");
				// [PC] The planar mirrors (mirror.rs) of the flat water planes below the eye the view looks at most: Water High up to
				// three planes (each its own pass and layer), Medium the one, Low none (the reflection march alone, no second scene pass).
				let picked: Vec<f32> = water_frame.as_ref().map(|f| mirror_pick(self.level, &wrec, cam.pos, &f.vp, mirror_count(gfx.water_q))).unwrap_or_default(); let mirrors = self.waterfx.steady_mirrors(picked);
				if water_log() {
					let planes = water_frame.as_ref().map(|f| super::mirror::view_stats(&wrec, cam.pos, super::mirror::inverse(&f.vp).as_ref())).unwrap_or_default();
					eprintln!("water: level {} draws {} ({:?}) mirrors {:?} planes (rays of 81, height) {:?}", self.level, wrec.len(), wrec.iter().map(|w| (w.kind, w.n, w.plane.map(|b| b[0]), w.flow)).collect::<Vec<_>>(), mirrors, planes);
				}
				let water = match (&water_frame, self.post.water_inputs()) {
					// (Also with no water in view but a waterfall, which the water pass draws.)
					(Some(f), Some(inputs)) if !wrec.is_empty() || self.waterfx.has_falls() => {
						self.waterfx.positions(&self.device, &self.queue, &mut enc, &self.depth, inputs, f, &wrec, &mirrors, part_on || covered);
						cuts.cut(&self.device, &mut enc, "f_wpos");
						for (k, &h) in mirrors.iter().enumerate() {
							if let Some(target) = self.waterfx.mirror_target(k) {
								self.draw_mirror(&mut enc, scene, &u, h, k, target, multi.as_ref().map(|m| &m.0));
								cuts.cut(&self.device, &mut enc, "f_mir");
							}
						}
						// [PC] What lies under the water (mirror.rs `draw_bed`), for the clear shallows and the lava's shore glow.
						if let Some(target) = self.waterfx.bed_target() {
							// (With the geometric sea waves (waterfx.rs `sea_waves`) between: the shore map from the bed, the sea's grid.)
							self.draw_bed(&mut enc, scene, &u, target, multi.as_ref().map(|m| &m.0));
							cuts.cut(&self.device, &mut enc, "f_bed");
						}
						true
					}
					_ => false,
				};
				self.sky_build(&mut enc, scene, u, cam.pos, water_frame.is_some() && (water || self.under_flag || self.under.active()));
				self.post.bind_water(&self.device, if water { self.waterfx.gi_mask() } else { None });
				let waterfx = &self.waterfx; let fall_tex = super::waterfx::fall_fx_tex(self.level).first().and_then(|t| self.fx_tex.get(t));
				// [PC] Water Low / Medium / High: the underwater look (underwater.rs), after the water pass, before bloom and the grade.
				let under_in = water_frame.as_ref().map(|f| super::underwater::frame(cam, aspect, self.under_flag, self.under_surf, &wrec, f.fog, f.key.map(|k| k.0), f.level, wfar));
				// [PC] Volumetric fog (volfog.rs): after the water pass, before bloom and the grade; not under water (its own look).
				let fog = super::volfog::Input::new(&self.post, &self.sun, dc.sun.as_ref(), dc.fog, u, (self.level, self.config.width, self.config.height), (cam, aspect, wfar), &self.sun_rows()); let fog = fog.map(|mut f| { f.water = water.then(|| self.waterfx.gi_mask().map(|m| m.0)).flatten(); f });
				let (volfog, under, device, queue, depth) = (&mut self.volfog, &mut self.under, &self.device, &self.queue, &self.depth);
				self.post.encode(&self.queue, &mut enc, cam, aspect, gfx.ao_q, gfx.sun_q, gfx.particles_q, part_on, &fx_lights, rays, gi_level, wfar, |enc, scene, ao| {
					if water {
						waterfx.shade(enc, scene, ao, fall_tex);
					}
					if let Some(fog) = &fog { volfog.run(device, queue, enc, fog, depth); }
					if let Some(uf) = &under_in {
						under.run(device, queue, enc, scene, depth, uf, waterfx.sky());
					}
				});
				cuts.cut(&self.device, &mut enc, "f_pe");
				self.post.composite(&mut enc, &self.msaa, gi_level);
				cuts.cut(&self.device, &mut enc, "f_post");
				pass = self.hud_pass(enc, cuts, view, rs);
			}
			// [PC] ReShade add-on (reshade.rs): in Classic the HUD pass starts here, after the effects.
			if rs && !post_on {
				drop(pass);
				pass = self.hud_pass(enc, cuts, view, true);
			}
			self.pass_hud(&mut pass, scene, warp, db);
			self.pass_previews_omni(&mut pass, scene, sw, sh);
		}
		Some(())
	}
}
