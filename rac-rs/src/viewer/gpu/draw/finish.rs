//! The end of the frame: movie overlay, screenshot readback, command buffer finish, submit / present.

use super::*;

impl Gpu {
	pub(super) fn frame_finish(&mut self, dt: DrawTarget, sw: f32, sh: f32, capture: bool) -> Option<crate::render::Image> {
		let DrawTarget { frame, view, mut enc, cuts, .. } = dt;
		let frame = frame?;
		self.movie.draw(&self.queue, &mut enc, &view, sw, sh);
		// [PC] Render scale (rscale.rs): the offscreen picture is copied onto the window image; readbacks are the window image.
		self.rscale_blit(&mut enc, &frame.texture.create_view(&Default::default()));
		let (w, h) = self.win_size();
		let row = (w * 4).next_multiple_of(256);
		let readback = capture.then(|| {
			let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("shot"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
			enc.copy_texture_to_buffer(
				frame.texture.as_image_copy(),
				wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
				wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
			);
			buf
		});
		super::profile::lap(super::profile::Phase::Encode);
		super::passtime::frame_end(&mut enc); // [PC] RAC_GPU_PASSES (passtime.rs)
		let _t = super::profile::span("finish");
		let cmd = enc.finish();
		drop(_t);
		// [PC] The present thread collects the finished parts, submits and presents (presenter.rs `present_frame`); a screenshot
		// frame needs its readback at once and submits here.
		let _t = super::profile::span("submit");
		if capture {
			cuts.submit(&self.queue, cmd);
			self.presenter.present(frame);
		} else {
			self.presenter.present_frame(cuts, cmd, frame);
		}
		drop(_t);
		super::profile::lap(super::profile::Phase::Present);
		let buf = readback?;
		buf.slice(..).map_async(wgpu::MapMode::Read, |_| {});
		self.device.poll(wgpu::PollType::wait_indefinitely()).ok()?;
		let data = buf.slice(..).get_mapped_range().ok()?;
		let bgr = matches!(self.config.format, wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb);
		let mut rgb = Vec::with_capacity((w * h * 3) as usize);
		for y in 0..h as usize {
			for px in data[y * row as usize..][..w as usize * 4].chunks_exact(4) {
				rgb.extend(if bgr { [px[2], px[1], px[0]] } else { [px[0], px[1], px[2]] });
			}
		}
		Some(crate::render::Image { w: w as usize, h: h as usize, rgb })
	}
}
