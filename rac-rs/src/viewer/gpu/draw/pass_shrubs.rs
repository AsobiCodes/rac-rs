//! Level-pass step for the vegetation: the shrub meshes, the billboards (two passes) and the extra plants (vegfx).

use super::*;

impl Gpu {
	pub(super) fn pass_shrubs(&self, mut pass: &mut wgpu::RenderPass<'_>, scene: &Scene, warp: bool, array: Option<(&super::bindless::Bindless, &wgpu::BindGroup)>, multi: &Option<(super::bindless::Runs, super::bindless::Runs, super::bindless::Runs)>) {
			crate::viewer::passtime::lap(pass, "pools");
			// REN-CULL-SHRUB: the shrub meshes after the sky (they are depth tested, and the faded / billboard fragments write no Z
			// everywhere), then the billboards: pass 1 with the Z-writing split, pass 2 never writing Z.
			if !warp && !dskip("shrub") && (!scene.shrub_opaque.is_empty() || !scene.shrub_faded.is_empty()) {
				pass.set_vertex_buffer(0, scene.shrubs.vbuf.slice(..));
				pass.set_index_buffer(scene.shrubs.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				// [PC] perf round 3 (bindless.rs): one texture-array bind group, the slot as the instance index.
				match array {
					Some((bl, tex_array)) if multi.is_some() => {
						pass.set_bind_group(1, tex_array, &[]);
						let (_, opaque, faded) = multi.as_ref().unwrap();
						bl.multi_draw(&mut pass, opaque, None);
						for p in &bl.shrub_faded {
							bl.multi_draw(&mut pass, faded, Some(p));
						}
					}
					Some((bl, tex_array)) => {
						pass.set_bind_group(1, tex_array, &[]);
						pass.set_pipeline(&bl.pipeline);
						for &k in &scene.shrub_opaque {
							let d = scene.shrubs.draws[k];
							if d.cull != 0 {
								pass.set_pipeline(&bl.pipeline_cull[d.cull as usize - 1]);
							}
							pass.draw_indexed(d.start..d.start + d.count, 0, d.texture as u32..d.texture as u32 + 1);
							if d.cull != 0 {
								pass.set_pipeline(&bl.pipeline);
							}
						}
						for p in &bl.shrub_faded {
							pass.set_pipeline(p);
							for &k in &scene.shrub_faded {
								let d = scene.shrubs.draws[k];
								let inst = super::bindless::first_instance(&d);
								pass.draw_indexed(d.start..d.start + d.count, 0, inst..inst + 1);
							}
						}
					}
					None => {
						pass.set_pipeline(&self.pipeline);
						for &k in &scene.shrub_opaque {
							let d = scene.shrubs.draws[k];
							if d.cull != 0 {
								pass.set_pipeline(&self.pipeline_cull[d.cull as usize - 1]);
							}
							pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
							pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
							if d.cull != 0 {
								pass.set_pipeline(&self.pipeline);
							}
						}
						for p in &self.shrub_pipelines[0..2] {
							pass.set_pipeline(p);
							for &k in &scene.shrub_faded {
								let d = scene.shrubs.draws[k];
								pass.set_bind_group(1, &scene.bind_groups[d.texture], &[]);
								pass.draw_indexed(d.start..d.start + d.count, 0, d.inst..d.inst + 1);
							}
						}
					}
				}
			}
			crate::viewer::passtime::lap(pass, "shrub meshes");
			if !warp && !dskip("bb") && !scene.shrub_bb_quads.is_empty() {
				let mut verts = Vec::with_capacity(scene.shrub_bb_quads.len() * 4);
				let mut runs: Vec<(u32, bool, u32, u32)> = Vec::new();
				for q in &scene.shrub_bb_quads {
					let base = verts.len() as u32 / 4 * 6;
					for (k, uv) in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]].into_iter().enumerate() {
						verts.push(Vertex { pos: q.pos[k], uv, normal: [q.lod_k, q.mxl, 0.0], colour: q.colour, light: [0.0, 0.0, 0.0, -1.0] });
					}
					match runs.last_mut() {
						Some(r) if r.0 == q.class_id && r.1 == q.pass1 => r.3 += 6,
						_ => runs.push((q.class_id, q.pass1, base, 6)),
					}
				}
				let idx: Vec<u32> = (0..verts.len() as u32 / 4).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
				let vb = buffer(&self.device, "shrub billboard vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
				self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
				let ib = buffer(&self.device, "shrub billboard indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
				self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
				pass.set_vertex_buffer(0, vb.slice(..));
				pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
				for (pipes, want) in [(&self.shrub_pipelines[2..4], true), (&self.shrub_pipelines[4..5], false)] {
					for p in pipes {
						pass.set_pipeline(p);
						for &(class, pass1, start, count) in runs.iter().filter(|r| r.1 == want) {
							let _ = pass1;
							pass.set_bind_group(1, &scene.shrub_bb_tex[&class].0, &[]);
							pass.draw_indexed(start..start + count, 0, 0..1);
						}
					}
				}
			}
			crate::viewer::passtime::lap(pass, "shrub billboards");
			// [PC] PREVIEW (vegfx.rs): the extra plants, after the level's own shrubs.
			if !warp && !dskip("veg") {
				self.veg.encode(&mut pass, scene);
			}
			crate::viewer::passtime::lap(pass, "veg");
	}
}
