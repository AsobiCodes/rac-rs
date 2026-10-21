use super::*;

/// A retail movie ended while retail presents no picture yet (`Retail::holds`): its last picture stays on screen.
static MOVIE_HOLD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

use super::herocache::{hero_cache_key, hero_check_compare, hero_check_on, hero_check_frame, next_gen, HeroCache};

/// Most frames the world stays black after a level switch (`App::arrival_black`).
const ARRIVAL_BLACK_MAX: u32 = 30;

impl ApplicationHandler for App {
	fn resumed(&mut self, el: &ActiveEventLoop) {
		if self.gpu.is_some() {
			return;
		}
		let mut attrs = Window::default_attributes().with_title(crate::app_name()).with_inner_size(winit::dpi::LogicalSize::new(1280, 720)).with_visible(!self.headless);
		// Window and taskbar icon: the logo, 64x64 raw RGBA embedded in the exe (assets/icon/rac-rs-64.rgba).
		if let Ok(icon) = winit::window::Icon::from_rgba(include_bytes!("../../assets/icon/rac-rs-64.rgba").to_vec(), 64, 64) {
			attrs = attrs.with_window_icon(Some(icon));
		}
		// Agent test runs (RAC_TEST_WINDOW=hidden, or the smoke check's RAC_RETAIL_SMOKE): the window is created off-screen, not activated and not in the taskbar, so
		// it never shows on the user's desktop while the swapchain keeps presenting. Without those variables the window is the normal one.
		if test_window() {
			attrs = attrs.with_active(false).with_position(winit::dpi::PhysicalPosition::new(-32000, -32000));
			// RAC_TEST_SIZE=WIDTHxHEIGHT (pixels): the test window's size (proof shots at other resolutions than 1280x720).
			if let Some((w, h)) = std::env::var("RAC_TEST_SIZE").ok().and_then(|v| v.split_once('x').and_then(|(a, b)| Some((a.trim().parse::<u32>().ok()?, b.trim().parse::<u32>().ok()?)))) {
				attrs = attrs.with_inner_size(winit::dpi::PhysicalSize::new(w, h));
			}
			#[cfg(windows)]
			{
				use winit::platform::windows::WindowAttributesExtWindows;
				attrs = attrs.with_skip_taskbar(true);
			}
		}
		let window = Arc::new(el.create_window(attrs).expect("window"));
		self.gpu = Some(Gpu::new(window));
		self.audio = if self.headless { None } else { audio::Audio::new() };
		if self.audio.is_none() {
			eprintln!("no audio output; running silent");
		}
		self.load_level();
		// The front end starts play itself (New Game / Load Game).
		if self.autoplay && self.front.is_none() {
			self.toggle_play();
		}
		self.last = Instant::now();
	}

	fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
		if let DeviceEvent::MouseMotion { delta } = event {
			crate::retail::prompts::note_mouse_motion(delta.0, delta.1);
			// [PC] DEV ONLY (RAC_DEV_TOOLS=1, retail/devtools.rs): the free camera takes the mouse while the window has the focus.
			if self.focused && self.retail.is_some() && crate::retail::devtools::ui_mouse(delta.0 as f32, delta.1 as f32) {
				return;
			}
			if self.grabbed {
				if self.retail.is_some() {
					crate::retail::modcam::ui_mouse(delta.0 as f32, delta.1 as f32);
				}
				self.mouse_dx += delta.0 as f32;
				self.mouse_dy += delta.1 as f32;
			} else if self.looking {
				self.cam.yaw -= delta.0 as f32 * 0.003;
				self.mouse_dx += delta.0 as f32;
				self.cam.pitch = (self.cam.pitch - delta.1 as f32 * 0.003).clamp(-1.55, 1.55);
			}
		}
	}

	fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
		// Button prompts follow the device: a key or mouse button press, the wheel or clear mouse movement make the keyboard the device (retail/prompts.rs).
		crate::retail::prompts::note_window_event(&event);
		match event {
			WindowEvent::CloseRequested => el.exit(),
			WindowEvent::Resized(s) => {
				if let Some(g) = &mut self.gpu {
					g.resize(s.width, s.height);
				}
			}
			WindowEvent::Focused(f) => {
				self.focused = f;
				// Retail play: a key released while another window had the focus never reports its release here, so it
				// would stay held in every recorded field. Losing the focus lets go of every key and mouse button.
				if !f && crate::retail::active() {
					self.keys.clear();
					self.attack_mouse = false;
					self.fire_mouse = false;
					self.quick_mouse = false;
					crate::retail::modcam::ui_release();
				}
			}
			WindowEvent::CursorMoved { position, .. } => {
				self.cursor = Some((position.x as f32, position.y as f32));
				// Retail page menus (retail/mouse_menu.rs): the cursor as fractions of the client area.
				if let (Some(r), Some(g)) = (self.retail.as_mut(), self.gpu.as_ref()) {
					r.mouse_cursor(position.x as f32 / g.config.width.max(1) as f32, position.y as f32 / g.config.height.max(1) as f32);
				}
			}
			// Retail page menus: the wheel steps the list (retail/mouse_menu.rs).
			WindowEvent::MouseWheel { delta, .. } if self.retail.as_ref().is_some_and(|r| r.mouse_menu_active()) => {
				use winit::event::MouseScrollDelta;
				let y = match delta {
					MouseScrollDelta::LineDelta(_, y) => y,
					MouseScrollDelta::PixelDelta(p) => p.y as f32,
				};
				if let Some(r) = self.retail.as_mut() {
					r.mouse_wheel(y);
				}
			}
			WindowEvent::MouseWheel { delta, .. } => {
				use winit::event::MouseScrollDelta;
				let lines = match delta {
					MouseScrollDelta::LineDelta(_, y) => y,
					MouseScrollDelta::PixelDelta(p) => p.y as f32 / 80.0,
				};
				// [PC] DEV ONLY: the free camera's speed (retail/devtools.rs).
				if self.retail.is_some() && crate::retail::devtools::ui_wheel(lines) {
					return;
				}
				self.zoom_acc -= lines * 0.75;
				if self.retail.is_some() && self.grabbed {
					crate::retail::modcam::ui_zoom(lines);
				}
			}
			// Retail page menus: right click = the screen's back button, left click = select + Cross (the fire button is off).
			WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Right, .. } if self.retail.as_ref().is_some_and(|r| r.mouse_menu_active()) => {
				if let Some(r) = self.retail.as_mut() {
					r.mouse_back();
				}
			}
			WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } if self.retail.as_ref().is_some_and(|r| r.mouse_menu_active()) => {
				self.fire_mouse = false;
				if let Some(r) = self.retail.as_mut() {
					r.mouse_click();
				}
			}
			// Retail + modern camera (retail/modcam.rs): RMB held aims.
			WindowEvent::MouseInput { state, button: MouseButton::Right, .. } if self.retail.is_some() && crate::retail::modcam::on() && !crate::retail::devtools::free_on() => crate::retail::modcam::ui_aim(state == ElementState::Pressed),
			WindowEvent::MouseInput { state, button: MouseButton::Right, .. } => self.looking = state == ElementState::Pressed,
			// Play: left / middle click (see `App::play_mouse`); in retail a click shorter than a frame still reaches the game.
			WindowEvent::MouseInput { state, button: b @ (MouseButton::Left | MouseButton::Middle), .. } => self.play_mouse((b == MouseButton::Middle) as usize, state == ElementState::Pressed, true),
			WindowEvent::KeyboardInput { event, .. } => {
				let PhysicalKey::Code(k) = event.physical_key else { return };
				// [PC] DEV ONLY (RAC_DEV_TOOLS=1, retail/devtools.rs): F1 planet list (its keys go nowhere else while it is open),
				// F2 free camera and its own keys. Off: nothing is taken.
				if self.retail.is_some() && crate::retail::devtools::ui_key(k, event.state == ElementState::Pressed, event.repeat) {
					return;
				}
				// Held keys are tracked everywhere (the Sound page steps volume while Left/Right is held).
				self.key_change(k, event.state == ElementState::Pressed, event.repeat);
				// Retail + modern camera: F7 toggles it (retail/modcam.rs).
				if k == KeyCode::F7 && event.state == ElementState::Pressed && !event.repeat && self.retail.is_some() && crate::retail::modcam::enabled() {
					crate::retail::modcam::ui_toggle();
				}
				// [PC] PREVIEW (viewer/vegfx.rs): F4 hides / shows the extra plants while RAC_VEG is on.
				if k == KeyCode::F4 && event.state == ElementState::Pressed && !event.repeat && super::vegfx::cfg().env.is_some_and(|v| v > 0) {
					super::vegfx::toggle();
				}
				// [PC] Frame smoothing test key (RAC_SMOOTH_KEY=1, test launchers only): F9 flips it for this session (interp.rs).
				if event.state == ElementState::Pressed && !event.repeat && self.retail.is_some() {
					super::interp::key(k, crate::retail::pcext::graphics().smoothing);
				}
				// Dev debug keys (RAC_DEBUG_KEYS=1, retail/debugkeys.rs): F5 immortality, F6 infinite ammo, F8 +10000 bolts.
				if event.state == ElementState::Pressed && !event.repeat && self.retail.is_some() && crate::retail::debugkeys::enabled() {
					// [PC] F3: draw distance Low / the Graphics row's level (viewer/fardraw.rs), for before / after.
					if k == KeyCode::F3 {
						super::fardraw::toggle();
					}
					crate::retail::debugkeys::ui_key(k);
				}
			}
			WindowEvent::RedrawRequested => {
				profile::frame_start();
				if let Some(d) = crate::retail::test_slow_render() {
					std::thread::sleep(d);
				}
				let dt = self.last.elapsed().as_secs_f32().min(0.1);
				self.last = Instant::now();
				self.pause_frame(dt);
				// The pause menu and the Quick Select wheel freeze the world.
				let paused = self.pause.is_some() || self.wheel.is_some() || self.travel.is_some();
				// World-side clocks (hero animation, camera rig, particles) stop with it; the HUD clock runs on.
				let wdt = if paused { 0.0 } else { dt };
				// Modern camera: grab and hide the cursor while playing.
				let grab = self.settings.cam_modern && self.shot.is_none() && self.focused && !paused && self.game.is_some() && self.rig.is_some();
				// Retail + modern camera (retail/modcam.rs): captured only while focused in gameplay (no menu, movie or scene).
				let grab = match &self.retail {
					Some(r) if crate::retail::modcam::enabled() => crate::retail::modcam::on() && self.shot.is_none() && self.focused && !paused && self.movie.is_none() && r.gameplay(),
					_ => grab,
				};
				// [PC] DEV ONLY: the free camera (retail/devtools.rs) captures the cursor too.
				let grab = grab || (self.retail.is_some() && crate::retail::devtools::free_on() && self.shot.is_none() && self.focused && self.movie.is_none());
				if !grab && self.grabbed && self.retail.is_some() {
					crate::retail::modcam::ui_release();
				}
				if grab != self.grabbed {
					if let Some(g) = &self.gpu {
						use winit::window::CursorGrabMode;
						let ok = if grab { g.window.set_cursor_grab(CursorGrabMode::Confined).or_else(|_| g.window.set_cursor_grab(CursorGrabMode::Locked)).is_ok() } else { g.window.set_cursor_grab(CursorGrabMode::None).is_ok() };
						g.window.set_cursor_visible(!grab);
						if ok || !grab {
							self.grabbed = grab;
						}
					}
				}
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
					let skip = input.jump || input.attack;
					// RAC_SHOT_SKIP: a long --shot skips cutscenes so the capture shows play.
					let mut skip_press = (skip && !self.skip_held) || (self.shot.as_ref().is_some_and(|s| s.1 > 5) && std::env::var_os("RAC_SHOT_SKIP").is_some());
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
					let mut tick_wait = 0.0f32;
					let mut frame_ticks = 0u32;
					while self.accum >= crate::retail::disc::field_dt() {
						// [PC] Only the frame's first tick may wait for the game thread; a later one runs only when its field is already
						// done, else it waits for the next frame (the time stays owed). Waiting for every owed field drew a picture every
						// second field when the thread is slower than real time (laptop Kerwan, 22 ms a field: 23 fps; now one picture per
						// field). Shots keep the old catch-up so their fields stay the same.
						if frame_ticks > 0 && self.shot.is_none() && self.retail.as_mut().is_some_and(|r| !r.field_ready()) {
							break;
						}
						frame_ticks += 1;
						self.accum -= crate::retail::disc::field_dt();
						self.smooth.begin_tick(game);
						// Retail mode: the interpreter runs this tick and its moby table / camera are mirrored (retail.rs).
						if let Some(r) = &mut self.retail {
							// The held keys plus the taps since the last field (Retail::press_key).
							let keys = &self.keys;
							r.tick_keys(|k| keys.contains(&k), self.fire_mouse, self.quick_mouse);
							// The game asked for a movie (retail/fmv_bridge.rs): stream it from the disc in the game's own language
							// (its value is the PSS audio track: 0 En, 2 Fr, 3 De, 4 Es, 5 It); drawn over the frame until it ends.
							// The retail front end (logos, title, attract and intro movies) is the mirror's: it takes its movies
							// in `front_mirror_tick`. While the front end shows in RAM but the mirror is not up yet (a slow
							// start-up), the request stays in `Retail` and the game thread stays parked in the player hook.
							if let Some(q) = if r.front_ram().is_some() { None } else { r.take_movie() } {
								let m = scene::Movie {   lba: q.lba, bytes: q.bytes as u64,   fps: crate::retail::disc::movie_fps(q.lba), };
								let lang = [0u32, 2, 3, 4, 5].iter().position(|&t| t == q.lang).unwrap_or(0) as u8;
								eprintln!("retail: movie lba {} bytes {} lang {} at field {} play tick {}", q.lba, q.bytes, q.lang, r.field_count(), self.play_ticks);
								if let Some(a) = &mut self.audio {
									a.pause_all(true);
								}
								// A movie that follows another one at once (Veldin's ending: the flagship movie right after the
								// short one) ends the held last picture's hold: the old fade no longer tints the presenter (its
								// first frame resets the tint, `Presenter::upload`), else the new movie played all black.
								MOVIE_HOLD.store(false, std::sync::atomic::Ordering::Relaxed);
								self.movie = Some(fmv::player::Player::start(&self.iso_path, &m, lang, self.audio.as_mut()));
							}
							if let Some(g) = &self.gpu {
								r.aspect = g.config.width as f32 / g.config.height.max(1) as f32;
							}
							r.sync(game);
							// [PC] Frame smoothing (interp.rs): this field's draw lists and its cut verdict (nothing kept while it is Off).
							let smooth_menu = if crate::retail::pcext::enabled() { super::interp::wanted(crate::retail::pcext::graphics().smoothing) } else { self.settings.smooth };
							super::interp::field(r, smooth_menu);
							if (!r.holds() || r.overlay_covers() || r.menu_presents() || r.gs_front_drops_movie()) && MOVIE_HOLD.swap(false, std::sync::atomic::Ordering::Relaxed) {
								if let Some(g) = &mut self.gpu {
									g.movie.clear();
								}
							}
							// The movie-end fade darkens the held movie picture (retail draws it into the display buffer).
							if let (true, Some(t), Some(g)) = (MOVIE_HOLD.load(std::sync::atomic::Ordering::Relaxed), r.movie_tint(), &mut self.gpu) {
								g.movie.tint = t;
							}
							if MOVIE_HOLD.load(std::sync::atomic::Ordering::Relaxed) && std::env::var_os("RAC_SCENE_LOG").is_some() {
								eprintln!("retail: movie hold field {} holds {} covers {} tint {:?}", r.field_count(), r.holds(), r.overlay_covers(), r.movie_tint());
							}
							// Scene actors play sequences retail appended to their classes (retail.rs `scene_seqs`).
							r.scene_seqs(&mut self.prop_anims, self.hero_anim.as_mut());
							// Mode 6 takeoff / landing stream: retail's ship gloss (canopy) quads of this field.
							self.gloss_quads.clear();
							self.gloss_quads.extend_from_slice(r.ship_gloss());
							// Mode 6 substate 4: the arrival warp from retail's state (travel/mirror_warp.rs).
							travel::mirror_warp::field(r, self.gpu.as_mut(), &self.iso_path);
							// The interpreter stopped (a crash or a spin): say so in the title bar, the frozen frame shows nothing.
							if let (Some(e), Some(g)) = (r.take_stop(), &self.gpu) {
								g.window.set_title(&format!("{}: RETAIL STOPPED: {e}", crate::app_name()));
							}
							// The first mirror is the savestate's own values, not a change the HUD should pop up for.
							if self.play_ticks == 0 {
								self.hud_seen = (game.bolts, game.health);
							}
							// 989snd: the exact driver's SPU2 register writes of the field play on the SPU2 voice mixer (retail/snd.rs).
							let ev = r.take_sound();
							if let Some(a) = &mut self.audio {
								crate::retail::snd::field(a, ev);
							}
							if let Some(c) = r.hero_clock() {
								c.apply(&mut self.hero_rig.hero);
							}
							// M3a: the held wrench, the back objects d0 / d4 and the loose wrenches play retail's clocks; with no held
							// wrench the rig keeps its last pose (the held-item hide rules cover the hand).
							if let Some(parts) = r.hero_parts() {
								// F5: retail's +0x60 layer list (gadget / weapon hold arms) poses the hero in place of the port's layers.
								self.hero_rig.hero.layers = parts.layers.clone();
								// Modern controls: the run / walk cycle plays backwards for a backpedal (retail/strafe.rs); a fade that
								// started from the reversed pose fades from the rig's own snapshot of it, not from retail's forward one.
								// The frame count of the hero's own clip (the rig clock carries no sequences: `frame_count_of` there
								// answered the 30-frame fallback, so the 23-frame run reversed past its end and held its last frame).
								let seq = self.hero_rig.hero.clock.new_seq as usize;
								let n = self.hero_anim.as_ref().and_then(|(_, a)| a.sequences.get(seq)).map_or(0, |s| s.frames.len());
								let from_rev = crate::retail::strafe::draw_clock(&mut self.hero_rig.hero.clock, n, parts.strafe.rev);
								// Retail's snapshot FROM frame (hero +0x68) while the hero clock fades.
								let joints = self.hero_anim.as_ref().map_or(0, |(_, a)| a.skeleton.parents.len());
								self.hero_rig.hero.from_ram = Some(&parts.from_frame).filter(|b| !from_rev && !b.is_empty() && joints > 0).and_then(|b| anim::parse_frame(b, 0, joints).ok());
								if let Some(w) = parts.wrench {
									w.clock.apply(&mut self.hero_rig.wrench);
								}
								if let Some(c) = parts.d0 {
									c.apply(&mut self.hero_rig.back);
								}
								if let Some(c) = parts.d4 {
									c.apply(&mut self.hero_rig.hand);
								}
								// Modern controls: the run clip flipped forward <-> reversed (its phase mirrors, the legs swap) or the leg
								// twist jumped past strafe.rs's rate limit (Strafe aiming switched off mid-twist): this tick's pose is drawn
								// unblended (frame smoothing, smooth.rs `hero_cut`).
								if self.retail_parts.as_ref().is_some_and(|o| o.strafe.rev != parts.strafe.rev || (o.strafe.twist - parts.strafe.twist).abs() > 0.6) {
									self.smooth.hero_cut = true;
								}
								self.loose_anims.retain(|a, _| parts.loose.iter().any(|l| l.addr == *a));
								for l in &parts.loose {
									l.clock.apply(self.loose_anims.entry(l.addr).or_insert_with(|| anim::PropAnim::new(l.clock.new_seq as usize)));
								}
								self.retail_parts = Some(parts);
							}
							// A menu field the GS capture needs 38 ms for: no catch-up burst (up to 12 blocking ticks in one redraw would
							// freeze the window); the menu just runs at the capture's pace.
							if r.paused() && r.ms > 20.0 {
								self.accum = 0.0;
							}
							self.popup = None;
							let c = r.camera(1.0);
							// Level frame counter 0x15f5ac (= `Game.frames`): the ocean texture blend, the sky shell spin, the waterfall and
							// Clank's pulse are keyed on it; the old `Game::ambient` that counted it does not run here (retail/oldsim.rs).
							if let Some(f) = r.fx().map(|f| f.frames) {
								game.frames = f;
							}
							// Retail's particle pass is not run in this branch: the class-760 fire emitters (visible ones) and the class-809
							// UV scroll advance once per retail tick here.
							if !r.paused() {
								self.particles.tick_scroll();
								let view = self.gpu.as_ref().and_then(|g| g.scene.as_ref().map(|_| dynlight::View::from_matrix(&view_proj(&Camera { pos: c.0, yaw: c.1, pitch: c.2, tan_half: c.3, up: r.camera_up(1.0) }, g.config.width as f32 / g.config.height.max(1) as f32, RETAIL_NEAR, RETAIL_FAR))));
								for f in &mut self.fires {
									if view.as_ref().is_none_or(|v| f.visible(v)) {
										f.tick();
									}
								}
							}
							tick_render_state(&mut self.gpu, game, (c.0, c.1, c.2, c.3 * PROJ_Y_RATIO), r.camera_up(1.0));
							self.play_ticks += 1;
							tick_wait += r.waited;
							if tick_wait > tick_wait_max {
								break;
							}
							continue;
						}
					}
					profile::lap(profile::Phase::Ticks);
					// PC-FPS-1: draw between the last two ticks (off, paused or capturing: the newest tick as is).
					// [PC] Frame smoothing: retail mode follows Graphics -> Frame smoothing (interp.rs; settings.txt `smooth` with RAC_PC_EXT=0),
					// and blends nothing on a cut field.
					let menu = if self.retail.is_some() && crate::retail::pcext::enabled() { super::interp::wanted(crate::retail::pcext::graphics().smoothing) } else { self.settings.smooth };
					let want = menu && !paused && (self.shot.is_none() || { static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *S.get_or_init(|| std::env::var_os("RAC_SMOOTH_SHOT").is_some()) });
					let on = super::interp::frame(self.retail.as_mut(), want, (self.accum / crate::retail::disc::field_dt()).clamp(0.0, 1.0));
					self.smooth.frame(on, self.accum);
					// Blob shadows (decomp/notes/shadows.md); the hero's is off while a cutscene poses him.
					let in_scene = self.flow.as_ref().is_some_and(|f| f.cutscene.is_some());
					// Retail mode: retail's own blob queue (retail.rs `blob_queue`), not the casters of the mirrored props.
					let mut world_quads = match self.retail.as_ref().and_then(|r| r.fx()) {
						Some(fx) => fx.blobs.iter().map(|&(p, s, n)| crate::shadows::blob_square(p, s, n)).collect(),
						None => { let _s = profile::span("shadows"); crate::shadows::blobs(w, &[], self.cam.pos) }
					};
					// NOV-14.5d: the ship streams' ground shadows (substate 3's flight fade is not played: None).
					if let (Some(p), Some((pad_prop, pad_pos, pad_yaw))) = (self.ship_play.as_ref(), game.ship_pad()) {
						if let (Some((sc, _)), Some(pad_ship)) = (self.scenes.get(&p.id), game.props.get(pad_prop)) {
							let tick = p.tick / scene::TICKS_PER_SECOND * scene::TICKS_PER_SECOND;
							let ship = pad_ship.class_id.wrapping_sub(0x213) as usize;
							world_quads.extend(ship_shadow_quads(p.id, sc, tick, (pad_pos, pad_yaw), ship, &self.actor_slots, &self.prop_anims, w, None));
						}
					}
					// SCN-4.3e / SCN-4.3d: the departure flight's ship shadow (pad ship with the override byte set, near the
					// camera) and the two seat-edge ribbons.
					if let Some(tr) = &self.travel {
						if let (Some(prop), Some(f), Some((cpos, crot))) = (tr.flight_shadow_prop(), tr.flight_state(), tr.flight_cam()) {
							if let (Some(pr), Some(((class, a), _))) = (game.props.get(prop), game.props.get(prop).and_then(|p| self.prop_anims.get(&p.class_id)).map(|x| (x, ()))) {
								let k = scene::CameraKey { pos: cpos, rot: crot, tan_half_fov: camera::PROJ_SCALAR, cut: false };
								let (fwd, up) = (k.forward(), k.up());
								let right = [fwd[1] * up[2] - fwd[2] * up[1], fwd[2] * up[0] - fwd[0] * up[2], fwd[0] * up[1] - fwd[1] * up[0]];
								let sphere = a.sequences.get(1).map_or([0.0; 4], |s| s.sphere);
								world_quads.extend(flight_shadow_quads(w, pr.pos, pr.rot, f.ship_type(), sphere, class.scale, f.c as i32 - 1, (cpos, fwd, right, up, k.tan_half_fov)));
							}
						}
						if let Some(f) = tr.flight_state().filter(|f| f.ribbon_live && !f.exit) {
							world_quads.extend(game::transit::ribbon_quads(&f.ribbon, f.head, f.count, f.ship_type(), 0x38, 0x13));
						}
					}
					if let Some(f) = &mut self.flow {
						f.help_text = self.settings.help_text && std::env::var_os("RAC_NO_HELP").is_none(); // RAC_NO_HELP: no HelpDesk box (shots)
						for id in f.requested.drain(..) {
							self.help_log.add(id);
						}
					}
					if let Some(mv) = &mut self.map {
						let p = game.player.pos;
						mv.hero = match mv.affine {
							Some(a) => map::affine(a, [p[0], p[1]]),
							None => map::world_to_map(mv.lo, mv.hi, [p[0], p[1]]),
						};
						mv.yaw = game.player.yaw;
						mv.explored.stamp(mv.hero[0], mv.hero[1]);
					}
					// BRIDGE-24: the sound requests resolved at their request points in the ticks (voice reserved, range drawn).
					let reqs = std::mem::take(&mut game.sound_requests);
					let drawn = |src: game::SoundSource, i: usize| reqs.iter().find(|q| q.kind == src && q.index == i).map(|q| q.random);
					{
						let pl = &game.player;
						let swing = pl.swing.map(|s| s.0);
						let ev = &self.sfx.ev;
						let mut push = |(c, v): (Option<usize>, f32), pan: f32, sh: i32, own: bool| sounds.push((c, v, pan, sh, own));
						// AUD-11d: jump, landing and hurt sounds are the hero sequence triggers (played below), not edges.
						let mut bolt = None;
						for (i, pr) in game.props.iter().enumerate() {
							if self.prev_shown.get(i).copied().unwrap_or(false) && !pr.shown && (game::BOLT..game::BOLT + 4).contains(&pr.class_id) {
								bolt = bolt.or(ev.bolt.get(&pr.class_id));
							}
						}
						if bolt.is_some() {
							push(cue(bolt), 0.0, 0, true);
						}
						// PAR-BRIDGE-B: crate breaks and robot deaths are made per tick in `emit_events`.
						self.prev_shown = game.props.iter().map(|p| p.shown).collect();
						self.prev_swing = swing;
						// AUD-11g: footsteps are animation-frame triggers (Player::footstep_scan); the surface class picks the
						// def (class * 4 + foot * 2, variant 0: the Magneboots variant is not obtainable), water on level 1 / 0x12
						// counts as class 3. Drained also while dead.
						for (fi, (class, water, foot)) in std::mem::take(&mut game.player.foot_cues).into_iter().enumerate() {
							let class = if water && matches!(game.level, 1 | 0x12) { 3 } else { class };
							let fc = self.sfx.ev.footsteps.get(class as usize * 4 + foot as usize * 2).and_then(|c| c.as_ref());
							let (c, vol) = cue(fc);
							let (v, pan, sh) = spatial_rand(&self.cam, game.player.pos, fc, &self.listener, None, drawn(game::SoundSource::Foot, fi));
							sounds.push((c, vol * v, pan, sh, true));
						}
					}
					for m in game.log.drain(..) {
						eprintln!("{m}");
						if self.retail.is_none() {
							self.messages.push((m, 3.0));
						}
					}
					// Bomb Glove events: sounds, the blast's particles and camera shake.
					let flat = |(c, v): (Option<usize>, f32)| (c, v, 0.0, 0, true);
					let mut voices = Vec::new();
					game.spawned.clear();
					for (ei, e) in std::mem::take(&mut game.events).into_iter().enumerate() {
						let g = &self.sfx.ev.glove;
						match e {
							game::Event::Throw(at) => {
								let (v, pan, sh) = spatial_rand(&self.cam, at, g.throw.as_ref(), &self.listener, None, drawn(game::SoundSource::Event, ei));
								let (c, vol) = cue(g.throw.as_ref());
								sounds.push((c, vol * v, pan, sh, false));
							}
							game::Event::Empty => sounds.push(flat(cue(g.empty.as_ref()))),
							game::Event::Draw => sounds.push(flat(cue(g.draw.as_ref()))),
							game::Event::Pickup(_) => sounds.push(flat(cue(g.pickup.as_ref()))),
							// NOV-5.8a: FUN_002a10a0(1, 0, 0) plays the def flat (position zeroed, no owner).
							game::Event::GlobalCue(1) => sounds.push(flat(cue(self.sfx.ev.global1.as_ref()))),
							game::Event::GlobalCue(_) => {}
							game::Event::Music { track, cue } => music_reqs.push((track, cue)),
							game::Event::WrenchThrow(at) => {
								let (v, pan, sh) = spatial_rand(&self.cam, at, self.sfx.ev.wrench_throw.as_ref(), &self.listener, None, drawn(game::SoundSource::Event, ei));
								let (c, vol) = cue(self.sfx.ev.wrench_throw.as_ref());
								sounds.push((c, vol * v, pan, sh, false));
							}
							game::Event::WrenchCatch => sounds.push(flat(cue(self.sfx.ev.wrench_catch.as_ref()))),
							game::Event::WrenchImpact { pos, cs } => {
								let (v, pan, sh) = spatial_rand(&self.cam, pos, self.sfx.ev.wrench_impact.get(cs).and_then(|c| c.as_ref()), &self.listener, None, drawn(game::SoundSource::Event, ei));
								let (c, vol) = cue(self.sfx.ev.wrench_impact.get(cs).and_then(|c| c.as_ref()));
								sounds.push((c, vol * v, pan, sh, false));
							}
							game::Event::Death => sounds.push(flat(cue(self.sfx.ev.death.as_ref()))),
							// AUD-11d: swing and slam sounds are hero sequence triggers (s3..5 / slam seqs), played from the clock.
							game::Event::Swing(_) | game::Event::SlamCue(_) => {}
							game::Event::SlamHit { sound: false, .. } => {}
							game::Event::SlamHit { pos, .. } => {
								let (v, pan, sh) = spatial_rand(&self.cam, pos, self.sfx.ev.wrench_impact[2].as_ref(), &self.listener, None, drawn(game::SoundSource::Event, ei));
								let (c, vol) = cue(self.sfx.ev.wrench_impact[2].as_ref());
								sounds.push((c, vol * v, pan, sh, false));
							}
							// Bolt collected: its sparkles were made in the tick (`emit_events`).
							game::Event::BoltCollect(_) => {}
							game::Event::Nanotech(at) => {
								let (v, pan, sh) = spatial_rand(&self.cam, at, self.sfx.ev.nanotech.as_ref(), &self.listener, None, drawn(game::SoundSource::Event, ei));
								let (c, vol) = cue(self.sfx.ev.nanotech.as_ref());
								sounds.push((c, vol * v, pan, sh, false));
							}
							game::Event::Blast { pos, big, gold } => {
								let (v, pan, sh) = spatial_rand(&self.cam, pos, if big { g.big.as_ref() } else { g.small[gold as usize].as_ref() }, &self.listener, None, drawn(game::SoundSource::Event, ei));
								let (c, vol) = cue(if big { g.big.as_ref() } else { g.small[gold as usize].as_ref() });
								sounds.push((c, vol * v.max(0.25), pan, sh, false));
							}
							game::Event::JetDown { .. } => { /* NOV-5.jets-D: cue 1 is invalid for the jet classes, no sound */ }
							// WPN-5.1c: the victim explosion has no cue.
							game::Event::SuckBlast { .. } => {}
				game::Event::Sound { class, cs, pos } => {
								// The toad's direct calls (cs0 hit callback, cs7 landing) live in its enemy table.
								let table = self.sfx.ev.class.get(&(class, cs)).or_else(|| self.sfx.ev.enemy_cs.get(&(class, cs))).or_else(|| if class == 0 { self.sfx.ev.hero_cs.get(&cs) } else { None });
								shots.extend(table.map(|c| (pos, *c, drawn(game::SoundSource::Event, ei), false, None)));
							}
							// ARI-27b: a flags-0x10 start (the Aridia launcher boom): same table, fixed level.
							game::Event::SoundLevel { class, cs, pos, level } => {
								let table = self.sfx.ev.class.get(&(class, cs)).or_else(|| self.sfx.ev.enemy_cs.get(&(class, cs)));
								shots.extend(table.map(|c| (pos, *c, drawn(game::SoundSource::Event, ei), false, Some(level))));
							}
							// BRIDGE-50: the held click plays like `Sound`; its voice id is kept for the liveness report.
							game::Event::HeldSound { class, cs, pos } => {
								let table = self.sfx.ev.class.get(&(class, cs)).or_else(|| self.sfx.ev.enemy_cs.get(&(class, cs)));
								shots.extend(table.map(|c| (pos, *c, drawn(game::SoundSource::Event, ei), true, None)));
							}
							// WPN-6.9a: the Taunter cut stops the held voice (a start still queued this frame never plays).
							game::Event::HeldStop => {
								shots.retain(|s| !s.3);
								if let Some(a) = self.audio.as_mut() {
									a.stop(self.held_click);
								}
								self.held_click = 0;
							}
							game::Event::Shake { amount, secs } => self.shake.a.write(amount, ((secs / 0.02).round() as i32).max(1)),
							// NOV-20.2/20.5: the splash was made in the tick (`emit_events`).
							game::Event::Splash(_) | game::Event::Ring(..) => {}
							// Break particles and debris were made in the tick (`emit_events`).
							game::Event::Broke { .. } => {}
							// PAR-BRIDGE-B: the crate sound when its stamp allowed it; the particles were made in the tick.
							game::Event::CrateBreak { class, pos, sound: true, .. } => {
								if let Some(c) = self.sfx.ev.crate_break.get(&class) {
									let (v, pan, sh) = spatial_rand(&self.cam, pos, Some(c), &self.listener, None, drawn(game::SoundSource::Event, ei));
									sounds.push((Some(c.sound), c.volume * v, pan, sh, false));
								}
							}
							game::Event::CrateBreak { .. } | game::Event::RobotDeath { .. } => {}
							game::Event::RockCue { .. } => {} // FRAME-1c: fed to the rig in the tick that raised it
							game::Event::Voice(id) => voices.push(id),
							// BRIDGE-49: the bend the game writes into a loop voice slot each tick.
							game::Event::LoopBend { owner, bend } => {
								self.loop_bend.insert(owner, bend);
							}
							game::Event::SoundLoop { class, cs, pos, owner, on } => loop_cues.push((owner, on.then(|| self.sfx.ev.class.get(&(class, cs)).or_else(|| if class == 0 { self.sfx.ev.hero_cs.get(&cs) } else { None }).map(|c| (pos, *c))).flatten(), on)),
						}
					}
					// NOV-13.5: the voice stream (TOC +0x1a0 + 8 * (id - 10000), a plain VAG, dry, unity volume) and the
					// gadget cinematic's pack, read from the disc now (the hero and gadget joint counts parse its frames).
					// BRIDGE-37 (d): the portrait voice has its own slot; its end sets `voice_ended` (retail IOP callback, state 3).
					game.voice_lang = self.settings.lang;
					let sounding = |au: &Option<audio::Audio>, id: Option<u64>| id.is_some_and(|id| au.as_ref().is_some_and(|a| a.playing(id)));
					if self.portrait_voice.is_some() && !sounding(&self.audio, self.portrait_voice) {
						self.portrait_voice = None;
						game.voice_ended = true;
					}
					if std::mem::take(&mut game.portrait_stop) {
						if let (Some(id), Some(a)) = (self.portrait_voice.take(), self.audio.as_mut()) {
							a.stop(id);
						}
						self.portrait_pending = None;
					}
					for id in voices {
						if id != game::gadget_scene::FANFARE_VOICE {
							self.portrait_pending = Some(id);
							continue;
						}
						let Ok(mut d) = iso::Disc::open(std::path::Path::new(&self.iso_path)) else { continue };
						let joints = [self.hero_anim.as_ref().map_or(0, |h| h.1.skeleton.parents.len()), self.pyro_rig.as_ref().map_or(0, |r| r.held.anim.skeleton.parents.len())];
						if let Err(e) = game.load_gadget_pack(&mut d, game::gadget_scene::PAL, joints) {
							eprintln!("gadget scene: {e}");
						}
						let clip = crate::toc::read_toc(&mut d).ok().and_then(|t| {
							let o = 0x1a0 + 8 * (id as usize).checked_sub(10000)?;
							let lba = crate::bytes::u32_at(&t, o).ok()?;
							crate::music::read_vag(&mut d, lba as u64, 0).map_err(|e| eprintln!("voice {id}: {e}")).ok()
						});
						self.gadget_voice = clip.and_then(|m| self.audio.as_mut().map(|a| a.play(&audio::Clip { rate: m.rate, pcm: std::sync::Arc::new(m.pcm), loop_start: None, reverb: false }, 1.0, 0.0)));
					}
					// Retail FUN_00279fa0 plays only while no voice stream is sounding (`0x151720 == 0`): otherwise the id is held.
					if let Some(id) = self.portrait_pending.filter(|_| !sounding(&self.audio, self.gadget_voice) && !sounding(&self.audio, self.portrait_voice)) {
						self.portrait_pending = None;
						let clip = iso::Disc::open(std::path::Path::new(&self.iso_path)).ok().and_then(|mut d| {
							let t = crate::toc::read_toc(&mut d).ok()?;
							let lba = crate::bytes::u32_at(&t, 0x1a0 + 8 * (id as usize).checked_sub(10000)?).ok()?;
							crate::music::read_vag(&mut d, lba as u64, 0).map_err(|e| eprintln!("voice {id}: {e}")).ok()
						});
						self.portrait_voice = clip.and_then(|m| self.audio.as_mut().map(|a| a.play(&audio::Clip { rate: m.rate, pcm: std::sync::Arc::new(m.pcm), loop_start: None, reverb: false }, 1.0, 0.0)));
						if self.portrait_voice.is_none() {
							game.voice_ended = true;
						}
					}
					// NOV-20.3: class-787 drop first contacts play class 787 sound 0 at the drop.
					if let Some(wt) = &mut self.water {
						for pos in wt.sounds.drain(..) {
							shots.extend(self.sfx.ev.class.get(&(crate::water::DROP_CLASS, 0)).map(|c| (pos, *c, None, false, None)));
						}
					}
					let p = &game.player;
					// FRAME-1: the newest camera tick (no display interpolation) for the per-tick glint cull.
					self.glint_cam = None;
					self.glint_up = [0.0, 0.0, 1.0];
					if let Some(rig) = &mut self.rig {
						// FRAME-1c: the rig stepped once per sim tick above; the display view lies between its last two ticks
						// like the hero and props (the newest tick when not smoothing or paused).
						let v = rig.view(if self.smooth.active { self.smooth.alpha } else { 1.0 });
						let d = [v.target[0] - v.pos[0], v.target[1] - v.pos[1], v.target[2] - v.pos[2]];
						let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
						self.cam.pos = v.pos;
						self.cam.yaw = d[1].atan2(d[0]);
						self.cam.pitch = (d[2] / l).clamp(-1.0, 1.0).asin();
						self.cam.tan_half = v.proj_scalar * PROJ_Y_RATIO;
						// BRIDGE-56: the remote camera's roll (Z for every other view).
						self.cam.up = rig.up();
						let t = rig.tick_view();
						let d = [t.target[0] - t.pos[0], t.target[1] - t.pos[1], t.target[2] - t.pos[2]];
						let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-6);
						self.glint_cam = Some((t.pos, d[1].atan2(d[0]), (d[2] / l).clamp(-1.0, 1.0).asin(), t.proj_scalar * PROJ_Y_RATIO));
						self.glint_up = rig.up();
					} else {
						self.orbit.yaw = self.cam.yaw;
						self.orbit.pitch = self.cam.pitch;
						if !self.looking {
							self.orbit.follow(p.yaw, length2(p.vel), wdt);
						}
						self.cam.yaw = self.orbit.yaw;
						let (eye, _, _) = self.orbit.eye(w, p.pos);
						self.cam.pos = eye;
						self.cam.tan_half = TAN_HALF_FOV;
						self.cam.up = [0.0, 0.0, 1.0];
					}
					self.rig_hero = p.pos;
					// RAC_CAM="dx dy dz yaw pitch": a debug camera offset from the hero (for --shot checks).
					if let Some(v) = std::env::var("RAC_CAM").ok().map(|v| v.split_whitespace().filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 5) {
						self.cam.pos = [p.pos[0] + v[0], p.pos[1] + v[1], p.pos[2] + v[2]];
						self.cam.yaw = v[3];
						self.cam.pitch = v[4];
						self.cam.up = [0.0, 0.0, 1.0];
						self.glint_cam = None;
						self.over_cam = None;
					}
					if let Some(r) = &self.retail {
						let c = travel::mirror_warp::camera(r).unwrap_or_else(|| r.camera(if self.smooth.active { self.smooth.alpha } else { 1.0 }));
						let c = (c.0, c.1, c.2, c.3 * PROJ_Y_RATIO);
						// Retail's view keeps the camera's roll (up row of its basis, 0x1673d0 + 0x20): upright Ratchet, the world turns
						// around him on magnetic walls. Zero roll everywhere else, so the view is unchanged there.
						let warp_cam = travel::mirror_warp::camera(r).is_some();
						let up = if warp_cam { [0.0, 0.0, 1.0] } else { r.camera_up(if self.smooth.active { self.smooth.alpha } else { 1.0 }) };
						(self.cam.pos, self.cam.yaw, self.cam.pitch, self.cam.tan_half, self.cam.up) = (c.0, c.1, c.2, c.3, up);
						self.glint_cam = Some(c);
						self.glint_up = if warp_cam { [0.0, 0.0, 1.0] } else { r.camera_up(1.0) };
					}
					// [PC] lane scratch_veg debug: RAC_VEG_CAM="x y z yaw pitch" an absolute world camera (vegetation checks).
					if let Some(v) = std::env::var("RAC_VEG_CAM").ok().map(|v| v.split_whitespace().filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<_>>()).filter(|v| v.len() == 5) {
						(self.cam.pos, self.cam.yaw, self.cam.pitch, self.cam.up) = ([v[0], v[1], v[2]], v[3], v[4], [0.0, 0.0, 1.0]);
					}
					// Camera shake (retail and modern alike): the offset is added to the final render position only.
					let so = self.shake.offset(self.cam.yaw, self.cam.pitch);
					self.cam.pos = [self.cam.pos[0] + so[0], self.cam.pos[1] + so[1], self.cam.pos[2] + so[2]];
					if let Some(c) = &mut self.glint_cam {
						let s = self.shake.offset(c.1, c.2);
						c.0 = [c.0[0] + s[0], c.0[1] + s[1], c.0[2] + s[2]];
					}
					// Flicker while invulnerable. A hit death plays seq 16 then 69 into the fade (game.death_anim);
					// a drowned hero keeps the drowning pose; a fade-only death (pit tumble) is hidden.
					let death = game.death_anim();
					let dying = game.dead.is_some() && !p.drowning() && death.is_none();
					let blink = game.invulnerable > 0 && (game.invulnerable as f32 * 0.02 * 10.0) as i32 % 2 == 1;
					let at = (!dying && !blink).then_some((p.pos, p.yaw));
					self.clock += dt;
					// Death cues are the class 0 triggers of the death sequence (69 hit death, 113, 17): the hero clock's cue scan
					// feeds them like the enemies' (decomp/notes/sound.md).
					// AUD-11d: every hero sequence trigger plays (class 0 sound, one-shot), the first crossed one per tick.
					for id in self.hero_rig.hero.take_cues().into_iter().take(1) {
						if let Some(c) = self.sfx.ev.hero_cs.get(&(id as usize)) {
							sounds.push((Some(c.sound), c.volume, 0.0, 0, true));
						}
					}
					profile::lap(profile::Phase::Post);
					let gloved = game.inv.equipped == crate::inventory::BOMB_GLOVE;
					let bomb_in_hand = game.bomb_in_hand();
					// The held Pyrocitor's pose and nozzle (game/pyro.rs); the wrench is put away while it is held.
					let (pyro_pose, nozzle) = (game.pyro_pose(), game.pyro_nozzle());
					let suck_pose = game.suck_pose();
					let held_pose = game.held_pose();
					let mut held_muzzle_model = None;
					let mut held_xform_model = None;
					let mut held_joint0_model = None;
					let mut held_rows = None;
					let mut muzzle_model = None;
					// PC perf (speed lane A, B1): everything below up to `skinned` is a function of the game state, which only changes in
					// a tick. A display frame with no tick and an equal key reuses the last frame's posed bundle (hero, glove, held items,
					// back objects, wrench, ghosts) and the side values derived with it; the per-frame blend stays in `smooth.hero`.
					// RAC_HERO_CHECK=1: always computes fresh, and compares it with what would have been reused.
					let hero_key = hero_cache_key(self.hero_anim.as_ref().map_or(0, |x| x as *const _ as usize), self.level, self.play_ticks, self.retail.as_ref().map(|r| r.field_count()), [self.hero_rig.hero.layers.len(), self.loose_anims.len(), self.held_rigs.len()], self.retail_parts.is_some() as u64 | (self.wrench_rig.is_some() as u64) << 1, game);
					let hero_cand = self.smooth.ticks == 0 && self.hero_cache.as_ref().is_some_and(|c| c.key == hero_key);
					let hero_check = hero_check_on();
					hero_check_frame();
					let hero_hit = hero_cand && !hero_check;
					let wrench_now = self.wrench_rig.as_ref().filter(|_| !hero_hit).map(|r| anim::skin(&r.class, &self.hero_rig.wrench.pose(&r.anim, &[])));
					// M3a, retail mode: retail's own parts. The hand wrench shows only while retail's held wrench is attached; slot 0
					// then draws the thrown (detached) held wrench and the two ghost slots the other loose class-71 mobys (afterimages),
					// each posed by its own clock at its retail +0x10 / rows.
					let rparts = self.retail.as_ref().and(self.retail_parts.as_ref());
					let rparts_fresh = rparts.filter(|_| !hero_hit);
					let r_hand = rparts.map(|q| q.wrench.is_some_and(|w| w.attached));
					let r_held = rparts.and_then(|q| q.wrench.map(|w| w.addr));
					let loose_skin = |l: &crate::retail::LooseWrench| {
						self.wrench_rig.as_ref().zip(self.loose_anims.get(&l.addr)).map(|(r, pa)| (*l, anim::skin(&r.class, &pa.pose(&r.anim, &[]))))
					};
					let r_thrown = rparts_fresh.filter(|_| r_hand == Some(false)).and_then(|q| q.loose.iter().find(|l| Some(l.addr) == r_held)).and_then(|l| loose_skin(l));
					let r_ghosts: Vec<_> = rparts_fresh.map_or(Vec::new(), |q| q.loose.iter().filter(|l| Some(l.addr) != r_held).take(2).filter_map(|l| loose_skin(l)).collect());
					// A thrown wrench holds its flight pose, sequence 6 (OV:30778).
					let wrench_fly = self.wrench_rig.as_ref().filter(|_| !hero_hit).filter(|r| r.anim.sequences.get(6).is_some_and(|s| !s.frames.is_empty())).map(|r| anim::skin(&r.class, &r.anim.pose_frame(6, 0)));
					// The two translucent wrench afterimages in `skinned`, then the three hero afterimages (BRIDGE-40):
					// (first vertex, end, alpha byte) (BRIDGE-28).
					let mut ghost_ranges: [Option<(usize, usize, u8)>; 5] = [None; 5];
					let mut bomb_range: Option<(usize, usize)> = None;
					let mut light_at: Option<[f32; 3]> = None;
					let mut hero_gen = 0u64;
					let skinned = if hero_hit {
						let c = self.hero_cache.as_ref().expect("hero_hit needs the cache");
						hero_gen = c.pose_gen;
						ghost_ranges = c.ghost_ranges;
						bomb_range = c.bomb_range;
						light_at = c.light_at;
						held_rows = c.held_rows;
						muzzle_model = c.muzzle_model;
						held_muzzle_model = c.held_muzzle_model;
						held_joint0_model = c.held_joint0_model;
						held_xform_model = c.held_xform_model;
						c.skinned.clone()
					} else { self.hero_anim.as_ref().map(|(class, a)| {
						let (hero_fx, throw_pose, layer1_pose, hold_poses) = hero_layers(rparts, &game.player, &a.slot_map);
						let mut mats = self.hero_rig.hero.pose_held(a, &hero_fx, throw_pose.as_ref(), layer1_pose.as_ref(), &hold_poses);
						// Modern controls (retail/strafe.rs): the legs twist toward the ground path while aiming; off = zero twist.
						if let Some(q) = rparts {
							crate::retail::strafe::twist_mats(&mut mats, a, q.strafe.twist);
						}
						// HELD-GLOVE: a glove in hand (FUN_0020d018) is posed from the hero's own live local joints, not its clock.
						let hand_copy = crate::inventory::hand_pose_copy(game.inv.equipped);
						let hero_local = if hand_copy { self.hero_rig.hero.local_held(a, &hero_fx, throw_pose.as_ref(), layer1_pose.as_ref(), &hold_poses) } else { Vec::new() };
						let (mut ps, mut ns) = anim::skin(class, &mats);
						// HERO-16: the limb pulse squashes the hero's x and y axes; held items follow below.
						let pulse = p.pulse_scale;
						let hero_pt = [0.0, 0.0, crate::player::HERO_POINT_Z];
						if let Some(s) = pulse {
							for v in ps.iter_mut() {
								v[0] *= s;
								v[1] *= s;
							}
						}
						if let Some((hp, hn)) = &self.wrench {
							// A retail loose wrench in world space -> Ratchet's model space.
							let hr = hero_rows(p);
							let loose_at = |l: &crate::retail::LooseWrench, v: [f32; 3], t: f32| {
								let w = [0, 1, 2].map(|k| l.scale * (v[0] * l.rows[0][k] + v[1] * l.rows[1][k] + v[2] * l.rows[2][k]) + t * (l.pos[k] - p.pos[k]));
								hero_to_model(&hr, w)
							};
							if r_hand == Some(false) {
								match &r_thrown {
									Some((l, (fp, fnn))) => {
										ps.extend(fp.iter().map(|&v| loose_at(l, v, 1.0)));
										ns.extend(fnn.iter().map(|&v| loose_at(l, v, 0.0)));
									}
									None => hide(&mut ps, &mut ns, hp.len()),
								}
							} else if let Some(w) = &p.wrench {
								// Thrown: spinning about z at its flight position, taken into Ratchet's model space.
								let hr = hero_rows(p);
								let (sc, ss) = (w.spin.cos(), w.spin.sin());
								let local = |v: [f32; 3], t: f32| {
									let r = [v[0] * sc - v[1] * ss, v[0] * ss + v[1] * sc, v[2]];
									let d = [r[0] + t * (w.pos[0] - p.pos[0]), r[1] + t * (w.pos[1] - p.pos[1]), r[2] + t * (w.pos[2] - p.pos[2])];
									hero_to_model(&hr, d)
								};
								let (fp, fnn) = wrench_fly.as_ref().map_or((hp, hn), |(a, b)| (a, b));
								ps.extend(fp.iter().map(|&v| local(v, 1.0)));
								ns.extend(fnn.iter().map(|&v| local(v, 0.0)));
							} else if gloved || pyro_pose.is_some() || suck_pose.is_some() || held_pose.is_some() || game.held_hidden || r_hand == Some(false) {
								hide(&mut ps, &mut ns, hp.len());
							} else {
								let wj = norm_cols(a.joint_world(&mats, WRIST));
								let wj = pulse.map_or(wj, |s| crate::player::pulse_item(wj, s, true, hero_pt));
								let (wp, wn) = wrench_now.as_ref().map_or((hp, hn), |(a, b)| (a, b));
								ps.extend(wp.iter().map(|&v| apply(&wj, v, 1.0)));
								ns.extend(wn.iter().map(|&v| apply(&wj, v, 0.0)));
							}
							// HERO-AFTERIMAGE-15: two more wrench copies at the trail entries, translucent; hidden when absent.
							let ghosts = if rparts.is_some() { [None, None] } else { p.wrench.as_ref().map_or([None, None], |w| w.ghosts()) };
							for (gi, g) in ghosts.into_iter().enumerate() {
								if let Some((l, (fp, fnn))) = r_ghosts.get(gi) {
									let base = ps.len();
									ps.extend(fp.iter().map(|&v| loose_at(l, v, 1.0)));
									ns.extend(fnn.iter().map(|&v| loose_at(l, v, 0.0)));
									ghost_ranges[gi] = (l.alpha < 0x80).then_some((base, ps.len(), l.alpha));
									continue;
								}
								let base = ps.len();
								let Some((gpos, gspin, alpha)) = g else {
									hide(&mut ps, &mut ns, hp.len());
									continue;
								};
								let hr = hero_rows(p);
								let (sc, ss) = (gspin.cos(), gspin.sin());
								let local = |v: [f32; 3], t: f32| {
									let r = [v[0] * sc - v[1] * ss, v[0] * ss + v[1] * sc, v[2]];
									let d = [r[0] + t * (gpos[0] - p.pos[0]), r[1] + t * (gpos[1] - p.pos[1]), r[2] + t * (gpos[2] - p.pos[2])];
									hero_to_model(&hr, d)
								};
								let (fp, fnn) = wrench_fly.as_ref().map_or((hp, hn), |(a, b)| (a, b));
								ps.extend(fp.iter().map(|&v| local(v, 1.0)));
								ns.extend(fnn.iter().map(|&v| local(v, 0.0)));
								ghost_ranges[gi] = Some((base, ps.len(), alpha));
							}
						}
						// The glove at joint 54 (attach point 6, axes not normalised), posed by the hero hand copy (HELD-GLOVE,
						// decomp/notes/bomb_glove.md); its bomb at glove joint 2.
						let bomb_n = self.bomb.as_ref().map_or(0, |b| b.0.len());
						match &self.glove {
							Some(g) if gloved && mats.len() > GLOVE_JOINT => {
								let gj = a.joint_world(&mats, GLOVE_JOINT);
								let gj = pulse.map_or(gj, |s| crate::player::pulse_item(gj, s, false, hero_pt));
								let gm = anim::hand_copy_pose(&g.anim, &hero_local);
								let (mut gp, mut gn) = anim::skin(&g.class, &gm);
								let n = g.class.mesh.positions.len();
								gp.resize(n, [0.0; 3]);
								gn.resize(n, [0.0, 0.0, 1.0]);
								ps.extend(gp.iter().map(|&v| apply(&gj, v, 1.0)));
								ns.extend(gn.iter().map(|&v| apply(&gj, v, 0.0)));
								match &self.bomb {
									Some((bp, bn)) if bomb_in_hand && gm.len() > GLOVE_BOMB_JOINT => {
										let bj = g.anim.joint_world(&gm, GLOVE_BOMB_JOINT);
										let o = GLOVE_BOMB_OFFSET;
										bomb_range = Some((ps.len(), ps.len() + bp.len()));
										ps.extend(bp.iter().map(|&v| apply(&gj, apply(&bj, [v[0] + o[0], v[1] + o[1], v[2] + o[2]], 1.0), 1.0)));
										ns.extend(bn.iter().map(|&v| apply(&gj, apply(&bj, v, 0.0), 0.0)));
									}
									_ => hide(&mut ps, &mut ns, bomb_n),
								}
							}
							g => hide(&mut ps, &mut ns, g.as_ref().map_or(0, |g| g.class.mesh.positions.len()) + bomb_n),
						}
						// The Pyrocitor at its hero joint, posed by its own sequence; the nozzle at the weapon's attach point
						// 0, spun about its y and scaled (FUN_002d0998). The muzzle point goes back to the game.
						if let Some(r) = &self.pyro_rig {
							let n = r.held.class.mesh.positions.len();
							let nn = r.nozzle.as_ref().map_or(0, |z| z.1.len());
							match pyro_pose.filter(|_| mats.len() > r.joint) {
								Some(pa) => {
									let hj = a.joint_world(&mats, r.joint);
									let gm = pa.pose(&r.held.anim, &[]);
									let (mut gp, mut gn) = anim::skin(&r.held.class, &gm);
									gp.resize(n, [0.0; 3]);
									gn.resize(n, [0.0, 0.0, 1.0]);
									ps.extend(gp.iter().map(|&v| apply(&hj, v, 1.0)));
									ns.extend(gn.iter().map(|&v| apply(&hj, v, 0.0)));
									let mj = r.muzzle.filter(|&j| j < gm.len()).map(|j| r.held.anim.joint_world(&gm, j));
									if let Some(m) = &mj {
										muzzle_model = Some(apply(&hj, [m[3], m[7], m[11]], 1.0));
									}
									match (&r.nozzle, nozzle.filter(|z| z.shown), &mj) {
										(Some((_, zp, zn)), Some(z), Some(m)) => {
											let (s, c) = z.spin.sin_cos();
											let turn = |v: [f32; 3], k: f32| [(v[0] * c + v[2] * s) * k, v[1] * k, (-v[0] * s + v[2] * c) * k];
											ps.extend(zp.iter().map(|&v| apply(&hj, apply(m, turn(v, z.scale), 1.0), 1.0)));
											ns.extend(zn.iter().map(|&v| apply(&hj, apply(m, turn(v, 1.0), 0.0), 0.0)));
										}
										_ => hide(&mut ps, &mut ns, nn),
									}
								}
								None => hide(&mut ps, &mut ns, n + nn),
							}
						}
						// The Suck Cannon (class 849) at its hero joint; its matrix goes back to the game as the held rows.
						if let Some(r) = &self.suck_rig {
							let n = r.held.class.mesh.positions.len();
							match suck_pose.filter(|_| mats.len() > r.joint) {
								Some(pa) => {
									let hj = a.joint_world(&mats, r.joint);
									let gm = pa.pose(&r.held.anim, &[]);
									let (mut gp, mut gn) = anim::skin(&r.held.class, &gm);
									gp.resize(n, [0.0; 3]);
									gn.resize(n, [0.0, 0.0, 1.0]);
									ps.extend(gp.iter().map(|&v| apply(&hj, v, 1.0)));
									ns.extend(gn.iter().map(|&v| apply(&hj, v, 0.0)));
									let mj = r.muzzle.filter(|&j| j < gm.len()).map(|j| r.held.anim.joint_world(&gm, j));
									if let Some(m) = &mj {
										muzzle_model = Some(apply(&hj, [m[3], m[7], m[11]], 1.0));
									}
									// Image of each model axis in the world: hero joint times the held root, then the hero yaw.
									let root = gm.first().map(|g| norm_cols(*g)).unwrap_or([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
									let hr = hero_rows(p);
									let mut rows = [[0.0f32; 3]; 3];
									for (k, row) in rows.iter_mut().enumerate() {
										let a0 = apply(&hj, apply(&root, [(k == 0) as u8 as f32, (k == 1) as u8 as f32, (k == 2) as u8 as f32], 0.0), 0.0);
										*row = hero_to_world(&hr, a0);
									}
									held_rows = Some(rows);
								}
								None => hide(&mut ps, &mut ns, n),
							}
						}
						// BRIDGE-38: each generic held rig at its hero joint when it is the equipped one; the others stay hidden.
						for c in game::held::rig_classes() {
							let Some(r) = self.held_rigs.get(&c) else { continue };
							let n = r.held.class.mesh.positions.len();
							match held_pose.filter(|(hc, _)| *hc == c).filter(|_| mats.len() > r.joint) {
								Some((_, pa)) => {
									let hj = a.joint_world(&mats, r.joint);
									// HELD-GLOVE: the Mine Glove / Glove of Doom take the hero hand copy like the Bomb Glove.
									let gm = if hand_copy { anim::hand_copy_pose(&r.held.anim, &hero_local) } else { pa.pose(&r.held.anim, &[]) };
									let (mut gp, mut gn) = anim::skin(&r.held.class, &gm);
									gp.resize(n, [0.0; 3]);
									gn.resize(n, [0.0, 0.0, 1.0]);
									ps.extend(gp.iter().map(|&v| apply(&hj, v, 1.0)));
									ns.extend(gn.iter().map(|&v| apply(&hj, v, 0.0)));
									let mj = r.muzzle.filter(|&j| j < gm.len()).map(|j| r.held.anim.joint_world(&gm, j));
									if let Some(m) = &mj {
										held_muzzle_model = Some(apply(&hj, [m[3], m[7], m[11]], 1.0));
									}
									// WPN-6.2a: the held actor's matrix (hero joint times the held root, then the hero yaw) and position.
									let root = gm.first().map(|g| norm_cols(*g)).unwrap_or([1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
									let hr = hero_rows(p);
									let mut rows = [[0.0f32; 3]; 3];
									for (k, row) in rows.iter_mut().enumerate() {
										let a0 = apply(&hj, apply(&root, [(k == 0) as u8 as f32, (k == 1) as u8 as f32, (k == 2) as u8 as f32], 0.0), 0.0);
										*row = hero_to_world(&hr, a0);
									}
									held_xform_model = Some((rows, apply(&hj, [0.0; 3], 1.0)));
									// BRIDGE-38 (d) (WPN-6.4a): the same row loop with the held joint-0 (muzzle) matrix in place of the
									// root: FUN_002640a0's rotation (joint rotation times the moby basis, no class scale).
									if let Some(jm) = mj.map(norm_cols) {
										let mut rows = [[0.0f32; 3]; 3];
										for (k, row) in rows.iter_mut().enumerate() {
											let a0 = apply(&hj, apply(&jm, [(k == 0) as u8 as f32, (k == 1) as u8 as f32, (k == 2) as u8 as f32], 0.0), 0.0);
											*row = hero_to_world(&hr, a0);
										}
										held_joint0_model = Some(rows);
									}
								}
								None => hide(&mut ps, &mut ns, n),
							}
						}
						// BRIDGE-76 (ARI-47c): the Swingshot hook (class 209) at the game's hook position with the rows copied from
						// the held moby, shown while the held Swingshot is drawn; taken into Ratchet's model space like the thrown wrench.
						if let Some(hk) = &self.hook_rig {
							let n = hk.class.mesh.positions.len();
							let shown = held_pose.is_some_and(|(hc, _)| hc == game::held::SWINGSHOT_HELD)
								&& self.held_rigs.get(&game::held::SWINGSHOT_HELD).is_some_and(|r| mats.len() > r.joint);
							match game.swing_rope().filter(|_| shown) {
								Some(d) => {
									let gm = d.hook_anim.pose(&hk.anim, &[]);
									let (mut gp, mut gn) = anim::skin(&hk.class, &gm);
									gp.resize(n, [0.0; 3]);
									gn.resize(n, [0.0, 0.0, 1.0]);
									let r = d.hook_rows.unwrap_or([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
									let hr = hero_rows(p);
									let at = [d.hook[0] - p.pos[0], d.hook[1] - p.pos[1], d.hook[2] - p.pos[2]];
									ps.extend(gp.iter().map(|&v| {
										let w = hero_to_world(&r, v);
										hero_to_model(&hr, [w[0] + at[0], w[1] + at[1], w[2] + at[2]])
									}));
									ns.extend(gn.iter().map(|&v| hero_to_model(&hr, hero_to_world(&r, v))));
								}
								None => hide(&mut ps, &mut ns, n),
							}
						}
						// BRIDGE-21: the back objects d0 (class 607) and d4 (Clank, 601) share one matrix, the hero joint of
						// attach point 5 (columns normalised, then the limb pulse unclamped: only the wrench slot is capped).
						let bj = self.back_joint.filter(|&j| mats.len() > j && !game.player.clank_hidden).map(|j| {
							let b = norm_cols(a.joint_world(&mats, j));
							pulse.map_or(b, |s| crate::player::pulse_item(b, s, false, hero_pt))
						});
						if let Some(r) = &self.back_rig {
							append_back_object(&mut ps, &mut ns, r.class.mesh.positions.len(), anim::skin(&r.class, &self.hero_rig.back.pose(&r.anim, &[])), bj.as_ref());
						}
						let mut hand_pose = Vec::new();
						if let Some(r) = &self.clank_rig {
							hand_pose = self.hero_rig.hand.pose_layered(&r.anim, &rparts.map_or_else(|| game.player.hero_joint_fx(1, &r.anim.slot_map), |q| q.clank_fx.clone()), None, None);
							append_back_object(&mut ps, &mut ns, r.class.mesh.positions.len(), anim::skin(&r.class, &hand_pose), bj.as_ref());
						}
						// BRIDGE-21c: the Clank light (class 1204, scaled 1.3) on joint 6 of the d4 pose, seen through the same back matrix.
						if let Some(lc) = &self.light_rig {
							let n = lc.mesh.positions.len();
							match bj.as_ref().zip(self.clank_rig.as_ref()).filter(|_| hand_pose.len() > CLANK_LIGHT_JOINT) {
								Some((bj, r)) => {
									let jw = r.anim.joint_world(&hand_pose, CLANK_LIGHT_JOINT);
									let through = |v: [f32; 3], t: f32| apply(bj, apply(&jw, v, t), t);
									ps.extend(lc.mesh.positions.iter().map(|v| through([v[0] * CLANK_LIGHT_SCALE, v[1] * CLANK_LIGHT_SCALE, v[2] * CLANK_LIGHT_SCALE], 1.0)));
									ns.extend((0..n).map(|k| through(lc.normals.get(k).copied().unwrap_or([0.0, 0.0, 1.0]), 0.0)));
									light_at = Some(through([0.0; 3], 1.0));
								}
								None => hide(&mut ps, &mut ns, n),
							}
						}
						// BRIDGE-40 HERO-GHOST (WPN-6.6a, trail 0x1409c0): up to three translucent copies of the hero's current
						// base pose at the lagged trail positions and yaws; hidden when absent. No layers or joint effects [C]: the
						// retail copy's +0x60 / +0x64 lists stay 0 (WPN-6.6b4, decomp/notes/weapons_framework.md). Untinted [C]: its
						// flags 0x80a clear the CLASS-tint bit 0x10, so the hero glow tint covers the hero range only. Lit with the
						// hero's current light, where retail freezes the spawn tick's light word [I].
						let hero_n = class.mesh.positions.len();
						let trail = p.hero_ghosts();
						let ghost_pose = trail.iter().any(Option::is_some).then(|| {
							let (mut gp, mut gn) = anim::skin(class, &self.hero_rig.hero.pose(a, &[]));
							gp.resize(hero_n, [0.0; 3]);
							gn.resize(hero_n, [0.0, 0.0, 1.0]);
							(gp, gn)
						});
						for (gi, g) in trail.into_iter().enumerate() {
							let base = ps.len();
							let (Some((gpos, gyaw, alpha)), Some((gp, gn))) = (g, ghost_pose.as_ref()) else {
								hide(&mut ps, &mut ns, hero_n);
								continue;
							};
							let (c, sn) = (p.yaw.cos(), p.yaw.sin());
							let (gc, gs) = (gyaw.cos(), gyaw.sin());
							let local = |v: [f32; 3], t: f32| {
								let r = [v[0] * gc - v[1] * gs, v[0] * gs + v[1] * gc, v[2]];
								let d = [r[0] + t * (gpos[0] - p.pos[0]), r[1] + t * (gpos[1] - p.pos[1]), r[2] + t * (gpos[2] - p.pos[2])];
								[d[0] * c + d[1] * sn, -d[0] * sn + d[1] * c, d[2]]
							};
							ps.extend(gp.iter().map(|&v| local(v, 1.0)));
							ns.extend(gn.iter().map(|&v| local(v, 0.0)));
							ghost_ranges[2 + gi] = Some((base, ps.len(), alpha));
						}
						(ps, ns)
					}) };
					if !hero_hit {
						hero_gen = next_gen();
						let fresh = HeroCache { key: hero_key, pose_gen: hero_gen, skinned: skinned.clone(), ghost_ranges, bomb_range, light_at, held_rows, muzzle_model, held_muzzle_model, held_joint0_model, held_xform_model };
						if hero_cand {
							hero_check_compare(self.hero_cache.as_ref().expect("hero_cand needs the cache"), &fresh);
						}
						self.hero_cache = Some(fresh);
					}
					// The muzzle in the world: the hero's position plus his yaw (the ship pad frame is not used while armed).
					self.suck_matrix = held_rows;
					self.pyro_muzzle = muzzle_model.map(|m: [f32; 3]| {
						let l = hero_to_world(&hero_rows(p), m);
						[p.pos[0] + l[0], p.pos[1] + l[1], p.pos[2] + l[2]]
					});
					self.held_muzzle = held_muzzle_model.map(|m: [f32; 3]| {
						let l = hero_to_world(&hero_rows(p), m);
						[p.pos[0] + l[0], p.pos[1] + l[1], p.pos[2] + l[2]]
					});
					self.held_joint0 = held_joint0_model;
					self.held_xform = held_xform_model.map(|(rows, m): ([[f32; 3]; 3], [f32; 3])| {
						let l = hero_to_world(&hero_rows(p), m);
						(rows, [p.pos[0] + l[0], p.pos[1] + l[1], p.pos[2] + l[2]])
					});
					self.clank_light_glow = light_at.map(|m: [f32; 3]| {
						let l = hero_to_world(&hero_rows(p), m);
						[p.pos[0] + l[0], p.pos[1] + l[1], p.pos[2] + l[2]]
					});
					let (at, skinned, tilt) = { let _s = profile::span("sm_hero"); self.smooth.hero(game, at, skinned, hero_gen) };
					let (mut at, mut skinned, mut tilt) = (at, skinned, tilt);
					// [PC] perf (lane R2): the GPU hero blends on the GPU; scenes and ship seats below replace the pose, so no blend.
					let mut hero_blend_t = self.smooth.hero_blend().map(|b| b.1);
					let mut hero_gens = self.smooth.hero_gens();
					// A ship stream plays like a scene, in the frame of the ship pad (FUN_002a39b0: positions through
					// the ship's matrix and position, camera yaw plus the ship's).
					let ship_play = self.ship_play.as_ref().and_then(|p| Some((p.id, self.scenes.get(&p.id)?, p.tick / scene::TICKS_PER_SECOND)));
					let pad = ship_play.and(game.ship_pad()).map(|(_, p, yaw)| (p, yaw));
					let pad_yaw = pad.map_or(0.0, |p| p.1);
					let place_at = |v: [f32; 3]| match pad {
						Some((p, yaw)) => {
							let (s, c) = yaw.sin_cos();
							[p[0] + v[0] * c - v[1] * s, p[1] + v[0] * s + v[1] * c, p[2] + v[2]]
						}
						None => v,
					};
					let turned = |(ps, ns): Posed| -> Posed {
						if pad_yaw == 0.0 {
							return (ps, ns);
						}
						let (s, c) = pad_yaw.sin_cos();
						let t = |v: [f32; 3]| [v[0] * c - v[1] * s, v[0] * s + v[1] * c, v[2]];
						(ps.into_iter().map(t).collect(), ns.into_iter().map(t).collect())
					};
					let playing = ship_play.or_else(|| self.flow.as_ref().and_then(|f| f.scene_time()).and_then(|(id, secs)| Some((id, self.scenes.get(&id)?, secs))));
					let mut actors: Vec<Option<(Posed, [f32; 3])>> = vec![None; self.actor_slots.len()];
					let mut eye_glows: Vec<([f32; 3], f32)> = Vec::new();
					if let Some((id, (sc, _), secs)) = playing {
						let raw = secs * scene::TICKS_PER_SECOND;
						// PC-FPS-1: a flow scene or ship stream is drawn between its last two ticks (FRAME-1e: both step per tick).
						let tick = self.smooth.scene_tick(sc, raw);
						if let Some(mut k) = sc.camera_at(tick) {
							k.rot[2] += pad_yaw;
							let f = k.forward();
							self.cam.pos = place_at(k.pos);
							self.cam.yaw = f[1].atan2(f[0]);
							self.cam.pitch = f[2].clamp(-1.0, 1.0).asin();
							self.cam.tan_half = k.tan_half_fov * PROJ_Y_RATIO;
							self.cam.up = [0.0, 0.0, 1.0];
							self.glint_cam = None;
						}
						if tick != raw {
							if let Some(mut k) = sc.camera_at(raw) {
								k.rot[2] += pad_yaw;
								let f = k.forward();
								self.smooth.tick_cam = Some((place_at(k.pos), f[1].atan2(f[0]), f[2].clamp(-1.0, 1.0).asin(), k.tan_half_fov * PROJ_Y_RATIO));
							}
						}
						// Ratchet (actor class 0) follows the scene; his facing is in the root joint [I].
						if let (Some(actor), Some((class, a))) = (sc.actors.iter().find(|a| a.class_id == 0), &self.hero_anim) {
							if let (Some((ka, kb, t)), Some(pos)) = (actor.pair(tick), actor.pos_at(tick)) {
								let mats = a.pose_frames(&ka.frame, &kb.frame, t);
								let (mut ps, mut ns) = anim::skin(class, &mats);
								if let Some((hp, hn)) = &self.wrench {
									let wj = a.joint_world(&mats, WRIST);
									ps.extend(hp.iter().map(|&v| apply(&wj, v, 1.0)));
									ns.extend(hn.iter().map(|&v| apply(&wj, v, 0.0)));
								}
								// No glove in cutscenes.
								let held = 2 * self.wrench.as_ref().map_or(0, |w| w.0.len())
									+ self.glove.as_ref().map_or(0, |g| g.class.mesh.positions.len())
									+ self.bomb.as_ref().map_or(0, |b| b.0.len())
									+ self.pyro_rig.as_ref().map_or(0, |r| r.held.class.mesh.positions.len() + r.nozzle.as_ref().map_or(0, |z| z.1.len()))
									+ self.suck_rig.as_ref().map_or(0, |r| r.held.class.mesh.positions.len())
									+ held_len(&self.held_rigs, &self.hook_rig)
								+ back_tail(&self.back_rig, &self.clank_rig, &self.light_rig)
								+ hero_ghost_len(class);
								hide(&mut ps, &mut ns, held);
								skinned = Some((ps, ns));
								hero_blend_t = None;
								hero_gens = (0, 0);
								ghost_ranges = [None; 5];
								at = Some((place_at(pos), pad_yaw));
								// BRIDGE-75 [I]: the scene frame poses the hero, untilted.
								tilt = [0.0; 2];
							}
						}
						ACTOR_TINTS.with(|t| *t.borrow_mut() = vec![None; actors.len()]);
						for (si, (slot, &(sid, index, class_id))) in actors.iter_mut().zip(&self.actor_slots).enumerate() {
							let (Some(actor), Some((class, a))) = (sc.actors.iter().find(|a| a.index == index).filter(|_| sid == id), self.prop_anims.get(&class_id)) else { continue };
							// Class +0x40 default tail colour from the spawn (FUN_0024e540); only the Clank update replaces it.
							ACTOR_TINTS.with(|t| t.borrow_mut()[si] = mobies::glow_rgb(class.glow).map(|c| (c, class.colour_start)));
							// Clank moby tint (FUN_00206f78 +0x90): 0x80 = 1.0 per channel, sine pulse over 83 ticks (fead0(100)), base + trunc(k * s) (veldin_ambient.md VEL-5 spec 4).
							if matches!(class_id, 10 | 0x555 | 0x1a3) {
								let c = Some(crate::game::clank_tint(game.frames));
								ACTOR_TINTS.with(|t| t.borrow_mut()[si] = c.map(|c| (c, class.colour_start)));
							}
							if let (Some((ka, kb, t)), Some(pos)) = (actor.pair(tick), actor.pos_at(tick)) {
								let mats = a.pose_frames(&ka.frame, &kb.frame, t);
								// Clank eye glow (FUN_00206f78 -> FUN_00207b80), at an animated joint, not the actor centre.
								if matches!(class_id, 10 | 0x555 | 0x1a3) {
									let jn = if class_id == 0x1a3 { 6 } else { 1 };
									if jn < mats.len() {
										let j = a.joint_world(&mats, jn);
										let mut p = place_at([pos[0] + j[3], pos[1] + j[7], pos[2] + j[11]]);
										if class_id == 0x1a3 {
											p[2] -= 0.07;
											if (place_at(pos)[2] - p[2]).abs() > 2.0 {
												eye_glows.push((p, 0.25));
											}
										} else {
											p[2] -= 0.007;
											eye_glows.push((p, 0.0469));
										}
									}
								}
								*slot = Some((turned(anim::skin(class, &mats)), place_at(pos)));
							}
						}
						let line = sc.subtitles_at(raw as u32).next().map(|s| s.text[0].clone()).filter(|s| !s.is_empty());
						if let Some(f) = &mut self.flow {
							f.decoded = true;
							f.subtitle = line;
						}
					}
					// SCN-4.3c: the flight's two passengers, drawn through the marker (Ratchet) and the first class-10 actor slot
					// (Clank) at the ship's seats (basis = the rotation the tick began with); retail draws no wrench on the passenger Ratchet (L394).
					let flight_pose = self.travel.as_ref().and_then(|t| t.flight_pose());
					let mut passenger_shown = false;
					if let Some(fp) = flight_pose.filter(|fp| fp.passengers) {
						let m = particles::euler_m([fp.rot[0], fp.rot[1], fp.rot[2]]);
						let turn = |(ps, ns): Posed| -> Posed {
							let t = |v: [f32; 3]| -> [f32; 3] { std::array::from_fn(|k| m[0][k] * v[0] + m[1][k] * v[1] + m[2][k] * v[2]) };
							(ps.into_iter().map(t).collect(), ns.into_iter().map(t).collect())
						};
						if let Some((class, a)) = self.hero_anim.as_ref().filter(|(_, a)| a.sequences.first().is_some_and(|s| !s.frames.is_empty())) {
							let (mut ps, mut ns) = anim::skin(class, &a.pose_frame(0, 0));
							let held = 3 * self.wrench.as_ref().map_or(0, |w| w.0.len())
								+ self.glove.as_ref().map_or(0, |g| g.class.mesh.positions.len())
								+ self.bomb.as_ref().map_or(0, |b| b.0.len())
								+ self.pyro_rig.as_ref().map_or(0, |r| r.held.class.mesh.positions.len() + r.nozzle.as_ref().map_or(0, |z| z.1.len()))
								+ self.suck_rig.as_ref().map_or(0, |r| r.held.class.mesh.positions.len())
								+ held_len(&self.held_rigs, &self.hook_rig)
								+ back_tail(&self.back_rig, &self.clank_rig, &self.light_rig)
								+ hero_ghost_len(class);
							hide(&mut ps, &mut ns, held);
							skinned = Some(turn((ps, ns)));
							hero_blend_t = None;
							hero_gens = (0, 0);
							ghost_ranges = [None; 5];
							at = Some((game::transit::seat_pos(fp.pos, fp.rot_prev, game::transit::RATCHET_SEAT[fp.ship_type.min(2)]), 0.0));
							// BRIDGE-75 [I]: the seat frame poses the hero, untilted.
							tilt = [0.0; 2];
							passenger_shown = true;
						}
						let clank = self.actor_slots.iter().position(|s| s.2 == 10).and_then(|si| Some((si, self.prop_anims.get(&10).filter(|(_, a)| a.sequences.first().is_some_and(|s| !s.frames.is_empty()))?)));
						if let Some((si, (class, a))) = clank {
							let posed = turn(anim::skin(class, &a.pose_frame(0, 0)));
							actors[si] = Some((posed, game::transit::seat_pos(fp.pos, fp.rot_prev, game::transit::CLANK_SEAT[fp.ship_type.min(2)])));
							ACTOR_TINTS.with(|t| {
								let mut t = t.borrow_mut();
								t.resize(actors.len(), None);
								t[si] = mobies::glow_rgb(class.glow).map(|c| (c, class.colour_start));
							});
						}
					}
					// SCN-4.3a: the departure flight's scripted camera (cuboid poses, world coordinates).
					if let Some((pos, rot)) = self.travel.as_ref().and_then(|t| t.flight_cam()) {
						let k = scene::CameraKey { pos, rot, tan_half_fov: camera::PROJ_SCALAR, cut: false };
						let fw = k.forward();
						self.cam.pos = pos;
						self.cam.yaw = fw[1].atan2(fw[0]);
						self.cam.pitch = fw[2].clamp(-1.0, 1.0).asin();
						self.cam.tan_half = camera::PROJ_SCALAR * PROJ_Y_RATIO;
						self.cam.up = [0.0, 0.0, 1.0];
						self.glint_cam = None;
						self.over_cam = None;
					}
					// SCN-4.1c: the arrival warp's stream camera (stream space: no pad placement).
					if let Some(k) = self.travel.as_ref().and_then(|t| t.warp_cam()) {
						let fw = k.forward();
						self.cam.pos = k.pos;
						self.cam.yaw = fw[1].atan2(fw[0]);
						self.cam.pitch = fw[2].clamp(-1.0, 1.0).asin();
						self.cam.tan_half = k.tan_half_fov * PROJ_Y_RATIO;
						self.cam.up = [0.0, 0.0, 1.0];
						self.glint_cam = None;
						self.over_cam = None;
					}
					// NOV-13.5b: the gadget cinematic's camera (angles applied x, y, z), Ratchet (actor 0) and the gadget
					// model (actor 1, in his frame: both face the scene yaw); the black fade goes over the HUD.
					if let Some(f) = self.gadget_frame {
						let k = scene::CameraKey { pos: f.cam_pos, rot: f.cam_ang, tan_half_fov: f.scale, cut: false };
						let fw = k.forward();
						self.cam.pos = f.cam_pos;
						self.cam.yaw = fw[1].atan2(fw[0]);
						self.cam.pitch = fw[2].clamp(-1.0, 1.0).asin();
						self.cam.tan_half = f.scale * PROJ_Y_RATIO;
						self.cam.up = [0.0, 0.0, 1.0];
						self.glint_cam = None;
						let gs = match &game.gadget {
							Some(game::gadget_scene::GadgetRun::Scene { scene, .. }) => Some(scene),
							_ => None,
						};
						if let (Some(gs), Some((class, a))) = (gs, &self.hero_anim) {
							if let Some(seq) = gs.sequence(0).filter(|s| !s.frames.is_empty()) {
								let p = f.actors[0];
								let n = seq.frames.len();
								let mats = a.pose_frames(&seq.frames[p.frame_a.min(n - 1)], &seq.frames[p.frame_b.min(n - 1)], p.blend);
								let (mut ps, mut ns) = anim::skin(class, &mats);
								let before = 3 * self.wrench.as_ref().map_or(0, |w| w.0.len()) + self.glove.as_ref().map_or(0, |g| g.class.mesh.positions.len()) + self.bomb.as_ref().map_or(0, |b| b.0.len());
								hide(&mut ps, &mut ns, before);
								if let Some(r) = &self.pyro_rig {
									let cnt = r.held.class.mesh.positions.len();
									match gs.sequence(1).filter(|s| !s.frames.is_empty() && f.actors[1].class == game::pyro::PYRO_CLASS) {
										Some(s1) => {
											let (q, m) = (f.actors[1], s1.frames.len());
											let gm = r.held.anim.pose_frames(&s1.frames[q.frame_a.min(m - 1)], &s1.frames[q.frame_b.min(m - 1)], q.blend);
											let (mut gp, mut gn) = anim::skin(&r.held.class, &gm);
											gp.resize(cnt, [0.0; 3]);
											gn.resize(cnt, [0.0, 0.0, 1.0]);
											let (s, c) = (-p.yaw).sin_cos();
											let d = [q.pos[0] - p.pos[0], q.pos[1] - p.pos[1], q.pos[2] - p.pos[2]];
											let o = [c * d[0] - s * d[1], s * d[0] + c * d[1], d[2]];
											ps.extend(gp.iter().map(|v| [v[0] + o[0], v[1] + o[1], v[2] + o[2]]));
											ns.extend(gn);
										}
										None => hide(&mut ps, &mut ns, cnt),
									}
									hide(&mut ps, &mut ns, r.nozzle.as_ref().map_or(0, |z| z.1.len()));
								}
								if let Some(r) = &self.suck_rig {
									hide(&mut ps, &mut ns, r.held.class.mesh.positions.len());
								}
								hide(&mut ps, &mut ns, held_len(&self.held_rigs, &self.hook_rig));
								hide(&mut ps, &mut ns, back_tail(&self.back_rig, &self.clank_rig, &self.light_rig));
								hide(&mut ps, &mut ns, hero_ghost_len(class));
								skinned = Some((ps, ns));
								hero_blend_t = None;
								hero_gens = (0, 0);
								ghost_ranges = [None; 5];
								at = Some((p.pos, p.yaw));
							}
						}
					}
					// [PC] DEV ONLY (RAC_DEV_TOOLS=1, retail/devtools.rs): the free camera replaces the drawn view only; retail's camera
					// RAM and the mirror's tick camera (glint culling, sound) stay retail's.
					if self.retail.is_some() {
						let held = &self.keys;
						if let Some(c) = crate::retail::devtools::frame(dt, |k| held.contains(&k), (self.cam.pos, self.cam.yaw, self.cam.pitch, self.cam.tan_half)) {
							(self.cam.pos, self.cam.yaw, self.cam.pitch, self.cam.tan_half, self.cam.up) = (c.0, c.1, c.2, c.3, [0.0, 0.0, 1.0]);
						}
					}
					// Retail mode: the orbs retail's draw queue holds (retail.rs `orb_draws`); rac-rs's own particle step does not run.
					let orbs: Vec<particles::OrbDraw> = match &self.retail {
						Some(r) => r.orb_draws().to_vec(),
						None => self.particles.orb_draws().to_vec(),
					};
					self.sprites.clear();
					self.ribbons.clear();
					self.lines.clear();
					let mut sprite_keys = Vec::new();
					self.particles.sprites_keyed(&mut self.sprites, Some(&mut sprite_keys));
					self.smooth.sprites(&mut self.sprites, &sprite_keys);
					game.pyro_sprites(&mut self.sprites);
					// [PC] The props on the hero (Clank, back pack, head items, bulb mesh) ride his drawn placement (smooth.rs `attach`).
					self.smooth.attach(&game.props, at, tilt);
					// Retail mode: retail's whole effect pool and the hero glow callback (retail.rs `queue_fx`).
					if let Some(fx) = self.retail.as_ref().and_then(|r| r.fx()) {
						// The pool's lines and ribbons join the pool sprites' depth walk in `set_particles` (queue_b.rs `pool_walk`).
						self.sprites.extend_from_slice(&fx.sprites);
						// [PC] Clank's bulb glow (retail's slot-3 item light) rides the bulb mesh as `attach` draws it (strafe turn, Smoothing).
						let bulb = self.retail.as_ref().and_then(|r| r.fx_tick()).and_then(|t| t.glows.iter().find(|g| crate::retail::is_bulb_glow(g))).and_then(|g| self.smooth.bulb_glow(&game.props, g.0));
						for g in &fx.glows {
							let g_at = if crate::retail::is_bulb_glow(g) { bulb.unwrap_or(g.0) } else { g.0 };
							particles::glow(&mut self.sprites, self.cam.pos, g_at, g.1, g.2, g.3);
						}
					}
					self.particles.arc_glows(&mut self.sprites);
					self.particles.ribbons(&mut self.ribbons);
					// Retail mode: the robot arcs retail's queue A holds (retail.rs `arc_draws`).
					if let Some(fx) = self.retail.as_ref().and_then(|r| r.fx()) {
						for a in &fx.arcs {
							particles::arc_ribbons(&a.strand_a, &a.strand_b, a.uv, &mut self.ribbons);
							particles::arc_end_glows(a.origin, a.end, a.phase_a, a.phase_b, &mut self.sprites);
						}
					}
					self.particles.lines(&mut self.lines); // KER-92
					self.effect_mobys = self.particles.effect_mobys();
					if let Some(wt) = &self.water {
						self.effect_mobys.extend(wt.effect_mobys()); // NOV-20.3
					}
					// Glow quads (FUN_00263228, particle texture 11): Clank's red joint-1 light (0x200000c0, R low; the green eyes are the moby tint pulse), pulled 0.15
					// towards the camera) and the red orbs (scale 1.0, bias 0.5).
					// BRIDGE-21c: the light's glow quad above joint 6, drawn while the back object is shown (hero kind 2 or 3).
					// Retail mode: retail's glow callback draws it (`queue_fx` glows).
					if let Some(m) = self.clank_light_glow.take().filter(|_| self.retail.is_none() && self.hero_rig.kind() >= 2 && !game.player.clank_hidden && !game.hero_hidden) {
						particles::glow(&mut self.sprites, self.cam.pos, [m[0], m[1], m[2] + CLANK_LIGHT_GLOW_Z], CLANK_LIGHT_GLOW.0, CLANK_LIGHT_GLOW.1, CLANK_LIGHT_GLOW.2);
					}
					for (at, half) in eye_glows {
						particles::glow(&mut self.sprites, self.cam.pos, at, half, 0.15, 0x200000c0);
					}
					// ARI-38 / BRIDGE-66: the flying transports' joint glows recorded by their ticks (draw callbacks FUN_002ebc38 / FUN_002ec598).
					for (at, word, half, bias) in game.transport_glows() {
						particles::glow(&mut self.sprites, self.cam.pos, at, half, bias, word);
					}
					let mut vendor_quads = Vec::new();
					// Retail mode: the vendors retail queued this field (queue B, class 11 in states 1 and 2; retail.rs `vendor_glows`).
					if let Some(fx) = self.retail.as_ref().and_then(|r| r.fx()) {
						for v in &fx.vendors {
							vendor_quads.extend(game::vendor::vendor_layers(v.pos, v.ramp, v.scroll, self.cam.yaw));
							for &at in &v.nodes {
								particles::glow(&mut self.sprites, self.cam.pos, at, 0.134, 0.15, v.word);
							}
						}
					}
					// Nanotech orbs: body, rim, sparkles and crate glass (decomp/notes/effects_lighting_mapping.md §4).
					for o in &orbs {
						self.particles.nanotech_orb(o, self.cam.pos, &mut world_quads);
					}
					profile::lap(profile::Phase::Hero);
					let mut posed: HashMap<PoseKey, Posed> = HashMap::new();
					// Vendor and orb: blend / joint-spin poses (vendor.rs vendor_fx), one skinning per prop.
					let mut fx_posed: HashMap<usize, Posed> = HashMap::new();
					for (pi, pr) in game.props.iter().enumerate().filter(|(_, p)| p.shown) {
						// SCN-4.3c: the flying ship blends sequence 1 frame 0 into sequence 2 by the flight's +0x54.
						if let (Some(fp), Some((class, a))) = (flight_pose.filter(|fp| fp.prop == pi), self.prop_anims.get(&pr.class_id)) {
							if a.sequences.len() > 2 && !a.sequences[1].frames.is_empty() && !a.sequences[2].frames.is_empty() {
								let (f0, f1) = game::transit::ship_blend_frames(a.sequences[1].frames.len());
								fx_posed.insert(pi, anim::skin_all(class, &a.pose_frames(&a.sequences[1].frames[f0], &a.sequences[2].frames[f1.min(a.sequences[2].frames.len() - 1)], fp.blend)));
								continue;
							}
						}
						// Vendor/orb spins, and any prop fading from a snapshot (its pose is its own).
						let spins = game.vendor_fx.poses.get(&pi).map(|fx| fx.spins.as_slice());
						if let Some((class, a)) = self.prop_anims.get(&pr.class_id) {
							// NOV-14.8e: the Plumber's head-look joint overrides.
							// KER-1a: the Kerwan walker's blocks drive the attach slot 2 / 3 joints.
							let mut fx: Vec<anim::JointFx> = Vec::new();
							// NOV-30c: the shop vendor's icon bar corner records.
							fx.extend(game.shop_bar_fx(pi, &a.slot_map));
							// M3a, retail mode: the moby's own +0x64 records (e.g. Clank's blink), as the evaluator FUN_00268a80 applies them.
							if let Some(r) = &self.retail {
								fx.extend(r.prop_fx(pi).iter().cloned());
							}
							if spins.is_some() || pr.anim.fading() || !fx.is_empty() {
								fx_posed.insert(pi, anim::skin_all(class, &pr.anim.pose_fx_full(a, spins.unwrap_or(&[]), &fx)));
							}
						}
					}
					// PC perf: no game tick since the last frame's poses (same stamp): the anim states are unchanged, so each
					// pose key's skinning is moved over from that frame instead of recomputed.
					let stamp = (game.frames, game.props.as_ptr() as usize, game.props.len());
					let mut last = if self.smooth.ticks == 0 && self.pose_stamp == Some(stamp) { self.smooth.take_posed() } else { HashMap::new() };
					self.pose_stamp = Some(stamp);
					// [PC] perf: the joint matrices per new key here, the skinning on the frame worker pool; the keys go into
					// `posed` in first-seen order as before.
					let mut todo: Vec<(PoseKey, Result<Posed, (&mobies::MobyClass, Vec<[f32; 12]>)>)> = Vec::new();
					for pr in game.props.iter().filter(|p| p.shown) {
						if let Some((class, a)) = self.prop_anims.get(&pr.class_id) {
							let key = pose_key(pr);
							if !posed.contains_key(&key) {
								todo.push((key, last.remove(&key).ok_or_else(|| (class, pr.anim.pose(a, &[])))));
								posed.insert(key, Default::default());
							}
						}
					}
					drop(last);
					{
						use rayon::prelude::*;
						let _s = profile::span("prop_skin");
						let done: Vec<Posed> = todo.par_iter_mut().map(|(_, job)| match job {
							Ok(p) => std::mem::take(p),
							Err((class, mats)) => anim::skin_all(class, mats),
						}).collect();
						for ((key, _), p) in todo.iter().zip(done) {
							posed.insert(*key, p);
						}
					}
					// PC-FPS-1: blended poses join `fx_posed`, blended placements go to `smooth::PROP_XF`.
					{ let _s = profile::span("sm_props"); self.smooth.props(&game.props, posed, &mut fx_posed); }
					profile::lap(profile::Phase::Props);
					let title = format!("{} - {} - bolts {} - health {}/4{}", crate::app_name(), toc::LEVEL_NAMES[self.level], game.bolts, game.health.max(0), crate::retail::debugkeys::title_suffix()) + &crate::retail::devtools::title_suffix(self.level, game.player.pos);
					self.shown_bolts = if (game.bolts as f32) < self.shown_bolts { game.bolts as f32 } else { (self.shown_bolts + dt * 60.0f32.max((game.bolts as f32 - self.shown_bolts) * 4.0)).min(game.bolts as f32) };
					// NOV-30f nano request latch (compares against the previous frame's health before `hud_seen` updates).
					hud::nano_request(&mut self.nano_live, &mut self.nano_hud, game.shop.is_some(), game.health, self.hud_seen.1);
					if (game.bolts, game.health) != self.hud_seen {
						if game.bolts != self.hud_seen.0 {
							self.hud_t.0 = 4.8;
						}
						self.hud_seen = (game.bolts, game.health);
					}
					self.hud_t = ((self.hud_t.0 - dt).max(0.0), (self.hud_t.1 - dt).max(0.0));
					// RAC_HUD_ALL: keep the bolt and nanotech counters up (HUD layout shots).
					if std::env::var_os("RAC_HUD_ALL").is_some() {
						self.hud_t = (4.8, 4.8);
						self.nano_hud = (100, 8, 8, game.health.clamp(0, 4));
					}
					// Nanotech meter: whole 50 Hz ticks off the frame dt (hud.md UI-7), not seconds.
					self.nano_acc += dt / 0.02;
					while self.nano_acc >= 1.0 {
						self.nano_acc -= 1.0;
						hud::nano_tick(&mut self.nano_live, &mut self.nano_hud, game.shop.is_some(), game.health);
					}
					// Kind-1 (buy) records show the bolt counter (novalis_flow.md: FUN_0024a900(2, 0x754e, ..)).
					if matches!(game.prompt, Some(1011 | 1012)) {
						self.hud_t.0 = self.hud_t.0.max(4.8);
					}
					// NOV-30f: the shop's bolt counter (slot 2) owns the bolt HUD while committed; one alpha = f1 / 8 [I].
					let mut bolt_alpha = (self.hud_t.0 / 1.8).min(1.0);
					if game.shop_hud.bolts.active == Some(crate::game::vendor::BOLTS) {
						self.hud_t.0 = 0.0;
						bolt_alpha = game.shop_hud.bolts.f1 as f32 / 8.0;
					}
					let eq = game.inv.equipped;
					// Ammo HUD fade (hud.md "Ammo HUD", update 0x23c9a0): shown = on foot with a weapon whose max
					// ammo != 0. f1 (bar, icon) then f2 (text) step 1/8 per tick; hiding drops f2 then f1, and a
					// weapon change always fades out and back in. Ticks taken as 50 Hz frames [I]; the short
					// hide delay (timer < fead0(5)) is not modelled [P].
					let want = crate::inventory::info(eq).filter(|i| i.max > 0).map(|_| eq);
					let step = dt * 50.0 / 8.0;
					let ah = &mut self.ammo_hud;
					if want.is_some() && ah.0 == want {
						if ah.1 < 1.0 {
							ah.1 = (ah.1 + step).min(1.0);
						} else {
							ah.2 = (ah.2 + step).min(1.0);
						}
					} else if ah.2 > 0.0 {
						ah.2 = (ah.2 - step).max(0.0);
					} else if ah.1 > 0.0 {
						ah.1 = (ah.1 - step).max(0.0);
					} else {
						ah.0 = want;
					}
					let q = |f: f32| (f * 8.0).floor() / 8.0;
					let (shown, f1, f2) = (ah.0, q(ah.1), q(ah.2));
					let mut ammo = shown.and_then(|id| crate::inventory::info(id).map(|i| (id, game.inv.ammo(id), i.max, f1, f2))).filter(|a| a.3 > 0.0);
					// NOV-30f: a shop ammo counter in slot 0 draws instead (same draw); the NORMAL handover restarts the
					// on-foot model from empty once it commits.
					let slot = &game.shop_hud.ammo;
					if slot.shop_owned() {
						self.ammo_hud = (None, 0.0, 0.0);
						let id = slot.active.filter(|&a| a != crate::game::vendor::NORMAL).or(slot.pending.map(|p| p.0)).unwrap_or(0) as u8;
						let max = crate::inventory::info(id).map_or(0, |i| i.max);
						ammo = Some((id, slot.display as u16, max, slot.f1 as f32 / 8.0, slot.f2 as f32 / 8.0)).filter(|_| slot.f1 > 0);
					}
					if slot.pending.is_some_and(|p| p.0 == crate::game::vendor::NORMAL) {
						self.ammo_hud = (None, 0.0, 0.0);
						ammo = None;
					}
					let reticle = game.lock.filter(|_| eq == crate::inventory::BOMB_GLOVE).and_then(|l| {
						let g = self.gpu.as_ref()?;
						project(&view_proj(&self.cam, g.config.width as f32 / g.config.height.max(1) as f32, RETAIL_NEAR, RETAIL_FAR), l)
					});
					// WPN-6.2g2 / BRIDGE-39: the screen-sprite queue with centres in window pixels; hero state 0x72 empties it.
					let sprites = self.gpu.as_ref().map_or(Vec::new(), |g| {
						let vp = view_proj(&self.cam, g.config.width as f32 / g.config.height.max(1) as f32, RETAIL_NEAR, RETAIL_FAR);
						queue_sprites(&game.sprite_queue, game.player.retail_ids().0, &vp, g.config.width as f32, g.config.height as f32)
					});
					let st = HudState { health: game.health.max(0), bolts: self.shown_bolts as u32, clock: self.clock, alpha: (bolt_alpha, 0.0), nano: (self.nano_hud.1, self.nano_hud.2), ammo, reticle, rc_static: game.rc.alpha.map(|a| (a, game.rc.tiles.clone())), marker: (game.marker_hud.a, game.marker_hud.b, game.frames), breath: game.breath_hud.snapshot(), counters: [game.count_hud[0].snapshot(), game.count_hud[1].snapshot()], sprites, pad: game.pad_hud.clone() };
					if let Some(g) = &mut self.gpu {
						hud(&mut g.hud, g.config.width as f32, g.config.height as f32, Some(st), &self.messages, self.flow.as_ref(), &self.art, self.settings.subtitles);
						// NOV-28 (water.md): last frame's fog swap goes live, the camera zone fog updates, then the
						// next swap is chosen (underwater fog 0..32 m, intensity 255 -> 48); the tint precedes the fade.
						g.fog_live = self.fog_next;
						g.part_far = self.part_far_next;
						if let Some(lt) = &g.lighting {
							lt.zone_fog(self.cam.pos, &mut self.fog_zone);
						}
						let (tint, under_fog) = self.water.as_ref().map_or(([0x18, 0x30, 0x90, 0x40], [0x18, 0x30, 0x90]), |w| w.submerged());
						self.fog_next = Some(if self.listener.underwater { (under_fog, crate::water::UNDERWATER_FOG) } else { self.fog_zone });
						// NOV-28.2: pool particles end at 64 m underwater, 500 m dry (one frame late, as the fog).
						self.part_far_next = if self.listener.underwater { crate::water::PART_FAR_UNDER } else { crate::water::PART_FAR };
						if self.listener.underwater {
							// NOV-28.3: the target is gamma-space Unorm, so the tint goes as raw GS bytes.
							overlay::rect(&mut g.hud, [0.0, 0.0, g.config.width as f32, g.config.height as f32], crate::water::tint_colour(tint));
						}
						let fade = game.fade().max(self.flow.as_ref().and_then(|f| f.exit_fade_dark()).unwrap_or(0.0));
						if fade > 0.0 {
							overlay::rect(&mut g.hud, [0.0, 0.0, g.config.width as f32, g.config.height as f32], [0.0, 0.0, 0.0, fade]);
						}
						if let Some(t) = &self.travel {
							t.draw(&mut g.hud, g.config.width as f32, g.config.height as f32);
						}
						if let Some(p) = &self.ship_play {
							p.draw(&mut g.hud, g.config.width as f32, g.config.height as f32);
						}
						let (w, h) = (g.config.width as f32, g.config.height as f32);
						if let Some(font) = &self.art.font {
							if let (Some((id, _, alpha)), Some(text)) = (self.popup.filter(|_| !paused), &self.ui_text) {
								popup(&mut g.hud, w, h, &self.art, font, text.get(id), alpha as f32 / 128.0);
							}
							if self.wheel_fade.0 > 0.0 {
								let sel = self.wheel_fade.1;
								let names: Vec<String> = game.inv.quick.iter().map(|&id| if id == 0 { String::new() } else { self.ui_text.as_ref().map_or(String::new(), |t| t.get(crate::inventory::name_text(id)).to_string()) }).collect();
								wheel(&mut g.hud, w, h, &self.art, font, &game.inv, &names, sel, self.wheel_fade.0, self.clock);
							}
						}
						if self.save_icon > 0.0 {
							save_icon(&mut g.hud, w, h, &self.art, self.clock);
						}
						g.preview = None;
						g.preview2 = None;
						g.preview_pair = None;
						g.preview_worn = None;
						// The vendor's shop screen and its spinning item (yaw += 0.05 a tick = 2.5 rad/s).
						g.hud_split = None;
						g.popup_bolt = game.shop_bolt().map(|(pos, rot)| place(pos, rot));
						// NOV-30d: the slot-0 bolt icon of the item target.
						g.icon_bolt = game.shop_icon_bolt().map(|(pos, rot)| place(pos, rot));
						// NOV-35b / BRIDGE-45: the bolts' draw spheres for their metal alpha and env matrix.
						[g.icon_bolt_sphere, g.popup_bolt_sphere] = game.shop_bolt_spheres();
						// BRIDGE-37 (b): the shop portrait actor posed for its target (never in the world pass).
						let portrait = game.shop.as_ref().and_then(|s| s.portrait.as_ref()).and_then(|p| {
							let (class, a) = self.prop_anims.get(&game::vendor::PORTRAIT_CLASS)?;
							Some((anim::skin(class, &p.anim.pose(a, &[])), p.pos, p.rot))
						});
						g.pose_portrait(portrait.as_ref().map(|(p, pos, rot)| (p, *pos, *rot)));
						if let (Some(sh), Some(font), Some(text)) = (&game.shop, &self.art.font, &self.ui_text) {
							// The six screens' corner joints (0,1,2, 4,5,6, .. 20,21,22) of the vendor model, projected (shop.rs).
							let joints: Option<Vec<[f32; 3]>> = game.props.iter().enumerate().find(|(_, p)| p.class_id == 11 && p.shown).and_then(|(pi, p)| {
								let (_, a) = self.prop_anims.get(&11)?;
								// NOV-30c: the icon bar corners (panel 5) narrowed by the shop's joint records.
								let mats = p.anim.pose_fx_full(a, &[], &game.shop_bar_fx(pi, &a.slot_map));
								let m = place(p.pos, p.rot);
								Some((0..36).map(|j| {
									let jm = a.joint_world(&mats, j.min(mats.len() - 1));
									let l = [jm[3] * p.scale, jm[7] * p.scale, jm[11] * p.scale];
									[0, 1, 2].map(|i| m[i] * l[0] + m[4 + i] * l[1] + m[8 + i] * l[2] + m[12 + i])
								}).collect())
							});
							let vp = view_proj(&self.cam, w / h, RETAIL_NEAR, RETAIL_FAR);
							let rects = joints.map(|j| crate::shop::screen_rects(&j, &|p| project(&vp, p).map(|c| [(c[0] + 1.0) * 0.5 * w, (1.0 - c[1]) * 0.5 * h])));
							let (items, ammo) = (&self.art.items, &self.art.ammo_items);
							let icon = |id: u8, a: bool| if a { ammo.get(&id) } else { items.get(&id) }.map(|i| i.0);
							// UI-5d / NOV-35a: the purchase prompt's attach points 0 / 1 / 2 (FUN_002b3370 list 0x1610f8 through
							// FUN_002641c8; class 1137: joints 2 / 1 / 4), with the same transform as the vendor's, projected to the popup rect.
							let popup = game.props.iter().find(|p| p.class_id == game::vendor::PROMPT_CLASS && p.shown).and_then(|p| {
								let (_, a) = self.prop_anims.get(&game::vendor::PROMPT_CLASS)?;
								let joints = [a.attach0?, a.attach12[0]?, a.attach12[1]?];
								let mats = p.anim.pose(a, &[]);
								if joints.iter().any(|&j| j >= mats.len()) {
									return None;
								}
								let m = place(p.pos, p.rot);
								let j = joints.map(|j| {
									let jm = a.joint_world(&mats, j);
									let l = [jm[3] * p.scale, jm[7] * p.scale, jm[11] * p.scale];
									[0, 1, 2].map(|i| m[i] * l[0] + m[4 + i] * l[1] + m[8 + i] * l[2] + m[12 + i])
								});
								crate::shop::popup_rect(&j, &|p| project(&vp, p).map(|c| [(c[0] + 1.0) * 0.5 * w, (1.0 - c[1]) * 0.5 * h]))
							});
							g.hud_split = crate::shop::draw(&mut g.hud, w, h, self.art.font2.as_ref().unwrap_or(font), text, sh, game.bolts, &game.inv, rects, &icon, Some(ITEM_PANEL_TEX), Some(PORTRAIT_PANEL_TEX), self.art.frame.map(|f| [f[1], f[2], f[3]]), self.art.led, popup, Some(POPUP_PANEL_TEX));
						}
						// NOV-13.5: the gadget cinematic's black (fanfare) and its end fade.
						if let Some(f) = self.gadget_frame.filter(|f| f.fade > 0.0) {
							overlay::rect(&mut g.hud, [0.0, 0.0, w, h], [0.0, 0.0, 0.0, f.fade]);
						}
						// NOV-13.3: the shop item models in the world (static slot and turntable slot).
						(g.shop_models, g.shop_panel_models) = split_item_poses(&game.shop_item_poses());
if let (Some(m), Some(a), Some(text)) = (&self.pause, &self.ui_art, &self.ui_text) {
							// The frozen world shows behind the menu under FUN_001f9fd0(0,0,0,0x30): black at 0x30/0x80
							// (retail PCSX2 captures; decomp/notes/ingame_pause_menu.md). The HUD is hidden.
							g.hud.clear();
							g.hud_split = None;
							overlay::rect(&mut g.hud, [0.0, 0.0, w, h], [0.0, 0.0, 0.0, 0.375]);
							m.draw(&mut g.hud, w, h, a, text);
// Selected item model in panel 3 (frontend.rs preview; turntable camera [P]).
let (pk, pox) = (h / 448.0, (w - 512.0 * h / 448.0) / 2.0);
let gadgets_page = m.page == frontend::Page::Gadgets;
g.preview = m.preview().and_then(|(id, r, t)| {
	// Gadgets: the cursor item's class (0x1df for 0x18), drawn only when owned; the other pages keep the held class.
	let class = if gadgets_page { crate::inventory::panel_class(id).filter(|_| m.gadget_owned(id))? } else { crate::inventory::info(id)?.held_class };
	Some((class, [pox + r[0] * pk, r[1] * pk, r[2] * pk, r[3] * pk], t, Some(id)))
});
g.preview_gold = g.preview.and_then(|p| p.3).is_some_and(|id| gold_lit(id, &m.ingame.golds));
// Category-3 ids also draw class 601 with the same placement.
g.preview_pair = g.preview.filter(|p| gadgets_page && p.3.is_some_and(|id| crate::inventory::category(id) == Some(3))).map(|p| (601, p.1, p.2, p.3));
// Panel models play sequence 1 (6 for item 2), clamped to the class's sequence count.
if self.panel_model.map(|p| p.0) != g.preview.map(|p| p.0) {
	self.panel_model = g.preview.map(|p| (p.0, self.clock));
}
let panel_start = self.panel_model.map_or(self.clock, |p| p.1);
for (class, id) in [g.preview, g.preview_pair].into_iter().flatten().filter_map(|p| Some((p.0, p.3?))) {
	if let Some((c, ia)) = self.menu_actor_classes.get(&class).filter(|(_, ia)| !ia.sequences.is_empty()) {
		let seq = (if id == 2 { 6 } else { 1 }).min(ia.sequences.len() - 1);
		let (ps, ns) = anim::skin(c, &ia.pose_after(seq, (self.clock - panel_start) * 50.0));
		g.set_preview_pose(class, &ps, &ns);
	}
}
g.preview2 = m.hero_preview().map(|(id, r)| {
	let key = if gpu_has_preview(&g.scene, HERO_PREVIEW + id as u32) { HERO_PREVIEW + id as u32 } else { HERO_PREVIEW };
	(key, [pox + r[0] * pk, r[1] * pk, r[2] * pk, r[3] * pk], 0.0)
}).or_else(|| m.gold_preview().map(|r| (GOLD_BOLT_PREVIEW, [pox + r[0] * pk, r[1] * pk, r[2] * pk, r[3] * pk], self.clock)));
// The hero plays the selected item's menu clip (table 0x1b93f0), the item following its joint.
match (m.hero_preview(), &self.hero_anim, self.menu_clip_base) {
	(Some((id, _)), Some((hc, ha)), Some(base)) => {
		if self.hero_preview_clip.as_ref().map(|c| c.id) != Some(id) {
			// Entering a clip: fade from the current pose (FUN_002575d8(hero, clip, 0, 10, 5)).
			let mut pa = self.hero_preview_clip.take().map_or_else(|| anim::PropAnim::new(IDLE), |m| m.pa);
			if !pa.attached() {
				pa.attach(Some(anim::share_timing(&ha.sequences)));
			}
			pa.blend_ex(menu_clip(base, id).map_or(IDLE, |c| c.0), 0, 10, 5);
			// The item's own clock: its requested itemseq (hero record +0x10) at frame 0, no blend.
			let item = self.hero_items.get(&id).and_then(|it| self.menu_actor_classes.get(&it.class)).map(|(_, ia)| {
				let mut ipa = anim::PropAnim::new(MENU_CLIP_ROWS.get(id as usize).map_or(1, |r| r.2 as usize));
				ipa.attach(Some(anim::share_timing(&ia.sequences)));
				ipa.set(ipa.seq, 0);
				ipa
			});
			self.hero_preview_clip = Some(MenuHero { id, pa, item, start: self.clock, ticks: 0, plays: 0, second: false });
		}
		let worn = game.inv.worn;
		let feet = worn[1] != 0;
		let ticks_before = self.hero_preview_clip.as_ref().map_or(0, |m| m.ticks);
		let (mats, start, ticks_now) = {
			let m = self.hero_preview_clip.as_mut().expect("preview clip set above");
			let target = ((self.clock - m.start).max(0.0) * 50.0) as u32;
			while m.ticks < target {
				m.ticks += 1;
				m.pa.step();
				// The item's anim ended on an itemseq other than 1: back to sequence 1, frame 0, no blend.
				if let Some(ipa) = &mut m.item {
					ipa.step();
					if ipa.wrapped && ipa.seq != 1 {
						ipa.set(1, 0);
					}
				}
				// The clip plays `loops` times, then clip2 fades in and loops.
				if let Some((_, loops, clip2)) = menu_clip(base, id).filter(|c| c.1 > 0 && !m.second) {
					if m.pa.wrapped && !m.pa.fading() {
						m.plays += 1;
						if m.plays >= loops as u32 {
							m.second = true;
							m.pa.blend_ex(clip2, 0, 10, 5);
						}
					}
				}
			}
			// Boots worn: the hero's own foot joints (attach points 0x16 / 0x17) collapse under them.
			let scales: Vec<(usize, [f32; 3])> = if feet { self.hero_worn.scale_joints.iter().flatten().map(|&j| (j, [0.0; 3])).collect() } else { Vec::new() };
			(m.pa.pose_scaled(ha, &scales), m.start, m.ticks)
		};
		let hero_local = self.hero_preview_clip.as_ref().map(|m| m.pa.local_trs(ha)).unwrap_or_default();
		let (mut ps, mut ns) = anim::skin(hc, &mats);
		let key = if gpu_has_preview(&g.scene, HERO_PREVIEW + id as u32) { HERO_PREVIEW + id as u32 } else { HERO_PREVIEW };
		if let Some(it) = self.hero_items.get(&id).filter(|_| key != HERO_PREVIEW) {
			let mut wj = ha.joint_world(&mats, it.joint);
			if it.mode != 0 {
				wj = normalise_columns(wj);
			}
			// Hand (category 0) and head (category 2) items with their own skeleton are skinned every tick:
			// mode 0 from the pose-copy frame of the hero's live joints, otherwise from their itemseq clock.
			let cat = crate::inventory::category(id);
			let own = self.menu_actor_classes.get(&it.class).filter(|(c, _)| matches!(cat, Some(0) | Some(2)) && c.mesh.positions.len() == it.mesh.positions.len());
			let (ip, inn) = match (own, self.hero_preview_clip.as_ref().and_then(|m| m.item.as_ref())) {
				// Feet and back items are drawn by the worn batch (UI-3b); their slot in this mesh collapses.
				_ if matches!(cat, Some(1) | Some(3)) => (vec![[0.0; 3]; it.mesh.positions.len()], vec![[0.0, 0.0, 1.0]; it.mesh.positions.len()]),
				(Some((c, ia)), Some(ipa)) => {
					let mats_i = if it.mode == 0 && !hero_local.is_empty() {
						let f = match (cat, it.class) {
							(Some(2), 433) => anim::pose_copy_frame(&hero_local, &anim::HEAD_COPY_433, anim::HEAD_COPY_433_EXTRA, &ia.skeleton.bind_translation),
							(Some(2), _) => anim::pose_copy_frame(&hero_local, &anim::HEAD_COPY, 0, &ia.skeleton.bind_translation),
							_ => anim::pose_copy_frame(&hero_local, &anim::HAND_COPY, 0, &ia.skeleton.bind_translation),
						};
						ia.pose_single(&f)
					} else {
						ipa.pose(ia, &[])
					};
					anim::skin(c, &mats_i)
				}
				_ => (it.mesh.positions.clone(), face_normals(&it.mesh)),
			};
			ps.extend(ip.iter().map(|&v| apply(&wj, v, 1.0)));
			ns.extend(inn.into_iter().map(|n| apply(&wj, n, 0.0)));
		}
		// Extra actors start their sequence when the item is selected (trigger 0) and loop [I].
		for &(class, seq, scale) in self.hero_actors.get(&id).filter(|_| key != HERO_PREVIEW).into_iter().flatten() {
			if let Some((c, a)) = self.menu_actor_classes.get(&class) {
				let (p, n) = anim::skin(c, &a.pose_after(seq, (self.clock - start) * 50.0));
				ps.extend(p.into_iter().map(|v| v.map(|x| x * scale)));
				ns.extend(n);
			}
		}
		g.set_preview_pose(key, &ps, &ns);
		// UI-3b: the worn mobies. The back item's clock steps with the hero clip; a changed back item respawns it.
		let back_class = (worn[3] != 0).then(|| crate::inventory::preview_class(worn[3])).flatten();
		if self.worn_back.as_ref().map(|b| b.class) != back_class {
			self.worn_back = back_class.and_then(|c| self.menu_actor_classes.get(&c)).map(|(_, ia)| anim::BackAnim::new(back_class.unwrap_or(0), anim::share_timing(&ia.sequences)));
		}
		if let Some(b) = &mut self.worn_back {
			for _ in ticks_before..ticks_now {
				b.step();
			}
		}
		if self.hero_worn.total > 0 {
			let (mut wp, mut wn) = (vec![[0.0f32; 3]; self.hero_worn.total], vec![[0.0, 0.0, 1.0]; self.hero_worn.total]);
			let feet_class = crate::inventory::preview_class(worn[1]).filter(|_| feet);
			for part in &self.hero_worn.parts {
				let on = match part.kind {
					WornKind::BootA | WornKind::BootB => feet_class == Some(part.class),
					WornKind::Back => back_class == Some(part.class),
					WornKind::Extra(item) => game.inv.have.get(item as usize).copied().unwrap_or(false),
				};
				if !on {
					continue;
				}
				let wj = normalise_columns(ha.joint_world(&mats, part.joint));
				let own = self.menu_actor_classes.get(&part.class).filter(|(c, _)| c.mesh.positions.len() == part.mesh.positions.len());
				let (ip, inn) = match (own, part.kind) {
					(Some((c, ia)), WornKind::BootA | WornKind::BootB) if !hero_local.is_empty() => {
						let list = &BOOT_COPY[(part.kind == WornKind::BootB) as usize];
						anim::skin(c, &ia.pose_single(&anim::pose_copy_frame(&hero_local, list, 0, &ia.skeleton.bind_translation)))
					}
					(Some((c, ia)), WornKind::Back) => match &self.worn_back {
						Some(b) => anim::skin(c, &b.pa.pose(ia, &[])),
						None => anim::skin(c, &ia.pose_after(1, 0.0)),
					},
					(Some((c, ia)), _) => anim::skin(c, &ia.pose_after(0, (self.clock - start) * 50.0)),
					_ => (part.mesh.positions.clone(), face_normals(&part.mesh)),
				};
				for (k, (p, n)) in ip.iter().zip(&inn).enumerate() {
					wp[part.range.start + k] = apply(&wj, *p, 1.0);
					wn[part.range.start + k] = apply(&wj, *n, 0.0);
				}
			}
			g.set_preview_pose(HERO_WORN, &wp, &wn);
			g.preview_worn = g.preview2.map(|(_, r, t)| (HERO_WORN, r, t, None));
		}
	}
	_ => {
		self.hero_preview_clip = None;
		self.worn_back = None;
		g.preview_worn = None;
	}
}
							if m.page == frontend::Page::Map {
								if let Some(mv) = &self.map {
									map_view(&mut g.hud, w, h, mv, self.art.marker, self.clock);
								}
							}
						}
						profile::lap(profile::Phase::Hud);
						g.points = [[0.0; 4]; 16];
						// Retail mode: the slots come from retail RAM (retail.rs `dyn_lights`). A slot keeps its serial while it
						// stays used; one freed and re-allocated between two snapshots keeps it too [I: not seen by the mirror].
						let retail_lights = self.retail.as_ref().map(|r| r.fx().map_or([None; 8], |fx| fx.lights));
						g.reserved_lights = self.retail.as_ref().and_then(|r| r.fx().and_then(|fx| fx.slots));
						let lights = retail_lights.unwrap_or_else(|| self.particles.point_lights());
						for (k, l) in lights.iter().enumerate() {
							if let Some(l) = l {
								g.points[2 * k] = [l.pos[0], l.pos[1], l.pos[2], l.radius];
								g.points[2 * k + 1] = [l.rgb[0], l.rgb[1], l.rgb[2], l.intensity];
							}
						}
						g.table = match retail_lights {
							Some(ls) => {
								let next = g.table.iter().flatten().map(|t| t.serial).max().unwrap_or(0) + 1;
								std::array::from_fn(|k| ls[k].map(|light| dynlight::SlotLight { light, serial: g.table[k].map_or(next + k as u32, |t| t.serial) }))
							}
							None => self.particles.light_table(),
						};
						let view = dynlight::View::from_matrix(&view_proj(&self.cam, g.config.width as f32 / g.config.height.max(1) as f32, RETAIL_NEAR, RETAIL_FAR));
						g.relight(Some(&view));
						// The nozzle's texture scroll (FUN_00263928): its vertices precede the Suck Cannon mesh at the model's end.
						// BRIDGE-40: the three hero afterimage copies close the model.
						let ghost_tail = self.hero_anim.as_ref().map_or(0, |(c, _)| hero_ghost_len(c));
						let tail = self.suck_rig.as_ref().map_or(0, |s| s.held.class.mesh.positions.len()) + held_len(&self.held_rigs, &self.hook_rig) + back_tail(&self.back_rig, &self.clank_rig, &self.light_rig) + ghost_tail;
						g.marker_v = nozzle.zip(self.pyro_rig.as_ref().and_then(|r| r.nozzle.as_ref())).zip(skinned.as_ref()).map(|((z, r), (ps, _))| (ps.len() - tail - r.1.len(), ps.len() - tail, z.v as f32 / 4096.0));
						g.marker_ghosts = ghost_ranges;
						g.marker_light = self.light_rig.as_ref().zip(light_at).zip(skinned.as_ref()).map(|((lc, _), (ps, _))| (ps.len() - ghost_tail - lc.mesh.positions.len(), ps.len() - ghost_tail, game::clank_light_rgb(game.frames)));
						g.marker_bomb = bomb_range.zip(game.bomb_held_tint()).map(|((a, b), t)| (a, b, t));
						// BRIDGE-75: the hero body tilt for this frame's marker.
						g.marker_tilt = tilt;
						// REN-CULL-OCC-M1: the Visibomb launch tick renders with occlusion fallback mode 1.
						if let Some(s) = g.scene.as_mut() {
							s.occ.mode = game.vis_render.occ_mode;
						}
						g.set_clip_view(Some((&self.cam, g.config.width as f32 / g.config.height.max(1) as f32)));
						g.hero_gens = hero_gens;
						g.set_marker(if game.hero_hidden && !passenger_shown { None } else { at }, skinned.as_ref(), hero_blend_t.and_then(|t| self.smooth.hero_before().map(|b| (b, t))));
						// NOV-10.5a-draw: the teleporter rings as debris-batch instances, the beams as additive fx quads.
						let rings: Vec<particles::WorldInst> = Vec::new();
						g.scene_actor_start(actors.iter().any(|a| a.is_some()), game.player.pos);
						PROP_TINTS.with(|t| {
							*t.borrow_mut() = (0..game.props.len())
								.map(|i| {
									let Some((class, _)) = self.prop_anims.get(&game.props[i].class_id) else {
										// Retail mode: a static class (no animation, so not in `prop_anims`) takes the tail colour too.
										let start = crate::retail::queue_q2::static_tint_start(game.props[i].class_id as u32)?;
										return game.props[i].retail_tint().flatten().map(|c| (c, start));
									};
									// Retail mode: the moby's own +0x90 word (class +0x40 at init, actor overrides) when +0x34 & 0x10.
									if let Some(t) = game.props[i].retail_tint() {
										return t.map(|c| (c, class.colour_start));
									}
									Some((mobies::glow_rgb(class.glow)?, class.colour_start))
								})
								.collect();
						});
						g.prop_v = None;
						g.class_uv = self.retail.as_ref().and_then(|r| r.fx()).map(|fx| fx.class_uv.iter().copied().collect()).unwrap_or_default();
						let (fade_c, fade_r) = (game.fade_centres(), game.fade_radii());
						g.set_props(Some(&game.props), self.smooth.posed(), &fx_posed, &actors, &self.debris, &rings, &fade_c, &fade_r);
						// BRIDGE-64a: each caster's sphere primitives with their joint applied (model space, class scale).
						let casters: Vec<_> = Vec::<(usize, [f32; 2], u8)>::new()
							.into_iter()
							.filter_map(|(pi, band, dist)| {
								let p = &game.props[pi];
								let (class, a) = self.prop_anims.get(&p.class_id)?;
								if !class.shadow_prims.iter().any(|s| matches!(s, mobies::ShadowPrim::Sphere { .. })) {
									return None;
								}
								let mats = p.anim.pose(a, &[]);
								let spheres = class
									.shadow_prims
									.iter()
									.filter_map(|s| match *s {
										mobies::ShadowPrim::Sphere { joint, n, at, r } if joint < mats.len() => {
											let j = a.joint_world(&mats, joint);
											let c = std::array::from_fn(|k| j[4 * k] * at[0] + j[4 * k + 1] * at[1] + j[4 * k + 2] * at[2] + j[4 * k + 3]);
											Some(crate::shadows::VolumeSphere { c, r, n })
										}
										_ => None,
									})
									.collect();
								Some((pi, spheres, band, dist))
							})
							.collect();
						g.set_volumes(&game.props, &casters, &fade_c, &fade_r);
						// M3a shadows: in retail mode the volumes come from retail's own shadow records (hero, enemies, every caster).
						if let Some((recs, late, hero_late)) = self.retail.as_ref().and_then(|r| r.shadows()) {
							g.set_volumes_retail(&recs, &late, hero_late);
						}
						// BRIDGE-33: the draw verdict feeds the next update's scheduler override (FRAME-1a: per tick while the rig owns the view).
						if !self.tick_cam_live {
							game.set_draw_verdict(g.props_drawn());
						}
						for f in &self.fires {
							f.quads(self.cam.pos, self.particles.fire_scroll(), &mut world_quads);
						}
						world_quads.extend(game::reticle_quads(&game.reticles, self.cam.pos, game.frames));
						world_quads.extend(game::reticle_quads(&game.mine_reticles, self.cam.pos, game.frames));
						world_quads.extend(game::reticle_quads(&game.doom_reticles, self.cam.pos, game.frames));
						// Retail mode: retail's queued landing markers and the queue-A / queue-B callbacks (retail.rs `queue_fx`).
						if let Some(fx) = self.retail.as_ref().and_then(|r| r.fx()) {
							world_quads.extend(game::reticle_quads_placed(&fx.reticles, fx.frames));
							for s in &fx.ship_shadows {
								let z = match (&self.world, s.ground) {
									(Some(w), true) => crate::shadows::ship_shadow_z(s.pos, s.m, s.ship, s.alpha, crate::shadows::ship_ground_z(w, s.pos)),
									(None, true) => crate::shadows::ship_shadow_z(s.pos, s.m, s.ship, s.alpha, 0.1),
									(_, false) => crate::shadows::ship_shadow_z(s.pos, s.m, s.ship, s.alpha, s.pos[2] + 0.1),
								};
								world_quads.extend(z);
							}
							// The Swingshot rope at its queue-A slot, from the held Swingshot's posed attach point 0 (FUN_002519d0(held, 0)).
							let (qa0, qa1) = fx.qa.split_at(fx.rope.map_or(fx.qa.len(), |r| r.0.min(fx.qa.len())));
							world_quads.extend(qa0.iter().copied());
							if let (Some((_, r)), Some(jt)) = (fx.rope, self.held_muzzle) {
								world_quads.extend(crate::retail::queue_b::rope_quads(&r, jt));
							}
							world_quads.extend(qa1.iter().copied());
							world_quads.extend(fx.qb.iter().copied());
						}
						// Retail shop: the hologram beam over the pad (retail.rs `shop_beam`, FUN_00290778).
						if let Some(sn) = self.retail.as_ref().and_then(|r| r.shop()) {
							world_quads.extend(sn.beam.iter().copied());
						}
						world_quads.extend(game.morph_beam_quads(self.cam.pos));
						// BRIDGE-76 (ARI-47c): the Swingshot rope ribbon.
						world_quads.extend(game.swing_rope_quads(self.cam.pos));
						world_quads.extend(game.taunt_ring_quads());
						world_quads.extend(game.tube_draw.iter().copied());
						world_quads.extend(game.debris_draws.iter().map(|&(p, r)| game::jet_debris_quad(self.cam.pos, p, r)));
						world_quads.extend(self.gloss_quads.iter().copied());
						let mut water_quads = Vec::new();
						if let Some((light, patches)) = self.retail.as_ref().and_then(|r| r.fx()).and_then(|fx| fx.water.as_ref()) {
							// Retail mode: the patch state lives in retail RAM (the old water tick never runs).
							let ps: Vec<crate::water::Patch> = patches.iter().map(|p| p.patch.clone()).collect();
							crate::water::emit_quads(&ps, &|k| &patches[k].grid[..], *light, self.cam.pos, &mut water_quads);
						} else if let Some(w) = &self.water {
							w.quads(self.cam.pos, &mut water_quads);
						}
						// Retail mode: the queue-B water callbacks (Drek's Fleet hangar water) draw with the fogged alpha quads.
						if let Some(fx) = self.retail.as_ref().and_then(|r| r.fx()) {
							water_quads.extend(fx.qw.iter().copied());
						}
						g.water_quads = water_quads;
						world_quads.extend(vendor_quads);
						// PC-FPS-1 / FRAME-1: off-view glints are freed (game state) once per tick with the tick camera, then drawn (pure).
						// BRIDGE-56: the scene tick camera has no roll; the rig's tick camera carries `glint_up`.
						let gup = if self.smooth.tick_cam.is_some() { [0.0, 0.0, 1.0] } else { self.glint_up };
						let gcam = self.smooth.tick_cam.or(self.glint_cam).map(|(pos, yaw, pitch, tan_half)| Camera { pos, yaw, pitch, tan_half, up: gup });
						let gcam = gcam.as_ref().unwrap_or(&self.cam);
						let f=forward(gcam);
						let right=crate::physics::normalize(crate::physics::cross(f,gcam.up));
						let up=crate::physics::cross(right,f);
						if self.smooth.ticks > 0 && !self.tick_cam_live {
							game.glint_cull(gcam.pos, right, up, f);
						}
						// Retail mode: the glint pool as retail's RAM holds it (retail/oldsim.rs); the old sim spawns none there.
						let retail_glints = self.retail.as_ref().and_then(|r| r.fx()).map(|fx| fx.glints);
						particles::glint_quads(retail_glints.as_ref().unwrap_or(&game.glints),gcam.pos,right,up,f,self.glint_u,&mut world_quads);
						g.fx_quads = std::mem::take(&mut world_quads);
						g.strip_draws = game.strip_out.clone(); // BRIDGE-23 / NOV-15.1b, NOV-26 per-tick list
						// Retail mode: the Novalis strip-surface callbacks of queue A (retail/queue_q1.rs).
						// Retail mode, Hoven: the queue-C water pools, drawn in their own slot before the shrubs.
						// [PC] Water hold (lane X pass 2): a field whose effect lists come up empty (the efx job's reply late, hold / lag fields, the
						// vendor screen) keeps the last non-empty pool strips and RAM oceans of the SAME level for `WATER_HOLD` fields, so the sea does
						// not vanish for a few display frames; a level change or a longer absence clears them. While the vendor screen is up the
						// hold has no time limit: retail stops sending the Hoven sea pools there after a few seconds (user's water_hoven_1.log,
						// 2026-10-20: 71 / 605 / 162 display frames with 0 water draws, all inside game mode 5), the world behind stands still.
						let rfield = self.retail.as_ref().map_or(0, |r| r.field_count());
						let vendor = self.retail.as_ref().is_some_and(|r| r.vendor());
						let (has_pools, has_oceans) = self.retail.as_ref().and_then(|r| r.fx()).map_or((false, false), |fx| (!fx.pools.is_empty(), !fx.oceans.is_empty()));
						let (hold_pools, hold_oceans) = WATER_KEEP.with(|k| {
							let mut k = k.borrow_mut();
							if k.0 != g.level {
								*k = (g.level, 0, 0);
							}
							if has_pools {
								k.1 = rfield.max(1);
							}
							if has_oceans {
								k.2 = rfield.max(1);
							}
							let fresh = |t: u32| t != 0 && (vendor || rfield.wrapping_sub(t) <= WATER_HOLD);
							(!has_pools && fresh(k.1), !has_oceans && fresh(k.2))
						});
						if !hold_pools {
							g.pool_draws.clear();
						}
						if let Some(fx) = self.retail.as_ref().and_then(|r| r.fx()) {
							g.strip_draws.extend(fx.strips.iter().cloned());
							g.pool_draws.extend(fx.pools.iter().cloned());
						}
						// ARI-49b / BRIDGE-63: Aridia ocean patches (pure), texture re-blended once per game frame (50 Hz tick).
						// KER-30: the Kerwan ocean is absent while `frames` < 2 (its moby's init tick).
						// Retail mode: Batalia / Oltanis come from RAM while their wrapper is queued (queue_q1::ocean, descriptor fog); the others
						// below. The level frame counter is `game.frames` (= RAM 0x15f5ac, set from `RetailFx.frames` each tick in the retail branch).
						let ram_ocean = self.retail.as_ref().and_then(|r| r.fx()).and_then(|f| f.ocean.as_deref());
						g.set_ocean(ram_ocean.or(self.ocean.as_ref().filter(|o| o.shown(game.frames))), self.cam.pos, &self.cuboids, game.vis_render.occ_mode != 0, game.frames, game.visibomb.is_some());
						// Retail mode: the oceans of Rilgar / Umbris / Gaspar from RAM (retail/oldsim.rs `oceans`; never the levels of `queue_q1::ocean`).
						if !hold_oceans {
							g.set_ocean_ram(self.retail.as_ref().and_then(|r| r.fx()).map_or(&[][..], |fx| &fx.oceans[..]), self.cam.pos);
						}
						g.set_sky_spin(self.level, game.frames); // NOV-31: the Novalis sky shells turn per game frame
						// ARI-4: the held sky sprites of the newest tick (no rand() here); [PC] x cull with the port's horizontal tangent.
						let tan_x = gcam.tan_half * g.config.width as f32 / g.config.height.max(1) as f32;
						// Retail mode: the sprite records retail's sky pass keeps in RAM (retail/oldsim.rs); the old stage never ticks there.
						let sprites = match self.retail.as_ref().and_then(|r| r.fx()) {
							Some(fx) => &fx.sky_sprites[..],
							None => self.sky_sprites.as_deref().unwrap_or(&[]),
						};
						g.set_sky_sprites(sprites, [right, up, f], right, up, tan_x);
						g.set_particles(&self.sprites, &self.ribbons, &self.lines, &self.effect_mobys, &self.cam, self.retail.as_ref().and_then(|r| r.fx()));
						// [PC] PREVIEW (vegfx.rs): the hero and the drawn mobys bend the extra plants.
						g.veg.actors(self.retail.as_ref().and_then(|r| r.fx()).and_then(|f| f.hero), self.retail.as_ref().map_or(&[][..], |r| r.live_mobys()).iter());
						// [PC] A Moderate <-> Lush change of the Vegetation row replans the plants on a thread (vegfx.rs).
						g.veg_follow(self.world.as_ref());
						if title != self.title {
							g.window.set_title(&title);
							self.title = title;
						}
					}
				}
				// PC-FPS-1: past the world draw lists the scene camera of the true tick is put back (the sound below and the
				// next game ticks never see the drawn, offset one); the final draw takes the drawn one.
				profile::lap(profile::Phase::Upload);
				let drawn_cam = self.smooth.tick_cam.take().map(|(pos, yaw, pitch, tan_half)| {
					let d = Camera { pos: self.cam.pos, yaw: self.cam.yaw, pitch: self.cam.pitch, tan_half: self.cam.tan_half, up: self.cam.up };
					self.cam = Camera { pos, yaw, pitch, tan_half, up: [0.0, 0.0, 1.0] };
					d
				});
				if let Some(a) = &mut self.audio {
					// Retail mode: every sound effect is the game's own 989snd request (retail/snd.rs, played in the tick). The
					// mirrored Game's own sound sources stay silent: its bolt-pickup edge (a mirrored bolt prop hiding), hero clock
					// cues (the heli-pack / jump triggers of the clock retail drives), footstep cues, class sounds and loops, enemy
					// cues and the weapon loops would otherwise play a second copy of retail's sounds, and keep playing it while
					// the mirror holds a state (2026-10-12: bolt and heli sounds heard over and over).
					let native_sfx = self.retail.is_none();
					if !native_sfx {
						sounds.clear();
						loop_cues.clear();
						shots.clear();
						self.loop_want.clear();
						self.loop_bend.clear();
					}
					// A scene's speech starts with it and stops when it ends or is skipped.
					let scene_id = self.flow.as_ref().and_then(|f| f.scene_time()).map(|s| s.0);
					if scene_id != self.scene_voice.map(|v| v.0) {
						if let Some((_, Some(id))) = self.scene_voice.take() {
							a.stop(id);
						}
						self.scene_voice = scene_id.map(|id| (id, self.scenes.get(&id).and_then(|(_, c)| c.as_ref()).map(|c| a.play(c, 1.0, 0.0))));
					}
					for (clip, volume, pan, shift, privileged) in sounds {
						if let Some(c) = clip.and_then(|i| self.sfx.clips.get(i)).and_then(|c| c.as_ref()) {
							a.play_with(c, volume, pan, sound::pitch_ratio(shift), privileged);
						}
					}
					// NOV-6.3: start a loop only when its owner has no live voice; stop and forget on `off`.
					// AUD-11i: `on`/`off` events set and clear the wanted-loop table; each frame every wanted loop is
					// started (gated by distance, retried when dropped), updated at its owner's position + (0,0,1) or
					// stopped when out of range (FUN_002fb1d8 / FUN_0030d870 re-request, FUN_0029ff68 update).
					for (owner, cue, on) in loop_cues {
						if !on {
							self.loop_want.remove(&owner);
							self.loop_bend.remove(&owner);
							if let Some(id) = self.loops.remove(&owner) {
								a.stop(id);
							}
						} else if let Some((_, c)) = cue {
							self.loop_want.insert(owner, c);
						}
					}
					if !paused {
						if let Some(game) = self.game.as_ref() {
							let mut wanted: Vec<(usize, sound::Cue)> = self.loop_want.iter().map(|(o, c)| (*o, *c)).collect();
							wanted.sort_by_key(|w| w.0);
							for (owner, c) in wanted {
								let Some(at) = loop_at(owner, &game.props, game.player.pos, game.visibomb.as_ref().map(|m| m.pos)) else { continue };
								let live = self.loops.get(&owner).is_some_and(|id| *id != 0 && a.playing(*id));
								if !live {
									self.loops.remove(&owner);
								}
								// RNG-4b: a start uses the pitch draw made at its request point in the tick; an update draws nothing.
								let act = loop_act(live, audible(&c, listener_dist(&self.cam, at)));
								let key = VoiceKey::Loop(owner);
								let rnd = if act == LoopAct::Start { self.voices.start(key, &c) } else { self.voices.current(key) };
								let (v, pan, sh) = spatial_rand(&self.cam, at, Some(&c), &self.listener, None, Some(rnd));
								// BRIDGE-49: the IOP term of a game-written bend (FUN_002a12b8) adds to the listener shift.
								let bend = self.loop_bend.get(&owner).map_or(0, |&b| sound::bend_term(c.bend, b));
								match act {
									LoopAct::Start => {
										if let Some(clip) = self.sfx.clips.get(c.sound).and_then(|x| x.as_ref()) {
											let id = a.play_with(clip, c.volume * v, pan, sound::pitch_ratio(sh + bend), false);
											if id != 0 {
												self.loops.insert(owner, id);
											}
										}
									}
									LoopAct::Stop => {
										if let Some(id) = self.loops.remove(&owner) {
											a.stop(id);
										}
									}
									LoopAct::Update => {
										if let Some(&id) = self.loops.get(&owner) {
											a.set(id, c.volume * v, pan);
											a.set_pitch(id, sound::pitch_ratio(sh + bend));
										}
									}
									LoopAct::Idle => {}
								}
							}
						}
					}
					for (at, c, rnd, held, level) in shots {
						if let Some(clip) = self.sfx.clips.get(c.sound).and_then(|x| x.as_ref()) {
							// AUD-11h: a start below the gain gate is refused (FUN_002a0cd0); ARI-27b: a fixed-level start
							// (flags 0x10) has no gate.
							if level.is_none() && !audible(&c, listener_dist(&self.cam, at)) {
								continue;
							}
							let (v, pan, sh) = spatial_rand(&self.cam, at, Some(&c), &self.listener, None, rnd);
							let (vf, ds) = self.listener.effects(self.cam.pos, at, [0.0; 3], c.fall.flags);
							// ARI-27b: the fixed level replaces the def volume and falloff (and the occlusion); the underwater
							// term still applies.
							let gain = match level {
								Some(l) => l as f32 / 1024.0 * vf,
								None => c.volume * v,
							};
							let id = a.play_with(clip, gain, pan, sound::pitch_ratio(sh), false);
							let random = sh - ds;
							if id != 0 {
								if held {
									self.held_click = id;
								}
								self.live_shots.push(LiveShot { id, at, cue: c, owner: None, random, level });
							}
						}
					}
					// Toad and robot sounds come from the animation (FUN_00232250 timed cues, FUN_0024e890 loop
					// sound; decomp/notes/sound.md "Animation sound cues"). Pause or a hidden prop stops the loops.
					{
						let mut keep = Vec::new();
						if let Some(game) = self.game.as_mut().filter(|_| !paused && native_sfx) {
							for i in 0..game.props.len() {
								if !game.props[i].shown || !game::CUE_CLASSES.contains(&game.props[i].class_id) {
									continue;
								}
								// Cue ids the clock fired since the last frame (the 0x264df8 scan, one per tick).
								let cues = game.props[i].anim.take_cues();
								let dying = false;
								let pr = &game.props[i];
								// A skeleton-less class (the fan) has no prop_anims entry; its loop sound is static.
								let want = match self.prop_anims.get(&pr.class_id) {
									Some((_, an)) => {
										let Some(seq) = an.sequences.get(pr.anim.seq).filter(|s| !s.frames.is_empty()) else { continue };
										(seq.loop_sound != 0xff).then_some(seq.loop_sound as usize)
									}
									None => match self.sfx.ev.static_loops.get(&pr.class_id) {
										Some(&w) => Some(w),
										None => continue,
									},
								};
								keep.push(i);
								let st = self.enemy_sfx.entry(i).or_insert(EnemySfx { seq: pr.anim.seq, voice: None });
								// A death plays once (the pose holds through the fade): no cues once it has wrapped.
								let once = dying && pr.anim.done;
								// NOV-22.7: every cue the ticks of this frame fired plays, oldest first (the clock already
								// silences a sequence-change tick itself).
								for (k, idx) in enemy_frame_cues(&cues, once).into_iter().enumerate() {
									let Some((c, clip)) = self.sfx.ev.enemy_cs.get(&(pr.class_id, idx)).and_then(|c| Some((c, self.sfx.clips.get(c.sound)?.as_ref()?))) else { continue };
									// AUD-11h: owned voices start at the moby position + (0,0,1), gated by distance.
									let at = voice_at(pr.pos);
									if !audible(c, listener_dist(&self.cam, at)) {
										continue;
									}
									let (v, pan, sh) = spatial_rand(&self.cam, at, Some(c), &self.listener, Some(i), Some(self.voices.start(VoiceKey::EnemyCue(i, k), c)));
									let id = a.play_with(clip, c.volume * v, pan, sound::pitch_ratio(sh), false);
									let random = sh - self.listener.effects(self.cam.pos, at, [0.0; 3], c.fall.flags).1;
									if id != 0 {
										self.live_shots.push(LiveShot { id, at, cue: *c, owner: Some(i), random, level: None });
									}
								}
								if st.seq != pr.anim.seq {
									if let Some((id, _)) = st.voice.take() {
										a.stop(id);
									}
									st.seq = pr.anim.seq;
								}
								// AUD-11h: a dead or dropped voice frees the slot; a loop whose gain fell below 0x20 is stopped
								// (FUN_0029ff68); the start below re-requests every frame (FUN_00263370).
								let at = voice_at(pr.pos);
								let dist = listener_dist(&self.cam, at);
								if st.voice.is_some_and(|(id, _)| id == 0 || !a.playing(id)) {
									st.voice = None;
								}
								if let Some((id, cur)) = st.voice {
									if want == Some(cur) && self.sfx.ev.enemy_cs.get(&(pr.class_id, cur)).is_some_and(|c| !audible(c, dist)) {
										a.stop(id);
										st.voice = None;
									}
								}
								match (st.voice, want) {
									(Some((id, cur)), Some(w)) if cur == w => {
										let c = self.sfx.ev.enemy_cs.get(&(pr.class_id, w));
										let (v, pan, sh) = spatial_rand(&self.cam, at, c, &self.listener, None, Some(self.voices.current(VoiceKey::EnemyLoop(i))));
										a.set(id, c.map_or(0.0, |c| c.volume) * v, pan);
										a.set_pitch(id, sound::pitch_ratio(sh));
									}
									(old, want) => {
										if let Some((id, _)) = old {
											a.stop(id);
										}
										st.voice = want.and_then(|w| {
											let c = self.sfx.ev.enemy_cs.get(&(pr.class_id, w))?;
											let clip = self.sfx.clips.get(c.sound)?.as_ref()?;
											if !audible(c, dist) {
												return None;
											}
											let (v, pan, sh) = spatial_rand(&self.cam, at, Some(c), &self.listener, None, Some(self.voices.start(VoiceKey::EnemyLoop(i), c)));
											let id = a.play_with(clip, c.volume * v, pan, sound::pitch_ratio(sh), false);
											(id != 0).then_some((id, w))
										});
									}
								}
							}
						}
						self.enemy_sfx.retain(|i, st| {
							let k = keep.contains(i);
							if !k {
								if let Some((id, _)) = st.voice.take() {
									a.stop(id);
								}
							}
							k
						});
					}
					// The Suck Cannon's suction loop: class 849 sound 2 at the muzzle point, flags 4 (game/suck.rs).
					{
						let want = self.game.as_ref().filter(|g| !paused && native_sfx && g.suck_loop()).map(|g| self.pyro_muzzle.unwrap_or(g.player.pos));
						match (want, self.suck_voice) {
							(Some(at), v) => {
								let c = self.sfx.ev.class.get(&(849, 2));
								let running = v.filter(|&v| a.playing(v));
								let rnd = c.map(|c| if running.is_none() { self.voices.start(VoiceKey::Suck, c) } else { self.voices.current(VoiceKey::Suck) });
								let (vol, pan, sh) = spatial_rand(&self.cam, at, c, &self.listener, None, rnd);
								match running {
									Some(v) => {
										a.set(v, c.map_or(0.0, |c| c.volume) * vol, pan);
										a.set_pitch(v, sound::pitch_ratio(sh));
									}
									None => self.suck_voice = c.and_then(|c| Some(a.play_with(self.sfx.clips.get(c.sound)?.as_ref()?, c.volume * vol, pan, sound::pitch_ratio(sh), true))),
								}
							}
							(None, Some(v)) => {
								a.stop(v);
								self.suck_voice = None;
							}
							(None, None) => {}
						}
					}
					// Running one-shots recompute gain, pan, Doppler and occlusion with the live listener each frame.
					if !paused {
						let props = self.game.as_ref().map(|g| &g.props);
						self.live_shots.retain(|s| a.playing(s.id));
						for s in &mut self.live_shots {
							if let Some(p) = s.owner.and_then(|o| props.and_then(|p| p.get(o))) {
								s.at = voice_at(p.pos);
							}
							let (v, pan, sh) = live_shot(&self.cam, s, s.at, &self.listener);
							a.set(s.id, v, pan);
							a.set_pitch(s.id, sound::pitch_ratio(sh));
						}
					}
					// The Pyrocitor's firing loop: class 176 sound 1 at the muzzle, restarted whenever it has ended (state 3,
					// flags 4); stopped with the burst, on pause and off play (game/pyro.rs).
					{
						let want = self.game.as_ref().filter(|g| !paused && native_sfx && g.pyro_loop()).map(|g| self.pyro_muzzle.unwrap_or(g.player.pos));
						match (want, self.pyro_voice) {
							(Some(at), v) => {
								let c = self.sfx.ev.class.get(&(game::pyro::PYRO_CLASS, 1));
								let running = v.filter(|&v| a.playing(v));
								let rnd = c.map(|c| if running.is_none() { self.voices.start(VoiceKey::Pyro, c) } else { self.voices.current(VoiceKey::Pyro) });
								let (vol, pan, sh) = spatial_rand(&self.cam, at, c, &self.listener, None, rnd);
								match running {
									Some(v) => {
										a.set(v, c.map_or(0.0, |c| c.volume) * vol, pan);
										a.set_pitch(v, sound::pitch_ratio(sh));
									}
									None => self.pyro_voice = c.and_then(|c| Some(a.play_with(self.sfx.clips.get(c.sound)?.as_ref()?, c.volume * vol, pan, sound::pitch_ratio(sh), true))),
								}
							}
							(None, Some(v)) => {
								a.stop(v);
								self.pyro_voice = None;
							}
							(None, None) => {}
						}
					}
					// Music (decomp/notes/music.md): music id 0 at level start = slot 0 once, then slot 1
					// looping; Veldin never changes track. Ducked under cutscene speech. The pause menu pauses
					// it (FUN_002651b8(0) on open, FUN_002651f0 on close: decomp/notes/ingame_pause_menu.md).
					// Entry freezes every live voice (music without ducking, ambience loops, SFX in flight:
					// IOP group 0x1d pause, decomp/notes/music.md); exit resumes them where they were.
					// Retail mode: the game's pause menu (mode 3) pauses the music (FUN_002651b8).
					let music_paused = self.pause.is_some() || self.retail.as_ref().is_some_and(|r| r.paused());
					if music_paused != self.audio_paused {
						a.pause_all(music_paused);
						self.audio_paused = music_paused;
					}
					// Retail mode: the driver pauses its own streams (45 / 46) and sets the reverb through its SPU2 writes
					// (retail/snd.rs); rac-rs's music player and reverb zones are off below.
					// HelpDesk voice (ingame_menu_leftovers.md P1-a): starts in box state 1 when Voice is on
					// (voice id + 30000, streamed as speech), holds state 5 while it plays, pauses with the
					// menu. Stopped when the box closes [I].
					let help = self.flow.as_ref().filter(|_| self.game.is_some()).map(|f| (f.help.state, f.help.id, f.voice_of(f.help.id)));
					match (help, self.help_voice) {
						(Some((1, id, Some(vid))), cur) if cur.map_or(true, |c| c.0 != id) && self.settings.help_voice => {
							if let Some((_, v)) = cur {
								a.stop(v);
							}
							let iso = &self.iso_path;
							let lang = self.settings.lang;
							let clip = self.help_clips.entry(vid).or_insert_with(|| match iso::Disc::open(std::path::Path::new(iso)).map_err(|e| e.to_string()).and_then(|mut d| sound::help_voice(&mut d, lang as usize, vid)) {
								Ok(c) => c.map(|(rate, pcm)| audio::Clip { rate, pcm: std::sync::Arc::new(pcm), loop_start: None, reverb: false }),
								Err(e) => {
									eprintln!("help voice {vid}: {e}");
									None
								}
							});
							self.help_voice = clip.as_ref().map(|c| (id, a.play(c, 1.0, 0.0)));
						}
						(Some((s, id, _)), Some((cur, v))) if s != 0 && cur == id => a.pause(v, music_paused),
						(_, Some((_, v))) => {
							a.stop(v);
							self.help_voice = None;
						}
						_ => {}
					}
					let speaking = self.help_voice.is_some_and(|(_, v)| a.playing(v));
					if let Some(f) = &mut self.flow {
						f.voice = speaking;
					}
					let volume = if self.scene_voice.is_some() { MUSIC_VOLUME * 0.3 } else { MUSIC_VOLUME };
					// Music controller (music.rs, music.md): the level start / a death restarts the track the
					// spawn area names (else the current one); zone exits feed requests that the controller
					// turns into a stinger transition. Reverb zones and the spawn area pick the reverb
					// (ambience.md). The pause menu freezes all of it (pause_all above).
					match self.game.as_ref().filter(|_| self.retail.is_none()).map(|g| (g.player.pos, g.story.deaths)) {
						Some((hero, deaths)) => {
							if self.music_player.is_none() || deaths != self.music_deaths {
								self.music_deaths = deaths;
								let lens: Vec<i32> = self.music.iter().map(|c| music::stream_units(c.pcm.len() as u64, c.rate)).collect();
								let mp = self.music_player.get_or_insert_with(|| music::Player::from_units(&lens));
								let area = ambience::area_at(&self.areas, hero);
								// SAV-1c: after a checkpoint commit the death restart restores the captured track, reverb and hero light.
								if let Some(env) = self.ckpt_env {
									mp.restart(Some(env.track as i16));
									self.reverb = env.reverb;
									if let Some(gpu) = &mut self.gpu {
										gpu.hero_env = env.hero_env;
									}
								} else {
									mp.restart(area.map(|a| a.music as i16));
									self.reverb = area.and_then(|a| a.reverb).unwrap_or_default();
								}
								for z in &mut self.zones.reverb {
									z.inside = false;
								}
								for z in &mut self.zones.music {
									z.inside = false;
								}
								for v in [self.music_voice.take(), self.sting_voice.take()].into_iter().flatten() {
									a.stop(v.0);
								}
							}
							if !music_paused {
								let cutscene = self.flow.as_ref().map_or(false, |f| f.cutscene.is_some());
								if let Some(mp) = &mut self.music_player {
									mp.update(dt);
									// BRIDGE-86: the actors' requests of this frame (FUN_00253138 keeps its own cooldown).
									for &(t, c) in &music_reqs {
										mp.request(t, c);
									}
									if !cutscene {
										for z in &mut self.zones.music {
											if let Some((t, c)) = z.update(hero) {
												mp.request(t, c);
											}
										}
										for z in &mut self.zones.reverb {
											if let Some(r) = z.update(hero) {
												self.reverb = r;
											}
										}
									}
									a.set_reverb(self.reverb.mode, self.reverb.amount);
									let want = mp.main_out();
									match (want, &mut self.music_voice) {
										(Some(m), Some(v)) if v.1 == m.slot => {
											a.set_music(v.0, volume * m.gain);
											a.pause(v.0, m.paused);
										}
										(Some(m), v) => {
											if let Some(o) = v.take() {
												a.stop(o.0);
											}
											if let Some(c) = self.music.get(m.slot) {
												let id = a.play_music(c, volume * m.gain);
												a.pause(id, m.paused);
												*v = Some((id, m.slot));
											}
										}
										(None, v) => {
											if let Some(o) = v.take() {
												a.stop(o.0);
											}
										}
									}
									match (mp.stinger_slot(), &mut self.sting_voice) {
										(Some(s), Some(v)) if v.1 == s => {
											a.set_music(v.0, volume);
											a.pause(v.0, mp.held());
										}
										(Some(s), v) => {
											if let Some(o) = v.take() {
												a.stop(o.0);
											}
											if let Some(c) = self.music.get(s) {
												let id = a.play_music(c, volume);
												a.pause(id, mp.held());
												*v = Some((id, s));
											}
										}
										(None, v) => {
											if let Some(o) = v.take() {
												a.stop(o.0);
											}
										}
									}
								}
							}
						}
						None => {
							for v in [self.music_voice.take(), self.sting_voice.take()].into_iter().flatten() {
								a.stop(v.0);
							}
							self.music_player = None;
						}
					}
					// Positional ambience (decomp/notes/ambience.md): spheres hold a loop while the hero is
					// inside, boxes retrigger a one-shot at a random point inside them every 4-12 s.
					// While the pause menu is up the loops stay frozen in place (pause_all above).
					let listener = self.game.as_ref().filter(|_| !paused && self.flow.as_ref().map_or(true, |f| f.cutscene.is_none())).map(|g| g.player.pos);
					// Retail mode: the interpreter plays its own ambience through 989snd (retail.rs).
					let retail_mode = self.retail.is_some();
					self.emitter_state.resize(self.emitters.len(), (None, 0.0));
					for (ei, (e, st)) in self.emitters.iter().zip(self.emitter_state.iter_mut()).enumerate().filter(|_| !music_paused && !retail_mode) {
												let clip = e.bank_sound.and_then(|i| self.sfx.clips.get(i)).and_then(|c| c.as_ref());
						let (Some(hero), Some(clip)) = (listener, clip) else {
							if let Some(id) = st.0.take() {
								a.stop(id);
							}
							continue;
						};
						match e.shape {
							ambience::Shape::Sphere => {
								// The loop's tone (centre byte above 127) is not decoded yet: off until it is.
								if !self.sfx.ambient_ok {
									continue;
								}
								let gain = ambience::gain(e, hero);
								let gain = if gain < 0x20 as f32 / 1024.0 { 0.0 } else { gain };
								let pan = if e.no_pan { 0.0 } else { spatial(&self.cam, e.pos, None, &self.listener).1 };
								let (vf, sh) = self.listener.effects(self.cam.pos, e.pos, [0.0; 3], e.flags);
								// A running loop keeps its own line-of-sight history (slot = emitter index).
								let occ = if gain > 0.0 { self.listener.occlusion(Some(ei as u32), self.cam.pos, e.pos, e.flags, true, None) } else { 1.0 };
								let (gain, ratio) = (gain * vf * occ, sound::pitch_ratio(sh));
								match st.0 {
									None if e.in_range(hero) => st.0 = Some(a.play_with(clip, gain, pan, ratio, false)).filter(|&id| id != 0),
									Some(id) if e.out_of_range(hero) => {
										a.stop(id);
										st.0 = None;
									}
									Some(id) => {
										a.set(id, gain, pan);
										a.set_pitch(id, ratio);
									}
									None => {}
								}
							}
							_ => {
								// The retrigger decision and its draws ran in the tick (S2, `stage_s1`); the voices it granted start here.
								for &(_, at, gain) in self.ambient_plays.iter().filter(|q| q.0 == ei) {
									let pan = if e.no_pan { 0.0 } else { spatial(&self.cam, at, None, &self.listener).1 };
									let (vf, sh) = self.listener.effects(self.cam.pos, at, [0.0; 3], e.flags);
									let occ = self.listener.occlusion(None, self.cam.pos, at, e.flags, false, None);
									a.play_with(clip, gain * vf * occ, pan, sound::pitch_ratio(sh), false);
								}
							}
						}
					}
					self.ambient_plays.clear();
					// Volume boxes (type 1, FUN_00319258) and the underwater loop (type 5, FUN_003199a8; ambience.md).
					// A box voice sits at the nearest point of its box, fades toward the shell, plays with play flags
					// 0x10 (+4 looped) so only the box's own volume applies; a loop below 0x20/1024 is stopped.
					self.volume_voices.resize(self.zones.volume.len(), None);
					self.volume_start.resize(self.zones.volume.len(), false);
					for (vi, (b, v)) in self.zones.volume.iter_mut().zip(self.volume_voices.iter_mut()).enumerate().filter(|_| !music_paused && !retail_mode) {
						*v = v.filter(|&id| a.playing(id));
						let clip = b.bank_sound.and_then(|i| self.sfx.clips.get(i)).and_then(|c| c.as_ref());
						let (Some(hero), Some(clip)) = (listener, clip) else {
							if let Some(id) = v.take() {
								a.stop(id);
							}
							continue;
						};
						let Some((level, at)) = b.sample(hero) else {
							if let Some(id) = v.take() {
								a.stop(id);
							}
							continue;
						};
						if b.looped && level < sound::MIN_START_VOLUME {
							if let Some(id) = v.take() {
								a.stop(id);
							}
							continue;
						}
						let (vf, sh) = self.listener.effects(self.cam.pos, at, [0.0; 3], b.fall.flags);
						let gain = if level < sound::MIN_START_VOLUME { 0.0 } else { level as f32 / 1024.0 * vf };
						let pan = spatial(&self.cam, at, None, &self.listener).1;
						let ratio = sound::pitch_ratio(sh);
						match *v {
							Some(id) => {
								a.set(id, gain, pan);
								a.set_pitch(id, ratio);
							}
							// The countdown test and the rearm draw ran in the tick (S2).
							None if std::mem::take(&mut self.volume_start[vi]) => {
								*v = Some(a.play_with(clip, gain, pan, ratio, false)).filter(|&id| id != 0);
							}
							None => {}
						}
					}
					self.volume_start.iter_mut().for_each(|f| *f = false);
					self.underwater_voices.resize(self.zones.underwater.len(), None);
					for (z, v) in self.zones.underwater.iter().zip(self.underwater_voices.iter_mut()).filter(|_| !music_paused && native_sfx) {
						*v = v.filter(|&id| a.playing(id));
						let clip = z.bank_sound.and_then(|i| self.sfx.clips.get(i)).and_then(|c| c.as_ref());
						match (self.listener.underwater, *v, clip) {
							(true, None, Some(clip)) => {
								let (_, sh) = self.listener.effects(self.cam.pos, self.cam.pos, [0.0; 3], z.fall.flags);
								*v = Some(a.play_with(clip, 1.0, 0.0, sound::pitch_ratio(sh), false)).filter(|&id| id != 0);
							}
							(false, Some(id), _) => {
								a.stop(id);
								*v = None;
							}
							_ => {}
						}
					}
				}
				if self.retail.is_some() {
					// Retail: the front end runs in the interpreter and the mirror draws it from its RAM (viewer/front_mirror.rs).
					self.front_mirror_enter();
					if self.front.is_some() && self.game.is_none() {
						self.front_mirror_tick(dt);
						self.front_mirror_frame(dt);
					} else {
						// [PC] Not in the front end: the sharp title state is off (viewer/hudsharp.rs).
						super::hudsharp::set(false, [usize::MAX; 2]);
					}
				}
				// Retail mode: the game travelled by itself (its own travel sequence and loader ran in the interpreter);
				// rac-rs loads the new level's assets and starts a game with the same running `Retail`.
				// [PC] The host side of the new level is parsed on a background thread as soon as retail starts travelling to it
				// (the landing stream takes seconds), so the switch below only uploads .
				if let Some(n) = self.retail.as_ref().and_then(|r| r.travel_target()) {
					if self.game.is_some() && !self.want_front && n < self.levels.len() && self.prefetch_tried != Some(n) {
						self.prefetch_tried = Some(n);
						crate::loadlog::say(|| format!("main: prefetch level {n}"));
						self.prefetch_level(n);
					}
				}
				self.prefetch_spec();
				self.switch_hold = self.retail.as_ref().is_some_and(|r| r.switch_pending()) && !self.prefetch_ready();
				if let Some((n, pools)) = if self.switch_hold { None } else { self.retail.as_mut().and_then(|r| r.switch()) } {
					eprintln!("retail: travelled to level {n}");
					self.prefetch_tried = None;
					crate::loadlog::say(|| format!("main: switch to level {n} (host load starts)"));
					crate::retail::set_pools(pools);
					self.retail_carry = self.retail.take();
					self.level = n;
					self.arrival_black = ARRIVAL_BLACK_MAX;
					// A held movie picture (MOVIE_HOLD) must not outlive the level it was shown over: it would cover the new level.
					MOVIE_HOLD.store(false, std::sync::atomic::Ordering::Relaxed);
					if let Some(g) = &mut self.gpu {
						g.movie.clear();
					}
					self.load_level();
					crate::loadlog::say(|| "main: load_level returned, toggle_play".into());
					self.toggle_play();
					crate::loadlog::say(|| "main: toggle_play returned (game running)".into());
				}
				// Retail mode: a class outgrew its prop pool (retail.rs `sync`); rebuild the level's prop slots with the grown
				// pools and start a game with the same running `Retail` (as after travel).
				// [PC] The new props are appended to the running game and their slots to the loaded scene (`append_pools`), drawn
				// and bound in a rebuilt game's order; the level, its audio and particles stay. Without room there the prop slots are rebuilt in the
				// scene (`regrow_props`) with a new game; only a slot the scene cannot build there reloads the level.
				if let Some(pools) = self.retail.as_mut().and_then(|r| r.take_grow_kept()) {
					let _pg = profile::span("pg_all");
					let t0 = std::time::Instant::now();
					let slots: u32 = pools.iter().map(|p| p.1).sum();
					let order = self.retail.as_ref().and_then(|r| r.bind_order().map(<[usize]>::to_vec));
					let _pa = profile::span("pg_append");
					let how = if let Some(order) = self.append_pools(&crate::retail::pools(), &pools, order.as_deref()) {
						drop(_pa);
						crate::retail::set_pools(pools);
						let _pr = profile::span("pg_rebind");
						if let Some(r) = self.retail.as_mut() {
							r.rebind(order);
						}
						drop(_pr);
						// PC-FPS-1: the props are bound anew; their first ticks draw unblended (as after a rebuild).
						self.smooth = Default::default();
						"appended in place"
					} else {
						if let Some(r) = self.retail.as_mut() {
							r.unbind();
						}
						crate::retail::set_pools(pools);
						self.retail_carry = self.retail.take();
						let in_place = self.regrow_props();
						if in_place {
							self.game = None;
							// PC-FPS-1: the new game's first ticks draw unblended (slot indices moved).
							self.smooth = Default::default();
						} else {
							self.load_level();
						}
						self.toggle_play();
						if in_place { "regrown in place" } else { "rebuilt by a level reload" }
					};
					let _pl = profile::span("pg_log");
					let (verts, cap) = self.gpu.as_ref().and_then(|g| g.scene.as_ref()).map_or((0, 0), |s| (s.props_model.len(), s.props.vbuf.size() as usize / std::mem::size_of::<Vertex>()));
					crate::loadlog::later(format!("retail: prop slots {how} ({slots} pool props, {verts} prop vertices of {cap}) in {:.1} ms", t0.elapsed().as_secs_f64() * 1e3));
				}
				for m in &mut self.messages {
					m.1 -= dt;
				}
				self.messages.retain(|m| m.1 > 0.0);
				if self.game.is_none() && self.front.is_none() {
					if let Some(g) = &mut self.gpu {
						hud(&mut g.hud, g.config.width as f32, g.config.height as f32, None, &self.messages, None, &self.art, self.settings.subtitles);
					}
				}
				let speed = if self.game.is_some() || self.front.is_some() { 0.0 } else if self.keys.contains(&KeyCode::ShiftLeft) { 120.0 } else { 30.0 } * dt;
				let f = forward(&self.cam);
				let r = [self.cam.yaw.sin(), -self.cam.yaw.cos(), 0.0];
				let mut mv = |v: [f32; 3], s: f32| {
					for j in 0..3 {
						self.cam.pos[j] += v[j] * s;
					}
				};
				if self.keys.contains(&KeyCode::KeyW) { mv(f, speed) }
				if self.keys.contains(&KeyCode::KeyS) { mv(f, -speed) }
				if self.keys.contains(&KeyCode::KeyD) { mv(r, speed) }
				if self.keys.contains(&KeyCode::KeyA) { mv(r, -speed) }
				if self.keys.contains(&KeyCode::KeyE) { mv([0.0, 0.0, 1.0], speed) }
				if self.keys.contains(&KeyCode::KeyQ) { mv([0.0, 0.0, 1.0], -speed) }
				profile::lap(profile::Phase::Audio);
				if let Some(g) = &mut self.gpu {
					// [PC] Graphics -> Lighting Enhanced (viewer/post.rs): only over a retail level drawn by rac-rs (set below).
					g.modern_ok = false;
					// Retail mode: the game's own HUD and menus (GS capture) replace rac-rs's. The 4:3 frame fills the
					// window height, centred; the edge columns stretch to the sides (clamped).
					if let Some(r) = self.retail.as_mut().filter(|r| r.gs && r.front_ram().is_none()) {
						// GS-full (--gsfull / RAC_GSFULL=1): the game's whole picture (gsfull/pipe.rs) instead of the overlay.
						let full = r.gsfull_view().is_some();
						g.modern_ok = !full;
						// Sharp HUD (lane G): the GPU capture draws the overlay again at this window's resolution (0: off).
						crate::retail::set_hud_target(if full { 0 } else { g.config.height });
						g.ship_fight = r.ship_fight();
						g.level = self.level;
						if full {
							if let Some(f) = r.take_gsfull_frame() {
								g.set_retail_overlay(f.w, f.h, bytemuck::cast_slice(&f.rgba));
							}
						} else if let Some((w, rows, px)) = r.take_overlay() {
							g.set_retail_overlay(w, rows, &px);
						}
						g.hud.clear();
						g.hud_split = None;
						if g.has_retail_overlay() {
							let (sw, sh) = (g.config.width as f32, g.config.height as f32);
							if full {
								// The 512x448 frame is a 4:3 picture (16:9 squeezed in wide mode, gsfull/wide.rs): centred at the
								// largest size that fits, black bars around; square pixels either way.
								crate::overlay::rect(&mut g.hud, [0.0, 0.0, sw, sh], [0.0, 0.0, 0.0, 1.0]);
								let ar = if r.gsfull_view().is_some_and(|f| f.wide) { crate::gsfull::wide::ASPECT } else { 4.0 / 3.0 };
								let (w, h) = if sw >= sh * ar { (sh * ar, sh) } else { (sw, sw / ar) };
								let (x, y) = ((sw - w) * 0.5, (sh - h) * 0.5);
								g.hud.push(crate::overlay::Quad { tex: RETAIL_OV_TEX, rect: [x, y, w, h], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
							} else {
								let k = (sw / sh) / (4.0 / 3.0);
								if r.fx().is_some_and(|f| f.rc.is_some()) && k > 1.0 {
									// RC range static (queue_b::rc_static): the overlay holds retail's static over the 4:3 centre, so it is
									// drawn there unclamped (edge-clamped columns would streak the bars) and each bar shows translated copies
									// of it [PC: widescreen].
									crate::retail::queue_b::rc_bar_quads(RETAIL_OV_TEX, sw, sh, &mut g.hud);
								} else {
									g.hud.push(crate::overlay::Quad { tex: RETAIL_OV_TEX, rect: [0.0, 0.0, sw, sh], uv: [0.5 - 0.5 * k, 0.0, 0.5 + 0.5 * k, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
								}
								// Retail shop screen: what retail draws through VU1 (absent from the GS capture), from the shop slot mobys
								// (retail::shop_snap): the world item over the pad, panel 1's item + bolt icon and panel 2's vendor actor.
								// The two targets go over the overlay's panel interiors (retail clears them black, blits them opaque,
								// then draws the panel text into them; the overlay holds that text, so the target is added onto it).
								g.shop_models.clear();
								g.shop_panel_models.clear();
								g.icon_bolt = None;
								g.icon_bolt_sphere = None;
								g.popup_bolt = None;
								g.popup_bolt_sphere = None;
								g.pose_portrait(None);
								g.pose_panel_item(None);
								if let (Some(sn), Some(game)) = (r.shop(), self.game.as_ref()) {
									let mat = |s: &crate::retail::ShopMoby| {
										let mut m = place(s.pos, s.rot);
										m[..12].iter_mut().for_each(|v| *v *= s.scale);
										(s.class, m)
									};
									g.shop_models = sn.world.iter().map(mat).collect();
									g.shop_panel_models = sn.panel.iter().map(mat).collect();
									// NOV-30d (1)/(3): item A is skinned at its retail clock (seq 0 / 1, stepped per tick), not drawn in bind
									// pose [C: vendor.md; s11_ammo bomb-glove panel 192 showed the fingers only posed]. Rig = level prop
									// animation, else the held gadget rigs (glove, Pyrocitor, Suck Cannon, swingshot, the other held items).
									let posed = sn.panel.as_ref().and_then(|s| {
										fn held(h: &Held) -> (&mobies::MobyClass, &anim::Animated) {
											(&h.class, &h.anim)
										}
										let (class, a) = self.prop_anims.get(&s.class).map(|(c, a)| (c, a))
											.or_else(|| self.held_rigs.get(&s.class).map(|r| held(&r.held)))
											.or_else(|| self.glove.as_ref().filter(|_| s.class == GLOVE_CLASS).map(held))
											.or_else(|| self.pyro_rig.as_ref().filter(|_| s.class == game::pyro::PYRO_CLASS).map(|r| held(&r.held)))
											.or_else(|| self.suck_rig.as_ref().filter(|_| s.class == 849).map(|r| held(&r.held)))
											.or_else(|| self.hook_rig.as_ref().filter(|_| s.class == game::held::SWINGSHOT_HOOK).map(held))?;
										let mut pa = crate::anim::PropAnim::new(0);
										s.clock.apply(&mut pa);
										let (mut ps, mut ns) = anim::skin(class, &pa.pose(a, &[]));
										let n = class.mesh.positions.len();
										ps.resize(n, [0.0; 3]);
										ns.resize(n, [0.0, 0.0, 1.0]);
										Some((s.class, ps, ns))
									});
									g.pose_panel_item(posed);
									g.icon_bolt = sn.icon.as_ref().map(|s| mat(s).1);
									// NOV-35b: the icon bolt's draw sphere (class 13 seq sphere at the slot's pose) for its metal pass.
									g.icon_bolt_sphere = sn.icon.as_ref().and_then(|s| game.bolt_sphere(s.pos, s.rot));
									let portrait = sn.portrait.as_ref().and_then(|s| {
										let (class, a) = self.prop_anims.get(&game::vendor::PORTRAIT_CLASS)?;
										let mut pa = crate::anim::PropAnim::new(0);
										s.clock.apply(&mut pa);
										Some((anim::skin(class, &pa.pose(a, &[])), s.pos, s.rot))
									});
									g.pose_portrait(portrait.as_ref().map(|(p, pos, rot)| (p, *pos, *rot)));
									// The six screens' corner joints of the vendor (class 11) as the shop path takes them, projected with
									// retail's own projection (horizontal tan = the retail scalar) into the 512x448 frame the overlay shows.
									let joints: Option<Vec<[f32; 3]>> = game.props.iter().find(|p| p.class_id == 11 && p.shown).and_then(|p| {
										let (_, a) = self.prop_anims.get(&11)?;
										let mats = p.anim.pose(a, &[]);
										let m = place(p.pos, p.rot);
										Some((0..36).map(|j| {
											let jm = a.joint_world(&mats, j.min(mats.len() - 1));
											let l = [jm[3] * p.scale, jm[7] * p.scale, jm[11] * p.scale];
											[0, 1, 2].map(|i| m[i] * l[0] + m[4 + i] * l[1] + m[8 + i] * l[2] + m[12 + i])
										}).collect())
									});
									let vp = view_proj(&self.cam, 1.0 / PROJ_Y_RATIO, RETAIL_NEAR, RETAIL_FAR);
									// Frame px (512 x 448) of a world point.
									let frame = |p: [f32; 3]| project(&vp, p).map(|c| [(c[0] + 1.0) * 256.0, (1.0 - c[1]) * 224.0]);
									// Frame px -> window px through the overlay's horizontal crop (u = 0.5 - k/2 + x/sw * k).
									let sx = |x: f32| (x / 512.0 - (0.5 - 0.5 * k)) / k * sw;
									// The purchase popup (retail FUN_002904f0): the slot +0x500 bolt in the popup target, blitted whole
									// ((0, 0)-(w, h), FUN_001ecaf8) to the rect of the mirrored prompt prop's attach points 0 / 1 / 2.
									g.popup_bolt = sn.popup_bolt.as_ref().map(|s| mat(s).1);
									g.popup_bolt_sphere = sn.popup_bolt.as_ref().and_then(|s| game.bolt_sphere(s.pos, s.rot));
									let popup = sn.prompt.and(game.props.iter().find(|p| p.class_id == game::vendor::PROMPT_CLASS && p.shown && p.is_retail())).and_then(|p| {
										let (_, a) = self.prop_anims.get(&game::vendor::PROMPT_CLASS)?;
										let joints = [a.attach0?, a.attach12[0]?, a.attach12[1]?];
										let mats = p.anim.pose(a, &[]);
										if joints.iter().any(|&j| j >= mats.len()) {
											return None;
										}
										let m = place(p.pos, p.rot);
										let j = joints.map(|j| {
											let jm = a.joint_world(&mats, j);
											let l = [jm[3] * p.scale, jm[7] * p.scale, jm[11] * p.scale];
											[0, 1, 2].map(|i| m[i] * l[0] + m[4 + i] * l[1] + m[8 + i] * l[2] + m[12 + i])
										});
										crate::shop::popup_rect(&j, &frame)
									});
									if let Some(q) = popup {
										let (x0, y0, w0, h0) = (q[0], q[1].min(q[1] + q[3]), q[2], q[3].abs());
										if w0 >= 1.0 && h0 >= 1.0 {
											let rect = [sx(x0), y0 / 448.0 * sh, sx(x0 + w0) - sx(x0), h0 / 448.0 * sh];
											let uv = [0.0, 0.0, w0 as i32 as f32 / 512.0, h0 as i32 as f32 / 128.0];
											g.hud.push(crate::overlay::Quad { tex: POPUP_PANEL_TEX, rect, uv, colour: [1.0; 4], angle: 0.0, corners: None });
										}
									}
									if let Some(rects) = joints.map(|j| crate::shop::screen_rects(&j, &frame)) {
										for (i, tex) in [(1, ITEM_PANEL_TEX), (2, PORTRAIT_PANEL_TEX)] {
											let q = rects[i];
											let (x0, y0, w0, h0) = (q[0], q[1].min(q[1] + q[3]), q[2], q[3].abs());
											if w0 <= 1.0 || h0 <= 1.0 {
												continue;
											}
											let rect = [sx(x0), y0 / 448.0 * sh, sx(x0 + w0) - sx(x0), h0 / 448.0 * sh];
											// The blit's source texels (0,0)-(w0 - 1, int(h0 * 13/14 - 1)) of the 512x128 target (shop::panel_uv).
											let uv = [0.0, 0.0, (w0 - 1.0) as i32 as f32 / 512.0, (h0 * 0.92857146 - 1.0) as i32 as f32 / 128.0];
											g.hud.push(crate::overlay::Quad { tex, rect, uv, colour: [1.0; 4], angle: 0.0, corners: None });
										}
									}
								}
								crate::retail::queue_b::screen_quads(r.fx().map_or(&[][..], |f| &f.screen), sw, sh, &mut g.hud);
							}
						}
					}
					// [PC] DEV ONLY: the dev tools' readout and planet list over everything (retail/devtools.rs; nothing with RAC_DEV_TOOLS off).
					if self.retail.is_some() {
						let hero = self.game.as_ref().map_or([0.0; 3], |gm| gm.player.pos);
						crate::retail::devtools::draw(&mut g.hud, g.config.width as f32, g.config.height as f32, self.level, hero);
					}
					// [PC] RAC_FPS_METER=1: fps, frame time and game speed in the top right corner (viewer/profile.rs `meter_draw`).
					profile::meter_draw(&mut g.hud, g.config.width as f32, g.config.height as f32);
					// --title boot (retail/title_boot.rs): plain black over the display (and no sound) while the game steers itself to its title.
					let cover = self.retail.as_ref().is_some_and(|r| r.title_cover());
					if let Some(a) = &mut self.audio {
						a.master = if cover { 0.0 } else if a.master == 0.0 { 1.0 } else { a.master };
					}
					if cover {
						g.hud.clear();
						g.hud_split = None;
						let (sw, sh) = (g.config.width as f32, g.config.height as f32);
						crate::overlay::rect(&mut g.hud, [0.0, 0.0, sw, sh], [0.0, 0.0, 0.0, 1.0]);
					}
					// Retail mode: the fog block, pool far clamp and shrub cap the retail draw of this field reads (retail.rs
					// `SwapWords`: zone fog, underwater swap and the Visibomb render snapshot alike) replace the level-data fog.
					if let Some(fx) = self.retail.as_ref().and_then(|r| r.fx()) {
						if let Some(w) = fx.swap {
							g.fog_live = Some((w.fog_rgb, w.fog));
							g.part_far = w.part_far;
							g.under_flag = w.under;
							g.under_surf = w.surf;
						}
						if let Some(c) = fx.shrub_cap {
							g.shrub_cap = c;
						}
					}
					// [PC] The world is black while retail is on another level than the one loaded here (its load is over, no frozen load screen, and its
					// landing has begun, the switch is still to come: the old level's picture showed for a few frames) and until it presents
					// its first picture of the new level (`arrival_black`). Retail draws that landing on black (its fade-in).
					let ahead = self.retail.as_ref().is_some_and(|r| r.front_ram().is_none() && !r.paused() && r.travel_target().is_some_and(|t| t < self.levels.len()));
					if self.arrival_black > 0 {
						self.arrival_black = if ahead || self.retail.as_ref().is_none_or(|r| !r.holds()) { 0 } else { self.arrival_black - 1 };
					}
					if ahead || self.arrival_black > 0 {
						let mut b = Vec::new();
						crate::overlay::rect(&mut b, [0.0, 0.0, g.config.width as f32, g.config.height as f32], [0.0, 0.0, 0.0, 1.0]);
						g.hud.splice(0..0, b);
						g.hud_split = g.hud_split.map(|s| s + 1);
					}
					// --shot: let a few frames settle, save one, quit.
					let capture = self.shot.as_ref().is_some_and(|s| s.1 == 0);
					// While the switch waits for the prefetch (`switch_hold`) the frame still draws: the world is black then (above), no stale picture stays up.
					let ly_img = g.draw(drawn_cam.as_ref().unwrap_or(&self.cam), capture);
					if let Some(img) = ly_img.filter(|_| capture) {
						let path = &self.shot.as_ref().unwrap().0;
						// RAC_SHOT_SEQ=N (dev): N consecutive captured frames, path_00.png .. (flicker checks); then quit.
						static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
						let seq = std::env::var("RAC_SHOT_SEQ").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
						let k = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed); if seq > 1 && std::env::var("RAC_SHOT_STEP").is_ok() { crate::retail::SHOT_ALLOW.store(k + 1, std::sync::atomic::Ordering::Relaxed); }
						let path = if seq > 1 { path.replace(".png", &format!("_{k:02}.png")) } else { path.clone() };
						match std::fs::write(&path, crate::render::png(&img)) {
							Ok(()) => eprintln!("saved {path} at play tick {} field {}", self.play_ticks, self.retail.as_ref().map_or(0, |r| r.field_count())),
							Err(e) => eprintln!("{path}: {e}"),
						}
						if k + 1 >= seq {
							el.exit();
						}
					}
					// RAC_SHOT_TICK: hold the capture until that many play ticks have run.
					let wait = std::env::var("RAC_SHOT_TICK").ok().and_then(|v| v.parse::<u32>().ok()).is_some_and(|t| self.play_ticks < t) && !paused // play ticks stop while paused
						|| self.retail.as_ref().is_some_and(|r| r.travelling() || r.shot_field_wait());
					if let Some(s) = &mut self.shot {
						s.1 = if wait { s.1.max(6) } else { s.1.saturating_sub(1) };
					}
					profile::frame_end(self.smooth.ticks);
					self.pace.wait(&g.window, g.config.present_mode != wgpu::PresentMode::Fifo);
					g.window.request_redraw();
				}
			}
			_ => {}
		}
	}
}

impl App {
	/// Mouse button `b` (0 left, 1 middle) went down or up in play. Retail: left = Circle (fire the weapon), middle =
	/// Triangle (Quick Select), no mouse Square (F is the wrench); `tap` also latches a click (`Retail::press_mouse`) so
	/// one shorter than a frame still reaches the game. The clean-room game: left = wrench, middle = weapon.
	pub(super) fn play_mouse(&mut self, b: usize, down: bool, tap: bool) {
		let flag = match (b, self.retail.is_some()) {
			(0, true) | (1, false) => &mut self.fire_mouse,
			(1, true) => &mut self.quick_mouse,
			_ => &mut self.attack_mouse,
		};
		*flag = down;
		if let (true, true, Some(r)) = (down, tap, &mut self.retail) {
			r.press_mouse(b);
		}
	}

	/// A key went down or up: the held set, and in retail play the tap latch (`Retail::press_key`), so a tap that begins
	/// and ends between two frames still reaches the game.
	pub(super) fn key_change(&mut self, k: KeyCode, pressed: bool, repeat: bool) {
		if pressed {
			self.keys.insert(k);
			if let (false, Some(r)) = (repeat, &mut self.retail) {
				r.press_key(k);
			}
		} else {
			self.keys.remove(&k);
		}
	}
}



/// BRIDGE-24: request-point bookkeeping of one frame's ticks (RNG-4b): how far `Game.events` / `Player.foot_cues`
/// were resolved, the voices granted but not yet started, and whether a new distance-attenuated voice (play flag 8,
/// def without the no-occlusion flag) asks for the tick's six start probes.
#[derive(Default)]
struct SoundStage {
}








/// NOV-22.7: the class sound slots of an enemy's queued animation cues that play this frame, oldest first; none once
/// a played-once death sequence has wrapped.
fn enemy_frame_cues(cues: &[u16], once: bool) -> Vec<usize> {
	if once {
		return Vec::new();
	}
	cues.iter().map(|&c| c as usize).collect()
}




/// A scene-local point through the ship pad frame (`place_at` of the viewer tick): yaw about z, then the pad position.
fn pad_place(p: [f32; 3], yaw: f32, v: [f32; 3]) -> [f32; 3] {
	let (s, c) = yaw.sin_cos();
	[p[0] + v[0] * c - v[1] * s, p[1] + v[0] * s + v[1] * c, p[2] + v[2]]
}



/// NOV-14.5d: the ground shadow quads of the ship-stream actors (class 0x213..=0x215) of scene `id` at `tick`, in the
/// pad frame `(pos, yaw)`; `ship` is the pad ship's type (retail `sRam0013e056`), `flight_tick` the substate-3 tick.
#[allow(clippy::too_many_arguments)]
/// SCN-4.3e: the flying pad ship's ground shadow (`cam` = eye, forward, right, up, projection scalar; `c` the flight's
/// tick counter, retail `sRam0013e054` = ticks done minus one).
#[allow(clippy::too_many_arguments)]
fn flight_shadow_quads(world: &World, pos: [f32; 3], rot: [f32; 3], t: usize, sphere: [f32; 4], scale: f32, c: i32, cam: ([f32; 3], [f32; 3], [f32; 3], [f32; 3], f32)) -> Option<particles::WorldQuad> {
	let m = particles::euler_m(rot);
	let (cx, cy, cz) = (sphere[0] * scale, sphere[1] * scale, sphere[2] * scale);
	let centre = [0, 1, 2].map(|i| pos[i] + (m[0][i] * cx + m[1][i] * cy + m[2][i] * cz) / 1024.0);
	let a = crate::shadows::ship_alpha(centre, sphere[3] * scale, cam.0, cam.1, cam.2, cam.3, cam.4, true)?;
	let a = crate::shadows::ship_fade(a, Some(c))?;
	crate::shadows::ship_shadow_m(world, pos, m, t, a)
}

fn ship_shadow_quads(id: u32, sc: &scene::Scene, tick: f32, pad: ([f32; 3], f32), ship: usize, actor_slots: &[(u32, usize, u32)], prop_anims: &HashMap<u32, (mobies::MobyClass, anim::Animated)>, world: &World, flight_tick: Option<i32>) -> Vec<particles::WorldQuad> {
	let (pad_pos, yaw) = pad;
	let Some(mut k) = sc.camera_at(tick) else { return Vec::new() };
	k.rot[2] += yaw;
	let eye = pad_place(pad_pos, yaw, k.pos);
	let (fwd, up) = (k.forward(), k.up());
	let right = [fwd[1] * up[2] - fwd[2] * up[1], fwd[2] * up[0] - fwd[0] * up[2], fwd[0] * up[1] - fwd[1] * up[0]];
	let m = particles::euler_m([0.0, 0.0, yaw]);
	let mut out = Vec::new();
	for &(sid, index, class_id) in actor_slots {
		if sid != id || class_id.wrapping_sub(0x213) >= 3 {
			continue;
		}
		let (Some(actor), Some((class, _))) = (sc.actors.iter().find(|a| a.index == index), prop_anims.get(&class_id)) else { continue };
		let (Some((ka, _, _)), Some(p0)) = (actor.pair(tick), actor.pos_at(tick)) else { continue };
		let pos = pad_place(pad_pos, yaw, p0);
		// moby[0..3] (FUN_00265770): pos * 1024 + R * (centre * s), radius r * s, in 1024 units.
		let (s, c) = (class.scale, ka.sphere);
		let centre = [0, 1, 2].map(|i| pos[i] + (m[0][i] * (c[0] * s) + m[1][i] * (c[1] * s) + m[2][i] * (c[2] * s)) / 1024.0);
		let Some(a) = crate::shadows::ship_alpha(centre, c[3] * s, eye, fwd, right, up, k.tan_half_fov, true) else { continue };
		let Some(a) = crate::shadows::ship_fade(a, flight_tick) else { continue };
		out.extend(crate::shadows::ship_shadow(world, pos, yaw, ship, a));
	}
	out
}








/// AUD-11f: the position a loop voice follows each frame: its owner (a prop, or the hero for the hero loop slots)
/// raised one unit (FUN_0029ff68). None for a prop index that does not exist. BRIDGE-49: the Visibomb missile's loop
/// follows `vis` (the missile position while it flies).
fn loop_at(owner: usize, props: &[game::Prop], hero: [f32; 3], vis: Option<[f32; 3]>) -> Option<[f32; 3]> {
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
enum LoopAct {
Start,
Stop,
Update,
Idle,
}

/// Wanted loop: start when dead and audible, stop when live and out of range, follow when live and audible.
fn loop_act(live: bool, audible: bool) -> LoopAct {
match (live, audible) {
	(false, true) => LoopAct::Start,
	(true, false) => LoopAct::Stop,
	(true, true) => LoopAct::Update,
	(false, false) => LoopAct::Idle,
}
}



/// FRAME-1e: the per-tick render state of this tick's camera `c`: the glint cull (FUN_00220518) and the FRAME-1a moby
/// draw verdict (BRIDGE-33), read by the next tick's scheduler. `up` is the view's up (BRIDGE-56: the rig's `up()`).
fn tick_render_state(gpu: &mut Option<Gpu>, game: &mut game::Game, c: ([f32; 3], f32, f32, f32), up: [f32; 3]) {
	let c = Camera { pos: c.0, yaw: c.1, pitch: c.2, tan_half: c.3, up };
	let f = forward(&c);
	let right = crate::physics::normalize(crate::physics::cross(f, up));
	let up = crate::physics::cross(right, f);
	game.glint_cull(c.pos, right, up, f);
	if let Some(g) = gpu {
		let aspect = g.config.width as f32 / g.config.height.max(1) as f32;
		let v = g.props_verdict(&c, aspect, &game.props, &game.fade_centres(), &game.fade_radii());
		if !v.is_empty() {
			game.set_draw_verdict(&v);
		}
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

/// BRIDGE-75 (decomp/notes/port_bridges.md): the hero model rows R = euler_m([roll, pitch, yaw]) of the game tick; a
/// model vector v goes to the world as v[0] R[0] + v[1] R[1] + v[2] R[2]. Untilted it is the yaw-only turn.
fn hero_rows(p: &crate::player::Player) -> [[f32; 3]; 3] {
	if p.body_tilt == [0.0; 2] {
		let (s, c) = p.yaw.sin_cos();
		[[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]]
	} else {
		particles::euler_m([p.body_tilt[0], p.body_tilt[1], p.yaw])
	}
}

/// M3a: the hero's joint override records and weighted layers for the draw: retail's (+0x64 records, `HeroParts`) in
/// retail mode, else the port's player state.
fn hero_layers(parts: Option<&crate::retail::HeroParts>, p: &crate::player::Player, map: &[Option<usize>]) -> (Vec<anim::JointFx>, Option<anim::LayerPose>, Option<anim::LayerPose>, [Option<anim::LayerPose>; 2]) {
	match parts {
		Some(q) => {
			// Modern controls: a backpedal fades out retail's leg bends, made for the legs facing the path (retail/strafe.rs).
			let mut fx = q.hero_fx.clone();
			crate::retail::strafe::leg_fx(&mut fx, q.strafe);
			(fx, None, None, [None, None])
		}
		None => (p.hero_joint_fx(0, map), p.throw_layer.map(|l| l.pose()), p.weapon_layer1.map(|l| l.pose()), p.hold_poses()),
	}
}

/// Hero model -> world offset through `hero_rows`.
fn hero_to_world(r: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
	[v[0] * r[0][0] + v[1] * r[1][0] + v[2] * r[2][0], v[0] * r[0][1] + v[1] * r[1][1] + v[2] * r[2][1], v[0] * r[0][2] + v[1] * r[1][2] + v[2] * r[2][2]]
}

/// World offset -> hero model (the transpose of `hero_to_world`).
fn hero_to_model(r: &[[f32; 3]; 3], d: [f32; 3]) -> [f32; 3] {
	[d[0] * r[0][0] + d[1] * r[0][1] + d[2] * r[0][2], d[0] * r[1][0] + d[1] * r[1][1] + d[2] * r[1][2], d[0] * r[2][0] + d[1] * r[2][1] + d[2] * r[2][2]]
}

#[cfg(test)]
mod tilt_tests {
	use super::*;

	// BRIDGE-75: tilt [0, 0] keeps the yaw-only rows; tilt [0, 0.3] at yaw 0 maps model (0, 0, 1) to (sin 0.3, 0, cos 0.3).
	#[test]
	fn hero_rows_tilt() {
		let mut p = crate::player::Player::new([0.0; 3], 0.7);
		let r = hero_rows(&p);
		let (s, c) = 0.7f32.sin_cos();
		assert_eq!(hero_to_world(&r, [1.0, 0.0, 0.0]), [c, s, 0.0]);
		p.yaw = 0.0;
		p.body_tilt = [0.0, 0.3];
		let r = hero_rows(&p);
		let v = hero_to_world(&r, [0.0, 0.0, 1.0]);
		assert!((v[0] - 0.3f32.sin()).abs() < 1e-6 && v[1].abs() < 1e-6 && (v[2] - 0.3f32.cos()).abs() < 1e-6, "{v:?}");
		let m = hero_to_model(&r, v);
		assert!(m[0].abs() < 1e-6 && m[1].abs() < 1e-6 && (m[2] - 1.0).abs() < 1e-6, "{m:?}");
	}
}

/// Agent test runs (RAC_TEST_WINDOW=hidden, or the smoke check's RAC_RETAIL_SMOKE): the window lives off-screen.
fn test_window() -> bool {
	std::env::var_os("RAC_TEST_WINDOW").map_or(false, |v| v == "hidden") || std::env::var_os("RAC_RETAIL_SMOKE").is_some()
}

/// Graphics -> Display mode: the `set_fullscreen` argument when the wanted mode (`want`, true = Fullscreen) differs from the
/// window's (`is`): borderless fullscreen on the window's current monitor (no video mode change, alt-tab stays instant), or
/// None = back to the window.
fn display_change(want: bool, is: bool) -> Option<Option<winit::window::Fullscreen>> {
	(want != is).then(|| want.then(|| winit::window::Fullscreen::Borderless(None)))
}

#[cfg(test)]
mod display_tests {
	use super::display_change;
	use winit::window::Fullscreen;

	#[test]
	fn display_mode_switches_only_on_change() {
		assert_eq!(display_change(false, false), None);
		assert_eq!(display_change(true, true), None);
		assert_eq!(display_change(true, false), Some(Some(Fullscreen::Borderless(None))));
		assert_eq!(display_change(false, true), Some(None));
	}
}

/// [PC] Water hold: (level, last field with pool strips, last field with RAM oceans) and how many fields an empty list is bridged.
const WATER_HOLD: u32 = 4;
thread_local! {
	static WATER_KEEP: std::cell::RefCell<(usize, u32, u32)> = const { std::cell::RefCell::new((usize::MAX, 0, 0)) };
}
