//! RedrawRequested: the game ticks of the frame, and the per-tick render state.

use super::*;

/// FRAME-1e: the per-tick render state of this tick's camera `c`: the glint cull (FUN_00220518) and the FRAME-1a moby
/// draw verdict (BRIDGE-33), read by the next tick's scheduler. `up` is the view's up (BRIDGE-56: the rig's `up()`).
pub(super) fn tick_render_state(gpu: &mut Option<Gpu>, game: &mut game::Game, c: ([f32; 3], f32, f32, f32), up: [f32; 3]) {
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

impl App {
	pub(super) fn frame_ticks(&mut self, f: &mut Frame) {
		let tick_wait_max = f.tick_wait_max;
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
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
					// [PC] TEST ONLY: RAC_TEST_CLOCK counts the retail fields shown (viewer/test_clock.rs).
					test_clock::set_fields(r.shown_field());
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
						let (from_rev, clock_jumped) = crate::retail::strafe::draw_clock(&mut self.hero_rig.hero.clock, n, parts.strafe.rev);
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
						// Modern controls: the drawn run phase jumped (the clip flipped forward <-> reversed with no pose before it to
						// carry on from, strafe.rs `draw_clock`) or the leg twist jumped past strafe.rs's rate limit (Strafe aiming switched
						// off mid-twist): this tick's pose is drawn unblended (frame smoothing, smooth.rs `hero_cut`; pose overrides appearing / vanishing at full weight too, anim_pose.rs `ops_need_cut`).
						if clock_jumped || self.retail_parts.as_ref().is_some_and(|o| (o.strafe.twist - parts.strafe.twist).abs() > 0.6 || crate::anim_pose::ops_need_cut(&o.pose_ops, &parts.pose_ops)) {
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
		}
	}
}
