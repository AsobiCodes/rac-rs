//! RedrawRequested: hero phase (hero pose and held items, scene actors and flight camera, effects).

use super::*;

impl App {
	pub(super) fn frame_hero(&mut self, f: &mut Frame) {
		let at = f.at;
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
			let p = &game.player;
			let gloved = game.inv.equipped == crate::inventory::BOMB_GLOVE;
			let bomb_in_hand = game.bomb_in_hand();
			// The held Pyrocitor's pose and nozzle (game/pyro.rs); the wrench is put away while it is held.
			let (pyro_pose, nozzle) = (game.pyro_pose(), game.pyro_nozzle());
			let suck_pose = game.suck_pose();
			let held_pose = game.held_pose();
			let mut held_muzzle_model = None;
			let mut held_xform_model = None;
			let mut held_joint0_model = None;
			let (mut held_rows, mut muzzle_model) = (None, None);
			// PC perf (speed lane A, B1): everything below up to `skinned` is a function of the game state, which only changes in
			// a tick. A display frame with no tick and an equal key reuses the last frame's posed bundle (hero, glove, held items,
			// back objects, wrench, ghosts) and the side values derived with it; the per-frame blend stays in `smooth.hero`.
			// RAC_HERO_CHECK=1: always computes fresh, and compares it with what would have been reused.
			let hero_key = hero_cache_key(self.hero_anim.as_ref().map_or(0, |x| x as *const _ as usize), self.level, self.play_ticks, self.retail.as_ref().map(|r| r.field_count()), [self.hero_rig.hero.layers.len(), self.loose_anims.len(), self.held_rigs.len()], self.retail_parts.is_some() as u64 | (self.wrench_rig.is_some() as u64) << 1, self.retail_parts.as_ref().map_or(0, |q| crate::anim_pose::ops_hash(&q.pose_ops)), game);
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
				// Modern controls (retail/strafe.rs): the legs twist toward the ground path while aiming (off = zero twist); retail's parts follow (`strafe_fix`).
				if let Some(q) = rparts {
					crate::retail::strafe::twist_mats(&mut mats, a, q.strafe.twist); follow::capture(&q.follow, mats.len(), |j| norm_cols(a.joint_world(&mats, j)));
				}
				if !(hero_check && self.smooth.ticks == 0) { self.smooth.set_pose_fix(/* once per tick: RAC_HERO_CHECK=1 poses every display frame */ rparts.and_then(|q| strafe_fix(&mut self.hero_rig.hero, a, q, self.retail.as_ref().and_then(|r| r.hero_clock()), &mats))); }
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
							let hj = pyro_basis(p, self.retail.as_ref(), &mut self.smooth, a.joint_world(&mats, r.joint));
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
							held_xform_model = Some((rows, apply(&hj, [0.0; 3], 1.0))); follow::capture_held(&hj, &root);
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
				let fresh = HeroCache { key: hero_key, pose_gen: hero_gen, skinned: herocache::copy_posed((!hero_cand).then(|| self.hero_cache.take()).flatten(), &skinned), ghost_ranges, bomb_range, light_at, held_rows, muzzle_model, held_muzzle_model, held_joint0_model, held_xform_model };
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
			f.at = at; f.skinned = skinned; f.tilt = tilt; f.hero_blend_t = hero_blend_t; f.hero_gens = hero_gens;
			f.ghost_ranges = ghost_ranges; f.bomb_range = bomb_range; f.light_at = light_at; f.nozzle = nozzle;
		}
	}

	pub(super) fn frame_hero_scene(&mut self, f: &mut Frame) {
		let (mut at, mut tilt, mut hero_blend_t, mut hero_gens) = (f.at, f.tilt, f.hero_blend_t, f.hero_gens);
		let mut skinned = std::mem::take(&mut f.skinned);
		let mut ghost_ranges = f.ghost_ranges;
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
			let p = &game.player;
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
			f.at = at; f.skinned = skinned; f.tilt = tilt; f.hero_blend_t = hero_blend_t; f.hero_gens = hero_gens;
			f.actors = actors; f.passenger_shown = passenger_shown; f.eye_glows = eye_glows; f.flight_pose = flight_pose; f.ghost_ranges = ghost_ranges;
		}
	}

	pub(super) fn frame_hero_fx(&mut self, f: &mut Frame) {
		let (mut at, tilt, dt) = (f.at, f.tilt, f.dt);
		let mut skinned = std::mem::take(&mut f.skinned); let (mut hero_blend_t, mut hero_gens, mut ghost_ranges) = (f.hero_blend_t, f.hero_gens, f.ghost_ranges);
		let eye_glows = std::mem::take(&mut f.eye_glows); let mut world_quads = std::mem::take(&mut f.world_quads);
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
			let p = &game.player;
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
			// [PC] The props on the hero (Clank, back pack, head items, bulb mesh, what retail placed in his hand) ride his drawn placement (smooth.rs `attach`).
			self.smooth.attach(&game.props, &self.retail.as_ref().map_or(Vec::new(), |r| r.hero_part_props()), at, tilt);
			// Retail mode: retail's whole effect pool and the hero glow callback (retail.rs `queue_fx`).
			if let Some(fx) = self.retail.as_ref().and_then(|r| r.fx()) {
				// The pool's lines and ribbons join the pool sprites' depth walk in `set_particles` (queue_b.rs `pool_walk`); [PC] a glow a hero part owns rides it.
				let tick = self.retail.as_ref().and_then(|r| r.fx_tick());
				self.smooth.pool_sprites(&game.props, fx, tick, |a| self.retail.as_ref().and_then(|r| r.prop_of_addr(a)), &mut self.sprites);
				for (i, g) in fx.glows.iter().enumerate() {
					// [PC] A hero glow rides the part it sits on as `attach` draws it (Clank's bulb on the bulb mesh, the Pyrocitor's pilot light on its flame).
					let g_at = if i >= fx.hero_glows { g.0 } else { self.smooth.hero_glow(&game.props, tick.and_then(|t| t.glows.get(i)).map_or(g.0, |t| t.0), g.0, crate::retail::is_bulb_glow(g), tick.unwrap_or(fx).gun_glows.contains(&i)) };
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
			f.vendor_quads = vendor_quads; f.skinned = skinned; f.world_quads = world_quads;
			f.at = at; f.hero_blend_t = hero_blend_t; f.hero_gens = hero_gens; f.ghost_ranges = ghost_ranges;
		}
	}
}
