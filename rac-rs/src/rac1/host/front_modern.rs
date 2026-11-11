//! The front end's menu drawn as a modern title page (v0.5 screens stage B) or, in the Classic style, as before. Called by
//! `front_mirror_frame` once per display frame: it decides in that frame whether the modern page is the picture (the hide switch is
//! set before the retail menu's own hide test, so no frame draws both), gathers the facts from the front end's RAM mirror
//! (`title::Facts`), builds the page (`title::build`) and hands the draw list to the engine's UI pass. The logo stays a hud quad
//! at the mockup's box; Press Start (and any page the bridge does not know) keeps the classic picture. See `title_host.rs`.
use super::front_mirror::{page_of, sel_of};
use super::*;
use crate::modern_menu::{host as mhost, style, title, title_host};
use crate::retail::menu_model::{text::decode, view::slot_views, Strings};
use crate::retail::mouse_menu::{MenuSnap, CIRCLE, CROSS, TRIANGLE};
use crate::retail::pcext::quitrow::{ID_QUITD, ID_QUITD_BODY};
use crate::retail::{title_quit, FrontDialog, FrontRam};
use rac_engine::ui::kit::Scale;
use rac_engine::ui::prim::{Prim, Rect, Rgba};
use rac_engine::ui::{self, gpu as ui_gpu};

/// The 4:3 menu of the classic picture is widened about the centre (512x448 square pixels -> the PS2's 4:3).
fn widen(out: &mut [Quad], w: f32) {
	let sx = (4.0 / 3.0 * 448.0) / 512.0;
	for q in out {
		q.rect[0] = w * 0.5 + (q.rect[0] - w * 0.5) * sx;
		q.rect[2] *= sx;
	}
}

/// The pad button a message-box word stands for (`menu_keys::front_dialog`).
fn button_bit(id: u32) -> u16 {
	match id {
		0x524f => CIRCLE,
		0x524b | 0x4fa8 => TRIANGLE,
		_ => CROSS,
	}
}

/// The words of the box over the page: the Overwrite page's question, or front mode 4's memory card message.
fn dialog_facts(page: frontend::Page, d: &FrontDialog, t: &dyn Fn(u32) -> String) -> Option<title::DialogFacts> {
	if page == frontend::Page::Overwrite {
		return Some(title::DialogFacts { lines: vec![t(0x4fb3)], buttons: vec![(CIRCLE, t(0x524f)), (TRIANGLE, t(0x524b))] });
	}
	let dt = frontend::dialog_text(d).filter(|_| d.mode == 4)?;
	let mut lines = vec![t(dt.body[0])];
	if dt.body[1] != 0 {
		lines.extend([String::new(), t(dt.body[1])]);
	}
	let buttons = [dt.yes, dt.no, dt.hint].into_iter().filter(|&id| id != 0).map(|id| (button_bit(id), t(id))).collect();
	Some(title::DialogFacts { lines, buttons })
}

/// The root page's own Quit question (`title_quit`, the host's fourth entry) as the box over the page: Circle = Yes, Triangle = No. The
/// host answers it, so it is no game box (the caller's `boxed` stays false for it). A game box (memory card) wins and withdraws it.
fn quit_question(page: frontend::Page, game: Option<title::DialogFacts>, t: &dyn Fn(u32) -> String) -> Option<title::DialogFacts> {
	if page != frontend::Page::Root || !title_quit::ask() {
		return game;
	}
	if game.is_some() {
		title_quit::set_ask(false);
		return game;
	}
	Some(title::DialogFacts { lines: vec![t(ID_QUITD_BODY)], buttons: vec![(CIRCLE, t(0x524f)), (TRIANGLE, t(0x524b))] })
}

/// Is the modern page the menu this frame? Unknown pages (a page change in flight) keep the last answer.
fn wants(fe: &FrontEnd, snap: bool, page: Option<frontend::Page>, movie: bool, picture: bool) -> bool {
	if !style::modern() || ui::is_open() || ui_gpu::refused() || movie || picture || fe.art.is_none() {
		return false;
	}
	match (snap, page) {
		(true, Some(p)) => p != frontend::Page::Title,
		(true, None) => title_host::up(),
		(false, _) => false,
	}
}

