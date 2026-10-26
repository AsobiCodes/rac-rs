//! The inputs of a volumetric fog frame: what the draw pass hands over (post targets, sun, retail fog, light) turned into
//! `Input`, and the small helpers it needs.

use super::super::sunshadow::{SunFrame, SunShadow};
use super::*;

/// The Fog level of the frame (Graphics -> Fog: 0 Classic, 1 Low, 2 Medium, 3 High), only with Lighting Enhanced like the other
/// effects. RAC_TUNE=fog_q=N (0..3) overrides the row for tuning.
pub(in crate::viewer) fn level(gfx: &crate::retail::pcext::Graphics) -> u8 {
	let tuned = crate::devtune::f32("fog_q", -1.0);
	if !gfx.enhanced() {
		return 0;
	}
	if tuned >= 0.0 { (tuned as u8).min(3) } else { gfx.fog_q.min(3) }
}

/// The world uniforms as the shaders read them with volumetric fog on: retail's own fog (the tfrag / tie / shrub / vegetation / effects
/// fog) keeps only `fogopt::gs_scale()` of its share, so it does not paint a flat opaque floor under the haze. The fog factor is
/// `f = clamp(ramp(distance)) / 255` and the picture is `mix(fog colour, scene, f)`; `f' = 1 - s (1 - f)` is the same ramp with both
/// intensities mapped `i' = 255 (1 - s) + s i` (the clamps map with it), so the shaders need no change except the effects layer's
/// per-vertex fog byte, which reads `s` from `fog_colour.w`. Scale 1.0 (volumetric fog off, Classic) returns `u` untouched.
pub(in crate::viewer) fn fade_retail_fog(mut u: Uniforms) -> Uniforms {
	let s = if crate::retail::splitview::classic() { 1.0 } else { crate::retail::pcext::fogopt::gs_scale() }; // [PC] split view: the Classic pass has no volumetric fog
	if s != 1.0 {
		let a = 255.0 * (1.0 - s);
		(u.fog[2], u.fog[3], u.fog_colour[3]) = (a + s * u.fog[2], a + s * u.fog[3], s);
	}
	u
}

/// The water level the thin mist lies on: the lowest water height that carries a good share of the water vertices (a high pond or
/// a stray vertex does not lift the mist off the main water). None without water.
fn water_level(zs: &[f32]) -> Option<f32> {
	let mut v: Vec<f32> = zs.iter().copied().filter(|z| z.is_finite()).collect();
	v.sort_by(f32::total_cmp);
	v.get(v.len() / 4).copied()
}

/// RAC_FOG_LOG=1 prints one line per frame about the fog pass (inputs, level parameters).
pub(in crate::viewer) fn fog_log() -> bool {
	static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
	*S.get_or_init(|| std::env::var("RAC_FOG_LOG").is_ok())
}

/// The key light's colour (0..1): the brighter of light row 0's two colours (the level's outdoor light, as the shadows use).
pub(in crate::viewer) fn key_colour(rows: &[[[f32; 4]; 4]; 16]) -> [f32; 3] {
	let luma = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
	let c = if luma(rows[0][0]) >= luma(rows[0][2]) { rows[0][0] } else { rows[0][2] };
	[c[0].clamp(0.0, 1.5), c[1].clamp(0.0, 1.5), c[2].clamp(0.0, 1.5)]
}

/// Seconds for the wind (not wrapped: a wrap would jump the wisps), the test clock under RAC_TEST_CLOCK.
pub(in crate::viewer) fn clock() -> f32 {
	static T0: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
	crate::viewer::test_clock::secs(|| T0.get_or_init(std::time::Instant::now).elapsed().as_secs_f64() as f32)
}

impl<'a> Input<'a> {
	/// The frame's inputs, or None when Fog is Classic, the post targets are not up, or `active` is false (under water).
	/// `sheet_z`: the heights of the level's fog sheet (its "ocean" mesh) on levels that have one (`fog_sheet_ocean`);
	/// `at` = (level, picture width, height); `view` = (camera, aspect, far plane); `rows`: the level light rows.
	#[allow(clippy::too_many_arguments)]
	pub(in crate::viewer) fn new(
		post: &'a super::super::post::Post,
		sun_shadow: &'a SunShadow,
		sun: Option<&'a SunFrame>,
		ocean_z: impl Iterator<Item = f32>,
		gfx: &crate::retail::pcext::Graphics,
		u: &Uniforms,
		at: (usize, u32, u32),
		view: (&'a Camera, f32, f32),
		active: bool,
		rows: &[[[f32; 4]; 4]; 16],
	) -> Option<Input<'a>> {
		let quality = level(gfx);
		if fog_log() {
			eprintln!("volfog: level {} quality {quality} active {active} water_inputs {} fog {:?} colour {:?}", at.0, post.water_inputs().is_some(), u.fog, u.fog_colour);
		}
		// Retail's own GS fog is faded (not removed) while the volumetric fog draws; 1.0 otherwise = the retail picture.
		crate::retail::pcext::fogopt::set_gs_scale(if quality == 0 || !active { 1.0 } else { crate::devtune::f32("fog_gs", 0.35) });
		if quality == 0 || !active {
			return None;
		}
		let (scene, lin, _, generation) = post.water_inputs()?;
		let cover = post.fog_cover()?;
		let is_sheet = super::super::waterfx::fog_sheet_ocean(at.0);
		let zs: Vec<f32> = if is_sheet || density::air(at.0) == 0.0 { ocean_z.collect() } else { Vec::new() };
		let mean_z = (!zs.is_empty()).then(|| zs.iter().sum::<f32>() / zs.len() as f32);
		let (sheet_z, mist_z) = if is_sheet { (mean_z, None) } else { (None, water_level(&zs)) };
		// A level with no volume at all (the space levels: no air haze, sheet or water mist) keeps retail's fog exactly.
		if density::air(at.0) == 0.0 && sheet_z.is_none() && mist_z.is_none() {
			crate::retail::pcext::fogopt::set_gs_scale(1.0);
			return None;
		}
		if fog_log() {
			eprintln!("volfog: sheet_z {sheet_z:?} mist_z {mist_z:?} ({} water vertices)", zs.len());
		}
		Some(Input {
			cam: view.0,
			aspect: view.1,
			scene,
			lin,
			generation,
			size: (at.1, at.2),
			far: view.2,
			sun_map: &sun_shadow.array,
			sun_cmp: &sun_shadow.cmp,
			sun,
			level: at.0,
			fog: u.fog,
			fog_col: [u.fog_colour[0], u.fog_colour[1], u.fog_colour[2]],
			light: key_colour(rows),
			sheet_z,
			mist_z,
			quality,
			time: clock(),
			water: None,
			floor: floor::current(at.0),
			cover,
		})
	}
}
