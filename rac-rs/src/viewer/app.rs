use super::*;

/// What `load_level` leaves in the app besides its big result tuple: the class rank and row tables (gameplay.rs).
#[derive(Default)]
pub(super) struct LevelExt(std::collections::HashMap<u32, u32>, std::collections::HashMap<u32, u32>);

type Prefetch<T> = std::thread::JoinHandle<(T, LevelExt, Vec<crate::retail::Effect>, bool, Option<Scene>, Option<crate::physics::World>)>;

/// [PC] What the prefetch thread tells the level switch as soon as its parse is done: the placements and actor classes the prop slot
/// order is made of, the global writes it queued (they run on the switch's thread), and whether its rand() was left alone.
pub(super) struct Early {
	instances: Vec<crate::gameplay::MobyInstance>,
	actors: Vec<u32>,
	effects: Vec<crate::retail::Effect>,
	widen: Vec<crate::retail::Widen>,
}

/// The switch's end of the prefetch thread's channels: `order` is the prop slot order the thread's scene build waits for.
pub(super) struct PrefetchLink {
	early: std::sync::mpsc::Receiver<Early>,
	/// (order, final): a speculative order (from `pools_hint`, before the arrival) or the final one (from the arrival's pools).
	order: std::sync::mpsc::Sender<(Vec<u32>, bool)>,
	/// The thread's `Early` once received, and the final order sent to it (from `prefetch_ready`, before the switch).
	got: Option<Early>,
	sent: Option<Vec<u32>>,
	/// The last speculative order sent (`prefetch_spec`) and the hint pools it came from.
	spec: Option<(Vec<(u32, u32)>, Vec<u32>)>,
	/// The hint pools `prefetch_spec` saw last and since when they hold unchanged; how many speculative orders it worked out.
	calm: Option<(Vec<(u32, u32)>, std::time::Instant)>,
	tries: u32,
	/// Set by the thread when its scene build is over.
	done: std::sync::Arc<std::sync::atomic::AtomicBool>,
	since: std::time::Instant,
}

/// [PC] `prefetch_spec` acts on the first hint pools at once (a normal landing: one hint ~0.4 s before the arrival, kept), on changed
/// ones only once they held this long unchanged, and works out at most `SPEC_TRIES` orders per travel. A hint that changes every
/// field (Rilgar -> Blarg before `retail::HINT_SLOTS`: ~30 s of it) cost a new order each frame (`slots_game`, 40-50 ms of the main
/// thread: 15-19 fps) and made the prefetch thread restart its prop slot build for each one (GPU memory +128 MB every 2-4 s,
/// 2.4 -> 4.2 GB). The arrival's order (`prefetch_ready`) still decides what is built.
const SPEC_STABLE: std::time::Duration = std::time::Duration::from_millis(300);
const SPEC_TRIES: u32 = 3;

/// [PC] Drops `t` on a short-lived thread: freeing a level's data (the old scene's buffers, the parsed meshes and textures,
/// hundreds of MB) took 20-70 ms of the main thread at the level switch. Nothing else changes (drop order of unrelated data).
fn drop_later<T: Send + 'static>(t: T) {
	let _ = std::thread::Builder::new().name("level-drop".into()).spawn(move || drop(t));
}

/// Sets the flag when dropped (the prefetch thread ends or panics): the switch stops waiting for it.
struct DoneGuard(std::sync::Arc<std::sync::atomic::AtomicBool>);

impl Drop for DoneGuard {
	fn drop(&mut self) {
		self.0.store(true, std::sync::atomic::Ordering::Release);
	}
}

/// The prefetch thread's handle type, named through the (unnameable) result type of the load job.
fn handle_of<T, F: FnOnce(&mut Option<frontend::Art>, &mut LevelExt) -> T>(_job: &F) -> std::marker::PhantomData<Prefetch<T>> {
	std::marker::PhantomData
}

fn downcast_handle<H: 'static>(b: Box<dyn std::any::Any + Send>, _ty: std::marker::PhantomData<H>) -> Option<H> {
	b.downcast::<H>().ok().map(|b| *b)
}

/// Ratchet with the wrench, the glove, the glove's bomb, the Pyrocitor and its nozzle appended (in that order); also built by the
/// prefetch thread (by reference), so a macro over the parse result's parts.
macro_rules! combined_hero {
	($hero_anim:expr, $wrench:expr, $glove:expr, $held_bomb:expr, $pyro_rig:expr, $suck_rig:expr, $held_rigs:expr, $back_rig:expr, $clank_rig:expr, $hook_rig:expr, $light_rig:expr) => {
$hero_anim.as_ref().map(|h| {
					let src = &h.0.mesh;
					let mut m = Mesh { name: src.name.clone(), positions: src.positions.clone(), uvs: src.uvs.clone(), faces: src.faces.clone(), colours: src.colours.clone(), lit: None, strip_phase: src.strip_phase.clone(), normals: src.normals.clone() };
					if let Some((wm, (held, _), ..)) = &$wrench {
						// The $wrench, then its two afterimage copies (BRIDGE-28), before the $glove.
						for _ in 0..3 {
							let base = m.positions.len() as u32;
							m.positions.extend(held.iter().copied());
							m.uvs.extend(wm.uvs.iter().copied());
							let was = m.strip_phase.len() == m.faces.len();
							m.faces.extend(wm.faces.iter().map(|&(t, f)| (t, f.map(|i| i + base))));
							m.extend_phases(wm, was);
						}
					}
					let mut append = |src: &Mesh| {
						let base = m.positions.len() as u32;
						m.positions.extend(src.positions.iter().copied());
						m.uvs.extend(src.uvs.iter().copied());
						let was = m.strip_phase.len() == m.faces.len();
						m.faces.extend(src.faces.iter().map(|&(t, f)| (t, f.map(|i| i + base))));
						m.extend_phases(src, was);
					};
					if let Some(g) = &$glove {
						append(&g.class.mesh);
					}
					if let Some((bm, _, _)) = &$held_bomb {
						append(bm);
					}
					if let Some(r) = &$pyro_rig {
						append(&r.held.class.mesh);
						if let Some((nm, ..)) = &r.nozzle {
							append(nm);
						}
					}
					if let Some(r) = &$suck_rig {
						append(&r.held.class.mesh);
					}
					for r in game::held::rig_classes().into_iter().filter_map(|c| $held_rigs.get(&c)) {
						append(&r.held.class.mesh);
					}
					// BRIDGE-76: the Swingshot hook after the held rigs.
					if let Some(h) = &$hook_rig {
						append(&h.class.mesh);
					}
					// BRIDGE-21: d0 then d4 after the suck cannon.
					for h in $back_rig.iter().chain($clank_rig.iter()) {
						append(&h.class.mesh);
					}
					if let Some(lc) = &$light_rig {
						append(&lc.mesh);
					}
					// BRIDGE-40 (WPN-6.6a): three whole-hero afterimage copies at the end.
					for _ in 0..3 {
						append(&h.0.mesh);
					}
					m
				})
	};
}

/// The game the prop slot order is read from (it must match the running game's `Game::with_story`).
/// NOV-24: the level's own pools (Novalis debris and pipe child) exist only for its level index, so the
/// order is built for it; at level -1 the later slots (the vendor orb) lost their meshes.
pub(super) fn slots_game(instances: &[crate::gameplay::MobyInstance], level: i32) -> Game {
	Game::with_story(instances, &HashMap::new(), ([0.0; 3], 0.0), 0.0, Default::default(), level)
}

impl App {
	/// Per frame: the pause menu (update, held volume keys, RAC_PAUSE_KEYS), the wheel, the settings.
	pub(super) fn pause_frame(&mut self, dt: f32) {
		let held = |k: &[KeyCode]| k.iter().any(|k| self.keys.contains(k));
		let dir = held(&[KeyCode::ArrowRight, KeyCode::KeyD]) as i32 - held(&[KeyCode::ArrowLeft, KeyCode::KeyA]) as i32;
		// Quick Select: Triangle held opens the wheel; the stick picks a sector (0 = up, clockwise).
		if let Some(t) = &mut self.tri_held {
			*t += dt;
			let open = self.game.as_ref().is_some_and(|g| g.dead.is_none()) && self.flow.as_ref().map_or(true, |f| f.cutscene.is_none());
			if open {
				let (x, y) = (dir as f32, held(&[KeyCode::ArrowUp, KeyCode::KeyW]) as i32 as f32 - held(&[KeyCode::ArrowDown, KeyCode::KeyS]) as i32 as f32);
				// Opens on the equipped item's sector [I].
				let eq = self.game.as_ref().and_then(|g| g.inv.quick.iter().position(|&q| q != 0 && q == g.inv.equipped));
				let cur = self.wheel.or(eq).unwrap_or(0);
				let sel = if x == 0.0 && y == 0.0 { cur } else { ((x.atan2(y).to_degrees() + 360.0 + 22.5) / 45.0) as usize % 8 };
				self.wheel = Some(sel);
			}
		}
		let f = &mut self.wheel_fade;
		f.0 = if self.wheel.is_some() { (f.0 + dt / WHEEL_FADE).min(1.0) } else { (f.0 - dt / WHEEL_FADE).max(0.0) };
		if let Some(s) = self.wheel {
			f.1 = s;
		}
		self.apply_settings();
	}

	/// Applies changed Options (audio volumes, mono) and writes them once off the Sound page.
	pub(super) fn apply_settings(&mut self) {
		if let Some(a) = &mut self.audio {
			let (sfx, music, mono) = (self.settings.sfx_gain(), self.settings.music_gain(), !self.settings.stereo);
			if (a.master, a.music, a.mono) != (sfx, music, mono) {
				(a.master, a.music, a.mono) = (sfx, music, mono);
			}
		}
	}

