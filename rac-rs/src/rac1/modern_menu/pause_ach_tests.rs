//! Tests of the modern pause menu's Achievements page (`pause_ach.rs`, `pause_host/ach.rs`; lane nach, 2026-11-03): the cards'
//! window over a long list, the cheated session's banner, a hidden entry's secret, the status column, the five languages, and the
//! routing (the open list is a page of the modern menu, not a HUD list the page steps aside for).
use super::pause::{build, list_open, wanted, Built, Gate};
use super::pause_ach::*;
use super::pause_tests::{facts, text_of, view_of};
use crate::achievements::{Grade, ListView, RowView};
use crate::retail::menu_model::extra::Extra;
use crate::retail::menu_model::pages::Page;
use rac_engine::ui::font::FontMeasure;
use rac_engine::ui::kit::page::Panel;
use rac_engine::ui::kit::page_draw;
use rac_engine::ui::prim::Prim;

const NAMES: [&str; 12] = ["N0", "N1", "N2", "N3", "N4", "N5", "N6", "N7", "N8", "N9", "N10", "N11"];

/// 12 rows: 0 and 1 unlocked (0 with a date), 2 a counter at 3 / 10, 3 hidden and locked, the rest plain locked.
fn list(cheated: bool) -> ListView {
	let rows: Vec<RowView> = (0..12)
		.map(|i| RowView {
			name: NAMES[i],
			desc: "D",
			grade: if i == 0 { Grade::Gold } else { Grade::Bronze },
			masked: i == 3,
			unlocked: i < 2,
			at: if i == 0 { 1_762_128_000 } else { 0 },
			progress: if i == 2 { 3 } else { 0 },
			max: if i == 2 { 10 } else { 0 },
		})
		.collect();
	ListView { rows, got: 2, all: 12, cheated }
}

fn page(cheated: bool, focus: usize, first: usize, lang: usize) -> Built {
	let v = view_of(Page::Options, &[(1, true), (2, true)], 0);
	let ex = Extra::default();
	let a = AchFacts { view: list(cheated), focus, first };
	let mut f = facts(&v, &ex, &text_of);
	f.lang = lang;
	f.ach = Some(&a);
	build(&f).expect("the achievements page builds")
}

fn cards(b: &Built) -> usize {
	match &b.screen.panel {
		Panel::Cards { cards, .. } => cards.len(),
		_ => 0,
	}
}

#[test]
fn five_cards_and_the_window_follows_the_focus() {
	let b = page(false, 0, 0, 0);
	let a = b.ach.as_ref().unwrap();
	assert_eq!((cards(&b), a.page, a.n, a.first, a.banner.is_none()), (CARDS, CARDS, 12, 0, true));
	assert_eq!(a.cards.iter().map(|c| c.index).collect::<Vec<_>>(), vec![0, 1, 2, 3, 4]);
	let b = page(false, 9, 0, 0);
	let a = b.ach.as_ref().unwrap();
	assert_eq!((a.first, a.focus), (5, 9));
	assert_eq!(a.item_of(9), Some(4));
	assert_eq!(a.row_at(4), Some(9));
	let Panel::Cards { cards, side, .. } = &b.screen.panel else { panic!("cards") };
	assert!(cards[4].selected && cards.iter().filter(|c| c.selected).count() == 1);
	assert_eq!(side, "2 / 12  \u{b7}  16%");
	// The focus past the end is the last row, the window the last page.
	let a = page(false, 99, 0, 0).ach.unwrap();
	assert_eq!((a.focus, a.first), (11, 7));
}

#[test]
fn the_cheated_session_shows_a_banner_over_the_full_list() {
	for lang in 0..5 {
		let b = page(true, 0, 0, lang);
		let a = b.ach.as_ref().unwrap();
		assert_eq!(a.banner.as_deref(), Some(BANNER[lang]));
		assert_eq!((cards(&b), a.page, a.cards.len()), (CARDS, CARDS - 1, CARDS - 1));
		assert_eq!((a.row_at(0), a.item_of(0), a.row_at(1)), (None, Some(1), Some(0)));
		for t in [&BANNER, &LOCKED, &UNLOCKED, &HIDDEN_NAME, &HIDDEN_DESC] {
			assert!(!t[lang].is_empty());
		}
		assert!(GRADES.iter().all(|g| !g[lang].is_empty()));
	}
}

#[test]
fn a_hidden_entry_keeps_its_secret_and_the_status_reads_right() {
	let a = page(false, 0, 0, 2).ach.unwrap();
	assert_eq!((a.cards[3].name.as_str(), a.cards[3].desc.as_str()), (HIDDEN_NAME[2], HIDDEN_DESC[2]));
	assert_eq!(a.cards[1].name, "N1");
	let l = list(false);
	assert_eq!(status(&l.rows[0], 0), "2025-11-03");
	assert_eq!(status(&l.rows[1], 2), UNLOCKED[2]);
	assert_eq!(status(&l.rows[2], 0), "3 / 10");
	assert_eq!(status(&l.rows[4], 1), LOCKED[1]);
}

#[test]
fn the_page_is_inert_and_keeps_the_options_tab() {
	let b = page(false, 0, 0, 0);
	assert!(b.items.iter().all(|i| i.is_none()) && b.menu.iter().all(|m| m.is_none()));
	assert!(!b.root && b.buttons.is_empty() && b.grid.is_none() && b.page == Page::Options);
	assert_eq!(b.screen.menu.iter().position(|m| m.selected), super::pause::branch(Page::Options));
	assert!(b.screen.menu.iter().all(|m| !m.live));
	// The kit lays the five cards out above the hint bar.
	let (l, _) = page_draw::frame(&b.screen, &FontMeasure);
	let items = &l.panel.as_ref().unwrap().items;
	assert_eq!(items.len(), CARDS);
	assert!(items.last().unwrap().rect.bottom() < 650.0);
}

#[test]
fn the_host_draws_names_banner_and_scroll_bar() {
	// `end` writes the host's statics and the command stamp: the host tests' lock (review R22 m4; each of them resets the host first).
	let _t = crate::retail::menu_model::cmd::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	let b = page(true, 0, 0, 0);
	let prims = super::pause_host::end(b, None, (1280, 720));
	let texts: Vec<&str> = prims.iter().filter_map(|p| if let Prim::Text(t) = p { Some(t.text.as_str()) } else { None }).collect();
	assert!(texts.contains(&BANNER[0]) && texts.contains(&"N0") && texts.contains(&"N2") && texts.contains(&HIDDEN_NAME[0]));
	assert!(!texts.contains(&"N4"), "row 4 is outside the 4-card window");
	assert!(prims.iter().any(|p| matches!(p, Prim::RRect { fill, .. } if *fill == rac_engine::ui::theme::SCROLLBAR)));
}

#[test]
fn the_open_list_is_a_page_not_a_list_to_step_aside_for() {
	// The list's open flag is shared with the host's tab click test (pause_host/tests.rs): its lock.
	let _t = crate::retail::menu_model::cmd::TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
	crate::achievements::open();
	let open = list_open();
	crate::achievements::close();
	assert!(!open, "the achievements list no longer makes the page step aside");
	let g = Gate { modern: true, ui_open: false, list_open: open, refused: false, front: false, paused: true, page: Some(Page::Options) };
	assert!(wanted(&g));
}
