//! Display-frame step of the modern pause menu (v0.5 screens stage C; the twin of `front_modern.rs` for the title): decides in this
//! frame whether the modern page is the picture (`pause::wanted`), gathers the facts the main thread lacks from the worker's snapshot
//! (`Retail::level_menu`: the page, its rows, the save slots, the Sound values), builds the page (`pause::build`) and hands the UI pass
//! its draw list. The hide switch of the retail pause picture is set in the same frame, before `frame_render`'s `hide_retail`, so no
//! frame draws both. In the Classic style, or on a page the modern set does not cover (Weapons, Gadgets, Quick Select, Items, Camera,
//! the maps, every memory card message box), nothing happens here: the picture and the input are today's. See `pause_host.rs`.
use super::*;
use crate::modern_menu::{host as mhost, pause, pause_host, style};
use crate::retail::menu_model::{view::view, Strings};
use crate::retail::{pcctl, pcext, pcmods};
use rac_engine::ui::{self, gpu as ui_gpu};

/// The PC options' words (the Graphics row and page, the Controls and Mods rows, the Achievements and Quit to Desktop rows) join the
/// language's strings: the pause menu's text bank (`ui_text`) is the disc's alone, and a list row without its word keeps the whole
/// page classic (`pause::list_parts`) [C: pause_0081 Options list, RAC_MENU_LOG 2026-11-02: row 0x7f70 had no word].
fn add_pc_texts(t: &mut crate::frontend::Text, lang: usize) {
	if t.get(pcext::ID_GRAPHICS).is_empty() {
		for (id, s) in pcext::texts(lang) {
			t.insert(id, s);
		}
		for id in [pcctl::ID_CONTROLS, pcctl::ID_DESC] {
			t.insert(id, pcctl::text(id, lang, 0, 0, false).iter().map(|&b| b as char).collect());
		}
	}
	if t.get(pcmods::ID_MODS).is_empty() {
		for (id, s) in pcmods::texts(lang) {
			t.insert(id, s);
		}
	}
}

impl App {
	pub(super) fn frame_pause(&mut self) {
		// The HUD-pass achievements list draws unless the modern page hosts it in THIS frame (set again by `pause_scene`): every path
		// without the page (Classic style, the page given back, the UI pipes refused) keeps today's list.
		crate::achievements::set_hosted(false);
		if !style::modern() {
			if pause_host::up() {
				pause_host::set_up(false); // the style was switched to Classic while the page was up
			}
			return;
		}
		let lang = self.modern_lang(); // the game's language (host/modern_lang.rs), not the PC settings'
		if let Some(t) = self.ui_text.as_mut() {
			add_pc_texts(t, lang);
		}
		let win = self.gpu.as_ref().map_or((1280, 720), |g| g.win_size());
		let scene = match (self.retail.as_ref(), self.ui_text.as_ref()) {
			(Some(r), Some(text)) => pause_scene(r, text, lang, win),
			_ => {
				pause_host::set_up(false);
				None
			}
		};
		if let (Some(prims), Some(g)) = (scene, self.gpu.as_mut()) {
			g.ui_gpu.set_scene(prims); // valid for the next draw only: every frame while the page is up
		}
	}
}

/// The open achievements list as the page's facts (`pause_ach`); None = closed. A list whose hub is not loaded yet shows empty.
fn ach_facts() -> Option<crate::modern_menu::pause_ach::AchFacts> {
	use crate::achievements as a;
	if !a::is_open() {
		return None;
	}
	let view = a::list_view().unwrap_or(a::ListView { rows: Vec::new(), got: 0, all: 0, cheated: a::cheated() });
	let (focus, first) = a::cursor();
	Some(crate::modern_menu::pause_ach::AchFacts { view, focus, first })
}

/// The modern pause menu's draw list of this frame; None = the classic picture (the hide switch and the input are given back).
fn pause_scene(r: &crate::retail::Retail, text: &crate::frontend::Text, lang: usize, win: (u32, u32)) -> Option<Vec<rac_engine::ui::prim::Prim>> {
	let was_up = pause_host::up();
	let Some(lm) = r.level_menu() else {
		// The menu closing: the retail pages stay hidden behind an empty page until they are gone (`hide::closing_hold`). The
		// achievements page's list closes with the menu (`close_ach`: pad Start passes on every page and closes the retail menu).
		pause_host::close_ach();
		let hold = crate::modern_menu::hide::closing_hold(crate::modern_menu::hide::Screen::Pause, r.menu_closing(), r.shown_field());
		pause_host::set_up(was_up && hold);
		return pause_host::up().then(Vec::new);
	};
	let strings = Strings(text);
	let v = view(lm.menu, &strings);
	let gate = pause::Gate {
		modern: true,
		ui_open: ui::is_open(),
		list_open: pause::list_open(), // [PC] the dev planet list owns the input and draws under the page (pause.rs `Gate`)
		refused: ui_gpu::refused(),
		front: r.front_ram().is_some(),
		paused: r.paused(),
		page: v.as_ref().map(|v| v.page),
	};
	let Some(v) = v.filter(|_| pause::wanted(&gate)) else {
		pause_host::set_up(false);
		return None;
	};
	let pad = crate::retail::prompts::pad_peek().map(|p| mhost::pad_sample(p.buttons, p.sticks));
	pause_host::begin(pad);
	let style = crate::modern_menu::btn_text::Style { device: pause_host::device(), glyphs: crate::modern_menu::look::glyph_set(), menu: false };
	let t = |id: u32| crate::modern_menu::btn_text::text(&strings, id, style); // buttons kept as chips (`btn_text`)
	let ach = ach_facts();
	let facts = pause::Facts {
		view: &v,
		extra: lm.extra,
		text: &t,
		goodies: pause_host::goodies(),
		level: lm.level,
		bolts: lm.bolts,
		local: pause_host::local(v.page),
		now: pause_host::now(),
		lang,
		device: pause_host::device(),
		glyphs: crate::modern_menu::look::glyph_set(),
		quit_desktop: pcext::quitrow::pending(),
		ach: ach.as_ref(),
	};
	match pause::build(&facts) {
		// A tripped watchdog refuses `set_up(true)`: then the classic menu is the picture, no modern page over it (R3-3).
		Some(built) => {
			pause_host::set_up(true);
			crate::achievements::set_hosted(built.ach.is_some() && pause_host::up());
			pause_host::up().then(|| pause_host::end(built, Some(lm.menu), win))
		}
		// A page flying in whose facts are not readable yet, or one just landed whose facts lag a field (`hold_unbuilt`): the last picture
		// stays (none coming up from the game), the retail one hidden.
		None if lm.menu.state == 1 || pause_host::hold_unbuilt(pause_host::now()) => {
			pause_host::set_up(true);
			pause_host::up().then(pause_host::repeat)
		}
		None => {
			pause_host::set_up(false);
			None
		}
	}
}
