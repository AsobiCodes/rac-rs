//! The render passes of the frame in order: level pass (sky, world, water, effects), the particle passes, post, the HUD pass.
//! This function owns the pass objects (a pass borrows the encoder and is dropped / re-begun between the stages); the stages
//! that only record draws are the `pass_*` steps in the sibling files.

use super::*;

impl Gpu {
	pub(super) fn draw_passes(&mut self, cam: &Camera, dc: &DrawFrame, dfx: &mut DrawFx, dt: &mut DrawTarget, db: &DrawBufs) -> Option<()> {
		let scene = self.scene.as_ref()?;
		let fire_want = dfx.part_on && dc.post_on && !scene.warp_shown && self.post.scene().is_some() && self.post.glow().is_some();
		self.firefx.gather(self.level, &self.fx_quads, cam, fire_want); // [PC] Fire (firefx.rs)
		self.firefx.settle(&mut self.part_verts); // [PC] Fire: the sprites it took over go back to the particle pass when it does not draw (post off, warp, Classic half)
		let (aspect, wfar, post_on, gfx) = (dc.aspect, dc.wfar, dc.post_on, dc.gfx);
		let (eye, u, (water_on, strips_water, part_on, rays)) = (&dc.eye, &dc.u, (dfx.water_on, dfx.strips_water, dfx.part_on, dfx.rays));
		let (wrec, strip_flows, river, river_groups, water_frame) = (&mut dfx.wrec, &dfx.strip_flows, &dfx.river, &dfx.river_groups, &dfx.water_frame);
		let ((view, gi_level, rs), (mut enc, mut cuts), (sw, sh, multi)) = ((&dt.view, dt.gi_level, dt.rs), (&mut dt.enc, &mut dt.cuts), (db.sw, db.sh, &db.multi));
		self.star_frame(scene, &gfx); // [PC] the animated Gemlik star (star.rs), decided in every pass
		let clouds_active = post_on && !self.under_flag && !self.under.active() && !scene.warp_shown && !self.ship_fight;
		// [PC] Volumetric smoke (volsmoke.rs): the held sprite quads are dropped while the volumes draw (not without post targets, A5), else given back.
		let smoke_on = Self::smoke_settle(&mut self.volsmoke, &mut self.part_verts, (self.level, &self.fx_quads, cam), clouds_active && self.post.water_inputs().is_some());
		if clouds_active && self.post.water_inputs().is_some() { self.volsmoke.warm(&self.device, &self.queue); } // [PC] Smoke: pipelines at the first frame it could draw (R7-3)
		if !crate::splitview::classic() { self.cloud_sky(enc, scene, u, &gfx, clouds_active); }
		// [PC] Metal On: the sky faces it reflects (metalenv.rs).
		self.metal_capture_env(enc, scene, u);
		self.metal_planar(enc, scene, u, multi.as_ref().map(|m| &m.0)); // [PC] Metal On: the planar floor reflection (metalplanar.rs)
		// [PC] Volumetric clouds: the new sky (volclouds/skymodel.rs), marched before the level pass (volclouds.rs)
		match super::volclouds::Input::new(&self.post, &gfx, u, (self.level, self.config.width, self.config.height), (cam, aspect, wfar), clouds_active, &self.sun_rows()) {
			Some(c) => { self.volclouds.march(&self.device, &self.queue, enc, &c); }
			None => self.volclouds.idle(),
		}
		{
			let mut pass = self.begin_level_pass(enc, scene, self.upscale.scene_view().unwrap_or(view), (post_on, part_on, water_on, rs || self.upscale.active())); // [PC] RAC_UPSCALE: a render-size resolve target
			pass.set_bind_group(0, &self.ubind, &[]);
			// BRIDGE-35: while the arrival warp plays only its ship batch is drawn.
			let warp = scene.warp_shown;
			if warp {
				self.draw_warp_fan(&mut pass);
			}
			self.pass_sky(&mut pass, scene, warp, self.volclouds.sky.skip(), true);
			// [PC] Volumetric clouds over the sky, under every world draw, effect and particle
			if self.volclouds.apply(&mut pass) { pass.set_bind_group(0, &self.ubind, &[]); }
			self.pass_world(&mut pass, scene, warp, dc, db, water_on, wrec);
			self.pass_oceans(&mut pass, scene, warp, water_on, wrec);
			self.pass_world_quads(&mut pass, cam, warp, water_on, wrec, river, river_groups, strips_water, strip_flows);
			self.pass_art(&mut pass, warp); // [PC] the pack's world art (R&C1: the Ko-fi drawings on the garage wall, kofi.rs; draw/pass_art.rs)
			self.pass_shop_models(&mut pass, scene, cam, eye, aspect, wfar, warp);
			// [PC] Particles Enhanced (partfx.rs): the pool (and the pool lines after it) in their own pass over the level's depth.
			// [PC] Particles Enhanced: the additive pool particles' light (partfx.rs `lights`; runs > 0 = the retail pool, so the
			// fixed glows and the ribbons at run 0 stay out) on the smoke and, in post, the scene.
			let fx_lights = self.pass_fx_lights(part_on && !warp, cam.pos, gfx.particles_q, gi_level);
			if let (true, Some(resolve), Some(glow)) = (part_on, self.post.scene(), self.post.glow()) {
				drop(pass);
				self.upscale.opaque_copy(enc, &self.msaa); // [PC] RAC_UPSCALE_PREP: the opaque scene before the particles (reactive mask)
				cuts.cut(&self.device, &mut enc, "f_lvl");
				let generation = self.post.water_inputs().map_or(0, |w| w.3);
				let (sim_env, size) = (super::firefx::FireEnv { device: &self.device, queue: &self.queue, pipeline: &self.pipeline, partfx: &self.partfx, format: self.config.format, depth_format: self.depth_format, view_proj: u.view_proj }, resolve.texture().size());
				self.firefx.simulate(&sim_env, &mut enc, cam, &super::firesim::StepIn { depth: &self.depth, key: (size.width, size.height, generation), near_far: [RETAIL_NEAR, wfar], aspect, particles_q: gfx.particles_q, level: crate::fire_option::level() }); // [PC] Fire simulated (firesim.rs)
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
			let fire_env = super::firefx::FireEnv { device: &self.device, queue: &self.queue, pipeline: &self.pipeline, partfx: &self.partfx, format: self.config.format, depth_format: self.depth_format, view_proj: u.view_proj };
			self.firefx.draw(&mut pass, &fire_env, cam); // [PC] Fire (firefx.rs)
			self.pass_pool_lines(&mut pass, warp, part_on || covered);
			// [PC] Enhanced image (post.rs): the level pass ends here; the post passes grade the picture into the MSAA target, and the
			// HUD pass loads it and draws everything below (HUD, retail overlay, prompt model, item previews) as the level pass would.
			if post_on {
				drop(pass);
				self.upscale.alpha_copy(enc, &self.msaa); // [PC] FSR 3: the scene before post, the reactive mask's other input (upscale.rs)
				pass = self.post_frame(enc, cuts, cam, dc, db, (wrec, water_frame, rays), (part_on, covered, smoke_on), &fx_lights, (view, gi_level, rs))?;
			}
			// [PC] ReShade add-on (reshade.rs) and RAC_UPSCALE_PREP (upscale.rs): in Classic the HUD pass starts here, after the effects.
			if (rs || self.upscale.active()) && !post_on {
				drop(pass);
				self.upscale_encode(enc, cuts);
				pass = self.hud_pass(enc, cuts, view, rs);
			}
			let scene = self.scene.as_ref()?; // (borrowed again: post_frame took &mut self)
			self.pass_hud(&mut pass, scene, warp, db);
			self.pass_previews_omni(&mut pass, scene, sw, sh);
		}
		Some(())
	}
}

impl Gpu {
	/// Smoke on this frame (the volumes can draw: `ok` = the post targets are up and the sky/clouds pass runs)? Takes the level fires'
	/// smoke quads over (`quads` = level, the effect quads, the eye's camera), then settles the held smoke sprite quads and those
	/// quads either way.
	fn smoke_settle(smoke: &mut super::volsmoke::VolSmoke, verts: &mut BTreeMap<(u32, bool, u32), Vec<Vertex>>, quads: (usize, &[crate::prims::WorldQuad], &Camera), ok: bool) -> bool {
		smoke.gather_quads((crate::pack::get().smoke_quads)(quads.0), quads.1, quads.2.pos, ok);
		let on = ok && smoke.ready();
		smoke.settle(on, quads.2, verts);
		on
	}

