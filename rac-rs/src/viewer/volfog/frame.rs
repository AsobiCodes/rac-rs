//! The inputs of a volumetric fog frame: what the draw pass hands over (post targets, sun, retail fog, light) turned into
//! `Input`, and the small helpers it needs.

use super::super::sunshadow::{SunFrame, SunShadow};
use super::*;

/// The Fog level of the frame (Graphics -> Fog: 0 Classic, 1 Low, 2 Medium, 3 High), only with Lighting Enhanced like the other
/// effects. RAC_TUNE=fog_q=N (0..3) overrides the row for tuning.
pub(in crate::viewer) fn level(gfx: &crate::retail::pcext::Graphics) -> u8 {
	level_if(gfx, crate::viewer::pipe_guard::off(crate::viewer::pipe_guard::Effect::VolFog))
}

/// `level` with the refusal given: tests check the gate without the process-wide mask, which the fog tests read in parallel.
pub(in crate::viewer) fn level_if(gfx: &crate::retail::pcext::Graphics, refused: bool) -> u8 {
	let tuned = crate::devtune::f32("fog_q", -1.0);
	if !gfx.enhanced() || refused {
		return 0; // [PC] a refused fog shader (pipe_guard.rs): retail's own fog stays unfaded
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

/// Whether `Plan::new` reads the ocean's heights on this level (the fog sheet of Kerwan, the water mist of the levels without air haze).
pub(in crate::viewer) fn wants_ocean_heights(level: usize) -> bool {
	super::super::waterfx::fog_sheet_ocean(level) || density::air(level) == 0.0
}

/// The height of the level's fog sheet: the mean over every patch of the whole ocean (`Ocean::heights`: (height, patches) per
/// descriptor, whatever the camera's distance), so it does not depend on which patches are near. None without an ocean.
fn sheet_level(h: &[(f32, usize)]) -> Option<f32> {
	let n: usize = h.iter().filter(|p| p.0.is_finite()).map(|p| p.1).sum();
	(n > 0).then(|| (h.iter().filter(|p| p.0.is_finite()).map(|p| p.0 as f64 * p.1 as f64).sum::<f64>() / n as f64) as f32)
}

/// The water level the thin mist lies on: the lowest water height that carries a good share of the water patches (a high pond or
/// a stray patch does not lift the mist off the main water): the patch at a quarter of the heights sorted. None without water.
fn water_level(h: &[(f32, usize)]) -> Option<f32> {
	let mut v: Vec<(f32, usize)> = h.iter().copied().filter(|p| p.0.is_finite() && p.1 > 0).collect();
	v.sort_by(|a, b| a.0.total_cmp(&b.0));
	let mut skip = v.iter().map(|p| p.1).sum::<usize>() / 4;
	for p in &v {
		if skip < p.1 {
			return Some(p.0);
		}
		skip -= p.1;
	}
	None
}

/// What the frame decides about the volumetric fog pass, before anything is drawn: the Fog level, and the heights of the water the
/// fog sheet / mist lie on. `Plan::new` is Some exactly when the pass draws (given its targets): the world uniforms
/// (viewer/gpu/draw/prepare.rs) and `Input::new` take the same plan, so retail's own fog is faded only while the pass runs.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::viewer) struct Plan {
	pub quality: u8,
	pub sheet_z: Option<f32>,
	pub mist_z: Option<f32>,
}

impl Plan {
	/// `fog` = the level's retail fog words of the frame (`Uniforms::fog`), `active` = Lighting Enhanced and not under water /
	/// warping / in a ship fight, `ocean` = `Ocean::heights` (empty without an ocean).
	pub(in crate::viewer) fn new(gfx: &crate::retail::pcext::Graphics, level_id: usize, fog: [f32; 4], active: bool, ocean: &[(f32, usize)]) -> Option<Plan> {
		let quality = level(gfx);
		if quality == 0 || !active {
			return None;
		}
		let is_sheet = super::super::waterfx::fog_sheet_ocean(level_id);
		let (sheet_z, mist_z) = if is_sheet {
			(sheet_level(ocean), None)
		} else if density::air(level_id) == 0.0 {
			(None, water_level(ocean))
		} else {
			(None, None)
		};
		// A level with no volume at all (the space levels: no air haze, sheet or water mist; or a retail fog that ends at nothing) keeps retail's fog exactly.
		density::level_params(level_id, fog, sheet_z, mist_z, 1.0)?;
		Some(Plan { quality, sheet_z, mist_z })
	}
}

/// The share of retail's own fog that stays this frame (`fogopt::gs_scale`): faded (not removed) while the volumetric fog draws, 1.0
/// (the retail picture) otherwise. Decided with the plan, before the world uniforms are written.
pub(in crate::viewer) fn retail_fog_scale(plan: Option<Plan>) -> f32 {
	plan.map_or(1.0, |_| crate::devtune::f32("fog_gs", 0.35))
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
	/// The frame's inputs, or None when the post targets are not up. `plan`: what `Plan::new` decided for the frame (Fog level,
	/// the fog sheet / mist heights); `at` = (level, picture width, height); `view` = (camera, aspect, far plane); `rows`: the level light rows.
	#[allow(clippy::too_many_arguments)]
	pub(in crate::viewer) fn new(
		post: &'a super::super::post::Post,
		sun_shadow: &'a SunShadow,
		sun: Option<&'a SunFrame>,
		plan: Option<Plan>,
		u: &Uniforms,
		at: (usize, u32, u32),
		view: (&'a Camera, f32, f32),
		rows: &[[[f32; 4]; 4]; 16],
	) -> Option<Input<'a>> {
		let Plan { quality, sheet_z, mist_z } = plan?;
		if fog_log() {
			eprintln!("volfog: level {} plan {plan:?} water_inputs {} fog {:?} colour {:?}", at.0, post.water_inputs().is_some(), u.fog, u.fog_colour);
		}
		let (scene, lin, _, generation) = post.water_inputs()?;
		let cover = post.fog_cover()?;
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
