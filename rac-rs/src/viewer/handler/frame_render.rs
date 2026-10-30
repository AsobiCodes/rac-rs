//! RedrawRequested: the render phase (final draw, capture, present, frame end).

use super::*;

impl App {
	pub(super) fn frame_render(&mut self, f: &mut Frame, el: &ActiveEventLoop) {
		let paused = f.paused;
		let drawn_cam = std::mem::take(&mut f.drawn_cam);
		if let Some(g) = &mut self.gpu {
			// [PC] Graphics -> Lighting Enhanced (viewer/post.rs): only over a retail level drawn by rac-rs (set below).
			g.modern_ok = false; g.hud_place = None;
			// Retail mode: the game's own HUD and menus (GS capture) replace rac-rs's. The 4:3 frame fills the
			// window height, centred; the edge columns stretch to the sides (clamped).
			if let Some(r) = self.retail.as_mut().filter(|r| r.gs && r.front_ram().is_none()) {
				// GS-full (--gsfull / RAC_GSFULL=1): the game's whole picture (gsfull/pipe.rs) instead of the overlay.
				let full = r.gsfull_view().is_some();
				g.modern_ok = !full;
				// Sharp HUD (lane G): the GPU capture draws the overlay again at this window's resolution (0: off).
				crate::retail::set_hud_target(if full { 0 } else { g.config.height });
				g.ship_fight = r.ship_fight();
				g.level = self.level;
				if full {
					if let Some(f) = r.take_gsfull_frame() {
						g.set_retail_overlay(f.w, f.h, bytemuck::cast_slice(&f.rgba), Default::default(), false);
					}
				} else if let Some((w, rows, px, guard)) = r.take_overlay() {
					g.set_retail_overlay_async(w, rows, px, guard, hud_place::live(r));
				}
				g.hud.clear(); crate::viewer::hudart::glyphtex::refresh(g);
				g.hud_split = None;
				if g.has_retail_overlay() {
					let (sw, sh) = (g.config.width as f32, g.config.height as f32);
					if full {
						// The 512x448 frame is a 4:3 picture (16:9 squeezed in wide mode, gsfull/wide.rs): centred at the
						// largest size that fits, black bars around; square pixels either way.
						crate::overlay::rect(&mut g.hud, [0.0, 0.0, sw, sh], [0.0, 0.0, 0.0, 1.0]);
						let ar = if r.gsfull_view().is_some_and(|f| f.wide) { crate::gsfull::wide::ASPECT } else { 4.0 / 3.0 };
						let (w, h) = if sw >= sh * ar { (sh * ar, sh) } else { (sw, sw / ar) };
						let (x, y) = ((sw - w) * 0.5, (sh - h) * 0.5);
						g.hud.push(crate::overlay::Quad { tex: RETAIL_OV_TEX, rect: [x, y, w, h], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
					} else {
						let k = (sw / sh) / (4.0 / 3.0);
						if r.fx().is_some_and(|f| f.rc.is_some()) && k > 1.0 {
							// RC range static (queue_b::rc_static): the overlay holds retail's static over the 4:3 centre, so it is
							// drawn there unclamped (edge-clamped columns would streak the bars) and each bar shows translated copies
							// of it [PC: widescreen].
							crate::retail::queue_b::rc_bar_quads(RETAIL_OV_TEX, sw, sh, &mut g.hud);
						} else {
							let qs = hud_place::overlay_quads(g, r, k); g.hud.extend(qs);
						}
						// Retail shop screen: what retail draws through VU1 (absent from the GS capture), from the shop slot mobys
						// (retail::shop_snap): the world item over the pad, panel 1's item + bolt icon and panel 2's vendor actor.
						// The two targets go over the overlay's panel interiors (retail clears them black, blits them opaque,
						// then draws the panel text into them; the overlay holds that text, so the target is added onto it).
						g.shop_models.clear();
						g.shop_panel_models.clear();
						g.icon_bolt = None;
						g.icon_bolt_sphere = None;
						g.popup_bolt = None;
						g.popup_bolt_sphere = None;
						g.pose_portrait(None);
						g.pose_panel_item(None);
						if let (Some(sn), Some(game)) = (r.shop(), self.game.as_ref()) {
							let mat = |s: &crate::retail::ShopMoby| {
								let mut m = place(s.pos, s.rot);
								m[..12].iter_mut().for_each(|v| *v *= s.scale);
								(s.class, m)
							};
							g.shop_models = sn.world.iter().map(mat).collect();
							g.shop_panel_models = sn.panel.iter().map(mat).collect();
							// NOV-30d (1)/(3): item A is skinned at its retail clock (seq 0 / 1, stepped per tick), not drawn in bind
							// pose [C: vendor.md; s11_ammo bomb-glove panel 192 showed the fingers only posed]. Rig = level prop
							// animation, else the held gadget rigs (glove, Pyrocitor, Suck Cannon, swingshot, the other held items).
							let posed = sn.panel.as_ref().and_then(|s| {
								fn held(h: &Held) -> (&mobies::MobyClass, &anim::Animated) {
									(&h.class, &h.anim)
								}
								let (class, a) = self.prop_anims.get(&s.class).map(|(c, a)| (c, a))
									.or_else(|| self.held_rigs.get(&s.class).map(|r| held(&r.held)))
									.or_else(|| self.glove.as_ref().filter(|_| s.class == GLOVE_CLASS).map(held))
									.or_else(|| self.pyro_rig.as_ref().filter(|_| s.class == game::pyro::PYRO_CLASS).map(|r| held(&r.held)))
									.or_else(|| self.suck_rig.as_ref().filter(|_| s.class == 849).map(|r| held(&r.held)))
									.or_else(|| self.hook_rig.as_ref().filter(|_| s.class == game::held::SWINGSHOT_HOOK).map(held))?;
								let mut pa = crate::anim::PropAnim::new(0);
								s.clock.apply(&mut pa);
								let (mut ps, mut ns) = anim::skin(class, &pa.pose(a, &[]));
								let n = class.mesh.positions.len();
								ps.resize(n, [0.0; 3]);
								ns.resize(n, [0.0, 0.0, 1.0]);
								Some((s.class, ps, ns))
							});
							g.pose_panel_item(posed);
							g.icon_bolt = sn.icon.as_ref().map(|s| mat(s).1);
							// NOV-35b: the icon bolt's draw sphere (class 13 seq sphere at the slot's pose) for its metal pass.
							g.icon_bolt_sphere = sn.icon.as_ref().and_then(|s| game.bolt_sphere(s.pos, s.rot));
							let portrait = sn.portrait.as_ref().and_then(|s| {
								let (class, a) = self.prop_anims.get(&game::vendor::PORTRAIT_CLASS)?;
								let mut pa = crate::anim::PropAnim::new(0);
								s.clock.apply(&mut pa);
								Some((anim::skin(class, &pa.pose(a, &[])), s.pos, s.rot))
							});
							g.pose_portrait(portrait.as_ref().map(|(p, pos, rot)| (p, *pos, *rot)));
							// The six screens' corner joints of the vendor (class 11) as the shop path takes them, projected with
							// retail's own projection (horizontal tan = the retail scalar) into the 512x448 frame the overlay shows.
							let joints: Option<Vec<[f32; 3]>> = game.props.iter().find(|p| p.class_id == 11 && p.shown).and_then(|p| {
								let (_, a) = self.prop_anims.get(&11)?;
								let mats = p.anim.pose(a, &[]);
								let m = place(p.pos, p.rot);
								Some((0..36).map(|j| {
									let jm = a.joint_world(&mats, j.min(mats.len() - 1));
									let l = [jm[3] * p.scale, jm[7] * p.scale, jm[11] * p.scale];
									[0, 1, 2].map(|i| m[i] * l[0] + m[4 + i] * l[1] + m[8 + i] * l[2] + m[12 + i])
								}).collect())
							});
							let vp = view_proj(&self.cam, 1.0 / PROJ_Y_RATIO, RETAIL_NEAR, RETAIL_FAR);
							// Frame px (512 x 448) of a world point.
							let frame = |p: [f32; 3]| project(&vp, p).map(|c| [(c[0] + 1.0) * 256.0, (1.0 - c[1]) * 224.0]);
							// Frame px -> window px through the overlay's horizontal crop (u = 0.5 - k/2 + x/sw * k).
							let sx = |x: f32| (x / 512.0 - (0.5 - 0.5 * k)) / k * sw;
							// The purchase popup (retail FUN_002904f0): the slot +0x500 bolt in the popup target, blitted whole
							// ((0, 0)-(w, h), FUN_001ecaf8) to the rect of the mirrored prompt prop's attach points 0 / 1 / 2.
							g.popup_bolt = sn.popup_bolt.as_ref().map(|s| mat(s).1);
							g.popup_bolt_sphere = sn.popup_bolt.as_ref().and_then(|s| game.bolt_sphere(s.pos, s.rot));
							let popup = sn.prompt.and(game.props.iter().find(|p| p.class_id == game::vendor::PROMPT_CLASS && p.shown && p.is_retail())).and_then(|p| {
								let (_, a) = self.prop_anims.get(&game::vendor::PROMPT_CLASS)?;
								let joints = [a.attach0?, a.attach12[0]?, a.attach12[1]?];
								let mats = p.anim.pose(a, &[]);
								if joints.iter().any(|&j| j >= mats.len()) {
									return None;
								}
								let m = place(p.pos, p.rot);
								let j = joints.map(|j| {
									let jm = a.joint_world(&mats, j);
									let l = [jm[3] * p.scale, jm[7] * p.scale, jm[11] * p.scale];
									[0, 1, 2].map(|i| m[i] * l[0] + m[4 + i] * l[1] + m[8 + i] * l[2] + m[12 + i])
								});
								crate::shop::popup_rect(&j, &frame)
							});
							if let Some(q) = popup {
								let (x0, y0, w0, h0) = (q[0], q[1].min(q[1] + q[3]), q[2], q[3].abs());
								if w0 >= 1.0 && h0 >= 1.0 {
									let rect = [sx(x0), y0 / 448.0 * sh, sx(x0 + w0) - sx(x0), h0 / 448.0 * sh];
									let uv = [0.0, 0.0, w0 as i32 as f32 / 512.0, h0 as i32 as f32 / 128.0];
									g.hud.push(crate::overlay::Quad { tex: POPUP_PANEL_TEX, rect, uv, colour: [1.0; 4], angle: 0.0, corners: None });
								}
							}
							if let Some(rects) = joints.map(|j| crate::shop::screen_rects(&j, &frame)) {
								for (i, tex) in [(1, ITEM_PANEL_TEX), (2, PORTRAIT_PANEL_TEX)] {
									let q = rects[i];
									let (x0, y0, w0, h0) = (q[0], q[1].min(q[1] + q[3]), q[2], q[3].abs());
									if w0 <= 1.0 || h0 <= 1.0 {
										continue;
									}
									let rect = [sx(x0), y0 / 448.0 * sh, sx(x0 + w0) - sx(x0), h0 / 448.0 * sh];
									// The blit's source texels (0,0)-(w0 - 1, int(h0 * 13/14 - 1)) of the 512x128 target (shop::panel_uv).
									let uv = [0.0, 0.0, (w0 - 1.0) as i32 as f32 / 512.0, (h0 * 0.92857146 - 1.0) as i32 as f32 / 128.0];
									g.hud.push(crate::overlay::Quad { tex, rect, uv, colour: [1.0; 4], angle: 0.0, corners: None });
								}
							}
						}
						crate::retail::queue_b::screen_quads(r.fx().map_or(&[][..], |f| &f.screen), sw, sh, &mut g.hud);
					}
				}
			}
			// [PC] DEV ONLY: the dev tools' readout and planet list over everything (retail/devtools.rs; nothing with RAC_DEV_TOOLS off).
			if self.retail.is_some() {
				let hero = self.game.as_ref().map_or([0.0; 3], |gm| gm.player.pos);
				crate::retail::devtools::draw(&mut g.hud, g.config.width as f32, g.config.height as f32, self.level, hero);
			}
			// [PC] RAC_FPS_METER=1: fps, frame time and game speed in the top right corner (viewer/profile.rs `meter_draw`).
			profile::meter_draw(&mut g.hud, g.config.width as f32, g.config.height as f32); if let Some(r) = self.retail.as_ref() { crate::retail::splitview::strip_hud(&mut g.hud, &mut g.hud_split, r.front_ram().is_none() && !r.paused() && g.shop_models.is_empty()); } // [PC] Hide HUD (F11)
			// --title boot (retail/title_boot.rs): plain black over the display (and no sound) while the game steers itself to its title.
			let cover = self.retail.as_ref().is_some_and(|r| r.title_cover());
			if let Some(a) = &mut self.audio {
				a.master = if cover { 0.0 } else if a.master == 0.0 { 1.0 } else { a.master };
			}
			if cover {
				g.hud.clear();
				g.hud_split = None;
				let (sw, sh) = (g.config.width as f32, g.config.height as f32);
				crate::overlay::rect(&mut g.hud, [0.0, 0.0, sw, sh], [0.0, 0.0, 0.0, 1.0]);
			}
			// Retail mode: the fog block, pool far clamp and shrub cap the retail draw of this field reads (retail.rs
			// `SwapWords`: zone fog, underwater swap and the Visibomb render snapshot alike) replace the level-data fog.
			if let Some(fx) = self.retail.as_ref().and_then(|r| r.fx()) {
				if let Some(w) = fx.swap {
					g.fog_live = Some((w.fog_rgb, w.fog));
					g.part_far = w.part_far;
					g.under_flag = crate::viewer::under_detect::camera_under(w.under, w.surf, drawn_cam.as_ref().unwrap_or(&self.cam).pos[2]);
					g.under_surf = w.surf;
				}
				if let Some(c) = fx.shrub_cap {
					g.shrub_cap = c;
				}
			}
			// [PC] The world is black while retail is on another level than the one loaded here (its load is over, no frozen load screen, and its
			// landing has begun, the switch is still to come: the old level's picture showed for a few frames) and until it presents
			// its first picture of the new level (`arrival_black`). Retail draws that landing on black (its fade-in).
			let ahead = self.retail.as_ref().is_some_and(|r| r.front_ram().is_none() && !r.paused() && r.travel_target().is_some_and(|t| t < self.levels.len()));
			if self.arrival_black > 0 {
				self.arrival_black = if ahead || self.retail.as_ref().is_none_or(|r| !r.holds()) { 0 } else { self.arrival_black - 1 };
			}
			if ahead || self.arrival_black > 0 {
				let mut b = Vec::new();
				crate::overlay::rect(&mut b, [0.0, 0.0, g.config.width as f32, g.config.height as f32], [0.0, 0.0, 0.0, 1.0]);
				g.hud.splice(0..0, b);
				g.hud_split = g.hud_split.map(|s| s + 1);
			}
			// --shot: let a few frames settle, save one, quit.
			let capture = self.shot.as_ref().is_some_and(|s| s.1 == 0);
			// While the switch waits for the prefetch (`switch_hold`) the frame still draws: the world is black then (above), no stale picture stays up.
			let ly_img = { let c = drawn_cam.as_ref().unwrap_or(&self.cam); super::frame_split::run(g, self.game.as_ref(), &self.smooth, &self.debris, f, c); g.draw(c, capture) };
			let mut taken = false; // RAC_TEST_SUB (test_clock.rs): this frame was captured
			if let Some(img) = ly_img.filter(|_| capture) {
				let path = &self.shot.as_ref().unwrap().0;
				// RAC_SHOT_SEQ=N (dev): N consecutive captured frames, path_00.png .. (flicker checks); then quit.
				static SEQ: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
				let seq = std::env::var("RAC_SHOT_SEQ").ok().and_then(|v| v.parse::<u32>().ok()).unwrap_or(1);
				let k = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed); if seq > 1 && std::env::var("RAC_SHOT_STEP").is_ok() { crate::retail::SHOT_ALLOW.store((k + 1) / test_clock::sub(), std::sync::atomic::Ordering::Relaxed); } taken = true;
				let path = if seq > 1 { path.replace(".png", &format!("_{k:02}.png")) } else { path.clone() };
				match std::fs::write(&path, crate::render::png(&img)) {
					Ok(()) => eprintln!("saved {path} at play tick {} field {}", self.play_ticks, self.retail.as_ref().map_or(0, |r| r.field_count())),
					Err(e) => eprintln!("{path}: {e}"),
				}
				if k + 1 >= seq {
					crate::texpack::discover::finish(); el.exit(); // [PC] DEV RAC_TEXNAMES: final write
				}
			}
			// RAC_SHOT_TICK: hold the capture until that many play ticks have run.
			let wait = std::env::var("RAC_SHOT_TICK").ok().and_then(|v| v.parse::<u32>().ok()).is_some_and(|t| self.play_ticks < t) && !paused // play ticks stop while paused
				|| self.retail.as_ref().is_some_and(|r| r.travelling() || r.shot_field_wait());
			if let Some(s) = &mut self.shot {
				s.1 = if wait { s.1.max(6) } else { s.1.saturating_sub(1) };
			}
			test_clock::sub_next(self.shot.is_some(), taken); profile::frame_end(self.smooth.ticks);
			if !test_clock::unpaced() { // [PC] TEST ONLY: RAC_TEST_UNPACED has no frame cap (viewer/test_clock.rs).
				self.pace.wait(&g.window, g.config.present_mode == wgpu::PresentMode::Fifo, self.retail.as_ref().is_some_and(|r| r.lockstep()));
			}
			g.window.request_redraw();
		}
	}
}
