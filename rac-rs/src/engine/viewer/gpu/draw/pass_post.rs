//! [PC] Enhanced image (post.rs), moved out of `draw_passes` (a pure move, lane ndlss): the level pass has ended; the water
//! views and water pass, smoke, fog, underwater, bloom / grade / GI write the graded picture back into the MSAA target, then
//! the HUD pass begins. Split view: `draw_passes` calls `post_frame` only under `DrawFrame.post_on` = `splitview::fx`
//! (prepare.rs), so the Classic half never runs it; its upscaler inputs are paused there too (split.rs, `Prep::pause`).

use super::*;

impl Gpu {
	/// The post passes of a Lighting Enhanced frame (no pass may be open) and the HUD pass begun after them. `water` = the
	/// level pass's water draws, the water frame and the sun rays; `on` = (part_on, covered, smoke_on); `target` = (view,
	/// gi_level, rs) of the frame. None only without a scene (draw_passes has one).
	#[allow(clippy::too_many_arguments)]
	pub(super) fn post_frame<'e>(&mut self, mut enc: &'e mut wgpu::CommandEncoder, cuts: &mut super::presenter::Cuts, cam: &Camera, dc: &DrawFrame, db: &DrawBufs, water: (&mut Vec<super::waterfx::WaterDraw>, &Option<super::waterfx::WaterFrame>, Option<([f32; 2], f32)>), on: (bool, bool, bool), (fx_lights, smoke_fx): (&super::partfx::FxLights, &super::partfx::FxLights), target: (&wgpu::TextureView, u8, bool)) -> Option<wgpu::RenderPass<'e>> {
		let scene = self.scene.as_ref()?;
		let (aspect, wfar, gfx, u, multi) = (dc.aspect, dc.wfar, dc.gfx, &dc.u, &db.multi);
		let ((wrec, water_frame, rays), (part_on, covered, smoke_on), (view, gi_level, rs)) = (water, on, target);
		cuts.cut(&self.device, &mut enc, "f_fx");
		// [PC] The planar mirrors (mirror.rs) of the flat water planes below the eye the view looks at most: Water High up to
		// three planes (each its own pass and layer), Medium the one, Low none (the reflection march alone, no second scene pass).
		let picked: Vec<f32> = water_frame.as_ref().map(|f| mirror_pick(self.level, &wrec, cam.pos, &f.vp, mirror_count(gfx.water_q))).unwrap_or_else(|| {
			mirror_forget();
			Vec::new()
		});
		let mirrors = self.waterfx.steady_mirrors(picked); self.firefx.set_water(&wrec); self.waterfx.mirror_cull.set_water(&mirrors);
		self.log_water(&wrec, &mirrors, water_frame.as_ref(), cam.pos);
		let water = match (&water_frame, self.post.water_inputs()) {
			// (Also with no water in view but a waterfall, which the water pass draws.)
			(Some(f), Some(inputs)) if !wrec.is_empty() || self.waterfx.has_falls() => {
				self.waterfx.positions(&self.device, &self.queue, &mut enc, &self.depth, inputs, f, &wrec, &mirrors, part_on || covered);
				cuts.cut(&self.device, &mut enc, "f_wpos");
				self.pass_water_views(enc, cuts, scene, u, &mirrors, multi.as_ref().map(|m| &m.0));
				true
			}
			_ => false,
		};
		// [PC] Graphics -> Underwater look Off (under_choice.rs, read once per frame here): the underwater look and the sky map see a dry
		// camera; the volumetric fog gate (prepare.rs) keeps the raw flag, so under water retail's fog keeps its full strength.
		let under_look = crate::viewer::under_choice::look(self.under_flag);
		self.sky_build(&mut enc, scene, u, cam.pos, water_frame.is_some() && (water || under_look || self.under.active()));
		self.post.bind_water(&self.device, if water { self.waterfx.gi_mask() } else { None });
		let waterfx = &self.waterfx;
		let fall_tex =super::waterfx::fall_fx_tex(self.level).first().and_then(|t| self.fx_tex.get(t));
		// [PC] Water Low / Medium / High: the underwater look (underwater.rs), after the water pass, before bloom and the grade.
		let under_in = water_frame.as_ref().map(|f| super::underwater::frame(cam, aspect, under_look, self.under_surf, &wrec, f.fog, f.key.map(|k| k.0), f.level, wfar));
		// [PC] Volumetric fog (volfog.rs): after the water pass, before bloom and the grade; not under water (its own look).
		let fog = super::volfog::Input::new(&self.post, &self.sun, dc.sun.as_ref(), dc.fog, u, (self.level, self.config.width, self.config.height), (cam, aspect, wfar), &self.sun_rows());
		let fog = fog.map(|mut f| { f.water = water.then(|| self.waterfx.gi_mask().map(|m| m.0)).flatten(); f });
		let smoke_in = if smoke_on { super::volsmoke::Input::new(&self.post, &self.sun, dc.sun.as_ref(), u, (self.level, self.config.width, self.config.height), (cam, aspect, wfar), &self.sun_rows()) } else { None };
		// [PC] Heat haze (heathaze.rs; Fire Medium / High): over the fires and the lava oceans, last in `mid`; not under water. Its
		// sources are only gathered while it may draw, and the fire cores once for the smoke and the haze (review N1 / N2).
		let haze_fire = if self.under.active() { 0 } else { crate::fire_option::level() };
		let haze_on = super::heathaze::wanted(haze_fire).is_some();
		let fire_cores = if smoke_on || haze_on { self.firefx.cores() } else { Vec::new() }; // [PC] volsmoke/fire_cut.rs
		if smoke_on { crate::diag::count(crate::diag::SMOKE_FIRES, fire_cores.len()); } // [PC] RAC_DIAG perf.csv
		let haze_lava = if haze_on { self.haze_lava() } else { Vec::new() };
		let haze_in = super::heathaze::Sources { fire_level: haze_fire, cores: &fire_cores, lava: &haze_lava, t: super::heathaze::clock(self.firefx.shown()), level: self.level };
		let haze_lin = self.post.water_inputs().map(|i| (i.1, i.3));
		let mark = water && gi_level > 0 && self.post.water_bound(); // [PC] tb17: post.rs `gi_skip` reads the scene as the water pass left it (only while GI runs)
		// [PC] volsmoke.rs runs before the fog, which covers it.
		let (volfog, volsmoke, under, heathaze, device, queue, depth) = (&mut self.volfog, &mut self.volsmoke, &mut self.under, &mut self.firefx.haze, &self.device, &self.queue, &self.depth);
		self.post.encode(&self.queue, &mut enc, cam, aspect, gfx.ao_q, gfx.sun_q, gfx.particles_q, part_on, &fx_lights, rays, gi_level, wfar, |enc, scene, ao| {
			if water {
				waterfx.shade(enc, scene, ao, fall_tex);
			}
			if mark {
				waterfx.mark_water(enc, scene);
			}
			if let Some(si) = &smoke_in {
				volsmoke.run(device, queue, enc, si, smoke_fx, &fire_cores); // [PC] lane dlit2: the fires of the surface light list (flitlist.rs `smoke`)
			}
			if let Some(fog) = &fog { volfog.run(device, queue, enc, fog, depth); }
			if let Some(uf) = &under_in {
				under.run(device, queue, enc, scene, depth, uf, waterfx.sky());
				// The underwater look counts as the water pass's (as before tb17; the smoke and the bounce light are off while it shows).
				if mark && under.active() {
					waterfx.mark_water(enc, scene);
				}
			}
			if let Some(lin) = haze_lin {
				heathaze.run(device, queue, enc, scene, lin, cam, aspect, &haze_in);
			}
		});
		self.upscale.set_heat(self.firefx.haze.take_mask()); // [PC] FSR 3: the hazed pixels go into the reactive mask (review S5)
		// [PC] Metal On: next frame's reflection source (metalenv.rs).
		self.metal_capture_ssr(enc, u);
		cuts.cut(&self.device, &mut enc, "f_pe");
		// [PC] Brightness / Contrast (grade_pass.rs): folded into the composite unless the upscaler slot's copy grades (it runs after).
		let grade = self.grade_k().filter(|_| self.upscale.hud_targets().is_none());
		self.post.composite(&self.device, &self.queue, &mut enc, &self.msaa, gi_level, grade);
		cuts.cut(&self.device, &mut enc, "f_post");
		self.upscale_encode(enc, cuts); // [PC] RAC_UPSCALE_PREP (upscale.rs): the upscaler inputs (+ object motion), where ReShade takes its own
		Some(self.hud_pass(enc, cuts, view, rs))
	}

	/// [PC] Heat haze: the boxes of this frame's lava oceans, from the ocean draws themselves (as pass_oceans draws them), so the haze
	/// needs no Water Enhanced (review S4). The level ocean only when it is drawn and no fog sheet; nothing in the warp.
	fn haze_lava(&self) -> Vec<[f32; 6]> {
		if self.scene.as_ref().is_none_or(|s| s.warp_shown) {
			return Vec::new();
		}
		let lava = |pos: &mut dyn Iterator<Item = [f32; 3]>| super::waterfx::lava_ocean_box(self.level, pos);
		let level_ocean = (self.ocean_tex.is_some() && !super::waterfx::fog_sheet_ocean(self.level)).then(|| lava(&mut self.ocean_draws.iter().filter(|s| s.pos.len() >= 3).flat_map(|s| s.pos.iter().copied()))).flatten();
		let ram = self.ocean_ram.iter().flat_map(|b| [(!b.idx.is_empty()).then_some(&b.verts), (!b.pidx.is_empty()).then_some(&b.pverts)]).flatten();
		level_ocean.into_iter().chain(ram.filter_map(|v| lava(&mut v.iter().map(|x| x.pos)))).collect()
	}
}
