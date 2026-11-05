//! The cloud sky's per-level numbers (the pack's `looks::Looks`: which levels get the new sky, the altitudes and the mix of cloud
//! genera of each planet, the retail sky shell draws the new sky replaces) and the pure helpers around them: the quality levels
//! and the sun the sky and the clouds are lit from.

/// What a Volumetric clouds level costs: march steps through the cloud shell, light-march steps, steps through the altocumulus
/// band, and the divisor of the full picture size for the grid.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Quality {
	pub steps: f32,
	pub light: f32,
	pub mid: f32,
	pub div: u32,
}

/// The settings of a Graphics -> Volumetric clouds level (`Graphics::clouds_q`: 0 Classic, 1 Low, 2 Medium, anything else High);
/// None = off.
pub const fn quality(level: u8) -> Option<Quality> {
	match level {
		0 => None,
		1 => Some(Quality { steps: 32.0, light: 4.0, mid: 3.0, div: 8 }),
		2 => Some(Quality { steps: 48.0, light: 5.0, mid: 4.0, div: 6 }),
		_ => Some(Quality { steps: 64.0, light: 6.0, mid: 6.0, div: 6 }), // High marches more steps on Medium's grid (user OK 2026-10-26); RAC_CLOUD_DIV overrides (gfxq.rs)
	}
}

pub use crate::looks::CloudLook as Look;

/// The cloud look of level `level` (the pack's `Looks::clouds`); None for levels that keep the retail sky.
pub fn look(level: usize) -> Option<Look> {
	(crate::looks::get().clouds)(level)
}

/// The sky's sun (a unit vector toward it): where the retail painted sky has its sun (`post::sky_sun`), else the level's key light;
/// never under `MIN_ELEV` (a sun at or below the horizon still lights the cloud bases).
pub fn sun_dir(painted: Option<[f32; 3]>, key_to_light: [f32; 3]) -> [f32; 3] {
	const MIN_ELEV: f32 = 0.06;
	let n = |v: [f32; 3]| {
		let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1.0e-6);
		[v[0] / l, v[1] / l, v[2] / l]
	};
	let mut v = n(painted.unwrap_or(key_to_light));
	if v[2] < MIN_ELEV {
		let h = (v[0] * v[0] + v[1] * v[1]).sqrt();
		if h < 1.0e-4 {
			return [0.0, (1.0 - MIN_ELEV * MIN_ELEV).sqrt(), MIN_ELEV];
		}
		let k = (1.0 - MIN_ELEV * MIN_ELEV).sqrt() / h;
		v = [v[0] * k, v[1] * k, MIN_ELEV];
	}
	v
}

/// The key light's direction (to the light) from the level light rows: the brighter of row 0's two lights, like `frame::key_colour`.
pub fn key_to_light(rows: &[[[f32; 4]; 4]; 16]) -> [f32; 3] {
	let luma = |c: [f32; 4]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
	let d = if luma(rows[0][0]) >= luma(rows[0][2]) { rows[0][1] } else { rows[0][3] };
	[d[0], d[1], d[2]]
}

/// The retail sky shell draws (indices into the sky batch's draws) the new sky replaces on `level` (the pack's `Looks::sky_mask`).
pub fn sky_mask(level: usize) -> u64 {
	(crate::looks::get().sky_mask)(level)
}

/// True when the new sky draws its own sun disc on `level` (the pack's `Looks::sun_disc`).
pub fn sun_disc(level: usize) -> bool {
	(crate::looks::get().sun_disc)(level)
}
