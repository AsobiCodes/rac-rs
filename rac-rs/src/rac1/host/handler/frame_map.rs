//! Display-frame step of the modern planet map (v0.5 build 12; the twin of `frame_ship.rs`): decides in this frame whether the modern
//! page is the picture (`map::wanted`: the Modern style, the planet map 0xb or its mission list 0xc, also while they fly in), reads the
//! shown planet and its gold bolts from the worker's snapshot (`Retail::level_menu`, `Extra::map` / `ship`), turns the game's legend
//! texts into the page's key list, builds the page (`map::build`) and hands the UI pass its draw list. While the Map tab's presses are
//! on their way (`retail::map_open::pending`) the page shows at once, its pictures still empty, so the pause menu becomes the map
//! without a frame of gameplay between them; a tab of the map page keeps it until the pause menu is up. The hide switch of the retail
//! map pictures is set in the same frame, before `frame_render`'s `hide_retail` (which places the crops). In the Classic style nothing
//! happens here. See `map_host.rs`.
use super::*;
use crate::modern_menu::map::{self, KeyRow, Stage};
use crate::modern_menu::{btn_text, hide, host as mhost, look, map_host, pause, pause_host, ship_marks, style};
use crate::retail::map_open::{self, Goal};
use crate::retail::menu_model::text::{Btn, RichText, Span};
use crate::retail::menu_model::Strings;
use crate::retail::mouse_menu::{CIRCLE, CROSS, TRIANGLE};
use crate::retail::{menu_keys, prompts};
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::prim::Prim;
use rac_engine::ui::{self, gpu as ui_gpu};

/// The map's legend [C: decomp/notes/ingame_menu_leftovers.md P1-c, page 0xb]: (text id, the pad button a click on the row presses;
/// 0 = none). Two columns of three: View Missions, Play Infobot, Exit | previous map, next map, track / zoom (the sticks).
const MAP_KEYS: [(u32, u16); 6] = [(0x4eee, CROSS), (0x4eef, CIRCLE), (0x4ee0, TRIANGLE), (0x4efa, 4), (0x4efb, 8), (0x4efc, 0)];
/// The mission list's one hint: Exit (Triangle goes back to the map).
const MISSION_KEYS: [(u32, u16); 1] = [(0x4ee0, TRIANGLE)];

impl App {
	pub(super) fn frame_map(&mut self) {
		if !style::modern() {
			if map_host::up() {
				map_host::set_up(false); // the style was switched to Classic while the page was up
			}
			return;
		}
		let lang = self.modern_lang(); // the game's language (host/modern_lang.rs), not the PC settings'
		let win = self.gpu.as_ref().map_or((1280, 720), |g| g.win_size());
		let scene = match (self.retail.as_ref(), self.ui_text.as_ref()) {
			(Some(r), Some(text)) => map_scene(r, text, lang, win),
			_ => {
				map_host::set_up(false);
				None
			}
		};
		if let (Some(prims), Some(g)) = (scene, self.gpu.as_mut()) {
			g.ui_gpu.set_scene(prims); // valid for the next draw only: every frame while the page is up
		}
	}
}

/// One legend text as a key-list row: its leading buttons as chips (the menu keys with the keyboard, `prompts::menu_key`), then its
/// words with any other button as a chip too. None for an empty text.
fn key_row(r: &RichText, style: btn_text::Style, key: &dyn Fn(Btn) -> String, bit: u16) -> Option<KeyRow> {
	let (mut leads, mut rest, mut leading) = (Vec::new(), Vec::new(), true);
	for s in &r.0 {
		match s {
			Span::Button(b) if leading => leads.push(btn_text::chip(*b, style, key)),
			Span::Text(t) if leading && t.trim().is_empty() => {}
			Span::Break | Span::Mark(_) if leading => {}
			_ => {
				leading = false;
				rest.push(s.clone());
			}
		}
	}
	let words = RichText(rest).plain_with(|b| btn_text::chip(b, style, key)).trim().to_string();
	if leads.is_empty() && words.is_empty() {
		return None;
	}
	let text = if leads.is_empty() { words } else { format!("{}  {}", leads.join(" "), words) };
	Some(KeyRow { text, bit })
}

/// The modern planet map's draw list of this frame; None = the classic picture (the hide switch and the mouse are given back).
fn map_scene(r: &crate::retail::Retail, text: &crate::frontend::Text, lang: usize, win: (u32, u32)) -> Option<Vec<Prim>> {
	let was_up = map_host::up();
	let lm = r.level_menu();
	if let Some(l) = &lm {
		map_host::note_header(l.level, l.bolts); // the shell's figures for the frames the Map tab's presses are on their way
	}
	let gate = map::Gate { modern: true, ui_open: ui::is_open(), refused: ui_gpu::refused(), page: lm.as_ref().map(|l| l.menu.id) };
	let page = lm.filter(|_| map::wanted(&gate));
	let opening = page.is_none()
		&& !gate.ui_open
		&& !gate.refused
		&& match map_open::pending() {
			Some(Goal::Map) => map_open::opening_map(),
			Some(Goal::Pause(_)) => was_up && !pause_host::up(),
			None => false,
		};
	if page.is_none() && !opening {
		// The map closing: the retail pages stay hidden behind an empty page until they are gone (`hide::closing_hold`).
		let hold = hide::closing_hold(hide::Screen::Map, r.menu_closing(), r.shown_field());
		map_host::set_up(was_up && hold);
		map_host::drop_crops(); // the overlay the pictures are cut from is going away
		return map_host::up().then(Vec::new);
	}
	let (stage, menu, level, bolts, extra) = match &page {
		Some(l) => (map::stage_of(l.menu.id)?, Some(l.menu), l.level, l.bolts, Some(l.extra)),
		None => {
			let (level, bolts) = map_host::header();
			(Stage::Map, None, level, bolts, None)
		}
	};
	map_host::set_up(true);
	if !map_host::up() {
		return None; // a tripped watchdog keeps the classic picture
	}
	map_host::begin(prompts::pad_peek().map(|p| mhost::pad_sample(p.buttons, p.sticks)));
	let strings = Strings(text);
	let (device, glyphs) = (map_host::device(), look::glyph_set());
	let style = btn_text::Style { device, glyphs, menu: true };
	let t = |id: u32| btn_text::text(&strings, id, style);
	let sc = menu_keys::screen_of_id(if stage == Stage::Missions { menu_keys::MISSIONS } else { menu_keys::MAP });
	let key = |b: Btn| prompts::menu_key(btn_text::glyph_code(b), &sc);
	let ids: &[(u32, u16)] = if stage == Stage::Missions { &MISSION_KEYS } else { &MAP_KEYS };
	let keys: Vec<KeyRow> = ids.iter().filter_map(|&(id, bit)| key_row(&strings.rich(id), style, &key, bit)).collect();
	let seen = extra.and_then(|x| x.map);
	let gold = seen.and_then(|s| s.planet).zip(extra.and_then(|x| x.ship.as_ref())).map(|(p, sh)| (sh.gold_found(p), ship_marks::gold_total(p))).filter(|g| g.1 > 0);
	let shell = pause::shell_for(level, bolts, lang, &t);
	let tabs = pause::tab_words(&t, lang, pause_host::goodies_seen());
	let settled = menu.is_some_and(|m| !m.busy());
	let facts = map::Facts { stage, seen, gold, text: &t, keys: &keys, shell, tabs: &tabs, lang, device, glyphs, settled };
	let built = map::build(&facts, &FontMeasure);
	Some(map_host::end(built, menu, win))
}
