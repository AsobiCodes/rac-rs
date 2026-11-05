//! Unit tests of the menu bridge on synthetic RAM maps (the `mouse_menu/tests.rs` galaxy pattern) and synthetic text banks.
use super::addrs::{self, P_LOAD, P_NEW};
use super::cmd::{self, Command, Intent, Pulses};
use super::pages::{route, Page};
use super::text::{decode, decode_bytes, font_char, lang_index, Bank, Btn, Span, Strings};
use super::view::{galaxy_rows, slot_views, view, Body};
use crate::frontend::Text;
use crate::modern_menu::hide::Screen;
use crate::retail::disc::tests::as_usa;
use crate::retail::menu_keys;
use crate::retail::mouse_instant::{Cmd, Request};
use crate::retail::mouse_menu::{self, Item, Kind, MenuSnap, CIRCLE, CROSS, L1, R1, TRIANGLE};
use crate::retail::mouse_slider::Slide;
use std::collections::HashMap;

fn text(a: u8) -> Span {
	Span::Text(String::from(a as char))
}

fn bytes(b: &[u8]) -> String {
	b.iter().map(|&c| c as char).collect()
}

// ---- text ---------------------------------------------------------------------------------------------------------------

#[test]
fn every_mapped_char_is_covered_by_the_ui_font() {
	let set = rac_engine::ui::font::FontSet::get();
	let missing: Vec<(u8, char)> = (0u8..=0xb0).filter_map(|b| font_char(b).map(|c| (b, c))).filter(|&(_, c)| !set.covers(c)).collect();
	assert!(missing.is_empty(), "the UI font lacks {missing:x?}");
}

#[test]
fn control_bytes_are_not_characters() {
	for b in (0u8..0x20).chain([0x7f]) {
		assert_eq!(font_char(b), None, "{b:#x}");
	}
	assert_eq!(font_char(0x41), Some('A'));
	assert_eq!(font_char(0xb0), Some('\u{153}'));
	assert_eq!(font_char(0xad), Some('\u{2014}'));
}

#[test]
fn accents_decode_in_every_language() {
	assert_eq!(decode(&bytes(b"Gr\x86ce \x85 tes")).plain(), "Gr\u{e2}ce \u{e0} tes");
	assert_eq!(decode(&bytes(b"\xafAlguna")).plain(), "\u{bf}Alguna");
	assert_eq!(decode(&bytes(b"K\x9fder")).plain(), "K\u{f6}der");
	assert_eq!(decode(&bytes(b"Stra\xace \xa8 \xa9 \xaa \xab \xae!")).plain(), "Stra\u{df}e \u{d1} \u{f1} \u{c7} \u{e7} \u{a1}!");
	// The vowel blocks: upper acute / grave / circumflex / umlaut, then lower case.
	assert_eq!(decode_bytes(&[0x80, 0x84, 0x88, 0x8f, 0x97, 0xa3, 0xa7]).plain(), "\u{c1}\u{e1}\u{c9}\u{eb}\u{ef}\u{dc}\u{fc}");
}

#[test]
fn button_glyphs_marks_and_breaks() {
	assert_eq!(decode("\u{10} Equip").0, vec![Span::Button(Btn::Cross), Span::Text(" Equip".into())]);
	assert_eq!(decode("\u{c}Infobot\u{8}").0, vec![Span::Mark(true), Span::Text("Infobot".into()), Span::Mark(false)]);
	assert_eq!(decode("a\u{1}b").0, vec![text(b'a'), Span::Break, text(b'b')]);
	assert_eq!(decode("a\u{1}b").plain(), "a b");
	let all: String = (0x10u8..=0x19).map(|b| b as char).collect();
	assert_eq!(decode(&all).buttons(), vec![Btn::Cross, Btn::Circle, Btn::Triangle, Btn::Square, Btn::L1, Btn::R1, Btn::L2, Btn::R2, Btn::StickR, Btn::StickL]);
	assert_eq!(decode("\u{11} Yes").plain_with(|b| format!("[{}]", b.label())), "[Circle] Yes");
	// An unknown control byte shows nothing; a real Unicode char stays.
	assert_eq!(decode("a\u{2}b\u{20ac}").plain(), "ab\u{20ac}");
	assert_eq!(decode_bytes(b"ab\0cd").plain(), "ab");
}

#[test]
fn buttons_map_to_pad_bits_and_symbols() {
	assert_eq!(Btn::from_glyph(0x0f), None);
	assert_eq!(Btn::from_glyph(0x1a), None);
	assert_eq!([Btn::Cross, Btn::Circle, Btn::Triangle, Btn::L1, Btn::R1].map(Btn::pad_bit), [CROSS, CIRCLE, TRIANGLE, L1, R1]);
	assert_eq!(Btn::from_pad(CROSS), Some(Btn::Cross));
	assert_eq!(Btn::from_pad(0), None);
	assert_eq!(Btn::StickL.pad_bit(), 0);
	assert!(Btn::Square.symbol().is_some() && Btn::L1.symbol().is_none());
}

#[test]
fn language_words_map_to_text_indexes() {
	assert_eq!([0, 2, 3, 4, 5, 9].map(lang_index), [0, 1, 2, 3, 4, 4]);
}