/// The logo at the mockup's box (48, 40, 300 x 150 of 1280 x 720), faded like the classic one by retail's alpha.
fn logo(g: &mut crate::viewer::Gpu, fe: &FrontEnd, w: f32, h: f32) {
	let alpha = fe.menu.ram_alpha.map_or(1.0, |a| a.0 / 128.0);
	if let (Some(t), true) = (fe.art.as_ref().and_then(|a| a.logo), alpha > 0.0) {
		let r = Scale::fit(w as u32, h as u32).to_window(Rect::new(48.0, 40.0, 300.0, 150.0));
		g.hud.push(Quad { tex: t, rect: [r.x, r.y, r.w, r.h], uv: [0.0, 0.0, 1.0, 1.0], colour: [1.0, 1.0, 1.0, alpha], angle: 0.0, corners: None });
	}
}

fn modern_page(g: &mut crate::viewer::Gpu, fe: &FrontEnd, fr: &FrontRam, m: &MenuSnap, page: frontend::Page) {
	let pad = crate::retail::prompts::pad_peek().map(|p| mhost::pad_sample(p.buttons, p.sticks));
	title_host::begin(pad);
	let sel = sel_of(m, page, fr.slot_cur);
	let style = crate::modern_menu::btn_text::Style { device: title_host::device(), glyphs: crate::modern_menu::look::glyph_set(), menu: true };
	let t = |id: u32| crate::modern_menu::btn_text::text(&Strings(&fe.text), id, style); // buttons kept as chips (`btn_text`)
	// The rows and the header are font bytes (`frontend::Text`): decoded like the page's other texts, or an accent byte shows as a
	// box (the German Sound page's volume rows).
	let rows: Vec<(String, bool)> = fe.menu.items(&fe.text).into_iter().map(|(s, on)| (decode(&s).plain(), on)).collect();
	let slots = slot_views(fr);
	let dialog = dialog_facts(page, &fr.dialog, &t);
	let boxed = dialog.is_some() && page != frontend::Page::Overwrite;
	let dialog = quit_question(page, dialog, &t);
	let mut root = vec![t(0x4f2a), t(0x4f28), t(0x4ee7)];
	root.extend(Some(t(ID_QUITD)).filter(|s| !s.is_empty())); // the host's own fourth entry, when its label is known
	let facts = title::Facts {
		page,
		rows: &rows,
		header: fe.menu.header(&fe.text).map(|h| decode(&h).plain()).unwrap_or_default(),
		root,
		slots: &slots,
		empty: t(0x5217),
		no_save: t(0x5232),
		sel: title_host::display_sel(page, sel),
		targets: title::flatten(m),
		dialog,
		sound: Some([fe.menu.settings.sfx, fe.menu.settings.music]),
		lang: fe.menu.lang,
		device: title_host::device(),
		glyphs: crate::modern_menu::look::glyph_set(),
	};
	rac_engine::ui::keycaps::set_lang(fe.menu.lang as usize);
	// The mouse's size is the WINDOW's (`Gpu::win_size`, as the pause, shop and map pages pass): `config` is the render size, half
	// the window with DLSS / FSR Performance or a render scale, and the page's hit test then read every cursor at twice its place
	// (user 2026-11-04 "why cant i use mouse in this menu", DLSS Performance: no row answered the mouse on the title pages).
	let mut prims = title_host::end(title::build(&facts), Some(m), sel, g.win_size(), boxed);
	// The scene's fade to black (around the menu) covers the page like the picture.
	if fr.fade > 0.0 {
		prims.push(Prim::RRect { rect: Rect::new(-5000.0, -5000.0, 10000.0, 10000.0), radius: 0.0, fill: Rgba::rgba(0, 0, 0, fr.fade.min(1.0)), border: None });
	}
	g.ui_gpu.set_scene(prims);
}

