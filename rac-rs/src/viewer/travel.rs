//! Ship travel after the Galactic Map: flight-away fade, the planet FMVs, the title cards, then the
//! level change through the pending-level path (decomp/notes/level_transition.md "Ship travel and
//! Galactic Map" section 6). The pad's takeoff and the same-planet landing play the prerecorded ship
//! streams (`ShipPlay`, table 0x138eb8); the flight away to another planet (substate 3, a path flight)
//! is `Step::Flight` driving `game::transit::Flight` (passengers, ribbons and the shadow are not drawn yet).

use super::*;

/// Retail mode: the arrival warp drawn from retail's RAM.
pub(super) mod mirror_warp;

/// Scene ids of the ship streams in `App::scenes` (scene.rs `ship_stream`).
pub(super) const TAKEOFF: u32 = u32::MAX - 1;
pub(super) const LANDING: u32 = u32::MAX - 2;

/// Takeoff / landing stream indices (FUN_002a1de8): ship + 1 / ship + 5; Orxon (level 10) with ship 1 while byte
/// 0x13d4c6 is clear: 0 / 4; Oltanis (level 14) with ship 2: 8 / 9.
pub(super) fn stream_ids(level: usize, ship: usize, orxon_clear: bool) -> (usize, usize) {
	match (level, ship) {
		(10, 1) if orxon_clear => (0, 4),
		(14, 2) => (8, 9),
		_ => (ship + 1, ship + 5),
	}
}

/// Takeoff / landing voice cues of the 40000 table (FUN_002a1de8, 40000 + v): takeoff ship + 6 (Orxon 0xb,
/// Oltanis 0xe as `stream_ids`); landing ship + 3 (Orxon 10, Oltanis 0xd), the 0x13e05c-clear case, which
/// every port landing is (boarding clears the flag, FUN_002a20f8).
pub(super) fn voice_ids(level: usize, ship: usize, orxon_clear: bool) -> (u16, u16) {
	let (take, land) = match (level, ship) {
		(10, 1) if orxon_clear => (0xb, 10),
		(14, 2) => (0xe, 0xd),
		_ => (ship + 6, ship + 3),
	};
	(40000 + take as u16, 40000 + land as u16)
}

/// The ship voice stream of a 40000-table cue (ToC +0x13c0 + 8 * (cue - 40000), a plain VAG, FUN_00279280);
/// None when the slot has no LBA or the read fails.
pub(super) fn ship_voice_clip(disc: &mut iso::Disc, cue: u16) -> Option<audio::Clip> {
	let t = toc::read_toc(disc).map_err(|e| eprintln!("ship voice {cue}: {e}")).ok()?;
	let lba = crate::bytes::u32_at(&t, 0x13c0 + 8 * (cue as usize).checked_sub(40000)?).ok()?;
	if lba == 0 {
		return None;
	}
	let m = music::read_vag(disc, lba as u64, 0).map_err(|e| eprintln!("ship voice {cue}: {e}")).ok()?;
	Some(audio::Clip { rate: m.rate, pcm: Arc::new(m.pcm), loop_start: None, reverb: false })
}

/// A playing ship stream: mode 6 substate 0 (takeoff) or 8 (landing), update FUN_002a39b0.
pub(super) struct ShipPlay {
	pub(super) id: u32,
	/// Ticks into the stream (DAT_0016cc94).
	pub(super) tick: f32,
	total: f32,
	/// Fade from black (DAT_0015f3fc): 1.0 at the start and after a skip, 0.125 less per tick.
	fade: f32,
	/// The ship prop, hidden while its stream double plays (FUN_002a1de8 sets its +0x34 bit 0).
	prop: usize,
}

/// SHR-FRAME1: the retail landing stream length (0x16c920, every landing stream; level_transition.md:110).
const LANDING_LEN: u32 = 614;

