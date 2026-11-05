//! The PC value pages of the pause Options list (Controls, Mods) as the modern pause menu reads them (v0.5 test build 4, lane v5p).
//! Both pages are copies of a retail value page whose rows are a table in RAM (6-word records `{label id, byte*, value id 0..3}`,
//! `pcctl.rs`, `pcmods.rs`); the label and value ids are dynamic texts that the two modules answer from their own state (the
//! bindings, the section, the installed mods). This file reads the table and resolves the ids with the same text functions the
//! game's text lookup uses, so the modern page shows exactly what the classic one shows; choosing a row still goes to the retail
//! menu (a Select / pad press), whose code starts a capture, flips a mod or steps the section. Pure: no statics of its own.
use super::pages::Page;
use super::text::{decode_bytes, lang_index};
use crate::retail::mouse_menu::{Kind, MenuSnap};
use crate::retail::{pcctl, pcmods};

/// Which PC page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PcKind {
	Controls,
	Mods,
}

impl PcKind {
	/// The in-level page this PC page is.
	pub fn page(self) -> Page {
		match self {
			PcKind::Controls => Page::PcControls,
			PcKind::Mods => Page::PcMods,
		}
	}
}

/// One row: the label and the value the classic page shows (the stepper row's value is its section, with the `< >` dropped).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PcRow {
	pub label: String,
	pub value: String,
	/// Row 0 of a page with sections: the "Page" stepper.
	pub stepper: bool,
	/// The table's text ids behind `label` / `value`. The page module answers its own ids only (0x7f50..); a retail id (the mod rows'
	/// On / Off are 0x4f5a / 0x4f5b) comes back empty from it, so the modern page resolves those with the game's own text (`pause_pc`).
	pub label_id: u32,
	pub value_id: u32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct PcPage {
	pub kind: PcKind,
	pub title: String,
	pub rows: Vec<PcRow>,
}

/// The PC page whose copy of the retail page lives at `page` (the page struct address of the menu snapshot).
pub fn kind_of(page: u32) -> Option<PcKind> {
	if page == pcctl::PAGE_AT {
		Some(PcKind::Controls)
	} else if page == pcmods::MPAGE {
		Some(PcKind::Mods)
	} else {
		None
	}
}

/// Words per record and the most rows a page can have (`pcctl::TABLE_RECS`: eight and the zero record).
const REC: u32 = 24;
const MAX_ROWS: u32 = 12;

/// A RAM address the PC tables can hold: the PC options region starts at `pcext::BASE` (0xB0000, below the game's 1 MB mark; the tables and
/// the bytes they point at live there) [C: pp run 2026-11-02, the table of the Controls page is 0xBAxxx].
fn in_ram(a: u32) -> bool {
	(crate::retail::pcext::BASE..0x200_0000).contains(&a)
}

fn rd8(rd: &dyn Fn(u32) -> u32, a: u32) -> u8 {
	(rd(a & !3) >> (8 * (a & 3))) as u8
}

/// A text of the page as plain text; the stepper's `< >` and the font's padding dropped.
fn plain(raw: &[u8]) -> String {
	decode_bytes(raw).plain().trim().to_string()
}

fn unstep(s: &str) -> String {
	s.trim_start_matches('<').trim_end_matches('>').trim().to_string()
}

/// The page on screen as rows; None when `m` is not a PC page or its table does not look right. `rd` = a 32-bit RAM word, `text` =
/// the page module's text of an id in the game's language.
pub fn read_with(kind: PcKind, m: &MenuSnap, rd: &dyn Fn(u32) -> u32, text: &dyn Fn(u32) -> Vec<u8>) -> Option<PcPage> {
	let it = m.items.iter().find(|i| matches!(i.kind, Kind::Value { .. }))?;
	let Kind::Value { n, .. } = it.kind else { return None };
	let table = rd(it.addr + 0x34);
	if n == 0 || n > MAX_ROWS || !in_ram(table) {
		return None;
	}
	let mut rows = Vec::new();
	for r in 0..n {
		let rec = table + REC * r;
		let (label_id, byte_at) = (rd(rec), rd(rec + 4));
		if label_id == 0 || !in_ram(byte_at) {
			return None;
		}
		let choice = (rd8(rd, byte_at) as u32).min(3);
		let mut value_id = rd(rec + 8 + 4 * choice);
		if value_id == 0 {
			value_id = rd(rec + 8);
		}
		let (label, value) = (plain(&text(label_id)), plain(&text(value_id)));
		let stepper = r == 0 && value.starts_with('<');
		rows.push(PcRow { label, value: if stepper { unstep(&value) } else { value }, stepper, label_id, value_id });
	}
	let title = plain(&text(match kind {
		PcKind::Controls => pcctl::ID_CONTROLS,
		PcKind::Mods => pcmods::ID_MODS,
	}));
	Some(PcPage { kind, title, rows })
}

