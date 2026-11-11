//! R&C1's cloud sky per planet (engine/looks.rs `Looks`, read by engine/viewer/volclouds/params.rs; engine split stage E2
//! batch 5, 2026-10-26: moved out of it): the cloud look of each planet, chosen after its painted sky, the retail sky shell
//! draws the new sky replaces and where it draws its own sun disc.

use crate::looks::CloudLook as Look;

/// Veldin 2 (level 18): true = candidate A, the retail painted sky with no volumetric clouds; false = candidate B (the retuned deck).
pub(super) const VELDIN2_PAINTED: bool = true;

/// The cloud look of level `level` (the loaded level index: veldin novalis aridia kerwan eudora rilgar nebula_g34 umbris batalia
/// gaspar orxon pokitaru hoven gemlik (in space) oltanis quartu kalebo fleet veldin2), chosen after each planet's painted sky; None
/// for levels without a sky to speak of (space, the dark interiors): those keep the retail sky.
pub(super) fn look(level: usize) -> Option<Look> {
	// The default: bright fair weather led by the cumulus, big banks of heaps in rows with wide gaps of blue, medium and small
	// heaps round them, a few congestus; a faint altocumulus band and cirrus veil.
	let fair = Look {
		cu: 0.6,
		sc: 0.08,
		ac: 0.1,
		ci: 0.15,
		cc: 0.0,
		tower: 0.45,
		base: 1400.0,
		top: 5200.0,
		mid: 6400.0,
		high: 9500.0,
		density: 1.0,
		haze: 26000.0,
		wind: 40.0,
		heading: 0.6,
		sun: 2.0,
		hue: 0.35,
		dusk: 0.0,
		dusk_amb: 4.0,
		amb: 0.9,
		size: 1.5,
		farcov: 0.18,
		pale: 0.0,
		expo: 1.0,
		soft: 0.0,
		tint: None,
		form: 0.0,
		rim: 1.0,
		lining: 1.0,
		under: 1.0,
		feather: 0.0,
	};
	match level {
		// Aridia keeps its retail swirling night sky whole (user 2026-10-25: no clouds there).
		2 | 6 | 13 | 15 | 17 => None,
		// Veldin at dusk: a broken stratocumulus deck and altocumulus bands, dark blue-grey
		// bodies with gold fringes from the low sun, few heaps.
		0 => Some(Look { cu: 0.3, sc: 0.5, ac: 0.15, ci: 0.12, cc: 0.0, tower: 0.15, base: 1900.0, top: 3800.0, density: 3.1, haze: 24000.0, wind: 30.0, heading: 2.2, sun: 2.5, hue: 1.0, dusk: 1.0, ..fair }),
		// Veldin 2 under its dark red sky (user 2026-11-05: the bright pink cotton-ball heaps looked out of place): candidate B, a low,
		// murky, soft broken deck in the sky's own red-brown over the retail dome, like the painted streaks (dark red-brown on the
		// orange glow): dim bodies lit only by the sky, feathered edges (no holes), no silver lining or lit rims, darker undersides.
		// `VELDIN2_PAINTED` = true gives candidate A, the retail painted sky whole (no volumetric clouds on this level).
		18 if VELDIN2_PAINTED => None,
		18 => Some(Look { cu: 0.1, sc: 0.5, ac: 0.1, ci: 0.0, cc: 0.0, tower: 0.0, base: 2400.0, top: 4000.0, density: 0.8, haze: 16000.0, wind: 25.0, heading: 2.2, sun: 0.15, hue: 1.0, dusk: 0.8, dusk_amb: 0.8, amb: 0.25, size: 1.3, farcov: 0.05, expo: 0.4, tint: Some([0.9, 0.4, 0.25]), rim: 0.1, lining: 0.0, under: 0.4, feather: 0.9, ..fair }),
		// Kerwan: hazy, flatter heaps over a thin deck, altocumulus.
		3 => Some(Look { cu: 0.62, sc: 0.22, ac: 0.12, ci: 0.12, cc: 0.0, tower: 0.3, haze: 24000.0, ..fair }),
		// Eudora: fuller cumulus, some congestus.
		4 => Some(Look { cu: 0.6, sc: 0.12, ac: 0.08, ci: 0.12, cc: 0.05, tower: 0.7, top: 6000.0, mid: 7200.0, high: 10500.0, ..fair }),
		// Pokitaru: tropical heaps building into towers, cirrus high above.
		11 => Some(Look { cu: 0.6, sc: 0.05, ac: 0.06, ci: 0.2, cc: 0.05, tower: 0.85, base: 1300.0, top: 6200.0, mid: 7400.0, high: 10500.0, ..fair }),
		// Rilgar at night: the retail starry sky stays whole (`sky_mask`); a thin, soft, murky night overcast over it in the sky's own
		// dark teal (the Orxon recipe: a broken stratocumulus deck, feathered edges, no silver lining or rims, dim bodies lit only
		// by the sky), so the nebula and the stars show through and the clouds belong to the sky instead of sitting on it (user
		// 2026-10-30: the crisp grey heaps looked pasted on). Tuned with RAC_TUNE on the combined exe; the tint is unchanged.
		5 => Some(Look { cu: 0.15, sc: 0.45, ac: 0.0, ci: 0.0, cc: 0.0, tower: 0.0, base: 1800.0, top: 3600.0, density: 0.0825, haze: 20000.0, sun: 0.1, hue: 0.0, dusk: 0.4, dusk_amb: 2.0, amb: 0.2, farcov: 0.04, expo: 0.28, tint: Some([0.55, 0.78, 0.85]), rim: 0.0, lining: 0.0, under: 0.3, feather: 1.0, ..fair }),
		// Umbris at night: the retail painted overcast (dark olive-brown, stars between its breaks, the blue planet, the orange horizon
		// glow) stays whole (`sky_mask`); a low, murky, low-contrast deck of the same olive-brown over it, lit only by the dim glow
		// of the horizon, no bright edges.
		7 => Some(Look { cu: 0.2, sc: 0.48, ac: 0.25, ci: 0.0, cc: 0.0, tower: 0.0, base: 1700.0, top: 3300.0, density: 0.625, haze: 12000.0, wind: 25.0, heading: 1.0, sun: 0.08, hue: 0.6, dusk: 0.8, dusk_amb: 0.3, amb: 0.15, size: 1.4, farcov: 0.1, expo: 0.35, tint: Some([0.85, 0.5, 0.28]), rim: 0.375, lining: 0.0, under: 0.5, feather: 0.6, ..fair }),
		// Batalia: a heavy dark storm deck under the rain with towering storm clouds (the cumulonimbus form) rising out of it, a
		// cool blue-grey light, strong dark undersides and softly lit tops (no silver lining or rims); the painted storm band stays.
		8 => Some(Look { cu: 0.5, sc: 0.85, ac: 0.0, ci: 0.0, cc: 0.0, tower: 0.3, base: 900.0, top: 3600.0, mid: 6400.0, density: 3.0, haze: 13000.0, wind: 70.0, heading: 1.1, sun: 0.4, hue: 0.0, dusk: 0.3, dusk_amb: 5.0, amb: 0.3, size: 1.2, farcov: 0.15, expo: 0.35, tint: Some([0.62, 0.72, 0.9]), form: 1.0, rim: 0.0, lining: 0.0, under: 0.3, feather: 0.6, ..fair }),
		// Gaspar at sunset: the retail sky stays whole (the painted streaks, the planet, the stars); dark-brown cloud banks (small to
		// big heaps, a stratocumulus deck and altocumulus rows) over it, their bodies dark and only the fringes lit by the low orange sun.
		9 => Some(Look { cu: 0.02, sc: 0.45, ac: 0.15, ci: 0.0, cc: 0.0, tower: 0.0, base: 1800.0, top: 3600.0, density: 1.0, haze: 20000.0, wind: 30.0, sun: 0.7, hue: 1.0, dusk: 1.0, dusk_amb: 0.15, amb: 0.3, size: 0.86, farcov: 0.0, expo: 0.7, rim: 0.2, under: 0.5, feather: 0.9, ..fair }),
		// Orxon: the retail painted toxic sky stays whole (`sky_mask`); a broken, soft, low murky overcast over it in the sky's
		// green, darker undersides, no lit rims.
		10 => Some(Look { cu: 0.32, sc: 0.32, ac: 0.0, ci: 0.0, cc: 0.0, tower: 0.2, base: 850.0, top: 3000.0, density: 0.9, haze: 11000.0, wind: 50.0, heading: 2.6, sun: 0.2, hue: 0.3, dusk: 0.4, dusk_amb: 2.0, amb: 0.3, size: 1.2, farcov: 0.1, expo: 0.5, tint: Some([0.75, 0.85, 0.55]), rim: 0.0, lining: 0.3, under: 0.4, feather: 0.8, ..fair }),
		// Hoven and Oltanis: stormy, dark as the retail storm sky (2026-11-06, vs PCSX2 14_0692): cumulonimbus towers (each leaning its own way, of its own height and width, cauliflower
		// crowns) over a dark deck, dark bodies with lit fringes.
		12 | 14 => Some(Look { cu: 0.62, sc: 0.85, ac: 0.15, ci: 0.08, cc: 0.0, tower: 0.6, base: 1600.0, top: 6400.0, mid: 7400.0, high: 10500.0, density: 2.5, haze: 20000.0, sun: 0.25, hue: 0.0, dusk: 0.6, dusk_amb: 8.0, amb: 0.3, expo: 0.3, form: 1.0, under: 0.6, feather: 0.6, ..fair }),
		// Kalebo III: bright heaps building into big towers, cirrocumulus patches; thinner, lighter bodies against the bright haze.
		16 => Some(Look { cu: 0.75, sc: 0.1, ac: 0.08, ci: 0.15, cc: 0.15, tower: 0.8, density: 0.85, sun: 2.6, amb: 1.2, ..fair }),
		// Novalis: after the planet's 2016 look, a near-continuous broken deck of big billowy banks with blue gaps high up, smaller
		// flatter clouds and haze low over the mountains; soft grey-white bodies with large blue-grey shaded parts under a paler,
		// hazier sky.
		1 => Some(Look { cu: 0.75, tower: 0.2, farcov: 0.3, sun: 1.4, amb: 0.8, expo: 0.9, soft: 0.05, pale: 0.2, ..fair }),
		_ => Some(fair),
	}
}

