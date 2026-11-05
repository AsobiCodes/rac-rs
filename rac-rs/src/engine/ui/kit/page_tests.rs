//! Tests of the page screen (`page.rs`, `page_draw.rs`): the mockup's numbers, the hit test, the dialog that swallows every other
//! hit, the scrim ramp, the fit of long texts.
use super::page::*;
use super::page_draw::{frame, scrim_alpha};
use super::text::FixedAdvance;
use crate::ui::input::Device;
use crate::ui::prim::{Prim, Symbol};

fn row(label: &str, selected: bool) -> PageRow {
	PageRow { label: label.to_string(), selected, ..Default::default() }
}

fn card(n: usize, selected: bool) -> PageRow {
	let st = |l: &str, v: &str| Stat { label: l.to_string(), value: v.to_string() };
	PageRow { label: format!("Slot {n}"), stats: vec![st("Bolts", "12345"), st("Play time", "1:02"), st("Saved", "01/02/03")], selected, ..Default::default() }
}

fn menu(sel: usize) -> Vec<MenuButton> {
	["New Game", "Load Game", "Options"].iter().enumerate().map(|(i, t)| MenuButton { text: t.to_string(), selected: i == sel, live: true }).collect()
}

fn new_page() -> PageScreen {
	PageScreen {
		menu: menu(0),
		panel: Panel::Cards { caption: "Select file".into(), side: "5 slots".into(), cards: (0..5).map(|i| card(i, i == 2)).collect(), extra: Some(row("Continue without saving", false)) },
		..Default::default()
	}
}

#[test]
fn menu_buttons_sit_at_the_mockup_numbers() {
	let m = FixedAdvance(0.5);
	let l = super::page_draw::frame(&new_page(), &m).0;
	assert_eq!(l.menu.len(), 3);
	assert_eq!((l.menu[0].rect.x, l.menu[0].rect.y, l.menu[0].rect.w, l.menu[0].rect.h), (48.0, 232.0, 360.0, 64.0));
	assert_eq!(l.menu[1].rect.y, 232.0 + 72.0, "gap 8");
	assert_eq!((l.menu[0].bar.w, l.menu[0].bar.h, l.menu[0].bar.x), (4.0, 28.0, 68.0));
	assert_eq!(l.menu[0].text.x, 86.0, "text at x + 38");
	assert_eq!(l.menu[0].text.text, "NEW GAME", "upper-cased");
}

#[test]
fn new_game_panel_fits_above_the_hint_bar() {
	let m = FixedAdvance(0.5);
	let l = frame(&new_page(), &m).0;
	let p = l.panel.expect("panel");
	assert_eq!((p.rect.x, p.rect.y, p.rect.w), (456.0, 116.0, 776.0));
	assert!(p.rect.bottom() <= 670.0, "bottom {} must clear the hint bar", p.rect.bottom());
	assert_eq!(p.items.len(), 6, "5 cards and the extra row");
	assert!(p.items[0].card && !p.items[5].card);
	let mut cards = p.items.iter().take(5).map(|i| i.rect);
	let first = cards.next().expect("card");
	assert!(cards.all(|r| r.x == first.x && r.w == first.w && r.y > first.y));
	for (lab, val) in &p.items[0].stats {
		assert!(lab.x == val.x && lab.y < val.y, "label over value, right aligned at the same x");
	}
}

#[test]
fn rows_panel_moves_up_above_six_rows() {
	let m = FixedAdvance(0.5);
	let rows = |n: usize| PageScreen { menu: menu(2), panel: Panel::Rows { caption: "Options".into(), rows: (0..n).map(|i| row("Row", i == 0)).collect() }, ..Default::default() };
	let p = frame(&rows(4), &m).0.panel.expect("panel");
	assert_eq!((p.rect.x, p.rect.y, p.rect.w), (456.0, 232.0, 560.0));
	assert_eq!(p.items[1].rect.y - p.items[0].rect.y, 58.0, "row 56 + gap 2");
	let p = frame(&rows(8), &m).0.panel.expect("panel");
	assert_eq!(p.rect.y, 116.0);
	assert_eq!(p.items[1].rect.y - p.items[0].rect.y, 58.0, "8 rows keep the full height");
	// A title Mods section (13 rows) shrinks its rows so the panel ends above the hint bar, as the shell's list does.
	let p = frame(&rows(13), &m).0.panel.expect("panel");
	assert!(p.rect.bottom() <= 656.0 + 0.01, "panel ends at {} (hint bar at 688)", p.rect.bottom());
	assert!(p.items[1].rect.h >= 34.0 && p.items[1].rect.h < ROW_H);
}

