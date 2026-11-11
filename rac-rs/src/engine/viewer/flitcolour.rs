//! [PC] Fire light v2 (lane dlit3, 2026-11-06; user: "we need a realistic fire lighting"): the colour of a fire light = a
//! blackbody at the flame's temperature. `rgb(t)`: linear sRGB (D65) of a blackbody at `t` K, scaled to luminance 1 (Rec.709 Y),
//! so a light's intensity is its luminance and its colour carries no extra energy. Table every 100 K from 1000 K to 4000 K
//! (Planck x the CIE 1931 2-degree colour matching functions in the Wyman / Sloan / Shirley 2013 multi-lobe fit, XYZ -> linear
//! sRGB, negatives clipped to 0), linear in between, held at the ends. `temperature`: a flame that dims is cooler (radiance ~ T^4):
//! T = T0 x flick^(1/4) (flitlist.rs `fire_lights`; design FIRE_LIGHT_V2.md 2.4).

/// The table's first temperature and step (K).
const T_LO: f32 = 1000.0;
const T_STEP: f32 = 100.0;

/// Linear sRGB at luminance 1 of a blackbody at 1000, 1100, ... 4000 K.
const TABLE: [[f32; 3]; 31] = [
	[4.312, 0.126, 0.000], // 1000 K
	[4.044, 0.206, 0.000], // 1100 K
	[3.794, 0.280, 0.000], // 1200 K
	[3.566, 0.347, 0.000], // 1300 K
	[3.358, 0.408, 0.000], // 1400 K
	[3.171, 0.462, 0.000], // 1500 K
	[3.002, 0.511, 0.000], // 1600 K
	[2.850, 0.555, 0.000], // 1700 K
	[2.712, 0.594, 0.000], // 1800 K
	[2.588, 0.629, 0.000], // 1900 K
	[2.474, 0.661, 0.019], // 2000 K
	[2.371, 0.689, 0.041], // 2100 K
	[2.277, 0.715, 0.064], // 2200 K
	[2.192, 0.738, 0.088], // 2300 K
	[2.113, 0.759, 0.113], // 2400 K
	[2.040, 0.778, 0.138], // 2500 K
	[1.973, 0.795, 0.164], // 2600 K
	[1.912, 0.811, 0.191], // 2700 K
	[1.855, 0.825, 0.217], // 2800 K
	[1.801, 0.838, 0.244], // 2900 K
	[1.752, 0.850, 0.271], // 3000 K
	[1.706, 0.861, 0.298], // 3100 K
	[1.663, 0.871, 0.325], // 3200 K
	[1.623, 0.880, 0.351], // 3300 K
	[1.586, 0.889, 0.378], // 3400 K
	[1.551, 0.896, 0.404], // 3500 K
	[1.518, 0.904, 0.431], // 3600 K
	[1.487, 0.910, 0.456], // 3700 K
	[1.457, 0.916, 0.482], // 3800 K
	[1.430, 0.922, 0.507], // 3900 K
	[1.404, 0.927, 0.532], // 4000 K
];

/// Rec.709 luminance of linear sRGB.
pub(super) fn luma(c: [f32; 3]) -> f32 {
	0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
}

/// Linear sRGB of a blackbody at `t` K with luminance 1 (`TABLE`, linear in between, held at 1000 / 4000 K).
pub(super) fn rgb(t: f32) -> [f32; 3] {
	let x = ((t - T_LO) / T_STEP).clamp(0.0, (TABLE.len() - 1) as f32);
	let i = (x.floor() as usize).min(TABLE.len() - 2);
	let f = x - i as f32;
	let (a, b) = (TABLE[i], TABLE[i + 1]);
	[a[0] + (b[0] - a[0]) * f, a[1] + (b[1] - a[1]) * f, a[2] + (b[2] - a[2]) * f]
}

/// A flame's temperature at flicker / fade `flick` (its light's share of full strength): T0 x flick^(1/4), at least the table's
/// first entry.
pub(super) fn temperature(t0: f32, flick: f32) -> f32 {
	(t0 * flick.max(0.0).powf(0.25)).max(T_LO)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn every_entry_has_luminance_one_and_reddens_as_it_cools() {
		for (k, c) in TABLE.iter().enumerate() {
			assert!((luma(*c) - 1.0).abs() < 0.01, "{} K: {c:?} Y {}", 1000 + 100 * k, luma(*c));
		}
		for w in TABLE.windows(2) {
			assert!(w[1][0] < w[0][0] && w[1][1] > w[0][1] && w[1][2] >= w[0][2], "hotter = less red, more green / blue");
		}
	}

	/// Spot values of the design's table (FIRE_LIGHT_V2.md 2.4, scratchpad bb.py of lane dlit2).
	#[test]
	fn the_table_matches_the_design() {
		let near = |a: [f32; 3], b: [f32; 3]| (0..3).all(|k| (a[k] - b[k]).abs() < 0.02);
		assert!(near(rgb(1900.0), [2.590, 0.628, 0.0]));
		assert!(near(rgb(2400.0), [2.115, 0.758, 0.113]));
		assert!(near(rgb(3000.0), [1.754, 0.849, 0.271]));
		assert!(near(rgb(1950.0), [0.5 * (2.588 + 2.474), 0.5 * (0.629 + 0.661), 0.5 * 0.019]), "linear in between");
		assert_eq!((rgb(500.0), rgb(9000.0)), (TABLE[0], TABLE[30]), "held at the ends");
	}

	#[test]
	fn a_dimmer_flame_is_cooler() {
		assert_eq!(temperature(1900.0, 1.0), 1900.0);
		assert!((temperature(1900.0, 0.5) - 1900.0 * 0.5f32.powf(0.25)).abs() < 1e-3);
		assert_eq!(temperature(1900.0, 0.0), T_LO);
	}
}
