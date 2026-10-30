//! RedrawRequested, part 1: the frame's shared locals (`Frame`), the frame start (pause, cursor grab, window mode,
//! listener state) and the input phase (movies, camera input).

use super::*;

/// The locals of one RedrawRequested frame that live across steps.
#[derive(Default)]
pub(super) struct Frame {
	pub(super) dt: f32,
	pub(super) paused: bool,
	pub(super) wdt: f32,
	pub(super) sounds: Vec<(Option<usize>, f32, f32, i32, bool)>,
	pub(super) music_reqs: Vec<(i16, u8)>,
	pub(super) shots: Vec<([f32; 3], sound::Cue, Option<i32>, bool, Option<i32>)>,
	pub(super) loop_cues: Vec<(usize, Option<([f32; 3], sound::Cue)>, bool)>,
	pub(super) tick_wait_max: f32,
	pub(super) world_quads: Vec<particles::WorldQuad>,
	pub(super) at: Option<([f32; 3], f32)>,
	pub(super) skinned: Option<Posed>,
	pub(super) tilt: [f32; 2],
	pub(super) hero_blend_t: Option<f32>,
	pub(super) hero_gens: (u64, u64),
	pub(super) ghost_ranges: [Option<(usize, usize, u8)>; 5],
	pub(super) bomb_range: Option<(usize, usize)>,
	pub(super) light_at: Option<[f32; 3]>,
	pub(super) nozzle: Option<game::pyro::Nozzle>,
	pub(super) actors: Vec<Option<(Posed, [f32; 3])>>,
	pub(super) passenger_shown: bool,
	pub(super) eye_glows: Vec<([f32; 3], f32)>,
	pub(super) vendor_quads: Vec<particles::WorldQuad>,
	pub(super) fx_posed: HashMap<usize, std::sync::Arc<Posed>>,
	pub(super) flight_pose: Option<travel::FlightPose>,
	pub(super) title: String,
	pub(super) st: Option<HudState>,
	pub(super) drawn_cam: Option<Camera>,
	pub(super) native_sfx: bool,
	pub(super) music_paused: bool,
}

