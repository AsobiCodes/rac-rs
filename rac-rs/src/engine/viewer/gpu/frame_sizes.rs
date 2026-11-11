//! [PC] One check of every size a frame's passes pair up, before anything is recorded (player crash 2026-11-03: the HUD pass
//! resolved an 853x480 MSAA colour into the 1280x720 window image after an option change in the pause menu; wgpu panics on that
//! on an enc-finish thread and the game closes). The sizes are planned in `rscale_plan` (render scale, upscaler, Aspect ratio
//! box) and the targets made in `resize`; a frame whose targets do not agree with each other is not drawn: the window image goes
//! back unpresented, the sizes are planned again from the window, and the next frame draws. A wrong rule must never freeze the
//! picture: after `MAX_SKIPS` skips in a row the frame is drawn anyway (the gpu_errors.rs net then keeps the game alive).

use super::*;
use std::sync::atomic::{AtomicU32, Ordering};

/// The sizes one frame's passes pair up (width, height).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Sizes {
	/// The acquired window image.
	pub frame: (u32, u32),
	/// The window size the swapchain was configured at (`real_size`).
	pub real: (u32, u32),
	/// The render size (`config`) and the scene's MSAA colour and depth.
	pub config: (u32, u32),
	pub msaa: (u32, u32),
	pub depth: (u32, u32),
	/// The shown size (`win_size`: the window, or the Aspect ratio box).
	pub win: (u32, u32),
	/// The picture the frame is drawn into and the HUD pass resolves into (render-scale picture, box picture or window image).
	pub view: (u32, u32),
	/// While the upscaler slot runs: its (render, window, HUD MSAA colour) sizes.
	pub slot: Option<((u32, u32), (u32, u32), (u32, u32))>,
}

/// The first rule `s` breaks (None: the frame's targets agree).
pub(super) fn disagree(s: &Sizes) -> Option<&'static str> {
	if s.frame != s.real {
		return Some("window image != swapchain size");
	}
	if s.msaa != s.config || s.depth != s.config {
		return Some("scene MSAA / depth != render size");
	}
	match s.slot {
		Some((render, win, hud)) => {
			if render != s.config || win != s.win || hud != s.win {
				return Some("upscaler targets != render / window size");
			}
			if s.view != s.win {
				return Some("frame picture != window size (upscaler on)");
			}
		}
		None if s.view != s.config => return Some("frame picture != render size (HUD resolve)"),
		None => {}
	}
	None
}

/// Skips in a row before a frame is drawn anyway.
pub(super) const MAX_SKIPS: u32 = 3;
/// Frames skipped in a row by this check (STATE.md).
static SKIPS: AtomicU32 = AtomicU32::new(0);
/// Frames skipped in the whole run (only for the log's rate limit).
static TOTAL: AtomicU32 = AtomicU32::new(0);

/// Whether a frame that broke a rule is skipped (`run` = the skips in a row before it), and the new run length.
pub(super) fn skip_of(run: u32) -> (bool, u32) {
	if run < MAX_SKIPS { (true, run + 1) } else { (false, run) }
}

fn size_of(t: &wgpu::Texture) -> (u32, u32) {
	(t.width(), t.height())
}

impl Gpu {
	/// This frame's sizes (`frame` = the acquired window image, `view` = the picture the frame is drawn into).
	fn frame_sizes(&self, frame: &crate::viewer::fgen::present::FrameImage, view: &wgpu::TextureView) -> Sizes {
		Sizes {
			frame: size_of(frame.texture()),
			real: self.real_size(),
			config: (self.config.width, self.config.height),
			msaa: size_of(self.msaa.texture()),
			depth: size_of(self.depth.texture()),
			win: self.win_size(),
			view: size_of(view.texture()),
			slot: self.upscale.slot_sizes(),
		}
	}

