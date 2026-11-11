//! [PC] The player's Aspect ratio (GitHub #45; no retail counterpart): the shape of the game picture. Auto (the default) = the
//! picture fills the window, exactly as before. Any other value draws the whole picture (3D world, the retail HUD of the GS capture,
//! modern and classic menus, movies, loading screens, toasts) into the largest centred rectangle of that shape inside the window
//! (`fit`); the rest of the window is black bars (a window wider than the shape = bars left and right, a narrower one = top and
//! bottom). The Gpu draws the frame at the box size and copies it into the window (gpu/aspect_pass.rs: `Gpu::win_size` is the box,
//! the swapchain and the screenshot are the window), so the camera aspect, HUD placement, UI scale, render scale and upscaler all
//! follow the box; the host maps the mouse into the box (`to_box`, `inside`; R&C1 rac1/host/handler/box_mouse.rs).
//! A window within 1 % of the shape counts as that shape (`fit`: no bars, no box pass, ReShade stays on).
//! Kept in the options file only (`aspect=auto|4:3|16:9|16:10|21:9|32:9`): no RAM byte, so the game and its RAM hash never see it.
//! A change applies at the next frame (`Gpu::rscale_sync` plans the sizes again, like a window resize). A game that never calls
//! `init` or the setter keeps Auto (R&C2 draws exactly as before).

use crate::pcopts::PcOptions;
use std::sync::atomic::{AtomicU8, Ordering};

/// Options file key of the row; values `NAMES`.
pub const KEY: &str = "aspect";

/// The picture shapes, in the menu row's order (the row value is the discriminant).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Ratio {
	#[default]
	Auto = 0,
	R4x3 = 1,
	R16x9 = 2,
	R16x10 = 3,
	R21x9 = 4,
	R32x9 = 5,
}

/// Every value in the row's order.
pub const ALL: [Ratio; 6] = [Ratio::Auto, Ratio::R4x3, Ratio::R16x9, Ratio::R16x10, Ratio::R21x9, Ratio::R32x9];
/// The options file values (`ALL` order).
const NAMES: [&str; 6] = ["auto", "4:3", "16:9", "16:10", "21:9", "32:9"];

impl Ratio {
	/// The row value `v` (anything out of range = Auto).
	pub fn from_index(v: i32) -> Ratio {
		usize::try_from(v).ok().and_then(|i| ALL.get(i).copied()).unwrap_or_default()
	}

	/// The options file value.
	pub fn name(self) -> &'static str {
		NAMES[self as usize]
	}

	/// An options file value: `4:3`, `21x9`, ` 16/9 ` (case and spaces ignored); a missing or unknown value is Auto.
	pub fn parse(s: &str) -> Ratio {
		let s = s.trim().to_ascii_lowercase().replace(['x', '/'], ":");
		NAMES.iter().position(|n| *n == s).map_or(Ratio::Auto, |i| ALL[i])
	}

	/// Width : height in whole numbers (None = Auto). 21:9 is the shape of the 2560 x 1080 (and 5120 x 2160) panels sold as 21:9,
	/// 64 : 27, so the box fills such a screen with no bars; 32:9 is 3840 x 1080 / 5120 x 1440.
	pub fn frac(self) -> Option<(u64, u64)> {
		match self {
			Ratio::Auto => None,
			Ratio::R4x3 => Some((4, 3)),
			Ratio::R16x9 => Some((16, 9)),
			Ratio::R16x10 => Some((16, 10)),
			Ratio::R21x9 => Some((64, 27)),
			Ratio::R32x9 => Some((32, 9)),
		}
	}
}

/// The picture's rectangle in window pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rect {
	pub x: u32,
	pub y: u32,
	pub w: u32,
	pub h: u32,
}

impl Rect {
	/// The whole window.
	pub fn full(win: (u32, u32)) -> Rect {
		Rect { x: 0, y: 0, w: win.0, h: win.1 }
	}

	pub fn size(self) -> (u32, u32) {
		(self.w, self.h)
	}

	/// The rectangle is the whole window `win` (no bars: the frame takes the plain path).
	pub fn is_full(self, win: (u32, u32)) -> bool {
		self == Rect::full(win)
	}
}