impl App {
	pub(super) fn frame_begin(&mut self) -> Frame {
		profile::frame_start();
		crate::gscap_guard::frame();
		// The run log's first frames (a freeze right after the load shows how far drawing got).
		static FRAMES: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
		if FRAMES.load(std::sync::atomic::Ordering::Relaxed) < 3 {
			let n = FRAMES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
			crate::loadlog::say(|| format!("host: frame {n} starts"));
		}
		if let Some(d) = crate::retail::test_slow_render() {
			std::thread::sleep(d);
		}
		let dt = test_clock::dt(test_clock::Site::Frame, || self.last.elapsed().as_secs_f32()).min(0.1);
		self.last = Instant::now();
		frame_clock::frame_started();
		self.pause_frame(dt);
		// The pause menu and the Quick Select wheel freeze the world.
		let paused = self.pause.is_some() || self.wheel.is_some() || self.travel.is_some();
		// World-side clocks (hero animation, camera rig, particles) stop with it; the HUD clock runs on.
		let wdt = if paused { 0.0 } else { dt };
		// Modern camera: grab and hide the cursor while playing.
		let grab = self.settings.cam_modern && self.shot.is_none() && self.focused && !paused && self.game.is_some() && self.rig.is_some();
		// Retail + modern camera (retail/modcam.rs): captured only while focused in gameplay (no menu, movie or scene); with Camera
		// Classic only while RMB holds the strafe aim.
		let grab = match &self.retail {
			Some(r) if crate::retail::modcam::enabled() => (crate::retail::modcam::on() || crate::retail::modcam::classic::aim_held()) && self.shot.is_none() && self.focused && !paused && self.movie.is_none() && r.gameplay(),
			_ => grab,
		};
		// [PC] DEV ONLY: the free camera (retail/devtools.rs) captures the cursor too.
		let grab = grab || (self.retail.is_some() && crate::retail::devtools::free_on() && self.shot.is_none() && self.focused && self.movie.is_none());
		// A mod asked for the cursor captured (OmniLoader `capture_mouse`, omni/input.rs): hidden and held in the window while it has the focus.
		let grab = grab || (self.retail.is_some() && crate::omni::input::capture_wanted() && self.shot.is_none() && self.focused && self.movie.is_none());
		if !grab && self.grabbed && self.retail.is_some() {
			crate::retail::modcam::ui_release();
		}
		if grab != self.grabbed {
			if let Some(g) = &self.gpu {
				use winit::window::CursorGrabMode;
				let ok = if grab { g.window.set_cursor_grab(CursorGrabMode::Confined).or_else(|_| g.window.set_cursor_grab(CursorGrabMode::Locked)).is_ok() } else { g.window.set_cursor_grab(CursorGrabMode::None).is_ok() };
				if ok || !grab {
					self.grabbed = grab;
				}
			}
		}
		// The OS cursor: hidden while captured, and for a pad / keyboard player; shown by real mouse use (cursor_vis.rs).
		if let Some(g) = &self.gpu {
			super::cursor_vis::apply(&g.window, grab, self.retail.is_some());
		}
		crate::omni::input::note_captured(self.grabbed);
		// Pause -> Options -> Graphics -> Display mode (retail/pcext.rs `fullscreen`): the window follows the menu value, also
		// the remembered one at start-up. An off-screen test window only logs the switch, so it never covers the desktop.
		if let (true, Some(g)) = (self.retail.is_some(), &self.gpu) {
			let want = crate::retail::pcext::fullscreen();
			if let Some(f) = display_change(want, self.fullscreen) {
				self.fullscreen = want;
				let skip = test_window();
				eprintln!("retail: display mode {}: set_fullscreen({f:?}){}", if want { "Fullscreen" } else { "Windowed" }, if skip { " skipped (test window)" } else { "" });
				if !skip {
					g.window.set_fullscreen(f);
				}
			}
		}
		// Listener velocity ring (60 Hz ticks, FUN_0029ff68) and the underwater flag (the camera water probe
		// FUN_0020e8f0, sound.rs `update_water`).
		// The listener velocity ring and its per-tick probe run in the in-tick audio stage (`audio_stage`, RNG-4b).
		let hero = self.game.as_ref();
		let scene_mode = self.flow.as_ref().is_some_and(|f| f.cutscene.is_some() || f.scene_time().is_some());
		self.listener.water_z = hero.and_then(|g| g.player.water).map_or(0.0, |w| w.z);
		if !paused {
			if let Some(g) = hero {
				let (solids, owners) = g.solids();
					self.listener.scene.set_solids_owned(solids, owners);
			}
			let input = sound::WaterInput {
				gameplay: !scene_mode,
				hero_under: hero.is_some_and(|g| g.player.underwater()),
				hero_z: hero.map_or(0.0, |g| g.player.pos[2]),
				water_z: hero.map(|g| g.player.water_level()),
			};
			let water = self.water.as_ref();
			self.listener.update_water_with(self.cam.pos, input, &|p| water.and_then(|w| w.surface_z(p[0], p[1], p[2])));
		}
		if let Some(a) = &mut self.audio {
			a.set_underwater(self.listener.underwater);
				a.set_scene(scene_mode);
		}
		// (clip, volume, pan, pitch shift, privileged voice owner)
		let mut sounds: Vec<(Option<usize>, f32, f32, i32, bool)> = Vec::new();
		// BRIDGE-86: actor music requests of this frame's game events, handed to the music player below.
		let mut music_reqs: Vec<(i16, u8)> = Vec::new();
			// Event-site one-shots kept live afterwards (position, cue, drawn random, held click, ARI-27b fixed level).
			let mut shots: Vec<([f32; 3], sound::Cue, Option<i32>, bool, Option<i32>)> = Vec::new();
			// NOV-6.3: (owner, cue, on) of this frame's SoundLoop events.
			let mut loop_cues: Vec<(usize, Option<([f32; 3], sound::Cue)>, bool)> = Vec::new();
		Frame { dt, paused, wdt, sounds, music_reqs, shots, loop_cues, tick_wait_max: f32::MAX, ..Default::default() }
	}

