//! The fog's own part of the volumetric pass: how thick the air is at a world point (height fog that pools low, drifting noise
//! wisps, and Kerwan's chasm fog sheet turned into a volume) and how the retail fog of a level sets its amount. The light
//! scattering, the march and the temporal accumulation are shared with other media (scatter.rs, temporal.rs); clouds would add
//! a `medium_density` of their own.

/// WGSL: the extinction per world unit at `p`, placed before `scatter::MARCH`.
pub const FOG_DENSITY: &str = r#"
@group(0) @binding(9) var wpos: texture_2d<f32>;

// The view depth of the water surface over fog pixel `c` (the water pass's position target, a > 0 where water was drawn), or a
// huge value: the fog ends at the water, never at the seabed or the void below it (Pokitaru's sea read as holes and floating
// seabed patches when the march ran on under the surface).
fn surface_t(c: vec2<i32>) -> f32 {
	let dim = vec2<i32>(textureDimensions(wpos));
	let w = textureLoad(wpos, min(c * i32(f.lin.z) * 2, dim - vec2<i32>(1)), 0);
	if (w.a < 0.5) { return 1.0e30; }
	let fw = f.cam[1].xyz;
	return max(dot(w.xyz - f.cam[0].xyz, fw) / dot(fw, fw), 0.0);
}

fn medium_density(p: vec3<f32>) -> f32 {
	let drift = vec3<f32>(f.wind.xy * (f.wind.z * f.wind.w), 0.0);
	let n1 = noise_at((p - drift) / f.look.y).r;
	let n2 = noise_at((p - drift * 1.6) / f.look.z).b;
	let c = clamp((n1 * 0.65 + n2 * 0.35 - 0.5) * 3.2 + 0.5, 0.0, 1.0);
	let hs = f.dens.y;
	// Strata: slowly drifting sheets (noise stretched along z) so the top of the pool breaks up into layers.
	let lay = noise_at(vec3<f32>((p.xy - drift.xy * 0.4) / (f.look.y * 2.1), p.z / (hs * 1.3))).a;
	// The top is soft and uneven: the noise lifts and sinks it by about a scale height, and the density fades over several
	// scale heights (a shelf, then a long exponential tail) instead of ending at a plane.
	let top = f.dens.z + ((0.5 - c) * 1.5 + (lay - 0.5) * 2.0) * hs * f.look.x;
	let h = p.z - top;
	let shelf = 1.0 - smoothstep(-1.0 * hs, 2.4 * hs, h);
	let tail = exp(-max(h, 0.0) / (2.5 * hs));
	let prof = shelf * 0.65 + tail * 0.35;
	let wisp = mix(1.0, 0.2 + 1.6 * c, f.look.x * 0.8);
	// Low drifting ground wisps: thicker puffs hugging the lowest part of the pool.
	let hg = max(p.z - f.dens.z, 0.0);
	let ground = c * c * exp(-hg / (0.5 * hs)) * f.look.x * 1.4;
	var d = f.dens.x * (f.dens.w + prof * wisp * (0.8 + 0.4 * lay) + ground);
	if (f.sheet.w > 0.5) {
		let hz = max(p.z - f.sheet.x + (0.5 - c) * 4.0, 0.0);
		d += f.sheet.z * exp(-hz / f.sheet.y) * (0.55 + 0.9 * c);
	}
	return d;
}
"#;

/// A fog sheet of the level (Kerwan's chasm fog): the plane the retail layer is drawn at; below it the air is thick, above it
/// it thins out over `height` units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sheet {
	pub z: f32,
	pub height: f32,
	pub sigma: f32,
}

/// What a level's fog makes of the volume.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Params {
	/// Extinction per world unit at the ground.
	pub sigma: f32,
	/// Height scale of the pool (world units), and the fraction of the ground density left high above it.
	pub height: f32,
	pub haze: f32,
	/// How far the march reaches (world units of view depth).
	pub cap: f32,
	pub sheet: Option<Sheet>,
}

/// Share of the picture the level's retail fog takes at its far end: 0 for a level without fog (both intensities 255), 1 for
/// fog that ends at the fog colour.
pub fn fog_weight(fog: [f32; 4]) -> f32 {
	((fog[2] - fog[3]) / 255.0).clamp(0.0, 1.0)
}

/// How much open-air haze a level gets, 0..1, on top of what its retail fog weight says (user 2026-10-24: "novalis should not have
/// fog at all, maybe a slight fog near the water"). Novalis and Kalebo III (identical sky-blue retail fog: a daylight horizon
/// haze the retail fade already paints) keep no air haze; the levels set in space (Nebula G34, Gemlik Base, Drek's fleet: user
/// 2026-10-24 "gemlik base ... literally set in space") none either; the others keep the amount their retail fog sets.
pub fn air(level: usize) -> f32 {
	match level {
		1 | 6 | 13 | 16 | 17 => 0.0,
		_ => 1.0,
	}
}

