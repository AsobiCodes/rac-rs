//! The data of the options screen (layer 2): what a game pack hands the engine through `Pack::ui` -- tabs, sections and rows with
//! their kinds, values, defaults, speed-impact levels and dependencies -- and the hooks to read and write them. All texts come
//! with the pack, in the game's languages (`LANGS`, five today); the engine only picks one. Tables are `'static`.

pub const LANGS: usize = 5;
/// One text in the five languages of the game (index = the game's language number).
pub type T5 = [&'static str; LANGS];

/// The text for `lang` (an out-of-range language falls back to the last one).
pub fn pick(t: &T5, lang: usize) -> &'static str {
	t[lang.min(LANGS - 1)]
}

/// Speed impact of a row: none shows no meter, otherwise 1..3 bars. The pack's tables say how each level was measured (R&C1:
/// rac1/modern_menu/costs.rs, GPU time per frame with the row Off against all rows at their top setting).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cost {
	None,
	Low,
	Medium,
	High,
}

impl Cost {
	pub fn level(self) -> u8 {
		match self {
			Cost::None => 0,
			Cost::Low => 1,
			Cost::Medium => 2,
			Cost::High => 3,
		}
	}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowKind {
	/// One of `values` (index), chevrons and pips.
	Choice,
	/// Off / On (`UiChrome::off_on`), a switch.
	Toggle,
	/// A number from `min` to `max` in steps of `step`, shown with `unit` after it.
	Slider { min: i32, max: i32, step: i32, unit: &'static str },
	/// The Low / Medium / High / Custom chips of `UiMenu::presets`; has no value of its own.
	Preset,
}

/// A row is inert while the value of row `row` is below `min` (decision 1: "Needs Enhanced lighting"), or while row `row` is inert
/// itself (a chain: Metal reflections needs Metal, which needs Lighting): the row is hidden (user 2026-11-08, was dimmed) and
/// the tab shows the banner of the matching `NeedsRule` (the rule of the link that holds it, the root of the chain first).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Needs {
	pub row: &'static str,
	pub min: i32,
	/// The highest value that keeps the row live (i32::MAX: no upper limit; Lock-on strength needs Lock-on Auto / On, not Off).
	pub max: i32,
}

pub struct UiRow {
	/// Stable id the pack's `get` / `set` know the row by.
	pub id: &'static str,
	pub kind: RowKind,
	pub label: T5,
	pub desc: T5,
	/// The value names of a Choice, in the order shown (the pack maps them to its own bytes).
	pub values: &'static [T5],
	/// Default value (a Choice's index, a Toggle's 0 / 1, a Slider's number).
	pub def: i32,
	pub cost: Cost,
	pub needs: Option<Needs>,
	/// An extra line under the description ("applies on restart").
	pub note: Option<T5>,
}

pub struct UiSection {
	pub title: T5,
	pub rows: &'static [UiRow],
}

pub struct UiTab {
	pub id: &'static str,
	pub name: T5,
	pub sections: &'static [UiSection],
}

/// A preset: the value each listed row gets. The preset shown is the first whose rows all match, else Custom.
pub struct UiPresetDef {
	pub name: T5,
	pub sets: &'static [(&'static str, i32)],
}

/// The texts and the fix of a dependency: `banner` over the tab's list, `detail` in the focused row's box (`{}` = the current
/// value name of row `row`; drawn only if a hidden row has the focus, which `Model::settle_focus` prevents), `button` on both; the
/// button sets row `row` to `set_to`. An empty `banner` (all languages) = no banner: the rows are only hidden (a quality row under
/// an Off switch, not worth a banner over the whole tab).
/// `for_row` = the one dependent row this rule serves ("" = every row that needs `row` and has no rule of its own); `fix` = the
/// value the button sets, asked when the box is drawn (None = `set_to`; a `{}` in `button` names the value it sets).
pub struct NeedsRule {
	pub row: &'static str,
	pub set_to: i32,
	pub for_row: &'static str,
	pub fix: Option<fn() -> i32>,
	pub banner: T5,
	pub detail: T5,
	pub button: T5,
}