	pub(super) fn frame_input(&mut self, f: &mut Frame) {
		let (dt, paused) = (f.dt, f.paused);
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
			let key = |k| self.keys.contains(&k) as i32 as f32;
			let input = Input {
				jump: self.keys.contains(&KeyCode::Space),
				// L1/L2 (pad mask 5): hero state 1, which puts the camera in free-look.
				attack: self.keys.contains(&KeyCode::KeyF) || self.attack_mouse || self.attack_script.iter().any(|&t| (t..t + 3).contains(&self.play_ticks)),
				// Circle: the weapon in hand (G or the middle mouse button).
			};
			// Fixed 50 Hz steps, as the PAL game runs.
			self.accum = if paused {
				0.0
			} else if let Some(a) = crate::retail::devtools::field_hold(crate::retail::disc::field_dt()) {
				// [PC] DEV ONLY: the free camera's freeze / single step (retail/devtools.rs).
				a
			} else {
				(self.accum + dt).min(0.25)
			};
			// [PC] The frame interval sets how far the worker may run ahead (retail.rs `Retail::ahead`).
			if let Some(r) = &mut self.retail {
				r.set_frame_ms(dt * 1000.0);
			}
			// The time this frame's ticks may wait for the worker in all (ms) before the rest waits for the next frame.
			let mut tick_wait_max = f32::MAX;
			// One field's time in ms (PAL 20, USA 16.7): the worker is busy when its field takes longer than that.
			let field_ms = crate::retail::disc::field_dt() * 1000.0;
			// Retail menus (the pause menu and other frozen game modes): the GS capture paces the interpreter at
			// ~35 ms a field there, slower than real time, so catching up only stacked five blocking fields into each
			// frame (5 fps, keys sampled every ~200 ms). One field per frame there.
			if self.retail.as_ref().is_some_and(|r| r.paused() && r.ms > 20.0) {
				self.accum = self.accum.min(crate::retail::disc::field_dt());
			} else if self.retail.as_ref().is_some_and(|r| r.ms_avg > field_ms) {
				// The worker is the bottleneck all along (its field takes longer than a field on average: 20 ms PAL, 16.7 ms
				// USA): every tick blocks until the worker's field is done, so a second blocking tick per frame only halves the
				// frames shown (17 fps at 27 ms a field) and adds no field. The frame stops ticking after its first tick that waited (`tick_wait`), but the
				// time owed stays (up to 4 fields): capping it to one field per frame ran the game at the frame rate, slow
				// motion on a slow GPU (31 fields/s at 31 fps, 2026-10-17 slow-PC imitation), and lost the worker's spare time.
				self.accum = self.accum.min(4.0 * crate::retail::disc::field_dt());
				tick_wait_max = 1.0;
			} else if self.retail.as_ref().is_some_and(|r| r.ms > field_ms) {
				// Gameplay on a busy host (the EE field over a field's time): the 0.25 s catch-up stacked five blocking fields
				// into every frame (8 fps at 24 ms a field, 2026-10-12). A frame waits at most one field's time for the
				// worker; the time owed stays (up to 4 fields), so a heavy field is caught up over the next frames.
				self.accum = self.accum.min(4.0 * crate::retail::disc::field_dt());
				tick_wait_max = field_ms;
			}
			// [PC] TEST ONLY: RAC_TEST_UNPACED runs exactly one field per frame, whatever the frame time (viewer/test_clock.rs).
			if test_clock::unpaced() && !paused {
				self.accum = if test_clock::sub_ticks(|| self.shot.is_none() || self.retail.as_ref().is_some_and(|r| r.shot_field_wait())) { crate::retail::disc::field_dt() } else { 0.0 };
			}
			let skip = input.jump || input.attack;
			let mut skip_press = skip && !self.skip_held;
			self.skip_held = skip;
			// FMV: a movie scene streams from the disc (fmv/player.rs); its end acts as a skip press.
			{
				let movie_id = self.flow.as_ref().and_then(|f| f.scene_time()).map(|s| s.0).filter(|id| id & 0x4000 != 0);
				if movie_id.is_some() && self.movie.is_none() {
					let started = iso::Disc::open(std::path::Path::new(&self.iso_path)).map_err(|e| e.to_string()).and_then(|mut d| scene::movie(&mut d, movie_id.unwrap()));
					match started {
						Ok(m) => {
							if let Some(a) = &mut self.audio {
								a.pause_all(true);
							}
							self.movie = Some(fmv::player::Player::start(&self.iso_path, &m, self.settings.lang, self.audio.as_mut()));
						}
						Err(e) => {
							eprintln!("movie: {e}");
							skip_press = true;
						}
					}
				} else if movie_id.is_none() && !crate::retail::fmv_bridge::waiting() && self.movie.take().is_some() {
					if let Some(a) = &mut self.audio {
						a.pause_all(false);
					}
					if let Some(g) = &mut self.gpu {
						g.movie.clear();
					}
				}
				if let Some(p) = &mut self.movie {
					if let (Some(f), Some(g)) = (p.poll(), &mut self.gpu) {
						g.movie.upload(&g.device, &g.queue, &f);
					}
					skip_press |= p.finished;
				}
				// Retail movies (retail/fmv_bridge.rs): the game thread waits in the player hook until the movie
				// ends or is skipped, then returns to the game as retail's player does. Retail's own skip rule
				// (fmv_bridge `Skip`, the words its player reads): the front-end mirror polls the same one.
				if let (Some(r), true) = (&mut self.retail, self.movie.is_some() && crate::retail::fmv_bridge::waiting()) {
					let keys = &self.keys;
					let skip = r.movie_pressed(|k| keys.contains(&k));
					if skip || self.movie.as_ref().is_some_and(|p| p.finished) {
						self.movie = None;
						// Retail keeps the movie's last picture up until the main loop presents again (Retail::holds,
						// no-present word 0x15f5b8): the presenter stays on until then (cleared after `r.sync`).
						MOVIE_HOLD.store(true, std::sync::atomic::Ordering::Relaxed);
						if let Some(a) = &mut self.audio {
							a.pause_all(false);
						}
						eprintln!("retail: movie {} at field {} play tick {}", if skip { "skipped" } else { "finished" }, r.field_count(), self.play_ticks);
						r.end_movie();
					}
				}
			}
			// A grant nobody started last frame is dropped (the voice was refused or its clip is missing).
			self.voices.pending.clear();
			let sstage = SoundStage {   ..Default::default() };
			// FRAME-1c: the camera's per-frame input (keys / mouse, PC modes); the rig steps once per sim tick below.
			let mouse = std::mem::take(&mut self.mouse_dx);
			let mouse_y = std::mem::take(&mut self.mouse_dy);
			let zoom = std::mem::take(&mut self.zoom_acc);
			// Q/E stand in for the right stick, R for R1; a right-drag turns by the mouse.
			let ticks = (dt / camera::TICK).max(1.0);
			// Rotation speed Slow/Medium/Fast scales the stick (ingame_menu_leftovers.md P1-d).
			let mut rotate = if paused { 0.0 } else { (key(KeyCode::KeyQ) - key(KeyCode::KeyE)) * self.settings.rotate_rate() / camera::ROTATE_RATE };
			if self.looking && !self.settings.cam_modern {
				rotate -= mouse * 0.003 / camera::ROTATE_RATE / ticks;
			}
			// Modern (port extra): the mouse orbits (moving right turns the view right,
			// moving down looks down), the wheel zooms; Reversed settings flip the axes.
			let sx = if self.settings.cam_lr { 1.0 } else { -1.0 };
			let sy = if self.settings.cam_ud { 1.0 } else { -1.0 };
			let (look, zoom) = if paused || !self.grabbed { ([0.0; 2], 0.0) } else { ([-mouse * 0.003 * sx, mouse_y * 0.003 * sy], zoom) };
			// Z/X stand in for the right stick Y. Left/Right and Up/Down Reversed flip the stick axes.
			let tilt = if paused { 0.0 } else { key(KeyCode::KeyZ) - key(KeyCode::KeyX) };
			let cam_in = camera::CamInput {      modern: self.settings.cam_modern, look, zoom, ..Default::default() };
			if let Some(rig) = &mut self.rig {
				rig.frame_input(game.player.pos, game.player.yaw, &cam_in);
			}
			self.tick_cam_live = false;
			self.smooth.ticks = 0;
			profile::lap(profile::Phase::Input);
			f.tick_wait_max = tick_wait_max;
		}
	}
}