	/// Enter play mode at the ground under the camera, or leave it.
	pub(super) fn toggle_play(&mut self) {
		if self.game.take().is_some() {
			self.flow = None;
			self.retail = None;
			if let Some(g) = &mut self.gpu {
				g.set_marker(None, None, None);
				g.set_props(None, &HashMap::new(), &HashMap::new(), &[], &[], &[], &[], &[]);
			}
			return;
		}
		let Some(w) = &self.world else { return };
		let here = self.keys.contains(&KeyCode::ShiftLeft) || self.spawn.is_none();
		let (feet, yaw) = if here {
			let c = self.cam.pos;
			match w.ground(c, 5000.0) {
				Some((z, _)) => ([c[0], c[1], z + 0.05], self.cam.yaw),
				None => {
					eprintln!("no ground under the camera");
					return;
				}
			}
		} else {
			let (p, yaw) = self.spawn.unwrap();
			([p[0], p[1], p[2] + 0.05], yaw)
		};
		self.cam.yaw = yaw;
		self.cam.pitch = player::CAMERA_PITCH;
		// RAC_HERO="x y z yaw": start somewhere else (for scripted --shot scenes).
		let hero: Option<Vec<f32>> = std::env::var("RAC_HERO").ok().map(|v| v.split_whitespace().filter_map(|x| x.parse().ok()).collect()).filter(|v: &Vec<f32>| v.len() == 4);
		let (feet, yaw) = hero.map_or((feet, yaw), |v| ([v[0], v[1], v[2]], v[3].to_radians()));
		let resume = self.resume.take().filter(|w| w.level == self.level);
		let (feet, yaw) = resume.as_ref().map_or((feet, yaw), |w| (w.pos, w.yaw));
		self.cam.yaw = yaw;
		// Story state: the running game's (death reload), else the save's (unlocks, flags, NPC records).
		let story = self.game.as_ref().map(|g| g.persist()).or_else(|| self.save.as_ref().map(|(_, sv)| sv.story()));
		// The mission bytes go in before the actors are built (phase bits, rewards).
		// Level init adds 180 A3 units before the first game frame.
		self.a3_ticks = self.a3_ticks.wrapping_add(180);
		self.hero_moby = None;
		self.ckpt_env = None;
		self.ckpt_seen = 0;
		self.game = Some(Game::with_story(&self.instances, &self.bounds, (feet, yaw), self.death_height, { let mut st = story.unwrap_or_default(); st.enter_level(self.level, self.a3_ticks); st }, self.level as i32));
		// SHR-FRAME1 rule 2: a level entry's first gameplay frame reads the counter retail leaves after its arrival.
		if let Some(g) = &mut self.game {
			g.frames = super::travel::arrival_frames(self.level as usize, &g.story);
		}
		if let Some(g) = &mut self.game {
			g.class_spheres = self.debris_bspheres.clone(); // NOV-9.1
			g.class_kinds = self.class_kinds.clone(); // WPN-6.6b1
			g.class_scales = self.class_scales.clone(); // BRIDGE-53
			g.class_ranks = self.class_ranks.clone(); // SCH-6b
			g.set_class_rows(self.class_rows.clone()); // ARI-6
			if let Some(ov) = &self.level_overlay {
				// REN-2 ship mesh (Veldin), NOV-15.1b / NOV-23.1 strip tables (Novalis); ARI-6 per-level dispatch.
				g.load_overlay_tables(self.level as i32, ov);
			}
			g.arrive(self.level);
			g.set_group_source(&self.instances, self.groups.clone()); // NOV-3.4
			g.set_cuboids(self.cuboids.clone());
			g.set_cuboid_basis(self.cuboid_basis.clone()); // KER-5
			g.set_paths(self.polygons.iter().map(|p| p.points.clone()).collect()); // ARI-25
			g.set_path_w(self.polygons.iter().map(|p| p.marks.clone()).collect()); // KER-1a
			g.set_grind_rails(self.grind_rails.clone()); // KER-60
			// ARI-22 / BRIDGE-59: the gameplay cylinders (the Aridia elevator shafts).
			g.set_cylinders(self.rig.as_ref().map(|r| { let grid = r.grid(); (0..grid.cylinder_count()).filter_map(|k| grid.cylinder(k)).collect() }).unwrap_or_default());
			g.set_colliders(self.colliders.clone());
			g.set_sequences(self.prop_anims.iter().map(|(&c, (_, a))| (c, anim::share_timing(&a.sequences))).collect());
			if let Some((_, a)) = &self.hero_anim {
				g.set_hero_sequences(&a.sequences);
			}
			if let Some(r) = &self.pyro_rig {
				g.set_pyro_timing(anim::share_timing(&r.held.anim.sequences));
			}
			if let Some(r) = &self.suck_rig {
				g.set_suck_timing(anim::share_timing(&r.held.anim.sequences));
			}
			for (&c, r) in &self.held_rigs {
				g.set_held_timing(c, anim::share_timing(&r.held.anim.sequences));
			}
			if let Some(h) = &self.hook_rig {
				g.set_held_timing(game::held::SWINGSHOT_HOOK, anim::share_timing(&h.anim.sequences));
			}
			g.set_pyro_frames(self.pyro_frames);
			g.set_muzzles(self.prop_anims.iter().filter_map(|(&c, (_, a))| a.attach_pos(game::BIRDBOT_FIRE.0, game::BIRDBOT_FIRE.1).map(|p| (c, p))).collect());
			g.set_attach_joints(self.prop_anims.iter().filter_map(|(&c, (_, a))| a.attach0.map(|j| (c, j))).collect()); // NOV-2.5
			g.set_attach_points(self.prop_anims.iter().filter(|(c, _)| game::JOINT_WORLD_CLASSES.contains(*c)).map(|(&c, (_, a))| (c, a.attach_points.clone())).collect()); // BRIDGE-3
			g.set_mouse_joints(self.prop_anims.get(&game::MOUSE).map_or([None; 2], |(_, a)| a.attach12)); // NOV-14.3
			g.set_seq_spheres(self.prop_anims.iter().map(|(&c, (m, a))| (c, (a.sequences.iter().map(|q| q.sphere).collect(), m.scale))).collect());
			g.set_jet_joint(self.prop_anims.iter().filter(|(c,_)| matches!(**c,660|688)).filter_map(|(&c,(_,a))| a.joint_pos(0,0.0,0).map(|p|(c,p))).collect());
			g.set_hit_joints(
				self.prop_anims
					.iter()
					.filter(|(c, _)| **c == game::BIRDBOT)
					.filter_map(|(&c, (_, a))| game::BIRDBOT_HIT_JOINTS.iter().map(|&j| a.joint_pos(game::BIRDBOT_FIRE.0, game::BIRDBOT_FIRE.1, j)).collect::<Option<Vec<_>>>().map(|v| (c, v)))
					.collect(),
			);
			// Joint primitives follow the live pose: each class with joint shapes gets its skeleton as a joint source.
			g.set_joint_rigs(
				self.prop_anims
					.iter()
					.filter(|(c, _)| **c == game::ROBOT || **c == game::TOAD || **c == game::BIRDBOT || **c == game::MOUSE || game::JOINT_WORLD_CLASSES.contains(*c) || self.colliders.get(c).is_some_and(|k| k.prims.iter().any(|(p, _)| matches!(p, mobies::Prim::JointSphere { .. } | mobies::Prim::JointCapsule { .. }))))
					.map(|(&c, (_, a))| {
						let a = std::rc::Rc::new(a.clone());
						let rig: game::JointRig = std::rc::Rc::new(move |clock: &anim::PropAnim, joints: &[usize]| {
							if !a.sequences.get(clock.seq).is_some_and(|s| !s.frames.is_empty()) {
								return None;
							}
							let pos = clock.joint_positions(&a)?;
							joints.iter().map(|&j| pos.get(j).copied()).collect()
						});
						(c, rig)
					})
					.collect(),
			);
			// BRIDGE-61 (ARI-24): the joint axes of the classes that read them, from the same posed skeleton: the joint world
			// matrix's columns (translation in column 3) are the joint's x / y / z axes.
			g.set_joint_rows_rigs(
				self.prop_anims
					.iter()
					.filter(|(c, _)| game::JOINT_ROWS_CLASSES.contains(*c))
					.map(|(&c, (_, a))| {
						let a = std::rc::Rc::new(a.clone());
						let rig: game::JointRowsRig = std::rc::Rc::new(move |clock: &anim::PropAnim, j: usize| {
							if !a.sequences.get(clock.seq).is_some_and(|s| !s.frames.is_empty()) {
								return None;
							}
							let mats = clock.pose(&a, &[]);
							if j >= mats.len() {
								return None;
							}
							let m = a.joint_world(&mats, j);
							Some([[m[0], m[4], m[8]], [m[1], m[5], m[9]], [m[2], m[6], m[10]]])
						});
						(c, rig)
					})
					.collect(),
			);
			if let Some((id, n)) = self.give {
				g.give_item(id, n);
			}
			if let Some(w) = &resume {
				eprintln!("resume at {:?} yaw {:.2}", w.pos, w.yaw);
				// Items, ammo and quick-select come from the save's own records; the sidecar only adds what
				// they do not hold (hand, worn items), and an old sidecar's inventory is the fallback.
				let mut inv = g.inv.clone();
				if inv.have.iter().filter(|h| **h).count() <= 1 {
					inv = w.inv.clone();
				} else {
					inv.equipped = w.inv.equipped;
					inv.worn = w.inv.worn;
				}
				g.resume(w.pos, w.yaw, w.health, w.moves, inv);
			}
			// RAC_HEALTH=n: start hurt (for scripted --shot scenes, e.g. the nanotech orb).
			if let Some(h) = std::env::var("RAC_HEALTH").ok().and_then(|v| v.parse::<i32>().ok()) {
				g.health = h.clamp(1, 4);
			}
		}
		self.play_ticks = 0;
		self.shake = camera::CamShake::default();
		if let Some(r) = &mut self.rig {
			r.reset(feet, yaw);
		}
		self.rig_hero = feet;
		self.flow = self.flow_start.clone();
		if let (Some(f), Some(w)) = (&mut self.flow, &resume) {
			f.restore(w.record, &w.seen);
		}
		self.hud_seen = (u32::MAX, -1);
		self.nano_hud = (0, 0, 0, -1);
		self.nano_live = true;
		self.nano_acc = 0.0;
		// Retail mode: the savestate's own game runs the level, so no rac-rs story flow either.
		if crate::retail::active() {
			self.flow = None;
			// The savestate is mid-level: no level-start nanotech meter.
			self.nano_live = false;
			self.retail = self.retail_carry.take().or_else(crate::retail::Retail::load);
			// The window plays the game's movies (retail/fmv_bridge.rs); headless runs never say so, so they skip them.
			crate::retail::fmv_bridge::set_serviced(self.retail.is_some());
		}
	}

	/// [PC] Retail pool growth (retail.rs `take_grow`) without a level reload: the prop slots for the grown pools (set
	/// by the caller with `retail::set_pools`) in `load_level`'s slot order, rebuilt in the loaded scene. False when the
	/// scene cannot take them; the caller then reloads the level as before.
	pub(super) fn regrow_props(&mut self) -> bool {
		if self.retail_prop_meshes.is_empty() {
			return false;
		}
		let slots_game = slots_game(&self.instances, self.level as i32);
		let mut order: Vec<u32> = slots_game.props.iter().map(|p| p.class_id).collect();
		order.extend(self.actor_slots.iter().map(|s| s.2));
		let Some(gpu) = &mut self.gpu else { return false };
		gpu.regrow_props(&order, &self.retail_prop_meshes)
	}

	/// [PC] Retail pool growth (retail.rs `take_grow_kept`) in the running game: the props `new` adds over `old` (per class
	/// in class order) appended to the game and their slots to the scene (`Gpu::append_props`), drawn in a rebuilt game's
	/// order. `order` is the order the game's props are bound in (retail.rs `bind_order`, None = index order). Every pool
	/// prop starts afresh, as in a rebuilt game. The new bind order, or None (game and scene untouched) when that cannot
	/// be done; the caller then rebuilds the slots (`regrow_props`).
	pub(super) fn append_pools(&mut self, old: &[(u32, u32)], new: &[(u32, u32)], order: Option<&[usize]>) -> Option<Vec<usize>> {
		let old: HashMap<u32, u32> = old.iter().copied().collect();
		let mut classes = Vec::new();
		for &(c, n) in new {
			let k = old.get(&c).copied().unwrap_or(0);
			if n < k {
				return None;
			}
			classes.extend(std::iter::repeat_n(c, (n - k) as usize));
		}
		let (Some(game), Some(gpu)) = (self.game.as_mut(), self.gpu.as_mut()) else { return None };
		let _h = super::profile::span("ap_order");
		let at = game.props.len();
		let order: Vec<usize> = order.filter(|o| o.len() == at).map_or_else(|| (0..at).collect(), <[usize]>::to_vec);
		if self.retail_prop_meshes.is_empty() || gpu.scene.as_ref().is_none_or(|s| s.props_ranges.len() != at + self.actor_slots.len()) {
			return None;
		}
		// A rebuilt game holds the pools first, by class, then its other props: each class's new props follow its old ones.
		let mut pool: BTreeMap<u32, Vec<usize>> = BTreeMap::new();
		let mut rest = Vec::new();
		for &i in &order {
			match game.props[i].is_retail() {
				true => pool.entry(game.props[i].class_id).or_default().push(i),
				false => rest.push(i),
			}
		}
		for (k, &c) in classes.iter().enumerate() {
			pool.entry(c).or_default().push(at + k);
		}
		let mut order: Vec<usize> = pool.into_values().flatten().collect();
		order.extend(rest);
		let slots: Vec<usize> = order.iter().copied().chain(at + classes.len()..at + classes.len() + self.actor_slots.len()).collect();
		drop(_h);
		if !gpu.append_props(at, &classes, &self.retail_prop_meshes, &slots) {
			return None;
		}
		let _g = super::profile::span("ap_game");
		game.append_retail(&classes, self.level as i32);
		Some(order)
	}

	/// [PC] Retail has arrived on another level (`Retail::switch_pending`): true once the level prefetch has built the whole scene (its prop
	/// slots need the pools the arrival sets, which are given to it here, before the switch sets them), or when there is none to wait for.
	/// False: keep the frame loop going for now (no new picture), the switch comes when it is done.
	pub(super) fn prefetch_ready(&mut self) -> bool {
		let Some(pools) = self.retail.as_ref().and_then(|r| r.pending_pools()) else { return true };
		let target = self.retail.as_ref().and_then(|r| r.travel_target());
		let Some((n, link)) = &mut self.prefetch_link else { return true };
		if Some(*n) != target || link.since.elapsed().as_secs_f32() > 120.0 {
			return true;
		}
		if link.got.is_none() {
			match link.early.try_recv() {
				Ok(e) => link.got = Some(e),
				Err(std::sync::mpsc::TryRecvError::Empty) => return false,
				Err(_) => return true,
			}
		}
		if link.sent.is_none() {
			let e = link.got.as_ref().unwrap();
			let level = *n as i32;
			let order = crate::retail::with_pools(crate::retail::widen_all(pools, &e.widen), || {
				let g = slots_game(&e.instances, level);
				let mut order: Vec<u32> = g.props.iter().map(|p| p.class_id).collect();
				order.extend(e.actors.iter().copied());
				order
			});
			let hit = link.spec.as_ref().is_some_and(|s| s.1 == order);
			let _ = link.order.send((order.clone(), true));
			link.sent = Some(order);
			crate::loadlog::say(|| format!("main: prop slot order sent to the prefetch (speculative one {})", if hit { "kept" } else { "differs" }));
		}
		link.done.load(std::sync::atomic::Ordering::Acquire)
	}

	/// [PC] Retail is landing on the level being prefetched: once its moby table holds mobys (`retail::pools_hint`), the prefetch
	/// thread gets the prop slot order those pools give, so it builds the slots before the arrival. The arrival's order
	/// (`prefetch_ready`) keeps that build when it is the same, else the slots are rebuilt for it.
	pub(super) fn prefetch_spec(&mut self) {
		let Some(target) = self.retail.as_ref().and_then(|r| r.travel_target()) else { return };
		let Some((n, link)) = &mut self.prefetch_link else { return };
		if *n != target || link.sent.is_some() {
			return;
		}
		let Some(hint) = crate::retail::pools_hint(target as u32) else { return };
		if link.spec.as_ref().is_some_and(|s| s.0 == hint) {
			return;
		}
		// [PC] The first hint at once, a changed one only once it held `SPEC_STABLE`; at most `SPEC_TRIES` orders per travel.
		if link.tries >= SPEC_TRIES {
			return;
		}
		match &mut link.calm {
			Some((h, t)) if *h == hint => {
				if link.tries > 0 && t.elapsed() < SPEC_STABLE {
					return;
				}
			}
			c => {
				*c = Some((hint.clone(), std::time::Instant::now()));
				if link.tries > 0 {
					return;
				}
			}
		}
		if link.got.is_none() {
			match link.early.try_recv() {
				Ok(e) => link.got = Some(e),
				Err(_) => return,
			}
		}
		link.tries += 1;
		let e = link.got.as_ref().unwrap();
		let level = *n as i32;
		let order = crate::retail::with_pools(crate::retail::widen_all(hint.clone(), &e.widen), || {
			let g = slots_game(&e.instances, level);
			let mut order: Vec<u32> = g.props.iter().map(|p| p.class_id).collect();
			order.extend(e.actors.iter().copied());
			order
		});
		let same = link.spec.as_ref().is_some_and(|s| s.1 == order);
		if !same {
			let _ = link.order.send((order.clone(), false));
			crate::loadlog::say(|| format!("main: speculative prop slot order sent ({} hint classes)", hint.len()));
		}
		link.spec = Some((hint, order));
	}