/// SHR-FRAME1 rule 2: the frame counter a level entry's first gameplay frame reads. Retail enters at once (counter 1)
/// on Veldin, on Novalis before Kerwan is unlocked, on Oltanis before its flag (no Rust reader yet: taken as clear)
/// and past level 0x13; every other entry first plays the unskipped arrival landing (not ported), which leaves L.
pub(super) fn arrival_frames(level: usize, story: &crate::save::Story) -> u32 {
	let direct = level == 0 || (level == 1 && story.unlock[3] == 0) || level == 0xe || level > 0x13;
	if direct { 1 } else { LANDING_LEN }
}


impl ShipPlay {

	pub(super) fn draw(&self, out: &mut Vec<Quad>, w: f32, h: f32) {
		if self.fade > 0.0 {
			overlay::rect(out, [0.0, 0.0, w, h], [0.0, 0.0, 0.0, self.fade]);
		}
	}
}


#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Step {
	/// The substate-3 flight away: black pre-fade, the ship voice hold, then the path flight (SCN-4.3a).
	Flight,
	/// The substate-4 arrival warp (SCN-4.1c): `game::transit::Warp` at 50 Hz with the ship actor posed every tick.
	Warp,
	/// A title card of one or two plates (FUN_002a5e10 arguments; plate entry = argument + 1), in ticks.
	Card { first: usize, second: Option<usize>, ticks: f32 },
	/// Planet FMV k (PAL mpegs/(52+k).pss; movie id 31 + k).
	Movie(usize),
}

/// The substate-3 flight state: pre-fade ticks, voice hold ticks, then the ported `Flight`.
pub(super) struct FlightRun {
	pre: u32,
	hold: u32,
	hold_ticks: u32,
	flight: Option<crate::game::transit::Flight>,
	prop: usize,
	acc: f32,
	/// The level has both camera cuboids (`Flight.cams` is private).
	cams: bool,
	/// Ship type 0..=2 (`Flight.ship_type` is private) and whether the flight follows a path (SCN-4.3c).
	ship_type: usize,
	has_path: bool,
	/// SCN-4.3e: the pad ship registered its ground shadow this tick (override byte set and within 32 horizontally).
	shadow_gate: bool,
	/// The game camera before this tick, for the gate when the level has no scripted flight camera.
	pub game_cam: Option<[f32; 3]>,
}

impl FlightRun {

	/// Black overlay alpha: the 10 pre-fade frames, full black while the voice plays, then the flight's fade
	/// (full again from the exit tick, when retail stops drawing the world).
	fn alpha(&self) -> f32 {
		match &self.flight {
			Some(f) if f.exit => 1.0,
			Some(f) => f.fade.min(1.0),
			None if self.hold > 0 => 1.0,
			None if self.pre > 0 => crate::game::transit::flight_fade_alpha(self.pre - 1) as f32 / 128.0,
			None => 0.0,
		}
	}

	/// The pose the draw wants while the flight runs: None before the first flight tick and from the exit tick on.
	fn pose(&self) -> Option<FlightPose> {
		let f = self.flight.as_ref().filter(|f| !f.exit)?;
		Some(FlightPose { prop: self.prop, ship_type: self.ship_type, blend: f.ship_blend, pos: f.ship_pos, rot: f.ship_rot, rot_prev: f.rot_prev, passengers: self.has_path })
	}

}

/// What the departure flight asks the draw for (SCN-4.3c): the ship prop's animation blend and the two passengers.
#[derive(Clone, Copy, Debug)]
pub(super) struct FlightPose {
	pub prop: usize,
	pub ship_type: usize,
	pub blend: f32,
	pub pos: [f32; 3],
	pub rot: [f32; 4],
	/// The rotation the tick began with (the seats' basis).
	pub rot_prev: [f32; 4],
	pub passengers: bool,
}

