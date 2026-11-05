//! Tests of the modern Galactic Map (`ship.rs`): when the page replaces retail's picture (`wanted`), one marker and one row per planet
//! the game lets the player choose, greyed rows that send nothing, the details stage without a painting or a click target, and every
//! word fitting its box and drawable in all five languages, the planet picture's hole (`ship_pic.rs`). The host's mouse is in
//! `ship_host.rs`'s own door; the list reader in `ship.rs`.
use super::hide::{hidden_in, Screen};
use super::ship::*;
use super::ship_pic;
use crate::retail::menu_model::addrs::PlanetFlag;
use crate::retail::menu_model::extra::ShipFacts;
use rac_engine::ui::font::{FontMeasure, FontSet};
use rac_engine::ui::input::{Device, GlyphSet};
use rac_engine::ui::kit::text::run_width;
use rac_engine::ui::prim::{Prim, Rect};
use crate::frontend::Text;
use crate::retail::menu_model::{view, Strings};
use crate::retail::mouse_menu::{Item, Kind, MenuSnap, CIRCLE};

fn row(planet: u32, enabled: bool) -> Row {
	let (location, name) = if enabled { (format!("Place {planet}"), format!("Planet {planet}")) } else { (String::new(), "????".to_string()) };
	Row { planet: Some(planet), location, name, enabled }
}

/// Eleven planets, the 4th and 9th greyed.
fn listing(sel: usize) -> Listing {
	Listing { item: 0x1234, rows: (1..=11).map(|p| row(p, p != 4 && p != 9)).collect(), sel }
}

fn page(l: &Listing, stage: Stage, lang: usize, device: Device) -> Built {
	let t = |id: u32| if id == 0x4eef { "Play Infobot".to_string() } else { String::new() };
	build(&Facts { list: l, stage, focus: None, scroll: 0, here: 3, planets: &facts(), infobot: true, text: &t, lang, device, glyphs: GlyphSet::PlayStation }, &FontMeasure).expect("the page builds")
}

fn centre(r: Rect) -> (f32, f32) {
	(r.cx(), r.cy())
}

/// Planet 1 visited with one gold bolt, planet 2 unlocked but not visited, every other planet unread beyond its flags.
fn facts() -> ShipFacts {
	let mut flags = vec![PlanetFlag::default(); 19];
	(flags[1], flags[2]) = (PlanetFlag { unlocked: true, visited: true }, PlanetFlag { unlocked: true, visited: false });
	ShipFacts { flags, gold: (0..19).map(|p| u32::from(p == 1)).collect() }
}

fn texts(b: &Built) -> Vec<String> {
	b.prims.iter().filter_map(|p| if let Prim::Text(t) = p { Some(t.text.clone()) } else { None }).collect()
}

#[test]
fn the_details_panel_shows_gold_bolts_found_and_the_visited_status_of_a_choosable_planet() {
	let t = texts(&page(&listing(0), Stage::Details, 0, Device::Pad));
	for want in ["Gold bolts found", "1 of 3", "Status", "Visited"] {
		assert!(t.iter().any(|s| s == want), "{want:?} in {t:?}");
	}
	assert!(texts(&page(&listing(1), Stage::Details, 0, Device::Pad)).iter().any(|s| s == "Not visited yet"));
	// A greyed planet (4) and a page whose facts were not read show neither line.
	assert!(!texts(&page(&listing(3), Stage::Details, 0, Device::Pad)).iter().any(|s| s == "Status"));
	let none = ShipFacts::default();
	let (l, tx) = (listing(0), |_: u32| String::new());
	let b = build(&Facts { list: &l, stage: Stage::Details, focus: None, scroll: 0, here: 3, planets: &none, infobot: false, text: &tx, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation }, &FontMeasure).unwrap();
	assert!(!texts(&b).iter().any(|s| s == "Status"), "no facts read: no lines");
}

#[test]
fn the_page_is_wanted_only_on_the_ship_pages() {
	let g = Gate { modern: true, ui_open: false, refused: false, page: Some(0xf) };
	assert!(wanted(&g) && wanted(&Gate { page: Some(0x10), ..g }));
	for page in [None, Some(2), Some(0xb), Some(0xc), Some(0xe), Some(0x11)] {
		assert!(!wanted(&Gate { page, ..g }), "page {page:?}");
	}
	assert!(!wanted(&Gate { modern: false, ..g }) && !wanted(&Gate { ui_open: true, ..g }) && !wanted(&Gate { refused: true, ..g }));
}

