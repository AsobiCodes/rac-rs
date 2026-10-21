//! A level's script on top of the gameplay: the HelpDesk tutorial messages, the cutscene chain and
//! the level change at the end. From decomp/notes/veldin_flow.md: the tutorial helper is class 1413,
//! the cutscene and level-end controller is class 834. Scenes 1-4 play from scene.wad (scene.rs, driven
//! from viewer.rs); the FMV (movie 0x4001) plays through scene::movie.

use crate::game::{self};
use crate::gameplay::{Cuboid, Help, MobyInstance};
use crate::game::hints::HelpRecords;
use crate::help::HelpBox;
use std::collections::HashMap;

const TUTOR: u32 = 1413;
const CONTROLLER: u32 = 834;
/// Help request (message id, help slot) per tutor check, as class 1413 codes them.
/// Alpha (/128) of frame `i` (0..n) of a blocking fade of `n` frames (FUN_0021b338 / FUN_0020a670):
/// `128 - ((p - 1) * 128) / p`, p = n - i, integer divide.
fn fade_alpha(i: usize, n: usize) -> f32 {
	let p = n as i32 - i as i32;
	(128 - (p - 1) * 128 / p) as f32 / 128.0
}


/// A dialogue record of the controller's table: (scene, next record, fires on proximity alone).
/// Scene ids with 0x4000 set are movies. Veldin's table is level-code data at 0x1b0870 [C].
const VELDIN_SCENES: [(u32, Option<usize>, bool); 5] = [(4, Some(1), true), (1, Some(2), true), (2, Some(3), true), (0x4001, Some(4), true), (3, None, false)];

#[derive(Clone)]
struct Tutor {
}

#[derive(Clone)]
struct Controller {
	/// The record that fires next; None once the chain has finished.
	record: Option<usize>,
}



#[derive(Clone)]
pub struct Flow {
	messages: HashMap<u32, (String, i16)>,
	controller: Option<Controller>,
	/// Help slot records and move counters (save records 16/17, game/hints.rs); synced with `Game::story`.
	records: HelpRecords,
	pending_seen: Vec<i16>,
	/// The HelpDesk box (help.rs); `triangle` dismisses it (set by the player, used by the next
	/// update), `help_text` is the HelpDesk Text option, `requested` the accepted help ids for the log.
	pub help: HelpBox,
	pub triangle: bool,
	pub help_text: bool,
	pub requested: Vec<u32>,
	/// The help message's voice is still playing (set by the viewer; holds box state 5).
	pub voice: bool,
	/// Messages the game asks for outside the tutor (e.g. the save icon help), shown when free.
	queue: Vec<u32>,
	/// The cutscene playing (caption, seconds left); the hero is frozen meanwhile.
	pub cutscene: Option<(String, f32)>,
	/// SCN-1 step 4: the blocking exit fade (FUN_0021b338(12)): frames shown so far, 0..=12; None outside it.
	fading: Option<u32>,
	/// Frames of the fade running in `fading`: 12 for the scene end, n for an actor's blocking fade.
	fade_len: u32,
	/// Set when the level asks to load the next one.
	pub next_level: Option<usize>,
	/// Decoded scene lengths (seconds) by scene id.
	durations: HashMap<u32, f32>,
	/// The viewer plays movies for real; else they last SCENE_TIME.
	pub movies: bool,
	/// The playing scene (id, length).
	scene: Option<(u32, f32)>,
	/// Set by the player while a decoded scene plays: its subtitle line, if any.
	pub decoded: bool,
	pub subtitle: Option<String>,
	/// The prompt box: last text, keepalive seconds left (10 frames), fade 0..1 (8 frames).
	pub prompt_box: (String, f32, f32),
}


impl Flow {
	pub fn new(instances: &[MobyInstance], cuboids: Vec<Cuboid>, help: &HashMap<u32, Help>, level: usize) -> Flow {
		// Prop slots follow instance order over the dynamic classes (see `Game::new`).
		let slot = |index: usize| -> Option<usize> {
			let m = instances.get(index)?;
			let actors = game::LevelActors::of(level as i32);
			actors.is_dynamic(m.class_id).then(|| instances[..index].iter().filter(|m| actors.is_dynamic(m.class_id)).count())
		};
		let mut tutors = Vec::new();
		let mut controller = None;
		for m in instances {
			let p = Some(m.pvars.as_slice());
			match m.class_id {
				// pvars: three zone cuboids, an unused word, the HelpDesk cuboid, then guard instances.
				TUTOR => tutors.push(Tutor {
				}),
				// pvar +0x60: the cuboid that widens its trigger radius.
				CONTROLLER if level == 0 => controller = Some(Controller {   record: Some(0) }),
				_ => {}
			}
		}
		Flow {
			messages: help.iter().map(|(&id, h)| (id, (h.text.clone(), h.slot))).collect(),
			controller,
			records: HelpRecords::default(),
			pending_seen: Vec::new(),
			help: HelpBox::default(),
			triangle: false,
			help_text: true,
			requested: Vec::new(),
			voice: false,
			queue: Vec::new(),
			cutscene: None,
			fading: None,
			fade_len: 12,
			next_level: None,
			durations: HashMap::new(),
			movies: false,
			scene: None,
			decoded: false,
			subtitle: None,
			prompt_box: (String::new(), 0.0, 0.0),
		}
	}

	pub fn restore(&mut self, record: Option<i32>, seen: &[i16]) {
		if let (Some(c), Some(r)) = (&mut self.controller, record) {
			c.record = usize::try_from(r).ok().filter(|&r| r < VELDIN_SCENES.len());
		}
		self.pending_seen.extend(seen.iter().copied());
	}

	pub fn set_durations(&mut self, durations: HashMap<u32, f32>) {
		self.durations = durations;
	}

	/// The playing scene and seconds into it.
	pub fn scene_time(&self) -> Option<(u32, f32)> {
		let (id, len) = self.scene?;
		let (_, left) = self.cutscene.as_ref()?;
		Some((id, (len - left).max(0.0)))
	}














	/// Help message `id`'s voice (entry +8; -1 = none).
	pub fn voice_of(&self, id: u32) -> Option<usize> {
		self.messages.get(&id).and_then(|m| usize::try_from(m.1).ok())
	}






	/// The cumulative darkness over the retained frame after the fade's frames so far (0 = none, 1 = black).
	pub fn exit_fade_dark(&self) -> Option<f32> {
		let k = self.fading.filter(|&k| k >= 1)?;
		Some(1.0 - (0..k as usize).fold(1.0f32, |b, i| b * (1.0 - fade_alpha(i, self.fade_len as usize))))
	}

}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::level::Level;
	use crate::{gameplay, iso, toc};

	// VEL-14: Clank stays off the hero's back on Veldin (class 834 present), never on other levels, and again after a respawn.
	// VEL-7.5 test 4: cuboid 0 asks (5, 9) only while the Comet Strike was never thrown, cuboid 1 asks (4, 8) only while the
	// Hyper Strike was never used.
	// VEL-7.5 tests 1 and 2: breaking a guard crate asks (1, 1) below 4 health, else (2, 2); the unused words ask nothing.
}
