//! [PC] The modern menu's GPU pass (`ui/gpu.rs`), drawn onto the window image after the render-scale copy (with an Aspect ratio box:
//! onto the box picture, before `abox_blit` copies it into the window). With no scene and no `RAC_UI_DEMO` it frees its resources and
//! draws nothing.

use super::*;

impl Gpu {
	pub(super) fn ui_pass(&mut self, enc: &mut wgpu::CommandEncoder, tex: &wgpu::Texture) {
		// The size of the image the pass draws on and copies from, not `win_size()`: the two can differ for a frame while the window is
		// resized, and a copy larger than its source is a wgpu validation error (a panic). Scale, params and the blur chain follow it too.
		let win = (tex.width(), tex.height());
		self.ui_gpu.draw(&self.device, &self.queue, enc, tex, self.config.format, win);
	}
}
