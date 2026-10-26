//! Sound helpers of the frame: sound-stage bookkeeping, enemy animation cues, loop voices (with their tests).

use super::*;

/// BRIDGE-24: request-point bookkeeping of one frame's ticks (RNG-4b): how far `Game.events` / `Player.foot_cues`
/// were resolved, the voices granted but not yet started, and whether a new distance-attenuated voice (play flag 8,
/// def without the no-occlusion flag) asks for the tick's six start probes.
#[derive(Default)]
pub(super) struct SoundStage {
}

/// NOV-22.7: the class sound slots of an enemy's queued animation cues that play this frame, oldest first; none once
/// a played-once death sequence has wrapped.
pub(super) fn enemy_frame_cues(cues: &[u16], once: bool) -> Vec<usize> {
	if once {
		return Vec::new();
	}
	cues.iter().map(|&c| c as usize).collect()
}

/// AUD-11f: the position a loop voice follows each frame: its owner (a prop, or the hero for the hero loop slots)
/// raised one unit (FUN_0029ff68). None for a prop index that does not exist. BRIDGE-49: the Visibomb missile's loop
/// follows `vis` (the missile position while it flies).
pub(super) fn loop_at(owner: usize, props: &[game::Prop], hero: [f32; 3], vis: Option<[f32; 3]>) -> Option<[f32; 3]> {
	if owner == game::VISIBOMB_LOOP_OWNER {
		return Some(voice_at(vis?));
	}
	// WPN-6.7a: the held Morph-o-Ray loop follows the hero [I].
	if owner == game::morph::MORPH_LOOP_OWNER {
		return Some(voice_at(hero));
	}
	let p = if owner >= game::HERO_LOOP_OWNER { hero } else { props.get(owner)?.pos };
	Some(voice_at(p))
}

/// What a wanted loop voice needs this frame (AUD-11i).
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
pub(super) enum LoopAct {
Start,
Stop,
Update,
Idle,
}

/// Wanted loop: start when dead and audible, stop when live and out of range, follow when live and audible.
pub(super) fn loop_act(live: bool, audible: bool) -> LoopAct {
match (live, audible) {
	(false, true) => LoopAct::Start,
	(true, false) => LoopAct::Stop,
	(true, true) => LoopAct::Update,
	(false, false) => LoopAct::Idle,
}
}

#[cfg(test)]
mod loop_tests {
	// AUD-11f: loop voices follow the owner (prop or hero) at +1.0 z.
	#[test]
	fn loop_at_follows_owner() {
		assert_eq!(super::loop_at(crate::game::HERO_LOOP_OWNER + 2, &[], [1.0, 2.0, 3.0], None), Some([1.0, 2.0, 4.0]));
		assert_eq!(super::loop_at(5, &[], [1.0, 2.0, 3.0], None), None);
	}

	// AUD-11i test 1: the wanted-loop decision table.
	#[test]
	fn loop_act_table() {
		use super::LoopAct::*;
		assert_eq!(super::loop_act(false, true), Start);
		assert_eq!(super::loop_act(true, false), Stop);
		assert_eq!(super::loop_act(true, true), Update);
		assert_eq!(super::loop_act(false, false), Idle);
	}

	// NOV-6.3 test 4 (writer): `live_loops` = the owners whose voice id plays.
	// NOV-14.5 test 4: ticks 393..=448 of scene 1 give one wake each (56); nothing is emitted from an empty input.
	// KER-28 tests 1-3: the class-1548 tick windows and smoke life bounds.
	#[test]
	fn kerwan28_scaled_ticks() {
		use crate::gameplay::pal_ticks;
		assert_eq!((pal_ticks(900), pal_ticks(10000)), (750, 8333));
		assert_eq!((pal_ticks(45), pal_ticks(60)), (37, 50));
	}

	// KER-28 tests 4-5: scene 3 slot 3 smokes from tick 750 (not 749) through 8333 on Kerwan only; the Kerwan exhaust
	// is scene 4 slot 2 / scene 10 slot 0 of a class-750 actor; Veldin and Novalis gates are unchanged and Kerwan
	// takes none of their arms.
	// KER-28 test 4 (emit): a kerwan28 input installs the two loft smokes (joints 0 and 1).
	// NOV-14.5c tests 1-3 and 5 (the pad frame and the actor filter; 14.5b selection of flow-scene ships).
	// PAR-BRIDGE-B test 3: a non-505 crate break made in the tick spawns the 6 fluffs and no sparks.
	// NOV-22.7 tests 1 and 2: every queued cue plays in order; none once a death has wrapped.
	#[test]
	fn enemy_frame_cues_keeps_all() {
		assert_eq!(super::enemy_frame_cues(&[3, 5], false), vec![3, 5]);
		assert_eq!(super::enemy_frame_cues(&[3, 5], true), Vec::<usize>::new());
	}

	// NOV-22.7 test 4: Peckbot cs0 falloff (0..40, vol 716) is audible at 38.0 (35) and gated at 38.5 (26 < 0x20).
	#[test]
	fn peckbot_cue_gate() {
		let c = crate::sound::Cue { sound: 0, volume: 1.0, looped: false, fall: crate::sound::Falloff { min_range: 0.0, max_range: 40.0, min_vol: 0, max_vol: 716, min_pitch: 0, max_pitch: 0, flags: 0 }, bend: (0, 0) };
		assert_eq!(c.fall.level(38.0), 35);
		assert!(crate::viewer::audible(&c, 38.0));
		assert_eq!(c.fall.level(38.5), 0);
		assert!(!crate::viewer::audible(&c, 38.5));
	}

	// NOV-22.7 test 3: a Peckbot (577) on sequence 2 fires its two triggers over several ticks; one drain keeps both.
	fn cue(min_pitch: i32, max_pitch: i32, flags: u8) -> crate::sound::Cue {
		crate::sound::Cue { sound: 0, volume: 1.0, looped: false, fall: crate::sound::Falloff { min_range: 0.0, max_range: 100.0, min_vol: 0, max_vol: 1000, min_pitch, max_pitch, flags }, bend: (0, 0) }
	}

	fn draws_since(seed: u32, f: impl FnOnce()) -> u32 {
		use crate::particles::Rng;
		Rng::shared().reseed(seed);
		f();
		let after = Rng::shared().next();
		Rng::shared().reseed(seed);
		let mut n = 0;
		while Rng::shared().next() != after && n < 64 {
			n += 1;
		}
		n
	}

	// RNG-4b test 2: a frame of two ticks with no sounds consumes 2 x 3 probe draws (one per tick's audio stage).
	// ARI-4 spec test 10: create + update on the first tick, 246 draws per later tick, none per render frame.
	// RNG-4b test 3: a movie frame (game state 1) runs no probe.
	// RNG-4b: a reserved attenuated voice asks for the six start probes (18 draws) after the tick's 3.
	// RNG-4b test 4: equal range endpoints draw nothing; a rejected voice (volume below 0x20) draws nothing; a range draws once.
	// ARI-27b: a fixed-level (flags 0x10) voice has no range gate and asks for no occlusion probes; it still draws the
	// def's pitch range at the request; `event_cue` resolves `SoundLevel` like `Sound`.
	// RNG-4b: a loop restart draws its pitch range once, at the request in the tick; the drain's start and the running
	// voice's updates draw nothing more.
	// FRAME-1e test 2: the per-tick over camera sources (flow scene, ship stream in the pad frame, none, unloaded id).
}