	pub(super) fn load_level(&mut self) {
		self.load_level_from(None);
	}

	/// [PC] Parses level `n` on a background thread (the host side of `load_level`: disc, geometry, textures, models), to be
	/// used by the `load_level` that follows the game's travel to it (`take_prefetch`). Touches nothing of the running level.
	pub(super) fn prefetch_level(&mut self, n: usize) {
		self.load_level_from(Some(n));
	}

	fn load_level_from(&mut self, prefetch: Option<usize>) {
		let level_no = prefetch.unwrap_or(self.level);
		if prefetch.is_none() {
crate::loadlog::say(|| "host: load_level start".into());
			// Level initialisation sets the shared rand() state to 1234 (setter FUN_001160c8) before populating.
			particles::Rng::shared().reseed(particles::LEVEL_SEED);
			// ARI-4: the reloaded sky header has sprite count 0; the first Aridia sky pass creates the sprites again.
			self.sky_sprites = None;
			// PC-FPS-1: a level load draws its first ticks unblended.
			self.smooth = Default::default();
		}
		let info = self.levels[level_no].clone();
		let front = prefetch.is_none() && self.want_front;
		let iso_path = self.iso_path.clone();
		let mut front_art = None;
		let job = { let info = info.clone(); move |front_art: &mut Option<frontend::Art>, ext: &mut LevelExt| -> Result<_, String> {
			let mut disc = iso::Disc::open(std::path::Path::new(&iso_path)).map_err(|e| e.to_string())?;
			let l = if front { Level::title(&mut disc)? } else { Level::load(&mut disc, &info)? };
crate::loadlog::say(|| "host: Level::load done (disc open + tables)".into());
			// Baked terrain/tie/shrub colours under the level's light table (decomp/notes/vertex_colours.md).
			let lights = gameplay::lighting(&l).map(|lt| lt.lights).unwrap_or_default();
			// Terrain fragments keep their own per-fragment LOD batch (BRIDGE-31); `meshes` holds the rest of the merged world.
			let occ_map = if l.title { None } else { crate::occlusion::OccMap::new(&l.gameplay) };
			let (tfrag_b, tfrags) = terrain::terrain_lods(l.terrain_block()?, Some(&lights), occ_map.as_ref())?;
crate::loadlog::say(|| "host: terrain_lods done".into());
			let mut meshes: Vec<Mesh> = Vec::new();
			// Ties and the rest are optional layers: report a failure but still show the terrain. Ties keep their own
			// per-instance batch with three LODs (BRIDGE-29), out of the merged world.
			let tie_set = match ties::lit_instances(&l, &lights) {
				Ok(instances) => {
					eprintln!("{} tie instances", instances.len());
					instances
				}
				Err(e) => {
					eprintln!("ties: {e}");
					Vec::new()
				}
			};
crate::loadlog::say(|| "host: ties done".into());
			let world = match collision::collision(&l) {
				Ok(faces) if !faces.is_empty() => {
					eprintln!("collision: {} triangles", faces.len());
					World::homed(faces.into_iter().map(|f| (f.v, f.kind, Some(f.cell))), 4.0)
				}
				r => {
					eprintln!("collision: {}; using the visible geometry", r.err().unwrap_or("empty".into()));
					World::new(tfrags.iter().map(|t| &t.near).chain(meshes.iter()).chain(tie_set.iter().map(|t| &t.lods[0])).flat_map(|m| m.faces.iter().map(|&(_, f)| f.map(|i| m.positions[i as usize]))), 4.0)
				}
			};
crate::loadlog::say(|| "host: collision done".into());
			let spawn = gameplay::spawn(&l).map_err(|e| eprintln!("spawn: {e}")).ok();
			// Shrubs keep their own per-instance batch (BRIDGE-27) with the billboard textures of their classes.
			let shrub_set = match shrubs::lit_instances(&l, &lights) {
				Ok(instances) => {
					eprintln!("{} shrub instances", instances.len());
					instances
				}
				Err(e) => {
					eprintln!("shrubs: {e}");
					Vec::new()
				}
			};
crate::loadlog::say(|| "host: shrubs done".into());
			let bb_tex = l.shrub_billboard_textures().unwrap_or_else(|e| {
				eprintln!("shrub billboard textures: {e}");
				HashMap::new()
			});
			let terrain_chains = if front { Vec::new() } else { l.terrain_mips().unwrap_or_else(|e| {
				eprintln!("terrain mips: {e}");
				Vec::new()
			}) };
			// Shrub mesh textures sample their authored mip chains at the TEX1 level (shrubs::K_LO_SHIFT).
			let shrub_chains = if front { Vec::new() } else { l.mip_chains(crate::level::TextureGroup::Shrub).unwrap_or_else(|e| {
				eprintln!("shrub mips: {e}");
				Vec::new()
			}) };
			// Tie textures too: the GS picks a tie's level from its TEX1 K and MXL like a shrub mesh (viewer.rs `fs`, `vs`).
			let tie_chains = if front { Vec::new() } else { l.mip_chains(crate::level::TextureGroup::Tie).unwrap_or_else(|e| {
				eprintln!("tie mips: {e}");
				Vec::new()
			}) };
			let terrain_chains = (terrain_chains, shrub_chains, tie_chains);
crate::loadlog::say(|| "host: terrain/shrub mips done".into());
			// Mobies stand in their placed pose for now (no animation yet); class 0 is Ratchet.
			let (mut hero, mut instances, mut prop_meshes, mut prop_anims) = (None, Vec::new(), HashMap::new(), HashMap::new());
			let mut leftover = HashMap::new();
			let mut held_bomb = None;
			let mut debris_bspheres = HashMap::new();
			// WPN-6.6b1: the class header +0x46 type of every loaded class of kind 5 (enemy) or 9 (crate).
			let mut class_kinds = HashMap::new();
			// BRIDGE-53: the class header +0x24 scale of every loaded class (pool-effect owner scales).
			let mut class_scales = HashMap::new();
			let mut colliders = HashMap::new();
			// NOV-35b: the classes with metal (env-mapped) packets, drawn by the per-frame metal batch (BRIDGE-45).
			let mut metal_classes = HashMap::new();
			mobies::reset_k_table();
			match mobies::moby_classes(&l).and_then(|c| Ok((gameplay::moby_instances(&l)?, c))) {
				Ok((all, mut classes)) => {
					colliders = classes.iter().filter_map(|(&id, c)| c.collision.clone().map(|k| (id, k))).collect();
					metal_classes = classes.iter().filter(|(_, c)| !c.metal.is_empty()).map(|(&id, c)| (id, c.metal.clone())).collect();
					class_kinds = classes.iter().filter(|(_, c)| c.kind == 5 || c.kind == 9).map(|(&id, c)| (id, c.kind)).collect();
					class_scales = classes.iter().map(|(&id, c)| (id, c.scale * 1024.0)).collect();
					// Retail mode: pools for the classes the game swaps in or spawns after the start state (retail::widen_pools),
					// before the slot order (`slots_game`) and the game (`toggle_play`) read them.
					if crate::retail::active() && !front {
						crate::retail::widen_pools(&all, classes.iter().filter(|(_, c)| !c.mesh.faces.is_empty()).map(|(&id, c)| (id, c.mesh.positions.len() as u32)));
						crate::retail::asset_feed::load(&l);
					}
					hero = classes.remove(&0);
					// The bomb sitting in the glove (class 121 in its first frame, or its bind pose).
					held_bomb = classes.get(&game::BOMB).map(|c| {
						let mats = anim::animation(&l, game::BOMB).ok().flatten().filter(|a| a.sequences.first().is_some_and(|s| !s.frames.is_empty())).map(|a| a.pose_frame(0, 0));
						let (p, n) = match mats {
							Some(m) if c.skin.len() == c.mesh.positions.len() => anim::skin(c, &m),
							_ => (c.mesh.positions.clone(), c.normals.clone()),
						};
						(Mesh { name: c.mesh.name.clone(), positions: c.mesh.positions.clone(), uvs: c.mesh.uvs.clone(), faces: c.mesh.faces.clone(), colours: Vec::new(), lit: None, strip_phase: c.mesh.strip_phase.clone(), normals: c.mesh.normals.clone() }, p, n)
					});
					// ARI-6: a placed class the level's actor table does not build stays in `classes` and draws as scenery.
					let actors = game::LevelActors::of(level_no as i32);
					let ids: Vec<u32> = classes.keys().copied().filter(|&c| actors.is_prop_class(c)).collect();
					for id in ids {
						let class = classes.remove(&id).unwrap();
						match anim::animation(&l, id) {
							Ok(Some(a)) if a.sequences.iter().any(|s| !s.frames.is_empty()) => {
								prop_meshes.insert(id, Mesh { name: class.mesh.name.clone(), positions: class.mesh.positions.clone(), uvs: class.mesh.uvs.clone(), faces: class.mesh.faces.clone(), colours: Vec::new(), lit: None, strip_phase: class.mesh.strip_phase.clone(), normals: class.mesh.normals.clone() });
								prop_anims.insert(id, (class, a));
							}
							// UI-5a / BRIDGE-37 (a): class 12 has no authored sequences; its 16 come from the global pack (TOC +0x198).
							Ok(Some(mut a)) if id == game::vendor::PORTRAIT_CLASS => {
								match anim::portrait_seqs(&mut disc, a.skeleton.parents.len()) {
									Ok(s) => a.sequences = s,
									Err(e) => eprintln!("portrait: {e}"),
								}
								prop_meshes.insert(id, Mesh { name: class.mesh.name.clone(), positions: class.mesh.positions.clone(), uvs: class.mesh.uvs.clone(), faces: class.mesh.faces.clone(), colours: Vec::new(), lit: None, strip_phase: class.mesh.strip_phase.clone(), normals: class.mesh.normals.clone() });
								if a.sequences.iter().any(|s| !s.frames.is_empty()) {
									prop_anims.insert(id, (class, a));
								}
							}
							_ => {
								crate::retail::queue_q2::note_static_tint(id, class.colour_start);
								prop_meshes.insert(id, class.mesh);
							}
						}
					}
					let mut placed = 0;
					// The title's placed mobies are the menu machinery, not scenery: only the scene actors show.
					for (i, m) in all.iter().enumerate().filter(|_| !front) {
						let Some(class) = classes.get(&m.class_id) else { continue }; // props, or helpers without a mesh
						let at = moby_matrix(m);
						let mut mesh = ties::transformed(&class.mesh, &at, format!("moby_{i}"));
						mesh.lit = Some(Lit { kind: LitKind::Moby, sphere: [at[12], at[13], at[14], 0.0], origin: [at[12], at[13], at[14]], cap: 0.0, normals: Vec::new(), raw: Vec::new() });
						meshes.push(mesh);
						placed += 1;
					}
					// Debris classes ride the props batch as transient slots (not placed mobies).
					for id in particles::DEBRIS_CLASSES.into_iter().chain(particles::DEBRIS_CLASSES_EXTRA).chain(std::iter::once(game::TELE_RING_CLASS)).chain(game::FRAGMENT_CLASSES.iter().copied()).chain([game::visibomb::AUX]).chain(game::POD_RING_CLASSES).chain([game::AGENT_PROP_CLASS]) {
						if let Some(c) = classes.get(&id) {
							debris_bspheres.insert(id, c.bsphere);
						}
						if let Some(c) = classes.get(&id).filter(|_| !prop_meshes.contains_key(&id)) {
							prop_meshes.insert(id, Mesh { name: c.mesh.name.clone(), positions: c.mesh.positions.clone(), uvs: c.mesh.uvs.clone(), faces: c.mesh.faces.clone(), colours: Vec::new(), lit: None, strip_phase: c.mesh.strip_phase.clone(), normals: c.mesh.normals.clone() });
						}
					}
					eprintln!("{} moby classes, {} instances, {placed} scenery", classes.len() + prop_meshes.len() + hero.is_some() as usize, all.len());
					instances = if front { Vec::new() } else { all };
					leftover = classes;
				}
				Err(e) => eprintln!("mobies: {e}"),
			}
crate::loadlog::say(|| "host: moby classes done".into());
			let mut hero_anim = hero.and_then(|c| match anim::animation(&l, 0) {
				Ok(Some(a)) => Some((c, a)),
				r => {
					eprintln!("Ratchet animation: {:?}", r.err());
					None
				}
			});
			// The pause-menu hero clips, appended after Ratchet's level sequences (index = menu_clip_base + N).
			let menu_clip_base = hero_anim.as_mut().and_then(|(_, a)| match anim::menu_clips(&mut disc, a.skeleton.parents.len()) {
				Ok(c) => {
					let base = a.sequences.len();
					a.sequences.extend(c);
					Some(base)
				}
				Err(e) => {
					eprintln!("menu clips: {e}");
					None
				}
			});
			// HERO-24 / BRIDGE-58: the weapon hold sets (classes 1 / 2) remapped onto Ratchet's joints.
			if let Some((_, a)) = hero_anim.as_mut() {
				let n = a.skeleton.parents.len();
				a.hold = [1, 2].map(|c| {
					anim::hold_sequences(&l, c, n).unwrap_or_else(|e| {
						eprintln!("hold class {c}: {e}");
						Vec::new()
					})
				});
			}
crate::loadlog::say(|| "host: hero anim + clips done".into());
			let mut hero_items: HashMap<u8, HeroItem> = HashMap::new();
			let mut hero_worn = WornSet::default();
			let flow = match gameplay::cuboids(&l).and_then(|c| Ok((c, gameplay::help_messages(&l)?))).and_then(|r| if front { Err("front end".into()) } else { Ok(r) }) {
				Ok((cuboids, help)) => Some(Flow::new(&instances, cuboids, &help, l.id)),
				Err(e) => {
					eprintln!("level script: {e}");
					None
				}
			};
			// The wrench (gadget 71) in its in-hand pose, posed in the wrist frame.
			let mut gadgets = mobies::gadget_classes(&l).map_err(|e| eprintln!("gadgets: {e}")).unwrap_or_default();
			// Retail mode: the gadget-WAD mobys retail spawns in the moby table (the Swingshot hook 209, gadget shots and
			// pieces) are props like the level's, with a pool each; the hand items keep their held rigs (`Game::mirror_held`).
			// A generic held class (held::rig_classes) is a prop too: retail draws it from its own matrix while it is not the
			// held moby 0x1403e0 (e1_0000 f1198: the Devastator 168 drawn with 0x1403e0 = 0 and item 0 during the switch) [C];
			// `sync` keeps the held moby itself out of the props.
			if crate::retail::active() && !front {
				// The held rigs (Bomb Glove 192, Pyrocitor 176, Suck Cannon 849) are props too: the purchase show-off scene (mode 5,
				// `Retail::sync`) plays the bought gadget's moby with its scene sequence like any prop, and the held moby itself never
				// reaches the props (`sync` leaves it to its rig). Only the wrench stays out: its loose copies are `HeroParts::loose`.
				let rigged: Vec<u32> = vec![mobies::WRENCH_CLASS];
				let mut added = Vec::new();
				let ids: Vec<u32> = gadgets.iter().filter(|(id, c)| !rigged.contains(id) && !c.mesh.faces.is_empty() && !prop_meshes.contains_key(*id)).map(|(&id, _)| id).collect();
				for id in ids {
					let c = &gadgets[&id];
					prop_meshes.insert(id, Mesh { name: c.mesh.name.clone(), positions: c.mesh.positions.clone(), uvs: c.mesh.uvs.clone(), faces: c.mesh.faces.clone(), colours: Vec::new(), lit: None, strip_phase: c.mesh.strip_phase.clone(), normals: c.mesh.normals.clone() });
					if let Ok(Some(a)) = anim::gadget_animation(&l, id) {
						if a.sequences.iter().any(|s| !s.frames.is_empty()) {
							prop_anims.insert(id, (c.clone(), a));
						}
					}
					added.push((id, c.mesh.positions.len() as u32));
				}
				crate::retail::widen_pools(&[], added);
			}
			let wrench = gadgets.remove(&mobies::WRENCH_CLASS).and_then(|class| {
				let a = anim::gadget_animation(&l, mobies::WRENCH_CLASS).ok().flatten()?;
				a.sequences.get(WRENCH_HELD).filter(|s| !s.frames.is_empty())?;
				let held = anim::skin(&class, &a.pose_frame(WRENCH_HELD, 0));
				let jm = a.pose_frame(WRENCH_HELD, 0);
				let at = |j: usize| {
					let m = a.joint_world(&jm, j);
					[m[3], m[7], m[11]]
				};
				let axis = (jm.len() > 1).then(|| [at(0), at(1)]);
				let mesh = Mesh { name: class.mesh.name.clone(), positions: class.mesh.positions.clone(), uvs: class.mesh.uvs.clone(), faces: class.mesh.faces.clone(), colours: class.mesh.colours.clone(), lit: None, strip_phase: class.mesh.strip_phase.clone(), normals: class.mesh.normals.clone() };
				Some((mesh, held, axis, Held { class, anim: a }))
			});
			// The Bomb Glove (gadget 192), posed by its own draw/idle sequences at hero joint 54.
			let glove = gadgets.remove(&GLOVE_CLASS).and_then(|class| {
				let anim = anim::gadget_animation(&l, GLOVE_CLASS).map_err(|e| eprintln!("glove animation: {e}")).ok().flatten()?;
				Some(Held { class, anim })
			});
			// The Pyrocitor (gadget 176) at the hero joint of its attach point (FUN_00253bb0), its muzzle at its own
			// attach point 0, and the nozzle 179 it spawns (FUN_002d08f8).
			let pyro_rig = gadgets.remove(&game::pyro::PYRO_CLASS).and_then(|class| {
				let anim = anim::gadget_animation(&l, game::pyro::PYRO_CLASS).map_err(|e| eprintln!("pyrocitor animation: {e}")).ok().flatten()?;
				// Its row's attach point is 0, the hand point the wrench also uses (weapons_framework.md "Held state").
				let joint = crate::inventory::attach_point(crate::inventory::PYROCITOR).and_then(|p| anim::attach_joint(&l, 0, p as usize))?;
				let muzzle = anim::gadget_attach_joint(&l, game::pyro::PYRO_CLASS, 0);
				let nozzle = leftover.get(&game::pyro::NOZZLE_CLASS).or(gadgets.get(&game::pyro::NOZZLE_CLASS)).map(|c: &mobies::MobyClass| {
					let normals = if c.normals.len() == c.mesh.positions.len() { c.normals.clone() } else { vec![[0.0, 0.0, 1.0]; c.mesh.positions.len()] };
					(Mesh { name: c.mesh.name.clone(), positions: c.mesh.positions.clone(), uvs: c.mesh.uvs.clone(), faces: c.mesh.faces.clone(), colours: Vec::new(), lit: None, strip_phase: c.mesh.strip_phase.clone(), normals: c.mesh.normals.clone() }, c.mesh.positions.clone(), normals)
				});
				if nozzle.is_none() {
					eprintln!("pyrocitor: no nozzle class {}", game::pyro::NOZZLE_CLASS);
				}
				Some(PyroRig { held: Held { class, anim }, joint, muzzle, nozzle })
			});
			// The Suck Cannon (gadget 849) at the hero joint of its attach point, its muzzle at its own attach point 0.
			let suck_rig = gadgets.remove(&849).and_then(|class| {
				let anim = anim::gadget_animation(&l, 849).map_err(|e| eprintln!("suck cannon animation: {e}")).ok().flatten()?;
				let joint = crate::inventory::attach_point(crate::inventory::SUCK_CANNON).and_then(|p| anim::attach_joint(&l, 0, p as usize))?;
				let muzzle = anim::gadget_attach_joint(&l, 849, 0);
				Some(PyroRig { held: Held { class, anim }, joint, muzzle, nozzle: None })
			});
			// BRIDGE-38: the generic held rigs (Blaster 168, Devastator 157, Visibomb 163), like the Suck Cannon at their item's hero attach point.
			let mut held_rigs: HashMap<u32, PyroRig> = HashMap::new();
			for c in game::held::rig_classes() {
				let Some(class) = gadgets.remove(&c).or_else(|| leftover.get(&c).cloned()) else { continue };
				let anim = anim::gadget_animation(&l, c).ok().flatten().or_else(|| anim::animation(&l, c).ok().flatten());
				let item = (1..crate::inventory::GADGETS as u8).find(|&id| crate::inventory::preview_class(id) == Some(c));
				let joint = item.and_then(crate::inventory::view_row).and_then(|r| anim::attach_joint(&l, 0, r.1 as usize));
				let (Some(anim), Some(joint)) = (anim, joint) else {
					eprintln!("held rig {c}: no animation or attach joint");
					continue;
				};
				let muzzle = anim::gadget_attach_joint(&l, c, 0);
				held_rigs.insert(c, PyroRig { held: Held { class, anim }, joint, muzzle, nozzle: None });
			}
			// BRIDGE-76 (ARI-47c): the Swingshot hook (class 209) has no inventory item; it is loaded beside its held rig.
			let hook_rig = held_rigs.contains_key(&game::held::SWINGSHOT_HELD).then_some(()).and_then(|_| {
				let class = gadgets.remove(&game::held::SWINGSHOT_HOOK).or_else(|| leftover.get(&game::held::SWINGSHOT_HOOK).cloned())?;
				let anim = anim::gadget_animation(&l, game::held::SWINGSHOT_HOOK).ok().flatten().or_else(|| anim::animation(&l, game::held::SWINGSHOT_HOOK).ok().flatten());
				let Some(anim) = anim else {
					eprintln!("swingshot hook {}: no animation", game::held::SWINGSHOT_HOOK);
					return None;
				};
				Some(Held { class, anim })
			});
			// BRIDGE-21: the hero's back objects d0 (class 607) and d4 (class 601, Clank) at attach point 5 of the hero.
			let back_rig = leftover.get(&BACK_D0_CLASS).and_then(|c| Some(Held { class: c.clone(), anim: anim::animation(&l, BACK_D0_CLASS).map_err(|e| eprintln!("back object animation: {e}")).ok().flatten()? }));
			let clank_rig = leftover.get(&BACK_D4_CLASS).and_then(|c| Some(Held { class: c.clone(), anim: anim::animation(&l, BACK_D4_CLASS).map_err(|e| eprintln!("clank object animation: {e}")).ok().flatten()? }));
			let light_rig = leftover.get(&CLANK_LIGHT_CLASS).filter(|_| clank_rig.is_some()).cloned();
			let back_joint = anim::attach_joint(&l, 0, BACK_ATTACH_POINT);
			// Pause-menu item previews: the gadget models in bind pose, by class (wrench, glove, the rest).
			let dup = |m: &Mesh| Mesh { name: m.name.clone(), positions: m.positions.clone(), uvs: m.uvs.clone(), faces: m.faces.clone(), colours: m.colours.clone(), lit: None, strip_phase: m.strip_phase.clone(), normals: m.normals.clone() };
			let mut previews: HashMap<u32, Mesh> = gadgets.into_iter().map(|(k, c)| (k, c.mesh)).collect();
			if let Some(r) = &pyro_rig {
				previews.insert(game::pyro::PYRO_CLASS, dup(&r.held.class.mesh));
			}
			if let Some(r) = &suck_rig {
				previews.insert(849, dup(&r.held.class.mesh));
			}
			// `gadgets.remove` above dropped these classes' pause-menu previews: put them back.
			for (&c, r) in &held_rigs {
				previews.insert(c, dup(&r.held.class.mesh));
			}
			if let Some((m, ..)) = &wrench {
				previews.insert(mobies::WRENCH_CLASS, dup(m));
			}
			if let Some(g) = &glove {
				previews.insert(GLOVE_CLASS, dup(&g.class.mesh));
			}
			// The Galactic details page's gold bolt (frontend.rs `gold_preview`).
			if let Some(m) = prop_meshes.get(&GOLD_BOLT_PREVIEW).or(leftover.get(&GOLD_BOLT_PREVIEW).map(|c: &mobies::MobyClass| &c.mesh)) {
				previews.insert(GOLD_BOLT_PREVIEW, dup(m));
			}
			// REN-9: the shop's ammo-row world models (B table classes: glove ammo 226, Pyrocitor ammo 214) and the UI-5d popup bolt (class 13).
			for c in [226, 214, 13] {
				if let Some(m) = prop_meshes.get(&c).or(leftover.get(&c).map(|c: &mobies::MobyClass| &c.mesh)) {
					previews.insert(c, dup(m));
				}
			}
			// Hero preview: built in idle frame 0, re-posed every frame with the item's menu clip (`hero_preview_pose`),
			// plus each item at its attach-point joint [C].
			if let Some((hc, ha)) = hero_anim.as_ref().filter(|(_, a)| a.sequences.first().is_some_and(|s| !s.frames.is_empty())) {
				let mats = ha.pose_frame(IDLE, 0);
				let (ps, _) = anim::skin(hc, &mats);
				let base = Mesh { name: hc.mesh.name.clone(), positions: ps, uvs: hc.mesh.uvs.clone(), faces: hc.mesh.faces.clone(), colours: hc.mesh.colours.clone(), lit: None, strip_phase: hc.mesh.strip_phase.clone(), normals: Vec::new() };
				for id in 1..crate::inventory::GADGETS as u8 {
					// Attach point -> joint through Ratchet's class +0x1c table (FUN_00253bb0) [C].
					// The row is found by class (FUN_00282150), attach point 0 included.
					let Some((class, attach, mode)) = crate::inventory::view_row(id) else { continue };
					let Some(joint) = anim::attach_joint(&l, 0, attach as usize) else { continue };
					let Some(item) = previews.get(&class).filter(|m| !m.positions.is_empty()) else { continue };
					let wj = ha.joint_world(&mats, joint);
					hero_items.insert(id, HeroItem { joint, mesh: dup(item), class, mode });
					let mut m = dup(&base);
					let at = m.positions.len() as u32;
					m.positions.extend(item.positions.iter().map(|&v| apply(&wj, v, 1.0)));
					m.uvs.extend(item.uvs.iter().copied());
					m.faces.extend(item.faces.iter().map(|&(t, f)| (t, f.map(|i| i + at))));
					if !m.colours.is_empty() || !item.colours.is_empty() {
						m.colours.resize(at as usize, [128.0; 4]);
						if item.colours.len() == item.positions.len() {
							m.colours.extend(item.colours.iter().copied());
						} else {
							m.colours.resize(m.positions.len(), [128.0; 4]);
						}
					}
					previews.insert(HERO_PREVIEW + id as u32, m);
				}
				previews.insert(HERO_PREVIEW, base);
				// UI-3b: every candidate worn moby in one batch (boots A / B per feet class, back items, extra items),
				// each at its hero attach point; the handler poses the worn ones and collapses the rest.
				let mut cand: Vec<(WornKind, u32, u8)> = Vec::new();
				for id in [28u8, 29, 30] {
					if let Some(c) = crate::inventory::preview_class(id) {
						cand.push((WornKind::BootA, c, 2));
						cand.push((WornKind::BootB, c, 3));
					}
				}
				for id in [1u8, 2, 3, 4] {
					if let Some(c) = crate::inventory::preview_class(id) {
						cand.push((WornKind::Back, c, 5));
					}
				}
				for (id, point) in [(35u8, 0x1eu8), (33, 0x1d), (34, 0x1d)] {
					if let Some(c) = crate::inventory::preview_class(id) {
						cand.push((WornKind::Extra(id), c, point));
					}
				}
				let mut wm = Mesh { name: "worn".into(), positions: Vec::new(), uvs: Vec::new(), faces: Vec::new(), colours: Vec::new(), lit: None, strip_phase: Vec::new(), normals: Vec::new() };
				for (kind, class, point) in cand {
					let Some(item) = previews.get(&class).filter(|m| !m.positions.is_empty()) else { continue };
					let Some(joint) = anim::attach_joint(&l, 0, point as usize) else { continue };
					let at = wm.positions.len() as u32;
					wm.positions.extend(item.positions.iter().copied());
					wm.uvs.extend(item.uvs.iter().copied());
					wm.faces.extend(item.faces.iter().map(|&(t, f)| (t, f.map(|i| i + at))));
					if !wm.colours.is_empty() || !item.colours.is_empty() {
						wm.colours.resize(at as usize, [128.0; 4]);
						if item.colours.len() == item.positions.len() {
							wm.colours.extend(item.colours.iter().copied());
						} else {
							wm.colours.resize(wm.positions.len(), [128.0; 4]);
						}
					}
					hero_worn.parts.push(WornPart { kind, class, joint, range: at as usize..wm.positions.len(), mesh: dup(item) });
				}
				hero_worn.total = wm.positions.len();
				hero_worn.scale_joints = [0x16, 0x17].map(|p| anim::attach_joint(&l, 0, p));
				if hero_worn.total > 0 {
					previews.insert(HERO_WORN, wm);
				}
			}
crate::loadlog::say(|| "host: previews done".into());
			let (mut art_textures, art) = hud_art(&l, &mut disc);
			// fx textures the world quads use: blob shadow 0 (shadows.md), orb glow 11 and body/glass 21
			// (effects_lighting_mapping.md §4), ship thruster flame 5 (retail/warp.rs `flames`), and the slots of every class-760 fire placement (particles.md; NOV-31:
			// Veldin 41 / 42, Novalis 46 / 47).
			let mut fx_keys = vec![crate::shadows::TEXTURE, 0x28, 0x29, 0x2a, 0x2b, 0x2c, 0x2d, 0x2e, 0x30, 0x35, 0x36, 0x38, 0x39, 0x18, 0x1b, 0x0b, 0x15, 0x13, 0x01, 0x11, 0x12, 0x05];
			for k in crate::retail::queue_b::FX_TEXTURES.into_iter().filter(|k| !fx_keys.contains(k)).collect::<Vec<_>>() { fx_keys.push(k); }
			// Effects layer (retail/efx_draw.rs): any fx slot an owned callback's TEX0 names, so every record of the table.
			for k in 0..frontend::fx_count(&l).min(0x400) {
				if !fx_keys.contains(&k) {
					fx_keys.push(k);
				}
			}
			for k in instances.iter().filter(|m| m.class_id == SMOKE).filter_map(|m| crate::fire::FirePvars::parse(&m.pvars)).flat_map(|p| p.tex) {
				if !fx_keys.contains(&k) {
					fx_keys.push(k);
				}
			}
crate::loadlog::say(|| "host: hud art done".into());
			let fx_textures: Vec<(usize, Texture)> = fx_keys
				.into_iter()
				.filter_map(|k| frontend::fx_texture(&l, k).map_err(|e| eprintln!("fx texture {k}: {e}")).ok().map(|t| (k, t)))
				.collect();
			// Particle textures ride in the fx table under PART_TEX + n; part_defs maps effect type -> n.
			let mut fx_textures = fx_textures;
			let part_defs = l.particle_def_textures().unwrap_or_default();
			let flame_frames: [u8; 10] = l.particle_def_frames(8, 10).ok().and_then(|v| v.try_into().ok()).unwrap_or_default();
			let dust_frames: [u8; 2] = l.particle_def_frames(0x42, 2).ok().and_then(|v| v.try_into().ok()).unwrap_or_default();
			let puff10_frames: [u8; 3] = l.particle_def_frames(0x10, 3).ok().and_then(|v| v.try_into().ok()).unwrap_or_default();
			let orb_frames: [u8; 2] = l.particle_def_frames(0x3e, 2).ok().and_then(|v| v.try_into().ok()).unwrap_or_default();
			// WPN-6.4e2: the type-0x40 water debris texture is its def's frame 1.
			let debris_frame: u8 = l.particle_def_frames(0x40, 2).ok().and_then(|v| v.get(1).copied()).unwrap_or_default();
			// The Pyrocitor flame's defs: segment 0xc (16 frames), big flame 0x18 and ember 0x19 (particles.md).
			let first = |ty: usize| l.particle_def_frames(ty, 1).ok().and_then(|v| v.first().copied()).unwrap_or_default();
			let pyro_frames = game::pyro::Frames {    };
			// Blast mobys: the fireball (class 112, moby texture on its faces) and the shard (122, untextured faces).
			let mut fx_meshes = HashMap::new();
			if !front {
				let moby_tex = l.textures(crate::level::TextureGroup::Moby).unwrap_or_default();
				for id in [particles::FIREBALL_CLASS, particles::SHARD_CLASS, particles::GLOW_CLASS, crate::water::DROP_CLASS, crate::water::RING_CLASS] {
					let Some(c) = leftover.get(&id) else { continue };
					let mut tris = Vec::new();
					for (key, f) in &c.mesh.faces {
						let n = (*key & 0xffff) as usize;
						if let Some(Some(t)) = moby_tex.get(n) {
							if !fx_textures.iter().any(|(k, _)| *k == FX_MOBY_TEX + n) {
								fx_textures.push((FX_MOBY_TEX + n, Texture { w: t.w, h: t.h, rgba: t.rgba.clone() }));
							}
							tris.push((FX_MOBY_TEX + n, *f));
						}
					}
					tris.extend(c.flat.iter().map(|f| (FX_WHITE, *f)));
					fx_meshes.insert(id, FxMesh { pos: c.mesh.positions.clone(), uv: c.mesh.uvs.clone(), tris });
				}
				fx_textures.push((FX_WHITE, Texture { w: 1, h: 1, rgba: vec![[255; 4]] }));
			}
			let part_defs = (part_defs, (flame_frames, pyro_frames, dust_frames, orb_frames, puff10_frames, debris_frame), fx_meshes);
			for (n, t) in l.particle_textures().unwrap_or_default().into_iter().enumerate() {
				if let Some(t) = t {
					fx_textures.push((PART_TEX as usize + n, t));
				}
			}
crate::loadlog::say(|| "host: fx textures done".into());
			let sfx = if front { global_sfx(&mut disc) } else { load_sfx(&l) };
			if front {
				// Logo and the five Press Start captions after the HUD art (slot = 1 + texture index).
				let mut slot = |t: Texture| {
					art_textures.push(t);
					art_textures.len()
				};
				let logo = frontend::logo(&l).map_err(|e| eprintln!("title logo: {e}")).ok().map(&mut slot);
				let press = (0..5).filter_map(|i| frontend::press_start(&l, i).map_err(|e| eprintln!("press start {i}: {e}")).ok().map(&mut slot)).collect();
				*front_art = art.font.clone().map(|font| frontend::Art { font, font2: art.font2.clone(), logo, press, vol: art.vol, frame: art.frame, icons: art.items.clone(), pics: art.pics.clone() });
			}
crate::loadlog::say(|| "host: sfx done".into());
			// The level's scenes (scene.wad chunks: camera, actors, subtitles, speech).
			let mut scenes: HashMap<u32, (scene::Scene, Option<audio::Clip>)> = if front {
				scene::title_scene(&l).map_err(|e| eprintln!("title scene: {e}")).ok().map(|sc| (sc.id, (sc, None))).into_iter().collect()
			} else if let Some(ids) = scene_ids(l.id) {
				ids.filter_map(|id| {
						let sc = scene::veldin_scene(&mut disc, &l, id).map_err(|e| eprintln!("scene {id}: {e}")).ok()?;
						let speech = sc.speech(&mut disc, scene::LANG_ENGLISH).map_err(|e| eprintln!("scene {id} speech: {e}")).ok().flatten();
						let clip = speech.map(|s| audio::Clip { rate: s.sample_rate, pcm: Arc::new(s.pcm), loop_start: None, reverb: false });
						Some((id, (sc, clip)))
					})
					.collect()
			} else {
				HashMap::new()
			};
crate::loadlog::say(|| "host: scenes done".into());
			// NOV-34c: every gameplay level installs the ship bundle (ship 0) after its own classes: textures at moby
			// indices N and N + 1 (N = the level's moby texture count) and the bundle classes the level lacks.
			let mut ship_textures = Vec::new();
			let mut ship_anims: HashMap<u32, anim::Animated> = HashMap::new();
			let mut bundle_ids: Vec<u32> = Vec::new();
			if !front {
				let n0 = l.table(crate::level::TextureGroup::Moby as usize, 16)?.len() as u32;
				// Retail mode: all three ship marks' bundles (the parked ship is the save's mark: class 533 = mark 2 on the
				// recorded Eudora saves, transit.rs `warp_ship_class`), each with its texture pair after the previous one.
				let ships = if crate::retail::active() { 3 } else { 1 };
				for ship in 0..ships {
				let n = n0 + 2 * ship as u32;
				match scene::ship_bundle(&mut disc, ship).and_then(|b| scene::ship_assets_of(&b, n, ship)) {
					Ok(sa) => {
						match mobies::moby_remaps(&l) {
							Ok(registered) => {
								for (id, class, block) in sa.classes {
									if bundle_ids.contains(&id) {
										continue;
									}
									// Retail mode: a bundle class the level registers without a model of its own (the parked
									// ship 533 on Eudora) takes the bundle's model as a prop.
									if registered.contains_key(&id) && crate::retail::active() && !class.mesh.faces.is_empty() && prop_meshes.get(&id).is_none_or(|m: &Mesh| m.faces.is_empty()) {
										if let Ok(Some(an)) = anim::animation_of_class_block(&block, id) {
											ship_anims.insert(id, an);
										}
										eprintln!("retail: ship bundle class {id} as a prop");
										bundle_ids.push(id);
										leftover.insert(id, class);
										continue;
									}
									if registered.contains_key(&id) {
										continue;
									}
									match anim::animation_of_class_block(&block, id) {
										Ok(Some(an)) => {
											ship_anims.insert(id, an);
										}
										r => eprintln!("ship bundle class {id} skeleton: {:?}", r.err()),
									}
									bundle_ids.push(id);
									leftover.insert(id, class);
								}
							}
							Err(e) => eprintln!("ship bundle: level classes: {e}"),
						}
						// Retail mode: the bundle classes (the player ship 533 parked on the pad) are props with a pool each.
						if crate::retail::active() {
							let mut added = Vec::new();
							for &id in &bundle_ids {
								let Some(c) = leftover.get(&id).filter(|c: &&mobies::MobyClass| !c.mesh.faces.is_empty() && prop_meshes.get(&id).is_none_or(|m: &Mesh| m.faces.is_empty())) else { continue };
								prop_meshes.insert(id, Mesh { name: c.mesh.name.clone(), positions: c.mesh.positions.clone(), uvs: c.mesh.uvs.clone(), faces: c.mesh.faces.clone(), colours: Vec::new(), lit: None, strip_phase: c.mesh.strip_phase.clone(), normals: c.mesh.normals.clone() });
								if let Some(an) = ship_anims.get(&id) {
									prop_anims.insert(id, (c.clone(), an.clone()));
								}
								added.push((id, c.mesh.positions.len() as u32));
							}
							crate::retail::widen_pools(&[], added);
						}
						ship_textures.push((n, sa.textures));
					}
					Err(e) => eprintln!("ship bundle {ship}: {e}"),
				}
				}
			}
crate::loadlog::say(|| "host: ship bundle done".into());
			// Ship takeoff / landing streams for a level with a ship pad (viewer/travel.rs; ship type 0, the
			// port's HEADER_SHIP); their actors join the scene slots below.
			if !front && gameplay::ship(&l).ok().flatten().is_some() {
				let (take, land) = travel::stream_ids(l.id, 0, false);
				// NOV-34a: each stream's voice (40000-table cue), preloaded as the scene's clip.
				let (take_voice, land_voice) = travel::voice_ids(l.id, 0, false);
				for (id, idx, cue) in [(travel::TAKEOFF, take, take_voice), (travel::LANDING, land, land_voice)] {
					match scene::ship_stream(&mut disc, idx) {
						Ok(mut sc) => {
							sc.id = id;
							scenes.insert(id, (sc, travel::ship_voice_clip(&mut disc, cue)));
						}
						Err(e) => eprintln!("ship stream {idx}: {e}"),
					}
				}
			}
			// Scene actors are ordinary mobies of the level's classes, posed by the scene's frames with
			// their class skeleton (decomp/notes/cutscenes.md "Scene actors").
			let mut actor_slots = Vec::new();
			for (&id, (sc, _)) in &scenes {
				for a in sc.actors.iter().filter(|a| a.class_id != 0) {
					if !prop_anims.contains_key(&a.class_id) {
						let Some(class) = leftover.remove(&a.class_id) else {
							eprintln!("scene {id} actor {}: class {} has no model", a.index, a.class_id);
							continue;
						};
						// NOV-34c: a bundle class the level lacks takes its skeleton from the bundle core.
						match anim::animation(&l, a.class_id).map(|an| an.or_else(|| ship_anims.remove(&a.class_id))) {
							Ok(Some(an)) => {
								prop_meshes.entry(a.class_id).or_insert_with(|| Mesh { name: class.mesh.name.clone(), positions: class.mesh.positions.clone(), uvs: class.mesh.uvs.clone(), faces: class.mesh.faces.clone(), colours: Vec::new(), lit: None, strip_phase: class.mesh.strip_phase.clone(), normals: class.mesh.normals.clone() });
								prop_anims.insert(a.class_id, (class, an));
							}
							r => {
								eprintln!("scene {id} actor {}: class {} skeleton: {:?}", a.index, a.class_id, r.err());
								continue;
							}
						}
					}
					actor_slots.push((id, a.index, a.class_id));
				}
			}
			actor_slots.sort();
			// Pause hero extra actors (hero record 0x1b93e8 +0x18..+0x2c, spawned by FUN_00283650 at the hero
			// transform with an empty callback; class 0x4a at scale 3): resident classes playing the aux clips.
			let mut menu_actor_classes: HashMap<u32, (mobies::MobyClass, anim::Animated)> = HashMap::new();
			let mut hero_actors: HashMap<u8, Vec<(u32, usize, f32)>> = HashMap::new();
			if let Some(base) = previews.get(&HERO_PREVIEW).map(|m| dup(m)) {
				for &(id, start, count, actors) in MENU_ACTORS.iter() {
					let mut list = Vec::new();
					for &(class, seq) in actors {
						if !menu_actor_classes.contains_key(&class) {
							let Some(c) = leftover.remove(&class).filter(|c| !c.skin.is_empty() && c.skin.len() == c.mesh.positions.len()) else {
								eprintln!("menu actor class {class}: no skinned model");
								continue;
							};
							match anim::animation(&l, class) {
								Ok(Some(a)) => {
									menu_actor_classes.insert(class, (c, a));
								}
								r => {
									eprintln!("menu actor class {class} skeleton: {:?}", r.err());
									continue;
								}
							}
						}
						let Some((_, a)) = menu_actor_classes.get_mut(&class) else { continue };
						for k in (start..start + count).filter(|&k| MENU_AUX.get(k) == Some(&(class, seq))) {
							match anim::menu_aux_clip(&mut disc, k, a.skeleton.parents.len()) {
								Ok(s) => {
									if a.sequences.len() <= seq {
										a.sequences.resize_with(seq + 1, || anim::Sequence { frames: Vec::new(), speed: 0.0, sphere: [0.0; 4], triggers: Vec::new(), loop_sound: 0xff });
									}
									a.sequences[seq] = s;
								}
								Err(e) => eprintln!("{e}"),
							}
						}
						list.push((class, seq, if class == 0x4a { 3.0 } else { 1.0 }));
					}
					if list.is_empty() {
						continue;
					}
					let key = HERO_PREVIEW + id as u32;
					let mut m = previews.remove(&key).unwrap_or_else(|| dup(&base));
					for &(class, _, scale) in &list {
						let add = &menu_actor_classes[&class].0.mesh;
						let at = m.positions.len() as u32;
						m.positions.extend(add.positions.iter().map(|v| v.map(|x| x * scale)));
						m.uvs.extend(add.uvs.iter().copied());
						m.faces.extend(add.faces.iter().map(|&(t, f)| (t, f.map(|i| i + at))));
						if !m.colours.is_empty() {
							m.colours.resize(m.positions.len(), [128.0; 4]);
						}
					}
					previews.insert(key, m);
					hero_actors.insert(id, list);
				}
			}
			// BRIDGE-19: the skinned item classes with their own skeleton and sequences (the aux clips of their hero
			// record installed), next to the extra actors' classes; the item clocks live in the viewer.
			if let Ok(mut all) = mobies::gadget_classes(&l) {
				for id in 1..crate::inventory::GADGETS as u8 {
					let Some(class) = crate::inventory::preview_class(id) else { continue };
					if menu_actor_classes.contains_key(&class) {
						continue;
					}
					let Some(c) = all.remove(&class).filter(|c| !c.skin.is_empty() && c.skin.len() == c.mesh.positions.len()) else { continue };
					if let Ok(Some(a)) = anim::gadget_animation(&l, class) {
						menu_actor_classes.insert(class, (c, a));
					}
				}
			}
			for (id, &(start, count)) in ITEM_AUX.iter().enumerate() {
				let Some(class) = crate::inventory::preview_class(id as u8) else { continue };
				let Some((_, a)) = menu_actor_classes.get_mut(&class) else { continue };
				for k in (start as usize..(start + count) as usize).filter(|&k| MENU_AUX.get(k).is_some_and(|x| x.0 == class)) {
					let seq = MENU_AUX[k].1;
					match anim::menu_aux_clip(&mut disc, k, a.skeleton.parents.len()) {
						Ok(s) => {
							if a.sequences.len() <= seq {
								a.sequences.resize_with(seq + 1, || anim::Sequence { frames: Vec::new(), speed: 0.0, sphere: [0.0; 4], triggers: Vec::new(), loop_sound: 0xff });
							}
							a.sequences[seq] = s;
						}
						Err(e) => eprintln!("{e}"),
					}
				}
			}
			let polygons = gameplay::polygons(&l).map_err(|e| eprintln!("polygons: {e}")).unwrap_or_default();
			let grind_rails = gameplay::grind_rails(&l).map_err(|e| eprintln!("grind rails: {e}")).unwrap_or_default();
			let groups = gameplay::group_lists(&l.gameplay).map_err(|e| eprintln!("groups: {e}")).unwrap_or_default();
			// Clips indexed by slot (absent slots empty), plus the zones and spawn areas the music and
			// reverb controllers read (ambience.md).
			let mut music_clips: Vec<audio::Clip> = (0..15).map(|_| audio::Clip { rate: 44100, pcm: Arc::new(Vec::new()), loop_start: None, reverb: false }).collect();
			for m in music::level_music(&mut disc, &l).map_err(|e| eprintln!("music: {e}")).unwrap_or_default() {
				if let Some(c) = music_clips.get_mut(m.slot) {
					*c = audio::Clip { rate: m.rate, pcm: Arc::new(m.pcm), loop_start: m.loop_start, reverb: false };
				}
			}
			let music = (
				music_clips,
				ambience::zones(&l).map_err(|e| eprintln!("zones: {e}")).unwrap_or_default(),
				ambience::areas(&l).map_err(|e| eprintln!("areas: {e}")).unwrap_or_default(),
			);
			let rig = camera::CameraRig::new(&l).map_err(|e| eprintln!("camera: {e}")).ok();
			let emitters = ambience::emitters(&l).map_err(|e| eprintln!("ambience: {e}")).unwrap_or_default();
			let lighting = gameplay::lighting(&l).map_err(|e| eprintln!("lighting: {e}")).ok();
			let volumes = gameplay::cuboids(&l).map_err(|e| eprintln!("cuboids: {e}")).unwrap_or_default();
			let cuboid_basis = gameplay::cuboid_basis(&l).map_err(|e| eprintln!("cuboid basis: {e}")).unwrap_or_default(); // KER-5
			let smoke: Vec<_> = instances
				.iter()
				.filter(|m| m.class_id == SMOKE)
				.filter_map(|m| {
					let pv = particles::SmokePvars::parse(&m.pvars)?;
					let c = volumes.get(usize::try_from(pv.volume).ok()?)?;
					Some((pv, particles::Cuboid { centre: c.pos, axes: invert3(c.inverse)? }, crate::fire::FirePvars { origin: m.pos, ..crate::fire::FirePvars::parse(&m.pvars)? }))
				})
				.collect();
			eprintln!("{} smoke emitters", smoke.len());
			let ship = gameplay::ship(&l).map_err(|e| eprintln!("ship: {e}")).unwrap_or(None);
			let flight_data = ship.and_then(|_| gameplay::ship_flight(&l).map_err(|e| eprintln!("ship flight: {e}")).ok()).map(|w| gameplay::flight_data(w, &polygons, &volumes));
			let death_height = gameplay::death_height(&l).map_err(|e| eprintln!("death height: {e}")).unwrap_or(f32::MIN);
			let sky = sky::sky(&l).unwrap_or_else(|e| {
				eprintln!("sky: {e}");
				None
			});
			let mut textures = l.all_textures()?;
			// The moby materials read the authored mip chain of their texture at the level the GS picks (gpu.rs `moby_word`).
			if !front {
				match l.mip_chains(crate::level::TextureGroup::Moby) {
					Ok(chains) => super::gpu::register_moby_chains(chains.into_iter().enumerate().filter_map(|(i, c)| Some((crate::level::texture_key(crate::level::TextureGroup::Moby, i as u32), c?))).collect()),
					Err(e) => eprintln!("moby mip chains: {e}"),
				}
			}
			// NOV-33.1: untextured moby faces draw with the retail 8 x 8 grey texel 0x80808080.
			textures.insert(crate::level::texture_key(crate::level::TextureGroup::Moby, crate::mobies::FLAT_SLOT), Texture { w: 8, h: 8, rgba: vec![[128, 128, 128, 255]; 64] });
crate::loadlog::say(|| "host: (rest) before metal textures".into());
			// NOV-35b: the chrome / glass sphere-map textures of moby metal packets.
			match l.metal_textures() {
				Ok([chrome, glass]) => {
					textures.insert(crate::level::CHROME_KEY, chrome);
					textures.insert(crate::level::GLASS_KEY, glass);
				}
				Err(e) => eprintln!("metal textures: {e}"),
			}
			// NOV-34c: the ship bundle textures sit past the level's moby table.
			for (n, [t0, t1]) in ship_textures {
				textures.entry(crate::level::texture_key(crate::level::TextureGroup::Moby, n)).or_insert(t0);
				textures.entry(crate::level::texture_key(crate::level::TextureGroup::Moby, n + 1)).or_insert(t1);
			}
			for (k, t) in crate::gadgets::gadget_textures(&l).map_err(|e| eprintln!("gadget textures: {e}")).unwrap_or_default() {
				textures.entry(k).or_insert(t);
			}
			// Anything still missing gets a steel grey.
			let held_faces = wrench
				.iter()
				.flat_map(|(m, ..)| &m.faces)
				.chain(glove.iter().flat_map(|g| &g.class.mesh.faces))
				.chain(held_bomb.iter().flat_map(|b| &b.0.faces))
				.chain(pyro_rig.iter().flat_map(|r| r.held.class.mesh.faces.iter().chain(r.nozzle.iter().flat_map(|n| &n.0.faces))))
				.chain(suck_rig.iter().flat_map(|r| &r.held.class.mesh.faces))
				.chain(game::held::rig_classes().into_iter().filter_map(|c| held_rigs.get(&c)).flat_map(|r| &r.held.class.mesh.faces))
				.chain(hook_rig.iter().flat_map(|h| &h.class.mesh.faces))
				.chain(back_rig.iter().chain(clank_rig.iter()).flat_map(|h| &h.class.mesh.faces))
				.chain(light_rig.iter().flat_map(|c| &c.mesh.faces));
			for (key, _) in held_faces.chain(previews.values().flat_map(|m| &m.faces)) {
				textures.entry(*key & crate::mobies::KEY_MASK).or_insert_with(|| Texture { w: 1, h: 1, rgba: vec![[150, 152, 160, 255]] });
			}
			// The Galactic Map and the terrain's xy box it is fitted to [P].
			let map_data = if front { None } else { map::load(&mut disc, info.id).map_err(|e| eprintln!("map: {e}")).ok().flatten() }.map(|b| {
				let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
				for p in tfrags.iter().map(|t| &t.near).chain(meshes.iter()).filter(|m| !m.name.contains("_instance_") && !m.name.starts_with("moby_")).flat_map(|m| &m.positions) {
					for k in 0..2 {
						lo[k] = lo[k].min(p[k]);
						hi[k] = hi[k].max(p[k]);
					}
				}
				(b, lo, hi)
			});
			// Novalis cutscene ship gloss tables (NOV-14.5b); None off Novalis.
			let moby_slots = gameplay::moby_slot_count(&l).ok();
			ext.0 = gameplay::class_ranks(&l).unwrap_or_default();
			ext.1 = gameplay::class_rows(&l).unwrap_or_default();
			// ARI-1 / KER-0a: Veldin, Novalis, Aridia and Kerwan keep the overlay image (consumers are gated per level, ARI-6).
			let level_overlay = (l.id <= 3).then(|| l.sections.first().cloned().flatten()).flatten();
			let ship_gloss = (l.id == 1).then(|| l.sections.first().and_then(|o| o.as_deref())).flatten().and_then(crate::shipgloss::ShipGloss::new);
			// Novalis ripple water (water.md): patches from the overlay, class 751's region cuboid ids.
			let water = (l.id == 1).then(|| l.sections.first().and_then(|o| o.as_deref())).flatten().and_then(|ov| {
				let pv = instances.iter().find(|m| m.class_id == crate::water::NOVALIS.class).map_or(&[][..], |m| &m.pvars[..]);
				crate::water::Water::new(ov, crate::water::NOVALIS, pv, lighting.as_ref().and_then(|lt| lt.lights.first()).map(|d| d.dir[0].map(|x| -x)))
			});
			// ARI-49a: the Aridia ocean (decomp/notes/aridia_presentation.md): overlay descriptors, class 854's region cuboid ids.
			// KER-30: the Kerwan ocean (class 994, one ungated descriptor; no pvars, so `pv` is empty there).
			let ocean_def = match l.id {
				2 => Some(&crate::ocean::ARIDIA),
				3 => Some(&crate::ocean::KERWAN),
				_ => None,
			};
			let ocean = ocean_def.and_then(|def| Some((def, l.sections.first()?.as_deref()?))).and_then(|(def, ov)| {
				let pv = instances.iter().find(|m| m.class_id == crate::ocean::CLASS).map_or(&[][..], |m| &m.pvars[..]);
				crate::ocean::Ocean::new(ov, &l, pv, def)
			});
			let occ = crate::occlusion::Occlusion::from_level(&l);
			Ok(((meshes, tie_set, occ, (tfrags, tfrag_b, shrub_set, (bb_tex, terrain_chains))), textures, sky, world, spawn, hero_anim, instances, prop_meshes, prop_anims, (death_height, (polygons, (volumes, cuboid_basis), ship, (groups, grind_rails)), emitters, rig, music, lighting, actor_slots, smoke, map_data), flow, (wrench, (glove, (pyro_rig, ((suck_rig, (held_rigs, hook_rig)), (back_rig, (clank_rig, (back_joint, light_rig)))))), held_bomb, (previews, (hero_items, hero_actors, menu_actor_classes, hero_worn), menu_clip_base, (debris_bspheres, class_kinds, class_scales), (colliders, metal_classes))), (art_textures, fx_textures, part_defs), art, sfx, scenes, ((water, ocean), ship_gloss, moby_slots, (level_overlay, flight_data))))
		} };
		if prefetch.is_some() {
			// Level load runs `job` itself; here its thread. `clean`: the thread's rand() was not drawn from (it is the thread's own copy).
			// Then the thread builds the level's scene on the GPU (the same `build_scene` as `load_scene`) up to the prop slots, whose order
			// only the switch knows (retail's pools): it sends `Early` and waits for the order.
			let ctx = self.gpu.as_ref().map(|g| gpu::SceneCtx { paced: true, ..g.ctx() });
			let (tx_early, rx_early) = std::sync::mpsc::channel::<Early>();
			let (tx_order, rx_order) = std::sync::mpsc::channel::<(Vec<u32>, bool)>();
			let done = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
			let done_t = DoneGuard(done.clone());
			let h = std::thread::Builder::new().name("level-prefetch".into()).stack_size(64 << 20).spawn(move || {
				crate::retail::defer_begin();
				let (mut fa, mut ext) = (None, LevelExt::default());
				let mut r = job(&mut fa, &mut ext);
				let clean = particles::Rng::shared().state() == particles::RAND_SEED;
				let mut effects = crate::retail::defer_take();
				let widen = crate::retail::defer_take_widen();
				let mut scene = None;
				// The collision copy the sound occlusion queries (`set_world` at the switch), made here too.
				let mut world_copy = None;
				if let (true, Some(ctx), Ok(res)) = (clean, ctx.as_ref(), &mut r) {
					let ((meshes, tie_set, occ, (tfrags, tfrag_b, shrub_set, (_bb_tex, terrain_chains))), textures, sky, w_t, _sp, hero_anim, instances, prop_meshes, _pa, (_dh, _p, _e, _r, _m, _l, actor_slots, _sm, _md), _f, (wrench, (glove, (pyro_rig, ((suck_rig, (held_rigs, hook_rig)), (back_rig, (clank_rig, (_bj, light_rig)))))), held_bomb, (previews, _h2, _mcb, _d, _c)), ..) = res;
					world_copy = Some(w_t.clone());
					let early = Early { instances: instances.clone(), actors: actor_slots.iter().map(|s| s.2).collect(), effects: std::mem::take(&mut effects), widen };
					if tx_early.send(early).is_ok() {
						let combined = combined_hero!(hero_anim, wrench, glove, held_bomb, pyro_rig, suck_rig, held_rigs, back_rig, clank_rig, hook_rig, light_rig);
						let tint = hero_anim.as_ref().and_then(|h| mobies::glow_rgb(h.0.glow).map(|c| (c, h.0.colour_start, h.0.mesh.positions.len())));
						let tfr = std::mem::take(tfrags);
						let mut cur: Option<(Vec<u32>, bool)> = None;
						scene = ctx.build_scene(meshes, shrub_set, tie_set, textures, sky.as_ref(), combined.as_ref(), tint, |built: Option<&[u32]>| {
							// The order to build (None: first) or, after a build, the one to keep: the newest order received; a build of
							// a speculative order waits until the final one says whether it stays.
							while let Ok(m) = rx_order.try_recv() {
								cur = Some(m);
							}
							loop {
								match (&cur, built) {
									(Some((o, _)), None) => return Some(o.clone()),
									(Some((o, fin)), Some(b)) if o.as_slice() != b || *fin => return Some(o.clone()),
									_ => {}
								}
								cur = Some(rx_order.recv().ok()?);
								while let Ok(m) = rx_order.try_recv() {
									cur = Some(m);
								}
							}
						}, prop_meshes, previews, occ.clone(), tfr, *tfrag_b, &terrain_chains.0, &terrain_chains.1, &terrain_chains.2);
						if scene.is_none() {
							eprintln!("level-prefetch: scene build abandoned");
						}
					}
				}
				drop(done_t);
				(r, ext, effects, clean, scene, world_copy)
			});
			if let Ok(h) = h {
				self.prefetch = Some((level_no, Box::new(h)));
				self.prefetch_link = Some((level_no, PrefetchLink { early: rx_early, order: tx_order, got: None, sent: None, spec: None, calm: None, tries: 0, done, since: std::time::Instant::now() }));
			}
			return;
		}
		// The parse of this level, started while the game was still travelling (`prefetch_level`), when there is one.
		let pre = if prefetch.is_none() { self.prefetch.take().filter(|(n, _)| *n == level_no).map(|p| p.1) } else { None };
		let link = if prefetch.is_none() { self.prefetch_link.take().filter(|(n, _)| *n == level_no).map(|p| p.1) } else { None };
		let t_parse = std::time::Instant::now();
		// The thread's parse is done: its queued global writes run now (as the inline parse ran them), and the prop slot order (from the
		// pools they leave) releases its scene build.
		let mut early_ran = false;
		let mut prebuilt_ok = true;
		if let (Some(mut link), true) = (link, pre.is_some()) {
			if let Some(e) = link.got.take().or_else(|| link.early.recv().ok()) {
				for f in e.effects {
					f();
				}
				early_ran = true;
				let g = slots_game(&e.instances, self.level as i32);
				let mut order: Vec<u32> = g.props.iter().map(|p| p.class_id).collect();
				order.extend(e.actors);
				match link.sent.take() {
					Some(was) => {
						if was != order {
							eprintln!("level-prefetch: slot order differs from the early one; scene rebuilt");
							prebuilt_ok = false;
						}
					}
					None => {
						let _ = link.order.send((order, true));
					}
				}
			}
		}
		let done = pre.and_then(|b| downcast_handle(b, handle_of(&job))).and_then(|h| h.join().ok()).filter(|r| r.3);
		let prefetched = done.is_some();
		let (mut result, ext, effects, mut prebuilt, mut world_copy) = match done {
			Some((r, ext, eff, _, sc, wc)) => (r, ext, if early_ran { Vec::new() } else { eff }, sc, wc),
			None => {
				let mut ext = LevelExt::default();
				let r = job(&mut front_art, &mut ext);
				(r, ext, Vec::new(), None, None)
			}
		};
		if !prebuilt_ok {
			// The thread built for another order: its scene is dropped, the terrain fragments it owns go back to the parse result.
			if let (Some(sc), Ok(res)) = (prebuilt.take(), &mut result) {
				res.0.3.0 = sc.tfrag_set;
			}
		}
crate::loadlog::say(|| format!("host: parse {} ({:.0} ms waited)", if prefetched { "prefetched" } else { "inline" }, t_parse.elapsed().as_secs_f64() * 1e3));
		for e in effects {
			e();
		}
		if result.is_ok() {
			(self.class_ranks, self.class_rows) = (ext.0, ext.1);
		}
crate::loadlog::say(|| "host: closure done (all host parsing)".into());
		let gpu = self.gpu.as_mut().unwrap();
		match result {
			Ok(((meshes, tie_set, occ, (tfrags, tfrag_b, shrub_set, (bb_tex, terrain_chains))), textures, sky, world, spawn, hero_anim, instances, prop_meshes, prop_anims, (death_height, (polygons, (volumes, cuboid_basis), ship, (groups, grind_rails)), emitters, rig, music, lighting, actor_slots, smoke, map_data), flow, (wrench, (glove, (pyro_rig, ((suck_rig, (held_rigs, hook_rig)), (back_rig, (clank_rig, (back_joint, light_rig)))))), held_bomb, (previews, (hero_items, hero_actors, menu_actor_classes, hero_worn), menu_clip_base, (debris_bspheres, class_kinds, class_scales), (colliders, metal_classes))), (art_textures, fx_textures, part_defs), art, sfx, scenes, ((water, ocean), ship_gloss, moby_slots, (level_overlay, flight_data)))) => {
				// Ratchet with the wrench, the glove, the glove's bomb, the Pyrocitor and its nozzle appended (in that
				// order); their vertices follow his joints each frame.
				let combined = if prebuilt.is_some() { None } else { combined_hero!(hero_anim, wrench, glove, held_bomb, pyro_rig, suck_rig, held_rigs, back_rig, clank_rig, hook_rig, light_rig) };
				let hero = combined.as_ref();
				// The prop slots must match `Game::new`'s order: build one to read it.
				if crate::retail::active() && !front {
					crate::retail::set_modelled(prop_meshes.iter().filter(|(_, m)| !m.faces.is_empty()).map(|(&c, m)| (c, m.positions.len() as u32)).collect());
				}
				// [PC] Retail pool growth rebuilds the prop slots from these (`regrow_props`), not the level.
				self.retail_prop_meshes = if crate::retail::active() && !front { prop_meshes.clone() } else { HashMap::new() };
				if crate::retail::active() && !front {
					let p = crate::retail::pools();
					eprintln!("retail: prop pools {} classes, {} props", p.len(), p.iter().map(|c| c.1).sum::<u32>());
				}
								let slots_game = slots_game(&instances, self.level as i32);
				let mut order: Vec<u32> = slots_game.props.iter().map(|p| p.class_id).collect();
				order.extend(actor_slots.iter().map(|s| s.2));
				let debris_classes: Vec<u32> = particles::DEBRIS_CLASSES.into_iter().chain(particles::DEBRIS_CLASSES_EXTRA).chain(std::iter::once(game::TELE_RING_CLASS)).filter(|c| prop_meshes.contains_key(c)).collect();
crate::loadlog::say(|| "host: pre gpu.load_scene".into());
				match prebuilt.take() {
					Some(sc) => drop_later(gpu.scene.replace(sc)),
					None => gpu.load_scene(&meshes, &shrub_set, &tie_set, &textures, sky.as_ref(), hero, hero_anim.as_ref().and_then(|h| mobies::glow_rgb(h.0.glow).map(|c| (c, h.0.colour_start, h.0.mesh.positions.len()))), &order, &prop_meshes, &previews, occ, tfrags, tfrag_b, &terrain_chains.0, &terrain_chains.1, &terrain_chains.2),
				}
crate::loadlog::say(|| "host: load_scene returned".into());
				gpu.set_shrub_bb_textures(&bb_tex);
				// [PC] PREVIEW (vegfx.rs, RAC_VEG): where the extra plants go.
				gpu.plan_veg(&world, &instances, &shrub_set, &textures, &terrain_chains.0, &terrain_chains.1, &prop_meshes, &tie_set, &terrain_chains.2, self.level);
crate::loadlog::say(|| "host: bb textures".into());
				gpu.load_debris(&debris_classes, &prop_meshes, &textures);
crate::loadlog::say(|| "host: debris".into());
				gpu.load_metal(metal_classes, &textures);
crate::loadlog::say(|| "host: metal".into());
				gpu.lighting = lighting;
crate::loadlog::say(|| "host: t lighting".into());
				// [PC] The old level's data these fields held is freed off the main thread (`drop_later`).
				let mut old: Vec<Box<dyn std::any::Any + Send>> = Vec::new();
				// NOV-28: the live fog restarts from the level fog (retail level load OV:45719-45725).
				self.fog_zone = gpu.lighting.as_ref().map_or(([0; 3], [0.0, 1.0e9, 255.0, 255.0]), |lt| (lt.fog_colour, lt.fog));
				old.push(Box::new(std::mem::replace(&mut self.fog_next, None)));
				gpu.fog_live = None;
				self.part_far_next = crate::water::PART_FAR;
				gpu.part_far = crate::water::PART_FAR;
				gpu.under_flag = false;
				gpu.under_surf = None;
				gpu.hero_placed = instances.iter().find(|m| m.class_id == 0).map_or((gameplay::SPAWN_AMBIENT, 0), |m| (m.col, m.light));
				gpu.hero_env = gameplay::placed_env(gpu.hero_placed.0, gpu.hero_placed.1);
				old.push(Box::new(std::mem::replace(&mut self.hero_anim, hero_anim)));
				if let (Some(g), Some((_, a))) = (&mut self.game, &self.hero_anim) {
					g.set_hero_sequences(&a.sequences);
				}
				old.push(Box::new(std::mem::replace(&mut self.hero_items, hero_items)));
				old.push(Box::new(std::mem::replace(&mut self.hero_worn, hero_worn)));
				self.worn_back = None;
				old.push(Box::new(std::mem::replace(&mut self.hero_actors, hero_actors)));
				old.push(Box::new(std::mem::replace(&mut self.menu_actor_classes, menu_actor_classes)));
				old.push(Box::new(std::mem::replace(&mut self.menu_clip_base, menu_clip_base)));
				self.hero_preview_clip = None;
				old.push(Box::new(std::mem::replace(&mut self.prop_anims, prop_anims)));
				old.push(Box::new(std::mem::replace(&mut self.actor_slots, actor_slots)));
				self.debris.clear();
				old.push(Box::new(std::mem::replace(&mut self.debris_bspheres, debris_bspheres)));
				old.push(Box::new(std::mem::replace(&mut self.class_kinds, class_kinds)));
				old.push(Box::new(std::mem::replace(&mut self.class_scales, class_scales)));
				old.push(Box::new(std::mem::replace(&mut self.colliders, colliders)));
				old.push(Box::new(std::mem::replace(&mut self.water, water)));
				old.push(Box::new(std::mem::replace(&mut self.ocean, ocean)));
				old.push(Box::new(std::mem::replace(&mut self.ship_gloss, ship_gloss)));
				old.push(Box::new(std::mem::replace(&mut self.level_overlay, level_overlay)));
				self.gloss_quads.clear();
				self.fires = smoke.iter().map(|&(_, vol, fp)| crate::fire::Fire::new(fp, vol)).collect();
				self.smoke = smoke.into_iter().map(|(pv, vol, _)| (pv, vol)).collect();
				self.particles = particles::Particles::new();
				if let Some(d) = moby_slots {
					self.particles.set_moby_limit(d.saturating_sub(1));
				}
				if instances.iter().any(|m| m.class_id == particles::SCROLL_CLASS) {
					self.particles.place_scroll();
				}
				self.bounds = prop_meshes.iter().map(|(&id, m)| (id, bounds(m))).collect();
				self.particles.set_wanders(instances.iter().filter(|m| m.class_id == particles::WANDER_CLASS).filter_map(|m| particles::Wander::new(m.pos, &m.pvars)).collect());
				old.push(Box::new(std::mem::replace(&mut self.instances, instances)));
				old.push(Box::new(std::mem::replace(&mut self.polygons, polygons)));
				old.push(Box::new(std::mem::replace(&mut self.groups, groups)));
				old.push(Box::new(std::mem::replace(&mut self.grind_rails, grind_rails)));
				old.push(Box::new(std::mem::replace(&mut self.cuboids, volumes)));
				old.push(Box::new(std::mem::replace(&mut self.cuboid_basis, cuboid_basis)));
				old.push(Box::new(std::mem::replace(&mut self.emitters, emitters)));
				old.push(Box::new(std::mem::replace(&mut self.rig, rig)));
				(self.music, self.zones, self.areas) = music;
				self.music_player = None;
				old.push(Box::new(std::mem::replace(&mut self.death_height, death_height)));
				old.push(Box::new(std::mem::replace(&mut self.ship, ship)));
				old.push(Box::new(std::mem::replace(&mut self.flight_data, flight_data)));
				self.flow_start = flow;
				if let Some(f) = &mut self.flow_start {
					f.movies = true;
					f.set_durations(scenes.iter().map(|(&id, (s, _))| (id, s.duration)).collect());
				}
				old.push(Box::new(std::mem::replace(&mut self.scenes, scenes)));
				self.wrench_axis = wrench.as_ref().and_then(|w| w.2);
				self.wrench = wrench.as_ref().map(|w| w.1.clone());
				self.wrench_rig = wrench.map(|w| w.3);
				self.hero_rig.reset_wrench();
				old.push(Box::new(std::mem::replace(&mut self.glove, glove)));
				self.bomb = held_bomb.map(|(_, p, n)| (p, n));
				if let (Some(g), Some(r)) = (&mut self.game, &pyro_rig) {
					g.set_pyro_timing(anim::share_timing(&r.held.anim.sequences));
				}
				if let (Some(g), Some(r)) = (&mut self.game, &suck_rig) {
					g.set_suck_timing(anim::share_timing(&r.held.anim.sequences));
				}
				if let Some(g) = &mut self.game {
					for (&c, r) in &held_rigs {
						g.set_held_timing(c, anim::share_timing(&r.held.anim.sequences));
					}
					if let Some(h) = &hook_rig {
						g.set_held_timing(game::held::SWINGSHOT_HOOK, anim::share_timing(&h.anim.sequences));
					}
				}
				old.push(Box::new(std::mem::replace(&mut self.held_rigs, held_rigs)));
				old.push(Box::new(std::mem::replace(&mut self.hook_rig, hook_rig)));
				old.push(Box::new(std::mem::replace(&mut self.held_muzzle, None)));
				old.push(Box::new(std::mem::replace(&mut self.held_xform, None)));
				old.push(Box::new(std::mem::replace(&mut self.held_joint0, None)));
				old.push(Box::new(std::mem::replace(&mut self.pyro_rig, pyro_rig)));
				old.push(Box::new(std::mem::replace(&mut self.suck_rig, suck_rig)));
				old.push(Box::new(std::mem::replace(&mut self.back_rig, back_rig)));
				old.push(Box::new(std::mem::replace(&mut self.clank_rig, clank_rig)));
				old.push(Box::new(std::mem::replace(&mut self.back_joint, back_joint)));
				old.push(Box::new(std::mem::replace(&mut self.light_rig, light_rig)));
				old.push(Box::new(std::mem::replace(&mut self.pyro_muzzle, None)));
				self.glint_u = fx_textures.iter().find(|(k, _)| *k == 0x13).map_or(1.0, |(_, t)| 32.0 / t.w as f32);
				drop_later(old);
crate::loadlog::say(|| "host: t fields assigned".into());
crate::loadlog::say(|| "host: pre set_hud_art/fx upload".into());
				gpu.set_hud_art(&art_textures);
crate::loadlog::say(|| "host: t set_hud_art".into());
				gpu.fx_tex = fx_textures.iter().map(|(k, t)| (*k, gpu.upload_texture(Some(t)))).collect();
				gpu.partfx.set_tints(&fx_textures);
				// GS CLAMP_1 = 5 variants (`particles::FX_CLAMP`): the hero swing trail's textures; no mips (TEX1 MXL 0).
				for (k, t) in fx_textures.iter().filter(|(k, _)| crate::retail::queue_b::FX_CLAMPED.contains(k) || crate::retail::queue_q2::FX_CLAMPED.contains(k) || *k < crate::particles::FX_TEX as usize) {
					let b = gpu.upload(Some(t), &gpu.clamp_sampler);
					gpu.fx_tex.insert(k | crate::particles::FX_CLAMP, b);
				}
				// GS CLAMP_1 0 + TEX1_1 MXL 0 (`particles::FX_NOMIP`): the glass textures repeat, filter linearly and have no mips.
				let nomip = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
					address_mode_u: wgpu::AddressMode::Repeat,
					address_mode_v: wgpu::AddressMode::Repeat,
					mag_filter: wgpu::FilterMode::Linear,
					min_filter: wgpu::FilterMode::Linear,
					..Default::default()
				});
				for (k, t) in fx_textures.iter().filter(|(k, _)| crate::retail::queue_q1::GLASS_TEX.contains(k)) {
					let b = gpu.upload(Some(t), &nomip);
					gpu.fx_tex.insert(k | crate::particles::FX_NOMIP, b);
				}
				let (part_defs, (flame_frames, pyro_frames, dust_frames, orb_frames, puff10_frames, debris_frame), fx_meshes) = part_defs;
				gpu.part_defs = part_defs;
				gpu.fx_meshes = fx_meshes;
				self.particles.flame_frames = flame_frames;
				self.particles.dust_frames = dust_frames;
				self.particles.puff10_frames = puff10_frames;
				self.particles.debris_frame = debris_frame;
				self.particles.orb_frames = orb_frames;
				self.pyro_frames = pyro_frames;
				if let Some(g) = &mut self.game {
					g.set_pyro_frames(pyro_frames);
				}
crate::loadlog::say(|| "host: fx upload done".into());
				self.ui_art = art.font.clone().map(|font| frontend::Art { font, font2: art.font2.clone(), logo: None, press: Vec::new(), vol: art.vol, frame: art.frame, icons: art.items.clone(), pics: art.pics.clone() });
				self.map = map_data.and_then(|(bundle, lo, hi)| {
					// B3002 of the active save (zero = nothing explored yet).
					let spans = bundle.spans.clone();
					let explored = map::Explored(self.save.as_ref().map_or_else(|| vec![0; 0x8000], |(_, sv)| sv.explored(self.level, &spans)));
					let slot = gpu.hud_texture(None, &map::composite(&bundle, &explored));
					Some(MapView { bundle, explored, spans, slot, lo, hi, hero: [0.0; 2], yaw: 0.0, affine: map::retail_affine(self.level), zoom: MAP_ZOOM.0, pan: [256.0; 2] })
				});
crate::loadlog::say(|| "host: t map".into());
				self.pause = None;
				if let Some(t) = self.pause_saved_tan_half.take() {
					self.cam.tan_half = t; // SCN-6 restore (close_pause inline: gpu is borrowed)
				}
				self.popup = None;
				self.wheel = None;
				drop_later((std::mem::replace(&mut self.art, art), std::mem::replace(&mut self.sfx, sfx)));
				if let Some(a) = &mut self.audio {
					a.stop_all();
				}
crate::loadlog::say(|| "host: t art/sfx/stop_all".into());
				self.emitter_state = vec![(None, 0.0); self.emitters.len()];
				self.volume_voices.clear();
				self.underwater_voices.clear();
				self.enemy_sfx.clear();
				self.music_voice = None;
				self.sting_voice = None;
				self.music_player = None;
				self.flow = None;
				// The occlusion lines and the camera water probe query a copy of the level collision.
				drop_later(self.listener.scene.replace_world(world_copy.take().unwrap_or_else(|| world.clone())));
				self.listener.reset_occlusion();
crate::loadlog::say(|| "host: t listener world".into());
				drop_later(self.world.replace(world));
				self.spawn = spawn;
				self.game = None;
crate::loadlog::say(|| "host: t world/game dropped".into());
				let s = gpu.scene.as_ref().unwrap();
				self.cam = match self.start.take() {
					Some([x, y, z, yaw, pitch]) => Camera { pos: [x, y, z], yaw: yaw.to_radians(), pitch: pitch.to_radians(), tan_half: TAN_HALF_FOV, up: [0.0, 0.0, 1.0] },
					None => Camera { pos: [s.centre[0] - s.radius * 0.6, s.centre[1] - s.radius * 0.6, s.centre[2] + s.radius * 0.5], yaw: 45f32.to_radians(), pitch: -30f32.to_radians(), tan_half: TAN_HALF_FOV, up: [0.0, 0.0, 1.0] },
				};
				gpu.window.set_title(&format!("{} - {} ({})", crate::app_name(), toc::LEVEL_NAMES[self.level], self.level));
crate::loadlog::say(|| "host: t title".into());
				if !front {
					let lang = self.settings.lang as usize;
					self.ui_text = iso::Disc::open(std::path::Path::new(&self.iso_path)).map_err(|e| e.to_string()).and_then(|mut d| frontend::Text::load(&mut d, TEXT_SLOT[lang.min(4)])).map_err(|e| eprintln!("text: {e}")).ok();
				}
				let lang = self.front.take().map_or(self.settings.lang as usize, |f| f.menu.lang);
				// The front end's capture picture and held movie frame end with it (viewer/front_mirror.rs).
				if self.fm_ov || self.fm_movie_hold {
					gpu.movie.clear();
				}
				(self.fm_ov, self.fm_movie_hold) = (false, false);
crate::loadlog::say(|| "host: t ui_text".into());
				// [PC] The parsed level data the scene and game were built from is freed off the main thread (40+ ms at a switch).
				// [PC] The plant planner keeps the data a Vegetation replan reads (vegfx.rs `keep_veg_src`); the rest is freed.
				let veg_drop = gpu.keep_veg_src(shrub_set, textures, terrain_chains, tie_set, self.instances.len(), self.level);
				drop_later((meshes, prop_meshes, previews, bb_tex, fx_textures, art_textures, sky, veg_drop));
				if front {
					gpu.window.set_title(crate::app_name());
					self.title = crate::app_name().into();
					let text = iso::Disc::open(std::path::Path::new(&self.iso_path))
						.map_err(|e| e.to_string())
						.and_then(|mut d| frontend::Text::load(&mut d, TEXT_SLOT[lang]))
						.unwrap_or_else(|e| {
							eprintln!("front-end text: {e}");
							frontend::Text::default()
						});
					let script = std::env::var("RAC_MENU_KEYS").unwrap_or_default().chars().filter_map(|c| {
						use frontend::Key::*;
						Some(match c { 'u' => Up, 'd' => Down, 'l' => Left, 'r' => Right, 'c' => Confirm, 'b' => Back, 'o' => Circle, _ => return None })
					}).collect();
					let mut menu = frontend::Menu::new(lang);
					menu.settings = self.settings;
					self.front = Some(FrontEnd { menu, text, art: front_art, tick: 0.0, bed: Vec::new(), timed: [None; 2], script, script_t: 200, att: frontend::Attract::new(crate::particles::Rng::shared().next() % 4), acc: 0.0, attract_movie: None });
					// Retail: the game's own 989snd commands play the title's sounds (retail/snd.rs), not this bed.
					if let (Some(a), false) = (&mut self.audio, crate::retail::active()) {
						let bed = TITLE_BED.iter().filter_map(|&i| self.sfx.clips.get(i)?.as_ref().filter(|c| c.loop_start.is_some()).map(|c| a.play(c, 0.5, 0.0))).collect();
						self.front.as_mut().unwrap().bed = bed;
					}
				}
			}
			Err(e) => eprintln!("level {}: {e}", self.level),
		}
		crate::loadlog::say(|| "host: load_level end".into());
	}
}

#[cfg(test)]
mod tests {
	use super::*;

	// NOV-24: the viewer's slot order has the same classes as the running Novalis game, so the vendor orb has a mesh slot.
}
