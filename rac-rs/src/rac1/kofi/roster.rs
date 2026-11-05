//! Ko-fi drawings: the list of supporters. ONE table: a name, the function that paints the sheet and where it hangs. To add a
//! supporter: write the scene function (like `scenes::scrolt`, or a file of its own like `achieve.rs`), add one `Entry` below, and
//! pick a wall spot (prove it with a shot: `RAC_TUNE` `kofiN_x/_y/_z/_facing/_w/_tilt` moves sheet N without a rebuild). The sheets are
//! painted in this order; the painter's seed is the entry's index, so new entries go LAST (the older sheets keep their pixels).

use super::hand::Hand;
use super::{achieve, desynkro, fans_a, fans_b, scenes};
use super::Placement;

pub struct Entry {
	/// The name exactly as it is spelled on the sheet.
	pub name: &'static str,
	/// Paints the scene on the canvas; returns every text written on it.
	pub paint: fn(&mut Hand) -> Vec<String>,
	/// Where the sheet hangs in the garage (see `Placement`).
	pub place: Placement,
}

pub static ROSTER: &[Entry] = &[
	Entry { name: "Scrolt", paint: scenes::scrolt, place: Placement { c: [140.78, 100.7, 35.0], facing: -0.25, w: 1.2, tilt: 0.22 } },
	Entry { name: "korixx", paint: scenes::korixx, place: Placement { c: [148.0, 87.0, 36.0], facing: std::f32::consts::FRAC_PI_2, w: 1.0, tilt: -0.2 } },
	// reTokyo: on the poster (+x) wall near the back corner, beside fleece (it floated by the crates under the Qwark poster, and
	// the poster itself stands in front of the wall there and would hide it).
	Entry { name: "reTokyo", paint: scenes::retokyo, place: Placement { c: [153.30, 90.00, 34.25], facing: 2.84, w: 0.90, tilt: 0.17 } },
	// khalifax10 (the 400 euro supporter, who suggested the achievements): a bigger sheet. The places are spread on purpose, uneven and crooked (user 2026-11-04: "messy, not tidy"): Scrolt and khalifax10 on the -x wall (Scrolt overlaps the corner), korixx on the back (-y) wall, reTokyo below the Qwark poster on the +x wall.
	Entry { name: "khalifax10", paint: achieve::khalifax10, place: Placement { c: [140.4, 99.6, 34.0], facing: -0.25, w: 1.5, tilt: -0.07 } },
	// DeSynkro (helped so much with the Linux support: a penguin, user 2026-11-04): on the back (-y) wall below korixx, crooked the other way (the wall's light panels stand out in front of y = 87 toward the middle, so not further right).
	Entry { name: "DeSynkro", paint: desynkro::desynkro, place: Placement { c: [147.45, 87.0, 34.5], facing: std::f32::consts::FRAC_PI_2, w: 1.15, tilt: 0.13 } },
	// The Ko-fi list of 2026-11-04 in the order of the page (newest first); later supporters go after these. Spots from
	// `RAC_TUNE kofi_map=1` (flat wall spots), spread over all four walls; facing: -x wall -0.26, back wall 1.31, poster (+x)
	// wall 2.84, front wall -1.84 (the garage is turned 15 degrees); kofi/walls.rs puts each sheet onto the wall itself.
	Entry { name: "DarkBlaze", paint: fans_a::darkblaze, place: Placement { c: [154.14, 93.90, 34.40], facing: 2.84, w: 0.95, tilt: -0.14 } },
	Entry { name: "Smoe05", paint: fans_a::smoe05, place: Placement { c: [138.50, 94.30, 34.10], facing: -0.26, w: 0.90, tilt: 0.11 } },
	Entry { name: "eardrummer", paint: fans_a::eardrummer, place: Placement { c: [141.30, 88.51, 34.70], facing: 1.31, w: 0.80, tilt: 0.19 } },
	Entry { name: "Chris", paint: fans_a::chris, place: Placement { c: [143.60, 102.57, 33.80], facing: -1.84, w: 0.80, tilt: -0.08 } },
	Entry { name: "Ryan Palmer", paint: fans_a::ryan_palmer, place: Placement { c: [144.60, 87.62, 37.00], facing: 1.31, w: 1.05, tilt: 0.06 } },
	Entry { name: "fleece", paint: fans_a::fleece, place: Placement { c: [153.33, 91.10, 34.00], facing: 2.84, w: 0.80, tilt: -0.21 } },
	Entry { name: "Justintime4u2bu", paint: fans_a::justintime4u2bu, place: Placement { c: [145.60, 87.36, 35.40], facing: 1.31, w: 1.15, tilt: -0.10 } },
	Entry { name: "Ga", paint: fans_b::ga, place: Placement { c: [153.20, 99.99, 33.70], facing: -1.84, w: 0.75, tilt: 0.16 } },
	Entry { name: "THUND3R_WOLF", paint: fans_b::thund3r_wolf, place: Placement { c: [149.30, 86.36, 35.20], facing: 1.31, w: 0.85, tilt: -0.17 } },
	Entry { name: "MK", paint: fans_b::mk, place: Placement { c: [139.47, 97.90, 33.80], facing: -0.26, w: 0.90, tilt: 0.20 } },
	Entry { name: "Ratchetenjoyer", paint: fans_b::ratchetenjoyer, place: Placement { c: [146.30, 87.17, 37.40], facing: 1.31, w: 0.80, tilt: -0.05 } },
	Entry { name: "Tonyswan", paint: fans_b::tonyswan, place: Placement { c: [142.30, 88.24, 36.10], facing: 1.31, w: 0.95, tilt: 0.13 } },
	Entry { name: "Luke4600", paint: fans_b::luke4600, place: Placement { c: [144.20, 101.42, 37.40], facing: -1.84, w: 0.80, tilt: -0.12 } },
];
