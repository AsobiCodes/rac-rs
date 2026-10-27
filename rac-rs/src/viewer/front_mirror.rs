//! The retail front end (frontbin: title, main menu, New / Load Game) drawn by the mirror renderer instead of the GS picture.
//!
//! The game's own code runs the front end in the interpreter. Everything shown comes from its RAM (`retail::FrontRam`,
//! `mouse_menu::MenuSnap`): the title scene's timeline tick, the logo and Press Start alphas, the black fade, the page, the
//! selected row and the save-slot list. frontend.rs only supplies the art, the layout and the drawing (`Menu::follow`); its own
//! key handling and clocks decide nothing. The picture is the title scene (scene.rs `title_scene`, level.rs `Level::title`) drawn
//! like a level (widescreen, high resolution); the 2D art is centred at 4:3 like the HUD, which is also the transform the mouse
//! uses (`mouse_menu::to_ps2` without a GS picture). RAC_FRONT_GS=1 shows the GS's own picture again (debug fallback).
use super::*;
use crate::retail::mouse_menu::{Kind, MenuSnap};

/// Page structs of the front end (frontend_flow.md section 2): root, New Game, Load Game, Options, Language, Video, Sound.
const P_ROOT: u32 = 0x1d4548;
const P_NEW: u32 = 0x1d4c08;
/// New Game's overwrite confirm (FUN_002230b8: Cross on a used slot sets the next page 0x1d4db8, id 0x1a): title, slot list,
/// text 0x4fb3 panel (FUN_00221650), Circle Yes / Triangle No labels.
const P_OVERWRITE: u32 = 0x1d4db8;
const P_LOAD: u32 = 0x1d4b80;
const P_OPTIONS: [u32; 2] = [0x1d4708, 0x1d47c0];
const P_LANGUAGE: u32 = 0x1d48c0;
const P_VIDEO: u32 = 0x1d4a38;
const P_SOUND: u32 = 0x1d4838;

fn page_of(m: Option<&MenuSnap>) -> Option<frontend::Page> {
	use frontend::Page::*;
	let Some(m) = m else { return Some(Title) };
	// The PC Graphics page lives in reserved RAM (retail/pcext.rs `title_options`), the same address on both discs.
	if m.page == crate::retail::pcext::TGPAGE {
		return Some(Graphics);
	}
	Some(match crate::retail::disc::unva(m.page) {
		P_ROOT => Root,
		P_NEW => NewGame,
		P_OVERWRITE => Overwrite,
		P_LOAD => LoadGame,
		p if P_OPTIONS.contains(&p) => Options,
		P_LANGUAGE => Language,
		P_VIDEO => Video,
		P_SOUND => Sound,
		_ => return None,
	})
}

/// The selected row of the page from retail's current item and that item's own cursor.
fn sel_of(m: &MenuSnap, page: frontend::Page, slot_cur: [u32; 2]) -> usize {
	let cur = m.items.iter().find(|i| i.addr == m.cur);
	let cursor = |it: &crate::retail::mouse_menu::Item| match it.kind {
		Kind::Label { cursor, .. } | Kind::Value { cursor, .. } | Kind::Grid { cursor, .. } | Kind::Ring { cursor } => cursor as usize,
		_ => 0,
	};
	match page {
		// One item (panel) per row.
		frontend::Page::Root => {
			let mut v: Vec<_> = m.items.iter().filter(|i| i.rect[2] > 0).collect();
			v.sort_by_key(|i| i.slot);
			v.iter().position(|i| i.addr == m.cur).unwrap_or(0)
		}
		// The slot list item (cursor = slot), and New Game's "continue without saving" panel as the row after the slots.
		frontend::Page::NewGame | frontend::Page::LoadGame => match cur {
			Some(it) if it.rect[3] < 100 && page == frontend::Page::NewGame && it.rect[1] > 200 => crate::save::SLOTS,
			Some(_) => (slot_cur[(page == frontend::Page::NewGame) as usize] as usize).min(crate::save::SLOTS - 1),
			None => 0,
		},
		// The confirm keeps New Game's slot list (item 0x1d4ce8) and its cursor on screen.
		frontend::Page::Overwrite => (slot_cur[1] as usize).min(crate::save::SLOTS - 1),
		_ => cur.map_or(0, cursor),
	}
}

