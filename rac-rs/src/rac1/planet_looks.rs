//! R&C1's per-planet looks (engine split stage E2 batch 5, 2026-10-26: the tables moved out of the engine's effect code, which
//! asks through engine/looks.rs): which draws of a level are water, lava, mud, sludge or ice (`water.rs`), its sky (the painted
//! sun, the turning shells, Gemlik's star, the star twinkle), its clouds (`clouds.rs`), its fog, its swamp and natural waters.
//! Level = the loaded level index: veldin novalis aridia kerwan eudora rilgar nebula_g34 umbris batalia gaspar orxon pokitaru
//! hoven gemlik oltanis quartu kalebo fleet veldin2.

use crate::looks::{FogFloor as Tune, Looks, FLOOR_REACH as REACH, FLOOR_SOFT_MIN as SOFT_MIN};

mod clouds;
mod water;

/// R&C1's answers (`Pack::looks`, rac1/pack_bind.rs).
pub static LOOKS: Looks = Looks {
	ocean_liquid: water::ocean_liquid,
	fog_sheet_ocean: water::fog_sheet_ocean,
	terrain_liquid: water::terrain_liquid,
	pool_water_tex: water::pool_water_tex,
	cold_pool: water::cold_pool,
	frozen_pool: water::frozen_pool,
	sea_fx_tex: water::sea_fx_tex,
	pond_fx_tex: water::pond_fx_tex,
	fall_fx_tex: water::fall_fx_tex,
	river_fx_tex: water::river_fx_tex,
	strips_are_water: water::strips_are_water,
	known_tide: water::known_tide,
	sky_sun,
	sky_key: |l| SKY_KEY.contains(&l),
	sky_key_lit: |l| SKY_KEY_LIT.contains(&l),
	sky_key_water: |l| SKY_KEY_WATER.contains(&l),
	shell_angle,
	star,
	twinkle,
	clouds: clouds::look,
	sky_mask: clouds::sky_mask,
	sun_disc: clouds::sun_disc,
	sunless: |l| l == FLEET,
	air,
	pool_cap,
	fog_floor: tune,
	liquid_bottom,
	ice_keys,
	swamp: |l| SWAMP_LEVELS.contains(&l),
	natural_water: |l| SEA_LEVELS.contains(&l),
};

/// Drek's fleet: space and ship insides, no sun. Its bright outdoor light row cast Dynamic shadows from the ceilings, and the
/// sun through the glass walls lit sharp patches on floors that retail lights evenly (user 2026-11-04 "dreks fleet is lit in some
/// places where it shouldnt"; fg2 17_0186 against its PCSX2 shot): `Looks::sunless`.
const FLEET: usize = 17;

// The sky.

/// Sun rays: the direction towards the sun level `level`'s sky paints (sky mesh axes = world axes, Z up), or None for a
/// sky without one. Measured from the sky shells (centroid of the brightest texels in a one-off dump of every level's sky, 2026-10-15): the day
/// skies of Novalis, Kerwan, Eudora, Pokitaru and Kalebo III share one dome (shell 0) whose bright glow is centred at
/// azimuth 0, elevation 36 degrees; Veldin's dusk sun sits on the horizon band (shell 4); Gemlik's red sun is a disc
/// (shell 2). The night, space, storm and lava skies have none.
fn sky_sun(level: usize) -> Option<[f32; 3]> {
	match level {
		0 => Some([0.9888, 0.0174, 0.1478]),
		1 | 3 | 4 | 11 | 16 => Some([0.8068, 0.0031, 0.5906]),
		13 => Some([-0.3961, 0.8970, 0.1917]),
		_ => None,
	}
}

/// Levels whose Enhanced sun follows the painted sky sun (engine/viewer/gpu/lighting.rs `Gpu::sun_rows`): every level whose retail
/// key light (light row 0) stands more than `SKY_KEY_MIN_DEG` away from the sun its sky paints (where the cloud sky also draws its
/// sun disc), `RETAIL_KEY` below: Veldin (65 degrees), Novalis (19), Eudora (111), Pokitaru (41; user 2026-11-03: "the shadows are
/// placed in the wrong direction relative to the sun"), Gemlik (64; user 2026-10-25: the big orange star stood behind Ratchet and
/// the shadows fell to his right), Kalebo III (29). Kerwan's key light stands 3 degrees from its painted sun: it keeps it.
const SKY_KEY: [usize; 6] = [0, 1, 4, 11, 13, 16];