/// The largest rectangle of shape `r` centred in the window `win` (whole pixels: the long side rounded half up, the offset rounded
/// down, at least 1 x 1); Auto = the whole window. A window within 1 % of the shape (`near`) is the whole window: no bars, no box
/// pass, ReShade stays on (a 3440 x 1440 monitor at 21:9, a 1366 x 768 laptop at 16:9). No stretch: the camera follows the window.
pub fn fit(win: (u32, u32), r: Ratio) -> Rect {
	match r.frac() {
		Some(f) => fit_shape(win, f),
		None => Rect::full(win),
	}
}

/// `fit` for the shape `a` : `b` in whole numbers.
pub fn fit_shape(win: (u32, u32), (a, b): (u64, u64)) -> Rect {
	let (ww, wh) = (win.0.max(1), win.1.max(1));
	let (a, b) = (a.max(1), b.max(1));
	if near((ww, wh), (a, b)) {
		return Rect::full(win);
	}
	if ww as u64 * b > wh as u64 * a {
		// wider than the shape: the full height, bars left and right
		let w = ((wh as u64 * a + b / 2) / b).clamp(1, ww as u64) as u32;
		Rect { x: (ww - w) / 2, y: 0, w, h: wh }
	} else {
		// narrower (or the same shape): the full width, bars top and bottom
		let h = ((ww as u64 * b + a / 2) / a).clamp(1, wh as u64) as u32;
		Rect { x: 0, y: (wh - h) / 2, w: ww, h }
	}
}

/// [PC] Resolution (resolution.rs): the box picture's size and its rectangle in the window `real` for the shape `r` and the frame
/// size `frame` (Some only while fullscreen at a chosen size): without one the picture is the rectangle `fit` gives (exactly as
/// before); with one, the picture is the largest box of the shape inside that size and the rectangle the largest one of the
/// picture's own shape in the window (the copy scales it).
pub fn picture(real: (u32, u32), r: Ratio, frame: Option<(u32, u32)>) -> ((u32, u32), Rect) {
	match frame {
		None => {
			let rect = fit(real, r);
			(rect.size(), rect)
		}
		Some(f) => {
			let pic = fit(f, r).size();
			(pic, fit_shape(real, (pic.0 as u64, pic.1 as u64)))
		}
	}
}

/// The window `ww` x `wh` is within 1 % of the shape `a` : `b` (|ww * b - wh * a| <= ww * b / 100).
fn near((ww, wh): (u32, u32), (a, b): (u64, u64)) -> bool {
	(ww as u64 * b).abs_diff(wh as u64 * a) * 100 <= ww as u64 * b
}

/// A window pixel position as a position in the box (outside 0..w x 0..h on a bar).
pub fn to_box(r: Rect, x: f64, y: f64) -> (f64, f64) {
	(x - r.x as f64, y - r.y as f64)
}

/// A box position (`to_box`) lies on the picture, not on a bar.
pub fn inside(r: Rect, x: f64, y: f64) -> bool {
	x >= 0.0 && y >= 0.0 && x < r.w as f64 && y < r.h as f64
}

static RATIO: AtomicU8 = AtomicU8::new(0);

/// Start-up: the row from the options file.
pub fn init(opts: &PcOptions) {
	let r = Ratio::parse(opts.extra_get(KEY).unwrap_or(""));
	RATIO.store(r as u8, Ordering::Relaxed);
	if r != Ratio::Auto {
		eprintln!("aspect: Aspect ratio {} (options file)", r.name());
	}
}

/// The shape in force.
pub fn current() -> Ratio {
	Ratio::from_index(RATIO.load(Ordering::Relaxed) as i32)
}

/// The menu set the row to `v`: in force from the next frame and remembered; true = the options file changed.
pub fn set_from_ui(opts: &mut PcOptions, v: i32) -> bool {
	let r = Ratio::from_index(v);
	if RATIO.swap(r as u8, Ordering::Relaxed) == r as u8 {
		return false;
	}
	opts.extra_set(KEY, Some(r.name().to_string()));
	eprintln!("aspect: PC options: Aspect ratio {}", r.name());
	true
}

#[cfg(test)]
#[path = "aspect_box_tests.rs"]
mod tests;