/// `read_with` for the live pages: the texts come from `pcctl` / `pcmods` in the game's language (RAM word 0x15ed88).
pub fn read(m: &MenuSnap, rd: &dyn Fn(u32) -> u32) -> Option<PcPage> {
	let kind = kind_of(m.page)?;
	let lang = lang_index(rd(super::addrs::LANG_WORD));
	match kind {
		PcKind::Controls => read_with(kind, m, rd, &|id| pcctl::text_now(id, lang)),
		PcKind::Mods => read_with(kind, m, rd, &|id| pcmods::text_now(id, lang)),
	}
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::mouse_menu::Item;
	use std::collections::HashMap;

	const TABLE: u32 = 0x4a_0000;
	const BYTES: u32 = 0x4b_0000;

	fn snap(n: u32) -> MenuSnap {
		let it = Item { addr: 0x40, slot: 3, rect: [0, 0, 100, 100], kind: Kind::Value { n, cursor: 0 }, texts: Vec::new(), planet_texts: Vec::new() };
		MenuSnap { page: pcctl::PAGE_AT, id: 0x1d, state: 0x1d, cur: 0x40, items: vec![it], front: false }
	}

	/// Records `(label, value0, value1, byte)`.
	fn ram(recs: &[(u32, u32, u32, u8)]) -> HashMap<u32, u32> {
		let mut w = HashMap::new();
		w.insert(0x40 + 0x34, TABLE);
		for (i, &(l, v0, v1, b)) in recs.iter().enumerate() {
			let at = TABLE + REC * i as u32;
			w.extend([(at, l), (at + 4, BYTES + i as u32), (at + 8, v0), (at + 12, v1)]);
			w.insert((BYTES + i as u32) & !3, w.get(&((BYTES + i as u32) & !3)).copied().unwrap_or(0) | ((b as u32) << (8 * ((BYTES + i as u32) & 3))));
		}
		w
	}

	fn texts(id: u32) -> Vec<u8> {
		match id {
			1 => b"Page".to_vec(),
			2 => b"< Keyboard buttons 1/2 >".to_vec(),
			3 => b"Cross".to_vec(),
			4 => b"Space".to_vec(),
			5 => b"Press a key".to_vec(),
			pcctl::ID_CONTROLS => b"Controls".to_vec(),
			_ => Vec::new(),
		}
	}

	#[test]
	fn the_pc_options_region_counts_as_ram() {
		// The real tables (0xBAxxx / 0xBCxxx) are below the game's 1 MB mark: the first pictures of the pages fell back to classic on it.
		assert!(in_ram(0xba_400) && in_ram(0xbc_100) && in_ram(0x4a_0000));
		assert!(!in_ram(0) && !in_ram(0x1000) && !in_ram(0x200_0000));
	}

	#[test]
	fn the_table_reads_into_labelled_rows() {
		let w = ram(&[(1, 2, 2, 0), (3, 4, 5, 0), (3, 4, 5, 1)]);
		let rd = |a: u32| w.get(&a).copied().unwrap_or(0);
		let p = read_with(PcKind::Controls, &snap(3), &rd, &texts).unwrap();
		assert_eq!(p.title, "Controls");
		assert_eq!(p.rows.len(), 3);
		assert!(p.rows[0].stepper);
		assert_eq!((p.rows[0].label.as_str(), p.rows[0].value.as_str()), ("Page", "Keyboard buttons 1/2"));
		assert_eq!(p.rows[1].value, "Space", "byte 0 shows the bound key");
		assert_eq!(p.rows[2].value, "Press a key", "byte 1 is the capture");
		assert!(!p.rows[1].stepper);
	}

	#[test]
	fn a_table_that_does_not_look_right_gives_no_page() {
		let w = ram(&[(1, 2, 2, 0)]);
		let rd = |a: u32| w.get(&a).copied().unwrap_or(0);
		assert!(read_with(PcKind::Controls, &snap(0), &rd, &texts).is_none(), "no rows");
		assert!(read_with(PcKind::Controls, &snap(30), &rd, &texts).is_none(), "too many rows");
		assert!(read_with(PcKind::Controls, &snap(2), &rd, &texts).is_none(), "the second record is empty");
		let none = |_: u32| 0u32;
		assert!(read_with(PcKind::Controls, &snap(1), &none, &texts).is_none(), "no table pointer");
	}

	#[test]
	fn the_two_pc_pages_are_told_apart_by_their_copy() {
		assert_eq!(kind_of(pcctl::PAGE_AT), Some(PcKind::Controls));
		assert_eq!(kind_of(pcmods::MPAGE), Some(PcKind::Mods));
		assert_eq!(kind_of(0x1b4bc0), None);
		assert_eq!(PcKind::Mods.page(), Page::PcMods);
	}
}