/// Of those, the levels whose Enhanced shading takes light row 0's turned key light too (frame uniforms slot 0: mobies, the hero,
/// Metal highlights on the world and mobys, fog glow, smoke), so a lit side faces the sun the shadows point away from (else a
/// shadow from the painted sun fell on the side retail's key light lit: Eudora's key light comes from behind its painted sun).
/// Veldin keeps retail's shading (approved 2026-10-24). The water's bed and mirror passes keep retail's key light where
/// `SKY_KEY_WATER` leaves the level out (`Looks::water_keeps_key`).
const SKY_KEY_LIT: [usize; 5] = [1, 4, 11, 13, 16];

/// Of those, the levels whose water sky map (waterfx/skymap.rs) takes the turned key light too: Veldin and Gemlik, which followed
/// the painted sun before Novalis, Eudora, Pokitaru and Kalebo III joined (lane nsun, 2026-11-03). The approved Enhanced water
/// (2026-10-15, Pokitaru and Novalis included) keeps retail's key light there; only their sun shadows, fog glow, clouds and
/// shading follow the painted sun.
const SKY_KEY_WATER: [usize; 2] = [0, 13];

/// NOV-31: the z angle of sky shell `shell` at game frame `frames` (`Game.frames`), or None when that shell keeps the
/// fixed orientation. The Novalis sky pass (level 1) turns shells 2, 3 and 4 by 2 pi per 2^18, 2^17 and 2^16 frames
/// (decomp/notes/novalis_environment.md "NOV-31 port spec"); the Kerwan sky pass (level 3, KER-72a in
/// decomp/notes/kerwan_environment.md) turns shells 1 and 2 by 2 pi per 2^16 and 2^17 frames and keeps shell 0 fixed.
/// Every turning shell starts at -pi. All other levels draw their shells unturned.
fn shell_angle(level: usize, shell: usize, frames: u32) -> Option<f32> {
	let (mask, step) = match (level, shell) {
		(1, 2) => (0x3ffff, f32::from_bits(0x37c9_0fdb)),
		(1, 3) | (3, 2) => (0x1ffff, f32::from_bits(0x3849_0fdb)),
		(1, 4) | (3, 1) => (0xffff, f32::from_bits(0x38c9_0fdb)),
		_ => return None,
	};
	Some((frames & mask) as f32 * step - std::f32::consts::PI)
}

/// Gemlik Base's big orange star (engine/viewer/star.rs, the animated star): the retail face key of its sky patch (sky shell 2,
/// texture hash 1885e61187b6ebe4), on level 13 only.
const STAR_LEVEL: usize = 13;
const STAR_KEY: u32 = 0x40005;

fn star(level: usize) -> Option<u32> {
	(level == STAR_LEVEL).then_some(STAR_KEY)
}

/// The twinkle dip depth of a level: the planets with air dip fully, the space levels (no air) softly.
fn twinkle(level: usize) -> f32 {
	if matches!(level, 13 | 17) { 0.2 } else { 0.35 }
}

// The fog.

