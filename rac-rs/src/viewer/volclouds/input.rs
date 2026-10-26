//! The inputs of a cloud frame: what the draw pass hands over (post target size, light rows, fog colour, camera) turned into `Input`.

use super::super::volfog::frame;
use super::*;

/// The Volumetric clouds level of the frame (Graphics -> Volumetric clouds: 0 Classic, 1 Low, 2 Medium, 3 High), only with Lighting
/// Enhanced like the other effects. RAC_TUNE=cl_q=N (0..3) overrides the row for tuning.
pub(in crate::viewer) fn level(gfx: &crate::retail::pcext::Graphics) -> u8 {
	let tuned = crate::devtune::f32("cl_q", -1.0);
	if !gfx.enhanced() {
		return 0;
	}
	if tuned >= 0.0 { (tuned as u8).min(3) } else { gfx.clouds_q.min(3) }
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
	pub time: f32,
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
			time: frame::clock(),
		})
	}
}
