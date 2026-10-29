//! RedrawRequested: the HUD phase (HUD state of the frame, HUD and pause menu draw).

use super::*;

impl App {
	pub(super) fn frame_hud_state(&mut self, f: &mut Frame) {
		let dt = f.dt;
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
			let p = &game.player;
			let title = format!("{} - {} - bolts {} - health {}/4{}", crate::app_name(), toc::LEVEL_NAMES[self.level], game.bolts, game.health.max(0), crate::retail::debugkeys::title_suffix()) + &crate::retail::devtools::title_suffix(self.level, game.player.pos);
			self.shown_bolts = if (game.bolts as f32) < self.shown_bolts { game.bolts as f32 } else { (self.shown_bolts + dt * 60.0f32.max((game.bolts as f32 - self.shown_bolts) * 4.0)).min(game.bolts as f32) };
			// NOV-30f nano request latch (compares against the previous frame's health before `hud_seen` updates).
			hud::nano_request(&mut self.nano_live, &mut self.nano_hud, game.shop.is_some(), game.health, self.hud_seen.1);
			if (game.bolts, game.health) != self.hud_seen {
				if game.bolts != self.hud_seen.0 {
					self.hud_t.0 = 4.8;
				}
				self.hud_seen = (game.bolts, game.health);
			}
			self.hud_t = ((self.hud_t.0 - dt).max(0.0), (self.hud_t.1 - dt).max(0.0));
			// RAC_HUD_ALL: keep the bolt and nanotech counters up (HUD layout shots).
			if std::env::var_os("RAC_HUD_ALL").is_some() {
				self.hud_t = (4.8, 4.8);
				self.nano_hud = (100, 8, 8, game.health.clamp(0, 4));
			}
			// Nanotech meter: whole 50 Hz ticks off the frame dt (hud.md UI-7), not seconds.
			self.nano_acc += dt / 0.02;
			while self.nano_acc >= 1.0 {
				self.nano_acc -= 1.0;
				hud::nano_tick(&mut self.nano_live, &mut self.nano_hud, game.shop.is_some(), game.health);
			}
			// Kind-1 (buy) records show the bolt counter (novalis_flow.md: FUN_0024a900(2, 0x754e, ..)).
			if matches!(game.prompt, Some(1011 | 1012)) {
				self.hud_t.0 = self.hud_t.0.max(4.8);
			}
			// NOV-30f: the shop's bolt counter (slot 2) owns the bolt HUD while committed; one alpha = f1 / 8 [I].
			let mut bolt_alpha = (self.hud_t.0 / 1.8).min(1.0);
			if game.shop_hud.bolts.active == Some(crate::game::vendor::BOLTS) {
				self.hud_t.0 = 0.0;
				bolt_alpha = game.shop_hud.bolts.f1 as f32 / 8.0;
			}
			let eq = game.inv.equipped;
			// Ammo HUD fade (hud.md "Ammo HUD", update 0x23c9a0): shown = on foot with a weapon whose max
			// ammo != 0. f1 (bar, icon) then f2 (text) step 1/8 per tick; hiding drops f2 then f1, and a
			// weapon change always fades out and back in. Ticks taken as 50 Hz frames [I]; the short
			// hide delay (timer < fead0(5)) is not modelled [P].
			let want = crate::inventory::info(eq).filter(|i| i.max > 0).map(|_| eq);
			let step = dt * 50.0 / 8.0;
			let ah = &mut self.ammo_hud;
			if want.is_some() && ah.0 == want {
				if ah.1 < 1.0 {
					ah.1 = (ah.1 + step).min(1.0);
				} else {
					ah.2 = (ah.2 + step).min(1.0);
				}
			} else if ah.2 > 0.0 {
				ah.2 = (ah.2 - step).max(0.0);
			} else if ah.1 > 0.0 {
				ah.1 = (ah.1 - step).max(0.0);
			} else {
				ah.0 = want;
			}
			let q = |f: f32| (f * 8.0).floor() / 8.0;
			let (shown, f1, f2) = (ah.0, q(ah.1), q(ah.2));
			let mut ammo = shown.and_then(|id| crate::inventory::info(id).map(|i| (id, game.inv.ammo(id), i.max, f1, f2))).filter(|a| a.3 > 0.0);
			// NOV-30f: a shop ammo counter in slot 0 draws instead (same draw); the NORMAL handover restarts the
			// on-foot model from empty once it commits.
			let slot = &game.shop_hud.ammo;
			if slot.shop_owned() {
				self.ammo_hud = (None, 0.0, 0.0);
				let id = slot.active.filter(|&a| a != crate::game::vendor::NORMAL).or(slot.pending.map(|p| p.0)).unwrap_or(0) as u8;
				let max = crate::inventory::info(id).map_or(0, |i| i.max);
				ammo = Some((id, slot.display as u16, max, slot.f1 as f32 / 8.0, slot.f2 as f32 / 8.0)).filter(|_| slot.f1 > 0);
			}
			if slot.pending.is_some_and(|p| p.0 == crate::game::vendor::NORMAL) {
				self.ammo_hud = (None, 0.0, 0.0);
				ammo = None;
			}
			let reticle = game.lock.filter(|_| eq == crate::inventory::BOMB_GLOVE).and_then(|l| {
				let g = self.gpu.as_ref()?;
				project(&view_proj(&self.cam, g.config.width as f32 / g.config.height.max(1) as f32, RETAIL_NEAR, RETAIL_FAR), l)
			});
			// WPN-6.2g2 / BRIDGE-39: the screen-sprite queue with centres in window pixels; hero state 0x72 empties it.
			let sprites = self.gpu.as_ref().map_or(Vec::new(), |g| {
				let vp = view_proj(&self.cam, g.config.width as f32 / g.config.height.max(1) as f32, RETAIL_NEAR, RETAIL_FAR);
				queue_sprites(&game.sprite_queue, game.player.retail_ids().0, &vp, g.config.width as f32, g.config.height as f32)
			});
			let st = HudState { health: game.health.max(0), bolts: self.shown_bolts as u32, clock: self.clock, alpha: (bolt_alpha, 0.0), nano: (self.nano_hud.1, self.nano_hud.2), ammo, reticle, rc_static: game.rc.alpha.map(|a| (a, game.rc.tiles.clone())), marker: (game.marker_hud.a, game.marker_hud.b, game.frames), breath: game.breath_hud.snapshot(), counters: [game.count_hud[0].snapshot(), game.count_hud[1].snapshot()], sprites, pad: game.pad_hud.clone() };
			f.title = title; f.st = Some(st);
		}
	}

	pub(super) fn frame_hud_draw(&mut self, f: &mut Frame) {
		let paused = f.paused;
		// No HUD state = no game or world (cold-boot front end, title): frame_hud_state skipped, so the draw skips too.
		let Some(st) = f.st.take() else { return };
		if let (Some(game), Some(w)) = (&mut self.game, &self.world) {
			if let Some(g) = &mut self.gpu {
				// The button glyphs style or the bindings changed (the Graphics page's Button glyphs row, the Controls page): the
				// pictures in the font sheet's ten button cells are the ones of the buttons the functions are played by now, so
				// the sheet is written from the one decoded off the disc and goes up again (xglyphs.rs).
				let (style, map) = (crate::retail::xglyphs::style(), crate::retail::binds::pad_map());
				if (style, map) != (self.glyph_style, self.glyph_map) {
					self.glyph_style = style;
					self.glyph_map = map;
					if let (Some(slot), Some(orig), Some(tex)) =
						(self.art.font.as_ref().map(|f| f.0), self.art.font_tex_orig.as_ref(), self.art.font_tex.as_mut())
					{
						if crate::retail::xglyphs::blit(orig, tex, style, &map) {
							g.hud_texture(Some(slot), tex);
						}
					}
				}
				let p = &game.player;
				hud(&mut g.hud, g.config.width as f32, g.config.height as f32, Some(st), &self.messages, self.flow.as_ref(), &self.art, self.settings.subtitles);
				// NOV-28 (water.md): last frame's fog swap goes live, the camera zone fog updates, then the
				// next swap is chosen (underwater fog 0..32 m, intensity 255 -> 48); the tint precedes the fade.
				g.fog_live = self.fog_next;
				g.part_far = self.part_far_next;
				if let Some(lt) = &g.lighting {
					lt.zone_fog(self.cam.pos, &mut self.fog_zone);
				}
				let (tint, under_fog) = self.water.as_ref().map_or(([0x18, 0x30, 0x90, 0x40], [0x18, 0x30, 0x90]), |w| w.submerged());
				self.fog_next = Some(if self.listener.underwater { (under_fog, crate::water::UNDERWATER_FOG) } else { self.fog_zone });
				// NOV-28.2: pool particles end at 64 m underwater, 500 m dry (one frame late, as the fog).
				self.part_far_next = if self.listener.underwater { crate::water::PART_FAR_UNDER } else { crate::water::PART_FAR };
				if self.listener.underwater {
					// NOV-28.3: the target is gamma-space Unorm, so the tint goes as raw GS bytes.
					overlay::rect(&mut g.hud, [0.0, 0.0, g.config.width as f32, g.config.height as f32], crate::water::tint_colour(tint));
				}
				let fade = game.fade().max(self.flow.as_ref().and_then(|f| f.exit_fade_dark()).unwrap_or(0.0));
				if fade > 0.0 {
					overlay::rect(&mut g.hud, [0.0, 0.0, g.config.width as f32, g.config.height as f32], [0.0, 0.0, 0.0, fade]);
				}
				if let Some(t) = &self.travel {
					t.draw(&mut g.hud, g.config.width as f32, g.config.height as f32);
				}
				if let Some(p) = &self.ship_play {
					p.draw(&mut g.hud, g.config.width as f32, g.config.height as f32);
				}
				let (w, h) = (g.config.width as f32, g.config.height as f32);
				if let Some(font) = &self.art.font {
					if let (Some((id, _, alpha)), Some(text)) = (self.popup.filter(|_| !paused), &self.ui_text) {
						popup(&mut g.hud, w, h, &self.art, font, text.get(id), alpha as f32 / 128.0);
					}
					if self.wheel_fade.0 > 0.0 {
						let sel = self.wheel_fade.1;
						let names: Vec<String> = game.inv.quick.iter().map(|&id| if id == 0 { String::new() } else { self.ui_text.as_ref().map_or(String::new(), |t| t.get(crate::inventory::name_text(id)).to_string()) }).collect();
						wheel(&mut g.hud, w, h, &self.art, font, &game.inv, &names, sel, self.wheel_fade.0, self.clock);
					}
				}
				if self.save_icon > 0.0 {
					save_icon(&mut g.hud, w, h, &self.art, self.clock);
				}
				g.preview = None;
				g.preview2 = None;
				g.preview_pair = None;
				g.preview_worn = None;
				// The vendor's shop screen and its spinning item (yaw += 0.05 a tick = 2.5 rad/s).
				g.hud_split = None;
				g.popup_bolt = game.shop_bolt().map(|(pos, rot)| place(pos, rot));
				// NOV-30d: the slot-0 bolt icon of the item target.
				g.icon_bolt = game.shop_icon_bolt().map(|(pos, rot)| place(pos, rot));
				// NOV-35b / BRIDGE-45: the bolts' draw spheres for their metal alpha and env matrix.
				[g.icon_bolt_sphere, g.popup_bolt_sphere] = game.shop_bolt_spheres();
				// BRIDGE-37 (b): the shop portrait actor posed for its target (never in the world pass).
				let portrait = game.shop.as_ref().and_then(|s| s.portrait.as_ref()).and_then(|p| {
					let (class, a) = self.prop_anims.get(&game::vendor::PORTRAIT_CLASS)?;
					Some((anim::skin(class, &p.anim.pose(a, &[])), p.pos, p.rot))
				});
				g.pose_portrait(portrait.as_ref().map(|(p, pos, rot)| (p, *pos, *rot)));
				if let (Some(sh), Some(font), Some(text)) = (&game.shop, &self.art.font, &self.ui_text) {
					// The six screens' corner joints (0,1,2, 4,5,6, .. 20,21,22) of the vendor model, projected (shop.rs).
					let joints: Option<Vec<[f32; 3]>> = game.props.iter().enumerate().find(|(_, p)| p.class_id == 11 && p.shown).and_then(|(pi, p)| {
						let (_, a) = self.prop_anims.get(&11)?;
						// NOV-30c: the icon bar corners (panel 5) narrowed by the shop's joint records.
						let mats = p.anim.pose_fx_full(a, &[], &game.shop_bar_fx(pi, &a.slot_map));
						let m = place(p.pos, p.rot);
						Some((0..36).map(|j| {
							let jm = a.joint_world(&mats, j.min(mats.len() - 1));
							let l = [jm[3] * p.scale, jm[7] * p.scale, jm[11] * p.scale];
							[0, 1, 2].map(|i| m[i] * l[0] + m[4 + i] * l[1] + m[8 + i] * l[2] + m[12 + i])
						}).collect())
					});
					let vp = view_proj(&self.cam, w / h, RETAIL_NEAR, RETAIL_FAR);
					let rects = joints.map(|j| crate::shop::screen_rects(&j, &|p| project(&vp, p).map(|c| [(c[0] + 1.0) * 0.5 * w, (1.0 - c[1]) * 0.5 * h])));
					let (items, ammo) = (&self.art.items, &self.art.ammo_items);
					let icon = |id: u8, a: bool| if a { ammo.get(&id) } else { items.get(&id) }.map(|i| i.0);
					// UI-5d / NOV-35a: the purchase prompt's attach points 0 / 1 / 2 (FUN_002b3370 list 0x1610f8 through
					// FUN_002641c8; class 1137: joints 2 / 1 / 4), with the same transform as the vendor's, projected to the popup rect.
					let popup = game.props.iter().find(|p| p.class_id == game::vendor::PROMPT_CLASS && p.shown).and_then(|p| {
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
						crate::shop::popup_rect(&j, &|p| project(&vp, p).map(|c| [(c[0] + 1.0) * 0.5 * w, (1.0 - c[1]) * 0.5 * h]))
					});
					g.hud_split = crate::shop::draw(&mut g.hud, w, h, self.art.font2.as_ref().unwrap_or(font), text, sh, game.bolts, &game.inv, rects, &icon, Some(ITEM_PANEL_TEX), Some(PORTRAIT_PANEL_TEX), self.art.frame.map(|f| [f[1], f[2], f[3]]), self.art.led, popup, Some(POPUP_PANEL_TEX));
				}
				// NOV-13.5: the gadget cinematic's black (fanfare) and its end fade.
				if let Some(f) = self.gadget_frame.filter(|f| f.fade > 0.0) {
					overlay::rect(&mut g.hud, [0.0, 0.0, w, h], [0.0, 0.0, 0.0, f.fade]);
				}
				// NOV-13.3: the shop item models in the world (static slot and turntable slot).
				(g.shop_models, g.shop_panel_models) = split_item_poses(&game.shop_item_poses());
if let (Some(m), Some(a), Some(text)) = (&self.pause, &self.ui_art, &self.ui_text) {
					// The frozen world shows behind the menu under FUN_001f9fd0(0,0,0,0x30): black at 0x30/0x80
					// (retail PCSX2 captures; decomp/notes/ingame_pause_menu.md). The HUD is hidden.
					g.hud.clear();
					g.hud_split = None;
					// The button glyphs style or the button bindings changed: the artwork goes into the font sheet's ten
					// button cells and the sheet goes up again.
					let (style, map) = (crate::retail::xglyphs::style(), crate::retail::binds::pad_map());
					if (style, map) != (self.glyph_style, self.glyph_map) {
						self.glyph_style = style;
						self.glyph_map = map;
						if let (Some(slot), Some(orig), Some(tex)) =
							(self.art.font.as_ref().map(|f| f.0), self.art.font_tex_orig.as_ref(), self.art.font_tex.as_mut())
						{
							if crate::retail::xglyphs::blit(orig, tex, style, &map) {
								g.hud_texture(Some(slot), tex);
							}
						}
					}
					overlay::rect(&mut g.hud, [0.0, 0.0, w, h], [0.0, 0.0, 0.0, 0.375]);
					m.draw(&mut g.hud, w, h, a, text);
// Selected item model in panel 3 (frontend.rs preview; turntable camera [P]).
let (pk, pox) = (h / 448.0, (w - 512.0 * h / 448.0) / 2.0);
let gadgets_page = m.page == frontend::Page::Gadgets;
g.preview = m.preview().and_then(|(id, r, t)| {
// Gadgets: the cursor item's class (0x1df for 0x18), drawn only when owned; the other pages keep the held class.
let class = if gadgets_page { crate::inventory::panel_class(id).filter(|_| m.gadget_owned(id))? } else { crate::inventory::info(id)?.held_class };
Some((class, [pox + r[0] * pk, r[1] * pk, r[2] * pk, r[3] * pk], t, Some(id)))
});
g.preview_gold = g.preview.and_then(|p| p.3).is_some_and(|id| gold_lit(id, &m.ingame.golds));
// Category-3 ids also draw class 601 with the same placement.
g.preview_pair = g.preview.filter(|p| gadgets_page && p.3.is_some_and(|id| crate::inventory::category(id) == Some(3))).map(|p| (601, p.1, p.2, p.3));
// Panel models play sequence 1 (6 for item 2), clamped to the class's sequence count.
if self.panel_model.map(|p| p.0) != g.preview.map(|p| p.0) {
self.panel_model = g.preview.map(|p| (p.0, self.clock));
}
let panel_start = self.panel_model.map_or(self.clock, |p| p.1);
for (class, id) in [g.preview, g.preview_pair].into_iter().flatten().filter_map(|p| Some((p.0, p.3?))) {
if let Some((c, ia)) = self.menu_actor_classes.get(&class).filter(|(_, ia)| !ia.sequences.is_empty()) {
let seq = (if id == 2 { 6 } else { 1 }).min(ia.sequences.len() - 1);
let (ps, ns) = anim::skin(c, &ia.pose_after(seq, (self.clock - panel_start) * 50.0));
g.set_preview_pose(class, &ps, &ns);
}
}
g.preview2 = m.hero_preview().map(|(id, r)| {
let key = if gpu_has_preview(&g.scene, HERO_PREVIEW + id as u32) { HERO_PREVIEW + id as u32 } else { HERO_PREVIEW };
(key, [pox + r[0] * pk, r[1] * pk, r[2] * pk, r[3] * pk], 0.0)
}).or_else(|| m.gold_preview().map(|r| (GOLD_BOLT_PREVIEW, [pox + r[0] * pk, r[1] * pk, r[2] * pk, r[3] * pk], self.clock)));
// The hero plays the selected item's menu clip (table 0x1b93f0), the item following its joint.
match (m.hero_preview(), &self.hero_anim, self.menu_clip_base) {
(Some((id, _)), Some((hc, ha)), Some(base)) => {
if self.hero_preview_clip.as_ref().map(|c| c.id) != Some(id) {
	// Entering a clip: fade from the current pose (FUN_002575d8(hero, clip, 0, 10, 5)).
	let mut pa = self.hero_preview_clip.take().map_or_else(|| anim::PropAnim::new(IDLE), |m| m.pa);
	if !pa.attached() {
		pa.attach(Some(anim::share_timing(&ha.sequences)));
	}
	pa.blend_ex(menu_clip(base, id).map_or(IDLE, |c| c.0), 0, 10, 5);
	// The item's own clock: its requested itemseq (hero record +0x10) at frame 0, no blend.
	let item = self.hero_items.get(&id).and_then(|it| self.menu_actor_classes.get(&it.class)).map(|(_, ia)| {
		let mut ipa = anim::PropAnim::new(MENU_CLIP_ROWS.get(id as usize).map_or(1, |r| r.2 as usize));
		ipa.attach(Some(anim::share_timing(&ia.sequences)));
		ipa.set(ipa.seq, 0);
		ipa
	});
	self.hero_preview_clip = Some(MenuHero { id, pa, item, start: self.clock, ticks: 0, plays: 0, second: false });
}
let worn = game.inv.worn;
let feet = worn[1] != 0;
let ticks_before = self.hero_preview_clip.as_ref().map_or(0, |m| m.ticks);
let (mats, start, ticks_now) = {
	let m = self.hero_preview_clip.as_mut().expect("preview clip set above");
	let target = ((self.clock - m.start).max(0.0) * 50.0) as u32;
	while m.ticks < target {
		m.ticks += 1;
		m.pa.step();
		// The item's anim ended on an itemseq other than 1: back to sequence 1, frame 0, no blend.
		if let Some(ipa) = &mut m.item {
			ipa.step();
			if ipa.wrapped && ipa.seq != 1 {
				ipa.set(1, 0);
			}
		}
		// The clip plays `loops` times, then clip2 fades in and loops.
		if let Some((_, loops, clip2)) = menu_clip(base, id).filter(|c| c.1 > 0 && !m.second) {
			if m.pa.wrapped && !m.pa.fading() {
				m.plays += 1;
				if m.plays >= loops as u32 {
					m.second = true;
					m.pa.blend_ex(clip2, 0, 10, 5);
				}
			}
		}
	}
	// Boots worn: the hero's own foot joints (attach points 0x16 / 0x17) collapse under them.
	let scales: Vec<(usize, [f32; 3])> = if feet { self.hero_worn.scale_joints.iter().flatten().map(|&j| (j, [0.0; 3])).collect() } else { Vec::new() };
	(m.pa.pose_scaled(ha, &scales), m.start, m.ticks)
};
let hero_local = self.hero_preview_clip.as_ref().map(|m| m.pa.local_trs(ha)).unwrap_or_default();
let (mut ps, mut ns) = anim::skin(hc, &mats);
let key = if gpu_has_preview(&g.scene, HERO_PREVIEW + id as u32) { HERO_PREVIEW + id as u32 } else { HERO_PREVIEW };
if let Some(it) = self.hero_items.get(&id).filter(|_| key != HERO_PREVIEW) {
	let mut wj = ha.joint_world(&mats, it.joint);
	if it.mode != 0 {
		wj = normalise_columns(wj);
	}
	// Hand (category 0) and head (category 2) items with their own skeleton are skinned every tick:
	// mode 0 from the pose-copy frame of the hero's live joints, otherwise from their itemseq clock.
	let cat = crate::inventory::category(id);
	let own = self.menu_actor_classes.get(&it.class).filter(|(c, _)| matches!(cat, Some(0) | Some(2)) && c.mesh.positions.len() == it.mesh.positions.len());
	let (ip, inn) = match (own, self.hero_preview_clip.as_ref().and_then(|m| m.item.as_ref())) {
		// Feet and back items are drawn by the worn batch (UI-3b); their slot in this mesh collapses.
		_ if matches!(cat, Some(1) | Some(3)) => (vec![[0.0; 3]; it.mesh.positions.len()], vec![[0.0, 0.0, 1.0]; it.mesh.positions.len()]),
		(Some((c, ia)), Some(ipa)) => {
			let mats_i = if it.mode == 0 && !hero_local.is_empty() {
				let f = match (cat, it.class) {
					(Some(2), 433) => anim::pose_copy_frame(&hero_local, &anim::HEAD_COPY_433, anim::HEAD_COPY_433_EXTRA, &ia.skeleton.bind_translation),
					(Some(2), _) => anim::pose_copy_frame(&hero_local, &anim::HEAD_COPY, 0, &ia.skeleton.bind_translation),
					_ => anim::pose_copy_frame(&hero_local, &anim::HAND_COPY, 0, &ia.skeleton.bind_translation),
				};
				ia.pose_single(&f)
			} else {
				ipa.pose(ia, &[])
			};
			anim::skin(c, &mats_i)
		}
		_ => (it.mesh.positions.clone(), face_normals(&it.mesh)),
	};
	ps.extend(ip.iter().map(|&v| apply(&wj, v, 1.0)));
	ns.extend(inn.into_iter().map(|n| apply(&wj, n, 0.0)));
}
// Extra actors start their sequence when the item is selected (trigger 0) and loop [I].
for &(class, seq, scale) in self.hero_actors.get(&id).filter(|_| key != HERO_PREVIEW).into_iter().flatten() {
	if let Some((c, a)) = self.menu_actor_classes.get(&class) {
		let (p, n) = anim::skin(c, &a.pose_after(seq, (self.clock - start) * 50.0));
		ps.extend(p.into_iter().map(|v| v.map(|x| x * scale)));
		ns.extend(n);
	}
}
g.set_preview_pose(key, &ps, &ns);
// UI-3b: the worn mobies. The back item's clock steps with the hero clip; a changed back item respawns it.
let back_class = (worn[3] != 0).then(|| crate::inventory::preview_class(worn[3])).flatten();
if self.worn_back.as_ref().map(|b| b.class) != back_class {
	self.worn_back = back_class.and_then(|c| self.menu_actor_classes.get(&c)).map(|(_, ia)| anim::BackAnim::new(back_class.unwrap_or(0), anim::share_timing(&ia.sequences)));
}
if let Some(b) = &mut self.worn_back {
	for _ in ticks_before..ticks_now {
		b.step();
	}
}
if self.hero_worn.total > 0 {
	let (mut wp, mut wn) = (vec![[0.0f32; 3]; self.hero_worn.total], vec![[0.0, 0.0, 1.0]; self.hero_worn.total]);
	let feet_class = crate::inventory::preview_class(worn[1]).filter(|_| feet);
	for part in &self.hero_worn.parts {
		let on = match part.kind {
			WornKind::BootA | WornKind::BootB => feet_class == Some(part.class),
			WornKind::Back => back_class == Some(part.class),
			WornKind::Extra(item) => game.inv.have.get(item as usize).copied().unwrap_or(false),
		};
		if !on {
			continue;
		}
		let wj = normalise_columns(ha.joint_world(&mats, part.joint));
		let own = self.menu_actor_classes.get(&part.class).filter(|(c, _)| c.mesh.positions.len() == part.mesh.positions.len());
		let (ip, inn) = match (own, part.kind) {
			(Some((c, ia)), WornKind::BootA | WornKind::BootB) if !hero_local.is_empty() => {
				let list = &BOOT_COPY[(part.kind == WornKind::BootB) as usize];
				anim::skin(c, &ia.pose_single(&anim::pose_copy_frame(&hero_local, list, 0, &ia.skeleton.bind_translation)))
			}
			(Some((c, ia)), WornKind::Back) => match &self.worn_back {
				Some(b) => anim::skin(c, &b.pa.pose(ia, &[])),
				None => anim::skin(c, &ia.pose_after(1, 0.0)),
			},
			(Some((c, ia)), _) => anim::skin(c, &ia.pose_after(0, (self.clock - start) * 50.0)),
			_ => (part.mesh.positions.clone(), face_normals(&part.mesh)),
		};
		for (k, (p, n)) in ip.iter().zip(&inn).enumerate() {
			wp[part.range.start + k] = apply(&wj, *p, 1.0);
			wn[part.range.start + k] = apply(&wj, *n, 0.0);
		}
	}
	g.set_preview_pose(HERO_WORN, &wp, &wn);
	g.preview_worn = g.preview2.map(|(_, r, t)| (HERO_WORN, r, t, None));
}
}
_ => {
self.hero_preview_clip = None;
self.worn_back = None;
g.preview_worn = None;
}
}
					if m.page == frontend::Page::Map {
						if let Some(mv) = &self.map {
							map_view(&mut g.hud, w, h, mv, self.art.marker, self.clock);
						}
					}
				}
				profile::lap(profile::Phase::Hud);
			}
		}
	}
}