/// How much open-air haze a level gets, 0..1, on top of what its retail fog weight says (user 2026-10-24: "novalis should not have
/// fog at all, maybe a slight fog near the water"). Novalis and Kalebo III (identical sky-blue retail fog: a daylight horizon
/// haze the retail fade already paints) keep no air haze; the levels set in space (Nebula G34, Gemlik Base, Drek's fleet: user
/// 2026-10-24 "gemlik base ... literally set in space") none either; the others keep the amount their retail fog sets.
fn air(level: usize) -> f32 {
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
fn pool_cap(level: usize) -> f32 {
	match level {
		0 => 12.0,
		4 => 20.0,
		_ => f32::INFINITY,
	}
}

/// The fog floor's shape per level (engine/viewer/volfog/floor.rs; `Tune` = `looks::FogFloor`).
fn tune(level: usize) -> Tune {
	match level {
		// Kerwan: a flat deck with its solid top at the retail fog sheet (fs_ocean_k, z -24; the lowest clear top is 4.6), grey like the
		// sheet's FOGCOL 96 (user 2026-10-25: retail's sheet hides every tower bottom, our deck at 2.6 covered whole buildings); a tower
		// ending over the sheet gets the solid top raised to its bottom (`solid_map`). Thin (sigma 0.12) under a 36-unit soft top
		// (clear top +12): the towers fade into the grey over ~30 units like retail's (user 2026-10-25, "look how abruptly our
		// buildings end"; a 6-unit soft top at sigma 1 cut them off).
		3 => Tune { lift: 7.4, soft: 36.0, cap: 36.0, reach: REACH, desat: 1.0, to_under: 0.0, under: [0; 3], flat: true, sigma: 0.12, void_fade: 0.0 },
		// Eudora: the fog hugs the nearest ground (reach 1 cell) and fades over 30 units, so each cliff tier fades under its own rim; over
	// the gaps between islands and under the rims it starts 12 units over the cliffs' bottoms (`lower_voids`), and it is thin (sigma
	// 0.06: user 2026-10-25 "too foggy in some places where it shouldnt", at sigma 1 an opaque wall stood a few units under every rim
	// and hid the cliff tiers Classic shows; at 0.15 the far cliffs seen across a gap from the paths still looked blue-washed).
		4 => Tune { lift: 0.0, soft: 30.0, cap: 1.0e9, reach: 1, desat: 0.5, to_under: 0.0, under: [0; 3], flat: false, sigma: 0.06, void_fade: 12.0 },
		// Quartu: the pit's green was a flat saturated plane the rock edges ended against.
		15 => Tune { lift: 0.0, soft: SOFT_MIN, cap: 1.0e9, reach: REACH, desat: 0.5, to_under: 0.0, under: [0; 3], flat: false, sigma: 1.0, void_fade: 0.0 },
		// Veldin 2: retail's canyon fades to the near-black clear colour (10,10,10), not to the dark green fog colour; the cliffs fade
		// into it over a thin floor under a 30-unit soft top raised 10 units (user 2026-10-25, the canyon cliffs ended against a flat
		// dark green with pale lit haze shapes).
		18 => Tune { lift: 10.0, soft: 30.0, cap: 1.0e9, reach: REACH, desat: 1.0, to_under: 0.0, under: [0; 3], flat: false, sigma: 0.15, void_fade: 0.0 },
		// Veldin: retail's alpha-faded cliff and rock bottoms fade into its red-brown sky below the horizon (77,40,25), not into the
		// darker fog colour (47,26,15) the floor painted (user 2026-10-25: Classic fades the bottoms into the red-brown, Enhanced into
		// near black).
		0 => Tune { lift: 0.0, soft: SOFT_MIN, cap: 1.0e9, reach: REACH, desat: 0.0, to_under: 1.0, under: [77, 40, 25], flat: false, sigma: 1.0, void_fade: 0.0 },
		_ => Tune { lift: 0.0, soft: SOFT_MIN, cap: 1.0e9, reach: REACH, desat: 0.0, to_under: 0.0, under: [0; 3], flat: false, sigma: 1.0, void_fade: 0.0 },
	}
}

/// Whether `level`'s bottom is a liquid (no floor there: nothing hangs over a void). Pokitaru (11): its retail sea writes no depth
/// and lies partly outside the water pass's position target, so its pixels read as sky to the fog: a floor 1 unit under the beach
/// stopped the deep downward march (scatter.rs `end_depth`) at the sea and laid solid fog over the seabed, and the open sea turned
/// bluer than dev s117's (the march stop made most of the change). Orxon (10): its toxic sea is terrain (`terrain_liquid`, z 27-62)
/// under the floor's clear top 29, which flattened it to the fog colour. Oltanis (14) has the floor again (agent 5, 2026-10-25:
/// fullgame1 14_0692 game and raised views vs dev s117: tower feet fade a little more, the ground and sea round them are unchanged).
/// Hoven (12): its chasms end at the river and the abyss sheet (z 25-29) between rock walls; the floor's clear top (24.08) laid a
/// flat fog-coloured deck over that rock where retail and Fog Classic show dark rock (user 2026-10-28 bridge screenshot, "there
/// shouldnt be any fog there"; PCSX2 fullgame1 0564 under the bridge).
fn liquid_bottom(level: usize) -> bool {
	matches!(level, 10 | 11 | 12)
}

// Ice and vegetation.

/// The level's ice textures: (tie keys, shrub keys). Hoven (level 12) only.
fn ice_keys(level: usize) -> (&'static [u32], &'static [u32]) {
	match level {
		12 => (HOVEN_TIES, HOVEN_SHRUBS),
		_ => (&[], &[]),
	}
}

/// Hoven's ice [C: debug runs drawing one key at a time in magenta on play/compare/Hoven.racpad fields 200 / 500 / 1950, 2026-10-16]: tie
/// texture 0x2f the ice floes and crystals standing in the abyss and by the village water, 0x31 the small ice pieces at the
/// village edge; shrub texture 8 the icicles under the roofs. Not ice: tie 0x28 (the platforms' light pillars), shrub 0x11
/// (snow-capped rocks).
const HOVEN_TIES: &[u32] = &[0x2002f, 0x20031];
const HOVEN_SHRUBS: &[u32] = &[0x30008];

/// The dark swamp levels (Umbris).
const SWAMP_LEVELS: [usize; 1] = [7];

/// The planets whose waters are natural (sea, lakes, rock pools): Novalis and Pokitaru. Quartu's indoor tanks and Rilgar's
/// city canals also have deep water but no place for kelp.
const SEA_LEVELS: [usize; 2] = [1, 11];

#[cfg(test)]
mod tests {
	use super::*;

	/// NOV-31 spec tests 2-5 (float32 values).
	#[test]
	fn novalis_shell_angles() {
		let pi = -std::f32::consts::PI;
		assert_eq!([2, 3, 4].map(|s| shell_angle(1, s, 0)), [Some(pi); 3]);
		assert_eq!([2, 3, 4].map(|s| shell_angle(1, s, 100000)), [Some(-0.74474764), Some(1.6520975), Some(0.16260195)]);
		assert_eq!([2, 3, 4].map(|s| shell_angle(1, s, 262149)), [Some(-3.1414728), Some(-3.1413531), Some(-3.1411133)]);
		assert!([0, 1, 5].iter().all(|&s| shell_angle(1, s, 12345).is_none()));
		assert!((0..6).all(|s| shell_angle(0, s, 100000).is_none() && shell_angle(2, s, 100000).is_none()));
	}

	/// KER-72a spec tests 1-4 (float32 values).
	#[test]
	fn kerwan_shell_angles() {
		let pi = -std::f32::consts::PI;
		assert_eq!(pi.to_bits(), 0xc049_0fdb);
		assert_eq!([1, 2].map(|s| shell_angle(3, s, 0)), [Some(pi); 2]);
		assert_eq!(shell_angle(3, 1, 32768).map(f32::to_bits), Some(0));
		assert_eq!(shell_angle(3, 1, 98304), shell_angle(3, 1, 32768));
		assert_eq!(shell_angle(3, 2, 98304).map(f32::to_bits), Some(0x3fc9_0fda));
		assert_eq!(shell_angle(3, 1, 65536 + 777), shell_angle(3, 1, 777));
		assert_eq!(shell_angle(3, 2, 131072 + 777), shell_angle(3, 2, 777));
		assert!([0, 3, 4].iter().all(|&s| shell_angle(3, s, 12345).is_none()));
	}

	/// The smallest angle between a painted sun and the retail key light at which the Enhanced sun follows the painted one.
	const SKY_KEY_MIN_DEG: f32 = 10.0;

	/// Retail light row 0's key light (to the light; the brighter of the pair) of the levels with a painted sun [C: the level
	/// gameplay light table, DirLight 0 of extracted/levels/*/gameplay.instances = `formats::gameplay::lighting`, 2026-11-03].
	const RETAIL_KEY: [(usize, [f32; 3]); 7] = [
		(0, [0.397131, -0.851651, 0.342020]),
		(1, [0.777424, -0.319597, 0.541729]),
		(3, [0.817714, -0.048515, 0.573576]),
		(4, [-0.715475, -0.583061, 0.384883]),
		(11, [0.576041, -0.646666, 0.5]),
		(13, [-0.939692, 0.000768, 0.342020]),
		(16, [0.724588, -0.474312, 0.5]),
	];

	fn angle_deg(a: [f32; 3], b: [f32; 3]) -> f32 {
		let len = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
		((a[0] * b[0] + a[1] * b[1] + a[2] * b[2]) / (len(a) * len(b))).clamp(-1.0, 1.0).acos().to_degrees()
	}

	/// The class rule of `SKY_KEY`: a painted sun more than `SKY_KEY_MIN_DEG` from the retail key light turns the Enhanced sun to it;
	/// every level with a painted sun is in `RETAIL_KEY`; a level without one is never in the list.
	#[test]
	fn the_enhanced_sun_follows_every_painted_sun_the_key_light_misses() {
		for level in 0..19 {
			let painted = sky_sun(level);
			let key = RETAIL_KEY.iter().find(|(l, _)| *l == level).map(|(_, k)| *k);
			assert_eq!(painted.is_some(), key.is_some(), "level {level}");
			let far = painted.zip(key).is_some_and(|(p, k)| angle_deg(p, k) > SKY_KEY_MIN_DEG);
			assert_eq!(SKY_KEY.contains(&level), far, "level {level}");
		}
		let deg = |l: usize| angle_deg(sky_sun(l).unwrap(), RETAIL_KEY.iter().find(|(x, _)| *x == l).unwrap().1);
		let want = [64.6, 18.9, 3.2, 110.6, 40.7, 64.0, 28.5];
		for (l, w) in [0, 1, 3, 4, 11, 13, 16].into_iter().zip(want) {
			assert!((deg(l) - w).abs() < 0.1, "level {l}: {} degrees", deg(l));
		}
	}

	/// The shading follows on every such level but Veldin (approved retail shading).
	#[test]
	fn sky_key_lit_is_sky_key_but_veldin() {
		assert!(SKY_KEY_LIT.iter().all(|l| SKY_KEY.contains(l)));
		assert_eq!(SKY_KEY.iter().filter(|l| !SKY_KEY_LIT.contains(l)).collect::<Vec<_>>(), [&0]);
		assert!((LOOKS.sky_key)(11) && (LOOKS.sky_key_lit)(11) && !(LOOKS.sky_key)(3) && !(LOOKS.sky_key_lit)(0));
	}

	/// The approved water keeps retail's key light in its sky map: only the two levels that followed the painted sun before lane
	/// nsun (Veldin, Gemlik) turn it; Pokitaru and Novalis (approved water 2026-10-15), Eudora and Kalebo III do not.
	#[test]
	fn the_water_sky_map_turns_only_where_it_did_before() {
		assert!(SKY_KEY_WATER.iter().all(|l| SKY_KEY.contains(l)));
		assert_eq!(SKY_KEY_WATER, [0, 13]);
		for l in [1, 4, 11, 16] {
			assert!((LOOKS.sky_key)(l) && !(LOOKS.sky_key_water)(l), "level {l}");
		}
		assert!((LOOKS.sky_key_water)(0) && (LOOKS.sky_key_water)(13) && !(crate::looks::NONE.sky_key_water)(0));
	}

	/// The water's bed and mirror passes keep retail's key light on the levels whose shading turned but whose water did not
	/// (review R26 M1: Pokitaru's pool floor took Metal highlights from the painted sun); Veldin, Gemlik and the rest unchanged.
	#[test]
	fn the_water_bed_and_mirror_keep_the_key_light_where_the_water_does() {
		let keeps: Vec<usize> = (0..19).filter(|&l| LOOKS.water_keeps_key(l)).collect();
		assert_eq!(keeps, [1, 4, 11, 16]);
		assert!((0..19).all(|l| !crate::looks::NONE.water_keeps_key(l)));
	}

	#[test]
	fn the_pool_stays_low_on_veldin_and_eudora() {
		assert_eq!((pool_cap(0), pool_cap(4)), (12.0, 20.0));
		assert!(pool_cap(3).is_infinite());
	}

	#[test]
	fn only_umbris_is_swamp() {
		let is_swamp = LOOKS.swamp;
		assert!(is_swamp(7));
		assert!(!is_swamp(1) && !is_swamp(4) && !is_swamp(11));
	}
}
