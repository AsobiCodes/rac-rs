//! Dev only (RAC_SKY_DUMP=<dir>): at the first sky capture of each level with the new sky on, the retail sky draw by draw (and all
//! draws together) as lat-long pictures `<dir>/sky_L<level>_d<draw>.png` / `sky_L<level>_all.png` (512 x 256: x = azimuth from +x,
//! y = elevation +90 .. -90 degrees), plus one stderr line per draw. For choosing `params::sky_mask` (which draws paint clouds or
//! the gradient, which hold landmarks). Stalls the frame while it reads back; never on in normal play.

use super::skymodel::Res;
use super::*;

pub(super) fn maybe(gpu: &Gpu, scene: &Scene, u: &Uniforms, r: &Res, draws: usize) {
	let Some(dir) = std::env::var_os("RAC_SKY_DUMP") else { return };
	let sm = &gpu.volclouds.sky;
	if sm.dumped.get() == gpu.level + 1 {
		return;
	}
	sm.dumped.set(gpu.level + 1);
	let _ = std::fs::create_dir_all(&dir);
	if let Some(sky) = scene.sky.as_ref() {
		for (i, d) in sky.draws.iter().enumerate() {
			eprintln!("sky dump: level {} draw {i}: start {} count {} texture {} inst {}", gpu.level, d.start, d.count, d.texture, d.inst);
		}
	}
	eprintln!("sky dump: level {} split {:?} sprites {}", gpu.level, scene.sky_shells.as_ref().map(|s| s.split), scene.sky_sprites.is_some());
	let device = &gpu.device;
	let (w, h) = (512u32, 256u32);
	let target = device.create_texture(&wgpu::TextureDescriptor {
		label: Some("sky dump"),
		size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
		mip_level_count: 1,
		sample_count: 1,
		dimension: wgpu::TextureDimension::D2,
		format: wgpu::TextureFormat::Rgba8Unorm,
		usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
		view_formats: &[],
	});
	let view = target.create_view(&Default::default());
	let buf = device.create_buffer(&wgpu::BufferDescriptor { label: Some("sky dump"), size: (w * h * 4) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
	for which in (0..draws).map(Some).chain([None]) {
		let mut enc = device.create_command_encoder(&Default::default());
		gpu.sky_faces(&mut enc, scene, u, r, which);
		{
			let mut p = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
				label: Some("sky dump"),
				color_attachments: &[Some(wgpu::RenderPassColorAttachment { view: &view, resolve_target: None, depth_slice: None, ops: wgpu::Operations { load: wgpu::LoadOp::Clear(wgpu::Color::BLACK), store: wgpu::StoreOp::Store } })],
				depth_stencil_attachment: None,
				timestamp_writes: crate::viewer::passtime::rw("sky dump"),
				occlusion_query_set: None,
				multiview_mask: None,
			});
			p.set_pipeline(&r.latlong);
			p.set_bind_group(0, &r.rgroup, &[]);
			p.draw(0..3, 0..1);
		}
		enc.copy_texture_to_buffer(
			target.as_image_copy(),
			wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) } },
			wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
		);
		gpu.queue.submit([enc.finish()]);
		buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
		if device.poll(wgpu::PollType::wait_indefinitely()).is_err() {
			return;
		}
		let rgb: Vec<u8> = match buf.slice(..).get_mapped_range() {
			Ok(data) => data.chunks_exact(4).flat_map(|p| [p[0], p[1], p[2]]).collect(),
			Err(_) => return,
		};
		buf.unmap();
		let name = which.map_or("all".to_string(), |i| format!("d{i}"));
		let path = std::path::Path::new(&dir).join(format!("sky_L{}_{name}.png", gpu.level));
		let _ = std::fs::write(path, crate::render::png(&crate::render::Image { w: w as usize, h: h as usize, rgb }));
	}
}