/// The Language page's five languages, in the order the game lists them: the page's list item holds the text id of each
/// language, and the game's cursor counts the rows in that order. The disc's ids (`LANGUAGES + 0..4`) are in the game's own
/// order, which puts German and Spanish the other way round than the console's list the game follows, so the rows the player
/// sees have to be the game's own. Empty when the page has no such item.
fn language_rows(items: &[crate::retail::mouse_menu::Item]) -> Vec<u32> {
	const FIRST: u16 = 0x4f2c;
	const LAST: u16 = 0x4f30;
	items
		.iter()
		.find(|i| !i.texts.is_empty() && i.texts.iter().all(|&t| (FIRST..=LAST).contains(&t)))
		.map(|i| i.texts.iter().map(|&t| t as u32).collect())
		.unwrap_or_default()
}

/// Wall time of one redraw the front end may spend on back-to-back load-loop fields.
const TURBO_BUDGET: std::time::Duration = std::time::Duration::from_millis(40);

impl App {

	/// The retail front end runs and rac-rs still shows a level: load the title scene (scene, art, text) in its place once, keeping the
	/// running `Retail`. `load_level` with `want_front` is the same path the old Quit Game used.
	pub(super) fn front_mirror_enter(&mut self) {
		if self.front.is_some() || !self.retail.as_ref().is_some_and(|r| r.front_ram().is_some()) {
			return;
		}
		// Dev only: RAC_DEV_MIRROR_DELAY_MS holds the set-up back that long after the front end first shows in RAM, which
		// is what a slow start-up does by itself (the game asks for its logo movie at field 4, before the mirror is up).
		if let Some(ms) = std::env::var("RAC_DEV_MIRROR_DELAY_MS").ok().and_then(|v| v.parse::<u64>().ok()) {
			static SEEN: std::sync::OnceLock<std::time::Instant> = std::sync::OnceLock::new();
			if SEEN.get_or_init(std::time::Instant::now).elapsed() < std::time::Duration::from_millis(ms) {
				return;
			}
		}
		let carry = self.retail.take();
		self.want_front = true;
		self.load_level();
		self.want_front = false;
		self.retail = carry;
		// A movie in flight (the game thread is parked in the player hook) goes on: the mirror services it from here on.
		// Dropping it would leave the game waiting for an end nobody reports.
		if !(self.movie.is_some() && crate::retail::fmv_bridge::waiting()) {
			self.movie = None;
			if let Some(g) = &mut self.gpu {
				g.movie.clear();
			}
		}
		(self.fm_ov, self.fm_movie_hold) = (false, false);
		eprintln!("retail: front end drawn by the mirror (title scene {})", self.scenes.contains_key(&u32::MAX));
	}

