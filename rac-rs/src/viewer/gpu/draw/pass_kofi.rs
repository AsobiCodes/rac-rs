//! [PC] Ko-fi drawings (kofi.rs): the three child-style sheets taped to Ratchet's garage wall on Veldin during the first visit.
//! Three crooked quads in the level pass (the world's own view-projection, depth-tested without depth write, alpha blended: the
//! transparent margin carries the shadow and the tape overhang), tinted to the dim garage light. The textures are uploaded
//! once, when the "kofi art" thread is done; the resources live in a thread-local (render thread), like `split.rs`.

use super::*;
use std::cell::RefCell;

/// Garage light on the paper: the garage's fill light is dim and warm, so the white sheets are tinted to match
/// (`RAC_TUNE` `kofi_tint` scales all three channels).
const TINT: [f32; 3] = [0.64, 0.6, 0.54];
/// How far the sheet floats off the wall (metres), against depth fighting.
const LIFT: f32 = 0.03;

struct Res {
	binds: Vec<wgpu::BindGroup>,
	vb: wgpu::Buffer,
	ib: wgpu::Buffer,
}

thread_local! {
	static RES: RefCell<Option<Res>> = const { RefCell::new(None) };
}

impl Gpu {
	/// Draws the sheets when this is the first Veldin visit, in the Enhanced half (the Classic half of the split view stays retail).
	pub(super) fn pass_kofi(&self, pass: &mut wgpu::RenderPass<'_>, warp: bool) {
		if warp || self.level != 0 || !crate::retail::splitview::fx(crate::kofi::first_visit()) {
			return;
		}
		let Some(art) = crate::kofi::art() else { return };
		RES.with(|r| {
			let mut r = r.borrow_mut();
			let res = r.get_or_insert_with(|| self.kofi_resources(art));
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

	fn kofi_resources(&self, art: &[crate::kofi::Drawing]) -> Res {
		let k = crate::devtune::f32("kofi_tint", 1.0);
		let tint = [TINT[0] * k, TINT[1] * k, TINT[2] * k, 1.0];
		let places = crate::kofi::placements();
		let (mut verts, mut idx, mut binds) = (Vec::new(), Vec::new(), Vec::new());
		for (i, d) in art.iter().enumerate() {
			let tex = crate::level::Texture { w: d.w, h: d.h, rgba: d.rgba.clone() };
			binds.push(self.upload_texture(Some(&tex)));
			let c = crate::kofi::corners(&places[i], d.w, d.h, LIFT);
			let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
			let base = verts.len() as u32;
			verts.extend((0..4).map(|k| Vertex { pos: c[k], uv: uv[k], normal: [0.0; 3], colour: tint, light: [0.0; 4] }));
			idx.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
		}
		let vb = buffer(&self.device, "kofi vertices", bytemuck::cast_slice(&verts), wgpu::BufferUsages::VERTEX);
		self.queue.write_buffer(&vb, 0, bytemuck::cast_slice(&verts));
		let ib = buffer(&self.device, "kofi indices", bytemuck::cast_slice(&idx), wgpu::BufferUsages::INDEX);
		self.queue.write_buffer(&ib, 0, bytemuck::cast_slice(&idx));
		Res { binds, vb, ib }
	}
}
