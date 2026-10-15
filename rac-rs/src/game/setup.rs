use super::*;

impl Game {

	/// `new` with the story (mission bytes B3004, 16 per level) in place before the actors are built, so
	/// phase bits and rewards read the level's own slots (overlay.elf.c:46155-46195).
	pub fn with_story(instances: &[MobyInstance], bounds: &HashMap<u32, (V3, V3)>, spawn: (V3, f32), death_height: f32, mut story: crate::save::Story, level: i32) -> Game {
		let mission: [u8; 16] = usize::try_from(level).ok().and_then(|l| story.mission.get(l)).copied().unwrap_or([0; 16]);
		let crate_nibbles: [u8; 16] = usize::try_from(level).ok().and_then(|l| story.crate_nibbles.get(l)).copied().unwrap_or([0; 16]);
		// A fresh level entry (FUN_0029a4f0): every table clear but the saved level bitmap.
		let uids = uidstate::UidState::new(usize::try_from(level).ok().and_then(|l| story.uids.get(l)).copied().unwrap_or([0; 256]), mission);
		let mut props = Vec::new();
		// ARI-6: the level's actor table picks which placements become live props; the rest stay static models.
		let actors = LevelActors::of(level);
		// Retail mode (retail.rs): one pool of inert props per class instead of the placed actors.
		if actors == LevelActors::Retail {
			for (class, n) in crate::retail::pools() {
				props.extend((0..n).map(|_| Prop::retail(class, level)));
			}
		}
		// The old per-planet actor loop and its spawned pools are archived: retail mode is the only mode.
		// COL-14: collision-only props for the null-update baked classes the loader files in the moby grid for life
		// (FUN_00241528 -> FUN_00250a08); after every pool so no earlier prop index moves. They draw nothing (the class
		// stays in the baked scenery) and Barrier has no update.
		for m in instances.iter().filter(|m| levels::static_solids(level).contains(&m.class_id) && !actors.is_dynamic(m.class_id)) {
			props.push(Prop { class_id: m.class_id, pos: m.pos, rot: m.rot, shown: m.in_phase(&mission), anim: crate::anim::PropAnim::new(0), scale: m.scale, tint: None, light: Some((m.col, m.light)), index: Some(m.index), sched: sched::Sched { dist: m.place.dist, over: 0,  rank: m.place.class_rank,  flags: sched::placed_flags(m.class_id),  draw: m.place.draw, occ: m.occ },  kind: Kind::Scenery });
		}
		let mut player = Player::new(add(spawn.0, [0.0, 0.0, 0.05]), spawn.1);
		player.slope_push_on = level == 0; // HERO-2: the downhill push exists on Veldin only
		// Level-start draws (RNG-4c-D3): init's blink countdown, then SetState(0)'s overwrites it.
		{
			let mut rng = crate::particles::Rng::shared();
			player.blink_count = rng.int(crate::gameplay::pal_ticks(180), crate::gameplay::pal_ticks(300));
			player.blink_count = rng.int(crate::gameplay::pal_ticks(50), crate::gameplay::pal_ticks(100));
		}
		// Every level load (0..18, so all of them) gives the glove if it is not owned (FUN_0023cd80).
		// The save's inventory (A9/A10/A13/A32) is in place before the actors exist; the last-chosen weapon
		// is the one in hand unless A22 says the wrench was the last pick (passive init FUN_0022ee88, savegame.md).
		let mut inv = story.inv.clone();
		inv.equip_on_load(story.fallback[1] != 0);
		inv.ensure_glove(0);
		player.item_applied = inv.equipped;
		let uid_map = crate::save::uid_map(instances);
		let saved = usize::try_from(level).ok().and_then(|l| story.uids.get(l)).map_or_else(HashSet::new, |b| crate::save::uid_saved(b, &uid_map));
		let place_flags = instances.iter().map(|m| (m.index, m.flags)).collect();
		// The loader's +0xb1 for every placement, in loader order (FUN_0029a480 runs before the collected test).
		let place_slot: HashMap<u32, i8> = {
			let table = usize::try_from(level).ok().and_then(|l| story.remembered.get_mut(l));
			let mut table = table;
			instances.iter().map(|m| {
				let slot = if m.flags == 0 { -2 } else if m.flags & 0x10 == 0 { -1 } else if let Some(t) = table.as_deref_mut() { crate::save::find_or_claim(t, m.uid as i32) } else { -1 };
				(m.index, slot)
			}).collect()
		};
		let (gold_slots, skill_points) = (story.gold, story.skill);
		let mut g = Game {
			glints: [Glint::default();16],
			debris_draws: Vec::new(),
			strips: Vec::new(),
			env_passes: Vec::new(),
			strip_groups: Default::default(),
			strip_out: Vec::new(),
			jet_joint: HashMap::new(),
			held_hidden: false,
			seq_spheres: HashMap::new(),
			placed: props.clone(),
			frames: 1,
			props,
			player,
			bolts: 0,
			health: MAX_HEALTH,
			class_spheres: HashMap::new(),
			class_kinds: HashMap::new(),
			class_scales: HashMap::new(),
			invulnerable: 0,
			dead: None,
			moves_done: 0,
			log: Vec::new(),
			inv_start: inv.clone(),
			popups: Vec::new(),
			inv,
			lock: None,
			bomb_hold: 0,
			ship_mesh: None,
			camera: None,
			events: Vec::new(),
			sound_requests: Vec::new(),
			spawned: Vec::new(),
			class_ranks: HashMap::new(),
			class_rows: HashMap::new(),
			tube_draw: Vec::new(),
			saved,
			uid_map,
			uids,
			place_flags,
			ckpt_serial: 0,
			cuboids: Vec::new(),
			cuboid_basis: Vec::new(),
			paths: Vec::new(),
			path_w: Vec::new(),
			grind_rails: Default::default(),
			transport_glows: HashMap::new(),
			group_src: Vec::new(),
			group_lists: Vec::new(),
			load_refs: HashMap::new(),
			load_dropped: HashSet::new(),
			cylinders: Vec::new(),
			hero_lens: Vec::new(),
			death_len: 0.0,
			interact: false,
			hero_hidden: false,
			reticles: Vec::new(),
			mine_reticles: Vec::new(),
			doom_reticles: Vec::new(),
			sprite_queue: Vec::new(),
			gold_bolts: 0,
			gold_slots,
			skill_points,
			code_n: 0,
			code_buf: [0; 20],
			save_request: false,
			pyro_seqs: None,
			held: None,
			held_seqs: HashMap::new(),
			held_muzzle: None,
			morph_beam: Default::default(),
			taunt_rings: Default::default(),
			muzzles: HashMap::new(),
			hit_joints: HashMap::new(),
			rigs: HashMap::new(),
			joint_rows: HashMap::new(),
			attach0: HashMap::new(),
			attach_points: HashMap::new(),
			colliders: HashMap::new(),
			pyro: None,
			flames: pyro::Flames::default(),
			pyro_frames: pyro::Frames::default(),
			suck: None,
			suck_seqs: None,
			screen_fade: 0.0,
			prompt: None,
			rc: RcLink::default(),
			visibomb: None,
			vis_render: Default::default(),
			crate_nibbles,
			marker_hud: MarkerHud::default(),
			count_hud: Default::default(),
			breath_hud: Default::default(),
			shop_hud: Default::default(),
			mouse_joint: [None; 2],
			depart: false,
			story,
			level,
			pad: None,
			cam_cut: None,
			pad_press: 0,
			pad_hud: Vec::new(),
			shop: None,
			gadget: None,
			voice_lang: 0,
			voice_ended: false,
			portrait_stop: false,
			portrait_timing: None,
			vendor_fx: Default::default(),
		};
		g.build_load_refs(instances, &mission);
		g
	}

