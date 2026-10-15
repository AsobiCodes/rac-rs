//! Ship travel after the Galactic Map: flight-away fade, the planet FMVs, the title cards, then the
//! level change through the pending-level path (decomp/notes/level_transition.md "Ship travel and
//! Galactic Map" section 6). The pad's takeoff and the same-planet landing play the prerecorded ship
//! streams (`ShipPlay`, table 0x138eb8); the flight away to another planet (substate 3, a path flight)
//! is `Step::Flight` driving `game::transit::Flight` (passengers, ribbons and the shadow are not drawn yet).

use super::*;

/// Retail mode: the arrival warp drawn from retail's RAM.
pub(super) mod mirror_warp;

/// Seconds per tick of the flight (PAL dt, f32 0x3ca3d70b).
const FLIGHT_DT: f32 = 0.02;
/// Card lengths in 50 Hz ticks: FUN_002a5e10 gets 0xf0 / 0xb4 scaled ticks (the note gives 180 for 0xf0
/// on PAL; 0xb4 scaled the same way).
const CARD: f32 = 180.0;
const CARD_SHORT: f32 = 135.0;

/// Scene ids of the ship streams in `App::scenes` (scene.rs `ship_stream`).
pub(super) const TAKEOFF: u32 = u32::MAX - 1;
pub(super) const LANDING: u32 = u32::MAX - 2;
/// Landing start: FUN_00220960(240) (V 0x1604f0, all three ships).
const LANDING_START: f32 = 200.0;

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

/// The sequence for travelling `from` -> `to` (FUN_002a6228; conditions on visited[] and unlock[]).
pub(super) fn steps(from: usize, to: usize, s: &save::Story) -> Vec<Step> {
	if from == to {
		return Vec::new();
	}
	let u = |i: usize| s.unlock.get(i).copied().unwrap_or(0) != 0;
	let v = |i: usize| s.visited.get(i).copied().unwrap_or(0);
	let card = |first, second, ticks| Step::Card { first, second, ticks };
	let mut out = vec![Step::Flight];
	if from == 0 && to == 1 && v(1) == 0 {
		out.extend([card(5, Some(6), CARD), Step::Movie(3), Step::Movie(4), card(7, None, CARD_SHORT), Step::Movie(5), card(8, None, CARD), Step::Warp]);
		return out;
	}
	if to == 4 && v(4) == 0 {
		out.extend([card(9, Some(10), CARD), Step::Movie(6)]);
	}
	if from == 7 && v(7) != 2 && u(8) {
		out.extend([card(11, None, CARD), Step::Movie(7)]);
	}
	if to == 13 && v(13) == 0 {
		out.extend([card(12, Some(13), CARD), Step::Movie(8)]);
	}
	if from == 14 && v(14) != 2 && u(15) {
		out.extend([card(14, None, CARD), Step::Movie(9)]);
	}
	if to == 16 && v(16) == 0 {
		out.extend([card(15, Some(16), CARD), Step::Movie(10)]);
	}
	out.push(Step::Warp);
	out
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
	fn new(ship_type: usize, prop: usize, cams: bool) -> FlightRun {
		let hold_ticks = (crate::game::transit::flight_voice_seconds(ship_type) * 50.0).ceil() as u32;
		FlightRun { pre: 0, hold: 0, hold_ticks, flight: None, prop, acc: 0.0, cams, ship_type: ship_type.min(2), has_path: false, shadow_gate: false, game_cam: None }
	}

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

	/// Runs the whole ticks that `dt` seconds make; true when the step is over.
	fn advance(&mut self, dt: f32, pad_held: bool, props: &mut [Prop], data: &gameplay::FlightData) -> bool {
		self.acc += dt;
		while self.acc >= FLIGHT_DT {
			self.acc -= FLIGHT_DT;
			if self.tick(pad_held, props, data) {
				return true;
			}
		}
		false
	}

	/// One 50 Hz tick; true when the step is over.
	fn tick(&mut self, pad_held: bool, props: &mut [Prop], data: &gameplay::FlightData) -> bool {
		if self.pre < 10 {
			self.pre += 1;
			return false;
		}
		if self.hold < self.hold_ticks {
			self.hold += 1;
			return false;
		}
		let t = self.ship_type(props);
		let path = data.0.as_deref().filter(|p| p.len() >= 3);
		let prop = self.prop;
		let cams = if self.cams { data.1 } else { None };
		self.ship_type = t;
		self.has_path = path.is_some();
		// SCN-4.3e: the actor pass at the head of the tick reads the ship's and the camera's previous-tick positions.
		let cam = if self.cams { self.flight.as_ref().map(|f| f.cam_pos).or(self.game_cam) } else { self.game_cam };
		self.shadow_gate = cam.is_some_and(|c| shadow_gate(props[prop].sched_over(), props[prop].pos, c));
		let f = self.flight.get_or_insert_with(|| {
			let p = &props[prop];
			crate::game::transit::Flight::new(t, p.pos, p.rot, path, cams)
		});
		f.tick(pad_held);
		if path.is_some() {
			let p = &mut props[prop];
			p.pos = f.ship_pos;
			p.rot = [f.ship_rot[0], f.ship_rot[1], f.ship_rot[2]];
			p.shown = true;
			f.exit
		} else {
			f.c >= 125
		}
	}

	/// The pose the draw wants while the flight runs: None before the first flight tick and from the exit tick on.
	fn pose(&self) -> Option<FlightPose> {
		let f = self.flight.as_ref().filter(|f| !f.exit)?;
		Some(FlightPose { prop: self.prop, ship_type: self.ship_type, blend: f.ship_blend, pos: f.ship_pos, rot: f.ship_rot, rot_prev: f.rot_prev, passengers: self.has_path })
	}

	fn ship_type(&self, props: &[Prop]) -> usize {
		(props[self.prop].class_id.wrapping_sub(0x213) as usize).min(2)
	}
}