	/// The retail fields of the front end (no `Game` runs there): keys in, sound and movies out, one per 50 Hz step.
	pub(super) fn front_mirror_tick(&mut self, dt: f32) {
		let (Some(r), Some(fe)) = (self.retail.as_mut(), self.front.as_mut()) else { return };
		fe.acc = (fe.acc + dt).min(crate::retail::disc::field_dt2());
		// The front end's level-load loop (retail.rs `in_load_loop`) takes no pace from the wall clock: fields run back to back for
		// up to `TURBO_BUDGET` of this frame (the window stays alive), the same fields as at 50 Hz.
		let t0 = std::time::Instant::now();
		while fe.acc >= crate::retail::disc::field_dt() || (r.turbo() && t0.elapsed() < TURBO_BUDGET) {
			fe.acc = (fe.acc - crate::retail::disc::field_dt()).max(0.0);
			// --shot (RAC_SHOT_TICK) counts the front end's fields as play ticks.
			self.play_ticks += 1;
			let keys = &self.keys;
			r.tick_keys(|k| keys.contains(&k), false, false);
			// The attract movie and the New Game intro (retail/fmv_bridge.rs): streamed from the disc, skipped like retail's (any pad press).
			if let Some(q) = r.take_movie() {
				let m = scene::Movie { lba: q.lba, bytes: q.bytes as u64, fps: crate::retail::disc::movie_fps(q.lba) };
				let lang = [0u32, 2, 3, 4, 5].iter().position(|&t| t == q.lang).unwrap_or(0) as u8;
				eprintln!("retail: front end movie lba {} bytes {} lang {}", q.lba, q.bytes, q.lang);
				if let Some(a) = &mut self.audio {
					a.pause_all(true);
				}
				// A new movie ends a held last picture's hold (as handler.rs's MOVIE_HOLD does; its first frame resets the tint).
				self.fm_movie_hold = false;
				self.movie = Some(fmv::player::Player::start(&self.iso_path, &m, lang, self.audio.as_mut()));
			}
			if let Some(g) = &self.gpu {
				r.aspect = g.config.width as f32 / g.config.height.max(1) as f32;
			}
			// A movie ended and the game fades its last picture out (retail draws the fade into the display buffer over the
			// movie): the held picture takes the fade's tint until the next picture (a caption) covers it. Mirrors
			// handler.rs's MOVIE_HOLD; before the overlay is taken below.
			// Only the New Game intro (mode 6) keeps a movie's last picture under the game's fades: in any other front-end mode
			// the mirror's own scene replaces it at once (the logo movie before the title, the attract movie).
			if let (true, Some(g)) = (self.fm_movie_hold, self.gpu.as_mut()) {
				if let Some(t) = r.movie_tint() {
					g.movie.tint = t;
				}
				if r.overlay_covers() || !r.front_ram().is_some_and(|f| f.dialog.mode == 6) {
					g.movie.clear();
					self.fm_movie_hold = false;
				}
			}
			let ev = r.take_sound();
			if let Some(a) = &mut self.audio {
				crate::retail::snd::field(a, ev);
			}
			if let (Some(e), Some(g)) = (r.take_stop(), &self.gpu) {
				g.window.set_title(&format!("{}: RETAIL STOPPED: {e}", crate::app_name()));
			}
		}
		if let Some(p) = &mut self.movie {
			if let (Some(f), Some(g)) = (p.poll(), &mut self.gpu) {
				g.movie.upload(&g.device, &g.queue, &f);
			}
		}
		if self.movie.is_some() && crate::retail::fmv_bridge::waiting() {
			// Retail's rule (fmv_bridge `Skip`): the intro movies end on a newly pressed Start (Esc / Enter; Space feeds Start too).
			let keys = &self.keys;
			if let Some(r) = self.retail.as_mut() {
				let skip = r.movie_pressed(|k| keys.contains(&k));
				if skip || self.movie.as_ref().is_some_and(|p| p.finished) {
					self.movie = None;
					if let Some(a) = &mut self.audio {
						a.pause_all(false);
					}
					// The intro's movie keeps its last picture up under the fade that follows (g.movie keeps it; see the hold
					// above); the logo and attract movies leave the title scene to show at once (it was black for good before).
					if r.front_ram().is_some_and(|f| f.dialog.mode == 6) {
						self.fm_movie_hold = true;
					} else if let Some(g) = &mut self.gpu {
						g.movie.clear();
					}
					eprintln!("retail: front end movie {} at field {}", if skip { "skipped" } else { "finished" }, r.field_count());
					r.end_movie();
				}
			}
		}
		// Mode 6 (the New Game intro: caption screens and the fades around the movies): the picture is the capture's display
		// buffer, drawn by `front_mirror_frame`; any other mode is the mirror's own scene.
		if let Some(r) = self.retail.as_mut() {
			if r.front_ram().is_some_and(|f| f.dialog.mode == 6) {
				if let (Some((w, rows, px)), Some(g)) = (r.take_overlay(), self.gpu.as_mut()) {
					g.set_retail_overlay(w, rows, &px);
					self.fm_ov = true;
				}
			} else {
				r.take_overlay();
				self.fm_ov = false;
			}
		}
	}

