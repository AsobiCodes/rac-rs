//! RedrawRequested: post phase (blob and ship shadows, sound events collected from the field's ticks, camera).

use super::*;

impl App {
	pub(super) fn frame_shadows(&mut self, f: &mut Frame) {
		let paused = f.paused;
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
			// PC-FPS-1: draw between the last two ticks (off, paused or capturing: the newest tick as is).
			// [PC] Frame smoothing: retail mode follows Graphics -> Frame smoothing (interp.rs; settings.txt `smooth` with RAC_PC_EXT=0),
			// and blends nothing on a cut field.
			let menu = if self.retail.is_some() && crate::retail::pcext::enabled() { super::interp::wanted(crate::retail::pcext::graphics().smoothing) } else { self.settings.smooth };
			let want = menu && !paused && (self.shot.is_none() || { static S: std::sync::OnceLock<bool> = std::sync::OnceLock::new(); *S.get_or_init(|| std::env::var_os("RAC_SMOOTH_SHOT").is_some()) });
			let on = super::interp::frame(self.retail.as_mut(), want, test_clock::alpha((self.accum / crate::retail::disc::field_dt()).clamp(0.0, 1.0)));
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
			f.world_quads = world_quads;
		}
	}

	pub(super) fn frame_sound_events(&mut self, f: &mut Frame) {
		let (sounds, music_reqs, shots, loop_cues) = (&mut f.sounds, &mut f.music_reqs, &mut f.shots, &mut f.loop_cues);
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
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
		}
	}

	pub(super) fn frame_camera(&mut self, f: &mut Frame) {
		let (dt, wdt) = (f.dt, f.wdt);
		let (sounds, shots) = (&mut f.sounds, &mut f.shots);
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
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
			// [PC] lane scratch_veg debug: RAC_VEG_CAM="x y z yaw pitch [yaw/s]" an absolute world camera (vegetation checks; an optional
			// turn rate); a leading "~" makes x y z an offset from the game's camera.
			let veg = std::env::var("RAC_VEG_CAM").ok().map(|v| (v.starts_with('~'), v.trim_start_matches('~').split_whitespace().filter_map(|x| x.parse::<f32>().ok()).collect::<Vec<_>>()));
			if let Some((rel, v)) = veg.filter(|v| v.1.len() == 5 || v.1.len() == 6) {
				let o = if rel { self.cam.pos } else { [0.0; 3] };
				(self.cam.pos, self.cam.yaw, self.cam.pitch, self.cam.up) = ([o[0] + v[0], o[1] + v[1], o[2] + v[2]], v[3] + v.get(5).map_or(0.0, |r| r * self.clock), v[4], [0.0, 0.0, 1.0]);
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
			f.at = at;
		}
	}
}
