//! Level-pass steps for liquids and effect quads: the oceans (also recorded for the water pass), the textured world quads
//! (sea, pond, river, fogged water) and the FIX-blend strips, the props metal faces and the queue-C water pools.

use super::*;

impl Gpu {
	pub(super) fn pass_oceans(&self, pass: &mut wgpu::RenderPass<'_>, scene: &Scene, warp: bool, water_on: bool, wrec: &mut Vec<super::waterfx::WaterDraw>) {
		crate::viewer::passtime::lap(pass, "world rest"); // [probe] RAC_GPU_PASSES lap (passtime.rs)
			pass.set_bind_group(0, &self.ubind, &[]);
			// ARI-49b (BRIDGE-63): the Aridia ocean, opaque and Z-writing after the world and mobies, before the
			// translucent passes; every patch strip as triangles (i, i+1, i+2), one draw.
			if let (false, Some((_, bind))) = (warp || self.ocean_draws.is_empty() || dskip("ocean"), &self.ocean_tex) {
				let mut verts = Vec::new();
				let mut idx: Vec<u32> = Vec::new();
				let ocean_light = self.ocean_fog.map_or([0.0; 4], |f| [f[0] as f32 / 255.0, f[1] as f32 / 255.0, f[2] as f32 / 255.0, 0.0]);
				for s in self.ocean_draws.iter().filter(|s| s.pos.len() >= 3) {
					let base = verts.len() as u32;
					verts.extend((0..s.pos.len()).map(|j| Vertex { pos: s.pos[j], uv: s.uv[j], normal: [0.0; 3], colour: s.colour[j], light: ocean_light }));
					idx.extend((0..s.pos.len() as u32 - 2).flat_map(|t| [base + t, base + t + 1, base + t + 2]));
				}
				if !idx.is_empty() {
					let vb = buffer(&self.device, "ocean vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
					let ib = buffer(&self.device, "ocean indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
					self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
					self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
					// KER-30: every strip carries the ocean's FIX; below 0x80 the blended pair with constant FIX / 128.
					let fix = self.ocean_draws[0].fix;
					let blended = fix < 0x80;
					if blended {
						let k = crate::ocean::blend_constant(fix);
						pass.set_blend_constant(wgpu::Color { r: k, g: k, b: k, a: 1.0 });
					}
					pass.set_pipeline(&self.ocean_pipeline[if self.ocean_fog.is_some() && !self.ocean_level_fog { 4 + blended as usize } else { blended as usize * 2 + self.ocean_level_fog as usize }]);
					pass.set_vertex_buffer(0, vb.slice(..));
					pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
					pass.set_bind_group(1, bind, &[]);
					pass.draw_indexed(0..idx.len() as u32, 0, 0..1);
					// [PC] Kerwan's "ocean" is the chasm FOG sheet (`fog_sheet_ocean`), never water.
					if water_on && !super::waterfx::fog_sheet_ocean(self.level) {
						wrec.push(super::waterfx::WaterDraw::new(vb, ib, idx.len() as u32, verts.iter().map(|v| v.pos)).kind("level ocean").liquid(self.level));
					}
				}
			}
			// [PC] Water Enhanced: a liquid drawn as level terrain (`waterfx::terrain_liquid`: Orxon's sludge) goes to the water pass
			// too: this frame's drawn terrain faces of its texture.
			if water_on && !warp {
				if let Some((tex, slot)) = super::waterfx::terrain_liquid(self.level) {
					let wrap = 3 << crate::shrubs::WRAP_SHIFT;
					let slots: Vec<usize> = scene.tex_slots.iter().filter(|(k, _)| **k & !wrap == tex).map(|(_, &s)| s).collect();
					for d in scene.tfrags.draws.iter().filter(|d| d.count > 0 && slots.contains(&d.texture)) {
						wrec.push(super::waterfx::WaterDraw::range(scene.tfrags.vbuf.clone(), scene.tfrags.ibuf.clone(), d.start, d.count, slot).kind("terrain liquid"));
					}
				}
			}
			// Retail mode: the Rilgar, Umbris and Gaspar oceans, one draw per batch (own texture, FIX blend without Z when FIX < 0x80).
			if !warp && !dskip("oceang") {
				for b in self.ocean_ram.iter().filter(|b| !b.idx.is_empty() || !b.pidx.is_empty()) {
					let blended = b.fix < 0x80;
					if blended {
						let k = crate::ocean::blend_constant(b.fix);
						pass.set_blend_constant(wgpu::Color { r: k, g: k, b: k, a: 1.0 });
					}
					pass.set_bind_group(1, &b.bind, &[]);
					if !b.idx.is_empty() {
						let vb = buffer(&self.device, "ocean ram vertices", bytemuck::cast_slice(&b.verts), wgpu::BufferUsages::VERTEX);
						let ib = buffer(&self.device, "ocean ram indices", bytemuck::cast_slice(&b.idx), wgpu::BufferUsages::INDEX);
						self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&b.verts));
						self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&b.idx));
						pass.set_pipeline(&self.ocean_g_pipeline[blended as usize]);
						pass.set_vertex_buffer(0, vb.slice(..));
						pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
						pass.draw_indexed(0..b.idx.len() as u32, 0, 0..1);
						if water_on {
							wrec.push(super::waterfx::WaterDraw::new(vb, ib, b.idx.len() as u32, b.verts.iter().map(|v| v.pos)).kind("RAM ocean").liquid(self.level));
						}
					}
					if !b.pidx.is_empty() {
						let pvb = buffer(&self.device, "ocean ram patch vertices", bytemuck::cast_slice(&b.pverts), wgpu::BufferUsages::VERTEX);
						let pib = buffer(&self.device, "ocean ram patch indices", bytemuck::cast_slice(&b.pidx), wgpu::BufferUsages::INDEX);
						self.queue.write_buffer(&pvb, 0, bytemuck::cast_slice(&b.pverts));
						self.queue.write_buffer(&pib, 0, bytemuck::cast_slice(&b.pidx));
						pass.set_pipeline(&self.ocean_g_pipeline[2 + blended as usize]);
						pass.set_vertex_buffer(0, pvb.slice(..));
						pass.set_index_buffer(pib.slice(..), wgpu::IndexFormat::Uint32);
						pass.draw_indexed(0..b.pidx.len() as u32, 0, 0..1);
						if water_on {
							wrec.push(super::waterfx::WaterDraw::new(pvb, pib, b.pidx.len() as u32, b.pverts.iter().map(|v| v.pos)).kind("RAM ocean").liquid(self.level));
						}
					}
				}
			}
	}

	pub(super) fn pass_world_quads(&self, mut pass: &mut wgpu::RenderPass<'_>, cam: &Camera, warp: bool, water_on: bool, mut wrec: &mut Vec<super::waterfx::WaterDraw>, river: &Vec<usize>, river_groups: &Vec<(u32, Vec<usize>)>, strips_water: bool, strip_flows: &Vec<u32>) {
		crate::viewer::passtime::lap(pass, "oceans"); // [probe] RAC_GPU_PASSES lap (passtime.rs)
			// Textured world quads, one draw per run of the same texture and blend.
			for fogged in [false, true].into_iter().filter(|_| !dskip("quads")) {
				let quads = if fogged { &self.water_quads } else { &self.fx_quads };
				// SCN-4.2: while the warp plays its own quads (big quad, ribbons, gloss) use the warp's texture table.
				let warp_fx = self.warp_fx.as_ref().filter(|_| warp);
				let table = warp_fx.map_or(&self.fx_tex, |w| &w.quad_tex);
				let quads: &[_] = if warp { warp_fx.filter(|_| !fogged).map_or(&[], |w| &w.quads) } else { quads };
				let mut verts = Vec::with_capacity(quads.len() * 4);
				let mut runs: Vec<(usize, bool, u32, u32)> = Vec::new();
				// [PC] Water Enhanced: a sea drawn as effect quads (Pokitaru) goes to the water pass too.
				let sea_tex = if water_on && !fogged && !warp { super::waterfx::sea_fx_tex(self.level) } else { &[] };
				let mut sea: Vec<Vertex> = Vec::new();
				let mut sea_t: Vec<usize> = Vec::new();
				// [PC] Water Enhanced: a pond drawn as effect quads (Pokitaru's waterfall ponds), its own draw (its own plane).
				let pond_tex = if water_on && !fogged && !warp { super::waterfx::pond_fx_tex(self.level) } else { &[] };
				let mut pond: Vec<Vertex> = Vec::new();
				// [PC] Water Enhanced: waterfall sheets drawn as effect quads (Pokitaru) are drawn by the water pass instead
				// (waterfx.rs `fs_fall`, fed by `sheet_falls` above).
				let fall_tex = if water_on && !fogged && !warp { super::waterfx::fall_fx_tex(self.level) } else { &[] };
				for q in quads {
					// The effects layer's GS fog rides in the high bits of `tex` (retail/efx_draw.rs `fog_tex`).
					let tex = crate::retail::efx_draw::plain_tex(q.tex);
					if !table.contains_key(&tex) || fall_tex.contains(&tex) {
						continue;
					}
					if sea_tex.contains(&tex) {
						sea_t.push(tex);
						sea.extend((0..4).map(|k| Vertex { pos: q.pos[k], uv: q.uv[k], normal: [0.0; 3], colour: q.colour[k], light: [0.0; 4] }));
					}
					if pond_tex.contains(&tex) {
						pond.extend((0..4).map(|k| Vertex { pos: q.pos[k], uv: q.uv[k], normal: [0.0; 3], colour: q.colour[k], light: [0.0; 4] }));
					}
					let base = verts.len() as u32 / 4 * 6;
					for k in 0..4 {
						verts.push(Vertex { pos: q.pos[k], uv: q.uv[k], normal: [0.0; 3], colour: q.colour[k], light: crate::retail::efx_draw::fog_light(q.tex, k, -1.0) });
					}
					match runs.last_mut() {
						Some(r) if r.0 == tex && r.1 == q.additive => r.3 += 6,
						_ => runs.push((tex, q.additive, base, 6)),
					}
				}
				if !verts.is_empty() {
					let idx: Vec<u32> = (0..verts.len() as u32 / 4).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
					let vb = buffer(&self.device, "fx quad vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
					self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
					let ib = buffer(&self.device, "fx quad indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
					self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
					pass.set_vertex_buffer(0, vb.slice(..));
					pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint32);
					for (tex, additive, start, count) in runs {
						pass.set_pipeline(&self.part_pipelines[if fogged { 2 } else { additive as usize }]);
						pass.set_bind_group(1, &table[&tex], &[]);
						pass.draw_indexed(start..start + count, 0, 0..1);
					}
					// The fogged run is the water quads (Novalis class 751, Drek fleet water).
					if fogged && water_on {
						wrec.push(super::waterfx::WaterDraw::new(vb, ib, idx.len() as u32, verts.iter().map(|v| v.pos)).kind("fogged quads"));
					}
				}
				if !sea.is_empty() {
					sea_shading_mesh(&mut sea, &sea_t, sea_tex[0], cam.pos);
					let idx: Vec<u32> = (0..sea.len() as u32 / 4).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
					let vb = buffer(&self.device, "sea quad vertices", bytemuck::cast_slice(&sea), wgpu::BufferUsages::VERTEX);
					self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&sea));
					let ib = buffer(&self.device, "sea quad indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
					self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
					wrec.push(super::waterfx::WaterDraw::new(vb, ib, idx.len() as u32, sea.iter().map(|v| v.pos)).kind("sea fx quads"));
				}
				if !pond.is_empty() {
					let idx: Vec<u32> = (0..pond.len() as u32 / 4).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
					let vb = buffer(&self.device, "pond quad vertices", bytemuck::cast_slice(&pond), wgpu::BufferUsages::VERTEX);
					self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&pond));
					let ib = buffer(&self.device, "pond quad indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
					self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
					let d = super::waterfx::WaterDraw::new(vb, ib, idx.len() as u32, pond.iter().map(|v| v.pos)).kind("pond fx quads");
					wrec.push(d.opaque());
				}
				for (slot, group) in river_groups.iter().filter(|_| !fogged && !warp) {
					{
						let q = |i: usize| &self.fx_quads[river[i]];
						let v: Vec<Vertex> = group.iter().flat_map(|&i| (0..4).map(move |k| (i, k))).map(|(i, k)| Vertex { pos: q(i).pos[k], uv: q(i).uv[k], normal: [0.0; 3], colour: q(i).colour[k], light: [0.0; 4] }).collect();
						let idx: Vec<u32> = (0..v.len() as u32 / 4).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
						let vb = buffer(&self.device, "river quad vertices", bytemuck::cast_slice(&v), wgpu::BufferUsages::VERTEX);
						self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&v));
						let ib = buffer(&self.device, "river quad indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
						self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
						wrec.push(super::waterfx::WaterDraw::new(vb, ib, idx.len() as u32, v.iter().map(|v| v.pos)).kind("river fx quads").flowing(*slot));
					}
				}
			}
			// BRIDGE-23: FIX-blend world strips (NOV-15.1b), triangles (i, i+1, i+2), one draw per strip.
			if !warp && !dskip("strips") {
				// [PC] Water Enhanced: on Novalis these strips are the river and its pools (waterfx.rs `strips_are_water`).
				draw_strips(&self.device, &self.queue, &mut pass, &self.fx_tex, &self.strip_draws, &self.strip_pipeline, &self.strip_pipeline, strips_water.then_some((&mut wrec, &strip_flows[..], &[][..])));
			}
	}

	pub(super) fn pass_pools(&self, mut pass: &mut wgpu::RenderPass<'_>, warp: bool, metal: Option<(&Batch, &[wgpu::BindGroup; 2])>, water_on: bool, mut wrec: &mut Vec<super::waterfx::WaterDraw>) {
		crate::viewer::passtime::lap(pass, "ties lt"); // [probe] RAC_GPU_PASSES lap (passtime.rs)
			if let Some((batch, tex)) = metal.filter(|_| !dskip("metal")) {
				pass.set_vertex_buffer(0, batch.vbuf.slice(..));
				pass.set_index_buffer(batch.ibuf.slice(..), wgpu::IndexFormat::Uint32);
				for d in &batch.draws {
					pass.set_pipeline(if d.cull != 0 { &self.pipeline_cull[d.cull as usize - 1] } else { &self.pipeline });
					pass.set_bind_group(1, &tex[d.texture], &[]);
					pass.draw_indexed(d.start..d.start + d.count, 0, 0..1);
				}
			}
			// Retail mode, Hoven: the queue-C water pools. The frame pass FUN_0021af28 (overlay.elf.c 10606-10700) flushes queue C
			// (FUN_0021be68) after the tfrag and tie passes and before the shrub pass FUN_0029f438, the moby pass and queue A; the shrubs
			// (ice floes) then blend over the pool (GSPIX at 12_0582, decomp/notes/fx_census_late.md).
			if !warp && !dskip("pools") {
				let pool_tex = super::waterfx::pool_water_tex(self.level);
				// Pools retail draws opaque (FIX 128 lerp) keep their look: no clear shallows. The abyss water gets the cold look, the ice
				// floor (door puddles and the sheet around them) is held still.
				let pool_slots: Vec<u32> = self.pool_draws.iter().map(|s| if super::waterfx::cold_pool(self.level, crate::retail::efx_draw::plain_tex(s.tex), s.add, s.fix) { super::waterfx::COLD_SLOT } else if let Some(k) = super::waterfx::frozen_pool(self.level, crate::retail::efx_draw::plain_tex(s.tex), s.add, s.fix) { k } else if !s.add && s.fix >= 120 { super::waterfx::OPAQUE_SLOT } else { 0 }).collect();
				draw_strips(&self.device, &self.queue, &mut pass, &self.fx_tex, &self.pool_draws, &self.pool_pipeline, &self.pool_add_pipeline, (water_on && !pool_tex.is_empty()).then_some((&mut wrec, &pool_slots[..], pool_tex)));
			}
	}
}