#[test]
fn the_retail_picture_is_hidden_only_while_the_page_is_drawn() {
	assert!(!hidden_in(Screen::Ship, 0, true, None) && hidden_in(Screen::Ship, 1 << Screen::Ship as u8, true, None));
	assert!(!hidden_in(Screen::Ship, 1 << Screen::Ship as u8, false, None), "Classic keeps retail's picture");
}

#[test]
fn every_choosable_planet_has_a_row_and_a_marker_and_greyed_ones_send_nothing() {
	let b = page(&listing(0), Stage::List, 0, Device::Pad);
	assert!(b.art && b.stage == Stage::List && b.count == 11);
	assert_eq!(b.rows.iter().map(|r| r.0).collect::<Vec<_>>(), vec![0, 1, 2, 4, 5, 6, 7], "9 rows in view (0..=8), the greyed ones (index 3 and 8) are not targets");
	assert_eq!(b.marks.len(), 9, "a marker per enabled planet, none for the greyed two");
	for &(i, r) in b.rows.iter().chain(&b.marks) {
		let (x, y) = centre(r);
		assert_eq!(b.hit(x, y), Hit::Row(i));
		assert!(b.enabled[i], "row {i}");
	}
	let b2 = page(&listing(10), Stage::List, 0, Device::Pad);
	assert!(b2.rows.iter().any(|r| r.0 == 10) && b2.sel == 10, "the list scrolls to the selection");
	assert!(b.prims.iter().any(|p| matches!(p, Prim::Text(t) if t.text == "????")), "a greyed row shows ????");
	assert!(b.prims.iter().any(|p| matches!(p, Prim::Text(t) if t.text == "You are here")), "the level the ship is over");
}

/// After the wheel walked the list down, a hover on a row in view keeps the window (user 2026-11-04: the Galactic Map jumped back to
/// the top under the mouse, as the shop did): the window moves only as far as the selection needs.
#[test]
fn a_row_chosen_inside_the_window_never_scrolls_the_list() {
	let t = |_: u32| String::new();
	let first = |sel: usize, scroll: usize| {
		let l = listing(sel);
		let f = Facts { list: &l, stage: Stage::List, focus: None, scroll, here: 3, planets: &facts(), infobot: true, text: &t, lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation };
		build(&f, &FontMeasure).expect("the page builds").first
	};
	assert_eq!(first(10, 0), 2, "the last planet: the window ends at it");
	assert_eq!(first(5, 2), 2, "a hover on a row in view keeps the window (it was 0: the selection's bottom row)");
	assert_eq!(first(1, 2), 1, "a row above the window moves it just enough");
	assert_eq!(first(0, 0), 0);
}

#[test]
fn the_details_stage_keeps_the_painting_but_has_no_list_and_no_click_target() {
	let b = page(&listing(2), Stage::Details, 0, Device::Pad);
	assert!(b.art && b.rows.is_empty() && b.marks.is_empty());
	for (x, y) in [(100.0, 150.0), (640.0, 360.0)] {
		assert_eq!(b.hit(x, y), Hit::None);
	}
	let texts: Vec<String> = b.prims.iter().filter_map(|p| if let Prim::Text(t) = p { Some(t.text.clone()) } else { None }).collect();
	assert!(texts.iter().any(|t| t == "Land on Planet 3"), "{texts:?}");
	assert!(page(&listing(0), Stage::Details, 0, Device::Pad).prims.iter().any(|p| matches!(p, Prim::Text(t) if t.text == "Blast off to Planet 1")));
	assert!(build(&Facts { list: &Listing::default(), stage: Stage::List, focus: None, scroll: 0, here: 0, planets: &ShipFacts::default(), infobot: false, text: &|_| String::new(), lang: 0, device: Device::Pad, glyphs: GlyphSet::PlayStation }, &FontMeasure).is_none());
}

/// The game's page 0xf in the mouse's terms: a Circle hint line, then the list item with planets 1..=3 (location text 0x4eb9 + id, name
/// text 0x4ecc + id), the 2nd row greyed, the cursor on `cursor`; and the texts of the player's language.
fn game_page(cursor: u32) -> (MenuSnap, Text) {
	let mut t = Text::default();
	for p in 1..=3u32 {
		t.insert(0x4eb9 + p, format!("Place {p}"));
		t.insert(0x4ecc + p, format!("\u{c}Planet {p}\u{8}"));
	}
	t.insert(0x4eef, "\u{11} Play Infobot".to_string());
	let hint = Item { addr: 0x50, slot: 0, rect: [0, 0, 10, 10], kind: Kind::Hint(CIRCLE), texts: vec![0x4eef], planet_texts: Vec::new() };
	let rows = vec![(20.0, true), (60.0, false), (100.0, true)];
	let kind = Kind::Label { rows, rowh: 40.0, cursor };
	let list = Item { addr: 0x1234, slot: 1, rect: [20, 86, 144, 251], kind, texts: vec![0x4eba, 0x4ebb, 0x4ebc], planet_texts: vec![0x4ecd, 0x4ece, 0x4ecf] };
	(MenuSnap { page: 0x1b3d98, id: 0xf, state: 2, cur: 0x1234, items: vec![hint, list], front: false }, t)
}

