//! The facts the modern pause pages need that a `MenuSnap` does not carry (v0.5 screens stage C): the five save slots of the Save /
//! Load / Overwrite pages, the on/off state behind the toggle rows of HelpDesk Options and Subtitles, and the Sound page's volumes and
//! stereo flag. Read on the WORKER in `snapshot.rs snap()` right after the menu (RAM is only there), published with the snap, never
//! written. Only the page on screen is read; a layout that does not look right yields no rows and that page stays classic.
use super::addrs::{planet_flags, PlanetFlag};
use super::pages::Page;
use crate::retail::disc::va;
use crate::retail::mouse_menu::{Kind, MenuSnap};

/// A save slot as the front end keeps it: level, bolts, play-time frames, date bytes (`front_end.rs front_snap` reads the same).
pub type Slot = (u32, u32, u32, [u8; 3]);

/// Slot 0 of the five 0x1c-byte slot summaries (level -1 = empty; +4 bolts, +0xc frames, +0x15..0x17 date).
const SLOTS_AT: u32 = 0x13d2b0;
const SLOT_STRIDE: u32 = 0x1c;
/// The gold-bolt collection bytes, 4 per planet (core, `trophies.rs GOLD_BOLTS`; the details page counts the nonzero ones).
const GOLD_AT: u32 = 0x14bec0;
/// A toggle row is 5 words {text, byte*, onText, offText, flags} (decomp/notes/ingame_pause_menu.md "Toggle list").
const TOGGLE_STRIDE: u32 = 20;
/// The Sound page: stereo flag byte, music volume and effects volume words; volumes run 0..0x400.
const STEREO_AT: u32 = 0x15ede8;
const MUSIC_AT: u32 = 0x15edec;
const SFX_AT: u32 = 0x15edf0;
pub const VOLUME_MAX: u32 = 0x400;
/// The value texts of the stereo row (ingame_menu_leftovers.md: 0 = MONO 0x5215, else STEREO 0x5216).
pub const TEXT_MONO: u32 = 0x5215;
pub const TEXT_STEREO: u32 = 0x5216;
/// The owned-item bytes (one per item id), the story collectibles of the Items page and the ammo words (4 per id).
const OWNED_AT: u32 = 0x13d4c0;
const COLLECT_AT: u32 = 0x13d388;
const AMMO_AT: u32 = 0x13d428;
/// The eight quick-select ids (one word each; the in-game ring's slots, 0 = empty). The pause page's ring item works on its own copy.
const QUICK_AT: u32 = 0x141ea0;
/// The ring item's copy of the eight ids (FUN_0027e100 draws it; +0x50 is the cursor).
pub const RING_IDS: u32 = 0x30;
pub const RING_CURSOR: u32 = 0x50;
/// How many item ids exist (`inventory::GADGETS`); a grid cell with another id is empty.
const ITEM_IDS: u32 = 37;
/// The Gadgets page's grid items, in the order of the groups Hand, Back, Head, Foot (page 4 slots).
const GADGET_SLOTS: [usize; 4] = [2, 3, 9, 10];
/// Where every text id of the retail pages lies (`mouse_menu::item_texts`).
const TEXT_IDS: std::ops::RangeInclusive<u32> = 0x4e00..=0x5700;

/// One toggle row: its label text, whether the byte behind it is set, and the two value texts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Toggle {
	pub text: u32,
	pub on: bool,
	pub on_text: u32,
	pub off_text: u32,
}

/// The Sound page's state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Sound {
	pub sfx: u32,
	pub music: u32,
	pub stereo: bool,
}

/// The ship map's per-planet facts (v0.5 screens stage F3): retail's details page shows the gold bolts found and the map marks a
/// planet visited or only unlocked; both live in core words (the same address on both discs, below 0x15ef00).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ShipFacts {
	/// Unlocked / visited per planet id (`addrs::planet_flags`).
	pub flags: Vec<PlanetFlag>,
	/// Gold bolts found per planet id: the nonzero bytes of the 4 at `GOLD_AT + 4 * planet` (FUN_0027c550).
	pub gold: Vec<u32>,
}

impl ShipFacts {
	pub fn flag(&self, planet: u32) -> Option<PlanetFlag> {
		self.flags.get(planet as usize).copied()
	}