	/// Whether effect quad `i` (fx texture `tex`) is drawn by an effect of its own instead of the quad pass: Fire's tongue over a flame
	/// billboard, Smoke's volume over a fire's grey column or sheet (`warp`: the warp's quads are not the effect quads).
	pub(super) fn quad_replaced(&self, i: usize, warp: bool) -> bool {
		(!warp && self.firefx.skips(i)) || (!warp && self.volsmoke.took(i))
	}

	/// The pool particles' lights (partfx.rs `lights`) with the fire's own warm lights first (firefx.rs `merge_lights`, by the Global
	/// illumination level `gi_q` = the frame's effective one: 0 under water / post off), then the weapon lights ranked in (shotlight.rs).
	fn pass_fx_lights(&self, on: bool, eye: [f32; 3], particles_q: u8, gi_q: u8) -> super::partfx::FxLights {
		let retail = match on && super::partfx::FX_ON {
			true => super::partfx::lights(self.part_verts.iter().filter(|((run, additive, key), _)| *run > 0 && *additive && *key != LINE_KEY).map(|((_, _, key), v)| (*key, &v[..])), &self.partfx.tints, eye, super::partfx::light_count(particles_q)),
			false => super::partfx::FxLights::default(),
		};
		crate::viewer::shotlight::merge(self.firefx.merge_lights(retail, gi_q), on, eye, particles_q) // [PC] weapon lights ranked in (shotlight.rs)
	}

