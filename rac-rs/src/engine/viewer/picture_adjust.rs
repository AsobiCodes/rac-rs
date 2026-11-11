//! [PC] Brightness and Contrast (v0.5.1, user 2026-11-07; no retail counterpart): two sliders 0..100 in steps of 5 that change the
//! GAME picture (the 3D world with every effect of either Lighting, after the upscaler, and the movies) before the HUD and the
//! menus are drawn over it, so the HUD and the menus keep their colours. Default 50 / 50 = today's picture bit for bit: `active` is
//! false there and every frame takes exactly the path it took before (the same pipelines, no extra pass, no extra target).
//!
//! Where the curve runs (`frame_k` latched once per frame, gpu/grade_pass.rs), always before ReShade and the frame generation
//! inputs: the upscaler slot's copy into the HUD target (upscale/slot_gpu.rs `fs_copy_grade`) when the slot runs; else Lighting
//! Enhanced's composite (post.rs `fs_comp_grade`); else (Classic) one pass of its own (gpu/grade_pass.rs). Movies: fmv/present.rs.
//!
//! The curve (`CURVE_WGSL`, `curve`), per colour channel on the display-encoded value x in 0..1 (the picture as the window shows it):
//! - Brightness b: a gamma lift `x' = x ^ g` with `g = 2 ^ ((50 - b) / 62.5)` (b 100: g = 0.574, b 0: g = 1.74). Black stays black,
//!   white stays white, the darks and mids rise (b > 50) or sink (b < 50); no channel clips.
//! - Contrast c: an S-curve about mid-grey, `y = x' < 0.5 ? 0.5 (2x')^k : 1 - 0.5 (2 - 2x')^k` with `k = 2 ^ ((c - 50) / 75)`
//!   (c 100: k = 1.59, c 0: k = 0.63). Black, mid-grey and white stay where they are; nothing clips.
//! At 50 both exponents are exactly 1 (`curve` is the identity; unit test), but no frame runs the curve then anyway.
//! Kept in the options file only (`brightness=N`, `contrast=N`, written only when not 50): no RAM byte, the game and its RAM hash
//! never see them. A change applies at the next frame (the slider is live). A game that never calls `init` or the setters keeps
//! 50 / 50 (its frames draw exactly as before).
//! Split view: no `splitview::fx` gate on purpose: this is the player's picture setting for either Lighting, not an Enhanced effect.

use crate::pcopts::PcOptions;
use std::sync::atomic::{AtomicU8, Ordering};

/// Options file keys.
pub const KEY_BRIGHT: &str = "brightness";
pub const KEY_CONTRAST: &str = "contrast";
/// The neutral value of both sliders (today's picture).
pub const NEUTRAL: u8 = 50;
pub const MAX: u8 = 100;
pub const STEP: u8 = 5;

static BRIGHT: AtomicU8 = AtomicU8::new(NEUTRAL);
static CONTRAST: AtomicU8 = AtomicU8::new(NEUTRAL);

/// A slider value from the file or the menu: the nearest step of 5 inside 0..=100.
pub fn snap(v: i64) -> u8 {
	let v = v.clamp(0, MAX as i64) as u32;
	(((v + STEP as u32 / 2) / STEP as u32) * STEP as u32).min(MAX as u32) as u8
}

/// A file value (`"70"`, `" 70 "`, `"70%"`); None when it is not a number (the key then reads as 50).
fn parse(s: &str) -> Option<u8> {
	s.trim().trim_end_matches('%').trim().parse::<i64>().ok().map(snap)
}

/// Start-up: the sliders from the options file.
pub fn init(opts: &PcOptions) {
	let b = opts.extra_get(KEY_BRIGHT).and_then(parse).unwrap_or(NEUTRAL);
	let c = opts.extra_get(KEY_CONTRAST).and_then(parse).unwrap_or(NEUTRAL);
	BRIGHT.store(b, Ordering::Relaxed);
	CONTRAST.store(c, Ordering::Relaxed);
	if b != NEUTRAL || c != NEUTRAL {
		eprintln!("picture: Brightness {b} Contrast {c} (options file)");
	}
}

/// Brightness in force (0..=100, 50 = today).
pub fn brightness() -> u8 {
	BRIGHT.load(Ordering::Relaxed)
}

/// Contrast in force (0..=100, 50 = today).
pub fn contrast() -> u8 {
	CONTRAST.load(Ordering::Relaxed)
}

/// The frame changes the picture (either slider off 50).
pub fn active() -> bool {
	brightness() != NEUTRAL || contrast() != NEUTRAL
}