	pub fn gold_found(&self, planet: u32) -> u32 {
		self.gold.get(planet as usize).copied().unwrap_or(0)
	}
}

/// One cell of an icon grid (Weapons, Gadgets, Items): the item id it shows, whether the player has it, and a weapon's ammo.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridCell {
	pub id: u8,
	/// The cell's kind halfword (+4); on the Items page 1 = a story collectible (byte 0x13d388 + id), else an owned item.
	pub kind: u8,
	pub owned: bool,
	/// Weapons page, owned weapon with an ammo cap: (rounds now, most it holds). None = no ammo to show ("No ammo").
	pub ammo: Option<(u32, u32)>,
}

/// The Quick Select page's ring: the ring item at `addr` (the game's cursor slot 0..7 at +0x50) and the eight item ids it holds
/// (the item's copy at +0x30, one word each, 0 = an empty slot; `QUICK_AT` before the page entered) [C: RAC_MENU_LOG of page 0x5 on
/// Kerwan, 2026-11-02: slots {10, 12, 0, ...}; FUN_00279fa0 / FUN_00279d78 / FUN_0027a038, 2026-11-03]. Slot i sits at i x 45 degrees
/// clockwise from the top in retail's page ring and in the in-game ring alike.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RingFacts {
	pub addr: u32,
	pub cursor: u32,
	pub ids: [u8; 8],
}

/// One icon grid of the page on screen (the item at `addr`, `rows` x `cols` cells, row by row).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GridFacts {
	pub addr: u32,
	pub rows: u32,
	pub cols: u32,
	pub cells: Vec<GridCell>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Extra {
	/// Save / Load / Overwrite: the five slots (None = empty). Empty on any other page.
	pub slots: Vec<Option<Slot>>,
	/// HelpDesk Options / Subtitles: the rows of the page's toggle list. Empty on any other page or when the list did not look right.
	pub toggles: Vec<Toggle>,
	/// Sound Options.
	pub sound: Option<Sound>,
	/// Galactic Map (pages 0xf and 0x10): the planets' flags and gold bolts.
	pub ship: Option<ShipFacts>,
	/// Weapons (one grid), Gadgets (four: Hand, Back, Head, Foot) and Items (one): the icon grids with their cells. Empty elsewhere.
	pub grids: Vec<GridFacts>,
	/// The PC Controls / Mods page (the Camera page's copy at its own address): its rows with the texts the classic page shows.
	pub pc: Option<super::pcpage::PcPage>,
	/// Quick Select: the ring (`grids` then holds its two grids: the weapons 3 x 5 first, the gadgets 2 x 3 second).
	pub ring: Option<RingFacts>,
	/// The planet map (0xb) and its mission list (0xc): the planet their titles name (`map_seen`; None = no title looked up lately);
	/// `ship` then holds the gold bolts found.
	pub map: Option<crate::retail::map_seen::Seen>,
	/// The Help page (7): the text id of the description of the row under the cursor (`help_seen`; None = not looked up lately).
	pub help_desc: Option<u32>,
}

/// The in-level page menu with everything the modern pause menu reads: the worker's extra facts and the header figures.
#[derive(Clone, Copy, Debug)]
pub struct LevelMenu<'a> {
	pub menu: &'a MenuSnap,
	pub extra: &'a Extra,
	pub level: u32,
	pub bolts: u32,
}

fn rd8(rd: &dyn Fn(u32) -> u32, a: u32) -> u8 {
	(rd(a & !3) >> (8 * (a & 3))) as u8
}

/// Is `a` a pointer into the EE's RAM (a byte flag of the toggle rows)?
fn ram(a: u32) -> bool {
	(0x10_0000..0x200_0000).contains(&a)
}

/// The five slot summaries.
pub fn read_slots(rd: &dyn Fn(u32) -> u32) -> Vec<Option<Slot>> {
	(0..5u32)
		.map(|i| {
			let a = SLOTS_AT + i * SLOT_STRIDE;
			let level = rd(a);
			(level < 19).then(|| (level, rd(a + 4), rd(a + 0xc), [rd8(rd, a + 0x15), rd8(rd, a + 0x16), rd8(rd, a + 0x17)]))
		})
		.collect()
}