/// The arrival warp (SCN-4.1c): the ported tick state, the bundle's assets, the current stream and the posed ship.
pub(super) struct WarpRun {
	warp: crate::game::transit::Warp,
	bundle: Vec<u8>,
	scene: scene::Scene,
	assets: scene::WarpAssets,
	anims: HashMap<u32, anim::Animated>,
	mk: usize,
	acc: f32,
	/// Actor scale over class scale (1.0 except during the type-4 shrink).
	factor: f32,
	/// The two shrink factors the last tick's iterations used (iteration 0 the old value, 1 the new one).
	iter_factors: [f32; 2],
	last: crate::game::transit::WarpTick,
	/// Posed actors for the batch: (class, skinned positions and normals with the factor applied, position).
	posed: Vec<(u32, Posed, [f32; 3])>,
	/// Slot-0 actor's joint-0 matrix, class and position (the SCN-4.2 gloss input).
	ship_j: Option<(u32, [f32; 12], [f32; 3])>,
	/// The big quad's and the banner's textures for this (from, to) pair (SCN-4.2).
	big: crate::level::Texture,
	banner: Option<crate::level::Texture>,
	/// The ship gloss quads of the last tick (SCN-4.2 Order 6).
	gloss: Vec<particles::WorldQuad>,
	/// The bundle's sound bank (`w[0x19]` section), None when it does not parse.
	bank: Option<crate::sound::Bank>,
	/// Warp sound index the last tick asked for (the tick-1 start), taken once by the viewer.
	sound_ty: Option<usize>,
}

impl WarpRun {

	fn cam_key(&self) -> Option<scene::CameraKey> {
		let mut k = self.scene.camera_at(self.warp.tick as f32)?;
		k.tan_half_fov = k.tan_half_fov.max(self.last.proj_floor);
		Some(k)
	}

}

pub(super) struct Travel {
	from: usize,
	to: usize,
	steps: Vec<Step>,
	i: usize,
	/// Seconds into the step.
	t: f32,
	begun: bool,
	/// The running flight (Step::Flight).
	flight: Option<FlightRun>,
	/// The running arrival warp (Step::Warp).
	warp: Option<WarpRun>,
	/// Plate entry -> HUD texture slot.
	plates: HashMap<usize, usize>,
	movie: Option<fmv::player::Player>,
	held: bool,
	/// The sequence ended and the level change is queued.
	done: bool,
}

impl Travel {
	/// The warp stream's camera key (stream space) with the projection scalar floored as SCN-4.1 step 4 says.
	pub(super) fn warp_cam(&self) -> Option<scene::CameraKey> {
		self.warp.as_ref()?.cam_key()
	}

	/// The running `Flight` (SCN-4.3d ribbons, SCN-4.3e shadow).
	pub(super) fn flight_state(&self) -> Option<&crate::game::transit::Flight> {
		self.flight.as_ref()?.flight.as_ref()
	}

	/// The pad ship's prop index while its flight shadow is queued (flight running, not exited, gate set).
	pub(super) fn flight_shadow_prop(&self) -> Option<usize> {
		let run = self.flight.as_ref()?;
		run.flight.as_ref().filter(|f| !f.exit && run.shadow_gate)?;
		Some(run.prop)
	}

	/// The flight's ship blend and passenger placement while it runs (None before the first flight tick and from the
	/// exit tick on).
	pub(super) fn flight_pose(&self) -> Option<FlightPose> {
		self.flight.as_ref()?.pose()
	}

	/// The scripted flight camera (world position, euler angles) while the flight has both cuboids.
	pub(super) fn flight_cam(&self) -> Option<([f32; 3], [f32; 3])> {
		let f = self.flight.as_ref().filter(|f| f.cams)?.flight.as_ref()?;
		Some((f.cam_pos, f.cam_rot))
	}

	/// Alpha of a card plate at tick `i` of `ticks`: in 4 per tick up to 0x80 (32 ticks), out 8 per tick over the last 16.
	fn alpha(i: f32, ticks: f32) -> f32 {
		let mut a = (i * 4.0).min(128.0);
		if i > ticks - 16.0 {
			a = ((ticks - i) * 8.0).min(a);
		}
		a.max(0.0) / 128.0
	}