#[test]
fn the_same_id_reads_per_language_and_a_bank_loads_once() {
	let mut en = Text::default();
	en.insert(0x4e00, "Gear".into());
	let mut fr = Text::default();
	fr.insert(0x4e00, bytes(b"\x10 \x88quipement"));
	assert_eq!(Strings(&en).plain(0x4e00), "Gear");
	assert_eq!(Strings(&fr).plain(0x4e00), " \u{c9}quipement");
	assert!(Strings(&fr).has(0x4e00) && !Strings(&fr).has(0x4e01));
	assert_eq!(Strings(&fr).rich(0x4e00).buttons(), vec![Btn::Cross]);
	let mut bank = Bank::default();
	let mut loads = 0;
	for _ in 0..3 {
		let got = bank.get_or_load(2, |i| {
			loads += 1;
			assert_eq!(i, 1);
			Some(Text::default())
		});
		assert!(got.is_some());
	}
	assert_eq!(loads, 1);
	assert!(bank.get_or_load(5, |_| None).is_none());
}

// ---- addresses ----------------------------------------------------------------------------------------------------------

/// A page menu in synthetic RAM: the page pointer `pp`, one list item with `n` records {text 0x4eb9 + p, type 1, text2 0x4ecc + p}.
fn ram_menu(pp: u32, id: u32, n: u32, cursor: u32) -> HashMap<u32, u32> {
	let mut w = HashMap::<u32, u32>::new();
	let (page, list, data) = (0x1b3d98u32, 0x1b3f98u32, 0x1b5000u32);
	w.extend([(pp, page), (pp - 4, 2), (page + 0x3c, id), (page + 0x40, list), (page + 0x44, list)]);
	for (k, v) in [20, 86, 144, 251].into_iter().enumerate() {
		w.insert(list + 0x18 + 4 * k as u32, v);
	}
	w.extend([(list + 0x34, data), (list + 0x40, cursor)]);
	for p in 0..n {
		w.insert(data + 12 * p, (0x4eb9 + p) | 1 << 16);
		w.insert(data + 12 * p + 8, 0x4ecc + p);
	}
	w
}

#[test]
fn the_page_pointer_differs_by_disc_by_zero_or_0x80_on_all_levels() {
	for level in 0..19u32 {
		let pal = addrs::page_ptr(level).unwrap();
		let usa = as_usa(|| addrs::page_ptr(level)).unwrap();
		assert!([0, 0x80].contains(&(usa - pal)), "level {level}: {pal:#x} {usa:#x}");
	}
	assert_eq!(addrs::page_ptr(19), None);
	assert_eq!((addrs::page_ptr(4).unwrap(), as_usa(|| addrs::page_ptr(4)).unwrap()), (0x1b9df4, 0x1b9e74));
}

#[test]
fn a_menu_is_read_back_on_either_disc_from_its_own_page_pointer() {
	let level = 4;
	let (pal, usa) = (addrs::page_ptr(level).unwrap(), as_usa(|| addrs::page_ptr(level)).unwrap());
	assert_ne!(pal, usa);
	let rd_pal = ram_menu(pal, 3, 2, 0);
	let rd_usa = ram_menu(usa, 3, 2, 0);
	let word = |w: &HashMap<u32, u32>| {
		let w = w.clone();
		move |a: u32| w.get(&a).copied().unwrap_or(0)
	};
	assert_eq!(mouse_menu::read(&word(&rd_pal), level, mouse_menu::MODE).map(|m| m.id), Some(3));
	assert_eq!(mouse_menu::read(&word(&rd_usa), level, mouse_menu::MODE), None);
	assert_eq!(as_usa(|| mouse_menu::read(&word(&rd_usa), level, mouse_menu::MODE)).map(|m| m.id), Some(3));
	assert_eq!(as_usa(|| mouse_menu::read(&word(&rd_pal), level, mouse_menu::MODE)), None);
	assert_eq!(mouse_menu::read(&word(&rd_pal), level, 4), None);
}

#[test]
fn the_galactic_map_pages_are_read_in_game_mode_3_and_not_during_the_takeoff_cutscene_on_either_disc() {
	let word = |w: HashMap<u32, u32>| move |a: u32| w.get(&a).copied().unwrap_or(0);
	for level in [0u32, 3, 4, 18] {
		let (pal, usa) = (addrs::page_ptr(level).unwrap(), as_usa(|| addrs::page_ptr(level)).unwrap());
		for id in [menu_keys::GALAXY, menu_keys::GALAXY_CHILD] {
			let rd = word(ram_menu(pal, id, 3, 1));
			let m = mouse_menu::read(&rd, level, 3).expect("the ship map is a page menu of game mode 3");
			assert_eq!((m.id, m.items.len()), (id, 1), "level {level} page {id:#x}");
			assert_eq!(mouse_menu::read(&rd, level, 6), None, "mode 6 (the takeoff stream) has no menu");
			let r = route(&m);
			assert_eq!((r.page, r.screen, r.ready), (if id == menu_keys::GALAXY { Page::GalaxyMap } else { Page::GalaxyDetails }, Screen::Ship, true));
			assert!(super::extra::read(Some(&m), &rd).is_some_and(|x| x.ship.is_some()), "the ship facts are read with the map page");
			let rd_usa = word(ram_menu(usa, id, 3, 1));
			assert_eq!(as_usa(|| mouse_menu::read(&rd_usa, level, 3)).map(|m| m.id), Some(id), "USA reads it from its own page pointer");
			assert_eq!(as_usa(|| mouse_menu::read(&rd_usa, level, 6)), None);
		}
	}
}

