//! The Graphics page's row table: the byte of each row (`GROWS`), its label and value texts (`ROWS`), how many values a row has.
//! Moved out of pcext.rs 2026-11-02 (lane v05c, a pure move; pcext.rs is at its size cap) when the Fire and Smoke rows made it 19 rows.
use super::*;

/// The Graphics page's row bytes in row order.
pub(super) const GROWS: [u32; 19] = [LIGHT, AO, SHADOWS, DISPLAY, WATER, PARTICLES, SUNRAYS, SMOOTH, VEG, CULL, GI, WAVES, DRAW, fogopt::FOG, cloudopt::CLOUDS, furopt::FUR, furopt::FURSTYLE, fireopt::FIRE, smokeopt::SMOKE];
/// The Graphics rows of both pages: label, row byte, the value texts for 0, 1, 2 and 3 (0: no such value; the retail item's
/// record holds four choices). The pause page's multi-choice item cycles a row through its values; the title's value item only
/// flips a byte, so `sync` steps the rows with more than two values on.
#[rustfmt::skip]
pub(in crate::retail) const ROWS: [[u32; 6]; 19] = [
	[ID_LIGHTING, LIGHT, ID_LIGHT_CLASSIC, ID_ENHANCED, 0, 0],
	[ID_AO, AO, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_SHADOWS, SHADOWS, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_DISPLAY, DISPLAY, ID_WINDOWED, ID_FULLSCREEN, 0, 0],
	[ID_WATER, WATER, ID_CLASSIC, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_PARTICLES, PARTICLES, ID_PART_CLASSIC, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_SUNRAYS, SUNRAYS, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_SMOOTHING, SMOOTH, ID_OFF, ID_ON, 0, 0],
	[ID_VEGETATION, VEG, ID_OFF, ID_VEG_MODERATE, ID_VEG_LUSH, 0],
	[ID_CULLING, CULL, ID_ON, ID_OFF, 0, 0],
	[ID_GI, GI, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_WAVES, WAVES, ID_ON, ID_OFF, 0, 0],
	[ID_DRAW, DRAW, ID_MEDIUM, ID_HIGH, ID_LOW, 0],
	[ID_FOG, fogopt::FOG, ID_CLASSIC, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_CLOUDS, cloudopt::CLOUDS, ID_CLASSIC, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_FUR, furopt::FUR, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_FURSTYLE, furopt::FURSTYLE, ID_SOFT, ID_FLUFFY, 0, 0],
	[ID_FIRE, fireopt::FIRE, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
	[ID_SMOKE, smokeopt::SMOKE, ID_OFF, ID_HIGH, ID_MEDIUM, ID_LOW],
];
/// How many values row `i` of `ROWS` has (2, 3 or 4).
pub(super) fn row_values(i: usize) -> u8 {
	ROWS[i][2..].iter().take_while(|&&v| v != 0).count() as u8
}

/// One field's step of the rows with more than two values (derived from `ROWS`; fix round M1: Fire and Smoke were missing from a
/// hand-kept index list, so on the title's page they only flipped 1 <-> 0 and never reached Medium / Low). The pause page's
/// multi-choice item cycles a row (+1 mod values), the title's value item only flips the byte; any change since `last` is one press,
/// so the row steps on from its last value on both pages (`write` gets each rewritten byte). Every byte is then held to the row's
/// last value (`min(n - 1)`).
pub(super) fn step_multi(g: &mut [u8; 19], last: Option<&[u8; 19]>, mut write: impl FnMut(usize, u8)) {
	for i in (0..g.len()).filter(|&i| row_values(i) > 2) {
		let n = row_values(i);
		if let Some(last) = last.filter(|l| l[i] != g[i]) {
			g[i] = (last[i] + 1) % n;
			write(i, g[i]);
		}
		g[i] = g[i].min(n - 1);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// The rows the old hand-kept list stepped, plus Fire (17) and Smoke (18): every row with more than two values.
	#[test]
	fn the_multi_value_rows_are_exactly_the_old_list_plus_fire_and_smoke() {
		let multi: Vec<usize> = (0..19).filter(|&i| row_values(i) > 2).collect();
		assert_eq!(multi, [1, 2, 4, 5, 6, 8, 10, 12, 13, 14, 15, 17, 18]);
	}

	/// The title's flip of Fire / Smoke (High 1 -> 0, Low 3 -> 0, ...) steps the row on from its last value like the draw-distance
	/// row's: High -> Medium -> Low -> Off -> High, and a byte past the last value is clamped.
	#[test]
	fn the_title_flip_steps_fire_and_smoke_through_all_four_values() {
		for row in [17usize, 18] {
			let mut last = [0u8; 19];
			last[row] = 1; // High
			// The value item flips 1 -> 0 (Off) each press; the row steps on: 1 -> 2 -> 3 -> 0 -> 1.
			let mut seen = Vec::new();
			let mut cur = 1u8;
			for _ in 0..5 {
				let mut g = last;
				g[row] = if cur == 0 { 1 } else { 0 }; // what the title's flip writes
				let mut wrote = Vec::new();
				step_multi(&mut g, Some(&last), |i, v| wrote.push((i, v)));
				assert_eq!(wrote, [(row, (cur + 1) % 4)], "row {row} from {cur}");
				cur = g[row];
				seen.push(cur);
				last = g;
			}
			assert_eq!(seen, [2, 3, 0, 1, 2], "row {row}");
			// No change = no step; a byte past the last value is clamped, and nothing is written for it.
			let mut g = last;
			step_multi(&mut g, Some(&last), |_, _| panic!("no press"));
			assert_eq!(g, last);
			g[row] = 9;
			let l2 = g;
			step_multi(&mut g, Some(&l2), |_, _| panic!("no press"));
			assert_eq!(g[row], 3, "min(n - 1)");
		}
		// The first field has no last value: bytes are only clamped.
		let mut g = [9u8; 19];
		step_multi(&mut g, None, |_, _| panic!("no press"));
		assert_eq!((g[1], g[8], g[12], g[17], g[18], g[0], g[16]), (3, 2, 2, 3, 3, 9, 9));
	}
}
