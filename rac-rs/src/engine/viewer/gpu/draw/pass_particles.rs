//! Particle steps: the pool runs gathered for this frame and their vertex / index buffers (the draws and the cover pass stay
//! with the pass objects in `passes.rs`).

use super::*;

impl Gpu {
	pub(super) fn particle_runs(&self, warp: bool) -> (Vec<Vertex>, Vec<Vertex>, Vec<(bool, bool, u32, u32, u32)>, bool) {
			// One vertex / index buffer for every run (the retail pool order makes many small runs), one draw per run;
			// retail pool lines (`LINE_KEY` runs) come from their own vertex buffer, drawn in walk order.
			let mut all: Vec<Vertex> = Vec::new();
			let mut lall: Vec<Vertex> = Vec::new();
			let mut runs: Vec<(bool, bool, u32, u32, u32)> = Vec::new();
			// [PC] Water Enhanced with Particles Classic: the particles' cover pass ran (partfx.rs `cover`).
			let mut covered = false;
			for ((_, additive, key), verts) in self.part_verts.iter().filter(|_| !warp) {
				if verts.is_empty() {
					continue;
				}
				if *key == LINE_KEY {
					runs.push((true, false, *key, lall.len() as u32, verts.len() as u32));
					lall.extend_from_slice(verts);
					continue;
				}
				if !(self.fx_tex.contains_key(&(*key as usize)) || self.part_tex.is_some()) {
					continue;
				}
				runs.push((false, *additive, *key, all.len() as u32 / 4 * 6, verts.len() as u32 / 4 * 6));
				all.extend_from_slice(verts);
			}
		(all, lall, runs, covered)
	}

	pub(super) fn particle_buffers(&self, all: &Vec<Vertex>, lall: &Vec<Vertex>) -> (Option<(wgpu::Buffer, wgpu::Buffer)>, Option<wgpu::Buffer>, Option<&wgpu::BindGroup>) {
				let tris = (!all.is_empty()).then(|| {
					let idx: Vec<u32> = (0..all.len() as u32 / 4).flat_map(|q| [q * 4, q * 4 + 1, q * 4 + 2, q * 4, q * 4 + 2, q * 4 + 3]).collect();
					let vb = buffer(&self.device, "particle vertices", bytemuck::cast_slice(&all), wgpu::BufferUsages::VERTEX);
					self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&all));
					let ib = buffer(&self.device, "particle indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
					self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
					(vb, ib)
				});
				let lvb = (!lall.is_empty()).then(|| {
					let vb = buffer(&self.device, "pool lines", bytemuck::cast_slice(&lall), wgpu::BufferUsages::VERTEX);
					self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&lall));
					vb
				});
				let any_tex = self.part_tex.as_ref().or_else(|| self.fx_tex.values().next());
		(tris, lvb, any_tex)
	}
}
