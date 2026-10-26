//! [PC] The shore map's height reference, free of the tide (Split view: inside the water pass, `DrawFx.water_on`). The shore map (`shore_map.rs`) sorts the ground round the sea by
//! height above the sea (low shore, wall, the surface band), but Pokitaru's retail sea has a tide: its plane rises and falls
//! 222.76 .. 223.28 (about 0.5) every ~360 fields [C: RAC_WATER_LOG at runs\st_a13 over 1500 fields, 2026-10-25]. Read against
//! the moving plane, a flat sand bank (z ~223.7, inside top + 0.6's sweep) flipped between "low shore" and "wall" once each
//! half tide, and the waves of the whole inlet switched off and on in one frame (user 2026-10-25: "the waves still flicker
//! change and disappear"). The map now reads heights against the plane's mid-tide: the middle of the lowest and highest
//! steadied plane seen since the level (or the sea) changed, seeded with the measured tide where it is known so it holds still
//! from the first frame. The wave and grid uniforms keep the moving plane (render-only, Enhanced geometric sea only).

/// A plane further than this outside the heights seen (more than any tide) is another sea: the reference starts again.
const NEW_SEA: f32 = 1.0;

/// The measured tide (lowest, highest plane) of a level's sea, by level [C: RAC_WATER_LOG]: Pokitaru 222.758 .. 223.275.
fn known_tide(level: usize) -> Option<(f32, f32)> {
	match level {
		11 => Some((222.758, 223.275)),
		_ => None,
	}
}

/// The lowest and highest steadied plane seen on `level`.
#[derive(Default)]
pub(super) struct ShoreRef {
	seen: Option<(usize, f32, f32)>,
}

impl ShoreRef {
	/// The tide-free height the shore map measures against, for this frame's highest steadied sea plane `top` on `level`.
	pub(super) fn at(&mut self, level: usize, top: f32) -> f32 {
		let fresh = || match known_tide(level) {
			Some((lo, hi)) if top >= lo - NEW_SEA && top <= hi + NEW_SEA => (level, lo.min(top), hi.max(top)),
			_ => (level, top, top),
		};
		let s = match self.seen {
			Some((l, lo, hi)) if l == level && top >= lo - NEW_SEA && top <= hi + NEW_SEA => (l, lo.min(top), hi.max(top)),
			_ => fresh(),
		};
		self.seen = Some(s);
		0.5 * (s.1 + s.2)
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	/// Over a tide the reference holds still on a level with a known tide, and settles after one cycle on any other.
	#[test]
	fn the_tide_does_not_move_it() {
		let tide = |f: f32| 223.017 + 0.258 * (f * std::f32::consts::TAU / 360.0).sin();
		let mut r = ShoreRef::default();
		let first = r.at(11, tide(0.0));
		assert!((0..2000).all(|f| (r.at(11, tide(f as f32)) - first).abs() < 1e-4));
		let mut o = ShoreRef::default();
		(0..400).for_each(|f| _ = o.at(4, tide(f as f32)));
		let settled = o.at(4, tide(400.0));
		assert!((400..2000).all(|f| (o.at(4, tide(f as f32)) - settled).abs() < 1e-3));
	}

	/// Another level or another sea (beyond any tide) starts again.
	#[test]
	fn a_new_sea_starts_again() {
		let mut r = ShoreRef::default();
		r.at(4, 10.0);
		r.at(4, 10.4);
		assert_eq!(r.at(4, 50.0), 50.0);
		assert_eq!(r.at(5, 10.0), 10.0);
	}
}
