//! The water pass each frame: `positions` (meshes into the position target, uniforms, mirrors), `sea_waves`, `shade` (inside
//! the post chain) and the target accessors the level pass and post read.

use super::*;

impl WaterFx {
	/// The water's clock in seconds (RAC_WATER_TIME=T (dev aid): held at T seconds, same-time A/B pictures).
	pub(in crate::viewer) fn clock(&self) -> f32 {
		static FIXED: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
		FIXED.get_or_init(|| std::env::var("RAC_WATER_TIME").ok().and_then(|v| v.parse::<f32>().ok())).unwrap_or_else(|| crate::viewer::test_clock::secs(|| self.start.elapsed().as_secs_f32()))
	}

	/// The planar mirrors' heights steadied (`steady.rs`): retail's sea bobs in 50 Hz steps, the reflection must not.
	pub(in crate::viewer) fn steady_mirrors(&mut self, mirrors: Vec<f32>) -> Vec<f32> {
		let now = self.clock();
		self.heave.mir.filter(now, &mirrors)
	}

	/// The sky map's targets before the groups that hold its depth view (made once water is in view), the post-bound targets, and an
	/// empty shore map for a new level (it keeps bed / ground marks over frames for 4e9 of them).
	fn ready(&mut self, device: &wgpu::Device, enc: &mut wgpu::CommandEncoder, (depth, scene, lin, glow, generation): (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView, u64), game: usize) {
		self.sky.ensure(device);
		self.prepare(device, depth, scene, lin, glow, generation);
		if self.shore_game != game {
			enc.clear_buffer(&self.shore, 0, None);
			self.shore_game = game;
		}
	}