#[test]
fn planet_flags_read_the_two_byte_arrays() {
	let mut w = HashMap::<u32, u32>::new();
	w.insert(addrs::UNLOCKED & !3, 0x0000_0101);
	w.insert(addrs::VISITED & !3, 0x0000_0100);
	let flags = addrs::planet_flags(&|a| w.get(&a).copied().unwrap_or(0));
	assert_eq!(flags.len(), 24);
	assert_eq!((flags[0].unlocked, flags[0].visited, flags[1].unlocked, flags[1].visited, flags[2].unlocked), (true, false, true, true, false));
}

// ---- pages --------------------------------------------------------------------------------------------------------------

fn snap(id: u32, page: u32, state: u32, front: bool, items: Vec<Item>) -> MenuSnap {
	MenuSnap { page, id, state, cur: items.first().map_or(0, |i| i.addr), items, front }
}

fn item(addr: u32, kind: Kind, texts: &[u16]) -> Item {
	Item { addr, slot: 0, rect: [20, 40, 100, 100], kind, texts: texts.to_vec(), planet_texts: Vec::new() }
}

#[test]
fn page_ids_route_to_their_pages_and_screens() {
	let cases = [
		(2, Page::Root, Screen::Pause),
		(3, Page::Weapons, Screen::Pause),
		(4, Page::Gadgets, Screen::Pause),
		(5, Page::QuickSelect, Screen::Pause),
		(8, Page::Options, Screen::Pause),
		(0xb, Page::PauseMap, Screen::Map),
		(0xc, Page::Missions, Screen::Map),
		(0xd, Page::QuitGame, Screen::Pause),
		(0xf, Page::GalaxyMap, Screen::Ship),
		(0x10, Page::GalaxyDetails, Screen::Ship),
		(0x1a, Page::Overwrite, Screen::Pause),
		(0x20, Page::Sound, Screen::Pause),
		(0x2c, Page::Commercials, Screen::Pause),
		(0x7f, Page::Other(0x7f), Screen::Pause),
	];
	for (id, page, screen) in cases {
		let r = route(&snap(id, 0x1b0000, 2, false, Vec::new()));
		assert_eq!((r.page, r.screen), (page, screen), "page id {id:#x}");
		assert_eq!(page.id(), Some(id));
	}
}

#[test]
fn every_page_id_round_trips() {
	for id in 0..0x40u32 {
		assert_eq!(Page::from_id(id).id(), Some(id));
	}
}

#[test]
fn a_page_flying_in_or_closing_is_not_ready_and_the_ring_needs_its_item() {
	for (state, ready) in [(2, true), (3, true), (1, false), (0x14, false)] {
		assert_eq!(route(&snap(3, 0x1b0000, state, false, Vec::new())).ready, ready, "state {state:#x}");
	}
	let ring = snap(5, 0x1b0000, 2, false, vec![item(0x40, Kind::Ring { cursor: 2 }, &[])]);
	let grid = snap(5, 0x1b0000, 2, false, vec![item(0x40, Kind::Panel, &[])]);
	assert!(route(&ring).ring && !route(&grid).ring);
	assert!(!route(&snap(3, 0x1b0000, 2, false, vec![item(0x40, Kind::Ring { cursor: 2 }, &[])])).ring);
}

#[test]
fn the_title_pages_route_by_struct_address_and_id() {
	let at = |page: u32, id: u32| route(&snap(id, page, 2, true, Vec::new()));
	assert_eq!(at(P_NEW, 0x1f).page, Page::TitleNew);
	assert_eq!(at(P_LOAD, 0x1f).page, Page::TitleLoad);
	assert_eq!(at(addrs::P_ROOT, 0x2e).page, Page::TitleRoot);
	assert_eq!(at(addrs::P_OPTIONS[1], 0x2f).page, Page::TitleOptions);
	assert_eq!(at(addrs::P_SOUND, 0x20).page, Page::TitleSound);
	assert_eq!(at(addrs::P_OVERWRITE, 0x1a).page, Page::TitleOverwrite);
	assert_eq!(at(crate::retail::pcext::TGPAGE, 0x2f).page, Page::TitleGraphics);
	assert_eq!(at(crate::retail::pcmods::TPAGE, 0x2f).page, Page::TitleMods);
	// An address this table does not know falls back to the id.
	assert_eq!(at(0x1b0000, 0x30).page, Page::TitleLanguage);
	assert_eq!(at(0x1b0000, 0x31).page, Page::TitleVideo);
	assert_eq!(at(0x1b0000, 0x55).page, Page::Other(0x55));
	assert!([P_NEW, P_LOAD, addrs::P_SOUND].iter().all(|&p| at(p, 0x1f).screen == Screen::Title));
	// A front page with an in-level id is still the title's screen.
	assert_eq!(at(0x1b0000, 3).screen, Screen::Title);
}