#[test]
fn the_planet_list_is_read_from_the_page_the_game_shows() {
	let none = |_: u32| String::new();
	let (m, t) = game_page(2);
	let v = view(&m, &Strings(&t)).expect("a view");
	let l = listing_of(&v, &none).expect("a planet list");
	assert_eq!((l.item, l.sel, l.rows.len()), (0x1234, 2, 3), "the list item, not the hint line");
	assert_eq!(l.rows.iter().map(|r| r.planet).collect::<Vec<_>>(), [Some(1), Some(2), Some(3)], "the planet id comes from the location text");
	let r = |i: usize| (l.rows[i].location.as_str(), l.rows[i].name.as_str(), l.rows[i].enabled);
	assert_eq!(r(0), ("Place 1", "Planet 1", true));
	assert_eq!(r(1), ("", "????", false), "a greyed row shows only ????, never its planet");
	assert_eq!(r(2), ("Place 3", "Planet 3", true));
	let own = |id: u32| if id == 0x4f54 { "Unknown".to_string() } else { String::new() };
	assert_eq!(listing_of(&v, &own).expect("a planet list").rows[1].name, "Unknown", "the game's own word for it when the language has one");
	assert!(infobot_of(&v), "the Circle hint line names the Infobot");

	// The cursor is clamped to the rows; each cursor row is the list's `sel`.
	for (cursor, sel) in [(0, 0), (1, 1), (9, 2)] {
		let (m, t) = game_page(cursor);
		assert_eq!(listing_of(&view(&m, &Strings(&t)).expect("a view"), &none).map(|l| l.sel), Some(sel), "cursor {cursor}");
	}

	// An enabled row without a planet name text shows ????; a list whose rows name no planet is not the planet list.
	let (mut m, t) = game_page(0);
	m.items[1].planet_texts = vec![0, 0, 0];
	assert_eq!(listing_of(&view(&m, &Strings(&t)).expect("a view"), &none).expect("a list").rows[0].name, "????");
	m.items[1].texts = vec![0x4e00, 0x4e01, 0x4e02];
	let v = view(&m, &Strings(&t)).expect("a view");
	assert_eq!(listing_of(&v, &none), None);
	m.items.remove(0);
	assert!(!infobot_of(&view(&m, &Strings(&t)).expect("a view")), "no Circle line, no Infobot hint");
}

#[test]
fn every_word_fits_and_is_drawable_in_all_five_languages() {
	let m = FontMeasure;
	let set = FontSet::get();
	for tab in all() {
		for s in tab.iter() {
			assert!(s.chars().all(|c| set.covers(c)), "{s:?}");
		}
	}
	let mut l = listing(1);
	l.rows[1].name = "Lanzagranadas de combate".into();
	l.rows[1].location = "Handschuh des Verderbens".into();
	for lang in 0..5 {
		for stage in [Stage::List, Stage::Details] {
			for device in [Device::Pad, Device::Keyboard] {
				let b = page(&l, stage, lang, device);
				for p in &b.prims {
					if let Prim::Text(r) = p {
						let w = run_width(&m, r.role, r.px, &r.text, r.track);
						assert!(r.max_w == 0.0 || w <= r.max_w * 1.7, "{:?} is {w:.1} px in a box of {} (lang {lang})", r.text, r.max_w);
					}
				}
				let left = b.prims.iter().filter_map(|p| match p {
					Prim::Text(r) if r.y == 688.0 => Some(r.x),
					Prim::Glyph { rect, .. } if rect.cy() == 688.0 => Some(rect.x),
					_ => None,
				});
				assert!(left.fold(f32::MAX, f32::min) >= 48.0, "hint bar (lang {lang}, {stage:?})");
			}
		}
	}
}