	/// After the level pass: the water meshes `draws` into the position target. Call before the post chain; `depth` is the level
	/// pass's MSAA depth, `scene` / `lin` / `glow` / `generation` the post's targets (`post::Post::water_inputs`); `parts`: the
	/// particle pass ran (Particles Enhanced), so the glow target holds their cover.
	#[allow(clippy::too_many_arguments)]
	pub(in crate::viewer) fn positions(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, enc: &mut wgpu::CommandEncoder, depth: &wgpu::TextureView, (scene, lin, glow, generation): (&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView, u64), f: &WaterFrame, draws: &[WaterDraw], mirrors: &[f32], parts: bool) {
		self.ready(device, enc, (depth, scene, lin, glow, generation), f.game);
		let Some(t) = &self.t else { return };
		let (w, h) = (t.key.0 as f32, t.key.1 as f32);
		let (sun, glint) = match f.key {
			// Travelling down (from above the horizon): the glint points back up the light.
			Some((d, luma)) if d[2] < -0.05 => ([-d[0], -d[1], -d[2]], GLINT * ((luma - 0.1) / 0.5).clamp(0.0, 1.0)),
			_ => ([0.0, 0.0, 1.0], 0.0),
		};
		let fog = f.fog.map(|c| c.max(0.0).powf(2.2));
		// RAC_WATER_TIME=T (dev aid): the water's clock held at T seconds (same-time A/B pictures).
		let now = self.clock();
		let boxes: Vec<[f32; 6]> = draws.iter().filter_map(|d| d.wake_box()).collect();
		if !std::mem::take(&mut self.falls_fresh) {
			self.falls.clear();
		}
		let (wake, splashes) = self.wakes.update(now, &boxes, &self.falls);
		for (p, size) in splashes {
			self.rings.splash(now, p, size);
		}
		let (rings, rn) = self.rings.update(now, draws.iter().filter_map(|d| d.plane));
		let hero = self.wakes.hero;
		// [PC] Geometric sea waves: the sea planes over which the camera is (one projected grid each, `sea_waves`), the grid
		// and the shore map round the camera.
		let look = geo_look(f.level).filter(|_| f.waves);
		let mut planes: Vec<[f32; 4]> = Vec::new();
		if look.is_some() {
			for d in draws.iter().filter(|d| d.sea()) {
				let z = d.plane.map_or(0.0, |b| b[0]);
				if f.eye[2] <= z + 0.05 {
					continue;
				}
				let bits = if d.kind == "sea fx quads" { 128.0 } else { 64.0 } + 512.0;
				let full = planes.len() >= GEO_PLANES;
				match planes.iter_mut().find(|p| (p[0] - z).abs() < 1.5) {
					Some(p) => p[1] = p[1].max(bits),
					None if !full => planes.push([z, bits, 0.0, 0.0]),
					None => {}
				}
			}
		}
		self.heave.steady_planes(now, &mut planes);
		let in_geo = |d: &WaterDraw| d.sea() && d.plane.is_some_and(|b| planes.iter().any(|p| (p[0] - b[0]).abs() < 1.5));
		let g = look.unwrap_or(GeoLook { cell: 8.0, band: 8.0, len: 9.0, sig: 0.0, sigq: 1.0, amp: 0.0 });
		let [mx, mb, mt] = GRID_MARGIN;
		let cols = ((w * (1.0 + mx) / g.cell).ceil() as u32).max(1);
		let rows = ((h * (1.0 + 0.5 * (mb + mt)) / g.cell).ceil() as u32).max(1);
		let ivp = super::mirror::inverse(&f.vp).unwrap_or(f.vp);
		let origin = [((f.eye[0] / SHORE_CELL).floor() - (SHORE_N / 2) as f32) * SHORE_CELL, ((f.eye[1] / SHORE_CELL).floor() - (SHORE_N / 2) as f32) * SHORE_CELL];
		let u = WaterU {
			vp: f.vp,
			eye: [f.eye[0], f.eye[1], f.eye[2], now % 3600.0],
			sun: [sun[0], sun[1], sun[2], glint],
			proj: f.proj,
			size: [w, h, 1.0 / w, 1.0 / h],
			look: [RIPPLES, REFRACTION, SHEEN, FOAM],
			fog: [fog[0], fog[1], fog[2], REFLECTION],
			mirror: [mirrors.first().copied().unwrap_or(0.0), !mirrors.is_empty() as u32 as f32, parts as u32 as f32, CLEAR],
			flows: std::array::from_fn(|k| if k == OPAQUE_SLOT as usize { [0.0; 4] } else if k == COLD_SLOT as usize { [0.0, 0.0, 0.0, COLD_LOOK] } else if k == FROZEN_SLOT as usize { [0.0, 0.0, 0.0, -1.0] } else if k == FROZEN_CLEAR_SLOT as usize { [0.0, 0.0, 1.0, -1.0] } else if k == LAVA_SLOT as usize { [0.0, 0.0, 0.0, -2.0] } else if k == MUD_SLOT as usize { [0.0, 0.0, 0.0, -4.0] } else if k == SLUDGE_SLOT as usize { [0.0, 0.0, 0.0, -5.0] } else { self.flows.get(k).map_or([0.0, 0.0, 1.0, 0.0], |f| if self.phased && k > 0 { [f[0], f[1], RIVER_CLEAR, -3.0] } else { [f[0], f[1], 1.0, 0.0] }) }),
			rn: [rn as f32, wake.n as f32, wake.cn as f32, march_steps(f.level)],
			rings,
			trail: wake.trail,
			wbox: wake.wbox,
			churn: wake.churn,
			hero: hero.map_or([0.0; 4], |h| [h[0], h[1], h[2], 1.0]),
			wave: wave_look(f.level),
			ivp,
			geo: std::array::from_fn(|k| planes.get(k).copied().unwrap_or([0.0; 4])),
			gmap: [origin[0], origin[1], SHORE_CELL, SHORE_N as f32],
			ggrid: [cols as f32, rows as f32, g.cell, g.band],
			gridb: [-1.0 - mx, -1.0 - mb, (2.0 + 2.0 * mx) / cols as f32, (2.0 + mb + mt) / rows as f32],
			wgeo: [g.sig, g.sigq, g.amp, g.len],
			mirror2: [mirrors.get(1).copied().unwrap_or(0.0), mirrors.get(2).copied().unwrap_or(0.0), mirrors.len() as f32, 0.0],
		};
		queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&u));
		self.geo_draw = None;
		if !planes.is_empty() {
			let top = self.heave.shore.at(f.game, planes.iter().map(|p| p[0]).fold(f32::MIN, f32::max)); // tide-free (`shore_ref.rs`)
			queue.write_buffer(&self.shore_u, 0, bytemuck::bytes_of(&ShoreU { ivp, vp: f.vp, map: [origin[0], origin[1], SHORE_CELL, top + SHORE_CAP], size: [SHORE_STRIDE as f32, SHORE_N as f32, top - SHORE_NEAR, top + SHORE_HIGH] }));
			if self.grid_ib.as_ref().is_none_or(|g| (g.1, g.2) != (cols, rows)) {
				let vc = cols + 1;
				let idx: Vec<u32> = (0..rows).flat_map(|j| (0..cols).flat_map(move |i| {
					let a = j * vc + i;
					[a, a + 1, a + vc, a + 1, a + vc + 1, a + vc]
				})).collect();
				let ib = device.create_buffer(&wgpu::BufferDescriptor { label: Some("water sea grid"), size: std::mem::size_of_val(&idx[..]) as u64, usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
				queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
				self.grid_ib = Some((ib, cols, rows));
			}
			self.geo_draw = Some((planes.len() as u32, cols * rows * 6));
			// The flat sea where it is visible: its height per pixel (`fs_mask`).
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("water sea mask"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.seamask, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color { r: -1e9, g: 0.0, b: 0.0, a: 0.0 }), store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: crate::viewer::passtime::rw("water sea mask"),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_pipeline(&self.p_mask);
			pass.set_bind_group(0, &t.g_pos, &[]);
			for d in draws.iter().filter(|d| in_geo(d)) {
				pass.set_vertex_buffer(0, d.vb.slice(..));
				pass.set_index_buffer(d.ib.slice(..), wgpu::IndexFormat::Uint32);
				pass.draw_indexed(d.first..d.first + d.n, 0, 0..1);
			}
		}
		// [PC] The waterfalls (`fs_fall`): the sheets, then over their feet the foam discs (hiding where a sheet meets the
		// water), the mist and the spray.
		let mut fv = std::mem::take(&mut self.fall_verts);
		self.fall = None;
		if !fv.is_empty() {
			// Each foot on the water it falls into (retail's sheets reach on under it: the nearest flat water at or above it);
			// one deeper under another water (Pokitaru's pond: its sheets reach on down to the sea under it) is hidden.
			let planes: Vec<[f32; 5]> = draws.iter().filter_map(|d| d.plane).collect();
			let feet: Vec<[f32; 3]> = self
				.falls
				.iter()
				.filter_map(|&f| {
					let on = |b: &&[f32; 5], m: f32| f[0] >= b[1] - m && f[0] <= b[3] + m && f[1] >= b[2] - m && f[1] <= b[4] + m;
					if planes.iter().any(|b| on(&b, 0.0) && b[0] > f[2] + FALL_DEEP) {
						return None;
					}
					let z = planes.iter().filter(|b| on(b, 1.0) && b[0] >= f[2] - 0.5 && b[0] <= f[2] + FALL_DEEP).map(|b| b[0]).min_by(f32::total_cmp);
					Some([f[0], f[1], z.unwrap_or(f[2])])
				})
				.collect();
			let quad = |pos: [f32; 3], kind: f32, size: f32, seed: f32| [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]].map(|uv| Vertex { pos, uv, normal: [kind, size, seed], colour: [1.0; 4], light: [0.0; 4] });
			let seed_of = |key: (i32, i32, i32)| ((key.0.wrapping_mul(73856093) ^ key.1.wrapping_mul(19349663) ^ key.2.wrapping_mul(83492791)) & 1023) as f32 / 1024.0;
			let key_of = |f: &[f32; 3], cell: f32| ((f[0] / cell).floor() as i32, (f[1] / cell).floor() as i32, (f[2] / 2.0).floor() as i32);
			let mut over = Vec::new();
			// A foam disc on every foot (seeded by its churn cell, so it holds still whatever order retail draws the sheets in).
			for f in feet.iter().take(FALL_BOILS) {
				over.extend(quad(*f, 3.0, FALL_BOIL_SIZE, seed_of(key_of(f, CHURN_CELL))));
			}
			// The feet in one FALL_PUFF_MERGE square share one set of mist and spray (a wide fall has a foot every churn cell).
			let mut spots: std::collections::BTreeMap<(i32, i32, i32), ([f32; 3], f32)> = std::collections::BTreeMap::new();
			for f in &feet {
				let e = spots.entry(key_of(f, FALL_PUFF_MERGE)).or_insert(([0.0; 3], 0.0));
				e.0 = [e.0[0] + f[0], e.0[1] + f[1], e.0[2] + f[2]];
				e.1 += 1.0;
			}
			for (key, (sum, n)) in spots.iter().take(FALL_SPOTS) {
				let f = sum.map(|v| v / n);
				let seed = seed_of(*key);
				for k in 0..FALL_PUFFS {
					over.extend(quad(f, 1.0, FALL_PUFF_SIZE, (seed + k as f32 * 0.618034).fract()));
				}
				for k in 0..FALL_DROPS {
					over.extend(quad(f, 2.0, FALL_DROP_SIZE, (seed * 3.7 + k as f32 * 0.381966).fract()));
				}
			}
			fv.append(&mut over);
			let idx: Vec<u32> = (0..fv.len() as u32 / 4).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
			let vb = device.create_buffer(&wgpu::BufferDescriptor { label: Some("water falls"), size: std::mem::size_of_val(&fv[..]) as u64, usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
			queue.write_buffer(&vb, 0, bytemuck::cast_slice(&fv));
			let ib = device.create_buffer(&wgpu::BufferDescriptor { label: Some("water falls"), size: std::mem::size_of_val(&idx[..]) as u64, usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST, mapped_at_creation: false });
			queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
			self.fall = Some((vb, ib, idx.len() as u32));
		}
		fv.clear();
		self.fall_verts = fv;
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("water positions"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.wpos, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT), store: wgpu::StoreOp::Store } })],
			// (Kept for the geometric sea's grid, `sea_waves`.)
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.wdepth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: if planes.is_empty() { wgpu::StoreOp::Discard } else { wgpu::StoreOp::Store } }), stencil_ops: None }),
			timestamp_writes: crate::viewer::passtime::rw("water positions"),
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.p_pos);
		pass.set_bind_group(0, &t.g_pos, &[]);
		// RAC_WATER_CLASS=1 (dev aid): the sea / lake class of every water draw, every 100th call.
		static CALLS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
		if std::env::var("RAC_WATER_CLASS").is_ok() && CALLS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) % 100 == 50 {
			for d in draws {
				eprintln!("waterclass: kind {:?} n {} flow {} z {:?} span {:?} sea {} lake {} box {:?} eye {:?} hero {:?}", d.kind, d.n, d.flow, d.plane.map(|b| b[0]), d.plane.map(|b| (b[3] - b[1]).max(b[4] - b[2])), d.sea(), d.lake(), d.plane, f.eye, hero);
			}
		}
		// (The geometric sea is the grid's, `sea_waves`.)
		for d in draws.iter().filter(|d| !in_geo(d)) {
			pass.set_vertex_buffer(0, d.vb.slice(..));
			pass.set_index_buffer(d.ib.slice(..), wgpu::IndexFormat::Uint32);
			// The instance is the flow slot, plus FLOWS on open sea (`swell`).
			// Plus another FLOWS for the sea drawn as effect quads (`fs_ao_sea`).
			// Plus 256 on a lake (`WaterDraw::lake`).
			let k = d.flow + if d.sea() { FLOWS as u32 * if d.kind == "sea fx quads" { 2 } else { 1 } } else if d.lake() { 256 } else { 0 };
			pass.draw_indexed(d.first..d.first + d.n, 0, k..k + 1);
		}
	}

	/// The targets (MSAA colour, resolve layer `k`, MSAA depth) of mirror pass `k` for `Gpu::draw_mirror`; after `positions`.
	pub(in crate::viewer) fn mirror_target(&self, k: usize) -> Option<(&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView)> {
		self.t.as_ref().and_then(|t| Some((&t.mirror_ms, t.mirror_layers.get(k)?, &t.mirror_depth)))
	}

	/// [PC] Geometric sea waves, after `positions` and `Gpu::draw_bed`: the shore map from the bed's depth, then the projected
	/// grid of each sea plane into the position target (nearest surface wins against the other water).
	pub(in crate::viewer) fn sea_waves(&self, enc: &mut wgpu::CommandEncoder) {
		let (Some(t), Some((planes, n)), Some((ib, ..))) = (&self.t, self.geo_draw, &self.grid_ib) else { return };
		// (The first four quarters: the last four keep the map over frames, `cs_shore_keep`.)
		enc.clear_buffer(&self.shore, 0, Some((4 * SHORE_N * SHORE_N * 4) as u64));
		{
			let mut cp = enc.begin_compute_pass(&wgpu::ComputePassDescriptor { label: Some("water shore map"), timestamp_writes: crate::viewer::passtime::cw("water shore map") });
			cp.set_pipeline(&self.p_shore);
			cp.set_bind_group(0, &t.g_shore, &[]);
			let (bw, bh) = ((t.key.0 / 2).max(1).div_ceil(SHORE_STRIDE), (t.key.1 / 2).max(1).div_ceil(SHORE_STRIDE));
			cp.dispatch_workgroups(bw.div_ceil(8), bh.div_ceil(8), 1);
			// [PC] The map kept over frames (`cs_shore_keep`), then the distance map (`cs_shore_h`, `cs_shore_v`).
			for p in [&self.p_shore_keep, &self.p_shore_h, &self.p_shore_v] {
				cp.set_pipeline(p);
				cp.set_bind_group(0, &t.g_shore, &[]);
				cp.dispatch_workgroups(SHORE_N.div_ceil(8), SHORE_N.div_ceil(8), 1);
			}
		}
		self.dry.dispatch(enc, &t.dry);
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("water sea grid"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &t.wpos, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment { view: &t.wdepth, depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Discard }), stencil_ops: None }),
			timestamp_writes: crate::viewer::passtime::rw("water sea grid"),
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.p_grid);
		pass.set_bind_group(0, &t.g_grid, &[]);
		pass.set_bind_group(1, &t.dry.g_in, &[]);
		pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
		pass.draw_indexed(0..n, 0, 0..planes);
	}

	/// The water bed's targets (MSAA colour, resolve, MSAA depth) for `Gpu::draw_bed`; after `positions`.
	pub(in crate::viewer) fn bed_target(&self) -> Option<(&wgpu::TextureView, &wgpu::TextureView, &wgpu::TextureView)> {
		self.t.as_ref().map(|t| (&t.bed_ms, &t.bed, &t.bed_depth))
	}

	/// [PC] Global illumination (post.rs `bind_water`): the position target and the scene copy `shade` takes before it draws.
	pub(in crate::viewer) fn gi_mask(&self) -> Option<(&wgpu::TextureView, &wgpu::Texture)> {
		self.t.as_ref().map(|t| (&t.wpos, &t.copy))
	}

	/// Inside the post chain (after the occlusion, before the bloom): shades the water pixels of the post scene `scene`;
	/// `fall_tex`: the bind group of the level's waterfall texture (`fall_fx_tex`), if any.
	pub(in crate::viewer) fn shade(&self, enc: &mut wgpu::CommandEncoder, scene: &wgpu::TextureView, ao: Option<&wgpu::TextureView>, fall_tex: Option<&wgpu::BindGroup>) {
		let Some(t) = &self.t else { return };
		// [PC] No screen-space occlusion on the far open sea (`fs_ao_sea`).
		if let Some(ao) = ao {
			let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("water sea occlusion"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: ao, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: crate::viewer::passtime::rw("water sea occlusion"),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			pass.set_pipeline(&self.p_ao_sea);
			pass.set_bind_group(0, &t.g_shade, &[]);
			pass.draw(0..3, 0..1);
		}
		enc.copy_texture_to_texture(scene.texture().as_image_copy(), t.copy.as_image_copy(), t.copy.size());
		let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
			label: Some("water shade"),
			color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: scene, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store } })],
			depth_stencil_attachment: None,
			timestamp_writes: crate::viewer::passtime::rw("water shade"),
			occlusion_query_set: None,
			multiview_mask: None,
		});
		pass.set_pipeline(&self.p_shade);
		pass.set_bind_group(0, &t.g_shade, &[]);
		pass.draw(0..3, 0..1);
		if let Some((vb, ib, n)) = &self.fall {
			pass.set_pipeline(&self.p_fall);
			pass.set_bind_group(1, fall_tex.unwrap_or(&self.fall_dummy), &[]);
			pass.set_vertex_buffer(0, vb.slice(..));
			pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
			pass.draw_indexed(0..*n, 0, 0..1);
		}
	}
}