	/// ARI-8 (decomp/notes/aridia_flow.md "ARI-8 port spec"; FUN_00243000 OV:45481-45511, 45665-45681): the loader-time
	/// relocation of authored instance references, once, with the loader-time uid state (later destruction, `spawn_skip`
	/// re-runs and phase changes never rebuild it). A placement is allocated when it is in phase and not `load_skipped`;
	/// crate death-slot gates and scene hiding do not count (retail allocates those).
	fn build_load_refs(&mut self, instances: &[MobyInstance], mission: &[u8; 16]) {
		let by_index: HashMap<u32, &MobyInstance> = instances.iter().map(|m| (m.index, m)).collect();
		for k in 0..self.props.len() {
			let Some(i) = self.props[k].index else { continue };
			let Some(m) = by_index.get(&i) else { continue };
			let skipped = self.load_skipped(k);
			if skipped {
				self.load_dropped.insert(i);
			}
			if m.in_phase(mission) && !skipped {
				self.load_refs.insert(i, k);
			}
		}
	}



	/// Debug `--give`: owns `id`, in hand, with `ammo` (None = its start ammo); kept across deaths.
	pub fn give(&mut self, id: u8, ammo: Option<u16>) {
		self.inv.unlock(id, true);
		self.inv.last = id;
		if let Some(n) = ammo {
			self.inv.ammo[id as usize] = 0;
			self.inv.add_ammo(id, n);
		}
		self.inv_start = self.inv.clone();
	}

