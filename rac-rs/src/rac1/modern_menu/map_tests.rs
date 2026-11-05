//! Tests of the modern planet map (`map.rs`): when the page replaces retail's picture (`wanted`), the holes for the game's pictures
//! (inside their boxes, at the crop's display aspect, uvs inside the overlay), the key rows that are or are not clickable, the tab bar,
//! the unknown planet, and the shell scrim with holes (`scrim_holes`, `compose`).
use super::map::*;
use crate::retail::map_seen::Seen;
use rac_engine::ui::font::{FontMeasure, FontSet};
use rac_engine::ui::input::{Device, GlyphSet};
use rac_engine::ui::kit::shell::{Shell, SHELL_SCRIM};
use rac_engine::ui::prim::{Prim, Rect};

fn keys() -> Vec<KeyRow> {
	(0..6).map(|i| KeyRow { text: format!("Key {i}"), bit: if i == 4 { 0 } else { 0x10 << i } }).collect()
}

fn tabs() -> Vec<String> {
	["Weapons", "Items", "Map"].iter().map(|s| s.to_string()).collect()
}

fn page_of(stage: Stage, seen: Option<Seen>, settled: bool, keys: &[KeyRow]) -> Built {
	let t = |id: u32| match id {
		0x4eb9..=0x4ecb => "Place".to_string(),
		0x4ecc..=0x4ee0 => "Planet".to_string(),
		_ => String::new(),
	};
	let tabs = tabs();
	let f = Facts { stage, seen, gold: Some((1, 3)), text: &t, keys, shell: Shell::default(), tabs: &tabs, lang: 0, device: Device::Keyboard, glyphs: GlyphSet::PlayStation, settled };
	build(&f, &FontMeasure)
}

fn known() -> Option<Seen> {
	Some(Seen { planet: Some(3), unknown: false })
}

fn texts(b: &Built) -> Vec<String> {
	b.prims.iter().filter_map(|p| if let Prim::Text(t) = p { Some(t.text.clone()) } else { None }).collect()
}

fn inside(inner: Rect, outer: Rect) -> bool {
	inner.x >= outer.x - 0.01 && inner.y >= outer.y - 0.01 && inner.right() <= outer.right() + 0.01 && inner.bottom() <= outer.bottom() + 0.01
}

#[test]
fn stage_and_gate() {
	assert_eq!(stage_of(0xb), Some(Stage::Map));
	assert_eq!(stage_of(0xc), Some(Stage::Missions));
	assert_eq!(stage_of(0xf), None);
	let g = Gate { modern: true, ui_open: false, refused: false, page: Some(0xb) };
	assert!(wanted(&g));
	assert!(wanted(&Gate { page: Some(0xc), ..g }));
	assert!(!wanted(&Gate { page: Some(0xf), ..g }));
	assert!(!wanted(&Gate { page: None, ..g }));
	assert!(!wanted(&Gate { modern: false, ..g }));
	assert!(!wanted(&Gate { ui_open: true, ..g }));
	assert!(!wanted(&Gate { refused: true, ..g }));
}

#[test]
fn settled_holes_sit_in_their_boxes_at_the_display_aspect() {
	let k = keys();
	let b = page_of(Stage::Map, known(), true, &k);
	assert_eq!(b.holes.len(), 2);
	for ((hole, uv), (bx, src)) in b.holes.iter().zip([(GLOBE_BOX, GLOBE_SRC), (MAP_BOX, MAP_SRC)]) {
		assert!(inside(*hole, bx), "{hole:?} in {bx:?}");
		let want = (src[2] - 6.0) * 7.0 / ((src[3] - 6.0) * 6.0);
		assert!((hole.w / hole.h - want).abs() < 0.01, "{} vs {want}", hole.w / hole.h);
		assert!(uv.iter().all(|u| (0.0..=1.0).contains(u)) && uv[0] < uv[2] && uv[1] < uv[3], "{uv:?}");
	}
	let b = page_of(Stage::Missions, known(), true, &k[..1]);
	assert_eq!(b.holes.len(), 3);
	for ((hole, uv), (bx, src)) in b.holes.iter().zip([(LIST_BOX, LIST_SRC), (PORTRAIT_BOX, PORTRAIT_SRC), (DESC_BOX, DESC_SRC)]) {
		assert!(inside(*hole, bx), "{hole:?} in {bx:?}");
		let want = (src[2] - 6.0) * 7.0 / ((src[3] - 6.0) * 6.0);
		assert!((hole.w / hole.h - want).abs() < 0.01);
		assert!(uv.iter().all(|u| (0.0..=1.0).contains(u)));
	}
}

#[test]
fn unsettled_page_has_no_holes() {
	let k = keys();
	for stage in [Stage::Map, Stage::Missions] {
		let b = page_of(stage, known(), false, &k);
		assert!(b.holes.is_empty());
		assert!(!b.prims.is_empty());
	}
}

#[test]
fn key_rows_click_by_their_bit() {
	let k = keys();
	let b = page_of(Stage::Map, known(), true, &k);
	assert_eq!(b.acts.len(), 5, "the sticks row (bit 0) is not clickable");
	// Row 0: left column, first cell (64, 448, 264, 56).
	assert_eq!(b.act_at(100.0, 470.0), Some(0x10));
	// Row 3: right column, first cell (328, 448).
	assert_eq!(b.act_at(400.0, 470.0), Some(0x10 << 3));
	// Row 4 (bit 0): right column, second cell.
	assert_eq!(b.act_at(400.0, 534.0), None);
	assert_eq!(b.act_at(20.0, 20.0), None);
	// More than six rows: the rest is ignored.
	let mut many = keys();
	many.push(KeyRow { text: "Extra".into(), bit: 1 });
	assert_eq!(page_of(Stage::Map, known(), true, &many).acts.len(), 5);
}

