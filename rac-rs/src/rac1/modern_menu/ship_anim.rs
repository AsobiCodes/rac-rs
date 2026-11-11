//! The modern Galactic Map in motion (lane gmap, 2026-11-06; the engine side is `rac_engine` `gpu/menu_art_anim.rs`). This file holds the
//! galaxy painting's facts and the clock: where its disc is (centre, ellipse, tilt: measured on `assets/galaxy/galactic_map.png`), how
//! slowly it turns, how the core breathes, the star map the painting's alpha channel gets (`mark_stars`), and the pulses of the page's
//! markers (`ship.rs` `marks`). Everything runs on DISPLAY time (`clock_s`, wall seconds), so it is smooth at any fps and independent
//! of the game's steps; the markers' places never move (clicking stays easy). `RAC_GMAP_T0` (seconds) shifts the clock for pictures.

use std::sync::atomic::{AtomicU64, Ordering};

/// The painting's disc: centre as a fraction of the picture, semi-axes in painting pixels, tilt of the long axis (rad, y down).
/// MEASURED (script `pp/fit_disc.py` in lane gmap): intensity-weighted second moments of the Gaussian-blurred (12 px) luminance of the
/// painting above thresholds 0.20..0.50 give centre (638..649, 335..337), axis ratio 0.37..0.39, angle -0.15..-0.20 rad; the rest of
/// the disc is the ellipse 560 x 215 (about 2 sigma along the major axis) at (640, 335), tilt -0.19. Checked offline with the shader's
/// maths: the blurred silhouette of the painting turned by 90 / 180 / 270 deg overlaps the original with IoU 0.98 (threshold 0.25) and
/// 0.91 (threshold 0.35).
pub const CENTRE: [f32; 2] = [0.5, 335.0 / 720.0];
pub const AXES: [f32; 2] = [560.0, 215.0];
pub const TILT: f32 = -0.19;
/// One full turn of the disc, seconds (slow: the arms drift when you look, no spin).
pub const TURN_PERIOD_S: f64 = 480.0;
/// Width of the feather ring (fraction of the disc radius).
pub const FEATHER: f32 = 0.45;
/// The core's glow radius (disc units), its breathing amplitude and period (s).
pub const CORE_RADIUS: f32 = 0.30;
pub const CORE_AMP: f32 = 0.065;
pub const CORE_PERIOD_S: f64 = 7.0;
/// Star twinkle strength.
pub const TWINKLE: f32 = 0.35;

static CLOCK_TEST: AtomicU64 = AtomicU64::new(u64::MAX);

static FRAMES: AtomicU64 = AtomicU64::new(0);

/// One display frame of the painting went by (`ship_host::place_art`): the clock of a deterministic run counts these.
pub fn tick() {
	FRAMES.fetch_add(1, Ordering::Relaxed);
}

/// Display time in seconds, plus `RAC_GMAP_T0`: wall clock since the host's first frame; in a deterministic run (scripted, replayed or
/// shot: `console_clock::deterministic`) the painting's frame count / 60, so a picture does not depend on the wall time; with
/// `RAC_GMAP_ANIM=0` it stands still at `RAC_GMAP_T0` (markers too).
pub fn clock_s() -> f64 {
	let t = CLOCK_TEST.load(Ordering::Relaxed);
	if t != u64::MAX {
		return f64::from_bits(t);
	}
	static T0: std::sync::OnceLock<f64> = std::sync::OnceLock::new();
	let t0 = *T0.get_or_init(|| std::env::var("RAC_GMAP_T0").ok().and_then(|s| s.trim().parse().ok()).unwrap_or(0.0));
	t0 + if !enabled() {
		0.0
	} else if crate::retail::console_clock::deterministic() {
		FRAMES.load(Ordering::Relaxed) as f64 / 60.0
	} else {
		super::ship_host::now() as f64 / 1000.0
	}
}

/// Whether the painting moves (`RAC_GMAP_ANIM=0` = the still painting, for A/B pictures).
pub fn enabled() -> bool {
	static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*ON.get_or_init(|| std::env::var("RAC_GMAP_ANIM").map_or(true, |v| v.trim() != "0"))
}

/// Tests: pin the clock (None = the real one).
#[cfg(test)]
pub fn set_clock_for_test(t: Option<f64>) {
	CLOCK_TEST.store(t.map_or(u64::MAX, f64::to_bits), Ordering::Relaxed);
}