/// SCN-4.3e: the pad ship queues its ground shadow when its scheduler override byte is set and it is within 32.0 of
/// the camera horizontally (strict).
pub(super) fn shadow_gate(over: u8, pos: [f32; 3], cam: [f32; 3]) -> bool {
	over != 0 && ((pos[0] - cam[0]).powi(2) + (pos[1] - cam[1]).powi(2)).sqrt() < 32.0
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

/// World position of joint `j` of a posed actor: `pos + factor * joint translation` (`FUN_002640a0`; the joint matrix is
/// already in game units, so retail's `+0x2c / 1024` becomes the shrink factor alone).
fn joint_point(pos: [f32; 3], factor: f32, joint: &[f32; 12]) -> [f32; 3] {
	[pos[0] + factor * joint[3], pos[1] + factor * joint[7], pos[2] + factor * joint[11]]
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
	fn new(from: usize, to: usize, story: &save::Story, disc: &mut iso::Disc) -> Result<WarpRun, String> {
		let u = |i: usize| story.unlock.get(i).copied().unwrap_or(0) != 0;
		let mk = crate::game::transit::ship_mark(u(8), u(14), to);
		let bundle = scene::warp_bundle(disc)?;
		let assets = scene::warp_assets(&bundle)?;
		let (big, banner) = scene::warp_pair_textures(&bundle, from, to)?;
		let mut anims = HashMap::new();
		for (id, _, block) in &assets.classes {
			if let Some(a) = anim::animation_of_class_block(block, *id)? {
				anims.insert(*id, a);
			}
		}
		let mut first = None;
		let warp = crate::game::transit::Warp::new(to, u(3), |ty| match scene::warp_stream_from(&bundle, ty) {
			Ok(s) => {
				let n = s.ticks;
				first = Some(s);
				n
			}
			Err(e) => {
				eprintln!("warp stream {ty}: {e}");
				0
			}
		});
		let scene = first.ok_or("warp stream did not load")?;
		let bank = (|| {
			let at = crate::bytes::u32_at(&bundle, 4).ok()? as usize + crate::bytes::u32_at(&bundle, 0x19 * 4).ok()? as usize;
			crate::sound::Bank::parse(bundle.get(at..)?).map_err(|e| eprintln!("warp sound bank: {e}")).ok()
		})();
		Ok(WarpRun { warp, bundle, scene, assets, anims, mk, acc: 0.0, factor: 1.0, iter_factors: [1.0; 2], last: Default::default(), posed: Vec::new(), ship_j: None, big, banner, gloss: Vec::new(), bank, sound_ty: None })
	}

	fn cam_key(&self) -> Option<scene::CameraKey> {
		let mut k = self.scene.camera_at(self.warp.tick as f32)?;
		k.tan_half_fov = k.tan_half_fov.max(self.last.proj_floor);
		Some(k)
	}

	/// Runs the whole ticks `dt` seconds make; true when the warp is over (a segment ended with `seg > 1`).
	fn frame(&mut self, dt: f32, mut gloss: Option<&mut crate::shipgloss::ShipGloss>) -> bool {
		self.acc += dt;
		while self.acc >= FLIGHT_DT {
			self.acc -= FLIGHT_DT;
			if self.tick(gloss.as_deref_mut()) {
				return true;
			}
		}
		false
	}

	fn tick(&mut self, gloss: Option<&mut crate::shipgloss::ShipGloss>) -> bool {
		let mut next = None;
		let bundle = &self.bundle;
		let t = self.warp.tick(|ty| match scene::warp_stream_from(bundle, ty) {
			Ok(s) => {
				let n = s.ticks;
				next = Some(s);
				n
			}
			Err(e) => {
				eprintln!("warp stream {ty}: {e}");
				0
			}
		});
		if t.done {
			return true;
		}
		if t.sound.is_some() {
			self.sound_ty = t.sound;
		}
		if let Some(s) = next {
			self.scene = s;
		}
		if t.new_segment {
			self.factor = 1.0;
		}
		self.posed.clear();
		self.ship_j = None;
		for (ai, actor) in self.scene.actors.iter().enumerate() {
			let c = if ai == 0 && actor.class_id == 0x215 { crate::game::transit::warp_ship_class(self.mk) } else { actor.class_id };
			let (Some(anim), Some((_, class, _))) = (self.anims.get(&c), self.assets.classes.iter().find(|k| k.0 == c)) else { continue };
			let mut kept = None;
			for it in 0..2 {
				let tk = self.warp.tick as f32 + 0.5 * it as f32;
				let (Some((ka, kb, f)), Some(pos)) = (actor.pair(tk), actor.pos_at(tk)) else { continue };
				let mats = anim.pose_frames(&ka.frame, &kb.frame, f);
				self.iter_factors[it] = self.factor;
				let (p1, p2) = (joint_point(pos, self.factor, &anim.joint_world(&mats, 1)), joint_point(pos, self.factor, &anim.joint_world(&mats, 2)));
				self.warp.push_history(p1, p2);
				if t.hidden {
					self.warp.count = 0;
				}
				if let Some(s) = t.shrink {
					self.factor = s;
				}
				kept = Some((mats, pos));
			}
			let Some((mats, pos)) = kept else { continue };
			let (mut ps, ns) = anim::skin(class, &mats);
			for p in &mut ps {
				*p = p.map(|x| x * self.factor);
			}
			self.posed.push((c, (ps, ns), pos));
			if ai == 0 {
				self.ship_j = Some((c, anim.joint_world(&mats, 0), pos));
			}
		}
		self.last = t;
		// SCN-4.2 Order 6: the ship gloss, under the ribbons' gate, once per tick for the slot-0 actor.
		self.gloss.clear();
		if let (true, Some(g), Some((class, j, pos)), Some(cam)) = (self.warp.ribbons_shown(), gloss, self.ship_j, self.scene.camera_at(self.warp.tick as f32)) {
			self.gloss = g.step(class, &j, pos, cam.pos, 6, true);
		}
		false
	}

	/// The fan, big quad, ribbons, banner and fade of the current tick (SCN-4.2); None without a camera key.
	fn frame_data(&self) -> Option<crate::game::transit::WarpFrame> {
		let k = self.cam_key()?;
		Some(self.warp.draw_frame(&self.last, k.tan_half_fov, self.assets.sky.meshes.len(), self.mk))
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

impl App {
	/// Opens the ship's Galactic Map (page 0xf) over the frozen game.
	pub(super) fn open_galactic(&mut self) {
		let cur = self.level;
		let Some(g) = &mut self.game else { return };
		if g.dead.is_some() || self.ui_text.is_none() || self.travel.is_some() || self.pause.is_some() {
			return;
		}
		// RAC_PLANETS=1,4,.. (for --shot): unlocks those planets first.
		if let Ok(v) = std::env::var("RAC_PLANETS") {
			for n in v.split(',').filter_map(|s| s.trim().parse::<usize>().ok()) {
				let count = g.story.unlock.iter().filter(|&&b| b != 0).count();
				if n < 20 && g.story.unlock[n] == 0 && count < 20 {
					g.story.unlock[n] = 1;
					g.story.order[count] = n as u32;
				}
			}
		}
		let mut planets = g.planets();
		if planets.is_empty() {
			planets.push(cur);
		}
		let mut travel = frontend::Travel { current: cur, planets, ..Default::default() };
		travel.unlock = g.story.unlock;
		for (d, s) in travel.visited.iter_mut().zip(g.story.visited.iter()) {
			*d = *s;
		}
		travel.missions = (0..19).map(|p| crate::missions::checklist(p, &g.story)).collect();
		travel.gold = g.story.gold.iter().map(|b| b.iter().filter(|&&x| x != 0).count() as u32).collect();
		let mut ig = self.ingame();
		ig.travel = Some(travel);
		if self.ui_sfx.is_empty() {
			if let Ok(mut d) = iso::Disc::open(std::path::Path::new(&self.iso_path)) {
				self.ui_sfx = global_sfx(&mut d).clips;
			}
		}
		self.pause = Some(frontend::Menu::galactic(self.settings.lang as usize, self.settings, ig));
		// Opened through FUN_00275cf8 like the pause menu: target == current page, cue 3.
		self.pause_cue(frontend::Cue::Open);
		self.wheel = None;
	}

	/// Starts the departure sequence for `to`; false when there is nothing to show (the same planet).
	pub(super) fn start_travel(&mut self, to: usize) -> bool {
		if self.travel.is_some() || self.gpu.is_none() {
			return false;
		}
		let from = self.level;
		let steps = match &self.game {
			Some(g) => steps(from, to, &g.story),
			None => return false,
		};
		if steps.is_empty() {
			return false;
		}
		let need: Vec<usize> = steps
			.iter()
			.filter_map(|s| if let Step::Card { first, second, .. } = s { Some([Some(*first + 1), second.map(|s| s + 1)]) } else { None })
			.flatten()
			.flatten()
			.chain(std::iter::once(0))
			.collect();
		let mut plates = HashMap::new();
		if need.len() > 1 {
			let lang = self.settings.lang as usize;
			let loaded = iso::Disc::open(std::path::Path::new(&self.iso_path)).map_err(|e| e.to_string()).and_then(|mut d| frontend::plates(&mut d, lang));
			match (loaded, &mut self.gpu) {
				(Ok(tex), Some(gpu)) => {
					for e in need {
						if let Some(t) = tex.get(e) {
							plates.insert(e, gpu.hud_texture(None, t));
						}
					}
				}
				(Err(e), _) => eprintln!("title cards: {e}"),
				_ => {}
			}
		}
		eprintln!("travel {from} -> {to}: {steps:?}");
		self.travel = Some(Travel { from, to, steps, i: 0, t: 0.0, begun: false, flight: None, warp: None, plates, movie: None, held: true, done: false });
		true
	}

	/// The level change the story asked for (pending_level): true while a travel sequence takes it over,
	/// false once the sequence has run (or there is none) and the level may change now.
	pub(super) fn begin_travel(&mut self, to: usize) -> bool {
		match &self.travel {
			Some(t) if t.done => {
				self.travel = None;
				false
			}
			Some(_) => true,
			None => self.game.is_some() && self.start_travel(to),
		}
	}

	/// Starts the takeoff (false) or landing stream; false when the level has none.
	pub(super) fn start_ship_stream(&mut self, landing: bool) -> bool {
		let id = if landing { LANDING } else { TAKEOFF };
		let Some((sc, clip)) = self.scenes.get(&id) else { return false };
		let (total, clip) = (sc.ticks as f32, clip.clone());
		let Some(g) = &mut self.game else { return false };
		let Some((prop, _, _)) = g.ship_pad() else { return false };
		g.props[prop].shown = false;
		let tick = if landing { LANDING_START } else { 0.0 };
		self.ship_play = Some(ShipPlay { id, tick, total, fade: 1.0,  prop,   });
		// One voice channel: the boarding/landing voice replaces any portrait or gadget voice and its pending cue,
		// and starts with the first stream tick (FUN_002a1de8 waits for the primed state, then FUN_00279790).
		self.portrait_pending = None;
		self.stop_ship_voice();
		if let Some(a) = &mut self.audio {
			for v in [self.portrait_voice.take(), self.gadget_voice.take()].into_iter().flatten() {
				a.stop(v);
			}
			self.ship_voice = clip.map(|c| a.play(&c, 1.0, 0.0));
		}
		// Boarding pauses the level music and stinger in place, no fade (FUN_00279d20(0)); the landing
		// keeps the hold from the takeoff.
		if !landing {
			if let Some(mp) = &mut self.music_player {
				mp.hold(true);
			}
		}
		true
	}

	/// Stops the ship voice (retail voice state 5 at a skip or the stream end).
	fn stop_ship_voice(&mut self) {
		if let (Some(id), Some(a)) = (self.ship_voice.take(), self.audio.as_mut()) {
			a.stop(id);
		}
	}

	/// Per frame, after the ticks: the end of a ship stream (stepped per tick by `ship_step`); at the end the
	/// takeoff opens the Galactic Map, the landing gives the game back.
	fn ship_frame(&mut self) {
		let Some(p) = self.ship_play.take() else { return };
		if p.tick < p.total {
			self.ship_play = Some(p);
			return;
		}
		self.stop_ship_voice();
		if let Some(pr) = self.game.as_mut().and_then(|g| g.props.get_mut(p.prop)) {
			pr.shown = true;
		}
		// The landing end resumes the music and stinger from where boarding froze them (FUN_00279d58).
		if p.id == LANDING {
			if let Some(mp) = &mut self.music_player {
				mp.hold(false);
			}
		}
		if p.id == TAKEOFF {
			self.open_galactic();
		}
	}

	/// Per frame: the ship pad plays the takeoff, then opens the map; a running travel sequence advances.
	pub(super) fn travel_frame(&mut self, dt: f32) {
		let open = self.game.as_mut().is_some_and(|g| std::mem::take(&mut g.depart));
		if open && self.ship_play.is_none() && self.flow.as_ref().map_or(true, |f| f.cutscene.is_none()) && !self.start_ship_stream(false) {
			self.open_galactic();
		}
		// RAC_SHIP_STREAM=t|l[:tick] (for --shot): the takeoff / landing once 50 play ticks have run, from that tick.
		static SHIP_TEST: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
		if self.play_ticks >= 50 && self.ship_play.is_none() {
			if let Some(v) = std::env::var("RAC_SHIP_STREAM").ok().filter(|_| !SHIP_TEST.swap(true, std::sync::atomic::Ordering::Relaxed)) {
				let (kind, from) = v.split_once(':').unwrap_or((&v, ""));
				if self.start_ship_stream(kind == "l") {
					if let (Some(p), Ok(t)) = (&mut self.ship_play, from.parse::<f32>()) {
						p.tick = t;
					}
				}
			}
		}
		if self.pause.is_none() {
			self.ship_frame();
		}
		let Some(mut tr) = self.travel.take() else { return };
		if tr.done {
			self.travel = Some(tr);
			return;
		}
		let held = [KeyCode::Space, KeyCode::KeyF, KeyCode::Enter].iter().any(|k| self.keys.contains(k));
		let skip = held && !tr.held;
		tr.held = held;
		let Some(step) = tr.steps.get(tr.i).copied() else {
			// Sequence over: visited[from] = 2 (not when leaving Umbris / Oltanis before their next planet is
			// known, FUN_002a6228), then the level change.
			if let Some(g) = &mut self.game {
				let s = &mut g.story;
				let keep = (tr.from == 7 && s.unlock[8] == 0) || (tr.from == 14 && s.unlock[15] == 0);
				if !keep {
					if let Some(v) = s.visited.get_mut(tr.from) {
						*v = 2;
					}
				}
			}
			tr.done = true;
			self.pending_level = Some(tr.to);
			self.travel = Some(tr);
			return;
		};
		let mut next = false;
		match step {
			Step::Warp => {
				// No skip: retail substate 4 has no pad read.
				if !tr.begun {
					tr.begun = true;
					let story = self.game.as_ref().map(|g| g.story.clone());
					let started = story.ok_or_else(|| "no game".to_string()).and_then(|s| {
						let mut d = iso::Disc::open(std::path::Path::new(&self.iso_path)).map_err(|e| e.to_string())?;
						WarpRun::new(tr.from, tr.to, &s, &mut d)
					});
					match started {
						Ok(w) => {
							if let Some(g) = &mut self.gpu {
								g.load_warp_actors(&w.assets);
								g.load_warp(&w.assets, &w.big, w.banner.as_ref());
							}
							tr.warp = Some(w);
						}
						Err(e) => {
							eprintln!("arrival warp: {e}");
							next = true;
						}
					}
				}
				if let Some(w) = &mut tr.warp {
					let done = w.frame(dt, self.ship_gloss.as_mut());
					if let (Some(ty), Some(a)) = (w.sound_ty.take(), self.audio.as_mut()) {
						let s = w.bank.as_ref().and_then(|b| b.sound_sample(ty).ok().flatten()).filter(|s| !s.pcm.is_empty());
						if let Some(s) = s {
							let clip = audio::Clip { rate: s.rate, pcm: std::sync::Arc::new(s.pcm), loop_start: s.loop_start, reverb: s.reverb };
							a.play_with(&clip, 1.0, 0.0, 1.0, false);
						}
					}
					if done {
						if let Some(g) = &mut self.gpu {
							g.clear_warp_actors();
							g.clear_warp();
						}
						tr.warp = None;
						next = true;
					} else if let Some(g) = &mut self.gpu {
						g.set_warp_actors(&w.posed, w.last.hidden, &[]);
						if let Some(f) = w.frame_data() {
							g.set_warp_frame(&f, &w.gloss);
						}
					}
				}
			}
			Step::Flight => {
				// No skip: retail substate 3 only reads the pad for the phase-2 fade trigger.
				if !tr.begun {
					tr.begun = true;
					tr.flight = None;
					if let Some(g) = &mut self.game {
						g.hero_hidden = true;
						let cams = self.flight_data.as_ref().is_some_and(|d| d.1.is_some());
						tr.flight = g.ship_pad().map(|(prop, _, _)| FlightRun::new((g.props[prop].class_id.wrapping_sub(0x213) as usize).min(2), prop, cams));
					}
					next = tr.flight.is_none();
				}
				if let (Some(fr), Some(g)) = (&mut tr.flight, &mut self.game) {
					let pad = [KeyCode::KeyT, KeyCode::Space].iter().any(|k| self.keys.contains(k));
					let empty: gameplay::FlightData = (None, None);
					let data = self.flight_data.as_ref().unwrap_or(&empty);
					fr.game_cam = g.camera;
					next = fr.advance(dt, pad, &mut g.props, data);
				}
			}
			Step::Card { ticks, .. } => {
				tr.t += dt;
				if let Some(n) = std::env::var("RAC_CARD_TICK").ok().and_then(|v| v.parse::<f32>().ok()) {
					tr.t = n / 50.0; // --shot: hold the card at tick n
				}
				next = tr.t * 50.0 >= ticks || skip;
			}
			Step::Movie(k) => {
				if !tr.begun {
					tr.begun = true;
					let started = iso::Disc::open(std::path::Path::new(&self.iso_path)).map_err(|e| e.to_string()).and_then(|mut d| scene::movie(&mut d, 31 + k as u32));
					match started {
						Ok(m) => {
							if let Some(a) = &mut self.audio {
								a.pause_all(true);
							}
							tr.movie = Some(fmv::player::Player::start(&self.iso_path, &m, self.settings.lang, self.audio.as_mut()));
						}
						Err(e) => {
							eprintln!("travel movie {k}: {e}");
							next = true;
						}
					}
				}
				if let Some(p) = &mut tr.movie {
					if let (Some(f), Some(g)) = (p.poll(), &mut self.gpu) {
						g.movie.upload(&g.device, &g.queue, &f);
					}
					next = p.finished || skip;
				}
				if next && tr.movie.take().is_some() {
					if let Some(a) = &mut self.audio {
						a.pause_all(false);
					}
					if let Some(g) = &mut self.gpu {
						g.movie.clear();
					}
				}
			}
		}
		if next {
			eprintln!("travel: {step:?} done");
			tr.i += 1;
			tr.t = 0.0;
			tr.begun = false;
			tr.flight = None;
			tr.warp = None;
		}
		self.travel = Some(tr);
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	#[test]
	fn sequences() {
		let mut s = save::Story::default();
		assert!(steps(1, 1, &s).is_empty());
		assert_eq!(steps(1, 2, &s), vec![Step::Flight, Step::Warp]);
		// Veldin -> Novalis, first time: cards, FMV 3, 4, card, FMV 5, arrival card.
		let v = steps(0, 1, &s);
		assert_eq!(v[0], Step::Flight);
		assert_eq!(v[v.len() - 2..], [Step::Card { first: 8, second: None, ticks: CARD }, Step::Warp]);
		assert_eq!(v.iter().filter(|x| matches!(x, Step::Movie(_))).count(), 3);
		s.visited[1] = 1;
		assert_eq!(steps(0, 1, &s), vec![Step::Flight, Step::Warp]);
		// Eudora's FMV only on the first visit.
		assert!(steps(1, 4, &s).contains(&Step::Movie(6)));
		s.visited[4] = 1;
		assert!(!steps(1, 4, &s).contains(&Step::Movie(6)));
		// Leaving Umbris needs Batalia unlocked.
		assert!(!steps(7, 3, &s).contains(&Step::Movie(7)));
		s.unlock[8] = 1;
		assert!(steps(7, 3, &s).contains(&Step::Movie(7)));
	}

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

	/// Ticks until the step is over, counting from 1.
	fn ticks_until_done(fr: &mut FlightRun, props: &mut [Prop], data: &gameplay::FlightData) -> u32 {
		for n in 1..2000 {
			if fr.tick(false, props, data) {
				return n;
			}
		}
		panic!("flight never ends");
	}

	fn warp_run(to: usize) -> WarpRun {
		let mut d = iso::Disc::open(std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../Ratchet & Clank (Europe) (En,Fr,De,Es,It) (v1.00).iso"))).unwrap();
		WarpRun::new(1, to, &save::Story::default(), &mut d).unwrap()
	}

	#[test]
	fn joint_point_scales_the_joint_translation() {
		let mut j = [0.0f32; 12];
		(j[3], j[7], j[11]) = (1.0, 2.0, 3.0);
		assert_eq!(joint_point([10.0, 0.0, 0.0], 0.5, &j), [10.5, 1.0, 1.5]);
	}

	#[test]
	fn warp_frames_run_whole_ticks() {
		// 50 frames of 0.02 s = 50 ticks: the history ring holds 32 entries, head (2 * 50) & 31 = 4 (one actor).
		let mut w = warp_run(5);
		for _ in 0..50 {
			assert!(!w.frame(0.02, None));
		}
		assert_eq!((w.warp.tick, w.warp.count, w.warp.head), (50, 32, 4));
		// 0.05 s on the first frame = 2 ticks, 0.01 s left.
		let mut w = warp_run(5);
		w.frame(0.05, None);
		assert_eq!(w.warp.tick, 2);
		assert!((w.acc - 0.01).abs() < 1e-6, "{}", w.acc);
		assert_eq!(w.posed.len(), 1);
	}

	#[test]
	fn quiet_warp_shrinks_inside_the_iteration_loop() {
		// to = 0: the type-4 segment only (end 148), the ship is hidden and the history stays empty.
		let mut w = warp_run(0);
		assert_eq!((w.warp.ty, w.warp.end), (4, 148));
		let k = f32::from_bits(0x3c92_4925);
		let mut seen = Vec::new();
		while w.warp.tick < 120 {
			assert!(!w.tick(None));
			assert!(w.last.hidden && w.warp.count == 0);
			if [92, 93, 120].contains(&w.warp.tick) {
				seen.push((w.warp.tick, w.iter_factors));
			}
		}
		assert_eq!(seen[0], (92, [1.0, 1.0]));
		assert_eq!(seen[1], (93, [1.0, 55.0 * k]));
		assert_eq!(seen[2], (120, [29.0 * k, 28.0 * k]));
		assert_eq!(seen[2].1[1], 0.5);
	}

	#[test]
	fn warp_frame_has_the_fan_fade_and_big_quad() {
		let mut w = warp_run(5);
		w.tick(None);
		let f = w.frame_data().unwrap();
		assert_eq!(f.fan.len(), 6);
		assert_eq!(f.fade, 96);
		assert!(f.fan_tan >= 0.63);
		assert_eq!(f.quads.iter().filter(|q| q.tex == crate::game::transit::WARP_BIG_TEX).count(), usize::from(w.warp.ty == 4));
		assert_eq!((w.big.w, w.big.h), (128, 128));
		assert!(w.assets.fx_textures.len() >= 2);
	}

	#[test]
	fn warp_camera_floors_the_projection_scalar() {
		let mut w = warp_run(0);
		w.tick(None);
		let k = w.cam_key().unwrap();
		assert!(k.tan_half_fov >= 0.63);
	}

	/// SHR-FRAME1 test d: the counter a level entry starts with.
	#[test]
	fn shr_frame1_arrival_frames() {
		let mut s = crate::save::Story::default();
		assert_eq!([0, 1, 2, 3, 0x14].map(|l| arrival_frames(l, &s)), [1, 1, 614, 614, 1]);
		s.unlock[3] = 1;
		assert_eq!(arrival_frames(1, &s), 614);
	}

	// FRAME-1e test 1: the ship stream steps once per 0.02 s tick, so its tick / fade sequence (and the skip) is the same
	// at any display rate.
}
