//! [PC] World art (a game pack's `Pack::world_art`; R&C1: the three Ko-fi drawings taped to Ratchet's garage wall on Veldin
//! during the first visit, rac1/kofi.rs): crooked quads in the level pass (the world's own view-projection, depth-tested without
//! depth write, alpha blended: the transparent margin carries the shadow and the tape overhang), tinted by the pack. The textures
//! are uploaded once, when the pack first hands the art over; the resources live in a thread-local (render thread), like `split.rs`.

use super::*;
use std::cell::RefCell;

struct Res {
	binds: Vec<wgpu::BindGroup>,
	vb: wgpu::Buffer,
	ib: wgpu::Buffer,
}

thread_local! {
	static RES: RefCell<Option<Res>> = const { RefCell::new(None) };
}

impl Gpu {
	/// Draws the pack's world art of this level, in the Enhanced half (the Classic half of the split view stays retail).
	pub(super) fn pass_art(&self, pass: &mut wgpu::RenderPass<'_>, warp: bool) {
		let art = if warp { None } else { (crate::pack::get().world_art)(self.level) };
		if !crate::splitview::fx(art.is_some()) {
			return;
		}
		let Some(art) = art else { return };
		RES.with(|r| {
			let mut r = r.borrow_mut();
			let res = r.get_or_insert_with(|| self.art_resources(art));
			pass.set_pipeline(&self.part_pipelines[0]);
			pass.set_bind_group(0, &self.ubind, &[]);
			pass.set_vertex_buffer(0, res.vb.slice(..));
			pass.set_index_buffer(res.ib.slice(..), wgpu::IndexFormat::Uint32);
			for (i, bind) in res.binds.iter().enumerate() {
				pass.set_bind_group(1, bind, &[]);
				pass.draw_indexed(i as u32 * 6..i as u32 * 6 + 6, 0, 0..1);
			}
		});
	}

	fn art_resources(&self, art: &crate::pack::WorldArt) -> Res {
		let (mut verts, mut idx, mut binds) = (Vec::new(), Vec::new(), Vec::new());
		for (tex, c) in &art.sheets {
			binds.push(self.upload_texture(Some(tex)));
			let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
			let base = verts.len() as u32;
			verts.extend((0..4).map(|k| Vertex { pos: c[k], uv: uv[k], normal: [0.0; 3], colour: art.tint, light: [0.0; 4] }));
			idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
		}
		let vb = buffer(&self.device, "world art vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
		self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
		let ib = buffer(&self.device, "world art indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
		self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
		Res { binds, vb, ib }
	}
}