// ---- view ---------------------------------------------------------------------------------------------------------------

#[test]
fn planet_texts_are_the_second_text_of_each_record() {
	let pp = addrs::page_ptr(3).unwrap();
	let w = ram_menu(pp, menu_keys::GALAXY, 3, 1);
	let m = mouse_menu::read_ptr(&|a| w.get(&a).copied().unwrap_or(0), pp).unwrap();
	assert_eq!(m.items.len(), 1);
	assert_eq!(m.items[0].texts, vec![0x4eb9, 0x4eba, 0x4ebb]);
	assert_eq!(m.items[0].planet_texts, vec![0x4ecc, 0x4ecd, 0x4ece]);
	// The page's own title is not a Label list: a Value item has none.
	let w = ram_menu(pp, 0x1b, 0, 0);
	let m = mouse_menu::read_ptr(&|a| w.get(&a).copied().unwrap_or(0), pp).unwrap();
	assert!(m.items.iter().all(|i| i.planet_texts.is_empty()));

	let mut t = Text::default();
	for p in 0..3 {
		t.insert(0x4eb9 + p, format!("Place {p}"));
		t.insert(0x4ecc + p, format!("\u{c}Planet {p}\u{8}"));
	}
	let w = ram_menu(pp, menu_keys::GALAXY, 3, 1);
	let v = view(&mouse_menu::read_ptr(&|a| w.get(&a).copied().unwrap_or(0), pp).unwrap(), &Strings(&t)).unwrap();
	assert_eq!((v.page, v.screen, v.id, v.ready, v.front), (Page::GalaxyMap, Screen::Ship, 0xf, true, false));
	assert_eq!(v.cur, Some(0));
	let rows = galaxy_rows(&v);
	assert_eq!(rows.len(), 3);
	assert_eq!(rows[1].location.plain(), "Place 1");
	assert_eq!(rows[1].planet.as_ref().map(|p| p.plain()), Some("Planet 1".to_string()));
	assert_eq!(rows.iter().map(|r| r.selected).collect::<Vec<_>>(), vec![false, true, false]);
	assert!(rows.iter().all(|r| r.enabled));
}

#[test]
fn a_view_carries_the_keys_hints_and_bodies_of_its_items() {
	let hint = item(0x50, Kind::Hint(TRIANGLE), &[0x4ee0]);
	let grid = item(0x60, Kind::Grid { rows: 3, cols: 4, cursor: 5 }, &[]);
	let mut t = Text::default();
	t.insert(0x4ee0, "\u{12} Back".into());
	let m = snap(3, 0x1b0000, 2, false, vec![grid, hint]);
	let v = view(&m, &Strings(&t)).unwrap();
	assert_eq!(v.items[0].body, Body::Grid { rows: 3, cols: 4, cursor: 5 });
	assert_eq!(v.items[1].body, Body::Hint { button: Btn::Triangle, text: crate::retail::menu_model::text::decode("\u{12} Back") });
	assert_eq!(v.keys, menu_keys::screen(&m));
	assert!(view(&snap(3, 0x1b0000, 2, false, Vec::new()), &Strings(&t)).is_none());
}

#[test]
fn save_slots_show_level_time_and_date() {
	let f = crate::retail::FrontRam { slots: vec![None, Some((1, 1234, 216_000 + 61 * 3600, [0x02, 0x11, 0x25]))], ..Default::default() };
	let s = slot_views(&f);
	assert_eq!(s[0], None);
	let s1 = s[1].as_ref().unwrap();
	assert_eq!((s1.level, s1.name, s1.bolts, s1.hours, s1.minutes, s1.date.as_str()), (1, crate::toc::LEVEL_NAMES[1], 1234, 2, 1, "11/02/25"), "bytes day 02, month 11, year 25 print as retail's list does: month/day/year");
	assert_eq!(slot_views(&crate::retail::FrontRam { slots: vec![Some((99, 0, 0, [0; 3]))], ..Default::default() })[0].as_ref().unwrap().name, "?");
}

// ---- commands -----------------------------------------------------------------------------------------------------------

fn label(addr: u32, enabled: &[bool]) -> Item {
	item(addr, Kind::Label { rows: enabled.iter().enumerate().map(|(i, &e)| (10.0 + 12.0 * i as f32, e)).collect(), rowh: 12.0, cursor: 0 }, &[])
}