	/// Put the hero where a save-anywhere save left him (save.rs `Where`, a rac-rs extension [P]).
	/// The level spawn is kept for death respawns; the inventory becomes the one death restores.
	pub fn resume(&mut self, pos: V3, yaw: f32, health: i32, moves: u32, inv: Inventory) {
		self.player = self.player.respawned(add(pos, [0.0, 0.0, 0.05]), yaw);
		self.player.slope_push_on = self.level == 0;
		self.health = health.clamp(1, MAX_HEALTH);
		self.moves_done = moves;
		self.inv = inv;
		self.player.item_applied = self.inv.equipped;
		self.inv_start = self.inv.clone();
	}

	/// What a level change keeps (FUN_0028e858 -> loader): bolts, gold bolts, owned items with ammo,
	/// quick select and hand, and the tutor bits. Health is not carried.
	pub fn carry_out(&self) -> Carry {
		Carry { bolts: self.bolts, gold_bolts: self.gold_bolts, gold_slots: self.gold_slots, skill_points: self.skill_points, inv: self.inv.clone(), moves: self.moves_done, story: self.persist() }
	}

	/// The persistent state as the save records see it (save.rs Story): the stored story with the live
	/// inventory, gold-bolt bytes, skill points and this level's UID bits merged in.
	pub fn persist(&self) -> crate::save::Story {
		let mut s = self.story.clone();
		s.inv = self.inv.clone();
		s.refresh_prefs();
		s.gold = self.gold_slots;
		s.skill = self.skill_points;
		if let Some(b) = usize::try_from(self.level).ok().and_then(|l| s.uids.get_mut(l)) {
			*b = *self.uids.persistent_bits();
		}
		if let Some(r) = usize::try_from(self.level).ok().and_then(|l| s.crate_nibbles.get_mut(l)) {
			*r = self.crate_nibbles;
		}
		s
	}

	pub fn carry_in(&mut self, c: &Carry) {
		self.bolts = c.bolts;
		self.gold_bolts = c.gold_bolts;
		self.gold_slots = c.gold_slots;
		self.skill_points = c.skill_points;
		self.moves_done = c.moves;
		self.story = c.story.clone();
		self.crate_nibbles = usize::try_from(self.level).ok().and_then(|l| self.story.crate_nibbles.get(l)).copied().unwrap_or([0; 16]);
		self.inv = c.inv.clone();
		self.inv_start = self.inv.clone();
	}


	/// This level's saved bits (class, uid), kept per level for revisits.
	pub fn saved_bits(&self) -> HashSet<(u32, u32)> {
		self.saved.clone()
	}

	pub fn restore_saved(&mut self, bits: &HashSet<(u32, u32)>) {
		for &(class, idx) in bits {
			self.mark_saved(class, idx);
		}
	}

	/// GiveItem (decomp/notes/ingame_menu_leftovers.md P1-b): the one path that unlocks an item and
	/// queues its pickup popup (text id) for the HUD.
	pub fn give_item(&mut self, id: u8, ammo: Option<u16>) {
		let again = self.inv.have.get(id as usize).copied().unwrap_or(false);
		self.give(id, ammo);
		if let Some(t) = crate::inventory::popup_text(id, again) {
			self.popups.push((t, crate::gameplay::pal_ticks(300) as u32));
		}
	}