/// A pill in the header while row `row` has value `value` ("Classic menus from the next time you open Options").
pub struct Notice {
	pub row: &'static str,
	pub value: i32,
	pub text: T5,
}

/// The fixed words of the screen.
pub struct UiChrome {
	/// Caption over the tabs, from the pause menu and from the title screen.
	pub title: T5,
	pub title_front: T5,
	pub autosave: T5,
	pub now: T5,
	pub default: T5,
	pub speed_impact: T5,
	/// Names of the meter levels 1..3.
	pub cost_names: [T5; 3],
	pub peek_note: T5,
	pub view_game: T5,
	pub back_to_menu: T5,
	pub change: T5,
	pub back: T5,
	pub reset: T5,
	pub switch_tab: T5,
	pub off_on: [T5; 2],
	pub custom: T5,
}

pub struct UiMenu {
	pub tabs: &'static [UiTab],
	pub presets: &'static [UiPresetDef],
	/// The preset the Preset row's "reset to default" applies (the mockup: High).
	pub preset_reset: usize,
	/// Settings without a row that every preset sends too (`UiPack::set`, after the preset's rows). Not used to find the preset
	/// shown; `validate` refuses an id that is also a row.
	pub preset_also: &'static [(&'static str, i32)],
	pub needs: &'static [NeedsRule],
	pub notices: &'static [Notice],
	pub chrome: UiChrome,
}

/// Sounds the screen asks for (`Pack::ui_sound`; default none).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiSound {
	/// Focus or tab moved.
	Move,
	/// A value changed or a row was activated.
	Select,
	Back,
}

/// The pack's part of the modern UI (`Pack::ui`; none = the game has no modern menu).
pub struct UiPack {
	pub menu: fn() -> &'static UiMenu,
	/// Current value of a row by id (None = unknown row).
	pub get: fn(&str) -> Option<i32>,
	/// Asks the game to set a row (applied by the game's worker at its next field).
	pub set: fn(&str, i32),
	/// The game's language, 0..LANGS.
	pub lang: fn() -> usize,
	/// The game's Button glyphs option: 0 PlayStation, 1 Letters, 2 Letters (Switch).
	pub glyphs: fn() -> u8,
}

impl UiRow {
	pub fn value_count(&self) -> usize {
		match self.kind {
			RowKind::Choice => self.values.len(),
			RowKind::Toggle => 2,
			_ => 0,
		}
	}
}

impl UiMenu {
	/// Every row with its tab and section index, in tab order.
	pub fn rows(&self) -> impl Iterator<Item = (usize, usize, &'static UiRow)> + '_ {
		let tabs: &'static [UiTab] = self.tabs;
		tabs.iter().enumerate().flat_map(|(t, tab)| {
			tab.sections.iter().enumerate().flat_map(move |(s, sec)| sec.rows.iter().map(move |r| (t, s, r)))
		})
	}

	pub fn find(&self, id: &str) -> Option<&'static UiRow> {
		self.rows().find(|(_, _, r)| r.id == id).map(|(_, _, r)| r)
	}

