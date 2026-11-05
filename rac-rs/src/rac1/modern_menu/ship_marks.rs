//! The Galactic Map's planets on our own galaxy painting (v0.5 screens stage F, DECISIONS.md items 9 and 10). The painting
//! (`assets/galaxy/galactic_map.png`, 1280 x 720, made with Codex for this port, not extracted from the disc) is embedded in the exe.
//! A planet is keyed by retail's planet id: the list's ids are the level numbers 1..=18 (Novalis .. Veldin's finale; id 0, Veldin's
//! tutorial, ends the order array and never shows in the list), the same on both discs. The row's id is its location text id minus
//! `LOCATION_BASE` (`level_transition.md`: location `0x4eb9 + id`). Positions are fractions of the painting. They were fitted, not copied:
//! retail's own table (`0x1c2338 + 16 (i-1)`, x / y in its 307 x 197 picture) was scaled into the free gap between the list and the
//! detail panel (x 500..830, y 190..530 of 1280 x 720, so every marker and its ring clear both panels), and each marker was then moved
//! to the brightest knot of the painting within 40 px of that place, 50 px or more from the others and the core. Retail's relative
//! layout (which planet is left of which) is kept.

/// The painting, PNG.
pub const ART_PNG: &[u8] = include_bytes!("../../../assets/galaxy/galactic_map.png");
/// Its size in pixels (the design space is 1280 x 720: one painting pixel per design unit).
pub const ART_SIZE: (u32, u32) = (1280, 720);

/// A list row's location text id is `LOCATION_BASE + planet id`.
pub const LOCATION_BASE: u32 = 0x4eb9;
/// A planet's NAME text id is `NAME_BASE + id` for the ids 1..=17; Veldin (ids 0 and 18) has `NAME_VELDIN` (the map header's table
/// 0x1c1e40, `ingame_other_screens.md`): the same on both discs, one text per language.
pub const NAME_BASE: u32 = 0x4ecc;
pub const NAME_VELDIN: u32 = 0x4ede;
/// The planet ids the list can show.
pub const FIRST: u32 = 1;
pub const LAST: u32 = 18;

/// (planet id, x, y), fractions of the painting.
pub const MARKS: [(u32, f32, f32); 18] = [
	(1, 0.441, 0.497),
	(2, 0.413, 0.706),
	(3, 0.464, 0.404),
	(4, 0.481, 0.511),
	(5, 0.519, 0.533),
	(6, 0.503, 0.407),
	(7, 0.484, 0.681),
	(8, 0.554, 0.625),
	(9, 0.561, 0.508),
	(10, 0.595, 0.651),
	(11, 0.633, 0.518),
	(12, 0.578, 0.438),
	(13, 0.582, 0.239),
	(14, 0.558, 0.331),
	(15, 0.523, 0.281),
	(16, 0.463, 0.281),
	(17, 0.404, 0.533),
	(18, 0.416, 0.443),
];

/// The marker of planet `id` as fractions of the painting; None for an id the list never shows.
pub fn mark(id: u32) -> Option<(f32, f32)> {
	MARKS.iter().find(|m| m.0 == id).map(|m| (m.1, m.2))
}

/// The game's own name text of the planet / level `id` (None = not a level of the game).
pub fn name_text(id: u32) -> Option<u32> {
	match id {
		0 | 18 => Some(NAME_VELDIN),
		1..=17 => Some(NAME_BASE + id),
		_ => None,
	}
}

/// The planet of a list row from its location text id (None = not a planet row).
pub fn planet_of(location_text: u32) -> Option<u32> {
	let id = location_text.checked_sub(LOCATION_BASE)?;
	(FIRST..=LAST).contains(&id).then_some(id)
}

/// Gold bolts to find per planet id (retail's table at overlay 0x1c4908, decomp/notes/level_transition.md; 40 in the game).
const GOLD_TOTALS: [u32; 19] = [0, 3, 4, 3, 1, 2, 2, 2, 2, 2, 2, 1, 2, 1, 4, 2, 2, 2, 3];

/// The gold bolts planet `id` holds; 0 for an id the table does not know.
pub fn gold_total(id: u32) -> u32 {
	GOLD_TOTALS.get(id as usize).copied().unwrap_or(0)
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_game_holds_forty_gold_bolts_and_every_listed_planet_has_some() {
		assert_eq!((FIRST..=LAST).map(gold_total).sum::<u32>(), 40);
		assert!((FIRST..=LAST).all(|p| (1..=4).contains(&gold_total(p))));
		assert_eq!((gold_total(0), gold_total(19)), (0, 0));
	}

	#[test]
	fn every_planet_the_list_can_show_has_one_marker_inside_the_gap_between_the_panels() {
		for id in FIRST..=LAST {
			assert_eq!(MARKS.iter().filter(|m| m.0 == id).count(), 1, "planet {id}");
			let (x, y) = mark(id).unwrap();
			let (px, py) = (x * 1280.0, y * 720.0);
			assert!((480.0..=840.0).contains(&px) && (140.0..=640.0).contains(&py), "planet {id} at {px},{py} touches a panel");
		}
		assert_eq!((mark(0), mark(19), MARKS.len() as u32), (None, None, LAST - FIRST + 1));
		for (i, a) in MARKS.iter().enumerate() {
			for b in &MARKS[i + 1..] {
				let d = ((a.1 - b.1) * 1280.0).hypot((a.2 - b.2) * 720.0);
				assert!(d >= 48.0, "planets {} and {} are {d} px apart", a.0, b.0);
			}
		}
	}

	#[test]
	fn a_row_is_a_planet_by_its_location_text() {
		assert_eq!((planet_of(0x4eb9 + 1), planet_of(0x4eb9 + 18), planet_of(0x4eb9), planet_of(0x4eb9 + 19), planet_of(0x100)), (Some(1), Some(18), None, None, None));
	}

	#[test]
	fn the_painting_loads_at_the_expected_size() {
		let img = crate::texpack::png::load(ART_PNG).expect("the embedded painting decodes");
		assert_eq!((img.w, img.h), ART_SIZE);
		assert_eq!(img.levels[0].len(), (ART_SIZE.0 * ART_SIZE.1 * 4) as usize);
		assert!(ART_PNG.len() < 1_600_000, "ships downscaled, not the 2.2 MB original");
	}
}
