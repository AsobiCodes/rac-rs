//! The options screen as kit data: turns a `Model` into a `kit::Screen` (tabs, banner, sections and rows with their controls, the
//! detail panel of the focused row, hints, notice) plus a `ScreenMap` from item numbers to row numbers. Pure; called every frame.
use super::kit::{BannerData, BoxData, Chip, Control, Detail, Footer, Hint, HintKind, Item, Meter, RowData, Screen};
use super::model::Model;
use super::rows::{pick, RowKind, T5};
use super::theme;

/// Which `Screen::items` entry is which row of the model (None = banner or section title).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ScreenMap {
	pub item_row: Vec<Option<usize>>,
}

impl ScreenMap {
	/// The global row index of item `item`.
	pub fn row(&self, item: usize) -> Option<usize> {
		self.item_row.get(item).copied().flatten()
	}

	/// The item number of global row `row`.
	pub fn row_item(&self, row: usize) -> Option<usize> {
		self.item_row.iter().position(|r| *r == Some(row))
	}
}

pub fn build(m: &Model) -> (Screen, ScreenMap) {
	let c = &m.menu.chrome;
	let t = |x: &T5| pick(x, m.lang).to_string();
	let mut items = Vec::new();
	let mut map = ScreenMap::default();
	if let Some(rule) = m.tab_rule() {
		// [PC] No button text (no button) when there is nothing to switch to (model_edit.rs `fix_value`).
		let button = m.fix_text(rule);
		items.push(Item::Banner(BannerData { text: t(&rule.banner), button }));
		map.item_row.push(None);
	}
	let mut sec = usize::MAX;
	for i in m.cur_shown() {
		let r = m.rows[i];
		if r.sec != sec {
			sec = r.sec;
			items.push(Item::Section(t(&m.menu.tabs[r.tab].sections[r.sec].title)));
			map.item_row.push(None);
		}
		items.push(Item::Row(RowData { label: t(&r.row.label), dim: false, control: control(m, i) })); // hidden rows are not here
		map.item_row.push(Some(i));
	}
	let s = Screen {
		title: t(if m.front { &c.title_front } else { &c.title }),
		status: t(&c.autosave),
		notice: m.menu.notices.iter().find(|n| m.value_of(n.row) == Some(n.value)).map(|n| t(&n.text)),
		tabs: m.menu.tabs.iter().map(|x| t(&x.name)).collect(),
		tab: m.tab,
		focus: map.row_item(m.focus),
		items,
		scroll: m.scroll,
		detail: detail(m),
		hints: hints(m),
		device: m.device,
		glyphs: m.glyphs,
		peek: m.peek,
		peek_text: t(&c.back_to_menu),
		accent: theme::ACCENT,
	};
	(s, map)
}

fn control(m: &Model, i: usize) -> Control {
	let row = m.rows[i].row;
	let v = m.value(i);
	match row.kind {
		RowKind::Choice => {
			let (index, count) = m.offered_place(i, v); // [PC] only the values the engine offers (ui/live.rs `offer`)
			Control::Choice { value: m.value_text(i), index, count }
		}
		RowKind::Toggle => Control::Toggle { value: m.value_text(i), on: v != 0 },
		RowKind::Slider { .. } => Control::Slider { value: m.value_text(i), t: m.slider_pos(i) },
		RowKind::Preset => {
			let cur = m.preset_index();
			let mut chips: Vec<Chip> = (0..m.menu.presets.len())
				.map(|p| Chip { text: m.preset_name(Some(p)), selected: cur == Some(p), disabled: false })
				.collect();
			chips.push(Chip { text: m.preset_name(None), selected: cur.is_none(), disabled: true });
			Control::Chips(chips)
		}
	}
}

fn detail(m: &Model) -> Option<Detail> {
	let i = m.focus;
	let r = m.rows.get(i)?;
	let c = &m.menu.chrome;
	let t = |x: &T5| pick(x, m.lang).to_string();
	let tab = &m.menu.tabs[r.tab];
	let boxed = m.rule(i).filter(|_| m.blocked(i)).map(|rule| {
		let cur = m.value_of(rule.row).map(|v| m.text_of(m.row_index(rule.row).unwrap_or(0), v)).unwrap_or_default();
		BoxData { text: crate::localization::ui_text(pick(&rule.detail, m.lang)).replace("{}", crate::localization::ui_text(&cur)), button: crate::localization::ui_text(&m.fix_text(rule)).to_owned() }
	});
	let meter = (r.row.cost.level() > 0).then(|| {
		let level = r.row.cost.level();
		Meter { label: t(&c.speed_impact), level, max: 3, text: t(&c.cost_names[level as usize - 1]) }
	});
	Some(Detail {
		path: format!("{} \u{b7} {}", t(&tab.name), t(&tab.sections[r.sec].title)),
		title: t(&r.row.label),
		now: Some((t(&c.now), m.value_text(i))),
		default: Some((t(&c.default), m.default_text(i))),
		body: t(&r.row.desc),
		note: super::live::note(r.row.id).or(r.row.note.as_ref()).map(t),
		boxed,
		meter,
		footer: Some(Footer { note: t(&c.peek_note), button: t(&c.view_game), lead: HintKind::Peek }),
	})
}

fn hints(m: &Model) -> Vec<Hint> {
	let c = &m.menu.chrome;
	let h = |kind, x: &T5| Hint { kind, text: pick(x, m.lang).to_string() };
	vec![
		h(HintKind::Change, &c.change),
		h(HintKind::Back, &c.back),
		h(HintKind::Reset, &c.reset),
		h(HintKind::Peek, &c.view_game),
		h(HintKind::Tabs, &c.switch_tab),
	]
}
