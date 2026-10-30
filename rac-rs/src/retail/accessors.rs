//! The viewer's read access to the latest snapshot: scene sequences, hero parts, shadows, orbs, shop, effects and
//! the frame-smoothing lists.

use super::*;

impl Retail {
	/// [PC] TEST ONLY: the field number of the snapshot now shown (fields taken from the game thread; RAC_TEST_CLOCK).
	pub fn shown_field(&self) -> u32 {
		self.snap.field
	}

	/// Puts the scene actors' sequences of the last field (`scene_seq`) into the viewer's class animations at the slots
	/// retail appended them to (class 0 = the hero's), as scene_chunk_parse does to the class tables; after the scene the
	/// viewer's own sequences come back (retail's clocks only reach a scene slot during the scene).
	pub fn scene_seqs(&mut self, anims: &mut HashMap<u32, (crate::mobies::MobyClass, crate::anim::Animated)>, mut hero: Option<&mut (crate::mobies::MobyClass, crate::anim::Animated)>) {
		if std::env::var_os("RAC_SCENE_LOG").is_some() && self.snap.mode != 0 && self.fields % 10 == 0 {
			eprintln!("retail: view field {} level {} mode {} ship {} frozen {} scene {}", self.snap.field, self.snap.level, self.snap.mode, self.snap.ship, self.snap.frozen, self.snap.scene);
			if self.snap.mode == 6 {
				let mut cls: Vec<(u32, bool)> = self.snap.mobys.iter().map(|m| (m.class, m.shown)).collect();
				cls.sort_unstable();
				cls.dedup();
				eprintln!("retail: view field {} classes (class, shown) {cls:?}", self.snap.field);
			}
		}
		if std::env::var_os("RAC_SCENE_LOG").is_some() && (self.snap.mode != 0 || self.snap.screen.2 > 0.0) {
			eprintln!("retail: screen field {} mode {} skip/mask/fade/white/gate {:?} pmode/dispfb/bg {:x?}", self.snap.field, self.snap.mode, self.snap.screen, self.snap.crtc);
		}
		if std::env::var_os("RAC_SCENE_LOG").is_some() && self.snap.scene && self.fields % 50 == 0 {
			let h = self.snap.hero.map(|(p, y, c)| (p, y, c.cur, c.next, c.old_seq, c.new_seq, c.p));
			eprintln!("retail: scene field {} seqs {:?} hero {h:?} shown {}", self.snap.field, self.snap.scene_seqs.iter().map(|x| (x.0, x.1)).collect::<Vec<_>>(), self.snap.hero_shown);
		}
		if self.snap.level != self.level || self.snap.hold {
			return;
		}
		SCENE_SEQS.with(|c| {
			let mut c = c.borrow_mut();
			if !self.snap.scene {
				for ((class, slot), q) in std::mem::take(&mut c.orig) {
					let a = if class == 0 { hero.as_deref_mut() } else { anims.get_mut(&class) };
					if let (Some((_, a)), Some(q)) = (a, q) {
						if let Some(s) = a.sequences.get_mut(slot as usize) {
							*s = q;
						}
					}
				}
				c.done.clear();
				return;
			}
			for (class, slot, b) in &self.snap.scene_seqs {
				let a = if *class == 0 { hero.as_deref_mut() } else { anims.get_mut(class) };
				let Some((_, a)) = a else { continue };
				let i = *slot as usize;
				if std::env::var_os("RAC_SCENE_LOG").is_some() && !c.done.contains_key(&(*class, *slot)) {
					eprintln!("retail: scene seq class {class} slot {slot} len {} bytes {}", a.sequences.len(), b.len());
				}
				let have = a.sequences.get(i).is_some_and(|s| !s.frames.is_empty());
				if have && c.done.get(&(*class, *slot)) == Some(b) {
					continue;
				}
				match crate::anim::parse_sequence_frames(b, b, a.skeleton.parents.len()) {
					Ok(q) => {
						if a.sequences.len() <= i {
							a.sequences.resize_with(i + 1, || crate::anim::Sequence { frames: Vec::new(), speed: 0.0, sphere: [0.0; 4], triggers: Vec::new(), loop_sound: 0xff });
						}
						let old = std::mem::replace(&mut a.sequences[i], q);
						c.orig.entry((*class, *slot)).or_insert(Some(old));
						c.done.insert((*class, *slot), b.clone());
					}
					Err(e) => eprintln!("retail: scene sequence of class {class} slot {slot}: {e}"),
				}
			}
		});
	}

	/// The hero moby's animation clock of the last field.
	pub fn hero_clock(&self) -> Option<Clock> {
		self.snap.hero.map(|h| h.2)
	}

	/// The +0x64 joint override records of the moby prop `i` draws (empty when none or not a retail prop).
	pub fn prop_fx(&self, i: usize) -> &[crate::anim::JointFx] {
		self.prop_fx.get(&i).map_or(&[], |v| v.as_slice())
	}