/// The `n` rows of the toggle list at `rows`; empty when any row's ids or byte pointer look wrong.
fn read_toggles(rd: &dyn Fn(u32) -> u32, rows: u32, n: u32) -> Vec<Toggle> {
	if !ram(rows) || !(1..=4).contains(&n) {
		return Vec::new();
	}
	let mut out = Vec::new();
	for k in 0..n {
		let a = rows + TOGGLE_STRIDE * k;
		let (text, flag, on_text, off_text) = (rd(a) & 0xffff, rd(a + 4), rd(a + 8) & 0xffff, rd(a + 12) & 0xffff);
		if !ram(flag) || ![text, on_text, off_text].iter().all(|t| TEXT_IDS.contains(t)) {
			return Vec::new();
		}
		out.push(Toggle { text, on: rd8(rd, flag) != 0, on_text, off_text });
	}
	out
}

fn read_sound(rd: &dyn Fn(u32) -> u32) -> Option<Sound> {
	let (sfx, music) = (rd(va(SFX_AT)), rd(va(MUSIC_AT)));
	(sfx <= VOLUME_MAX && music <= VOLUME_MAX).then(|| Sound { sfx, music, stereo: rd8(rd, va(STEREO_AT)) != 0 })
}

/// The ship map's planet flags and the gold bolts found per planet (19 planets, one word of 4 collection bytes each).
pub fn read_ship(rd: &dyn Fn(u32) -> u32) -> ShipFacts {
	let gold = (0..19u32).map(|p| rd(GOLD_AT + 4 * p).to_le_bytes().iter().filter(|&&b| b != 0).count() as u32).collect();
	ShipFacts { flags: planet_flags(rd), gold }
}

fn rd16(rd: &dyn Fn(u32) -> u32, a: u32) -> u32 {
	(rd(a & !3) >> (8 * (a & 2))) & 0xffff
}

/// Which grid items of the page `page` are the page's icon grids, in the order the modern page lists them: the Gadgets page's four
/// groups by slot (Hand 2, Back 3, Head 9, Foot 10: the order of retail's `GADGET_LISTS` labels), the biggest grid on Weapons / Items.
fn grid_items(m: &MenuSnap, page: Page) -> Vec<&crate::retail::mouse_menu::Item> {
	let mut g: Vec<_> = m.items.iter().filter(|i| matches!(i.kind, Kind::Grid { .. })).collect();
	let cells = |i: &crate::retail::mouse_menu::Item| if let Kind::Grid { rows, cols, .. } = i.kind { rows * cols } else { 0 };
	match page {
		Page::Gadgets => {
			g.retain(|i| GADGET_SLOTS.contains(&i.slot));
			g.sort_by_key(|i| GADGET_SLOTS.iter().position(|&s| s == i.slot));
			g
		}
		Page::Weapons | Page::Items => {
			g.sort_by_key(|i| std::cmp::Reverse(cells(i)));
			g.truncate(1);
			g
		}
		// Quick Select: the weapons grid (3 x 5) and below it the gadgets grid (2 x 3), the two biggest.
		Page::QuickSelect => {
			g.sort_by_key(|i| std::cmp::Reverse(cells(i)));
			g.truncate(2);
			g
		}
		_ => Vec::new(),
	}
}

/// The Quick Select ring of the page `m` on `level`, as retail's own ring draw shows it (FUN_0027e100 draws item +0x30 and the cursor
/// +0x50). The page works on the ring item's copy: its enter callback copies `QUICK_AT` in and sets the cursor (`entry_cursor`), Cross
/// writes the copy, its leave callback copies it back to `QUICK_AT` (decomp/notes/ingame_pause_menu.md "Quick Select ring"). So a page
/// that has landed reads the copy (live: an Equip shows in the field it happens), a page still flying in (not entered yet: the copy and
/// the cursor are those of the last visit, or the overlay's zeros) reads what its entry will copy.
fn read_ring(m: &MenuSnap, level: u32, rd: &dyn Fn(u32) -> u32) -> Option<RingFacts> {
	let (addr, cursor) = m.items.iter().find_map(|i| if let Kind::Ring { cursor } = i.kind { Some((i.addr, cursor)) } else { None })?;
	let entered = crate::retail::mouse_menu::page_ptr(level).is_none_or(|pp| rd(pp) == m.page);
	let from = if entered { addr + RING_IDS } else { QUICK_AT };
	let mut ids = [0u8; 8];
	for (i, id) in ids.iter_mut().enumerate() {
		*id = rd(from + 4 * i as u32) as u8;
	}
	let cursor = if entered { cursor & 7 } else { entry_cursor(&ids) };
	Some(RingFacts { addr, cursor, ids })
}