#[test]
fn fourteen_rows_still_fit_above_the_hint_bar() {
	let m = FixedAdvance(0.5);
	let rows = |n: usize| PageScreen { menu: menu(2), panel: Panel::Rows { caption: "Options".into(), rows: (0..n).map(|i| row("Row", i == 0)).collect() }, ..Default::default() };
	let p = frame(&rows(14), &m).0.panel.expect("panel");
	assert!(p.rect.bottom() <= 656.0 + 0.01 && p.items[0].rect.h >= 30.0, "14 rows: panel ends at {}, row {}", p.rect.bottom(), p.items[0].rect.h);
	let p = frame(&rows(12), &m).0.panel.expect("panel");
	assert!((p.items[0].rect.h - (472.0 / 12.0 - 2.0)).abs() < 0.01, "12 rows keep today's pitch");
}

#[test]
fn a_long_rows_panel_shrinks_above_the_hint_bar_and_hints_win_the_click() {
	let m = FixedAdvance(0.5);
	let mut s = PageScreen { menu: menu(2), panel: Panel::Rows { caption: "Options".into(), rows: (0..10).map(|i| row("Row", i == 0)).collect() }, ..Default::default() };
	s.hints = vec![crate::ui::kit::data::Hint { kind: crate::ui::kit::data::HintKind::Back, text: "Back".into() }];
	let l = frame(&s, &m).0;
	let p = l.panel.as_ref().expect("panel");
	assert!(p.items[9].rect.bottom() <= 656.0 - PANEL_PAD + 0.01, "last row bottom {}", p.items[9].rect.bottom());
	assert!(p.items[1].rect.y - p.items[0].rect.y < 58.0, "10 rows shrink");
	let h = l.hints.iter().find(|h| h.kind == crate::ui::kit::data::HintKind::Back).expect("hint").rect;
	assert_eq!(l.hit(&s, h.cx(), h.cy()), PageHit::Hint(crate::ui::kit::data::HintKind::Back));
}

#[test]
fn hit_finds_menu_cards_and_ignores_inert_buttons() {
	let m = FixedAdvance(0.5);
	let mut s = new_page();
	let l = frame(&s, &m).0;
	assert_eq!(l.hit(&s, 100.0, 260.0), PageHit::Menu(0));
	assert_eq!(l.hit(&s, 100.0, 340.0), PageHit::Menu(1));
	let c3 = l.panel.as_ref().expect("panel").items[3].rect;
	assert_eq!(l.hit(&s, c3.cx(), c3.cy()), PageHit::Item(3));
	assert_eq!(l.hit(&s, 1270.0, 5.0), PageHit::None);
	s.menu[1].live = false;
	assert_eq!(l.hit(&s, 100.0, 340.0), PageHit::None, "an inert button is not clickable");
}

#[test]
fn dialog_swallows_other_hits_and_centres() {
	let m = FixedAdvance(0.5);
	let mut s = new_page();
	s.dialog = Some(Dialog {
		lines: vec!["Overwrite this file?".into(), String::new(), "The old save is lost.".into()],
		buttons: vec![DialogButton { symbol: Symbol::Circle, key: "Enter".into(), text: "Yes".into() }, DialogButton { symbol: Symbol::Triangle, key: "Esc".into(), text: "No".into() }],
	});
	let l = frame(&s, &m).0;
	let d = l.dialog.as_ref().expect("dialog");
	assert_eq!(d.rect.w, 520.0);
	assert!((d.rect.cx() - 640.0).abs() < 0.01 && (d.rect.cy() - 360.0).abs() < 0.01);
	assert_eq!(l.hit(&s, 100.0, 260.0), PageHit::None, "the menu behind is inert while the dialog is up");
	let b1 = d.buttons[1].rect;
	assert_eq!(l.hit(&s, b1.cx(), b1.cy()), PageHit::Button(1));
	assert!(d.buttons[0].rect.right() < d.buttons[1].rect.x);
	assert!(matches!(d.buttons[0].lead.what, super::layout::LeadWhat::Key(_)), "keyboard: keycap");
	s.device = Device::Pad;
	let l = frame(&s, &m).0;
	assert!(matches!(l.dialog.as_ref().expect("dialog").buttons[0].lead.what, super::layout::LeadWhat::Pad(Symbol::Circle)), "pad: the retail symbol");
}

#[test]
fn scrim_ramp_matches_the_mockup() {
	assert!((scrim_alpha(0.0) - 0.88).abs() < 1e-5);
	assert!((scrim_alpha(576.0) - 0.55).abs() < 1e-5);
	assert!(scrim_alpha(960.0).abs() < 1e-5 && scrim_alpha(1200.0) == 0.0);
	let prims = frame(&new_page(), &FixedAdvance(0.5)).1;
	let strips = prims.iter().filter(|p| matches!(p, Prim::RRect { radius, border: None, .. } if *radius == 0.0)).count();
	assert_eq!(strips, 32);
	assert!(!prims.iter().any(|p| matches!(p, Prim::Backdrop { .. })), "no blur backdrop: the scene stays sharp");
}

