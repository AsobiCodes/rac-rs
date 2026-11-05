//! Display-frame step of the modern Galactic Map (v0.5 screens stage F2; the twin of `frame_vendor.rs`): decides in this frame whether
//! the modern page is the picture (`ship::wanted`: the Modern style, the ship's page 0xf or 0x10, settled), reads the planet list from
//! the worker's menu snapshot and ship facts (`Retail::level_menu`), builds the page (`ship::build`) and hands the UI pass its draw list. The hide switch
//! of the retail ship pictures is set in the same frame, before `frame_render`'s `hide_retail` (which also lays the painting under the
//! page), so no frame draws both. In the Classic style nothing happens here. See `ship_host.rs`.
use super::*;
use crate::modern_menu::{host as mhost, ship, ship_host, style};
use crate::retail::menu_model::{view::view, Strings};
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::prim::Prim;
use rac_engine::ui::{self, gpu as ui_gpu};

impl App {
	pub(super) fn frame_ship(&mut self) {
		if !style::modern() {
			if ship_host::up() {
				ship_host::set_up(false); // the style was switched to Classic while the page was up
			}
			return;
		}
		let lang = self.modern_lang(); // the game's language (host/modern_lang.rs), not the PC settings'
		let win = self.gpu.as_ref().map_or((1280, 720), |g| g.win_size());
		let scene = match (self.retail.as_ref(), self.ui_text.as_ref()) {
			(Some(r), Some(text)) => ship_scene(r, text, lang, win),
			_ => {
				ship_host::set_up(false);
				None
			}
		};
		if let (Some(prims), Some(g)) = (scene, self.gpu.as_mut()) {
			g.ui_gpu.set_scene(prims); // valid for the next draw only: every frame while the page is up
		}
	}
}

/// The modern Galactic Map's draw list of this frame; None = the classic picture (the hide switch and the mouse are given back).
fn ship_scene(r: &crate::retail::Retail, text: &crate::frontend::Text, lang: usize, win: (u32, u32)) -> Option<Vec<Prim>> {
	let was_up = ship_host::up();
	let Some(lm) = r.level_menu() else {
		// The menu closing: the retail pages stay hidden behind an empty page until they are gone (`hide::closing_hold`).
		let hold = crate::modern_menu::hide::closing_hold(crate::modern_menu::hide::Screen::Ship, r.menu_closing(), r.shown_field());
		ship_host::set_up(was_up && hold);
		ship_host::drop_planet(); // the overlay the planet picture is cut from is flying away
		return ship_host::up().then(Vec::new);
	};
	let (m, level, planets) = (lm.menu, lm.level, lm.extra.ship.clone().unwrap_or_default());
	let gate = ship::Gate { modern: true, ui_open: ui::is_open(), refused: ui_gpu::refused(), page: Some(m.id) };
	let stage = ship::stage_of(m.id).filter(|_| ship::wanted(&gate));
	let strings = Strings(text);
	let style = crate::modern_menu::btn_text::Style { device: ship_host::device(), glyphs: crate::modern_menu::look::glyph_set() };
	let t = |id: u32| crate::modern_menu::btn_text::text(&strings, id, style); // buttons kept as chips (`btn_text`)
	let v = view(m, &strings);
	let fresh = match (stage, v.as_ref()) {
		(Some(ship::Stage::List), Some(v)) => ship::listing_of(v, &t),
		_ => None,
	};
	let Some((stage, listing)) = stage.zip(fresh.clone().or_else(ship_host::memo).filter(|_| stage != Some(ship::Stage::List) || fresh.is_some())) else {
		// A page flying in whose list is not readable yet: the last picture stays (none coming up from the game), the retail one hidden.
		if stage.is_some() && m.state == 1 {
			ship_host::set_up(true);
			return ship_host::up().then(ship_host::last); // a tripped watchdog keeps the classic picture (R3-3)
		}
		ship_host::set_up(false);
		return None;
	};
	ship_host::begin(crate::retail::prompts::pad_peek().map(|p| mhost::pad_sample(p.buttons, p.sticks)));
	let infobot = v.as_ref().is_some_and(ship::infobot_of);
	let focus = ship_host::local().filter(|_| stage == ship::Stage::List);
	let facts = ship::Facts { list: &listing, stage, focus, scroll: ship_host::scroll(), here: level, planets: &planets, infobot, text: &t, lang, device: ship_host::device(), glyphs: crate::modern_menu::look::glyph_set() };
	match ship::build(&facts, &FontMeasure) {
		Some(built) => {
			ship_host::set_up(true);
			ship_host::up().then(|| ship_host::end(built, fresh.as_ref(), Some(m), win))
		}
		None => {
			ship_host::set_up(false);
			None
		}
	}
}
