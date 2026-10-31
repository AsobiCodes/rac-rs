//! Binding the game's props to retail's mobys: the level switch, the prop pools' (re)bind order and `sync`, which
//! mirrors the last field into the game.

use super::*;

impl Retail {
	/// Retail finished travelling to another level: (level, its prop pools) once, after which the viewer loads that
	/// level and starts a game with this `Retail` again (the props bind anew).
	/// Retail arrived on another level and `switch` has not run yet (the viewer still holds the old level's assets).
	pub fn switch_pending(&self) -> bool {
		self.snap.level != self.level && self.snap.pools.is_some()
	}

	/// [PC] The level retail has begun travelling to (its level word changed) while `switch` has not run: the viewer starts that
	/// level's host-side parse now (`App::prefetch_level`), during the landing stream, instead of at the switch.
	pub fn pending_pools(&self) -> Option<Vec<(u32, u32)>> {
		if self.snap.level == self.level { None } else { self.snap.pools.clone() }
	}

	pub fn travel_target(&self) -> Option<usize> {
		(self.snap.level != self.level).then_some(self.snap.level as usize)
	}

	pub fn switch(&mut self) -> Option<(usize, Vec<(u32, u32)>)> {
		if self.snap.level == self.level {
			return None;
		}
		let pools = self.snap.pools.take()?;
		self.level = self.snap.level;
		self.free.clear();
		self.held.clear();
		self.bound = false;
		self.order = None;
		self.cam_prev = self.cam;
		self.cam_up_prev = self.cam_up;
		Some((self.level as usize, pools))
	}


	/// [PC] The grown pools of `take_grow`, the bound props left as they are: the viewer appends the new props to the
	/// running game (`App::append_pools`) and hands them out anew with `rebind`, or calls `unbind` and rebuilds.
	pub fn take_grow_kept(&mut self) -> Option<Vec<(u32, u32)>> {
		self.grow.take()
	}

	/// Every prop free again: the next `sync` binds the game's props anew (a rebuilt game, index order).
	pub fn unbind(&mut self) {
		self.free.clear();
		self.held.clear();
		self.bound = false;
		self.order = None;
	}

	/// [PC] The game's prop order `bind` follows (`order`), None = index order.
	pub fn bind_order(&self) -> Option<&[usize]> {
		self.order.as_deref()
	}

	/// [PC] An in-place pool growth (`App::append_pools`): every prop free again, handed out in `order` from the next
	/// `sync` on (as a rebuilt game binds its pools).
	pub fn rebind(&mut self, order: Vec<usize>) {
		self.unbind();
		self.order = Some(order);
	}

	/// RAC_RETAIL_TRAVEL is set and rac-rs has not reached its level yet (shots wait for it).
	pub fn travelling(&self) -> bool {
		self.travel.is_some_and(|(l, _)| l != self.level)
	}

	pub(super) fn bind(&mut self, g: &Game) {
		let order: Vec<usize> = match &self.order {
			Some(o) if o.len() == g.props.len() => o.clone(),
			_ => (0..g.props.len()).collect(),
		};
		for i in order {
			let p = &g.props[i];
			if p.is_retail() {
				self.free.entry(p.class_id).or_default().push(i);
			}
		}
		for v in self.free.values_mut() {
			v.reverse();
		}
		self.bound = true;
	}

