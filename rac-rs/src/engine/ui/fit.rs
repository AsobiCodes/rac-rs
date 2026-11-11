//! Text checks for a modern-UI table, kept outside the test modules so a game pack's own test can run them on its real `UiMenu`
//! (the engine's tests run them on the demo menu): every string of a table, the characters the fonts lack, and the drawn texts
//! that are wider than the box the layout gave them (the renderer would shrink such a text, so it is a table bug, not a crash).
use super::font::FontSet;
use super::kit::{layout::Layout, text::run_width, TextMeasure};
use super::prim::Prim;
use super::rows::{RowKind, UiMenu, LANGS, T5};
use super::Ui;
use std::collections::BTreeSet;

fn add(out: &mut Vec<String>, t: &T5) {
	out.extend(t.iter().map(|s| s.to_string()));
}

/// Every string of the table in every language: tabs, sections, labels, descriptions, values, notes, slider units, presets,
/// dependency texts, notices and the fixed words of the screen.
pub fn all_texts(menu: &UiMenu) -> Vec<String> {
	let mut out = Vec::new();
	for tab in menu.tabs {
		add(&mut out, &tab.name);
		for sec in tab.sections {
			add(&mut out, &sec.title);
			for r in sec.rows {
				add(&mut out, &r.label);
				add(&mut out, &r.desc);
				r.values.iter().for_each(|v| add(&mut out, v));
				r.note.iter().for_each(|n| add(&mut out, n));
				if let RowKind::Slider { unit, .. } = r.kind {
					out.push(unit.to_string());
				}
			}
		}
	}
	menu.presets.iter().for_each(|p| add(&mut out, &p.name));
	for n in menu.needs {
		add(&mut out, &n.banner);
		add(&mut out, &n.detail);
		add(&mut out, &n.button);
	}
	menu.notices.iter().for_each(|n| add(&mut out, &n.text));
	let c = &menu.chrome;
	for t in [&c.title, &c.title_front, &c.autosave, &c.now, &c.default, &c.speed_impact, &c.peek_note, &c.view_game, &c.back_to_menu, &c.change, &c.back, &c.reset, &c.switch_tab, &c.custom] {
		add(&mut out, t);
	}
	c.cost_names.iter().chain(c.off_on.iter()).for_each(|t| add(&mut out, t));
	out
}

/// Characters of `texts` the fonts have no glyph for, each character once with the first text that holds it.
pub fn missing_in(texts: &[String]) -> Vec<(String, char)> {
	let fonts = FontSet::get();
	let mut seen = BTreeSet::new();
	let mut out = Vec::new();
	for t in texts {
		for c in t.chars() {
			if !fonts.covers(c) && seen.insert(c) {
				out.push((t.clone(), c));
			}
		}
	}
	out
}

/// `missing_in` for every string of the table.
pub fn missing_chars(menu: &UiMenu) -> Vec<(String, char)> {
	missing_in(&all_texts(menu))
}

/// Calls `f(lang, tab, focused row, draw list)` for every language, both caption variants, every tab and every focused row.
pub fn each_frame(menu: &'static UiMenu, make: &dyn Fn() -> Box<dyn TextMeasure + Send>, mut f: impl FnMut(usize, usize, usize, &[Prim])) {
	each_screen(menu, make, |lang, tab, focus, _, prims| f(lang, tab, focus, prims));
}

/// Applies the fixes of the rules that hide row `i` (the root of its chain first), as far as the engine offers a value for them.
fn reveal(m: &mut super::model::Model, i: usize) {
	for _ in 0..8 {
		let Some(rule) = m.rule(i).filter(|_| m.blocked(i)) else { return };
		let before = m.vals.clone();
		m.enhance(rule);
		if m.vals == before {
			return; // nothing to switch to on this PC
		}
	}
}

/// `each_frame` with the screen's layout too.
fn each_screen(menu: &'static UiMenu, make: &dyn Fn() -> Box<dyn TextMeasure + Send>, mut f: impl FnMut(usize, usize, usize, &Layout, &[Prim])) {
	for lang in 0..LANGS {
		for front in [false, true] {
			let mut ui = Ui::new(menu, lang, make());
			ui.begin(front, None);
			for tab in 0..menu.tabs.len() {
				ui.model.set_tab(tab);
				for focus in ui.model.cur_rows() {
					// A row hidden at the defaults (Upscale quality while the Upscaler is Off) is checked as it is shown: its
					// banner fixes applied for this frame only.
					let (vals, touched) = (ui.model.vals.clone(), ui.model.touched.clone());
					reveal(&mut ui.model, focus);
					if ui.model.shown(focus) {
						ui.model.focus = focus;
						let (l, prims) = ui.frame();
						f(lang, tab, focus, &l, &prims);
					}
					(ui.model.vals, ui.model.touched) = (vals, touched);
					ui.model.take_changes();
					ui.model.take_sounds();
				}
			}
		}
	}
}

/// Every text the screen really draws (headers in capitals, wrapped description lines, hint bar ...), over `each_frame`.
pub fn drawn_texts(menu: &'static UiMenu, make: &dyn Fn() -> Box<dyn TextMeasure + Send>) -> Vec<String> {
	let mut out = BTreeSet::new();
	each_frame(menu, make, |_, _, _, prims| {
		out.extend(prims.iter().filter_map(|p| if let Prim::Text(t) = p { Some(t.text.clone()) } else { None }));
	});
	out.into_iter().collect()
}

/// Drawn texts wider than their `max_w` box, as one line each ("lang 2 tab 1 row 7: 'TEXT' is 210 px, box 176").
pub fn overflows(menu: &'static UiMenu, make: &dyn Fn() -> Box<dyn TextMeasure + Send>) -> Vec<String> {
	let m = make();
	let mut bad = BTreeSet::new();
	each_frame(menu, make, |lang, tab, row, prims| {
		for p in prims {
			let Prim::Text(t) = p else { continue };
			if t.max_w > 0.0 {
				let w = run_width(&*m, t.role, t.px, &t.text, t.track);
				if w > t.max_w + 0.5 {
					bad.insert(format!("lang {lang} tab {tab} row {row}: '{}' is {w:.0} px, box {:.0}", t.text, t.max_w));
				}
			}
		}
	});
	bad.into_iter().collect()
}

/// Detail panels whose body / note text was cut short (a line ending in an ellipsis) or whose lowest line or box passes the meter /
/// footer separator (R1-8), as one line each ("lang 1 tab 2 row 3: 'LAST LINE' cut" / "... ends at 640, floor 600").
pub fn detail_cuts(menu: &'static UiMenu, make: &dyn Fn() -> Box<dyn TextMeasure + Send>) -> Vec<String> {
	let mut bad = BTreeSet::new();
	each_screen(menu, make, |lang, tab, row, l, _| {
		let Some(d) = &l.detail else { return };
		if let Some(cut) = d.body.iter().chain(&d.note).find(|t| t.text.ends_with('\u{2026}')) {
			bad.insert(format!("lang {lang} tab {tab} row {row}: '{}' cut", cut.text));
		}
		if let Some(b) = d.text_bottom().filter(|b| *b > d.floor() + 0.5) {
			bad.insert(format!("lang {lang} tab {tab} row {row}: ends at {b:.0}, floor {:.0}", d.floor()));
		}
	});
	bad.into_iter().collect()
}