	/// KER-92: the pool lines (untextured; any texture bind group satisfies the layout). `covered` = the lines draw with the
	/// particle pass's line pipeline (Particles Enhanced, or the water cover pass ran).
	fn pass_pool_lines(&self, pass: &mut wgpu::RenderPass<'_>, warp: bool, covered: bool) {
		if let (false, Some(tex)) = (warp || self.line_verts.is_empty(), self.part_tex.as_ref().or_else(|| self.fx_tex.values().next())) {
			let vb = buffer(&self.device, "particle lines", bytemuck::cast_slice(&self.line_verts), wgpu::BufferUsages::VERTEX);
			self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&self.line_verts));
			pass.set_pipeline(if covered { &self.partfx.line } else { &self.line_pipeline });
			pass.set_bind_group(1, tex, &[]);
			pass.set_vertex_buffer(0, vb.slice(..));
			pass.draw(0..self.line_verts.len() as u32, 0..1);
		}
	}

	/// [PC] The planar mirrors (mirror.rs) of the picked planes, one pass and layer each; then what lies under the water (mirror.rs
	/// `draw_bed`), for the clear shallows and the lava's shore glow (with the geometric sea waves (waterfx.rs `sea_waves`) between:
	/// the shore map from the bed, the sea's grid).
	pub(super) fn pass_water_views(&self, enc: &mut wgpu::CommandEncoder, cuts: &mut super::presenter::Cuts, scene: &Scene, u: &Uniforms, mirrors: &[f32], ties: Option<&super::bindless::Runs>) {
		let u = &self.water_uniforms(u); // [PC] lane nsun: retail's key light where the water keeps it (gpu/lighting.rs)
		for (k, &h) in mirrors.iter().enumerate() {
			if let Some(target) = self.waterfx.mirror_target(k) {
				self.draw_mirror(enc, scene, u, h, k, target, ties);
				cuts.cut(&self.device, enc, "f_mir");
			}
		}
		if let Some(target) = self.waterfx.bed_target() {
			self.draw_bed(enc, scene, u, target, ties);
			cuts.cut(&self.device, enc, "f_bed");
		}
	}

	/// RAC_WATER_LOG: the frame's water draws, the mirror planes picked and the flat planes below the eye.
	pub(super) fn log_water(&self, wrec: &[super::waterfx::WaterDraw], mirrors: &[f32], water_frame: Option<&super::waterfx::WaterFrame>, eye: [f32; 3]) {
		if !water_log() {
			return;
		}
		let planes = water_frame.map(|f| super::mirror::view_stats(wrec, eye, super::mirror::inverse(&f.vp).as_ref())).unwrap_or_default();
		eprintln!("water: level {} draws {} ({:?}) mirrors {:?} planes (rays of 81, height) {:?}", self.level, wrec.len(), wrec.iter().map(|w| (w.kind, w.n, w.plane.map(|b| b[0]), w.flow)).collect::<Vec<_>>(), mirrors, planes);
	}
}