	/// The bomb is drawn in the glove: the glove is out and there is ammo for it.
	pub fn bomb_in_hand(&self) -> bool {
		self.inv.equipped == BOMB_GLOVE && self.inv.ammo(BOMB_GLOVE) > 0
	}

	/// NOV-3.4 (FUN_002554c8 OV:46340-46405): keeps the group section and builds each group's runtime list: the blob
	/// moby index maps to the prop of that placement when it was instantiated (dynamic and in phase), else it is
	/// dropped; the survivors keep blob order.
	pub fn set_group_source(&mut self, instances: &[MobyInstance], lists: Vec<Option<Vec<u16>>>) {
		let mission = usize::try_from(self.level).ok().and_then(|l| self.story.mission.get(l)).copied().unwrap_or([0; 16]);
		let mut map: HashMap<u32, usize> = HashMap::new();
		let actors = LevelActors::of(self.level);
		for (n, m) in instances.iter().filter(|m| actors.is_dynamic(m.class_id)).enumerate() {
			// ARI-8: a member the loader omitted by its uid test is a dropped reference (all dropped = a null group, OV:45731).
			if m.in_phase(&mission) && !self.load_dropped.contains(&m.index) {
				map.insert(m.index, n);
			}
		}
		self.group_lists = lists.iter().map(|l| l.as_ref().map_or(Vec::new(), |l| l.iter().filter_map(|&w| map.get(&((w & 0x7fff) as u32)).copied()).collect())).collect();
		self.group_src = lists;
	}


	/// [PC] Retail mode: one more pool prop per class in `classes` after the existing props (a grown pool, viewer
	/// `App::append_pools`), in the reset snapshot too while it mirrors the props. Every pool prop starts afresh (as in a
	/// new game; retail.rs `rebind` hands them out anew).
	pub fn append_retail(&mut self, classes: &[u32], level: i32) {
		for p in self.props.iter_mut().filter(|p| p.is_retail()) {
			*p = Prop::retail(p.class_id, level);
		}
		if self.placed.len() == self.props.len() {
			self.placed.extend(classes.iter().map(|&c| Prop::retail(c, level)));
		}
		self.props.extend(classes.iter().map(|&c| Prop::retail(c, level)));
	}

	/// The level's cuboids (gameplay +0x60), for the actors' trigger volumes.
	/// ARI-25: the level's gameplay paths (points only).
	pub fn set_paths(&mut self, paths: Vec<Vec<V3>>) {
		self.paths = paths;
	}

	/// KER-1a: the w lane of every gameplay path vertex (`Polygon.marks`, same index and length as `set_paths`).
	pub fn set_path_w(&mut self, w: Vec<Vec<f32>>) {
		self.path_w = w;
	}


	/// KER-60: the level's grind rails (`gameplay::grind_rails`), shared with the hero's rail probe.
	pub fn set_grind_rails(&mut self, rails: Vec<gameplay::GrindRail>) {
		let rails = std::rc::Rc::new(rails);
		self.player.grind_rails = rails.clone();
		self.grind_rails = rails;
	}

	pub fn set_cuboids(&mut self, cuboids: Vec<Cuboid>) {
		self.cuboids = cuboids;
		self.placed = self.props.clone();
	}

	/// ARI-22 / BRIDGE-59: the level's gameplay cylinders (`CamGrid::cylinder`), for the Aridia elevators' shaft test.
	pub fn set_cylinders(&mut self, cylinders: Vec<(V3, [V3; 3], [V3; 3])>) {
		self.cylinders = cylinders;
	}

	/// KER-5: the cuboid records' forward basis rows (`gameplay::cuboid_basis`), for `Game::cuboid_rand_point`.
	pub fn set_cuboid_basis(&mut self, basis: Vec<[V3; 3]>) {
		self.cuboid_basis = basis;
	}


	/// Each class's collision block (mobies::MobyClass::collision), for the hero push-out.
	pub fn set_colliders(&mut self, c: HashMap<u32, crate::mobies::Collision>) {
		self.colliders = c;
	}