/// The ring cursor the page's enter callback sets (FUN_00279fa0): the first empty slot, 0 when every slot holds an item.
pub fn entry_cursor(ids: &[u8; 8]) -> u32 {
	ids.iter().position(|&id| id == 0).map_or(0, |i| i as u32)
}

/// The icon grids of the Weapons / Gadgets / Items page `m`: cells {icon, frame, kind, id, 0} (10 bytes each) at item + 0x48. A cell is
/// owned when its byte is set (0x13d4c0 + id; on Items a kind-1 cell uses the collectible byte 0x13d388 + id). `level` finds the weapons'
/// ammo caps (the price record table of `vendor::words`); an unknown level reads no ammo.
fn read_grids(m: &MenuSnap, page: Page, level: u32, rd: &dyn Fn(u32) -> u32) -> Vec<GridFacts> {
	let p0 = crate::retail::vendor::words(level).map(|w| w.p0);
	let mut out = Vec::new();
	for it in grid_items(m, page) {
		let Kind::Grid { rows, cols, .. } = it.kind else { continue };
		let cells = rd(it.addr + 0x48);
		if !ram(cells) {
			continue;
		}
		let cell = |k: u32| {
			let a = cells + 10 * k;
			let (kind, id) = (rd16(rd, a + 4), rd16(rd, a + 6));
			let valid = id < ITEM_IDS;
			let at = if page == Page::Items && kind == 1 { COLLECT_AT } else { OWNED_AT };
			let owned = valid && rd8(rd, at + id) != 0;
			let max = p0.filter(|_| owned && page == Page::Weapons).map_or(0, |p| rd16(rd, p + id * 0x18 + 0xe));
			let ammo = (max > 0).then(|| (rd(AMMO_AT + 4 * id), max));
			GridCell { id: id as u8, kind: kind as u8, owned, ammo }
		};
		out.push(GridFacts { addr: it.addr, rows, cols, cells: (0..rows * cols).map(cell).collect() });
	}
	out
}

/// The extra facts of the in-level page menu `menu` (None = no in-level menu on screen), level unknown (no ammo caps). `rd` = a 32-bit word read of RAM.
pub fn read(menu: Option<&MenuSnap>, rd: &dyn Fn(u32) -> u32) -> Option<Extra> {
	read_at(menu, u32::MAX, rd)
}

/// `read` for the level `level` (the worker's snapshot passes the running level).
pub fn read_at(menu: Option<&MenuSnap>, level: u32, rd: &dyn Fn(u32) -> u32) -> Option<Extra> {
	let m = menu.filter(|m| !m.front)?;
	let mut x = Extra::default();
	match Page::from_id(m.id) {
		Page::Save | Page::Load | Page::Overwrite => x.slots = read_slots(rd),
		Page::HelpDesk | Page::Subtitles => {
			// The toggle list is the value item in slot 3 (`mouse_menu::classify`): its +0x34 is the row table.
			if let Some(it) = m.items.iter().find(|i| i.slot == 3 && matches!(i.kind, Kind::Value { .. })) {
				if let Kind::Value { n, .. } = it.kind {
					x.toggles = read_toggles(rd, rd(it.addr + 0x34), n);
				}
			}
		}
		Page::Sound => x.sound = read_sound(rd),
		Page::Help => x.help_desc = crate::retail::help_seen::desc(crate::retail::prompts::field_no()),
		Page::Camera => x.pc = super::pcpage::read(m, rd),
		Page::GalaxyMap | Page::GalaxyDetails => x.ship = Some(read_ship(rd)),
		Page::PauseMap | Page::Missions => {
			x.ship = Some(read_ship(rd));
			x.map = crate::retail::map_seen::shown(crate::retail::prompts::field_no());
		}
		p @ (Page::Weapons | Page::Gadgets | Page::Items | Page::QuickSelect) => {
			x.grids = read_grids(m, p, level, rd);
			x.ring = (p == Page::QuickSelect).then(|| read_ring(m, level, rd)).flatten();
		}
		_ => {}
	}
	Some(x)
}