#[test]
fn intents_resolve_per_page() {
	let quit = snap(menu_keys::QUIT_GAME, 0x1b0000, 2, false, vec![label(0x40, &[true, true])]);
	assert_eq!(cmd::resolve(&quit, Intent::Confirm), Some(Command::Press(CIRCLE)));
	assert_eq!(cmd::resolve(&quit, Intent::Back), Some(Command::Press(TRIANGLE)));
	let weapons = snap(3, 0x1b0000, 2, false, vec![item(0x60, Kind::Grid { rows: 3, cols: 4, cursor: 0 }, &[])]);
	assert_eq!(cmd::resolve(&weapons, Intent::Confirm), Some(Command::Press(CROSS)));
	assert_eq!(
		cmd::resolve(&weapons, Intent::Select { item: 0x60, row: 5, confirm: true }),
		Some(Command::Request(Request { page_id: 3, item: 0x60, sub: 5, confirm: true, slide: None }))
	);
	assert_eq!(cmd::resolve(&weapons, Intent::Prev), Some(Command::Press(L1)));
	assert_eq!(cmd::resolve(&weapons, Intent::Next), Some(Command::Press(R1)));
	assert_eq!(cmd::resolve(&weapons, Intent::Press(0x800)), Some(Command::Press(0x800)));
	assert_eq!(cmd::resolve(&weapons, Intent::Press(0)), None);
}

#[test]
fn selecting_needs_a_real_enabled_row() {
	let m = snap(8, 0x1b0000, 2, false, vec![label(0x40, &[true, false]), item(0x50, Kind::Panel, &[]), item(0x60, Kind::Other, &[]), item(0x70, Kind::Ring { cursor: 0 }, &[])]);
	let sel = |item, row| cmd::resolve(&m, Intent::Select { item, row, confirm: false });
	assert!(matches!(sel(0x40, 0), Some(Command::Request(Request { sub: 0, confirm: false, .. }))));
	assert_eq!(sel(0x40, 1), None, "a disabled row sends nothing");
	assert_eq!(sel(0x40, 2), None, "no such row");
	assert_eq!((sel(0x50, 0), sel(0x60, 0), sel(0x70, 0), sel(0x99, 0)), (None, None, None, None));
}

#[test]
fn a_slider_needs_a_value_item() {
	let m = snap(0x20, 0x1b0000, 2, false, vec![item(0x40, Kind::Value { n: 3, cursor: 0 }, &[]), label(0x50, &[true])]);
	assert_eq!(
		cmd::resolve(&m, Intent::Slide { item: 0x40, row: 1, slide: Slide::To(0x200) }),
		Some(Command::Request(Request { page_id: 0x20, item: 0x40, sub: 1, confirm: false, slide: Some(Slide::To(0x200)) }))
	);
	assert_eq!(cmd::resolve(&m, Intent::Slide { item: 0x50, row: 0, slide: Slide::By(8) }), None);
}

#[test]
fn a_pulse_is_one_field_pressed_and_two_released() {
	let mut p = Pulses::new();
	assert_eq!((0..10).map(|_| p.step(Some(8), true, false)).sum::<u16>(), 0, "an empty queue leaves the pad alone");
	p.push(CROSS);
	p.push(TRIANGLE);
	let seq: Vec<u16> = (0..7).map(|_| p.step(Some(8), true, false)).collect();
	assert_eq!(seq, vec![CROSS, 0, 0, TRIANGLE, 0, 0, 0]);
}

#[test]
fn a_pulse_waits_for_a_settled_menu_and_free_hands_and_goes_stale() {
	let mut p = Pulses::new();
	p.push(CROSS);
	assert_eq!([p.step(Some(8), false, false), p.step(Some(8), true, true), p.step(Some(8), false, true)], [0, 0, 0]);
	assert_eq!(p.step(Some(8), true, false), CROSS);
	let mut p = Pulses::new();
	p.push(CROSS);
	for _ in 0..200 {
		assert_eq!(p.step(Some(8), false, false), 0);
	}
	assert_eq!(p.step(Some(8), true, false), 0, "the queue was dropped");
	for _ in 0..20 {
		p.push(CIRCLE);
	}
	assert_eq!((0..40).map(|_| p.step(Some(8), true, false)).filter(|&b| b != 0).count(), 8, "at most 8 wait");
}

#[test]
fn a_pulse_fires_only_on_the_page_it_was_made_for() {
	let mut p = Pulses::new();
	p.stamp(Some(2));
	p.push(TRIANGLE); // Back on the pause root, queued while Start was held
	p.stamp(Some(0xd));
	p.push(CIRCLE); // "Yes" of the Quit Game page
	// The root went away and another page took its place (ids differ): the first pulse is dropped, never sent to the new page.
	assert_eq!((0..4).map(|_| p.step(Some(0xd), true, false)).collect::<Vec<_>>(), [CIRCLE, 0, 0, 0], "only the Quit page's own pulse fires");
	p.stamp(Some(7));
	p.push(CROSS);
	assert_eq!((0..10).map(|_| p.step(Some(0x1a), true, false)).sum::<u16>(), 0, "a pulse for page 7 never fires on page 0x1a");
	assert_eq!((0..4).map(|_| p.step(Some(7), true, false)).collect::<Vec<_>>(), [0; 4], "and it is gone, not kept for later");
	// A pulse with no page (the screen had no menu snapshot) behaves as before: any settled menu.
	p.stamp(None);
	p.push(CROSS);
	assert_eq!(p.step(Some(0x33), true, false), CROSS);
}