	/// One drawn frame of the front end: the title scene at retail's tick, the menu from retail's page, the black fade.
	pub(super) fn front_mirror_frame(&mut self, dt: f32) {
		let Some(r) = self.retail.as_ref() else { return };
		let (Some(fr), Some(fe)) = (r.front_ram().cloned(), self.front.as_mut()) else { return };
		let snap = r.front_menu().cloned();
		let dump = std::env::var_os("RAC_FRONT_DUMP").is_some();
		// RAC_FRONT_DLG=1: the message box's words, once per change (mode 4).
		if fr.dialog.mode == 4 && std::env::var_os("RAC_FRONT_DLG").is_some() {
			use std::sync::Mutex;
			static LAST: Mutex<Option<(u32, u32, u32, u32, u32)>> = Mutex::new(None);
			let d = &fr.dialog;
			let key = (d.kind, d.state, d.flags, d.card, d.step);
			if LAST.lock().is_ok_and(|mut l| l.replace(key) != Some(key)) {
				eprintln!("front dialog: tick {} {:?}", fr.tick, d);
			}
		}
		if dump && fr.tick % 60 == 0 {
			eprintln!("front: tick {} logo {} press {} fade {:.2} lang {} page {:?}", fr.tick, fr.logo, fr.press, fr.fade, fr.lang, snap.as_ref().map(|m| (format!("{:x}", m.page), m.id, m.state, format!("{:x}", m.cur), m.items.iter().map(|i| (format!("{:x}", i.addr), i.slot, i.rect, i.kind.clone(), i.texts.clone())).collect::<Vec<_>>())));
		}
		// Language of the running game: the Press Start caption and the page strings follow it.
		// Retail's language code (0 En, 2 Fr, 3 De, 4 Es, 5 It) -> all_text slot and Press Start caption: max(code - 1, 0)
		// (slots EN, FR, DE, ES, IT; FUN_001eb090 picks the caption the same way).
		let (lang, press) = { let l = (fr.lang.saturating_sub(1) as usize).min(4); (l, l) };
		fe.menu.press_lang = press;
		if lang != fe.menu.lang {
			match iso::Disc::open(std::path::Path::new(&self.iso_path)).map_err(|e| e.to_string()).and_then(|mut d| frontend::Text::load(&mut d, lang)) {
				Ok(t) => fe.text = t,
				Err(e) => eprintln!("front-end text {lang}: {e}"),
			}
			fe.menu.lang = lang;
		}
		// The PC options' texts (the Graphics row and page) join the language's strings.
		if fe.text.get(crate::retail::pcext::ID_GRAPHICS).is_empty() {
			for (id, s) in crate::retail::pcext::texts(lang) {
				fe.text.insert(id, s);
			}
			// The Controls page's own texts, and the Button glyphs row the title screen's Graphics page shows.
			for (id, s) in crate::retail::pcctl::fixed_texts(lang) {
				fe.text.insert(id, s);
			}
		}
		let slots: [Option<save::Summary>; save::SLOTS] = std::array::from_fn(|i| fr.slots.get(i).copied().flatten().map(|(level, bolts, play_frames, date)| save::Summary { level: level as usize, bolts, play_frames, date }));
		match snap.as_ref().map(|m| (page_of(Some(m)), m)) {
			Some((Some(page), m)) => {
				// The Options rows as the game lists them (its list item, slot 3): PAL Language, Video, Sound; USA Language, Sound;
				// then Graphics (retail/pcext.rs). Kept while the item is not on screen.
				if page == frontend::Page::Options {
					if let Some(it) = m.items.iter().find(|i| i.slot == 3 && !i.texts.is_empty()) {
						fe.menu.front_rows = it.texts.iter().map(|&t| t as u32).collect();
					}
				}
				// The Language page's five languages as the game lists them (its list item): the rows the player sees have to be in
				// the order the game's cursor counts, or the name under the cursor is the wrong language.
				if page == frontend::Page::Language {
					let rows = language_rows(&m.items);
					if !rows.is_empty() {
						fe.menu.lang_rows = rows;
					}
				}
				fe.menu.follow(page, sel_of(m, page, fr.slot_cur), slots)
			}
			Some((None, _)) => {}
			None => fe.menu.follow(frontend::Page::Title, 0, slots),
		}
		fe.menu.ram_alpha = Some((fr.logo as f32, fr.press as f32));
		fe.menu.update(dt);
		// The title scene at retail's timeline tick: camera keyframes and actors.
		let mut actors: Vec<Option<(Posed, [f32; 3])>> = vec![None; self.actor_slots.len()];
		let (mut at, mut skinned) = (None, None);
		if let Some((sc, _)) = self.scenes.get(&u32::MAX) {
			let tick = (fr.tick as f32).min(sc.ticks.max(1) as f32 - 1.0);
			fe.tick = tick;
			if let Some(k) = sc.camera_at(tick) {
				let f = k.forward();
				self.cam.pos = k.pos;
				self.cam.yaw = f[1].atan2(f[0]);
				self.cam.pitch = f[2].clamp(-1.0, 1.0).asin();
				self.cam.tan_half = k.tan_half_fov * PROJ_Y_RATIO;
			}
			if let (Some(actor), Some((class, a))) = (sc.actors.iter().find(|a| a.class_id == 0), &self.hero_anim) {
				if let (Some((ka, kb, t)), Some(pos)) = (actor.pair(tick), actor.pos_at(tick)) {
					let mats = a.pose_frames(&ka.frame, &kb.frame, t);
					let (mut ps, mut ns) = anim::skin(class, &mats);
					// The marker model is Ratchet with the wrench, glove and nozzle appended: a pose must cover every vertex or the
					// whole model stays in its bind pose (the T-pose); the items are hidden like the play path does (`hide`).
					let model = self.gpu.as_ref().and_then(|g| g.marker_len());
					if std::env::var_os("RAC_FRONT_DLG").is_some() {
						static ONCE: std::sync::Once = std::sync::Once::new();
						ONCE.call_once(|| eprintln!("front pose: skinned {} marker_model {:?} keys {} joints {} seqs {} tick {}", ps.len(), model, actor.keys.len(), mats.len(), a.sequences.len(), tick));
					}
					if let Some(n) = model.filter(|&n| n > ps.len()) {
						let missing = n - ps.len();
						hide(&mut ps, &mut ns, missing);
					}
					skinned = Some((ps, ns));
					at = Some((pos, 0.0));
				}
			}
			for (slot, &(sid, index, class_id)) in actors.iter_mut().zip(&self.actor_slots) {
				let (Some(actor), Some((class, a))) = (sc.actors.iter().find(|a| a.index == index).filter(|_| sid == u32::MAX), self.prop_anims.get(&class_id)) else { continue };
				if let (Some((ka, kb, t)), Some(pos)) = (actor.pair(tick), actor.pos_at(tick)) {
					*slot = Some((anim::skin(class, &a.pose_frames(&ka.frame, &kb.frame, t)), pos));
				}
			}
		}
		if let Some(g) = &mut self.gpu {
			g.hud.clear();
			g.hud_split = None;
			// A front-end frame draws no sharp overlay: the sharp HUD target of the last level would make a New Game caption
			// (mode 6, the capture's picture) wait for a plane that is never drawn.
			crate::retail::set_hud_target(0);
			// [PC] Sharp title / main menu (hudsharp.rs): the art quads take the edge-aware sampler on a tall enough window; the
			// noise and scanline sprites of the panel frames keep the nearest look.
			let flat = fe.art.as_ref().and_then(|a| a.frame).map_or([usize::MAX; 2], |f| [f[1].0, f[2].0]);
			super::hudsharp::set(crate::retail::ui_sharp(g.config.height), flat);
			g.movie.black = false;
			let (w, h) = (g.config.width as f32, g.config.height as f32);
			// Mode 6 with the capture's picture up: it replaces the menu and the title's fade (the game's own fade is in it).
			let picture = self.fm_ov && fr.dialog.mode == 6;
			if let (Some(art), None, false) = (&fe.art, &self.movie, picture) {
				// The menu draws at 512x448 square pixels centred; the PS2 picture is 4:3, so the quads are widened about the centre.
				let n0 = g.hud.len();
				fe.menu.draw(&mut g.hud, w, h, art, &fe.text);
				let sx = (4.0 / 3.0 * 448.0) / 512.0;
				for q in &mut g.hud[n0..] {
					q.rect[0] = w * 0.5 + (q.rect[0] - w * 0.5) * sx;
					q.rect[2] *= sx;
				}
			}
			// FUN_001f4e50: a black quad of alpha 128 * DAT_0015f43c over the picture (scene fade in / out around the menu).
			if fr.fade > 0.0 && !picture {
				crate::overlay::rect(&mut g.hud, [0.0, 0.0, w, h], [0.0, 0.0, 0.0, fr.fade.min(1.0)]);
			}
			// Mode 4: the modal message box (memory card messages; FUN_001fb780 draws it after the fade), widened like the menu.
			if let (Some(art), None, 4) = (&fe.art, &self.movie, fr.dialog.mode) {
				let n1 = g.hud.len();
				fe.menu.draw_dialog(&mut g.hud, w, h, art, &fe.text, &fr.dialog);
				let sx = (4.0 / 3.0 * 448.0) / 512.0;
				for q in &mut g.hud[n1..] {
					q.rect[0] = w * 0.5 + (q.rect[0] - w * 0.5) * sx;
					q.rect[2] *= sx;
				}
			}
			// Retail's display buffer (caption strips, fades), 4:3 centred on the window height; the edge columns stretch to the
			// sides like the gameplay overlay (handler.rs).
			if picture && g.has_retail_overlay() {
				let k = (w / h) / (4.0 / 3.0);
				g.hud.push(crate::overlay::Quad { tex: RETAIL_OV_TEX, rect: [0.0, 0.0, w, h], uv: [0.5 - 0.5 * k, 0.0, 0.5 + 0.5 * k, 1.0], colour: [1.0; 4], angle: 0.0, corners: None });
			}
			g.set_clip_view(None);
			g.set_marker(at, skinned.as_ref(), None);
			g.set_props(Some(&[]), &HashMap::new(), &HashMap::new(), &actors, &[], &[], &[], &[]);
			g.set_particles(&[], &[], &[], &[], &self.cam, None);
		}
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::mouse_menu::Item;

	fn item(texts: Vec<u16>) -> Item {
		Item { addr: 1, slot: 3, rect: [0; 4], kind: Kind::Label { rows: Vec::new(), rowh: 0.0, cursor: 0 }, texts }
	}

	/// The Language page's rows are the game's own: the game lists the languages in the console's order (English, French,
	/// Spanish, German, Italian) and the disc's ids are in the game's (English, French, German, Spanish, Italian), so the two
	/// put German and Spanish the other way round. The rows follow the game, or the name under the cursor is the wrong
	/// language.
	#[test]
	fn the_language_page_follows_the_games_own_order() {
		// The game's list item: the five language ids in its own order.
		let rows = language_rows(&[item(vec![0x4f2c, 0x4f2d, 0x4f30, 0x4f2e, 0x4f2f])]);
		assert_eq!(rows, vec![0x4f2c, 0x4f2d, 0x4f30, 0x4f2e, 0x4f2f]);
		// The page has no such item: the rows keep the disc's own order.
		assert!(language_rows(&[]).is_empty());
		assert!(language_rows(&[item(vec![])]).is_empty(), "an item without texts");
		assert!(language_rows(&[item(vec![0x4f2b, 0x4f2c])]).is_empty(), "the Options row is not a language list");
	}
}