	/// The hero's held wrench, back objects, joint records and the loose wrenches retail shows (with the 16:9 side
	/// widening of `sync`) of the last gameplay field; None while frozen or while the level rac-rs shows is not retail's
	/// (the caller keeps the last).
	pub fn hero_parts(&self) -> Option<HeroParts> {
		if self.snap.frozen || self.snap.level != self.level || self.snap.hero.is_none() {
			return None;
		}
		let wide = !cfg!(test) || std::env::var_os("RAC_WIDE_OFF").is_none();
		let mut p = self.snap.parts.clone();
		let lag = self.view_lag();
		p.loose.retain(|l| l.shown || (wide && l.side_only.is_some_and(|s| s.inside_lag(self.aspect, lag))));
		Some(p)
	}

	/// M3a shadows: retail's shadow records of the last drawn frame, the props on the deferred draw path (drawn after the
	/// volumes) and whether the hero is on it; None while the level rac-rs shows is not retail's.
	pub fn shadows(&self) -> Option<(Vec<crate::shadows::RecCaster>, Vec<usize>, bool)> {
		if self.snap.level != self.level {
			return None;
		}
		let s = self.snap.smooth.as_ref().and_then(|d| d.shadows.as_ref()).or(self.snap.shadows.as_ref())?;
		let late = s.deferred.iter().filter_map(|a| self.held.get(a).copied()).collect();
		Some((s.casters.iter().map(|c| c.1.clone()).collect(), late, s.hero_deferred))
	}

	/// Mirrors the last field's moby table into the game's props and the hero moby into the player.
	/// The nanotech orbs retail queued for drawing in the last gameplay field (`orb_draws`); empty while the level
	/// rac-rs shows is not retail's (travel).
	pub fn orb_draws(&self) -> &[crate::particles::OrbDraw] {
		match self.snap.smooth.as_ref().and_then(|d| d.orbs.as_ref()).or(self.snap.orbs.as_ref()) {
			Some(o) if self.snap.level == self.level => o,
			_ => &[],
		}
	}

	/// The effect pool and draw-queue extras of the last gameplay field (`queue_fx`); None while the level rac-rs shows
	/// is not retail's.
	/// The vendor shop mobys of the last field (`shop_snap`), None outside the shop screen.
	pub fn shop(&self) -> Option<&ShopSnap> {
		self.snap.shop.as_ref().filter(|_| self.snap.level == self.level)
	}

	/// [PC] PREVIEW (viewer/vegfx.rs touch): the mobys of the last field, empty while the level rac-rs shows is not retail's.
	pub fn live_mobys(&self) -> &[Moby] {
		if self.snap.level == self.level { &self.snap.mobys } else { &[] }
	}

	/// [PC] The shown field's own effect lists as the game ticked them, before Smoothing blends them with the field before (`fx`).
	pub fn fx_tick(&self) -> Option<&RetailFx> {
		self.snap.fx.as_ref().filter(|_| self.snap.level == self.level)
	}

	pub fn fx(&self) -> Option<&RetailFx> {
		match self.snap.smooth.as_ref().and_then(|d| d.fx.as_ref()).or(self.snap.fx.as_ref()) {
			Some(f) if self.snap.level == self.level => Some(f),
			_ => None,
		}
	}

	/// [PC] Frame smoothing (viewer/interp.rs): the shown field's own lists (fx, shadow records, orbs) while rac-rs shows retail's
	/// level, and (level, game mode, scene view, frozen or held) of the shown field.
	pub fn smooth_src(&self) -> (Option<&RetailFx>, Option<&RetailShadows>, Option<&[crate::particles::OrbDraw]>, (u32, u32, bool, bool)) {
		let mine = self.snap.level == self.level;
		(self.snap.fx.as_ref().filter(|_| mine), self.snap.shadows.as_ref().filter(|_| mine), self.snap.orbs.as_deref().filter(|_| mine), (self.snap.level, self.snap.mode, self.snap.scene, self.snap.frozen || self.snap.hold || self.snap.front))
	}

	/// The prop index that draws the moby at `addr` this field (None: not a prop, e.g. the held weapon the viewer draws as a rig).
	pub fn prop_of_addr(&self, addr: u32) -> Option<usize> {
		self.held.get(&addr).copied()
	}

	/// The props of the mobys retail placed on the hero from its own pose this field (`RetailFx::hero_parts`; empty without the
	/// Modern camera strafe draw).
	pub fn hero_part_props(&self) -> Vec<usize> {
		self.fx_tick().map_or(Vec::new(), |fx| fx.hero_parts.iter().filter_map(|&a| self.prop_of_addr(a)).collect())
	}

	/// The address of the moby in the hero's hand this field (the held weapon), if any.
	pub fn held_addr(&self) -> Option<u32> {
		self.snap.held.map(|h| h.0)
	}

	/// [PC] Frame smoothing: the blended lists this display frame draws (None = the field's own).
	pub fn set_smooth(&mut self, d: Option<SmoothDraw>) {
		self.snap.smooth = d.map(Box::new);
	}

	/// [PC] Frame smoothing: the blended effect lists `set_smooth` gave this display frame, for the queue-A / queue-B blend that
	/// waits for the effects layer (viewer/interp/late.rs).
	pub fn smooth_fx_mut(&mut self) -> Option<&mut RetailFx> {
		self.snap.smooth.as_mut().and_then(|d| d.fx.as_mut())
	}
}