#[cfg(test)]
mod tests {
	use super::*;
	use crate::retail::disc::tests::as_usa;
	use crate::retail::mouse_menu::Item;
	use std::collections::HashMap;

	fn ram_of(words: &[(u32, u32)]) -> HashMap<u32, u32> {
		words.iter().copied().collect()
	}

	fn value_item(addr: u32, n: u32) -> Item {
		Item { addr, slot: 3, rect: [10, 10, 200, 100], kind: Kind::Value { n, cursor: 0 }, texts: Vec::new(), planet_texts: Vec::new() }
	}

	fn menu(id: u32, items: Vec<Item>) -> MenuSnap {
		MenuSnap { page: 0x1b4bc0, id, state: id, cur: 0, items, front: false }
	}

	#[test]
	fn slots_read_used_and_empty() {
		let mut w = vec![(SLOTS_AT, 3), (SLOTS_AT + 4, 1234), (SLOTS_AT + 0xc, 216_000 + 7200), (SLOTS_AT + 0x14, 0x03_02_01_00)];
		w.extend((1..5).map(|i| (SLOTS_AT + i * SLOT_STRIDE, u32::MAX)));
		let ram = ram_of(&w);
		let s = read_slots(&|a| ram.get(&a).copied().unwrap_or(0));
		assert_eq!(s.len(), 5);
		assert_eq!(s[0], Some((3, 1234, 223_200, [0x01, 0x02, 0x03])));
		assert!(s[1..].iter().all(|x| x.is_none()), "level -1 is an empty slot");
	}

	#[test]
	fn toggles_read_their_bytes_and_texts() {
		let rows = 0x27e408;
		let flags = 0x15ee1c;
		let ram = ram_of(&[(rows, 0x4f9f), (rows + 4, flags), (rows + 8, 0x4f5a), (rows + 12, 0x4f5b), (rows + 20, 0x4fa0), (rows + 24, flags + 1), (rows + 28, 0x4f5a), (rows + 32, 0x4f5b), (flags & !3, 0x0000_0100)]);
		let rd = |a: u32| ram.get(&a).copied().unwrap_or(0);
		let t = read_toggles(&rd, rows, 2);
		assert_eq!(t.len(), 2);
		assert_eq!((t[0].text, t[0].on, t[0].on_text, t[0].off_text), (0x4f9f, false, 0x4f5a, 0x4f5b));
		assert!(t[1].on, "byte 0x15ee1d is set");
		// A table that does not look right gives no rows (the page stays classic).
		assert!(read_toggles(&|a| if a == rows { 0x1234 } else { rd(a) }, rows, 2).is_empty(), "label id out of range");
		assert!(read_toggles(&|a| if a == rows + 4 { 0 } else { rd(a) }, rows, 2).is_empty(), "no byte pointer");
		assert!(read_toggles(&rd, 0, 2).is_empty() && read_toggles(&rd, rows, 0).is_empty());
	}

	#[test]
	fn read_picks_the_facts_of_the_page_on_screen() {
		let rows = 0x27e408;
		let ram = ram_of(&[(0x1b4bc0 + 0x34, rows), (rows, 0x4f24), (rows + 4, 0x15ee40), (rows + 8, 0x4f5a), (rows + 12, 0x4f5b), (SFX_AT, 0x400), (MUSIC_AT, 0x2cc), (STEREO_AT, 1)]);
		let rd = |a: u32| ram.get(&a).copied().unwrap_or(0);
		let sub = menu(0x1b, vec![Item { addr: 0x1b4bc0, ..value_item(0x1b4bc0, 1) }]);
		let x = read(Some(&sub), &rd).unwrap();
		assert_eq!(x.toggles.len(), 1);
		assert!(x.slots.is_empty() && x.sound.is_none());
		let snd = read(Some(&menu(0x20, vec![value_item(0x1b5598, 3)])), &rd).unwrap();
		assert_eq!(snd.sound, Some(Sound { sfx: 0x400, music: 0x2cc, stereo: true }));
		assert_eq!(read(Some(&menu(0x1e, Vec::new())), &rd).unwrap().slots.len(), 5);
		assert_eq!(read(Some(&menu(8, Vec::new())), &rd), Some(Extra::default()), "the Options list needs nothing extra");
		let front = MenuSnap { front: true, ..menu(0x20, Vec::new()) };
		assert_eq!((read(Some(&front), &rd), read(None, &rd)), (None, None), "no in-level menu, no facts");
	}