/// The engine's animation of the painting at time `t` seconds.
pub fn anim_at(t: f64) -> rac_engine::viewer::ArtAnim {
	let breathe = (t / CORE_PERIOD_S * std::f64::consts::TAU).sin() as f32;
	rac_engine::viewer::ArtAnim {
		centre: CENTRE,
		axes: AXES,
		tilt: TILT,
		angle: rac_engine::viewer::turn_angle(t, TURN_PERIOD_S),
		feather: FEATHER,
		core_gain: 1.0 + CORE_AMP * breathe,
		core_radius: CORE_RADIUS,
		time: (t % rac_engine::viewer::TWINKLE_WRAP_S) as f32, // wrapped; the shader's twinkle rates are whole cycles per this period
		twinkle: TWINKLE,
	}
}

/// A slow smooth pulse 0..=1 with period `period` s, shifted by `phase` (0..1 of a period).
pub fn pulse(t: f64, period: f64, phase: f64) -> f32 {
	(0.5 - 0.5 * ((t / period + phase) * std::f64::consts::TAU).cos()) as f32
}

/// 0..1 position of a light travelling along a line, once per `period` s.
pub fn travel(t: f64, period: f64) -> f32 {
	(t / period).fract() as f32
}

fn lum(px: &[u8], w: usize, x: usize, y: usize) -> f32 {
	let i = (y * w + x) * 4;
	px[i].max(px[i + 1]).max(px[i + 2]) as f32 / 255.0
}

/// Writes the star map into the painting's alpha: 255 = not a star, 1..=253 = a star's code (its pixels: those within 4 px of a
/// small bright peak that are at least 35 % bright). A peak is a 5 x 5 local maximum of at least 62 % brightness that is "small"
/// (the ring 6 px round it is under 60 % of it), outside the glowing core (disc units 0.25).
pub fn mark_stars(px: &mut [u8], w: u32, h: u32) {
	let (w, h) = (w as usize, h as usize);
	if px.len() != w * h * 4 {
		return;
	}
	for p in px.chunks_exact_mut(4) {
		p[3] = 255;
	}
	let mut peaks = Vec::new();
	for y in 7..h.saturating_sub(7) {
		for x in 7..w.saturating_sub(7) {
			let l = lum(px, w, x, y);
			if l < 0.62 || in_core(x as f32, y as f32, w, h) {
				continue;
			}
			let mut max = true;
			'n: for dy in -2i32..=2 {
				for dx in -2i32..=2 {
					let (nx, ny) = ((x as i32 + dx) as usize, (y as i32 + dy) as usize);
					let n = lum(px, w, nx, ny);
					if n > l || (n == l && (dy < 0 || (dy == 0 && dx < 0))) {
						max = false;
						break 'n;
					}
				}
			}
			if !max {
				continue;
			}
			let ring: f32 = (0..8).map(|k| {
				let a = k as f32 * std::f32::consts::FRAC_PI_4;
				lum(px, w, (x as f32 + 6.0 * a.cos()).round() as usize, (y as f32 + 6.0 * a.sin()).round() as usize)
			}).sum::<f32>() / 8.0;
			if ring < 0.6 * l {
				peaks.push((x, y));
			}
		}
	}
	for (x, y) in peaks {
		let code = (1 + (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)) % 253) as u8;
		for dy in -4i32..=4 {
			for dx in -4i32..=4 {
				if dx * dx + dy * dy > 16 {
					continue;
				}
				let (nx, ny) = ((x as i32 + dx) as usize, (y as i32 + dy) as usize);
				if nx < w && ny < h && px[(ny * w + nx) * 4 + 3] == 255 && lum(px, w, nx, ny) >= 0.35 {
					px[(ny * w + nx) * 4 + 3] = code;
				}
			}
		}
	}
}

/// Whether the painting pixel is inside the glowing core.
fn in_core(x: f32, y: f32, w: usize, h: usize) -> bool {
	let (q0, q1) = ((x - CENTRE[0] * w as f32), (y - CENTRE[1] * h as f32));
	let (c, s) = (TILT.cos(), TILT.sin());
	let (e0, e1) = (c * q0 + s * q1, -s * q0 + c * q1);
	((e0 / AXES[0]).powi(2) + (e1 / AXES[1]).powi(2)).sqrt() < 0.25
}

#[cfg(test)]
mod tests;
