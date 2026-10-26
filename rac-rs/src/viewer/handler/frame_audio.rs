//! RedrawRequested: the audio phase (playing the frame's collected sounds, animation cues, music, positional ambience).

use super::*;

impl App {
	pub(super) fn frame_audio_events(&mut self, f: &mut Frame) {
		let paused = f.paused;
		let (mut sounds, mut loop_cues, mut shots) = (std::mem::take(&mut f.sounds), std::mem::take(&mut f.loop_cues), std::mem::take(&mut f.shots));
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
			f.native_sfx = native_sfx;
		}
	}

	pub(super) fn frame_audio_anim(&mut self, f: &mut Frame) {
		let paused = f.paused;
		let native_sfx = f.native_sfx;
		if let Some(a) = &mut self.audio {
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
		}
	}

	pub(super) fn frame_audio_music(&mut self, f: &mut Frame) {
		let dt = f.dt;
		let music_reqs = std::mem::take(&mut f.music_reqs);
		if let Some(a) = &mut self.audio {
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
			f.music_paused = music_paused;
		}
	}

	pub(super) fn frame_audio_ambience(&mut self, f: &mut Frame) {
		let paused = f.paused;
		let (native_sfx, music_paused) = (f.native_sfx, f.music_paused);
		if let Some(a) = &mut self.audio {
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
	}
}