	pub(super) fn draw(&self, out: &mut Vec<Quad>, w: f32, h: f32) {
		let (k, ox) = (h / 448.0, (w - 512.0 * h / 448.0) / 2.0);
		let (alpha, plates): (f32, Vec<(usize, f32, f32)>) = match self.steps.get(self.i) {
			Some(Step::Flight) => (self.flight.as_ref().map_or(0.0, FlightRun::alpha), Vec::new()),
			Some(&Step::Card { first, second, ticks }) => {
				let i = self.t * 50.0;
				let a1 = Travel::alpha(i, ticks);
				let mut v = Vec::new();
				match second {
					None => v.push((first + 1, 192.0, a1)),
					Some(s) => {
						v.push((first + 1, 178.0, a1));
						if i > 64.0 {
							let a2 = if i < 96.0 { (i - 64.0) * 4.0 / 128.0 } else { a1 };
							v.push((s + 1, 224.0, a2));
						}
					}
				}
				(1.0, v)
			}
			_ => (1.0, Vec::new()),
		};
		overlay::rect(out, [0.0, 0.0, w, h], [0.0, 0.0, 0.0, alpha]);
		// FUN_002a5e10: the 64x64 texture (entry 0) over the plate rect, S 0..4, T scrolling a full turn
		// every 600 ticks and spanning 0.4, modulated 0x808080 with the fade alpha (FUN_002a5920); then
		// the plate's black mask over it at its texel alpha (FUN_0021bd90; its colour argument is an
		// unset register in the decompile, taken as 0x80808080 [I]).
		let v0 = ((self.t * 50.0) as u32 % 600) as f32 / 600.0;
		for (entry, y, a) in plates {
			let rect = [ox, y * k, 512.0 * k, 64.0 * k];
			if let Some(&back) = self.plates.get(&0) {
				out.push(Quad { tex: back, rect, uv: [0.0, v0, 4.0, v0 + 0.4], colour: [1.0, 1.0, 1.0, a], angle: 0.0, corners: None });
			}
			if let Some(&slot) = self.plates.get(&entry) {
				out.push(Quad { tex: slot, rect, uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
			}
		}
	}
}


#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn ship_stream_ids() {
		assert_eq!(stream_ids(1, 0, false), (1, 5));
		assert_eq!(stream_ids(10, 1, true), (0, 4));
		assert_eq!(stream_ids(10, 1, false), (2, 6));
		assert_eq!(stream_ids(14, 2, false), (8, 9));
	}

	#[test]
	fn ship_voice_ids() {
		assert_eq!(voice_ids(1, 0, false), (40006, 40003));
		assert_eq!(voice_ids(10, 1, true), (40011, 40010));
		assert_eq!(voice_ids(10, 1, false), (40007, 40004));
		assert_eq!(voice_ids(14, 2, false), (40014, 40013));
	}

	#[test]
	fn ship_voice_reads_the_40000_table() {
		let path = std::path::PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"));
		let mut disc = iso::Disc::open(&path).unwrap();
		let t = toc::read_toc(&mut disc).unwrap();
		assert_eq!(crate::bytes::u32_at(&t, 0x13c0 + 8 * 6).unwrap(), 1881536);
		assert_eq!(crate::bytes::u32_at(&t, 0x13c0).unwrap(), 1880762);
		let c = ship_voice_clip(&mut disc, 40006).unwrap();
		assert_eq!(c.rate, 44100);
	}

	/// SHR-FRAME1 test d: the counter a level entry starts with.
	#[test]
	fn shr_frame1_arrival_frames() {
		let mut s = crate::save::Story::default();
		assert_eq!([0, 1, 2, 3, 0x14].map(|l| arrival_frames(l, &s)), [1, 1, 614, 614, 1]);
		s.unlock[3] = 1;
		assert_eq!(arrival_frames(1, &s), 614);
	}
}
