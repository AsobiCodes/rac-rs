//! Draw steps before the render passes: uniforms and sun, water and particle flags, surface image and encoder, HUD quads and
//! indirect arguments. Each step is a contiguous block of the old `Gpu::draw`, in the same order.

use super::*;

impl Gpu {
	pub(super) fn frame_uniforms(&mut self, cam: &Camera) -> Option<DrawFrame> {
		self.tag_ice();
		self.cull_shrubs(cam);
		self.cull_ties();
		self.cull_tfrags();
		let scene = self.scene.as_ref()?;
		let aspect = self.config.width as f32 / self.config.height as f32;
		let eye = Camera { pos: [0.0; 3], yaw: cam.yaw, pitch: cam.pitch, tan_half: cam.tan_half, up: cam.up };
		let sp_un = super::profile::span("d_uniforms");
		// [PC] Draw distance High pushes the world far plane out (fardraw.rs); Low and Medium keep retail's.
		let wfar = self.world_far();
		let mut u = self.uniforms(cam, &eye, aspect, (RETAIL_NEAR, wfar));
		drop(sp_un);
		// [PC] Graphics -> Lighting Enhanced (post.rs): the level pass resolves into the post scene target, the post
		// passes grade it and the HUD pass draws over the result. Dynamic shadows On adds the cascades (sunshadow.rs) from the
		// level's key light. Classic: the frame below is drawn exactly as before (the shadow uniforms stay zero).
		let gfx = crate::retail::pcext::graphics();
		let post_on = crate::retail::splitview::fx(gfx.enhanced() && self.modern_ok); // [PC] split view: the one gate of every Enhanced pass below
		let sun = match post_on && gfx.shadows && !scene.warp_shown && !self.ship_fight {
			true => {
				let rows = self.sun_rows();
				let id = scene.world_verts.len() ^ (scene.radius.to_bits() as usize) << 20;
				self.sun.frame(cam, aspect, &rows, id, super::sunshadow::quality(gfx.shadows_q))
			}
			false => None,
		};
		super::sunshadow::write_uniforms(&mut u, sun.as_ref());
		// [PC] Enhanced flag for the shaders (`crisp_leaf`).
		u.sun_texel[3] = if post_on { 1.0 } else { 0.0 };
		u.sun_q[3] = crate::retail::pcext::texfilter::enabled(post_on) as u8 as f32;
		self.queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&super::super::volfog::fade_retail_fog(u)));
		// [PC] PREVIEW (vegfx.rs, RAC_VEG): more vegetation with Lighting Enhanced.
		let sp_veg = super::profile::span("d_veg");
		self.veg.prepare(&self.queue, scene, post_on && !scene.warp_shown && !self.ship_fight, cam.pos, &u.view_proj);
		drop(sp_veg);
		Some(DrawFrame { aspect, eye, wfar, u, gfx, post_on, sun })
	}

	pub(super) fn frame_effects(&mut self, cam: &Camera, dc: &DrawFrame) -> Option<DrawFx> {
		let scene = self.scene.as_ref()?;
		let (aspect, wfar, post_on, gfx, u) = (dc.aspect, dc.wfar, dc.post_on, dc.gfx, &dc.u);
		// [PC] Graphics -> Water Enhanced (waterfx.rs): the level pass records its water draws for the water pass.
		let water_on = post_on && gfx.water && !scene.warp_shown && !self.ship_fight;
		let mut wrec: Vec<super::waterfx::WaterDraw> = Vec::new();
		let strips_water = water_on && super::waterfx::strips_are_water(self.level);
		self.waterfx.clear_falls();
		let strip_flows = if strips_water { self.waterfx.strip_flows(&self.strip_draws) } else { Vec::new() };
		// [PC] Water Enhanced: a river drawn as effect quads (Batalia) goes to the water pass, grouped by its flow downstream.
		let river: Vec<usize> = match super::waterfx::river_fx_tex(self.level).filter(|t| water_on && self.fx_tex.contains_key(t)) {
			Some(t) => (0..self.fx_quads.len()).filter(|&i| !self.fx_quads[i].additive && crate::retail::efx_draw::plain_tex(self.fx_quads[i].tex) == t).collect(),
			None => Vec::new(),
		};
		let river_groups = if river.is_empty() { Vec::new() } else { self.waterfx.quad_flows(&river.iter().map(|&i| (self.fx_quads[i].pos, self.fx_quads[i].uv)).collect::<Vec<_>>()) };
		// [PC] Water Enhanced: waterfall sheets drawn as effect quads (Pokitaru) churn the water at their feet.
		let fall_tex = super::waterfx::fall_fx_tex(self.level);
		if water_on && !fall_tex.is_empty() {
			use crate::retail::efx_draw::{plain_tex, FX_REFILL};
			self.waterfx.sheet_falls(self.level, self.fx_quads.iter().filter(|q| fall_tex.contains(&(plain_tex(q.tex) & !FX_REFILL))).map(|q| (q.pos, q.uv, q.colour, q.tex & FX_REFILL != 0)));
		}
		// [PC] Graphics -> Particles Enhanced (partfx.rs): the level pass keeps its MSAA colour for the particle pass.
		let part_on = post_on && gfx.particles && !scene.warp_shown;
		// [PC] Graphics -> Sun rays On (post.rs `fs_rays`): from the sun the level's sky paints (`sky_sun`); off in ship fights.
		let rays = match post_on && gfx.sun_rays && !scene.warp_shown && !self.ship_fight {
			true => super::post::sky_sun(self.level).and_then(|to| super::post::sun_on_screen(&u.view_proj, cam.pos, to.map(|c| -c), 1.0)),
			false => None,
		};
		self.post.rays_tight = self.volclouds.sky.skip() != 0;
		let water_frame = water_on.then(|| {
			let rows = self.light_rows();
			let (a, b, t, _) = self.hero_env;
			super::waterfx::WaterFrame {
				vp: u.view_proj,
				eye: cam.pos,
				proj: [cam.tan_half * aspect, cam.tan_half, RETAIL_NEAR, wfar],
				key: super::sunshadow::key_light(&rows, a, b, t),
				fog: [u.fog_colour[0], u.fog_colour[1], u.fog_colour[2]],
				level: gfx.water_q,
				game: self.level,
				waves: gfx.waves,
			}
		});
		Some(DrawFx { water_on, wrec, strips_water, strip_flows, river, river_groups, part_on, rays, water_frame })
	}

	pub(super) fn frame_target(&mut self, cam: &Camera, dc: &DrawFrame) -> Option<(DrawTarget, Vec<Quad>)> {
		let scene = self.scene.as_ref()?;
		let (aspect, post_on, gfx, sun) = (dc.aspect, dc.post_on, dc.gfx, &dc.sun);
		self.write_warp_uniforms(cam, aspect);
		let warp_hud = self.warp_hud(self.config.width as f32, self.config.height as f32);
		super::profile::lap(super::profile::Phase::Cull);
		// [PC] Split view (split.rs): the Classic pass draws into the offscreen target and acquires / presents nothing.
		let frame = match crate::retail::splitview::classic() {
			true => None,
			false => Some(self.acquire_frame()?),
		};
		let view = match &frame {
			Some(f) => f.texture.create_view(&Default::default()),
			None => super::split::view()?,
		};
		let mut enc = self.device.create_command_encoder(&Default::default());
		if let Some(g) = &scene.tie_gpu {
			g.encode(&mut enc);
		}
		if let Some(h) = &scene.marker_gpu {
			h.record(&mut enc);
		}
		let mut cuts = super::presenter::Cuts::new();
		// [PC] Global illumination (post.rs `fs_gi`): with Lighting Enhanced, not under water (the underwater look replaces the light).
		let gi_level = if post_on && !self.under_flag && !self.under.active() { gfx.gi_q } else { 0 };
		if post_on {
			self.post.prepare(&self.device, self.config.width, self.config.height, &self.depth, gi_level);
			// [PC] Volumetric fog (volfog/soft.rs): the no-Z texel buffer emptied for this frame; a new one is rebound in group 0.
			if self.volfog.soft.prepare(&self.device, &mut enc, (self.config.width, self.config.height))
				&& let Some(soft) = self.volfog.soft.buffer()
			{
				self.ubind = self.device.create_bind_group(&wgpu::BindGroupDescriptor { label: Some("uniforms"), layout: self.volfog.soft.layout(), entries: &self.sun.entries_soft(&self.ubuf, soft) });
			}
		}
		if let Some(f) = &sun {
			let skin = Some((&self.props_gpu.bind, scene.props_sj.as_ref()));
			self.sun.encode(&self.device, &self.queue, &mut enc, scene, f, skin);
			cuts.cut(&self.device, &mut enc, "f_sun");
		}
		self.draw_item_panel(&mut enc, cam, scene);
		cuts.cut(&self.device, &mut enc, "f_pre");
		let rs = !crate::retail::splitview::classic() && self.reshade_begin();
		Some((DrawTarget { frame, view, enc, cuts, gi_level, rs }, warp_hud))
	}

	/// The next window image. The surface holds one acquired image at a time: the previous frame's present (on the present thread)
	/// must have returned before the next acquire, or wgpu panics "Surface image is already acquired" (seen 2026-10-19 on
	/// fg1/1_0016, dev s49). None = skip the frame (surface outdated / lost: reconfigured).
	fn acquire_frame(&self) -> Option<wgpu::SurfaceTexture> {
		self.presenter.flush();
		let acquire = crate::runlog::waiting("the GPU driver's next window image (vsync / swapchain)");
		let frame = match self.surface.get_current_texture() {
			wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
			wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
				self.presenter.flush();
				configure_surface(&self.surface, &self.device, &self.config);
				return None;
			}
			_ => return None,
		};
		drop(acquire);
		super::profile::lap(super::profile::Phase::Acquire);
		Some(frame)
	}

	pub(super) fn frame_buffers(&mut self, warp_hud: Vec<Quad>) -> Option<DrawBufs> {
		let scene = self.scene.as_ref()?;
		// Overlay quads to clip space, one draw per run of the same texture.
		let (sw, sh) = (self.config.width as f32, self.config.height as f32);
		crate::omni::draw::note_view(&self.hud, RETAIL_OV_TEX, sw, sh);
		let (mut hv, mut hi, mut hd) = (Vec::new(), Vec::new(), Vec::<(Draw, bool)>::new());
		// BRIDGE-36 (c4): the draw group of the first quad after the prompt model (`hud_split`).
		let (split, mut split_group) = (self.hud_split.filter(|&s| s <= self.hud.len()), None);
		for (qi, q) in self.hud.iter().chain(&warp_hud).enumerate() {
			let at_split = split == Some(qi);
			if at_split {
				split_group = Some(hd.len());
			}
			let [x, y, w, h] = q.rect;
			let base = hv.len() as u32;
			// [PC] Sharp title / main menu (hudsharp.rs): the quad's UV rectangle rides in the flat vertex light; off: zero as before.
			let sharp = super::hudsharp::wants(q.tex, self.hud_textures.len());
			let (cx, cy, (sa, ca)) = (x + w / 2.0, y + h / 2.0, q.angle.sin_cos());
			for (k, (px, py, u, v)) in [(x, y, q.uv[0], q.uv[1]), (x + w, y, q.uv[2], q.uv[1]), (x + w, y + h, q.uv[2], q.uv[3]), (x, y + h, q.uv[0], q.uv[3])].into_iter().enumerate() {
				// BRIDGE-74: four free corners (window pixels) replace the rotated rect, with the same UV pattern.
				let (px, py) = match q.corners {
					Some(c) => (c[k][0], c[k][1]),
					None => {
						let (dx, dy) = (px - cx, py - cy);
						(cx + dx * ca - dy * sa, cy + dx * sa + dy * ca)
					}
				};
				let [r, g, b, a] = q.colour;
				hv.push(Vertex { pos: [px / sw * 2.0 - 1.0, 1.0 - py / sh * 2.0, 0.0], uv: [u, v], normal: [0.0; 3], colour: [r, g, b, a.abs()], light: if sharp { q.uv } else { [0.0; 4] } });
			}
			let add = q.colour[3] < 0.0;
			if let Some((d, _)) = hd.last_mut().filter(|(d, a)| !at_split && d.texture == q.tex && *a == add && (d.inst != 0) == sharp) {
				d.count += 6;
			} else {
				// `inst` (unused by the HUD pipelines) marks a sharp group.
				hd.push((Draw { inst: sharp as u32, cull: 0, texture: q.tex, start: hi.len() as u32, count: 6 }, add));
			}
			hi.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
		}
		if split == Some(self.hud.len()) && split_group.is_none() {
			split_group = Some(hd.len());
		}
		// [PC] perf round 3 (bindless.rs): this frame's tie and shrub draw lists as indirect arguments (multi-draw runs).
		let sp_args = super::profile::span("d_args");
		let multi = match (&mut self.bindless, &scene.tex_array) {
			(Some(bl), Some(_)) if bl.indirect.is_some() => {
				let mut args = Vec::with_capacity(scene.ties.draws.len() + scene.shrub_opaque.len() + scene.shrub_faded.len());
				let ties = super::bindless::push_args(&mut args, scene.ties.draws.iter().copied(), true);
				let opaque = super::bindless::push_args(&mut args, scene.shrub_opaque.iter().map(|&k| scene.shrubs.draws[k]), true);
				let faded = super::bindless::push_args(&mut args, scene.shrub_faded.iter().map(|&k| scene.shrubs.draws[k]), false);
				bl.write_args(&self.device, &self.queue, &args);
				Some((ties, opaque, faded))
			}
			_ => None,
		};
		drop(sp_args);
		let sharp_pipes = hd.iter().any(|(d, _)| d.inst != 0).then(|| super::hudsharp::pipelines(&self.device, &self.pipeline, &self.tex_layout, self.config.format, self.depth_format));
		let hud_buffers = (!hv.is_empty()).then(|| {
			let vb = buffer(&self.device, "hud vertices", bytemuck::cast_slice(&hv), wgpu::BufferUsages::VERTEX);
			self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&hv));
			let ib = buffer(&self.device, "hud indices", bytemuck::cast_slice(&hi), wgpu::BufferUsages::INDEX);
			self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&hi));
			(vb, ib)
		});
		Some(DrawBufs { sw, sh, hd, split_group, multi, sharp_pipes, hud_buffers })
	}
}
