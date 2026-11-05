//! The game's 2D space (512 x 448, where retail draws its HUD and menus) <-> window pixels, for the mods' draw list.
//!
//! The renderer shows the GS overlay (the picture holding retail's HUD and menus) as ONE textured quad whose rect (window pixels) and UV
//! rectangle are decided per frame (viewer/handler/frame_render.rs, viewer/front_mirror.rs): in the normal mode the quad fills the window and its UV
//! crops or edge-extends the picture by `k = (w / h) / (4 / 3)` around the centre; in GS-full it is a centred 4:3 (or 16:9) rect with black bars.
//! `Layout` is read from that very quad (`from_hud`), so a mod's bar lands on the same pixel as the same game coordinate of a retail menu in every
//! mode, at every size and aspect, on both backends. Without an overlay quad (no picture yet) the normal mode's formula (`plain`) stands in.

use crate::overlay::Quad;

/// The game's 2D space.
pub const GAME_W: f32 = 512.0;
pub const GAME_H: f32 = 448.0;

/// Where the 512 x 448 picture sits in the window: the quad's rect (x, y, w, h in window pixels) and the UV rectangle (u0, v0, u1, v1) it shows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
	pub rect: [f32; 4],
	pub uv: [f32; 4],
}

impl Layout {
	/// The normal mode: the picture fills the window height, 4:3 in the centre, the UV extends past [0, 1] on a wider window (edge pixels stretch).
	pub fn plain(sw: f32, sh: f32) -> Layout {
		let k = (sw / sh) / (4.0 / 3.0);
		Layout { rect: [0.0, 0.0, sw, sh], uv: [0.5 - 0.5 * k, 0.0, 0.5 + 0.5 * k, 1.0] }
	}

	/// From the first quad of the retail overlay texture `ov_tex` in the frame's HUD list (the centre quad of the RC range's bar layout is the first
	/// too); `plain` when there is none or its UV is degenerate.
	pub fn from_hud(hud: &[Quad], ov_tex: usize, sw: f32, sh: f32) -> Layout {
		match hud.iter().find(|q| q.tex == ov_tex && q.angle == 0.0 && q.corners.is_none()) {
			Some(q) if (q.uv[2] - q.uv[0]).abs() > 1e-4 && (q.uv[3] - q.uv[1]).abs() > 1e-4 && q.rect[2] > 0.0 && q.rect[3] > 0.0 => Layout { rect: q.rect, uv: q.uv },
			_ => Layout::plain(sw, sh),
		}
	}

	/// Window pixels of a game-space point.
	pub fn to_window(&self, gx: f32, gy: f32) -> [f32; 2] {
		let [rx, ry, rw, rh] = self.rect;
		let [u0, v0, u1, v1] = self.uv;
		[rx + (gx / GAME_W - u0) / (u1 - u0) * rw, ry + (gy / GAME_H - v0) / (v1 - v0) * rh]
	}

	/// Game-space point of a window pixel (the inverse of `to_window`).
	pub fn to_game(&self, px: f32, py: f32) -> [f32; 2] {
		let [rx, ry, rw, rh] = self.rect;
		let [u0, v0, u1, v1] = self.uv;
		[((px - rx) / rw * (u1 - u0) + u0) * GAME_W, ((py - ry) / rh * (v1 - v0) + v0) * GAME_H]
	}

	/// Window pixels per game pixel, horizontally and vertically.
	pub fn scale(&self) -> [f32; 2] {
		[self.rect[2] / (self.uv[2] - self.uv[0]) / GAME_W, self.rect[3] / (self.uv[3] - self.uv[1]) / GAME_H]
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	fn near(a: [f32; 2], b: [f32; 2]) {
		assert!((a[0] - b[0]).abs() < 0.01 && (a[1] - b[1]).abs() < 0.01, "{a:?} vs {b:?}");
	}

	#[test]
	fn four_by_three_fills_the_window() {
		let l = Layout::plain(1024.0, 768.0);
		near(l.to_window(0.0, 0.0), [0.0, 0.0]);
		near(l.to_window(512.0, 448.0), [1024.0, 768.0]);
		near(l.to_window(256.0, 224.0), [512.0, 384.0]);
	}

	#[test]
	fn wide_windows_keep_the_four_by_three_centre() {
		// 16:9: the 4:3 picture is the middle 3/4 of the width; game x 0 and 512 land at the picture's edges, not the window's.
		let l = Layout::plain(1920.0, 1080.0);
		near(l.to_window(0.0, 0.0), [240.0, 0.0]);
		near(l.to_window(512.0, 448.0), [1680.0, 1080.0]);
		near(l.to_window(256.0, 224.0), [960.0, 540.0]);
		// 21:9 (2560 x 1080): the 4:3 centre is 1440 wide.
		let w = Layout::plain(2560.0, 1080.0);
		near(w.to_window(0.0, 224.0), [560.0, 540.0]);
		near(w.to_window(512.0, 224.0), [2000.0, 540.0]);
		// A narrow window (3:2 portrait-ish 900 x 900): the picture is cropped at the sides but still centred.
		let s = Layout::plain(900.0, 900.0);
		near(s.to_window(256.0, 224.0), [450.0, 450.0]);
	}

	#[test]
	fn the_inverse_round_trips_at_other_resolutions() {
		for (w, h) in [(640.0, 480.0), (1280.0, 720.0), (1366.0, 768.0), (2560.0, 1440.0), (3440.0, 1440.0), (800.0, 1280.0)] {
			let l = Layout::plain(w, h);
			for (x, y) in [(0.0, 0.0), (100.0, 300.0), (511.0, 447.0), (-40.0, 10.0)] {
				let p = l.to_window(x, y);
				near(l.to_game(p[0], p[1]), [x, y]);
			}
		}
	}

	#[test]
	fn the_overlay_quad_decides_the_layout() {
		let q = |tex, rect: [f32; 4]| Quad { tex, rect, uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None };
		// GS-full: a centred 4:3 rect with bars, the whole picture on it.
		let hud = [q(7, [0.0, 0.0, 1920.0, 1080.0]), q(9, [240.0, 0.0, 1440.0, 1080.0])];
		let l = Layout::from_hud(&hud, 9, 1920.0, 1080.0);
		near(l.to_window(0.0, 0.0), [240.0, 0.0]);
		near(l.to_window(512.0, 448.0), [1680.0, 1080.0]);
		// No overlay quad: the plain formula.
		assert_eq!(Layout::from_hud(&hud, 5, 1920.0, 1080.0), Layout::plain(1920.0, 1080.0));
		assert_eq!(Layout::from_hud(&[], 5, 640.0, 480.0), Layout::plain(640.0, 480.0));
	}

	#[test]
	fn scale_is_window_pixels_per_game_pixel() {
		let l = Layout::plain(1024.0, 768.0);
		let s = l.scale();
		assert!((s[0] - 2.0).abs() < 1e-4 && (s[1] - 768.0 / 448.0).abs() < 1e-4);
	}
}