/// Area of the overlap of two rects.
fn overlap(a: Rect, b: Rect) -> f32 {
	let (w, h) = (a.right().min(b.right()) - a.x.max(b.x), a.bottom().min(b.bottom()) - a.y.max(b.y));
	if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

#[test]
fn the_list_stage_leaves_a_hole_for_the_retail_planet_picture_in_the_card() {
	let b = page(&listing(0), Stage::List, 0, Device::Pad);
	let hole = b.planet.expect("the list stage has a picture hole");
	assert!(CARD.x <= hole.x && hole.right() <= CARD.right() && CARD.y <= hole.y && hole.bottom() <= CARD.bottom(), "{hole:?} inside {CARD:?}");
	assert!((hole.w / hole.h - 3.08).abs() < 0.01, "the panel's display aspect, letterboxed: {}", hole.w / hole.h);
	assert!((hole.cx() - CARD.cx()).abs() < 1e-3 && (hole.cy() - CARD.cy()).abs() < 1e-3, "centred in the card");
	// No plate, card strip, disc or text covers the hole; only the card's transparent border runs along it.
	for p in &b.prims {
		match p {
			Prim::RRect { rect, fill, border: None, .. } if fill.a > 0 => assert!(overlap(*rect, hole) < 1e-3, "{rect:?} covers the hole"),
			Prim::Text(t) => assert!(!(t.x < hole.right() && t.x > hole.x && t.y > hole.y && t.y < hole.bottom()), "text {:?} in the hole", t.text),
			_ => {}
		}
	}
	// The plate and the card each leave the same hole (a Border-only RRect has no fill).
	let strips: Vec<Rect> = b.prims.iter().filter_map(|p| match p { Prim::RRect { rect, fill, .. } if fill.a > 0 && rect.h > 0.0 && overlap(*rect, CARD) > 0.0 => Some(*rect), _ => None }).collect();
	assert_eq!(strips.len(), 4, "the plate's and the card's top and bottom strips (their side strips are empty): {strips:?}");
}

#[test]
fn the_picture_hole_follows_no_selection_and_the_details_stage_keeps_the_discs() {
	let (a, c) = (page(&listing(0), Stage::List, 0, Device::Pad), page(&listing(7), Stage::List, 0, Device::Pad));
	assert_eq!(a.planet, c.planet, "the hole is the same for every planet; the picture inside follows the game's selection");
	let d = page(&listing(0), Stage::Details, 0, Device::Pad);
	assert_eq!(d.planet, None, "the retail overlay holds no planet picture on page 0x10");
	let discs = d.prims.iter().filter(|p| matches!(p, Prim::RRect { rect, radius, .. } if (*radius - rect.w * 0.5).abs() < 1e-3 && overlap(*rect, CARD) > 0.0)).count();
	assert_eq!(discs, 3, "the three discs of the badge stay on the details page");
}

#[test]
fn the_crop_of_the_panel_is_the_inset_panel_and_the_knob_moves_only_the_uv() {
	let hole = ship_pic::hole_in(CARD);
	assert_eq!(ship_pic::src(0.0), ship_pic::PANEL);
	assert_eq!(ship_pic::src(3.0), [186.0, 307.0, 301.0, 114.0]);
	let uv = ship_pic::uv(ship_pic::INSET);
	assert_eq!(uv, [186.0 / 512.0, 307.0 / 448.0, 487.0 / 512.0, 421.0 / 448.0]);
	assert_ne!(ship_pic::uv(0.0), uv);
	assert_eq!(ship_pic::hole_in(CARD), hole, "the hole does not depend on the knob");
	assert_eq!(ship_pic::inset_now(), ship_pic::INSET, "no RAC_TUNE: the default");
}

#[test]
fn a_page_that_flies_in_fills_the_hole_with_the_card_colour_and_places_no_crop() {
	let settled = page(&listing(0), Stage::List, 0, Device::Pad);
	for state in [2, 0] {
		let mut b = settled.clone();
		ship_pic::settle(&mut b, state);
		assert_eq!(b, settled, "state {state}: left as built");
	}
	for state in [ship_pic::FLYING, 0x14] {
		let mut b = settled.clone();
		ship_pic::settle(&mut b, state);
		assert_eq!(b.planet, None, "state {state}: no crop while the retail panel is not in place");
		assert_eq!(b.prims.len(), settled.prims.len() + 1);
		let hole = settled.planet.unwrap();
		assert!(matches!(b.prims.last(), Some(Prim::RRect { rect, fill, border: None, .. }) if *fill == ship_pic::CARD_FILL && overlap(*rect, hole) > hole.w * hole.h * 0.95), "the hole is filled");
	}
	let mut d = page(&listing(0), Stage::Details, 0, Device::Pad);
	let before = d.clone();
	ship_pic::settle(&mut d, ship_pic::FLYING);
	assert_eq!(d, before, "the details page has no hole to fill");
}
