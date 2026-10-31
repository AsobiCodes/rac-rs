//! RedrawRequested: the level switch and travel, messages and the free camera keys.

use super::*;

impl App {
	pub(super) fn frame_switch(&mut self, f: &mut Frame) {
		let dt = f.dt;
		if self.retail.is_some() {
			// Retail: the front end runs in the game's own code (compiled or interpreted) and the mirror draws it from its RAM (viewer/front_mirror.rs).
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
					// [PC] The appended game starts every pool prop afresh (shown and cast off): bind this field's mobys again at
					// once, else the display frames left in this field draw no retail moby and no off-screen moby's sun shadow.
					if let Some(g) = self.game.as_mut() {
						r.sync(g);
					}
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
	}
}