/// The retail sky shell draws (indices into the sky batch's draws) the new sky replaces on `level`: the painted gradient and the
/// painted clouds. Draws that hold landmarks players know (moons, planets in the sky) stay.
pub(super) fn sky_mask(level: usize) -> u64 {
	match level {
		// (From RAC_SKY_DUMP of each level.) Veldin: the gradient dome (0) and the painted cloud band (5); the stars, the two
		// planets and the city skyline stay.
		0 => 0x21,
		// Veldin 2: the dome with its painted clouds; stars, moons and planets, the horizon bands and skylines stay.
		18 => 0x1,
		// Eudora: the dome and the two cloud bands; the mountain silhouettes (draws 3..) stay.
		4 => 0x7,
		// Gaspar: nothing (its painted streaks, planet and stars are its charm; the thin bands go over them).
		9 => 0,
		// Pokitaru, Hoven: the dome and three cloud bands; the far islands / horizon line (4) stay.
		11 | 12 => 0xF,
		// Oltanis: the dome and the green storm clouds; the glowing horizon band stays.
		14 => 0x3,
		// Kalebo III: the dome and two cloud bands; the horizon haze (1) and the far structures (4..) stay.
		16 => 0xD,
		// Rilgar: nothing; its painted dome is the starry nebula night sky, which stays whole under the night clouds.
		5 => 0,
		// Batalia: the dark dome; the painted storm band (1) and the horizon silhouettes (2) stay under the storm clouds.
		8 => 0x1,
		// Orxon: nothing (the painted toxic sky stays whole; the soft broken overcast goes over it).
		10 => 0,
		// Umbris: nothing (RAC_SKY_DUMP: draw 0 the olive dome with the stars, 1 and 3 the ships, 2 and 4 the two planets, 5 the dark
		// horizon band over the orange glow; all stay under the murky deck).
		7 => 0,
		// Novalis, Kerwan: every draw is the dome or painted clouds.
		_ => u64::MAX,
	}
}

/// True when the new sky draws its own sun disc on `level` (the painted sun went with the shells the mask drops).
pub(super) fn sun_disc(level: usize) -> bool {
	matches!(level, 0 | 1 | 3 | 4 | 11 | 16 | 18)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn closed_levels_keep_the_retail_sky() {
		for l in [2, 6, 13, 15, 17] {
			assert!(look(l).is_none(), "level {l}");
		}
		assert_eq!(look(18).is_none(), VELDIN2_PAINTED);
		for l in [0, 1, 3, 4, 5, 7, 8, 9, 10, 11, 12, 14, 16] {
			let k = look(l).unwrap_or_else(|| panic!("level {l}"));
			assert!(k.top > k.base && k.base > 0.0 && [k.cu, k.sc, k.ac, k.ci, k.cc, k.tower].iter().all(|c| (0.0..=1.0).contains(c)) && k.mid > k.top && k.high > k.mid, "level {l}");
		}
	}
}