	/// Problems of the tables (empty = fine): duplicate ids, defaults out of range, presets / dependencies / notices naming
	/// unknown rows or values, empty texts. A pack's own test calls this on its table.
	pub fn validate(&self) -> Vec<String> {
		let mut bad = Vec::new();
		let mut seen = std::collections::BTreeSet::new();
		for (_, _, r) in self.rows() {
			if !seen.insert(r.id) {
				bad.push(format!("duplicate row id {}", r.id));
			}
			check_row(r, &mut bad);
		}
		for p in self.presets {
			for (id, v) in p.sets {
				match self.find(id) {
					None => bad.push(format!("preset names unknown row {id}")),
					Some(r) if !value_ok(r, *v) => bad.push(format!("preset value {v} out of range for {id}")),
					_ => {}
				}
			}
		}
		for u in self.needs.iter().filter(|u| !u.for_row.is_empty()) {
			if !self.find(u.for_row).is_some_and(|r| r.needs.is_some_and(|n| n.row == u.row)) {
				bad.push(format!("the rule on {} is for row {}, which does not need it", u.row, u.for_row));
			}
		}
		for (id, _) in self.preset_also {
			if self.find(id).is_some() {
				bad.push(format!("preset_also names row {id} (put it in the presets)"));
			}
		}
		for (_, _, r) in self.rows() {
			if let Some(n) = r.needs {
				if self.find(n.row).is_none() {
					bad.push(format!("{} needs unknown row {}", r.id, n.row));
				}
				if !self.needs.iter().any(|u| u.row == n.row && (u.for_row.is_empty() || u.for_row == r.id)) {
					bad.push(format!("{} needs {} but no NeedsRule has texts for it", r.id, n.row));
				}
				// A chain (Metal reflections -> Metal -> Lighting) must end: a loop would hide its rows for good.
				let mut chain = vec![r.id];
				let mut next = Some(n);
				while let Some(n) = next {
					if chain.contains(&n.row) {
						bad.push(format!("{} needs a loop through {}", r.id, n.row));
						break;
					}
					chain.push(n.row);
					next = self.find(n.row).and_then(|x| x.needs);
				}
			}
		}
		for n in self.notices {
			if self.find(n.row).is_none() {
				bad.push(format!("notice names unknown row {}", n.row));
			}
		}
		if self.preset_reset >= self.presets.len() && !self.presets.is_empty() {
			bad.push("preset_reset out of range".to_string());
		}
		check_chrome(&self.chrome, &mut bad);
		bad
	}
}

fn value_ok(r: &UiRow, v: i32) -> bool {
	match r.kind {
		RowKind::Choice => v >= 0 && (v as usize) < r.values.len(),
		RowKind::Toggle => v == 0 || v == 1,
		RowKind::Slider { min, max, .. } => v >= min && v <= max,
		RowKind::Preset => false,
	}
}

fn t5_ok(t: &T5, what: &str, bad: &mut Vec<String>) {
	for (i, s) in t.iter().enumerate() {
		if s.trim().is_empty() {
			bad.push(format!("empty text ({what}, language {i})"));
		}
	}
}

fn check_row(r: &UiRow, bad: &mut Vec<String>) {
	t5_ok(&r.label, r.id, bad);
	t5_ok(&r.desc, r.id, bad);
	if let Some(n) = &r.note {
		t5_ok(n, r.id, bad);
	}
	for v in r.values {
		t5_ok(v, r.id, bad);
	}
	match r.kind {
		RowKind::Choice if r.values.len() < 2 => bad.push(format!("choice {} has fewer than 2 values", r.id)),
		RowKind::Slider { min, max, step, .. } if step <= 0 || min >= max => bad.push(format!("slider {} has a bad range", r.id)),
		_ => {}
	}
	if r.kind != RowKind::Preset && !value_ok(r, r.def) {
		bad.push(format!("default {} out of range for {}", r.def, r.id));
	}
}

fn check_chrome(c: &UiChrome, bad: &mut Vec<String>) {
	let all: [(&T5, &str); 11] = [
		(&c.title, "title"),
		(&c.title_front, "title_front"),
		(&c.autosave, "autosave"),
		(&c.now, "now"),
		(&c.default, "default"),
		(&c.speed_impact, "speed_impact"),
		(&c.peek_note, "peek_note"),
		(&c.view_game, "view_game"),
		(&c.back_to_menu, "back_to_menu"),
		(&c.change, "change"),
		(&c.custom, "custom"),
	];
	for (t, w) in all {
		t5_ok(t, w, bad);
	}
	for (t, w) in [(&c.back, "back"), (&c.reset, "reset"), (&c.switch_tab, "switch_tab")] {
		t5_ok(t, w, bad);
	}
	for t in c.cost_names.iter().chain(c.off_on.iter()) {
		t5_ok(t, "cost_names / off_on", bad);
	}
}
