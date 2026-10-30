//! Look parameters: the geometric sea (`GeoLook`), the water uniform block (`WaterU`), the tuning constants and the
//! per-Graphics-level steps (`march_steps`, `wave_look`).

use super::*;

/// [PC] Geometric sea waves by Graphics level of Water (`vs_grid`): grid cell (px), first train's length (units) and steepness,
/// steepness growth per train, height gain (x the RAC_WAVES gain); none at Low or with RAC_WAVES=off.
#[derive(Clone, Copy)]
pub(super) struct GeoLook {
	pub(super) cell: f32,
	/// The cell (px) the wave band limit is made for (`sea_w`); the grid itself is finer, so the same waves are sampled more often (a crest
	/// keeps its shape as the camera moves) and a close view is rounder.
	pub(super) band: f32,
	pub(super) len: f32,
	pub(super) sig: f32,
	pub(super) sigq: f32,
	pub(super) amp: f32,
}

pub(super) fn geo_look(level: u8) -> Option<GeoLook> {
	if wave_look(level)[0] == 0.0 {
		return None;
	}
	let mut g = match level {
		2 => GeoLook { cell: 6.0, band: 12.0, len: 12.0, sig: 0.2, sigq: 1.15, amp: 0.3 },
		_ => GeoLook { cell: 4.0, band: 8.0, len: 14.0, sig: 0.24, sigq: 1.18, amp: 0.5 },
	};
	g.amp *= wave_look(level)[2];
	Some(g)
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(super) struct WaterU {
	pub(super) vp: [[f32; 4]; 4],
	pub(super) eye: [f32; 4],
	pub(super) sun: [f32; 4],
	pub(super) proj: [f32; 4],
	pub(super) size: [f32; 4],
	pub(super) look: [f32; 4],
	pub(super) fog: [f32; 4],
	pub(super) mirror: [f32; 4],
	pub(super) flows: [[f32; 4]; FLOWS],
	pub(super) rn: [f32; 4],
	pub(super) rings: [[f32; 4]; 2 * RINGS],
	pub(super) trail: [[f32; 4]; 2 * TRAIL],
	pub(super) wbox: [f32; 4],
	pub(super) churn: [[f32; 4]; 2 * CHURN],
	pub(super) hero: [f32; 4],
	/// Sea state: trains (0 = off), High (shade and whitecaps), slope gain.
	pub(super) wave: [f32; 4],
	/// [PC] Geometric sea waves (`vs_grid`): vp's inverse; the planes (height, class bits); the shore map (origin x, y, cell,
	/// cells across); the grid (columns, rows, cell px) and its first corner and step in NDC; the trains (first steepness,
	/// growth per train, height gain, first length).
	pub(super) ivp: [[f32; 4]; 4],
	pub(super) geo: [[f32; 4]; GEO_PLANES],
	pub(super) gmap: [f32; 4],
	pub(super) ggrid: [f32; 4],
	pub(super) gridb: [f32; 4],
	pub(super) wgeo: [f32; 4],
	/// [PC] The 2nd and 3rd mirrored planes (Water High; `mirror` holds the 1st): their heights, the number of planes mirrored.
	pub(super) mirror2: [f32; 4],
	/// [PC] The far-sea look (`far_lod.rs`): near edge and far edge of the blend (units), on, the near-identical switches.
	pub(super) lod: [f32; 4],
}

/// The look's numbers (one tuning for every level).
pub(super) const RIPPLES: f32 = 1.0;
/// How far (world units, per unit of ripple slope) the picture under the surface shifts.
pub(super) const REFRACTION: f32 = 0.5;
/// The least reflectance, above the physical Fresnel (so the surface reads from straight above too).
pub(super) const SHEEN: f32 = 0.1;
pub(super) const FOAM: f32 = 0.4;
pub(super) const REFLECTION: f32 = 1.0;
pub(super) const GLINT: f32 = 2.2;
/// How clear the water is over its bed (0 = the retail colour throughout).
pub(super) const CLEAR: f32 = 1.0;

/// Steps of the reflection ray march (`reflect_ray`) by Graphics level; High = the 32 of before the levels.
pub(in crate::viewer) const fn march_steps(level: u8) -> f32 {
	match level {
		1 => 16.0,
		2 => 24.0,
		_ => 32.0,
	}
}

/// [PC] Sea state by Graphics level of Water (`waves`): 1 Low = none (the flat retail surface with its ripples), 2 Medium = four
/// trains, no shade or whitecaps, 3 High = seven trains with both. RAC_WAVES=off (dev aid): none at any level; RAC_WAVES=N: slope gain N.
pub(super) fn wave_look(level: u8) -> [f32; 4] {
	static GAIN: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
	let gain = *GAIN.get_or_init(|| match std::env::var("RAC_WAVES") {
		Ok(v) if v == "off" => 0.0,
		Ok(v) => v.parse::<f32>().unwrap_or(1.0),
		Err(_) => 1.0,
	});
	if gain == 0.0 {
		return [0.0; 4];
	}
	// RAC_WAVE_CAPT=x (dev aid): whitecap threshold shift.
	static CAPT: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
	let capt = *CAPT.get_or_init(|| std::env::var("RAC_WAVE_CAPT").ok().and_then(|v| v.parse::<f32>().ok()).unwrap_or(0.0));
	match level {
		1 => [0.0; 4],
		2 => [4.0, 0.0, gain, 0.0],
		_ => [7.0, 1.0, gain, capt],
	}
}