#[test]
fn the_queue_is_dropped_when_the_menu_closes() {
	let mut p = Pulses::new();
	p.stamp(Some(2));
	p.push(TRIANGLE);
	// The pad is held (Start), so the pulse waits; the pause menu closes and opens again as the same page id.
	assert_eq!((0..5).map(|_| p.step(Some(2), true, true)).sum::<u16>(), 0);
	assert_eq!(p.step(None, false, true), 0);
	assert_eq!((0..10).map(|_| p.step(Some(2), true, false)).sum::<u16>(), 0, "the reopened menu does not get the old Back");
	// A pulse already pressed finishes its three fields even if the menu vanishes under it.
	p.push(CROSS);
	assert_eq!(p.step(Some(2), true, false), CROSS);
	assert_eq!([p.step(None, false, false), p.step(None, false, false)], [0, 0]);
}

/// The statics in one test: other tests never touch them.
#[test]
fn submitted_commands_reach_the_worker_once() {
	let _t = cmd::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let r = Request { page_id: 3, item: 0x60, sub: 2, confirm: true, slide: None };
	let ready = snap(3, 0x1b0000, 2, false, Vec::new());
	// A host test that hovered a row may have left its request or pulses in the queues: start from empty ones.
	let _ = cmd::take_request();
	for _ in 0..80 {
		let _ = cmd::pad_step(Some(&ready), 0);
	}
	let (mut b, mut instant) = (0u16, None);
	cmd::feed(Some(&ready), &mut b, &mut instant);
	assert_eq!((b, instant), (0, None), "nothing queued = unchanged");
	cmd::submit(Command::Request(Request { sub: 9, ..r }));
	cmd::submit(Command::Request(r));
	cmd::submit(Command::Press(CROSS));
	cmd::feed(Some(&ready), &mut b, &mut instant);
	assert_eq!((b, instant), (CROSS, Some(Cmd::Submit(r))));
	let (mut b, mut instant) = (0u16, None);
	cmd::feed(Some(&ready), &mut b, &mut instant);
	assert_eq!((b, instant), (0, None), "the request was taken once, the pulse lasted one field");
	// The mouse's own command this field wins; the screen's waits.
	cmd::submit(Command::Request(r));
	let mut instant = Some(Cmd::Cancel);
	cmd::feed(None, &mut b, &mut instant);
	assert_eq!(instant, Some(Cmd::Cancel));
	assert_eq!(cmd::take_request(), Some(Cmd::Submit(r)));
	assert_eq!(cmd::take_request(), None);
}

// ---- review fixes (lane nrv2) -----------------------------------------------------------------------------------------------

#[test]
fn the_shop_key_tells_the_list_from_the_prompt() {
	use crate::retail::mouse_vendor::{stage, ID};
	let shop = |state| snap(ID, 0x1ca898, state, false, Vec::new());
	assert_eq!([stage(&shop(2)), stage(&shop(4)), stage(&shop(3)), stage(&shop(1))], [1, 1, 2, 3], "list, cyclic list, prompt, animating");
	assert_eq!(stage(&snap(3, 0x1b0000, 3, false, Vec::new())), 0, "every other menu: the page id alone");
	assert_eq!(cmd::key(&shop(3)), (ID, 2));
}

/// R4-1 / R2b-1: a double click on Buy queues a second Cross made on the list; it must never confirm the prompt the first one opened.
#[test]
fn a_cross_queued_on_the_shop_list_never_confirms_its_prompt() {
	let (list, prompt, busy) = ((0xfe, 1), (0xfe, 2), (0xfe, 3));
	let mut p = Pulses::new();
	p.stamp_key(Some(list));
	p.push(CROSS);
	p.push(CROSS);
	assert_eq!((0..3).map(|_| p.step_key(Some(list), true, false)).collect::<Vec<_>>(), [CROSS, 0, 0], "the first opens the prompt");
	assert_eq!((0..5).map(|_| p.step_key(Some(busy), false, false)).sum::<u16>(), 0, "the second waits while the shop animates");
	assert_eq!((0..5).map(|_| p.step_key(Some(prompt), true, false)).sum::<u16>(), 0, "and is dropped on the settled prompt");
	assert_eq!((0..5).map(|_| p.step_key(Some(list), true, false)).sum::<u16>(), 0, "for good");
	// A Triangle queued on the prompt (double click on Cancel) never leaves the shop from the list it went back to.
	p.stamp_key(Some(prompt));
	p.push(TRIANGLE);
	p.push(TRIANGLE);
	assert_eq!(p.step_key(Some(prompt), true, false), TRIANGLE);
	assert_eq!((0..8).map(|_| p.step_key(Some(list), true, false)).sum::<u16>(), 0);
	// A press made while the shop animated is dropped once it settles (retail ignores Cross during the animation too).
	p.stamp_key(Some(busy));
	p.push(CROSS);
	assert_eq!((0..4).map(|_| p.step_key(Some(prompt), true, false)).sum::<u16>(), 0);
	// The list's own pulse still fires on the list after an animation.
	p.stamp_key(Some(list));
	p.push(RIGHT_BIT);
	assert_eq!([p.step_key(Some(busy), false, false), p.step_key(Some(list), true, false)], [0, RIGHT_BIT]);
}
const RIGHT_BIT: u16 = 0x20;

