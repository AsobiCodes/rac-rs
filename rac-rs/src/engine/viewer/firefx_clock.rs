//! [PC] Fire: the clock of the fire shaders (firefx.wgsl, the mesh flames in gpu/moby_fire.rs) and of the High lights (review A7 / A17 / A26).
//!
//! The shaders get seconds inside a PERIOD (512 s), not the session's age, so no argument grows into the tens of thousands (f32 noise
//! coordinates and `sin` arguments lose their fine steps there) and no clock is wrapped at a bad moment:
//! - Everything that repeats (the embers' climb cycles, their wobble and flicker phases) uses whole cycles per period (the shader rounds
//!   `PERIOD / life` and `PERIOD * omega / tau` to whole numbers), so it is seamless at the wrap by construction.
//! - The noise pattern (the flame, the fireball, the mesh flame) cannot be periodic, so for the last BLEND seconds before the wrap the
//!   shader mixes the pattern of `t` with the pattern of `t - PERIOD` (the moment it wraps to) by `blend`, a smooth 0 -> 1: at the wrap
//!   both sides show the same picture, and outside that window `blend` is 0 and nothing costs more.
//! - The lights (High) work on the unwrapped f64 game time, where `sin` is exact for years.
//! The scroll speed knob (`fire_speed`) scales the time here (`Clock::game`), not in the shader.
//! Everything runs on the GAME clock (`firefx_gametime.rs`): the shader noise, the embers, the mesh flames and the High lights' flicker
//! (review R7-7) as well as the patches, scorch, burnt grass and jet memory, so the whole fire stands still in a pause or the Quick
//! Select wheel (retail's sprites and quads are frozen there too) and flows like the wall clock while the game runs. The wall clock
//! `age()` below is only the input of that game clock.

use super::test_clock;

/// Seconds the shader clock runs through before it wraps. A power of two (the shaders divide by it exactly); firefx.wgsl and
/// gpu/moby_fire.rs repeat the number (a test compares them).
pub(in crate::viewer) const PERIOD: f64 = 512.0;
/// The last seconds of a period in which the next period's pattern is blended in.
pub(in crate::viewer) const BLEND: f64 = 3.0;

/// The fire clock of a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::viewer) struct Clock {
	/// Seconds into the period, 0..PERIOD.
	pub t: f32,
	/// Weight of the pattern at `t - PERIOD` (the one the clock wraps to): 0 until BLEND seconds before the wrap, then up to 1 smoothly.
	pub blend: f32,
}

impl Clock {
	/// The clock `age` seconds after the start.
	pub(in crate::viewer) fn at(age: f64) -> Clock {
		let t = age.rem_euclid(PERIOD);
		let x = ((t - (PERIOD - BLEND)) / BLEND).clamp(0.0, 1.0);
		Clock { t: t as f32, blend: (x * x * (3.0 - 2.0 * x)) as f32 }
	}

	/// The shader clock at game time `game` (`FireFx::shown`, `GameClock::shown` in firefx_gametime.rs), the speed knob applied.
	pub(in crate::viewer) fn game(game: f64) -> Clock {
		Clock::at(game * crate::devtune::f32("fire_speed", 1.0) as f64)
	}
}

/// Seconds since the first call (wall time, or the test clock's), unwrapped: the input of the game clock (`GameClock::update`).
pub(in crate::viewer) fn age() -> f64 {
	static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
	test_clock::secs64(|| T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64())
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_clock_stays_inside_its_period_and_blends_only_before_the_wrap() {
		assert_eq!(Clock::at(0.0), Clock { t: 0.0, blend: 0.0 });
		assert_eq!(Clock::at(100.25).blend, 0.0);
		assert_eq!(Clock::at(PERIOD - BLEND - 0.01).blend, 0.0);
		let mid = Clock::at(PERIOD - BLEND * 0.5).blend;
		assert!((mid - 0.5).abs() < 1e-6, "smoothstep is 0.5 in the middle of the window: {mid}");
		for k in [1.0, 7.0, 1000.0, 90000.0] {
			let c = Clock::at(k * PERIOD + 12.5);
			assert_eq!((c.t, c.blend), (12.5, 0.0), "{k} periods later the same clock");
		}
	}

	/// No jump at the wrap: the window ends at blend 1 on `t - PERIOD` just before and at blend 0 on `t` just after, and `t - PERIOD`
	/// just before the wrap is the `t` just after it; the blend is monotonic and has no step.
	#[test]
	fn the_wrap_is_continuous() {
		let before = Clock::at(PERIOD - 1e-4);
		let after = Clock::at(PERIOD + 1e-4);
		assert!((before.blend - 1.0).abs() < 1e-6 && after.blend == 0.0);
		assert!(((before.t as f64 - PERIOD) - (after.t as f64)).abs() < 1e-3, "the pattern shown just before = the pattern just after");
		let mut last = 0.0;
		let mut age = PERIOD - BLEND;
		while age < PERIOD {
			let b = Clock::at(age).blend;
			assert!(b >= last && b - last < 0.02, "blend rises in small steps: {last} -> {b}");
			last = b;
			age += 0.005;
		}
	}

	/// The number the shaders repeat is this module's.
	#[test]
	fn the_shaders_repeat_the_period() {
		let want = format!("PERIOD: f32 = {:.1};", PERIOD);
		assert!(include_str!("firefx.wgsl").contains(&want), "firefx.wgsl PERIOD");
		assert!(super::super::gpu::moby_fire::WGSL.contains(&format!("MF_{want}")), "moby_fire.rs MF_PERIOD");
		assert!(PERIOD.log2().fract() == 0.0, "a power of two: the shaders divide by it exactly");
	}
}
