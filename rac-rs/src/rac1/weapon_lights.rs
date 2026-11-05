//! [PC] Weapon lights (v0.5, lane nwlit): R&C1's answer to `Pack::shot_lights` (engine/shot_rows.rs, viewer/shotlight.rs). The
//! Devastator rockets and R.Y.N.O. rockets light from their moby (one light each, at the drawn, smoothed place), the Blaster bolts
//! from their moby's place in the moby table (the shot moby 305 has no model, so it is no prop: retail/fx_queue.rs `shot_mobys`), the
//! Pyrocitor stream from its pool sprites (gathered into a few lights along the jet) and the Tesla Claw from its beam (both ends and
//! the middle, fed per field by retail/queue_b.rs `tesla_points`). The colours below are fallbacks: a light takes its hue from the
//! shot's own effect particles where one is near (`Tracker::sample`); the point rows (the Blaster bolt and the Tesla beam) keep their
//! row colour, for the Tesla the blue the beam shows (measured on the beam's pixels: about (80, 80, 170) added), dimmer per point than
//! a rocket since three points light one beam. One Blaster bolt is one light: in a rapid-fire run (RAC_SHOT_LOG, 8_0328, 81 frames)
//! every frame with a bolt had exactly one class-305 moby, so the bolt's trail mobys (omni_weapons.md: head + 16 trail) are not 305.
//!
//! Sources [C]: decomp/notes/omni_weapons.md (shot classes 305 Blaster, 153 Devastator, 457 R.Y.N.O.), omni_balance.md (held classes),
//! rac1/fire_look.rs (the Pyrocitor jet = pool effect type 12, its sub-bursts = 2), retail/queue_b.rs `tesla` (the beam's points).

use rac_engine::shot_rows::{ShotRow, ShotSrc};

/// The `ShotPoint::tag` of the Tesla Claw beam points (retail/queue_b.rs `tesla_points`).
pub const TESLA: u8 = 0;
/// The `ShotPoint::tag` of a Blaster bolt (retail/fx_queue.rs `shot_mobys`).
pub const BLASTER: u8 = 1;

/// Shot moby classes [C: omni_weapons.md].
const BLASTER_SHOT: u32 = 305;
const DEVASTATOR_ROCKET: u32 = 153;
const RYNO_ROCKET: u32 = 457;
/// Pyrocitor pool sprite effect types [C: fire_look.rs `FireKind::Jet` / `JetBurst`].
const PYRO_JET: u8 = 12;
const PYRO_BURST: u8 = 2;

const fn row(src: ShotSrc, rgb: [f32; 3], reach: f32, gain: f32, flicker: f32, gather: f32, max: u8) -> ShotRow {
	ShotRow { src, rgb, reach, gain, spec: 1.0, flicker, gather, max, with: None, hero_r: 0.0, follow: 0.0 }
}

/// The Pyrocitor rows count only within this distance of the hero (m): other fires of the level use the same effect types (Batalia:
/// type-2 bursts of a burning wreck lit a smoke cloud 18 m away).
const HERO_R: f32 = 10.0;
/// How far a Tesla point may move in one frame and stay the same light (m): the beam's end jumps with its target, the light with it.
const TESLA_FOLLOW: f32 = 40.0;

/// The shot mobys read as light points from the moby table (class, `ShotPoint::tag`): those with no model, never a prop.
pub static MOBY_POINTS: &[(u32, u8)] = &[(BLASTER_SHOT, BLASTER)];

/// The table (`Pack::shot_lights`): source, fallback colour, reach (m), brightness, flicker, gathering radius (m), lights at most.
pub static ROWS: &[ShotRow] = &[
	row(ShotSrc::Point(BLASTER), [1.0, 0.72, 0.28], 5.0, 1.0, 0.0, 0.0, 8),
	row(ShotSrc::Moby(DEVASTATOR_ROCKET), [1.0, 0.55, 0.2], 8.0, 1.3, 0.12, 0.0, 4),
	row(ShotSrc::Moby(RYNO_ROCKET), [1.0, 0.6, 0.25], 7.0, 1.2, 0.12, 0.0, 8),
	ShotRow { hero_r: HERO_R, ..row(ShotSrc::Sprite(PYRO_JET), [1.0, 0.5, 0.15], 7.0, 1.2, 0.25, 2.5, 3) },
	ShotRow { with: Some(ShotSrc::Sprite(PYRO_JET)), hero_r: HERO_R, ..row(ShotSrc::Sprite(PYRO_BURST), [1.0, 0.45, 0.12], 6.0, 0.8, 0.3, 3.0, 1) },
	ShotRow { follow: TESLA_FOLLOW, ..row(ShotSrc::Point(TESLA), [0.5, 0.5, 1.0], 4.5, 0.4, 0.25, 1.5, 3) },
];

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn the_five_weapons_have_rows_and_r_and_c2_has_none() {
		let has = |s: ShotSrc| ROWS.iter().any(|r| r.src == s);
		assert!(has(ShotSrc::Point(BLASTER)) && has(ShotSrc::Moby(153)) && has(ShotSrc::Moby(457)));
		assert!(MOBY_POINTS.contains(&(305, BLASTER)) && BLASTER != TESLA);
		assert!(has(ShotSrc::Sprite(12)) && has(ShotSrc::Point(TESLA)));
		assert!(ROWS.iter().any(|r| r.src == ShotSrc::Sprite(PYRO_BURST) && r.with == Some(ShotSrc::Sprite(PYRO_JET))));
		assert!(ROWS.iter().all(|r| matches!(r.src, ShotSrc::Sprite(_)) == (r.hero_r > 0.0)), "the flame rows only next to the hero");
		assert!(ROWS.iter().all(|r| r.reach > 0.0 && r.gain > 0.0 && r.max > 0 && r.rgb.iter().all(|c| (0.0..=1.0).contains(c))));
		assert!(rac_engine::pack::NONE.shot_lights.is_empty());
		assert_eq!(crate::pack_bind::RAC1.shot_lights.len(), ROWS.len());
	}
}