/// R2-3: `clear_pulses` (the window lost the focus, or the game's pad is neutral this field) drops the queue but keeps the stamp.
#[test]
fn clearing_drops_the_queue_and_keeps_the_page() {
	let mut p = Pulses::new();
	p.stamp(Some(2));
	p.push(TRIANGLE);
	assert_eq!(p.step(Some(2), true, false), TRIANGLE);
	p.push(CROSS);
	p.clear();
	assert_eq!((0..6).map(|_| p.step(Some(2), true, false)).sum::<u16>(), 0, "nothing queued or half pressed survives");
	p.push(CROSS);
	assert_eq!((0..6).map(|_| p.step(Some(3), true, false)).sum::<u16>(), 0, "a new pulse is still made for page 2");
	let src = include_str!("../input.rs");
	let body = &src[src.find("fn screen_presses(").expect("the screens' presses")..];
	let (feed, clear) = (body.find("menu_model::cmd::feed(").expect("the feed"), body.find("menu_model::cmd::neutral_field()").expect("the clear"));
	assert!(clear < feed && body[clear..feed].contains("return;"), "a neutral field clears the queued pulses instead of feeding them");
}

/// R2-3 narrowed: only the first field of a neutral stretch drops the queue. A click made on a page that is up during the stretch
/// (the pause page while the key that closed the Options page is held) presses once the stretch ends, on that page only.
#[test]
fn a_neutral_stretch_drops_only_what_was_queued_before_it() {
	let mut p = Pulses::new();
	p.stamp(Some(2));
	p.push(TRIANGLE);
	p.neutral();
	p.push(CROSS);
	p.neutral();
	p.neutral();
	let mut got: Vec<u16> = (0..6).map(|_| p.step(Some(2), true, false)).collect();
	got.retain(|b| *b != 0);
	assert_eq!(got.first(), Some(&CROSS), "the Back from before the stretch is gone, the click made during it presses");
	assert!(!got.contains(&TRIANGLE));
	p.push(CROSS);
	p.neutral();
	assert_eq!((0..6).map(|_| p.step(Some(2), true, false)).sum::<u16>(), 0, "a new stretch after a fed field drops again");
	p.push(CROSS);
	p.neutral();
	p.stamp(Some(3));
	p.push(CROSS);
	assert_eq!((0..6).map(|_| p.step(Some(2), true, false)).sum::<u16>(), 0, "a pulse made during the stretch for another page never fires");
}

/// R2b-1: a row's second click on the shop list (a Select request with Cross) is dropped once the shop shows its prompt.
#[test]
fn a_shop_request_made_on_the_list_is_dropped_on_the_prompt() {
	let _t = cmd::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let _ = cmd::take_request();
	use crate::retail::mouse_vendor::ID;
	let shop = |state| snap(ID, 0x1ca898, state, false, Vec::new());
	let r = Request { page_id: ID, item: 0x1ca85c, sub: 0, confirm: true, slide: None };
	cmd::stamp(Some(&shop(2)));
	for (now, want) in [(shop(3), None), (shop(1), Some(Cmd::Submit(r))), (shop(2), Some(Cmd::Submit(r)))] {
		cmd::submit(Command::Request(r));
		let (mut b, mut instant) = (0u16, None);
		cmd::feed(Some(&now), &mut b, &mut instant);
		assert_eq!(instant, want, "made on the list, the shop now in state {}", now.state);
	}
	cmd::stamp(Some(&snap(3, 0x1b0000, 3, false, Vec::new())));
	cmd::submit(Command::Request(Request { page_id: 3, ..r }));
	let (mut b, mut instant) = (0u16, None);
	cmd::feed(Some(&snap(3, 0x1b0000, 2, false, Vec::new())), &mut b, &mut instant);
	assert!(instant.is_some(), "other menus: the request goes on (mouse_instant checks the page)");
	cmd::stamp(None);
	let _ = cmd::take_request();
}

/// R4-1 / R2b-1, part 4 (pp run 2026-11-02): the game shows the screen a shop Cross opened only 4 fields later (Cross in field 50,
/// the list read at 51..53, the prompt from 54), so a second Cross queued on the list (a double click on Buy or the hint's Enter 1-3
/// fields apart) still matched the list's key and bought one ammo with no prompt. The first press spends the list.
#[test]
fn a_second_click_on_a_shop_screen_the_game_is_leaving_presses_nothing() {
	let (list, prompt) = ((0xfe, 1), (0xfe, 2));
	for gap in 1..=3 {
		let mut p = Pulses::new();
		p.stamp_key(Some(list));
		p.push(CROSS);
		let mut got = Vec::new();
		for f in 0..4 {
			if f == gap {
				p.push(CROSS);
			}
			got.push(p.step_key(Some(list), true, false));
		}
		got.extend((0..8).map(|_| p.step_key(Some(prompt), true, false)));
		assert_eq!(got.iter().filter(|b| **b == CROSS).count(), 1, "gap {gap}: one Cross, the prompt stays up");
	}
	// The prompt's quantity arrows keep it: every notch steps, and the Cross after them confirms; a second Cross (a double click on
	// Kaufen) or a Triangle made on the prompt before the game shows the list again presses nothing.
	let mut p = Pulses::new();
	p.stamp_key(Some(prompt));
	for b in [mouse_menu::RIGHT, mouse_menu::RIGHT, CROSS, CROSS, TRIANGLE] {
		p.push(b);
	}
	let got: Vec<u16> = (0..15).map(|_| p.step_key(Some(prompt), true, false)).filter(|b| *b != 0).collect();
	assert_eq!(got, [mouse_menu::RIGHT, mouse_menu::RIGHT, CROSS]);
	// A double click on Cancel: one Triangle; the list it goes back to sees no second one.
	let mut p = Pulses::new();
	p.stamp_key(Some(prompt));
	p.push(TRIANGLE);
	let mut got = vec![p.step_key(Some(prompt), true, false)];
	p.push(TRIANGLE);
	got.extend((0..3).map(|_| p.step_key(Some(prompt), true, false)));
	got.extend((0..8).map(|_| p.step_key(Some(list), true, false)));
	assert_eq!(got.iter().filter(|b| **b == TRIANGLE).count(), 1, "the second Triangle never leaves the shop");
}