	/// `frame_target`, right after the frame's picture is chosen: the frame and picture back when the sizes agree (or no window
	/// image: the Classic half of a split view), else None: both dropped unpresented and the sizes planned again (the frame is
	/// skipped; at most `MAX_SKIPS` in a row).
	pub(super) fn frame_sizes_check(&mut self, frame: Option<crate::viewer::fgen::present::FrameImage>, view: wgpu::TextureView) -> Option<(Option<crate::viewer::fgen::present::FrameImage>, wgpu::TextureView)> {
		let Some(f) = &frame else { return Some((frame, view)) };
		let s = self.frame_sizes(f, &view);
		let Some(why) = disagree(&s) else {
			SKIPS.store(0, Ordering::Relaxed);
			return Some((frame, view));
		};
		let (skip, run) = skip_of(SKIPS.load(Ordering::Relaxed));
		SKIPS.store(run, Ordering::Relaxed);
		if crate::viewer::gpu_errors::logs(TOTAL.fetch_add(1, Ordering::Relaxed) + 1) {
			eprintln!("gpu: frame sizes disagree ({why}): {s:?}; {}", if skip { "frame skipped, sizes planned again" } else { "drawn anyway" });
		}
		if !skip {
			return Some((frame, view));
		}
		// The surface cannot be configured while its image is held: drop both first.
		drop(view);
		drop(frame);
		let real = self.real_size();
		self.resize(real.0, real.1);
		None
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// A frame whose targets agree: window `win`, render size `render`, no box, the upscaler `slot` or none.
	fn ok(win: (u32, u32), render: (u32, u32), slot: bool) -> Sizes {
		let view = if slot { win } else { render };
		Sizes { frame: win, real: win, config: render, msaa: render, depth: render, win, view, slot: slot.then_some((render, win, win)) }
	}

	#[test]
	fn agreeing_frames_pass() {
		assert_eq!(disagree(&ok((1280, 720), (1280, 720), false)), None, "100 %");
		assert_eq!(disagree(&ok((1280, 720), (640, 360), false)), None, "render scale 50 %");
		assert_eq!(disagree(&ok((1280, 720), (2560, 1440), false)), None, "render scale 200 %");
		assert_eq!(disagree(&ok((1280, 720), (853, 480), true)), None, "FSR Quality");
		// Aspect ratio 4:3 box in a 16:9 window: the frame is drawn at the box, the window image is the window.
		let boxed = Sizes { frame: (1920, 1080), real: (1920, 1080), config: (1440, 1080), msaa: (1440, 1080), depth: (1440, 1080), win: (1440, 1080), view: (1440, 1080), slot: None };
		assert_eq!(disagree(&boxed), None, "4:3 box at 100 %");
		let boxed_fsr = Sizes { config: (960, 720), msaa: (960, 720), depth: (960, 720), slot: Some(((960, 720), (1440, 1080), (1440, 1080))), ..boxed };
		assert_eq!(disagree(&boxed_fsr), None, "4:3 box with FSR");
	}

	/// The player crash (then FSR 1, 2026-11-03): an upscaler -> Off between the plan and the frame (targets at the FSR render size, slot off).
	#[test]
	fn fsr_to_off_with_stale_targets_is_caught() {
		let s = Sizes { slot: None, ..ok((1280, 720), (853, 480), true) };
		assert!(disagree(&s).is_some_and(|w| w.contains("HUD resolve")));
	}

	/// Off -> an upscaler the other way round: the slot runs at the window size, the targets were planned for the plain picture.
	#[test]
	fn off_to_fsr_with_stale_targets_is_caught() {
		let plain = ok((1280, 720), (1280, 720), false);
		assert!(disagree(&Sizes { slot: Some(((853, 480), (1280, 720), (1280, 720))), ..plain }).is_some(), "slot render != config");
		let s = Sizes { config: (853, 480), msaa: (853, 480), depth: (853, 480), view: (853, 480), slot: Some(((853, 480), (1280, 720), (1280, 720))), ..plain };
		assert!(disagree(&s).is_some_and(|w| w.contains("upscaler on")), "picture at the render size");
	}

	#[test]
	fn render_scale_changes_with_stale_targets_are_caught() {
		for (from, to) in [((640, 360), (1280, 720)), ((1280, 720), (2560, 1440)), ((2560, 1440), (640, 360))] {
			let s = Sizes { config: to, view: to, ..ok((1280, 720), from, false) };
			assert!(disagree(&s).is_some_and(|w| w.contains("MSAA")), "{from:?} -> {to:?}");
			let s = Sizes { view: to, ..ok((1280, 720), from, false) };
			assert!(disagree(&s).is_some_and(|w| w.contains("HUD resolve")), "{from:?} picture -> {to:?}");
		}
	}

	#[test]
	fn a_box_change_or_window_resize_with_stale_targets_is_caught() {
		// Display mode Windowed -> Fullscreen: the swapchain at the new size, the targets at the old one.
		let s = Sizes { frame: (1920, 1080), real: (1920, 1080), win: (1920, 1080), view: (1920, 1080), ..ok((1280, 720), (1280, 720), false) };
		assert!(disagree(&s).is_some());
		// A window image of another size than the swapchain was configured at.
		assert!(disagree(&Sizes { frame: (1920, 1080), ..ok((1280, 720), (1280, 720), false) }).is_some_and(|w| w.contains("swapchain")));
		// Aspect ratio Auto -> 4:3: the box picture planned, the targets still at the window size.
		let s = Sizes { frame: (1920, 1080), real: (1920, 1080), config: (1920, 1080), msaa: (1920, 1080), depth: (1920, 1080), win: (1440, 1080), view: (1440, 1080), slot: None };
		assert!(disagree(&s).is_some());
		// 4:3 -> Auto with the FSR slot: slot window size != the shown size.
		let s = Sizes { frame: (1920, 1080), real: (1920, 1080), config: (960, 720), msaa: (960, 720), depth: (960, 720), win: (1920, 1080), view: (1920, 1080), slot: Some(((960, 720), (1440, 1080), (1440, 1080))) };
		assert!(disagree(&s).is_some_and(|w| w.contains("upscaler targets")));
	}

	#[test]
	fn skips_stop_after_max_in_a_row() {
		let mut run = 0;
		for _ in 0..MAX_SKIPS {
			let (skip, r) = skip_of(run);
			assert!(skip);
			run = r;
		}
		assert_eq!(skip_of(run), (false, MAX_SKIPS), "drawn anyway: a wrong rule never freezes the picture");
	}
}
