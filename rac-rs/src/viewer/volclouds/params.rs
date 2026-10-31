//! The cloud sky's per-level numbers and the pure helpers around them: which levels get the new sky, the altitudes and the mix of
//! cloud genera of each planet, the quality levels, the retail sky shell draws the new sky replaces, and the sun the sky and the
//! clouds are lit from.

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

/// How a planet's sky takes clouds: the mix of the four genera (`genera.rs`) and their altitudes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Look {
	/// Coverage (0..1) of each genus: cumulus (single heaps of many sizes, from small puffs to towers), stratocumulus (a broad deck
	/// of soft rolls low in the shell), altocumulus (rippled wave bands higher up), cirrus (thin streaks) and cirrocumulus (small
	/// cells in patches) in the veil at the top.
	pub cu: f32,
	pub sc: f32,
	pub ac: f32,
	pub ci: f32,
	pub cc: f32,
	/// Vertical development of the cumulus: 0 flat fair-weather heaps, 0.5 congestus, 1 cumulonimbus towers to the shell's top.
	pub tower: f32,
	/// Cloud shell base and top (the tallest towers), the altocumulus band and the cirrus veil above the level's reference height
	/// (world units).
	pub base: f32,
	pub top: f32,
	pub mid: f32,
	pub high: f32,
	/// Extinction scale (thicker, darker clouds above 1) and the distance of the aerial haze (world units).
	pub density: f32,
	pub haze: f32,
	/// Wind speed (world units per second) and heading (radians).
	pub wind: f32,
	pub heading: f32,
	/// Sun gain and how far the sun's colour leans to the painted sun's hue (0 the key light's colour, 1 the painted hue).
	pub sun: f32,
	pub hue: f32,
	/// 0 day; 1 dusk: dark bodies lit by the sky away from the low sun, the thin fringes catching it.
	pub dusk: f32,
	/// Gain of the dusk ambient (the sky away from the sun lights the bodies).
	pub dusk_amb: f32,
	/// Gain of the sky ambient on the cloud bodies (brighter bodies against a bright hazy sky above 0.9).
	pub amb: f32,
	/// Scale of the cloud fields and shapes (1 = weather tile 14000, shape tile 3000 world units).
	pub size: f32,
	/// Extra cumulus coverage of the far band low in the sky (on top of `cu`).
	pub farcov: f32,
	/// Grade of the sky model (0 the painted sky's colours; 1 paler, softer, hazier toward the horizon).
	pub pale: f32,
	/// Exposure of the cloud light, and extra soft multi-scattered light in the bodies (softer, lower-contrast shading).
	pub expo: f32,
	pub soft: f32,
	/// The main light's colour in place of the level key light's (night: a cool moonlight; storm: a flat grey), or None.
	pub tint: Option<[f32; 3]>,
	/// Cumulonimbus form (1 on the storm levels: leaning towers of their own height and width with cauliflower crowns, all on the
	/// shell's base; 0 the fair-weather towers of the heaps), the fringe light's gain (times `dusk`), the silver lining (1 the full
	/// forward glow toward the sun, 0 none: an overcast), the undersides' light (below 1 darker) and how soft the cloud surfaces are
	/// (0 crisp painted masses; above 0 diffuse, no hard edges).
	pub form: f32,
	pub rim: f32,
	pub lining: f32,
	pub under: f32,
	pub feather: f32,
}

