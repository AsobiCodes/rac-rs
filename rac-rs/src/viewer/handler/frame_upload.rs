//! RedrawRequested: the upload phase (draw lists to the GPU, window title), the scene camera put back, and the water hold
//! bookkeeping.

use super::*;

/// [PC] Water hold: (level, last field with pool strips, last field with RAM oceans) and how many fields an empty list is bridged.
pub(super) const WATER_HOLD: u32 = 4;
thread_local! {
	pub(super) static WATER_KEEP: std::cell::RefCell<(usize, u32, u32)> = const { std::cell::RefCell::new((usize::MAX, 0, 0)) };
}

impl App {
	pub(super) fn frame_upload(&mut self, f: &mut Frame) {
		let (at, tilt, hero_blend_t, hero_gens) = (f.at, f.tilt, f.hero_blend_t, f.hero_gens);
		let (ghost_ranges, bomb_range, light_at, nozzle) = (f.ghost_ranges, f.bomb_range, f.light_at, f.nozzle);
		let passenger_shown = f.passenger_shown; let skinned = std::mem::take(&mut f.skinned);
		let mut world_quads = std::mem::take(&mut f.world_quads); let vendor_quads = std::mem::take(&mut f.vendor_quads);
		let actors = std::mem::take(&mut f.actors); let fx_posed = std::mem::take(&mut f.fx_posed); let title = std::mem::take(&mut f.title);
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
			if let Some(g) = &mut self.gpu {
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
				g.set_props(Some(&game.props), self.smooth.posed(), &fx_posed, &actors, &self.debris, &rings, &fade_c, &fade_r); (f.actors, f.fx_posed) = (actors, fx_posed); // [PC] split view: kept for the Classic half's props re-run (frame_split.rs)
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
	}

	pub(super) fn frame_cam_restore(&mut self, f: &mut Frame) {
		// PC-FPS-1: past the world draw lists the scene camera of the true tick is put back (the sound below and the
		// next game ticks never see the drawn, offset one); the final draw takes the drawn one.
		profile::lap(profile::Phase::Upload);
		let drawn_cam = self.smooth.tick_cam.take().map(|(pos, yaw, pitch, tan_half)| {
			let d = Camera { pos: self.cam.pos, yaw: self.cam.yaw, pitch: self.cam.pitch, tan_half: self.cam.tan_half, up: self.cam.up };
			self.cam = Camera { pos, yaw, pitch, tan_half, up: [0.0, 0.0, 1.0] };
			d
		});
		f.drawn_cam = drawn_cam;
	}
}
