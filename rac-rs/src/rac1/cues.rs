//! Which bank sound R&C1 plays for each gameplay event (engine split stage E2, 2026-10-26: moved out of formats/sound.rs,
//! which keeps the bank, sound-map and class-sound formats): the hero, wrench, Bomb Glove, bolt, crate, enemy and
//! pause-menu cues by R&C1 moby class (decomp/notes/sound.md "Event table").

use crate::bytes::R;
use crate::level::Level;
use crate::sound::{bend_range, class_block, class_cue, class_sound_counts, level_bank, sound_map, static_loop_sound, Cue};
use std::collections::HashMap;

/// Which bank sound the game plays for each gameplay event (see notes/sound.md "Event table").
/// Pitch is always unshifted (the game only adds doppler), so no pitch field.
#[derive(Debug, Clone, Default)]
pub struct Events {
	/// Hero jump grunt (class sound 26; trigger of sequences 7-9).
	pub jump: Option<Cue>,
	/// Hero landing (class sounds 30 and 31, sequence 12).
	pub land: Option<Cue>,
	/// Hero hurt grunt (class sound 33, sequence 16).
	pub hurt: Option<Cue>,
	/// Wrench swoosh per combo hit (class sounds 27 for hits 1-2, 54 for hit 3).
	pub swing: Vec<Cue>,
	/// Bolt magnet pickup by bolt class (13 = 1 bolt .. 16 = 50), class sound 0 of the bolt class.
	pub bolt: HashMap<u32, Cue>,
	/// Crate break by crate class (class sound 0 of the crate).
	pub crate_break: HashMap<u32, Cue>,
	/// Nanotech crate health pickup (class 501 sound 1).
	pub nanotech: Option<Cue>,
	/// 16 footstep cues indexed `surface_class * 4 + foot * 2 + variant`.
	pub footsteps: Vec<Option<Cue>>,
	/// First looping level sound (emitter ambience).
	pub ambient: Option<Cue>,
	/// NOV-5.8a: level sound-map def 1 (FUN_002a10a0(1, 0, 0), the Take Aim cue), played flat.
	pub global1: Option<Cue>,
	/// Enemy sounds by (moby class, event name): class sounds named by animation role.
	pub enemy: HashMap<(u32, &'static str), Cue>,
	/// Every class sound of the toad (0..=8) and robot (0..=10) by (class, index), for animation cues and
	/// loop sounds (sound.md "Animation sound cues").
	pub enemy_cs: HashMap<(u32, usize), Cue>,
	/// Loop sound (class sound index) of skeleton-less cue classes (the fan), played from sequence 0's loop byte.
	pub static_loops: HashMap<u32, usize>,
	pub glove: GloveCues,
	/// Wrench throw and catch (hero cs27 / cs5, banks 47 / 25; decomp/notes/sound.md task 2).
	pub wrench_throw: Option<Cue>,
	pub wrench_catch: Option<Cue>,
	/// Thrown-wrench impacts: class 71 cs0, cs1 (moby hit) and cs2 (wall), banks 95..97.
	pub wrench_impact: [Option<Cue>; 3],
	/// Pause-menu cues: class 0x472 (the menu panel mobys) cs0..7 through the level's class remap (Veldin banks
	/// 213..220, Novalis 272..279), indexed by `frontend::Cue` (decomp/notes/ingame_pause_menu.md s2).
	pub menu: Vec<Option<Cue>>,
	/// Hyper-Strike seq 43 triggers: hero cs53 and cs27 (veldin_enemies.md 7.2).
	pub slam: [Option<Cue>; 2],
	/// Hero health death (states 0x7b / 0x7c): hero cs9, bank 29.
	pub death: Option<Cue>,
	/// Every hero (class 0) cue 0..80 by class sound index, for the sequence triggers (death sequences
	/// 69 / 113 / 17; decomp/notes/sound.md).
	pub hero_cs: HashMap<usize, Cue>,
	/// Level mechanism sounds by (moby class, class sound index) (game::Event::Sound).
	pub class: HashMap<(u32, usize), Cue>,
}

/// Bomb Glove sounds, all in the level bank (decomp/notes/glove_prereq_leftovers.md, sound table).
#[derive(Debug, Clone, Default)]
pub struct GloveCues {
	/// Throw whoosh (hero cs26, bank 46).
	pub throw: Option<Cue>,
	/// Fire with no ammo (class 192 cs0, bank 154).
	pub empty: Option<Cue>,
	/// Glove drawn (class 192 cs1, bank 155).
	pub draw: Option<Cue>,
	/// Ammo pickup (class 213 cs0, bank 164).
	pub pickup: Option<Cue>,
	/// Small explosion (class 121 cs0 / cs1 gold: bank 104 / 105) and the big one (hero cs22, bank 42).
	pub small: [Option<Cue>; 2],
	pub big: Option<Cue>,
}

/// Number of global defs preceding the footstep defs on Veldin.
const FOOTSTEP_BASE: usize = 2;

/// First footstep def of a level: the global base 2 plus the per-level byte of the retail table at 0x1bdc20
/// (FUN_002a11c8; decomp/notes/sound.md AUD-11g): Veldin 2, Novalis 4.
pub fn footstep_base(level_id: usize) -> usize {
	const TABLE: [usize; 8] = [0, 2, 6, 1, 0, 2, 2, 0];
	FOOTSTEP_BASE + TABLE.get(level_id).copied().unwrap_or(0)
}

pub fn events(level: &Level) -> R<Events> {
	let mut ev = Events::default();
	let cs = |class: u32, i: usize| class_cue(level, class, i);
	ev.jump = cs(0, 26)?;
	ev.land = cs(0, 30)?;
	ev.hurt = cs(0, 33)?;
	for i in [27, 54] {
		ev.swing.extend(cs(0, i)?);
	}
	for class in 13..=16 {
		if let Some(c) = cs(class, 0)? {
			ev.bolt.insert(class, c);
		}
	}
	for class in [500, 501, 502, 505, 511] {
		if let Some(c) = cs(class, 0)? {
			ev.crate_break.insert(class, c);
		}
	}
	ev.nanotech = cs(501, 1)?;
	// Every cue is a class sound with its own def (gadget classes through the gadget table); an unmapped
	// index plays nothing, as in the game (Novalis maps hero cs9 to 0xffff: no death cue there).
	ev.glove = GloveCues {
		throw: cs(0, 26)?,
		empty: cs(192, 0)?,
		draw: cs(192, 1)?,
		pickup: cs(213, 0)?,
		small: [cs(121, 0)?, cs(121, 1)?],
		big: cs(0, 22)?,
	};
	ev.wrench_throw = cs(0, 27)?;
	ev.wrench_catch = cs(0, 5)?;
	ev.wrench_impact = [cs(71, 0)?, cs(71, 1)?, cs(71, 2)?];
	ev.death = cs(0, 9)?;
	for i in 0..80 {
		if let Some(c) = cs(0, i)? {
			ev.hero_cs.insert(i, c);
		}
	}
	// NOV-13.2: every def the class block declares (+0x0d), not only the first four.
	let counts: HashMap<u32, usize> = class_sound_counts(level)?.into_iter().filter_map(|(c, n)| n.map(|n| (c, n))).collect();
	for &class in crate::game::SOUND_CLASSES {
		// Gadget classes (the held Suck Cannon 849) have no level-table block: their count is the gadget block's.
		let n = match counts.get(&class) {
			Some(&n) => n,
			None => class_block(level, class)?.map_or(4, |b| b.get(0x0d).copied().unwrap_or(0) as usize),
		};
		for i in 0..n {
			if let Some(c) = cs(class, i)? {
				ev.class.insert((class, i), c);
			}
		}
	}
	ev.slam = [cs(0, 53)?, cs(0, 27)?];
	let map = sound_map(level)?;
	let bank = level_bank(level)?;
	for i in 0..16 {
		let d = map.defs.get(footstep_base(level.id) + i).filter(|d| d.bank_sound.is_some());
		ev.footsteps.push(d.map(|d| Cue {
			sound: d.bank_sound.unwrap() as usize,
			volume: (d.max_vol.max(0) as f32 / 1024.0).min(1.0),
			looped: false,
			fall: d.falloff(),
			bend: bend_range(&bank, d.bank_sound.unwrap() as usize),
		}));
	}
	ev.global1 = map.defs.get(1).filter(|d| d.bank_sound.is_some()).map(|d| Cue {
		sound: d.bank_sound.unwrap() as usize,
		volume: (d.max_vol.max(0) as f32 / 1024.0).min(1.0),
		looped: false,
		fall: d.falloff(),
		bend: bend_range(&bank, d.bank_sound.unwrap() as usize),
	});
	ev.ambient = map.defs.iter().skip(FOOTSTEP_BASE + 16).find(|d| d.looped && d.bank_sound.is_some()).map(|d| Cue {
		sound: d.bank_sound.unwrap() as usize,
		volume: (d.max_vol.max(0) as f32 / 1024.0).min(1.0),
		looped: true,
		fall: d.falloff(),
		bend: bend_range(&bank, d.bank_sound.unwrap() as usize),
	});
	// Enemy roles (class sound per animation role; roles from sequence triggers, see notes).
	let toad = [("hit", 0), ("death", 1), ("attack", 2), ("step", 3), ("attack_end", 4), ("land", 6), ("idle", 8)];
	let robot = [("idle", 0), ("death", 1), ("death2", 2), ("hurt", 4), ("chase_loop", 5), ("zap", 6), ("zap2", 8), ("patrol_loop", 10)];
	for &class in crate::game::CUE_CLASSES {
		let n = counts.get(&class).copied().unwrap_or(0);
		for i in 0..n {
			if let Some(c) = cs(class, i)? {
				ev.enemy_cs.insert((class, i), c);
			}
		}
		if let Some(w) = static_loop_sound(level, class)? {
			ev.static_loops.insert(class, w);
		}
	}
	for (class, list) in [(749u32, &toad[..]), (1440, &robot[..])] {
		for &(name, i) in list {
			if let Some(c) = cs(class, i)? {
				ev.enemy.insert((class, name), c);
			}
		}
	}
	ev.menu = (0..8).map(|i| cs(MENU_CLASS, i)).collect::<R<_>>()?;
	Ok(ev)
}

/// The menu panel moby class (14 spawned per page menu); its class sounds are the menu cues.
pub const MENU_CLASS: u32 = 0x472;