	/// The mobys the hero queries test (mask 0x24: collision meshes and set-A primitives, collision_and_gameplay.md "Hero
	/// against solid mobys") near the hero, with their prop indices. A moby collides when its class has a collision
	/// block (class +0x10; a class without one has +0x94 = 0 from the spawn and no collision at all) and the game has
	/// not zeroed its +0x94: the robot's death (state 8), a hovering Birdbot asleep, dormant pooled Amoeboids, mission
	/// NPCs and teleporters until shown (their `shown` mirrors the +0x41 flag writes). Shown crates, enemies, foes and
	/// actors take part; the other props are left out [P: bolts, pickups and shots have no +0x94 writer read].
	pub fn solids(&self) -> (Vec<MobySolid>, Vec<usize>) {
		self.solids_around(self.player.pos)
	}

	/// `solids` within 16 m of `centre` (the foe step queries use the foe's position, COL-5).
	pub fn solids_around(&self, centre: V3) -> (Vec<MobySolid>, Vec<usize>) {
		let near = centre;
		let (mut out, mut index) = (Vec::new(), Vec::new());
		for (i, p) in self.props.iter().enumerate().filter(|(_, p)| p.shown && length(sub(p.pos, near)) < 16.0) {
			// Retail mode: every prop that has a collider (scenery and the retail pools) is a solid.
			let on = matches!(&p.kind, Kind::Scenery | Kind::Retail { .. });
			let Some(c) = self.colliders.get(&p.class_id).filter(|_| on) else { continue };
			let m = moby_axes(p.rot);
			let world = |l: V3| add(p.pos, add(scale(m[0], l[0] * p.scale), add(scale(m[1], l[1] * p.scale), scale(m[2], l[2] * p.scale))));
			let mut s = MobySolid::default();
			for &(f, attr) in &c.faces {
				if let (Some(&a), Some(&b), Some(&d)) = (c.verts.get(f[0] as usize), c.verts.get(f[1] as usize), c.verts.get(f[2] as usize)) {
					s.tris.push(([world(a), world(b), world(d)], attr));
				}
			}
			s.prims = self.prim_list(p);
			s.disabled = self.prim_disabled(p);
			if !s.tris.is_empty() || !s.prims.is_empty() {
				out.push(s);
				index.push(i);
			}
		}
		(out, index)
	}


	/// BRIDGE-4: the class collision primitives of a prop in world space, in class order (`index` = the +0x98 bit),
	/// radius times the relative scale; empty for a class without a collision block (retail +0x94 = 0).
	pub(super) fn prim_list(&self, p: &Prop) -> Vec<crate::physics::SolidPrim> {
		let Some(c) = self.colliders.get(&p.class_id) else { return Vec::new() };
		let m = moby_axes(p.rot);
		let world = |l: V3| add(p.pos, add(scale(m[0], l[0] * p.scale), add(scale(m[1], l[1] * p.scale), scale(m[2], l[2] * p.scale))));
		let mut out = Vec::new();
		// Primitives in class order (the +0x98 disable bits index them). A joint shape is placed from the
		// joint's live model-space position (the class skeleton posed at the prop's current animation).
		let live = self.live_joints(p, &c.prims.iter().flat_map(|(prim, _)| match *prim {
			crate::mobies::Prim::JointSphere { joint, .. } => vec![joint],
			crate::mobies::Prim::JointCapsule { a, b, .. } => vec![a, b],
			_ => Vec::new(),
		}).collect::<Vec<_>>());
		let joint = |j: usize| live.iter().find(|&&(k, _)| k == j).map(|&(_, l)| world(l));
		for (k, &(prim, sets)) in c.prims.iter().enumerate() {
			let shape = match prim {
				crate::mobies::Prim::Sphere { centre, r } => Some((world(centre), world(centre), r, 1)),
				crate::mobies::Prim::Upright { base, h, r } => Some((world(base), add(world(base), [0.0, 0.0, h * p.scale]), r, 3)),
				crate::mobies::Prim::JointSphere { joint: j, offset, r } => joint(j).map(|at| {
					let o = sub(world(offset), p.pos);
					(add(at, o), add(at, o), r, 1)
				}),
				crate::mobies::Prim::JointCapsule { a, b, r } => joint(a).zip(joint(b)).map(|(a, b)| (a, b, r, 4)),
			};
			if let Some((a, b, r, kind)) = shape {
				out.push(crate::physics::SolidPrim { index: k as u8, a, b, r: r * p.scale, sets, kind });
			}
		}
		out
	}