	pub fn sync(&mut self, g: &mut Game) {
		if !self.bound {
			self.bind(g);
		}
		// The pause menu (mode 3) borrows mobys for its own scene while gameplay is frozen: keep the last frame.
		if self.snap.hold || self.snap.level != self.level {
			return;
		}
		// A scene with actors draws its own: its Ratchet (when retail draws it), Clank as an actor of its own, no held
		// item. A mode-2 field without actors (a scripted camera over gameplay) keeps the placed hero as it is.
		SCENE_SEQS.with(|c| {
			let mut c = c.borrow_mut();
			if self.snap.scene && !self.snap.scene_seqs.is_empty() {
				c.saved.get_or_insert((g.hero_hidden, g.player.clank_hidden, g.held_hidden));
				(g.hero_hidden, g.player.clank_hidden, g.held_hidden) = (!self.snap.hero_shown, true, true);
			} else if let Some(f) = c.saved.take() {
				(g.hero_hidden, g.player.clank_hidden, g.held_hidden) = f;
			}
		});
		g.health = self.snap.health;
		g.bolts = self.snap.bolts;
		if self.snap.have.len() == crate::inventory::GADGETS {
			g.inv.have.copy_from_slice(&self.snap.have);
			g.inv.ammo.copy_from_slice(&self.snap.ammo);
			(g.inv.equipped, g.inv.last) = (self.snap.equipped, self.snap.last);
		}
		// rac-rs's own story popups (planet unlocks at game start) are not retail's.
		g.popups.clear();
		// NOV-30g: retail hides the hero (and his parts) in the shop; the marker follows `hero_hidden`.
		// [merge M7] A scene with actors keeps its own choice (SCENE_SEQS above).
		if !(self.snap.scene && !self.snap.scene_seqs.is_empty()) {
			g.hero_hidden = self.snap.hero_hidden;
		}
		if let Some((pos, yaw, _)) = self.snap.hero {
			g.player.pos = pos;
			g.player.yaw = yaw;
			g.player.body_tilt = self.snap.hero_tilt;
		}
		// The held moby drives the viewer's held rigs (game/pyro.rs `mirror_held`); a moby a rig took is not a prop. The Bomb
		// Glove (192) has a viewer rig of its own (viewer.rs GLOVE_CLASS) and a prop pool for the purchase scene: its held moby
		// stays out of the props too.
		let held_addr = match self.snap.held {
			Some((a, c, clock)) => (g.mirror_held(Some(c), |p| clock.apply(p)) || c == 192).then_some(a),
			None => {
				g.mirror_held(None, |_| ());
				None
			}
		};
		let mut seen = HashSet::new();
		self.prop_fx.clear();
		// Live mobys per class that found no free prop: (live count, pool) for `grow`.
		let mut short: HashMap<u32, u32> = HashMap::new();
		// Tests only: RAC_WIDE_OFF=1 draws retail's 4:3 verdict as it was (the before pictures of scratch_widecull).
		let wide = !cfg!(test) || std::env::var_os("RAC_WIDE_OFF").is_none();
		let lag = self.view_lag();
		// Retail's draw position: its main walk in table order, then the deferred list (`RetailShadows::deferred`) in the same order.
		let deferred: &[u32] = self.snap.shadows.as_ref().map_or(&[], |s| &s.deferred);
		for (rank, mb) in self.snap.mobys.iter().enumerate().filter(|(_, mb)| Some(mb.addr) != held_addr) {
			let held = self.held.get(&mb.addr).copied().filter(|&i| g.props[i].class_id == mb.class);
			if held.is_none() {
				if let Some(i) = self.held.remove(&mb.addr) {
					release(g, &mut self.free, i);
				}
			}
			if let Some(i) = held.or_else(|| self.free.get_mut(&mb.class).and_then(|v| v.pop())) {
				self.held.insert(mb.addr, i);
				seen.insert(mb.addr);
				let p = &mut g.props[i];
				p.pos = mb.pos;
				p.rot = mb.rot;
				p.mirror = mb.mirror;
				p.order = rank as u32 | if deferred.contains(&mb.addr) { crate::game::DEFERRED_ORDER } else { 0 };
				p.scale = mb.scale;
				// Retail's verdict stays as it is in RAM (gameplay reads it); rac-rs's wide window also shows what retail culled only
			// at its 4:3 side plane.
			p.shown = mb.shown || (wide && mb.side_only.is_some_and(|s| s.inside_lag(self.aspect, lag)));
			p.retail_drawn = mb.shown;
			// [PC] Past its draw distance: posed (shadow and picture) only with Culling Off.
			p.cast = !p.shown && match mb.view_culled { Culled::View => true, Culled::Far => !pcext::graphics().culling, Culled::No => false };
				p.light = Some((mb.rgb, mb.light));
				p.set_retail_draw(mb.alpha, mb.ovr, mb.draw);
				p.set_retail_additive(mb.additive);
				if dbg_class() == Some(mb.class as u32) {
					eprintln!("DBGCLASS f{} {} {:#x} slot {i} rgb {:?} light {:#x} alpha {:#x} draw {} ovr {:?} shown {} scale {}", self.fields, mb.class, mb.addr, mb.rgb, mb.light, mb.alpha, mb.draw, mb.ovr, mb.shown, mb.scale);
				}
				mb.clock.apply(&mut p.anim);
				if !mb.fx.is_empty() {
					self.prop_fx.insert(i, mb.fx.clone());
				}
			} else if modelled(mb.class) {
				short.insert(mb.class, 0);
			}
		}
		if !short.is_empty() && self.grow.is_none() {
			for mb in &self.snap.mobys {
				if let Some(n) = short.get_mut(&mb.class) {
					*n += 1;
				}
			}
			let mut p: HashMap<u32, u32> = pools().into_iter().collect();
			for (&c, &need) in &short {
				let e = p.entry(c).or_insert(0);
				crate::loadlog::later(format!("retail: pool of class {c} full ({} props, {need} live): rebuilding the prop slots", *e));
				*e = grown(c, *e, need);
			}
			let mut p: Vec<(u32, u32)> = p.into_iter().collect();
			p.sort();
			self.grow = Some(p);
		}
		let gone: Vec<(u32, usize)> = self.held.iter().filter(|(a, _)| !seen.contains(a)).map(|(&a, &i)| (a, i)).collect();
		for (a, i) in gone {
			self.held.remove(&a);
			release(g, &mut self.free, i);
		}
	}
}

pub(super) fn release(g: &mut Game, free: &mut HashMap<u32, Vec<usize>>, i: usize) {
	g.props[i].shown = false;
	g.props[i].retail_drawn = false;
	g.props[i].cast = false;
	free.entry(g.props[i].class_id).or_default().push(i);
}
