//! [S2] FidelityFX frame generation, render thread (RAC_FG=fsr): the frame's generation inputs, recorded at the end of
//! `upscale_encode` (upscale.rs), before the HUD pass.
//!
//! Inputs: the upscaler inputs' motion vectors and depth (render size, upscale.rs `outputs`; with no upscaler they run as
//! Mode::Still, no jitter) and the hudless picture (the scene before the HUD, resolved from the MSAA colour into a window-size
//! texture of the backbuffer's format; only when the scene is already at the window size, else the frame goes without it).
//! Vulkan: the prepare dispatch is recorded here, in a raw command buffer between the frame's passes (`Cuts::raw`). DirectX 12:
//! the dispatch's description and resources go with the frame to the present thread (`DxPrep`, present.rs).
//! The frame's id, its prepared flag and hudless resource wait in `Screen::set_pending` for gpu/draw/finish.rs.

use crate::viewer::Gpu;
#[cfg(windows)]
use std::sync::atomic::{AtomicBool, Ordering};

/// Frame generation is paused (`fg_paused`: a picture box, or lane fglive's follow.rs: Off, the Upscaler, another maker).
#[cfg(windows)]
static PAUSED: AtomicBool = AtomicBool::new(false);
/// A pause ended and no frame has sent its reset yet (`fg_resume_reset`).
#[cfg(windows)]
static RESUMED: AtomicBool = AtomicBool::new(false);
/// The pause's cause last logged (0 runs, 1 picture box, 2.. follow.rs `Why` + 2): one log line per change.
#[cfg(windows)]
static CAUSE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// One frame's pause state (`boxed`): Some(new state) when it changed; leaving a pause arms `resumed`, entering one disarms it.
#[cfg(windows)]
fn pause_step(paused: &AtomicBool, resumed: &AtomicBool, boxed: bool) -> Option<bool> {
	if paused.swap(boxed, Ordering::Relaxed) == boxed {
		return None;
	}
	resumed.store(!boxed, Ordering::Relaxed);
	Some(boxed)
}

impl Gpu {
	/// [S2] The frame generation inputs of this frame (nothing unless our swapchain runs FSR frame generation).
	pub(in crate::viewer) fn fg_encode(&mut self, enc: &mut wgpu::CommandEncoder, cuts: &mut crate::viewer::presenter::Cuts) {
		super::follow::test_tick(); // [Test] RAC_FG_TEST_STEPS (lane fglive)
		#[cfg(windows)]
		self.fg_encode_win(enc, cuts);
		#[cfg(not(windows))]
		let _ = (enc, cuts);
	}