/// The highest world z the fog pool's top may stand at (the pool otherwise follows the camera, 3 units under it). Veldin: the
/// canyon pool lies low (the canyon view at z 14.5 keeps its soft pool); on the open ground at z 30 and over the platforms
/// the air is clear apart from the thin haze, wisps and the pool far below. Eudora: the islands' paths lie at z ~50; a pool
/// following a camera there blued the cliff faces a few dozen units away (user 2026-10-25, hero 179.7 220.2 51 and a free
/// camera at z 77; at a cap of 45 still the cliffs across the gap from hero 181.2 214.0 51): the pool stays well under the
/// island tops, the gaps keep their fog floor.
pub fn pool_cap(level: usize) -> f32 {
	match level {
		0 => 12.0,
		4 => 20.0,
		_ => f32::INFINITY,
	}
}

/// The most extinction per world unit the fog adds at the ground: levels whose retail fog ends within 60-100 units (Batalia, Orxon,
/// Gemlik) would otherwise get the thickest mist; this keeps about 35% haze at 100 units at most.
const MAX_SIGMA: f32 = 0.0035;

/// Height of the thin mist above a water surface on a level without general haze, in world units.
const MIST_HEIGHT: f32 = 2.5;

/// The volume of a level: driven by its retail fog `fog` = (near, far, intensity near, intensity far) so each planet keeps its
/// mood (a thick retail fog gets thicker mist, a level without fog none), scaled by `air(level)`, plus its fog sheet where it
/// has one (Kerwan's chasm) or, on a level without air haze, a thin mist over its water at `mist_z`. None: nothing to draw.
pub fn level_params(level: usize, fog: [f32; 4], sheet_z: Option<f32>, mist_z: Option<f32>, k: f32) -> Option<Params> {
	let w = fog_weight(fog);
	let span = (fog[1] - fog[0]).clamp(60.0, 500.0);
	let air = air(level);
	let mist = (air == 0.0).then_some(mist_z).flatten();
	let sheet = sheet_z.map(|z| Sheet { z, height: 16.0, sigma: 0.075 }).or_else(|| mist.map(|z| Sheet { z: z + 0.5, height: MIST_HEIGHT, sigma: 0.06 }));
	let w = if w < 0.04 { 0.0 } else { w * air };
	if w == 0.0 && sheet.is_none() {
		return None;
	}
	Some(Params { sigma: (k * w / span).min(MAX_SIGMA), height: 15.0, haze: 0.12, cap: fog[1].clamp(90.0, 260.0), sheet })
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn retail_fog_sets_the_amount() {
		// Veldin: near 0, far 235, intensity 255 -> 53.55 (decomp/notes/lighting_fog.md section 1).
		let veldin = level_params(0, [0.0, 235.0, 255.0, 53.55], None, None, 0.8).unwrap();
		assert!(veldin.sigma > 0.001 && veldin.sigma < 0.006, "{veldin:?}");
		assert_eq!(veldin.cap, 235.0);
		// A level without retail fog gets nothing, unless it has a sheet.
		assert!(level_params(5, [0.0, 900.0, 255.0, 255.0], None, None, 0.8).is_none());
		let kerwan = level_params(3, [0.0, 900.0, 255.0, 255.0], Some(-24.0), None, 0.8).unwrap();
		assert_eq!((kerwan.sigma, kerwan.cap, kerwan.sheet.map(|s| s.z)), (0.0, 260.0, Some(-24.0)));
	}

	#[test]
	fn novalis_has_no_haze_only_water_mist() {
		let novalis = [0.0, 240.0, 255.0, 102.0];
		assert!(level_params(1, novalis, None, None, 0.8).is_none());
		let mist = level_params(1, novalis, None, Some(10.0), 0.8).unwrap();
		assert_eq!(mist.sigma, 0.0);
		assert!(mist.sheet.is_some_and(|s| s.z > 10.0 && s.height < 5.0));
		// A level with air haze never gets a water mist sheet.
		assert!(level_params(0, novalis, None, Some(10.0), 0.8).unwrap().sheet.is_none());
	}

	#[test]
	fn the_pool_stays_low_on_veldin_and_eudora() {
		assert_eq!((pool_cap(0), pool_cap(4)), (12.0, 20.0));
		assert!(pool_cap(3).is_infinite());
	}
}