#[test]
fn missions_hint_bar_is_right_aligned_and_clickable() {
	let k = vec![KeyRow { text: "Exit".into(), bit: 0x40 }, KeyRow { text: "Sticks".into(), bit: 0 }];
	let b = page_of(Stage::Missions, known(), true, &k);
	assert_eq!(b.acts.len(), 1);
	let r = b.acts[0].1;
	assert!(r.right() <= 1232.0 + 20.0 && r.y > 660.0);
	assert_eq!(b.act_at(r.cx(), r.cy()), Some(0x40));
	let none = page_of(Stage::Missions, known(), true, &[]);
	assert!(none.acts.is_empty());
}

#[test]
fn map_tab_is_last_and_alone_selected() {
	let k = keys();
	let b = page_of(Stage::Map, known(), true, &k);
	assert_eq!(b.map_tab, 2);
	assert_eq!(b.screen.menu.len(), 3);
	assert!(b.screen.menu.iter().enumerate().all(|(i, t)| t.selected == (i == 2) && !t.live));
	assert!(b.screen.shell.is_some() && b.screen.hints.is_empty());
}

#[test]
fn unknown_planet_shows_question_marks_and_no_location() {
	let k = keys();
	let b = page_of(Stage::Map, Some(Seen { planet: None, unknown: true }), true, &k);
	let t = texts(&b);
	assert!(t.iter().any(|s| s == "????"), "{t:?}");
	assert!(!t.iter().any(|s| s.contains("PLACE")));
	let none = page_of(Stage::Map, None, true, &k);
	assert!(!texts(&none).iter().any(|s| s == "????" || s == "Planet"));
	let known = page_of(Stage::Map, known(), true, &k);
	let t = texts(&known);
	assert!(t.iter().any(|s| s == "PLACE") && t.iter().any(|s| s == "Planet"), "{t:?}");
	assert!(t.iter().any(|s| s == "1 of 3"));
}

fn rects_of(prims: &[Prim]) -> Vec<Rect> {
	prims.iter().filter_map(|p| if let Prim::RRect { rect, fill, .. } = p { (*fill == SHELL_SCRIM).then_some(*rect) } else { None }).collect()
}

#[test]
fn scrim_covers_everything_but_the_holes() {
	let holes = [Rect::new(64.0, 132.0, 200.0, 100.0), Rect::new(300.0, 200.0, 150.0, 300.0), Rect::new(700.0, 132.0, 100.0, 50.0), Rect::new(100.0, 700.0, 50.0, 10.0)];
	let none: [Rect; 0] = [];
	for hs in [&holes[..], &holes[..1], &none[..]] {
		let rects = rects_of(&scrim_holes(hs));
		if hs.is_empty() {
			assert_eq!(rects.len(), 1);
		}
		let mut y = -3000.37;
		while y < 3000.0 {
			let mut x = -3000.13;
			while x < 3000.0 {
				let n = rects.iter().filter(|r| r.contains(x, y)).count();
				let in_hole = hs.iter().any(|h| x > h.x && x < h.right() && y > h.y && y < h.bottom());
				assert_eq!(n, if in_hole { 0 } else { 1 }, "({x}, {y})");
				x += 17.3;
			}
			y += 13.7;
		}
		for r in &rects {
			assert!(hs.iter().all(|h| !r.overlaps(h)), "{r:?}");
		}
	}
}

#[test]
fn compose_replaces_the_scrim_and_appends_the_page() {
	let k = keys();
	let b = page_of(Stage::Map, known(), true, &k);
	let giant = Prim::RRect { rect: Rect::new(-5000.0, -5000.0, 10000.0, 10000.0), radius: 0.0, fill: SHELL_SCRIM, border: None };
	let shell = vec![Prim::ClipPop, giant.clone(), Prim::ClipPop];
	let out = compose(shell.clone(), &b);
	let holes: Vec<Rect> = b.holes.iter().map(|h| h.0).collect();
	let cut = scrim_holes(&holes);
	assert_eq!(out.len(), shell.len() - 1 + cut.len() + b.prims.len());
	assert!(!out.contains(&giant));
	assert_eq!(&out[out.len() - b.prims.len()..], &b.prims[..]);
	// The page's own prims hold no SHELL_SCRIM fill, so every one in the output comes from the cut scrim.
	assert_eq!(out.iter().filter(|p| matches!(p, Prim::RRect { fill, .. } if *fill == SHELL_SCRIM)).count(), cut.len());
	// Unsettled: no holes, the shell stays as it is.
	let flat = page_of(Stage::Map, known(), false, &k);
	let out = compose(shell.clone(), &flat);
	assert_eq!(out.len(), shell.len() + flat.prims.len());
	assert!(out.contains(&giant));
}

#[test]
fn words_are_drawable() {
	let set = FontSet::get();
	for tab in all() {
		for s in tab.iter() {
			assert!(s.chars().all(|c| set.covers(c)), "{s:?}");
		}
	}
}