/// The menu set Brightness (`contrast` false) or Contrast to `v`: in force from the next frame and remembered; true = the options
/// file changed.
pub fn set_from_ui(opts: &mut PcOptions, contrast: bool, v: i32) -> bool {
	let v = snap(v as i64);
	let (cell, key, name) = if contrast { (&CONTRAST, KEY_CONTRAST, "Contrast") } else { (&BRIGHT, KEY_BRIGHT, "Brightness") };
	if cell.swap(v, Ordering::Relaxed) == v {
		return false;
	}
	opts.extra_set(key, (v != NEUTRAL).then(|| v.to_string()));
	eprintln!("picture: PC options: {name} {v}");
	true
}

/// The shared WGSL curve `grade_curve(c, k)` (k.x = gamma g, k.y = contrast exponent): appended to every shader that grades.
pub const CURVE_WGSL: &str = include_str!("gpu/grade_curve.wgsl");

/// The curve's two exponents for sliders `b`, `c`: (gamma g, contrast k); (1, 1) at 50 / 50.
pub fn factors(b: u8, c: u8) -> (f32, f32) {
	let g = ((NEUTRAL as f32 - b.min(MAX) as f32) / 62.5).exp2();
	let k = ((c.min(MAX) as f32 - NEUTRAL as f32) / 75.0).exp2();
	(g, k)
}

/// The shaders' uniform for sliders `b`, `c` (`grade_curve`'s `k`).
pub fn shader_k(b: u8, c: u8) -> [f32; 4] {
	let (g, k) = factors(b, c);
	[g, k, 0.0, 0.0]
}

/// The sliders in force as the shaders' uniform; None at 50 / 50 (nothing grades).
pub fn frame_k() -> Option<[f32; 4]> {
	active().then(|| shader_k(brightness(), contrast()))
}

/// The curve of `grade_curve` (gpu/grade_curve.wgsl) on one channel (the unit tests' reference; the shaders compute the same).
pub fn curve(x: f32, b: u8, c: u8) -> f32 {
	let (g, k) = factors(b, c);
	let x = x.clamp(0.0, 1.0).powf(g);
	if x < 0.5 { 0.5 * (2.0 * x).powf(k) } else { 1.0 - 0.5 * (2.0 - 2.0 * x).max(0.0).powf(k) }
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn fifty_fifty_is_the_identity_and_inactive() {
		assert_eq!(factors(50, 50), (1.0, 1.0));
		for i in 0..=255u32 {
			// (x - 0.5) + 0.5 can move the last float bit: the 8-bit value it stores is the same (the pass never runs at 50 / 50).
			let x = i as f32 / 255.0;
			assert_eq!((curve(x, 50, 50) * 255.0).round() as u32, i, "x = {x}");
		}
	}

	#[test]
	fn the_ends_and_the_direction() {
		for b in (0..=100).step_by(5) {
			assert_eq!(curve(0.0, b, 50), 0.0, "black stays black at b {b}");
			assert_eq!(curve(1.0, b, 50), 1.0, "white stays white at b {b}");
		}
		assert!(curve(0.25, 100, 50) > 0.25 && curve(0.25, 0, 50) < 0.25);
		assert!(curve(0.8, 50, 100) > 0.8 && curve(0.2, 50, 100) < 0.2, "more contrast spreads from mid-grey");
		assert!(curve(0.8, 50, 0) < 0.8 && curve(0.2, 50, 0) > 0.2, "less contrast pulls toward mid-grey");
		for c in (0..=100).step_by(5) {
			assert_eq!((curve(0.0, 50, c), curve(0.5, 50, c), curve(1.0, 50, c)), (0.0, 0.5, 1.0), "black, mid-grey and white fixed at c {c}");
		}
		let (lo, hi) = (curve(0.25, 50, 100), curve(0.25, 50, 0));
		assert!(lo > 0.1 && lo < 0.25 && hi > 0.25 && hi < 0.4, "the S-curve stays gentle: {lo} {hi}");
	}

	#[test]
	fn the_file_values() {
		assert_eq!(parse("70"), Some(70));
		assert_eq!(parse(" 72 % "), Some(70));
		assert_eq!(parse("73"), Some(75));
		assert_eq!(parse("400"), Some(100));
		assert_eq!(parse("-3"), Some(0));
		assert_eq!(parse("bright"), None);
	}

	#[test]
	fn options_round_trip_and_neutral_removes_the_key() {
		let _g = crate::pcopts::globals_guard();
		let mut o = PcOptions::parse("brightness=80\ncontrast=35\n");
		init(&o);
		assert_eq!((brightness(), contrast(), active()), (80, 35, true));
		assert!(set_from_ui(&mut o, false, 50));
		assert!(!set_from_ui(&mut o, false, 50), "no change, no save");
		assert!(set_from_ui(&mut o, true, 50));
		assert_eq!((o.extra_get(KEY_BRIGHT), o.extra_get(KEY_CONTRAST), active()), (None, None, false));
		init(&PcOptions::new());
		assert_eq!((brightness(), contrast()), (50, 50));
	}
}