	/// The runtime per-primitive disable mask (moby +0x98) of a prop. The writers found for the classes the port
	/// places belong to the hero and the weapon children; no placed class's updater writes it.
	pub(super) fn prim_disabled(&self, _p: &Prop) -> u32 {
		0
	}

	/// Model-space positions of `joints` of the prop's class at its current animation (set_joint_rigs), else the
	/// firing-pose samples of set_hit_joints (the Birdbot's BIRDBOT_HIT_JOINTS).
	pub(crate) fn live_joints(&self, p: &Prop, joints: &[usize]) -> Vec<(usize, V3)> {
		if joints.is_empty() {
			return Vec::new();
		}
		if let Some(rig) = self.rigs.get(&p.class_id) {
			if let Some(v) = rig(&p.anim, joints) {
				return joints.iter().copied().zip(v).collect();
			}
		}
		let sampled = self.hit_joints.get(&p.class_id);
		joints.iter().filter_map(|&j| foes::BIRDBOT_HIT_JOINTS.iter().position(|&k| k == j).and_then(|k| sampled.and_then(|v| v.get(k))).map(|&l| (j, l))).collect()
	}



	/// BRIDGE-61: each class's joint-axis source (viewer: the class skeleton posed at the prop's clock).
	pub fn set_joint_rows_rigs(&mut self, r: HashMap<u32, JointRowsRig>) {
		self.joint_rows = r;
	}


	/// Each class's joint-position source (viewer: the class skeleton posed at the prop's clock), for the joint
	/// primitives and the foes' hit shapes in place of the sampled firing pose.
	pub fn set_joint_rigs(&mut self, r: HashMap<u32, JointRig>) {
		self.rigs = r;
	}

	/// BRIDGE-3: each class's attach-point -> joint table for `joint_world`.
	pub fn set_attach_points(&mut self, m: HashMap<u32, Vec<Option<usize>>>) {
		self.attach_points = m;
	}

	/// NOV-14.3: the sand mouse's attach points 1 and 2 as joints of its class.
	pub fn set_mouse_joints(&mut self, j: [Option<usize>; 2]) {
		self.mouse_joint = j;
	}














	/// Screen darkness 0..1: the 16-tick fade to black before the reset (FUN_001f97b8). Nothing fades
	/// back in: the rebuilt level cuts straight to full brightness.
	pub fn fade(&self) -> f32 {
		// ARI-41: the global fade word 0x15f3fc (a pickup's fade) covers the screen as the death fade does.
		self.dead.map_or(0.0, |t| 1.0 - (t / DEATH_FADE).min(1.0)).max(self.screen_fade)
	}

	/// Ratchet's sequences (viewer::load): their lengths in seconds of 50 Hz ticks and each frame's clock rate in frames
	/// per tick (the swim gait integral advances the hero anim clock by them).
	pub fn set_hero_sequences(&mut self, seqs: &[crate::anim::Sequence]) {
		self.hero_lens = seqs.iter().map(|q| q.tick_len() / 50.0).collect();
		self.player.frame_speeds = std::rc::Rc::new(seqs.iter().map(|q| q.rates()).collect());
		self.player.layer_rates = std::rc::Rc::new(seqs.iter().map(|q| q.frames.iter().map(|f| f.speed).collect()).collect());
	}



	/// The hero's death sequence and seconds into it while it plays (seq 16 for the hit state, then 69
	/// holding its last frame through the fade); None for a fade-only death.
	pub fn death_anim(&self) -> Option<(usize, f32)> {
		let left = self.dead?;
		if self.death_len <= 0.0 {
			return None;
		}
		let t = self.death_len + DEATH_FADE - left;
		let pose = ((t - DEATH_HIT - DEATH_BLEND).max(0.0) / 1.2).min((self.death_len - DEATH_HIT - DEATH_BLEND) / 1.2 - 1.0 / 60.0);
		Some(if t < DEATH_HIT { (DEATH_HIT_SEQ, t / 1.2) } else { (DEATH_SEQ, pose.max(0.0)) })
	}
}
