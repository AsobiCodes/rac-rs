//! The end of the frame: movie overlay, screenshot readback, command buffer finish, submit / present.

use super::*;

impl Gpu {
	pub(super) fn frame_finish(&mut self, dt: DrawTarget, sw: f32, sh: f32, capture: bool) -> Option<crate::render::Image> {
		let DrawTarget { frame, view, mut enc, cuts, .. } = dt;
		#[allow(unused_mut)]
		let mut frame = frame?;
		#[cfg(windows)]
		frame.attach_fg(self.screen.take_pending()); // [S2] fgen/frame.rs: this frame's generation inputs go with its present
		#[cfg(windows)]
		frame.attach_sl(self.screen.take_sl()); // [S3] fgen/sl_frame.rs: this frame's Streamline frame goes with its present
		let sp_fin = super::profile::span("e_fin");
		// [PC] Brightness / Contrast (picture_adjust.rs) also on movies; the modern menu below stays ungraded.
		self.movie.set_grade(&self.device, crate::viewer::picture_adjust::frame_k());
		self.movie.draw(&self.queue, &mut enc, &view, sw, sh);
		// [PC] Render scale (rscale.rs): the offscreen picture is copied onto the window image; readbacks are the window image.
		// [PC] Aspect ratio (aspect_pass.rs): with a box, the copy and the modern menu go onto the box picture, which then goes into its
		// rectangle of the window image between black bars; the readback is the whole window image.
		let boxed = self.abox_tex();
		let out = boxed.as_ref().unwrap_or(frame.texture());
		self.rscale_blit(&mut enc, &out.create_view(&Default::default()));
		self.ui_pass(&mut enc, out); // [PC] modern menu (ui/gpu.rs): nothing unless a scene is set
		self.abox_blit(&mut enc, frame.texture());
		let pt = super::passtime::open(&mut enc, "present copy"); // [PC] lane gspeed: RAC_GPU_PASSES row of the copies below
		frame.encode_out(&mut enc); // [PC] fgen/present.rs: our own swapchain: the backbuffer onto the window image (nothing on wgpu's surface)
		let (w, h) = self.real_size();
		let row = (w * 4).next_multiple_of(256);
		let readback = capture.then(|| {
			let buf = self.device.create_buffer(&wgpu::BufferDescriptor { label: Some("shot"), size: (row * h) as u64, usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ, mapped_at_creation: false });
			enc.copy_texture_to_buffer(
				frame.shot().as_image_copy(),
				wgpu::TexelCopyBufferInfo { buffer: &buf, layout: wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(row), rows_per_image: Some(h) } },
				wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
			);
			buf
		});
		drop(sp_fin);
		super::profile::lap(super::profile::Phase::Encode);
		frame.encode_present(&mut enc); // [PC] fgen/present.rs: our own swapchain image to the present state
		super::passtime::close(&mut enc, pt);
		super::passtime::frame_end(&mut enc); // [PC] RAC_GPU_PASSES (passtime.rs)
		// [PC] perf (P9d): on the deferred path the final encoder is finished on an enc-finish thread as well. A screenshot frame
		// takes the old path unless RAC_FINISH_DEFER_SHOT=1 (proof aid: --shot then exercises the deferred path too).
		if self.presenter.defers_finish(&cuts) && (!capture || shot_defer()) {
			let _t = super::profile::span("submit");
			self.presenter.present_frame_enc(cuts, enc, frame);
			drop(_t);
			super::profile::lap(super::profile::Phase::Present);
			if !capture {
				return None;
			}
			eprintln!("finish: screenshot frame took the deferred path (RAC_FINISH_DEFER_SHOT=1)");
			let _gate = self.queue.sync(); // the readback below needs the submit: wait for the present thread's submit
		} else {
			let _t = super::profile::span("finish");
			let cmd = enc.finish();
			drop(_t);
			// [PC] The present thread collects the finished parts, submits and presents (presenter.rs `present_frame`); a screenshot
			// frame needs its readback at once and submits here.
			let _t = super::profile::span("submit");
			if capture {
				cuts.submit_frame(self.queue.sync(), cmd, Some(&frame));
				self.presenter.present(frame);
			} else {
				self.presenter.present_frame(cuts, cmd, frame);
			}
			drop(_t);
			super::profile::lap(super::profile::Phase::Present);
		}
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

/// RAC_FINISH_DEFER_SHOT=1: screenshot frames also finish their last encoder on the enc-finish pool (picture proof of the deferred path).
fn shot_defer() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_FINISH_DEFER_SHOT").is_ok_and(|v| v == "1"))
}
