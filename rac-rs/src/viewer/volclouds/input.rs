//! The inputs of a cloud frame: what the draw pass hands over (post target size, light rows, fog colour, camera) turned into `Input`.

use super::super::volfog::frame;
use super::*;

/// The Volumetric clouds level of the frame (Graphics -> Volumetric clouds: 0 Classic, 1 Low, 2 Medium, 3 High), only with Lighting
/// Enhanced like the other effects. RAC_TUNE=cl_q=N (0..3) overrides the row for tuning.
pub(in crate::viewer) fn level(gfx: &crate::retail::pcext::Graphics) -> u8 {
	use crate::viewer::pipe_guard::{off, Effect};
	level_if(gfx, off(Effect::VolClouds) || off(Effect::Post))
}

/// `level` with the refusal given: tests check the gate without the process-wide mask, which other tests read in parallel.
fn level_if(gfx: &crate::retail::pcext::Graphics, refused: bool) -> u8 {
	let tuned = crate::devtune::f32("cl_q", -1.0);
	if !gfx.enhanced() || refused {
		return 0; // [PC] a refused cloud or post shader (pipe_guard.rs): no sky backdrop without its clouds, no Enhanced sky under a refused Post
	}
	if tuned >= 0.0 { (tuned as u8).min(3) } else { gfx.clouds_q.min(3) }
}

/// Seconds for the wind and the cloud evolution, as `frame::clock` but in f64 (the test clock's value under RAC_TEST_CLOCK).
fn clock() -> f64 {
	static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
	if crate::viewer::test_clock::clock_on() {
		return f64::from(crate::viewer::test_clock::secs(|| 0.0));
	}
	T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() - f64::from(crate::viewer::handler::frame_clock::lag())
}

/// The inputs of one frame.
pub(in crate::viewer) struct Input<'a> {
	pub cam: &'a Camera,
	pub aspect: f32,
	/// The post targets' generation and the full picture size (the grid follows them).
	pub generation: u64,
	pub size: (u32, u32),
	pub far: f32,
	pub level: usize,
	/// The level's retail fog colour (0..1) and the key light's colour (0..1) and direction to the light.
	pub fog_col: [f32; 3],
	pub light: [f32; 3],
	pub to_light: [f32; 3],
	pub quality: u8,
	/// Seconds of the cloud clock (f64: it is never wrapped, and an f32 loses the frame step after hours).
	pub time: f64,
}

impl<'a> Input<'a> {
	/// The frame's inputs, or None when the clouds are Classic, the level has no sky for them, the post targets are not up, or
	/// `active` is false (under water). `at` = (level, picture width, height); `view` = (camera, aspect, far plane).
	pub(in crate::viewer) fn new(
		post: &'a super::super::post::Post,
		gfx: &crate::retail::pcext::Graphics,
		u: &Uniforms,
		at: (usize, u32, u32),
		view: (&'a Camera, f32, f32),
		active: bool,
		rows: &[[[f32; 4]; 4]; 16],
	) -> Option<Input<'a>> {
		let quality = level(gfx);
		if quality == 0 || !active || params::look(at.0).is_none() {
			return None;
		}
		let (_, _, _, generation) = post.water_inputs()?;
		Some(Input {
			cam: view.0,
			aspect: view.1,
			generation,
			size: (at.1, at.2),
			far: view.2,
			level: at.0,
			fog_col: [u.fog_colour[0], u.fog_colour[1], u.fog_colour[2]],
			light: frame::key_colour(rows),
			to_light: params::key_to_light(rows),
			quality,
			time: clock(),
		})
	}
}

#[cfg(test)]
mod level_tests {
	use super::*;

	#[test]
	fn a_refused_cloud_shader_gives_level_zero() {
		let g = crate::retail::pcext::Graphics { lighting: 1, clouds_q: 2, ..crate::retail::pcext::Graphics::CLASSIC };
		assert_eq!(level_if(&g, false), 2);
		assert_eq!(level_if(&g, true), 0, "no sky backdrop without its clouds");
	}
}