/// A spent shop screen is free again once the game settled after leaving it (Buy all ammo: list, animation, list), or after SPENT_MAX
/// fields if the press changed nothing (a row the player cannot afford), so a later click presses as usual.
#[test]
fn a_spent_shop_screen_frees_once_the_game_moved_on_or_the_press_did_nothing() {
	let (list, busy) = ((0xfe, 1), (0xfe, 3));
	const SQUARE: u16 = 0x80;
	let mut p = Pulses::new();
	p.stamp_key(Some(list));
	p.push(SQUARE);
	p.push(SQUARE);
	let mut got: Vec<u16> = (0..4).map(|_| p.step_key(Some(list), true, false)).collect();
	got.extend((0..10).map(|_| p.step_key(Some(busy), false, false)));
	got.extend((0..6).map(|_| p.step_key(Some(list), true, false)));
	assert_eq!(got.iter().filter(|b| **b == SQUARE).count(), 1, "one refill per double click, also after the animation");
	p.push(CROSS);
	assert_eq!(p.step_key(Some(list), true, false), CROSS, "the list after the refill takes a new click at once");
	// Nothing changes on the press: the list stays spent for SPENT_MAX fields, then a click presses again.
	let mut p = Pulses::new();
	p.stamp_key(Some(list));
	p.push(CROSS);
	assert_eq!(p.step_key(Some(list), true, false), CROSS);
	let mut n = 1;
	while n <= cmd::SPENT_MAX {
		p.push(CROSS);
		assert_eq!(p.step_key(Some(list), true, false), 0, "field {n}: still the screen the Cross went to");
		n += 1;
	}
	p.push(CROSS);
	assert_eq!(p.step_key(Some(list), true, false), CROSS, "free again");
	// Other menus (key word 0) change their page on the press: two pulses there press twice, as before.
	let mut p = Pulses::new();
	p.stamp(Some(2));
	p.push(CROSS);
	p.push(CROSS);
	assert_eq!((0..6).map(|_| p.step(Some(2), true, false)).filter(|b| *b == CROSS).count(), 2);
}

/// The request path (a click on the list's current row, or Buy while the hover is ahead of the game's cursor) shares the spent screen
/// with the pulses: a Cross request handed to the settled list spends it, and one made for a spent screen is dropped.
#[test]
fn a_shop_cross_request_and_a_cross_pulse_spend_the_same_screen() {
	let _t = cmd::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let _ = cmd::take_request();
	use crate::retail::mouse_vendor::ID;
	let shop = |state| snap(ID, 0x1ca898, state, false, Vec::new());
	let (list, prompt, other) = (shop(2), shop(3), snap(3, 0x1b0000, 2, false, Vec::new()));
	let r = Request { page_id: ID, item: 0x1ca85c, sub: 0, confirm: true, slide: None };
	let feed = |m: &MenuSnap| {
		let (mut b, mut instant) = (0u16, None);
		cmd::feed(Some(m), &mut b, &mut instant);
		(b, instant)
	};
	for _ in 0..4 {
		let _ = feed(&other); // a spent screen left by an earlier test is freed on another settled menu
	}
	cmd::stamp(Some(&list));
	cmd::submit(Command::Request(r));
	assert_eq!(feed(&list), (0, Some(Cmd::Submit(r))), "the first click's request goes out");
	cmd::submit(Command::Press(CROSS));
	cmd::submit(Command::Request(Request { confirm: false, ..r }));
	assert_eq!(feed(&list), (0, Some(Cmd::Submit(Request { confirm: false, ..r }))), "the second click's Cross is dropped, a hover is not");
	cmd::submit(Command::Request(r));
	assert_eq!(feed(&list), (0, None), "nor a second Cross request");
	assert_eq!(feed(&prompt), (0, None), "the prompt shows");
	// Now a deliberate click on the prompt presses (the page stamps it for the prompt).
	cmd::stamp(Some(&prompt));
	cmd::submit(Command::Press(CROSS));
	assert_eq!(feed(&prompt).0, CROSS);
	for _ in 0..4 {
		let _ = feed(&other);
	}
	cmd::stamp(None);
	let _ = cmd::take_request();
}