/// The cloud look of level `level` (the loaded level index: veldin novalis aridia kerwan eudora rilgar nebula_g34 umbris batalia
/// gaspar orxon pokitaru hoven gemlik (in space) oltanis quartu kalebo fleet veldin2), chosen after each planet's painted sky; None
/// for levels without a sky to speak of (space, the dark interiors): those keep the retail sky.
pub fn look(level: usize) -> Option<Look> {
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
		// Veldin 2 under its dark red sky: dimmer, many small scattered heaps high up, no deck.
			18 => Some(Look { cu: 0.3, sc: 0.0, ac: 0.04, ci: 0.06, cc: 0.08, tower: 0.1, base: 2400.0, top: 4300.0, density: 2.0, haze: 24000.0, wind: 30.0, heading: 2.2, sun: 1.4, hue: 1.0, dusk: 1.0, size: 0.8, ..fair }),
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
		8 => Some(Look { cu: 0.5, sc: 0.55, ac: 0.0, ci: 0.0, cc: 0.0, tower: 0.7, base: 900.0, top: 3600.0, mid: 6400.0, density: 3.0, haze: 13000.0, wind: 70.0, heading: 1.1, sun: 0.85, hue: 0.0, dusk: 0.3, dusk_amb: 5.0, amb: 0.45, size: 1.2, farcov: 0.15, expo: 0.6, tint: Some([0.62, 0.72, 0.9]), form: 1.0, rim: 0.0, lining: 0.0, under: 0.3, feather: 0.35, ..fair }),
		// Gaspar at sunset: the retail sky stays whole (the painted streaks, the planet, the stars); dark-brown cloud banks (small to
		// big heaps, a stratocumulus deck and altocumulus rows) over it, their bodies dark and only the fringes lit by the low orange sun.
		9 => Some(Look { cu: 0.37, sc: 0.45, ac: 0.3, ci: 0.0, cc: 0.0, tower: 0.2, base: 1800.0, top: 3600.0, density: 3.75, haze: 20000.0, wind: 30.0, sun: 1.9, hue: 0.65, dusk: 1.0, dusk_amb: 0.15, amb: 0.1, size: 0.86, farcov: 0.0, expo: 0.8, rim: 3.0, under: 0.5, ..fair }),
		// Orxon: the retail painted toxic sky stays whole (`sky_mask`); a broken, soft, low murky overcast over it in the sky's
		// green, darker undersides, no lit rims.
		10 => Some(Look { cu: 0.32, sc: 0.32, ac: 0.0, ci: 0.0, cc: 0.0, tower: 0.2, base: 850.0, top: 3000.0, density: 0.9, haze: 11000.0, wind: 50.0, heading: 2.6, sun: 0.2, hue: 0.3, dusk: 0.4, dusk_amb: 2.0, amb: 0.3, size: 1.2, farcov: 0.1, expo: 0.5, tint: Some([0.75, 0.85, 0.55]), rim: 0.0, lining: 0.3, under: 0.4, feather: 0.8, ..fair }),
		// Hoven and Oltanis: stormy; cumulonimbus towers (each leaning its own way, of its own height and width, cauliflower
		// crowns) over a dark deck, dark bodies with lit fringes.
		12 | 14 => Some(Look { cu: 0.62, sc: 0.5, ac: 0.15, ci: 0.08, cc: 0.0, tower: 1.0, base: 1600.0, top: 6400.0, mid: 7400.0, high: 10500.0, density: 2.5, haze: 20000.0, sun: 0.7, hue: 0.0, dusk: 0.6, dusk_amb: 8.0, form: 1.0, under: 0.6, feather: 0.08, ..fair }),
		// Kalebo III: bright heaps building into big towers, cirrocumulus patches; thinner, lighter bodies against the bright haze.
		16 => Some(Look { cu: 0.75, sc: 0.1, ac: 0.08, ci: 0.15, cc: 0.15, tower: 0.8, density: 0.85, sun: 2.6, amb: 1.2, ..fair }),
		// Novalis: after the planet's 2016 look, a near-continuous broken deck of big billowy banks with blue gaps high up, smaller
		// flatter clouds and haze low over the mountains; soft grey-white bodies with large blue-grey shaded parts under a paler,
		// hazier sky.
		1 => Some(Look { cu: 0.75, tower: 0.2, farcov: 0.3, sun: 1.4, amb: 0.8, expo: 0.9, soft: 0.05, pale: 1.0, ..fair }),
		_ => Some(fair),
	}
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

/// The retail sky shell draws (indices into the sky batch's draws) the new sky replaces on `level`: the painted gradient and the
/// painted clouds. Draws that hold landmarks players know (moons, planets in the sky) stay.
pub fn sky_mask(level: usize) -> u64 {
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
pub fn sun_disc(level: usize) -> bool {
	matches!(level, 0 | 1 | 3 | 4 | 11 | 16 | 18)
}
