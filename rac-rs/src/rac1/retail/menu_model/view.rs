//! The menu on screen as a modern screen reads it: the page, its items with the rows' texts decoded in the player's language, the
//! keys of the screen. Pure functions of a `MenuSnap` and a `Strings`; nothing here draws or sends.
use super::pages::{route, Page};
use super::text::{Btn, RichText, Strings};
use crate::modern_menu::hide::Screen;
use crate::retail::menu_keys;
use crate::retail::mouse_menu::{Item, Kind, MenuSnap};
use crate::retail::FrontRam;

/// One row of a list item: the text id of the record, its text, a second text (the Galactic Map's planet name) and whether the row
/// can be chosen.
#[derive(Clone, Debug, PartialEq)]
pub struct RowView {
	pub text_id: u16,
	pub text: RichText,
	pub name: Option<RichText>,
	pub enabled: bool,
}

/// What an item is, with its texts.
#[derive(Clone, Debug, PartialEq)]
pub enum Body {
	Rows { rows: Vec<RowView>, cursor: usize },
	/// A value page's rows (Subtitles, Camera, Sound, HelpDesk, the PC Graphics rows); their texts are the page's own.
	Values { n: u32, cursor: u32 },
	Grid { rows: u32, cols: u32, cursor: u32 },
	Ring { cursor: u32 },
	Panel,
	/// A button-hint line: the pad button it names and its text.
	Hint { button: Btn, text: RichText },
	Other,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ItemView {
	pub addr: u32,
	pub slot: usize,
	pub rect: [i32; 4],
	pub body: Body,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PageView {
	pub screen: Screen,
	pub page: Page,
	pub id: u32,
	/// The page has settled.
	pub ready: bool,
	pub front: bool,
	/// What the keys press on this page.
	pub keys: menu_keys::Screen,
	/// Index in `items` of the page's current item.
	pub cur: Option<usize>,
	pub items: Vec<ItemView>,
}

fn body(it: &Item, s: &Strings) -> Body {
	match &it.kind {
		Kind::Label { rows, cursor, .. } => {
			let rows = rows
				.iter()
				.enumerate()
				.map(|(i, &(_, enabled))| {
					let text_id = it.texts.get(i).copied().unwrap_or(0);
					let name = it.planet_texts.get(i).copied().filter(|&t| t != 0).map(|t| s.rich(t as u32));
					RowView { text_id, text: s.rich(text_id as u32), name, enabled }
				})
				.collect();
			Body::Rows { rows, cursor: *cursor as usize }
		}
		Kind::Value { n, cursor } => Body::Values { n: *n, cursor: *cursor },
		Kind::Grid { rows, cols, cursor } => Body::Grid { rows: *rows, cols: *cols, cursor: *cursor },
		Kind::Ring { cursor } => Body::Ring { cursor: *cursor },
		Kind::Panel => Body::Panel,
		Kind::Hint(bits) => match Btn::from_pad(*bits) {
			Some(button) => Body::Hint { button, text: it.texts.first().map_or_else(RichText::default, |&t| s.rich(t as u32)) },
			None => Body::Other,
		},
		Kind::Other => Body::Other,
	}
}

/// The page on screen as the modern screens read it; None when no item of it is on screen.
pub fn view(m: &MenuSnap, s: &Strings) -> Option<PageView> {
	if m.items.is_empty() {
		return None;
	}
	let r = route(m);
	let items: Vec<ItemView> = m.items.iter().map(|i| ItemView { addr: i.addr, slot: i.slot, rect: i.rect, body: body(i, s) }).collect();
	let cur = m.items.iter().position(|i| i.addr == m.cur);
	Some(PageView { screen: r.screen, page: r.page, id: m.id, ready: r.ready, front: m.front, keys: menu_keys::screen(m), cur, items })
}

/// One save slot of the Load / New Game pages as the list shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotView {
	pub level: u32,
	pub name: &'static str,
	pub bolts: u32,
	pub hours: u32,
	pub minutes: u32,
	pub date: String,
}

/// The five slots from the front end's RAM (None = empty slot).
pub fn slot_views(f: &FrontRam) -> Vec<Option<SlotView>> {
	slot_views_of(&f.slots)
}

/// The same from the raw slot summaries (the pause menu's Save / Load pages read them on the worker, `extra::read_slots`).
pub fn slot_views_of(slots: &[Option<super::extra::Slot>]) -> Vec<Option<SlotView>> {
	slots
		.iter()
		.map(|s| {
			let &(level, bolts, frames, d) = s.as_ref()?;
			Some(SlotView {
				level,
				name: crate::toc::LEVEL_NAMES.get(level as usize).copied().unwrap_or("?"),
				bolts,
				hours: (frames / 216_000).min(99), // as the list caps it (frontend.rs slot rows)
				minutes: frames / 3600 % 60,
				date: crate::save::card_date(d), // retail's order: month/day/year (the bytes are day, month, year); zero clock: a dash
			})
		})
		.collect()
}

/// One line of the Galactic Map's planet list.
#[derive(Clone, Debug, PartialEq)]
pub struct GalaxyRow {
	pub location: RichText,
	pub planet: Option<RichText>,
	pub enabled: bool,
	pub selected: bool,
}

/// The planet list of the Galactic Map page (empty on any other page).
pub fn galaxy_rows(v: &PageView) -> Vec<GalaxyRow> {
	if v.page != Page::GalaxyMap {
		return Vec::new();
	}
	let Some(Body::Rows { rows, cursor }) = v.items.iter().map(|i| &i.body).find(|b| matches!(b, Body::Rows { rows, .. } if rows.len() > 1)) else { return Vec::new() };
	rows.iter().enumerate().map(|(i, r)| GalaxyRow { location: r.text.clone(), planet: r.name.clone(), enabled: r.enabled, selected: i == *cursor }).collect()
}