#[test]
fn long_label_gets_a_bounded_width() {
	let m = FixedAdvance(0.5);
	let s = PageScreen { menu: menu(2), panel: Panel::Rows { caption: "x".into(), rows: vec![PageRow { label: "A very long option label that cannot fit".into(), value: "Some long value".into(), ..Default::default() }] }, ..Default::default() };
	let it = &frame(&s, &m).0.panel.expect("panel").items[0];
	assert!(it.label_w <= 330.0 && it.label_w >= 60.0);
	assert!(it.value.as_ref().expect("value").x > it.label.x);
}

fn shell_page(tabs: &[&str], sel: usize, rows: usize) -> PageScreen {
	use super::shell::Shell;
	PageScreen {
		menu: tabs.iter().enumerate().map(|(i, t)| MenuButton { text: t.to_string(), selected: i == sel, live: true }).collect(),
		panel: Panel::Rows { caption: String::new(), rows: (0..rows).map(|i| PageRow { label: format!("Row {i}"), value: "50%".into(), bar: Some(0.5), selected: i == 1, ..Default::default() }).collect() },
		shell: Some(Shell { label: "Paused".into(), site_label: "Logging site".into(), site: "Eudora".into(), bolts: "1,185".into() }),
		..Default::default()
	}
}

#[test]
fn shell_tabs_sit_in_a_row_the_list_at_the_mockup_corner_and_a_slider_has_a_bar() {
	let m = FixedAdvance(0.5);
	let l = frame(&shell_page(&["Weapons", "Gadgets", "Help", "Options"], 3, 10), &m).0;
	assert_eq!(l.menu.len(), 4);
	assert_eq!((l.menu[0].rect.x, l.menu[0].rect.y), (48.0, 61.0));
	assert!(l.menu.windows(2).all(|w| w[1].rect.x == w[0].rect.right()), "tabs touch");
	let p = l.panel.as_ref().expect("panel");
	assert_eq!((p.rect.x, p.rect.y, p.rect.w), (48.0, 116.0, 520.0));
	assert!(p.rect.bottom() <= 660.0, "ten rows shrink to stay above the hint bar: {}", p.rect.bottom());
	assert!(p.items[0].bar.is_some_and(|b| b.w == BAR_W));
	assert!(l.shell.is_some());
}

#[test]
fn shell_tabs_shrink_instead_of_running_into_the_site_block() {
	let m = FixedAdvance(0.5);
	let long = ["Armes", "Gadgets", "Selection rapide", "Objets", "Aide", "Options", "Bonus"];
	let l = frame(&shell_page(&long, 0, 3), &m).0;
	let sh = l.shell.as_ref().expect("shell");
	assert!(sh.tab_px <= 25.0 && sh.tab_px >= 15.0);
	let end = l.menu.last().map_or(0.0, |b| b.rect.right());
	assert!(end <= sh.site.x - 24.0 + 1.0, "tabs end {end} before the site block");
}

#[test]
fn shell_hit_finds_a_live_tab_and_the_dialog_still_swallows() {
	let m = FixedAdvance(0.5);
	let s = shell_page(&["Weapons", "Help"], 0, 2);
	let l = frame(&s, &m).0;
	let t = l.menu[1].rect;
	assert_eq!(l.hit(&s, t.cx(), t.cy()), PageHit::Menu(1));
	let r = l.panel.as_ref().expect("panel").items[0].rect;
	assert_eq!(l.hit(&s, r.cx(), r.cy()), PageHit::Item(0));
}

#[test]
fn hit_finds_the_hint_buttons_but_not_the_tabs_hint() {
	use super::data::{Hint, HintKind};
	let m = FixedAdvance(0.5);
	let mut s = new_page();
	s.hints = vec![
		Hint { kind: HintKind::Tabs, text: "Switch tab".into() },
		Hint { kind: HintKind::Change, text: "Select".into() },
		Hint { kind: HintKind::Back, text: "Back".into() },
	];
	let l = frame(&s, &m).0;
	assert_eq!(l.hints.len(), 3);
	let at = |k: HintKind| l.hints.iter().find(|h| h.kind == k).expect("hint").rect;
	let (change, back, tabs) = (at(HintKind::Change), at(HintKind::Back), at(HintKind::Tabs));
	assert_eq!(l.hit(&s, change.cx(), change.cy()), PageHit::Hint(HintKind::Change));
	assert_eq!(l.hit(&s, back.cx(), back.cy()), PageHit::Hint(HintKind::Back));
	assert_eq!(l.hit(&s, tabs.cx(), tabs.cy()), PageHit::None, "the Tabs hint is plain text");
	assert!(change.right() < back.x && back.right() <= 1240.0 + 1.0, "right-aligned, no overlap");
	s.dialog = Some(Dialog { lines: vec!["Sure?".into()], buttons: vec![] });
	let l = frame(&s, &m).0;
	assert_eq!(l.hit(&s, back.cx(), back.cy()), PageHit::None, "a dialog swallows the hint bar too");
}