	#[cfg(windows)]
	fn fg_encode_win(&mut self, enc: &mut wgpu::CommandEncoder, cuts: &mut crate::viewer::presenter::Cuts) {
		use super::ffx_fg::{DxPrep, FgFrame, PrepIn};
		use crate::viewer::upscale::{ffx, ffx_abi as abi};
		// [S6] Only the pass that presents (split view's Classic pass draws offscreen): its id is the one `take_pending` takes.
		if crate::splitview::classic() {
			return;
		}
		// [S3] DLSS frame generation (sl_frame.rs): Streamline's tags and constants instead.
		if let Some(sl) = self.screen.sl() {
			self.sl_encode(enc, sl);
			return;
		}
		let Some(fg) = self.screen.fg().filter(|f| f.usable()).cloned() else { return };
		super::trace(format_args!("encode: up {} view {}", self.upscale.outputs().is_some(), self.upscale.fg_view().is_some()));
		let id = fg.peek_id();
		let none = FgFrame { id, prepared: false, hudless: abi::Resource::none(), dx: None };
		if self.fg_paused(super::follow::Maker::Fsr) {
			self.screen.set_pending(none);
			return;
		}
		let (Some((motion, depth, jitter, reset)), Some((eye, fwd, up))) = (self.upscale.outputs(), self.upscale.fg_view()) else {
			self.screen.set_pending(none);
			return;
		};
		let (motion, depth) = (motion.texture().clone(), depth.texture().clone());
		let render = (self.config.width, self.config.height);
		let display = fg.size();
		if render.0 > display.0 || render.1 > display.1 {
			super::trace(format_args!("encode: render {render:?} > display {display:?}"));
			self.screen.set_pending(none);
			return;
		}
		let hudless = self.fg_hudless(enc, display);
		let read = wgpu::TextureUses::RESOURCE;
		let mut list = vec![&motion, &depth];
		list.extend(hudless.iter());
		enc.transition_resources(std::iter::empty(), list.iter().map(|&texture| wgpu::TextureTransition { texture, selector: None, state: read }));
		let (far, fov_y) = self.upscale.fg_cam();
		// The first frame after a pause starts a new history (`fg_resume_reset`).
		let reset = self.fg_resume_reset() || reset;
		// The game's world is Z-up (viewer.rs `forward`); `up` is the camera's up hint (Z unless the camera rolls).
		let p = PrepIn { render, jitter, reset, far, fov_y, eye, fwd, up };
		let state = abi::STATE_PIXEL_COMPUTE_READ;
		let tex = |h: *mut std::ffi::c_void, t: &wgpu::Texture, usage: u32| ffx::texture(h, t.format(), (t.width(), t.height()), usage, state);
		let vk = |t: &wgpu::Texture| unsafe { t.as_hal::<wgpu::hal::api::Vulkan>() }.map(|h| unsafe { std::mem::transmute_copy::<_, u64>(&h.raw_handle()) } as usize as *mut std::ffi::c_void);
		if let Some(m) = vk(&motion) {
			// Vulkan: recorded now, between the frame's passes.
			let (Some(d), h) = (vk(&depth), hudless.as_ref().and_then(vk)) else {
				self.screen.set_pending(none);
				return;
			};
			let hl = match (h, hudless.as_ref()) {
				(Some(h), Some(t)) => tex(h, t, abi::USAGE_RENDERTARGET),
				_ => abi::Resource::none(),
			};
			let mut desc = fg.prepare_desc(id, tex(d, &depth, abi::USAGE_READ_ONLY), tex(m, &motion, abi::USAGE_READ_ONLY), &p);
			let mut raw = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("fgen prepare") });
			super::trace(format_args!("prepare_vk id {id} hudless {}", hudless.is_some()));
			let ok = unsafe { raw.as_hal_mut::<wgpu::hal::api::Vulkan, _, _>(|e| e.is_some_and(|e| fg.prepare_vk(&mut desc, std::mem::transmute_copy::<_, *mut std::ffi::c_void>(&e.raw_handle())))) };
			super::trace(format_args!("prepare_vk done {ok}"));
			cuts.raw(&self.device, enc, raw.finish());
			self.screen.set_pending(FgFrame { id, prepared: ok, hudless: hl, dx: None });
			return;
		}
		// DirectX 12: the description and its resources go to the present thread (present.rs), which records and runs it.
		use windows::core::Interface;
		let res = |t: &wgpu::Texture| unsafe { t.as_hal::<wgpu::hal::api::Dx12>() }.map(|h| unsafe { h.raw_resource() }.clone());
		let (Some(m), Some(d)) = (res(&motion), res(&depth)) else {
			self.screen.set_pending(none);
			return;
		};
		let h = hudless.as_ref().and_then(res);
		let hl = match (h.as_ref(), hudless.as_ref()) {
			(Some(h), Some(t)) => tex(h.as_raw(), t, abi::USAGE_RENDERTARGET),
			_ => abi::Resource::none(),
		};
		let desc = fg.prepare_desc(id, tex(d.as_raw(), &depth, abi::USAGE_READ_ONLY), tex(m.as_raw(), &motion, abi::USAGE_READ_ONLY), &p);
		super::trace(format_args!("prepare dx id {id} hudless {}", hudless.is_some()));
		let keep = [Some(m), Some(d), h].into_iter().flatten().collect();
		self.screen.set_pending(FgFrame { id, prepared: true, hudless: hl, dx: Some(DxPrep { desc, keep }) });
	}

	/// Frame generation of the running maker `m` pauses (no generated frames, no restart: the frames are today's real ones) while
	/// a picture box runs (aspect_pass.rs: an Aspect ratio with black bars or a fullscreen Resolution other than Desktop: the
	/// backbuffer then holds the scaled box, not the window-size picture the inputs describe) and, lane fglive, while the value in
	/// force is Off (follow.rs `pause_why`: chosen Off, Upscaler Off or of the other maker, a value that needs a restart). One log
	/// line per change. The first frame after a pause sends a history reset (`fg_resume_reset`): the generator's history still
	/// holds the boxed or pre-pause picture and camera.
	#[cfg(windows)]
	pub(super) fn fg_paused(&self, m: super::follow::Maker) -> bool {
		let why = if self.abox_tex().is_some() { Some(1u8) } else { super::follow::pause_why(m).map(|w| w as u8 + 2) };
		let paused = why.is_some();
		let changed = pause_step(&PAUSED, &RESUMED, paused).is_some();
		if CAUSE.swap(why.unwrap_or(0), std::sync::atomic::Ordering::Relaxed) != why.unwrap_or(0) || changed {
			let text = match why {
				None => "running".to_string(),
				Some(1) => "paused (picture box on)".to_string(),
				Some(_) => format!("paused ({})", super::follow::pause_why(m).map_or("?", |w| w.text())),
			};
			eprintln!("fgen: frame generation {m:?} {text}");
		}
		paused
	}

	/// Whether this frame is the first one with generation inputs since a pause ended (`fg_paused`): true once, the frame's inputs
	/// then carry a reset. Taken only where the reset is sent, so a resumed frame that goes without inputs passes it on.
	#[cfg(windows)]
	pub(super) fn fg_resume_reset(&self) -> bool {
		RESUMED.swap(false, std::sync::atomic::Ordering::Relaxed)
	}

	/// The hudless picture: the scene's MSAA colour resolved into the window-size texture, when it is at the window size, multi-
	/// sampled, in the backbuffer's format and not boxed (aspect_pass.rs). None: the frame goes without it.
	#[cfg(windows)]
	pub(super) fn fg_hudless(&mut self, enc: &mut wgpu::CommandEncoder, display: (u32, u32)) -> Option<wgpu::Texture> {
		let src = self.upscale.hud_targets().map(|t| t.0.clone()).unwrap_or_else(|| self.msaa.clone());
		let t = src.texture();
		if (t.width(), t.height()) != display || self.win_size() != display || t.format() != self.config.format || self.abox_tex().is_some() {
			return None;
		}
		let dst = self.screen.hudless(&self.device, display, self.config.format);
		let view = dst.create_view(&Default::default());
		let att = crate::viewer::scene_msaa::att(&src, Some(&view), wgpu::Operations { load: wgpu::LoadOp::Load, store: wgpu::StoreOp::Store }); // lane noaa: 1 sample = copy
		drop(enc.begin_render_pass(&wgpu::RenderPassDescriptor { label: Some("fgen hudless"), color_attachments: &[Some(att)], ..Default::default() }));
		crate::viewer::scene_msaa::flush(enc);
		Some(dst)
	}
}

#[cfg(all(test, windows))]
mod tests {
	use super::pause_step;
	use std::sync::atomic::{AtomicBool, Ordering};

	#[test]
	fn a_resume_after_a_box_resets_once() {
		let (p, r) = (AtomicBool::new(false), AtomicBool::new(false));
		assert_eq!(pause_step(&p, &r, false), None, "no box: nothing changes (FG on without a box)");
		assert!(!r.load(Ordering::Relaxed));
		assert_eq!(pause_step(&p, &r, true), Some(true), "paused");
		assert_eq!(pause_step(&p, &r, true), None);
		assert!(!r.load(Ordering::Relaxed), "no reset while boxed");
		assert_eq!(pause_step(&p, &r, false), Some(false), "resumed");
		assert!(r.swap(false, Ordering::Relaxed), "the first resumed frame resets");
		assert_eq!(pause_step(&p, &r, false), None);
		assert!(!r.load(Ordering::Relaxed), "only once");
		pause_step(&p, &r, true);
		pause_step(&p, &r, false);
		pause_step(&p, &r, true);
		assert!(!r.load(Ordering::Relaxed), "a box again before any frame took the reset: the next resume arms it again");
	}
}
