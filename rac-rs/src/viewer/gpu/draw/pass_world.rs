//! Level-pass steps that record the world: sky shells and sprites, the opaque world (terrain, ties, shrubs, props, mobies,
//! shadow volumes, ABE texels), and the vendor shop models.

use super::*;

impl Gpu {
	pub(in crate::viewer) fn pass_sky(&self, mut pass: &mut wgpu::RenderPass<'_>, scene: &Scene, warp: bool, skip: u64, star: bool) {
			// The sky first, as retail: the sky pass FUN_0023fbb8 is the first draw of the frame render (aridia_presentation.md;
			// GSPIX play/eudora1_0039 (384,37): sky texel, then the tfrag texel blended over it). It writes no Z, so every world draw
			// covers it, and blended world texels (tfrag texels below alpha 0x60 write no Z) mix with the sky, not the clear colour
			// (Eudora cliff undersides, fx_census_late.md run 21). ARI-4 (FUN_00289d78): shells 0 and 1, the sky sprites (additive),
			// then shells 2 and 3.
			if let Some(sky) = scene.sky.as_ref().filter(|_| !warp && !dskip("sky")) {
				self.volclouds.sky.backdrop(pass); // [PC] Volumetric clouds: the new sky's backdrop first (volclouds/skymodel.rs)
				let split = scene.sky_shells.as_ref().map_or(sky.draws.len(), |s| s.split.min(sky.draws.len()));
				let shells = |pass: &mut wgpu::RenderPass, draws: &[Draw], at: usize| { // [PC] `skip`: shell draws the volumetric clouds replace (volclouds.rs)
					pass.set_pipeline(&self.sky_pipeline);
					pass.set_vertex_buffer(0, sky.vbuf.slice(..));
					pass.set_index_buffer(sky.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					for d in draws.iter().enumerate().filter(|(i, _)| (skip >> (at + i).min(63)) & 1 == 0).map(|(_, d)| d) {
						if self.star_draw(pass, scene, d) { continue; } // [PC] Starfield (starfield.rs): the star texture's shell draw, Enhanced
						pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
						let animated = star && self.volclouds.sky.star.begin(pass, d.texture); // [PC] Gemlik's animated star (viewer/star.rs)
						pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
						if animated { pass.set_pipeline(&self.sky_pipeline); }
					}
				};
				shells(&mut pass, &sky.draws[..split], 0);
				if let Some(sp) = scene.sky_sprites.as_ref().filter(|sp| sp.counts.iter().any(|&c| c > 0)) {
					pass.set_pipeline(&self.sky_add_pipeline);
					pass.set_vertex_buffer(0, sp.vbuf.slice(..));
					pass.set_index_buffer(sp.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					let mut first = 0;
					for (k, &n) in sp.counts.iter().enumerate() {
						if n > 0 {
							pass.set_bind_group(1, &scene.bind_groups[sp.bind[k]], &[]);
							pass.draw_indexed(first * 6..(first + n) * 6, 0, 0..1);
						}
						first += n;
					}
				}
				shells(&mut pass, &sky.draws[split..], split);
			}
	}

	pub(super) fn pass_world(&self, mut pass: &mut wgpu::RenderPass<'_>, scene: &Scene, warp: bool, dc: &DrawFrame, db: &DrawBufs, water_on: bool, wrec: &mut Vec<super::waterfx::WaterDraw>) {
		let (post_on, sun) = (dc.post_on, &dc.sun);
		let multi = &db.multi;
		// Moby draws: the material's TEX1 word (`moby_word`), so a moby texel takes the level the PS2 picks, only for the Classic
		// picture; Lighting Enhanced keeps the sharper plain sample (user, 2026-10-16).
		let moby_inst = move |d: &Draw| d.inst | if post_on { 0 } else { scene.slot_words.get(d.texture).copied().unwrap_or(0) };
			let warp_batch = scene.warp_batch.as_ref().filter(|_| warp);
			let marker = (scene.marker_shown && !warp).then_some(&scene.marker);
			let props = (scene.props_shown && !warp).then_some(&scene.props);
			let debris = scene.debris.as_ref().filter(|_| scene.props_shown && !warp);
			let clipped = (scene.props_shown && !warp && !scene.clip_batch.draws.is_empty()).then_some(&scene.clip_batch);
			let skinned = scene.props_sj.as_ref().map(|sj| (&self.props_gpu, sj));
			let array = self.bindless.as_ref().zip(scene.tex_array.as_ref());
			// BRIDGE-64a: with volumes this frame the deferred-list slots leave the opaque props draws for the redraw after them.
			// M3a shadows: in retail mode a frame whose only deferred caster is the hero still draws its volumes.
			// [PC] Dynamic shadows replace the retail shadow volumes (no double shadows).
			let volumes = self.volume_buf.as_ref().filter(|_| sun.is_none() && self.volume_counts != (0, 0) && (props.is_some() && !scene.props_late.is_empty() || self.shadow.marker_late && marker.is_some()));
			let marker_late = marker.filter(|_| volumes.is_some() && self.shadow.marker_late);
			let (props_main, props_late) = match volumes {
				Some(_) => super::shadowvol::split_late(&scene.props.draws, &scene.slot_runs, &scene.props_late),
				None => (Vec::new(), Vec::new()),
			};
			// Retail frame order: the tfrag pass, then the tie pass (FUN_0021af28: tfrag_frame, then TieProc), so a blended tie texel
			// mixes with the terrain behind it. The texels that write no Z (tfrag below alpha 0x60, tie below 0x40) come last.
			// `tie`: Some(pass) for the two tie passes (`tie_pipelines`).
			for (bi, (pipeline, batch, tie)) in [(&self.pipeline, Some(&scene.world).filter(|_| !warp), None), (&self.tfrag_pipeline, Some(&scene.tfrags).filter(|_| !warp), None), (&self.pipeline, Some(&scene.ties).filter(|_| !warp), Some(0)), (&self.pipeline, marker.filter(|_| marker_late.is_none()), None), (&self.pipeline, props, None), (&self.pipeline, clipped, None), (&self.pipeline, debris, None), (&self.pipeline, warp_batch, None), (&self.tfrag_pipeline_lt, Some(&scene.tfrags).filter(|_| !warp), None), (&self.pipeline, Some(&scene.ties).filter(|_| !warp), Some(1))].into_iter().enumerate() {
				let Some(batch) = batch else { continue };
				if dskip(["world", "tfrags", "ties", "marker", "props", "clipped", "debris", "warp", "tfrags", "ties"][bi]) {
					continue;
				}
				if let Some(tp) = tie {
					pass.set_vertex_buffer(0, batch.vbuf.slice(..));
					pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					// [PC] perf round 3 (bindless.rs): the tie draws through the texture array (first instance = slot | word << 16).
					if let Some((bl, tex_array)) = array {
						pass.set_bind_group(1, tex_array, &[]);
						if let Some((ties, _, _)) = &multi {
							bl.multi_draw_set(&mut pass, ties, &bl.tie[tp]);
							continue;
						}
						for d in &batch.draws {
							pass.set_pipeline(&bl.tie[tp][d.cull as usize]);
							let inst = super::bindless::first_instance(d);
							pass.draw_indexed(d.start..d.start + d.count, 0, inst..inst + 1);
						}
						continue;
					}
					for d in &batch.draws {
						pass.set_pipeline(&self.tie_pipelines[tp][d.cull as usize]);
						pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
						pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
					}
					continue;
				}
				// [PC] perf round 3: the props batch is posed by `vs_props` (gpuskin.rs).
				let (pipeline, cull) = match skinned.filter(|_| std::ptr::eq(batch, &scene.props)) {
					Some((pg, sj)) => {
						pass.set_bind_group(2, &pg.bind, &[]);
						pass.set_vertex_buffer(1, sj.slice(..));
						(&pg.pipeline, &pg.pipeline_cull)
					}
					None => (pipeline, &self.pipeline_cull),
				};
				pass.set_pipeline(pipeline);
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				let draws: &[Draw] = if volumes.is_some() && std::ptr::eq(batch, &scene.props) { &props_main } else { &batch.draws };
				// The moby batches (marker, props, clipped props, debris) carry the material's TEX1 K word as the first instance
				// (`moby_word`); the world and tfrag batches keep theirs (the tfrag pass reads its own word from it).
				let moby_pass = matches!(bi, 3 | 4 | 5 | 6);
				for d in draws {
					if d.cull != 0 {
						pass.set_pipeline(&cull[d.cull as usize - 1]);
					}
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					let inst = if moby_pass { moby_inst(d) } else { d.inst };
					pass.draw_indexed(d.start..d.start + d.count, 0, inst..inst + 1);
					if d.cull != 0 {
						pass.set_pipeline(pipeline);
					}
				}
			}
			// BRIDGE-45 (NOV-35b): the props' metal faces (chrome / glass, clamp sampler) after the LOD batches.
			let metal = self.metal.props.as_ref().zip(self.metal.tex.as_ref()).filter(|_| scene.props_shown && !warp);
			self.pass_pools(&mut pass, warp, metal, water_on, wrec);
			self.pass_shrubs(&mut pass, scene, warp, array, multi);
			// BRIDGE-64a: the shadow volumes and the darken, then the deferred-list slots drawn opaque over them.
			if let Some(vb) = volumes.filter(|_| !dskip("vol")) {
				pass.set_bind_group(0, &self.ubind, &[]);
				self.shadow.draw(&mut pass, vb, self.volume_counts);
			}
			// M3a shadows: the hero on retail's deferred path, drawn opaque after the darken.
			if let Some(batch) = marker_late.filter(|_| !dskip("hlate")) {
				pass.set_pipeline(&self.pipeline);
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				for d in &batch.draws {
					pass.set_pipeline(if d.cull != 0 { &self.pipeline_cull[d.cull as usize - 1] } else { &self.pipeline });
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					let inst = moby_inst(d);
					pass.draw_indexed(d.start..d.start + d.count, 0, inst..inst + 1);
				}
			}
			if let (Some(_), Some(batch)) = (volumes.filter(|_| !dskip("plate")), props) {
				let (pipeline, cull) = match skinned {
					Some((pg, sj)) => {
						pass.set_bind_group(2, &pg.bind, &[]);
						pass.set_vertex_buffer(1, sj.slice(..));
						(&pg.pipeline, &pg.pipeline_cull)
					}
					None => (&self.pipeline, &self.pipeline_cull),
				};
				pass.set_pipeline(pipeline);
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				for d in &props_late {
					pass.set_pipeline(if d.cull != 0 { &cull[d.cull as usize - 1] } else { pipeline });
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					let inst = moby_inst(d);
					pass.draw_indexed(d.start..d.start + d.count, 0, inst..inst + 1);
				}
			}
			// Retail draws every moby texel through one ABE pass: texels below alpha 0x80 blend here (MOBY-GS).
			if !dskip("abe") && (scene.props_shown || scene.marker_shown || warp) {
				for batch in [marker, props, clipped, debris, warp_batch].into_iter().flatten() {
					pass.set_vertex_buffer(0, batch.vbuf.slice(..));
					pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					let (pipelines, culled) = match skinned.filter(|_| std::ptr::eq(batch, &scene.props)) {
						Some((pg, sj)) => {
							pass.set_bind_group(2, &pg.bind, &[]);
							pass.set_vertex_buffer(1, sj.slice(..));
							(&pg.moby_pipelines, &pg.moby_cull)
						}
						None => (&self.moby_pipelines, &self.moby_cull),
					};
					for (k, pipeline) in pipelines.iter().enumerate() {
						pass.set_pipeline(pipeline);
						for d in &batch.draws {
							// Retail culls in VU1 before the GS: the blended texels of a culled packet keep its cull mode.
							if d.cull != 0 {
								pass.set_pipeline(&culled[2 * (d.cull as usize - 1) + k]);
							}
							pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
							let inst = moby_inst(d);
							pass.draw_indexed(d.start..d.start + d.count, 0, inst..inst + 1);
							if d.cull != 0 {
								pass.set_pipeline(pipeline);
							}
						}
					}
				}
				// BRIDGE-45: the metal texels below alpha 0x80 and every metal face drawn with a metal alpha below 0x80.
				if let Some((batch, tex)) = metal {
					pass.set_vertex_buffer(0, batch.vbuf.slice(..));
					pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
					for pipeline in &self.moby_pipelines {
						pass.set_pipeline(pipeline);
						for d in &batch.draws {
							pass.set_bind_group(1, &tex[d.texture], &[]);
							pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
						}
					}
				}
			}
	}

	pub(super) fn pass_shop_models(&self, pass: &mut wgpu::RenderPass<'_>, scene: &Scene, cam: &Camera, eye: &Camera, aspect: f32, wfar: f32, warp: bool) {
			// NOV-13.3: the vendor shop's item models, world mobies seen by the world camera (decomp/notes/vendor.md
			// "NOV-13.3"). Their meshes are the model-space preview batches, so the view-projection takes the model
			// matrix and the eye, the point lights and the light direction move into the model frame. Lit by the
			// preview's fixed menu light [P] (the shop slots' moby lighting is not mapped).
			for (k, (class, m)) in self.shop_models.iter().take(if warp { 0 } else { 2 }).enumerate() {
				let Some((batch, ..)) = scene.previews.get(class) else { continue };
				let slot = 2 + k;
				let mut pu = self.uniforms(cam, &eye, aspect, (RETAIL_NEAR, wfar));
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
				let (l, c) = (MENU_LIGHT_DIR, MENU_LIGHT_COLOUR);
				let to_light = [0, 1, 2].map(|k| -(m[4 * k] * l[0] + m[4 * k + 1] * l[1] + m[4 * k + 2] * l[2]));
				pu.lights[MENU_LIGHT_SLOT] = [[c[0], c[1], c[2], -0.25], [to_light[0], to_light[1], to_light[2], 0.0], [0.0; 4], [0.0; 4]];
				self.queue.write_buffer(&self.pbuf[slot], 0, bytemuck::bytes_of(&pu));
				pass.set_pipeline(&self.pipeline);
				pass.set_bind_group(0, &self.pbind[slot], &[]);
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				for d in &batch.draws {
					pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
				}
				// Texels below alpha 0x80 blend (MOBY-GS), as in the scene pass.
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