	#[test]
	fn a_volume_out_of_range_is_not_a_sound_page() {
		assert_eq!(read_sound(&|a| if a == SFX_AT { 0x401 } else { 0 }), None);
	}

	/// Synthetic RAM with byte writes (words keyed by their aligned address).
	#[derive(Default)]
	struct Ram(HashMap<u32, u32>);

	impl Ram {
		fn b(&mut self, a: u32, v: u8) {
			let w = self.0.entry(a & !3).or_insert(0);
			*w = (*w & !(0xff << (8 * (a & 3)))) | (v as u32) << (8 * (a & 3));
		}
		fn h(&mut self, a: u32, v: u16) {
			self.b(a, v as u8);
			self.b(a + 1, (v >> 8) as u8);
		}
		fn w(&mut self, a: u32, v: u32) {
			self.0.insert(a, v);
		}
		fn rd(&self) -> impl Fn(u32) -> u32 + '_ {
			|a| self.0.get(&a).copied().unwrap_or(0)
		}
		/// A grid item at `addr` whose cells (icon, frame, kind, id, 0) sit at `cells`.
		fn grid(&mut self, addr: u32, cells: u32, ids: &[(u16, u16)]) {
			self.w(addr + 0x48, cells);
			for (k, &(kind, id)) in ids.iter().enumerate() {
				let a = cells + 10 * k as u32;
				self.h(a + 4, kind);
				self.h(a + 6, id);
			}
		}
	}

	fn grid_item(addr: u32, slot: usize, rows: u32, cols: u32) -> Item {
		Item { addr, slot, rect: [10, 10, 100, 100], kind: Kind::Grid { rows, cols, cursor: 0 }, texts: Vec::new(), planet_texts: Vec::new() }
	}

	/// A Weapons grid of 5 cells (ids 4, 5, 6, 7, 0xff) with weapon 4 owned and loaded, 5 owned and ammoless, 6 not owned.
	fn weapons_ram(p0: u32) -> Ram {
		let mut r = Ram::default();
		r.grid(0x1b2e00, 0x1b3000, &[(0, 4), (0, 5), (0, 6), (0, 7), (0, 0xff)]);
		for id in [4u32, 5] {
			r.b(OWNED_AT + id, 1);
		}
		r.w(AMMO_AT + 16, 17);
		r.h(p0 + 4 * 0x18 + 0xe, 40);
		r.h(p0 + 6 * 0x18 + 0xe, 99);
		r
	}

	fn weapons_facts(level: u32, p0: u32) -> Vec<GridFacts> {
		let ram = weapons_ram(p0);
		let m = menu(3, vec![grid_item(0x1b2e00, 2, 1, 5), grid_item(0x1b2f00, 3, 1, 1)]);
		read_at(Some(&m), level, &ram.rd()).unwrap().grids
	}

	#[test]
	fn the_weapons_grid_reads_owned_and_ammo_on_both_discs() {
		for usa in [false, true] {
			let p0 = |l: u32| crate::retail::vendor::words(l).unwrap().p0;
			let (p0_0, g) = if usa {
				let p = as_usa(|| p0(0));
				(p, as_usa(|| weapons_facts(0, p)))
			} else {
				(p0(0), weapons_facts(0, p0(0)))
			};
			assert_eq!(g.len(), 1, "usa {usa}: the 1-cell item is no second grid");
			assert_eq!((g[0].addr, g[0].rows, g[0].cols, g[0].cells.len()), (0x1b2e00, 1, 5, 5));
			let c = &g[0].cells;
			assert_eq!((c[0].id, c[0].owned, c[0].ammo), (4, true, Some((17, 40))), "usa {usa} p0 {p0_0:#x}");
			assert_eq!((c[1].id, c[1].owned, c[1].ammo), (5, true, None), "an owned weapon with no cap has no ammo to show");
			assert_eq!((c[2].owned, c[2].ammo), (false, None), "not owned: no ammo even with a cap");
			assert_eq!((c[3].owned, c[4].owned), (false, false), "an empty cell (id 0xff) is never owned");
		}
	}

	#[test]
	fn an_unknown_level_reads_no_ammo_and_other_pages_no_grids() {
		let p0 = crate::retail::vendor::words(0).unwrap().p0;
		assert!(weapons_facts(u32::MAX, p0)[0].cells.iter().all(|c| c.ammo.is_none()));
		let ram = weapons_ram(p0);
		let m = menu(3, vec![grid_item(0x1b2e00, 2, 1, 5)]);
		assert_eq!(read(Some(&m), &ram.rd()).unwrap().grids[0].cells[0].ammo, None, "read = unknown level");
		let q = menu(5, vec![grid_item(0x1b2e00, 1, 1, 5)]);
		assert!(!read_at(Some(&q), 0, &ram.rd()).unwrap().grids.is_empty(), "Quick Select reads its grids since v5i (the two biggest)");
		let bad = Ram::default();
		assert!(read_at(Some(&m), 0, &bad.rd()).unwrap().grids.is_empty(), "a grid without a cell table is skipped");
	}

	#[test]
	fn the_gadgets_groups_come_in_the_order_hand_back_head_foot() {
		let mut r = Ram::default();
		// Items listed in memory order Foot, Head, Hand, Back plus a stray grid on another slot.
		r.grid(0x1b4000, 0x1b5000, &[(0, 29), (0, 28)]);
		r.grid(0x1b4100, 0x1b5100, &[(0, 6), (0, 5), (0, 7)]);
		r.grid(0x1b4200, 0x1b5200, &[(0, 26), (0, 22), (0, 12), (0, 32), (0, 27), (0, 31)]);
		r.grid(0x1b4300, 0x1b5300, &[(0, 2), (0, 3), (0, 4)]);
		r.grid(0x1b4400, 0x1b5400, &[(0, 1)]);
		r.b(OWNED_AT + 22, 1);
		r.b(OWNED_AT + 28, 1);
		let m = menu(4, vec![grid_item(0x1b4000, 10, 1, 2), grid_item(0x1b4100, 9, 1, 3), grid_item(0x1b4200, 2, 2, 3), grid_item(0x1b4300, 3, 1, 3), grid_item(0x1b4400, 5, 1, 1)]);
		let g = read_at(Some(&m), 3, &r.rd()).unwrap().grids;
		assert_eq!(g.iter().map(|g| g.addr).collect::<Vec<_>>(), [0x1b4200, 0x1b4300, 0x1b4100, 0x1b4000], "slots 2, 3, 9, 10");
		assert_eq!(g[0].cells.iter().map(|c| (c.id, c.owned)).collect::<Vec<_>>(), [(26, false), (22, true), (12, false), (32, false), (27, false), (31, false)]);
		assert!(g[3].cells[1].owned && !g[3].cells[0].owned);
		assert!(g.iter().flat_map(|g| &g.cells).all(|c| c.ammo.is_none()), "gadgets show no ammo");
	}

	#[test]
	fn the_items_page_reads_collectibles_by_kind() {
		let mut r = Ram::default();
		// ITEM_GRID order, shortened: item 0x1e owned (kind 0), collectible 4 (kind 1) set, collectible 5 not set although owned byte 5 is
		// (a kind-1 cell never reads the owned bytes), item 5 as a kind-0 cell is owned.
		r.grid(0x1b5e00, 0x1b6100, &[(0, 0x1e), (1, 4), (1, 5), (0, 5)]);
		r.b(OWNED_AT + 0x1e, 1);
		r.b(OWNED_AT + 5, 1);
		r.b(COLLECT_AT + 4, 1);
		let m = menu(6, vec![grid_item(0x1b5e00, 3, 2, 2)]);
		let x = read_at(Some(&m), 0, &r.rd()).unwrap();
		let c = &x.grids[0].cells;
		assert_eq!(c.iter().map(|c| (c.id, c.kind, c.owned)).collect::<Vec<_>>(), [(0x1e, 0, true), (4, 1, true), (5, 1, false), (5, 0, true)]);
	}
}
