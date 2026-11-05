//! Display-frame step of the modern vendor shop (v0.5 screens stage E; the twin of `frame_pause.rs`): decides in this frame whether the
//! modern page is the picture (`vendor::wanted`: the Modern style, the shop in its list phase), gathers the facts the main thread lacks
//! from the worker's snapshot (`Retail::shop`'s `ShopView`: rows, prices, bolts, ammo, stage), builds the page (`vendor::build`) and hands
//! the UI pass its draw list. The hide switch of the retail shop overlay is set in the same frame, before `frame_render`'s `hide_retail`
//! (which also moves the game's item preview into the page's card), so no frame draws both. The page is drawn from the opening fade on;
//! while it cannot be (the list not readable yet, or the shop closing) an empty page keeps the retail shop hidden. In the Classic
//! style nothing happens here: the picture and the input are today's. See `vendor_host.rs`.
use super::*;
use crate::modern_menu::{host as mhost, style, vendor, vendor_host};
use crate::retail::menu_model::Strings;
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::prim::Prim;
use rac_engine::ui::{self, gpu as ui_gpu};

impl App {
	pub(super) fn frame_vendor(&mut self) {
		if !style::modern() {
			if vendor_host::up() {
				vendor_host::set_up(false); // the style was switched to Classic while the page was up
			}
			return;
		}
		let lang = self.modern_lang(); // the game's language (host/modern_lang.rs), not the PC settings'
		let win = self.gpu.as_ref().map_or((1280, 720), |g| g.win_size());
		let scene = match (self.retail.as_ref(), self.ui_text.as_ref()) {
			(Some(r), Some(text)) => vendor_scene(r, text, lang, win),
			_ => {
				vendor_host::set_up(false);
				None
			}
		};
		if let (Some(prims), Some(g)) = (scene, self.gpu.as_mut()) {
			g.ui_gpu.set_scene(prims); // valid for the next draw only: every frame while the page is up
		}
	}
}

/// The modern shop's draw list of this frame; None = the classic picture (the hide switch and the mouse are given back).
fn vendor_scene(r: &crate::retail::Retail, text: &crate::frontend::Text, lang: usize, win: (u32, u32)) -> Option<Vec<Prim>> {
	let view = r.shop().and_then(|s| s.view.as_ref());
	let gate = vendor::Gate { modern: true, ui_open: ui::is_open(), refused: ui_gpu::refused(), vendor: r.vendor(), view };
	let Some(view) = view.filter(|_| vendor::wanted(&gate)) else {
		// Flying in before the list reads, or closing: the retail shop stays hidden behind an empty page (`vendor::holds`).
		let hold = vendor::holds(&gate);
		vendor_host::set_up(hold);
		if hold {
			vendor_host::hold(); // an empty page: no stale rows to click, and the watchdog sees the empty list
			vendor_host::holding(r.shown_field()); // a slow fly-in does not trip the watchdog (R4-4)
		}
		return vendor_host::up().then(Vec::new); // a tripped watchdog keeps the classic picture (R3-3)
	};
	vendor_host::begin(crate::retail::prompts::pad_peek().map(|p| mhost::pad_sample(p.buttons, p.sticks)));
	let strings = Strings(text);
	let names: Vec<String> = view.rows.iter().map(|row| strings.plain(crate::inventory::name_text(row.item as u8)).trim().to_string()).collect();
	let facts = vendor::Facts { view, names: &names, focus: vendor_host::local(), buyall: crate::modern_menu::vendor_nav::focused(), scroll: vendor_host::scroll(), lang, device: vendor_host::device(), glyphs: crate::modern_menu::look::glyph_set() };
	match vendor::build(&facts, &FontMeasure) {
		Some(built) => {
			vendor_host::set_up(true);
			vendor_host::up().then(|| vendor_host::end(built, r.shop_menu(), win))
		}
		None if vendor_host::up() => Some(vendor_host::last()),
		None => {
			vendor_host::set_up(false);
			None
		}
	}
}