/// The menu of a front-end frame (logo, Press Start, pages). True = the modern page is up.
pub(super) fn draw_menu(g: &mut crate::viewer::Gpu, fe: &FrontEnd, fr: &FrontRam, snap: Option<&MenuSnap>, movie: bool, picture: bool) -> bool {
	let win = (g.config.width as f32, g.config.height as f32);
	let page = snap.and_then(|m| page_of(Some(m)));
	let modern = wants(fe, snap.is_some(), page, movie, picture);
	// A front page the title cannot map keeps the last picture only through a page change: after `GIVE_UP_MS` the classic menu draws.
	let modern = modern && !title_host::gives_up(snap.is_some() && page.is_none());
	title_host::set_up(modern);
	let modern = modern && title_host::up(); // a tripped watchdog keeps the classic menu, no modern page over it (R3-3)
	if modern {
		logo(g, fe, win.0, win.1);
		match (snap, page) {
			(Some(m), Some(p)) => modern_page(g, fe, fr, m, p),
			_ => g.ui_gpu.set_scene(title_host::last()),
		}
	} else if let (Some(art), false, false) = (&fe.art, movie, picture || crate::modern_menu::hide::retail_hidden(crate::modern_menu::hide::Screen::Title)) { // [PC] the modern Options page over the title replaces the menu
		let n0 = g.hud.len();
		fe.menu.draw(&mut g.hud, win.0, win.1, art, &fe.text);
		super::front_quit_draw::draw(&mut g.hud, win, &fe.menu, art, &fe.text, snap); // [PC] the host's fourth entry "Quit to Desktop" and its question (front_quit_draw.rs)
		widen(&mut g.hud[n0..], win.0);
	}
	modern
}

/// Mode 4, the modal message box (memory card messages; FUN_001fb780 draws it after the fade): the modern page shows it itself
/// when it knows the words, otherwise the classic box.
pub(super) fn draw_dialog(g: &mut crate::viewer::Gpu, fe: &FrontEnd, fr: &FrontRam, movie: bool, modern: bool) {
	let (w, h) = (g.config.width as f32, g.config.height as f32);
	if let (Some(art), false, 4) = (&fe.art, movie, fr.dialog.mode) {
		if modern && frontend::dialog_text(&fr.dialog).is_some() {
			return;
		}
		let n1 = g.hud.len();
		fe.menu.draw_dialog(&mut g.hud, w, h, art, &fe.text, &fr.dialog);
		widen(&mut g.hud[n1..], w);
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::menu_model::cmd::TEST_LOCK;

	fn t(id: u32) -> String {
		format!("T{id:x}")
	}

	#[test]
	fn the_quit_question_is_the_box_over_the_root_page_and_a_game_box_wins() {
		let _l = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
		title_quit::clear();
		let game = title::DialogFacts { lines: vec!["Card".into()], buttons: vec![(CROSS, "Continue".into())] };
		assert_eq!(quit_question(frontend::Page::Root, None, &t), None, "not asked: no box");
		assert_eq!(quit_question(frontend::Page::Root, Some(game.clone()), &t), Some(game.clone()));
		title_quit::set_ask(true);
		let q = quit_question(frontend::Page::Root, None, &t).expect("asked: the question is the box");
		assert_eq!(q.lines, ["T7f33"], "its words are the body text");
		assert_eq!(q.buttons, [(CIRCLE, "T524f".to_string()), (TRIANGLE, "T524b".to_string())], "Circle = Yes, Triangle = No");
		assert!(title_quit::ask(), "showing it keeps it");
		assert_eq!(quit_question(frontend::Page::NewGame, None, &t), None, "only on the root page");
		assert_eq!(quit_question(frontend::Page::Root, Some(game.clone()), &t), Some(game), "a memory card box wins ...");
		assert!(!title_quit::ask(), "... and withdraws the question");
		title_quit::clear();
	}
}
